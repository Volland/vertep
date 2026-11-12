//! A custodian's copy of one durable object: the promised epoch, the accepted epoch, the
//! entries (in clear at hosts, encrypted at keepers), the configurations and key wraps it
//! accepted, and equivocation evidence.
//!
//! The replica is durable as an append-only log of custody records in a journal store, under
//! the actor `<object>/custody-log`: a promise, an accepted batch (with the position it
//! replaced the tail from) or a piece of evidence. Opening a replica replays the log. Replacing
//! an uncommitted tail is a record like any other, so no store needs a truncate operation.

use core::fmt;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use oxrdf::{Literal, NamedNode, TermRef, Triple};
use serde::{Deserialize, Serialize};
use vertep_federation::PublicKey;
use vertep_store::{AppendMeta, Entry, JournalStore, Position, StoreError};

use super::config::{CustodyConfig, Role};
use super::wire::{
    AppendBatch, CustodyRequest, CustodyResponse, EpochPromise, Evidence, KeyDirectory, KeyWrap,
    WireEntry, WireFacts, entry_from_nquads, verify_keeper_authorship, verify_keeper_seal,
};
use crate::chain::{ChainHead, Digest, EntrySigner, HostKeys, verify_entry};

/// The predicate holding a custody record's JSON in the custody log.
pub const CUSTODY_RECORD: &str = "https://w3id.org/vertep/vt#custodyRecord";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[allow(clippy::large_enum_variant)] // written and replayed one at a time
enum LogRecord {
    Promised {
        epoch: u64,
    },
    Accepted {
        epoch: u64,
        /// The entries after this position were replaced by `entries`.
        base: u64,
        entries: Vec<WireEntry>,
        config: Option<CustodyConfig>,
        keys: Vec<KeyWrap>,
    },
    Evidence {
        evidence: Evidence,
    },
}

/// Receives equivocation evidence, to forward it to the object's controller.
pub trait EvidenceSink: Send + Sync {
    /// Called once per piece of evidence, after it has been recorded.
    fn report(&self, evidence: &Evidence);
}

#[derive(Clone, Debug, Default)]
struct State {
    promised: u64,
    accepted_epoch: u64,
    entries: Vec<WireEntry>,
    facts: Vec<WireFacts>,
    /// (epoch from which a configuration holds, configuration), ascending.
    configs: Vec<(u64, CustodyConfig)>,
    keys: Vec<KeyWrap>,
    evidence: Vec<Evidence>,
    refused: BTreeSet<u64>,
    log_head: u64,
}

impl State {
    fn config(&self) -> &CustodyConfig {
        &self
            .configs
            .last()
            .expect("a replica has a configuration")
            .1
    }

    fn config_at(&self, epoch: u64) -> &CustodyConfig {
        self.configs
            .iter()
            .rev()
            .find(|(from, _)| *from <= epoch)
            .map_or_else(|| &self.configs[0].1, |(_, c)| c)
    }

    fn head(&self) -> u64 {
        self.entries.len() as u64
    }

    fn head_hash(&self) -> Option<Digest> {
        self.facts.last().map(|f| f.hash)
    }

    fn apply(&mut self, rec: &LogRecord) {
        match rec {
            LogRecord::Promised { epoch } => self.promised = self.promised.max(*epoch),
            LogRecord::Accepted {
                epoch,
                base,
                entries,
                config,
                keys,
            } => {
                self.promised = self.promised.max(*epoch);
                self.accepted_epoch = *epoch;
                self.entries.truncate(*base as usize);
                self.facts.truncate(*base as usize);
                for e in entries {
                    if let Ok(f) = e.facts() {
                        self.facts.push(f);
                        self.entries.push(e.clone());
                    }
                }
                if let Some(c) = config
                    && c.version > self.config().version
                {
                    self.configs.push((*epoch, c.clone()));
                }
                for k in keys {
                    if !self.keys.contains(k) {
                        self.keys.push(k.clone());
                    }
                }
            }
            LogRecord::Evidence { evidence } => {
                self.refused.insert(evidence.epoch);
                self.evidence.push(evidence.clone());
            }
        }
    }
}

/// Host keys of a replica: a writer is a host of an epoch when the configuration in force at
/// that epoch names it a host, and its keys come from the key directory.
struct ConfigHostKeys<'a> {
    state: &'a State,
    directory: &'a dyn KeyDirectory,
}

impl HostKeys for ConfigHostKeys<'_> {
    fn host_keys(&self, _object: &NamedNode, epoch: u64, writer: &str) -> Option<Vec<PublicKey>> {
        if !self.state.config_at(epoch).is_host(writer) {
            return None;
        }
        self.directory.verification_keys(writer)
    }
}

/// A custodian's replica of one durable object.
pub struct Replica {
    object: NamedNode,
    log_actor: NamedNode,
    me: Arc<dyn EntrySigner>,
    store: Arc<dyn JournalStore>,
    directory: Arc<dyn KeyDirectory>,
    sink: Mutex<Option<Arc<dyn EvidenceSink>>>,
    history: Mutex<Option<Arc<super::history::History>>>,
    state: Mutex<State>,
}

impl fmt::Debug for Replica {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Replica")
            .field("object", &self.object)
            .field("custodian", &self.me.writer())
            .finish_non_exhaustive()
    }
}

/// The custody-log actor of `object`.
pub fn log_actor(object: &NamedNode) -> NamedNode {
    NamedNode::new_unchecked(format!("{}/custody-log", object.as_str()))
}

impl Replica {
    /// Opens the replica of `object` kept by `me` in `store`, replaying its custody log.
    /// `initial` is the configuration from the object's DID document, used until the replica
    /// accepts a newer one.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the log cannot be read or holds an unreadable record.
    pub fn open(
        object: NamedNode,
        me: Arc<dyn EntrySigner>,
        store: Arc<dyn JournalStore>,
        directory: Arc<dyn KeyDirectory>,
        initial: CustodyConfig,
    ) -> Result<Self, StoreError> {
        let log_actor = log_actor(&object);
        let mut state = State {
            configs: vec![(0, initial)],
            ..State::default()
        };
        let p = NamedNode::new_unchecked(CUSTODY_RECORD);
        for r in store.read(&log_actor, Position::ZERO)? {
            let text = r
                .entry
                .triples()
                .objects_for_subject_predicate(r.entry.name(), &p)
                .find_map(|o| match o {
                    TermRef::Literal(l) => Some(l.value().to_owned()),
                    _ => None,
                })
                .ok_or_else(|| StoreError::invalid("custody record without vt:custodyRecord"))?;
            let rec: LogRecord = serde_json::from_str(&text)
                .map_err(|e| StoreError::invalid(format!("custody record: {e}")))?;
            state.apply(&rec);
            state.log_head = r.position.get();
        }
        Ok(Replica {
            object,
            log_actor,
            me,
            store,
            directory,
            sink: Mutex::new(None),
            history: Mutex::new(None),
            state: Mutex::new(state),
        })
    }

    /// Recovers from a crash: drops the in-memory state and replays the custody log from the
    /// store, as opening the replica again would.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the log cannot be read.
    pub fn reload(&self) -> Result<(), StoreError> {
        let initial = self.lock().configs[0].1.clone();
        let fresh = Replica::open(
            self.object.clone(),
            self.me.clone(),
            self.store.clone(),
            self.directory.clone(),
            initial,
        )?;
        let st = fresh.lock().clone();
        *self.lock() = st;
        if let Some(h) = self.history() {
            h.record(serde_json::json!({"kind": "restart", "custodian": self.custodian()}));
        }
        Ok(())
    }

    /// Sends equivocation evidence to `sink` from now on.
    pub fn set_evidence_sink(&self, sink: Arc<dyn EvidenceSink>) {
        *self
            .sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sink);
    }

    /// Records this replica's promises and accepted batches in `history`.
    pub fn set_history(&self, history: Arc<super::history::History>) {
        *self
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(history);
    }

    fn history(&self) -> Option<Arc<super::history::History>> {
        self.history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn log_hashes(st: &State) -> Vec<String> {
        st.facts.iter().map(|f| f.hash.to_hex()).collect()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The object.
    pub fn object(&self) -> &NamedNode {
        &self.object
    }

    /// The custodian's DID.
    pub fn custodian(&self) -> &str {
        self.me.writer()
    }

    /// The head of the replica.
    pub fn head(&self) -> Position {
        Position::new(self.lock().head())
    }

    /// The highest epoch promised.
    pub fn promised(&self) -> u64 {
        self.lock().promised
    }

    /// The epoch the journal was last accepted in.
    pub fn accepted_epoch(&self) -> u64 {
        self.lock().accepted_epoch
    }

    /// The configuration in force.
    pub fn config(&self) -> CustodyConfig {
        self.lock().config().clone()
    }

    /// Every configuration accepted, with the epoch it holds from.
    pub fn configs(&self) -> Vec<(u64, CustodyConfig)> {
        self.lock().configs.clone()
    }

    /// The stored entries, in order.
    pub fn entries(&self) -> Vec<WireEntry> {
        self.lock().entries.clone()
    }

    /// The key wraps held.
    pub fn keys(&self) -> Vec<KeyWrap> {
        self.lock().keys.clone()
    }

    /// The equivocation evidence recorded.
    pub fn evidence(&self) -> Vec<Evidence> {
        self.lock().evidence.clone()
    }

    fn write(&self, state: &mut State, rec: LogRecord) -> Result<(), StoreError> {
        let n = state.log_head + 1;
        let name = NamedNode::new_unchecked(format!("{}/{n}", self.log_actor.as_str()));
        let text = serde_json::to_string(&rec).map_err(StoreError::backend)?;
        let entry = Entry::new(
            name.clone(),
            [Triple::new(
                name,
                NamedNode::new_unchecked(CUSTODY_RECORD),
                Literal::new_simple_literal(text),
            )],
        );
        self.store
            .append(
                &self.log_actor,
                Position::new(state.log_head),
                &[entry],
                &AppendMeta::none(),
            )
            .map_err(|e| match e {
                vertep_store::AppendError::Storage(s) => s,
                other => StoreError::Backend(other.to_string()),
            })?;
        state.log_head = n;
        state.apply(&rec);
        Ok(())
    }

    /// Answers one custody request.
    pub fn handle(&self, request: &CustodyRequest) -> CustodyResponse {
        let object = match request {
            CustodyRequest::Prepare { object, .. } | CustodyRequest::Read { object, .. } => object,
            CustodyRequest::Append(b) => &b.object,
        };
        if object != self.object.as_str() {
            return CustodyResponse::Failed {
                reason: format!("{} keeps {}, not {object}", self.custodian(), self.object),
            };
        }
        match request {
            CustodyRequest::Prepare {
                epoch, candidate, ..
            } => {
                let before = self.history().map(|_| {
                    let st = self.lock();
                    (st.promised, st.accepted_epoch, Self::log_hashes(&st))
                });
                let answer = self.prepare(*epoch);
                if let (Some(h), Some((highest, ae, log))) = (self.history(), before) {
                    h.record(serde_json::json!({
                        "kind": "prepare", "custodian": self.custodian(), "candidate": candidate,
                        "epoch": epoch, "promised": matches!(answer, CustodyResponse::Promise(_)),
                        "highest_before": highest, "accepted_epoch": ae, "log": log,
                    }));
                }
                answer
            }
            CustodyRequest::Append(batch) => {
                let answer = self.append(batch);
                if let Some(h) = self.history() {
                    let st = self.lock();
                    let outcome = match &answer {
                        CustodyResponse::Ack { .. } => "ack",
                        CustodyResponse::Behind { .. } => "behind",
                        CustodyResponse::Diverged { .. } => "diverged",
                        // Only an epoch below the promise is a refusal of the protocol; the
                        // others (outdated configuration, an epoch refused after equivocation)
                        // are checks outside it and change nothing.
                        CustodyResponse::Refused { highest, .. } if batch.epoch < *highest => {
                            "refused"
                        }
                        _ => "failed",
                    };
                    let entries: Vec<String> = batch
                        .entries
                        .iter()
                        .filter_map(|e| e.facts().ok().map(|f| f.hash.to_hex()))
                        .collect();
                    h.record(serde_json::json!({
                        "kind": "accept", "custodian": self.custodian(), "writer": batch.writer,
                        "epoch": batch.epoch, "after": batch.after, "entries": entries,
                        "outcome": outcome, "log_after": Self::log_hashes(&st),
                        "accepted_epoch_after": st.accepted_epoch,
                    }));
                }
                answer
            }
            CustodyRequest::Read { from, .. } => self.read(*from),
        }
    }

    // @tg: implements:: [[openspec:object-custody#Quorum-Promised Custody Epochs]]
    fn prepare(&self, epoch: u64) -> CustodyResponse {
        let mut st = self.lock();
        if epoch <= st.promised {
            return CustodyResponse::Refused {
                highest: st.promised,
                reason: "a higher or equal epoch was promised".into(),
            };
        }
        if let Err(e) = self.write(&mut st, LogRecord::Promised { epoch }) {
            return CustodyResponse::Failed {
                reason: e.to_string(),
            };
        }
        let promise = EpochPromise {
            object: self.object.as_str().to_owned(),
            epoch,
            custodian: self.custodian().to_owned(),
            head: st.head(),
            head_hash: st.head_hash().map(|d| d.to_hex()),
            accepted_epoch: st.accepted_epoch,
            config: st.config().clone(),
            signature: String::new(),
        };
        match promise.sign(self.me.as_ref()) {
            Ok(p) => CustodyResponse::Promise(p),
            Err(e) => CustodyResponse::Failed {
                reason: e.to_string(),
            },
        }
    }

    fn read(&self, from: u64) -> CustodyResponse {
        let st = self.lock();
        let from = from.max(1) as usize;
        CustodyResponse::Entries {
            entries: st.entries.iter().skip(from - 1).cloned().collect(),
            keys: st.keys.clone(),
            configs: st.configs.clone(),
        }
    }

    /// Checks `entries` as the continuation of the replica's entries up to `base`.
    fn verify_new(
        &self,
        st: &State,
        base: u64,
        entries: &[WireEntry],
        batch_config: &CustodyConfig,
    ) -> Result<Vec<WireFacts>, String> {
        // Configurations in force while checking: the replica's, plus the batch's for its epoch.
        let mut view = st.clone();
        if batch_config.version > view.config().version {
            view.configs.push((u64::MAX, batch_config.clone()));
        }
        let keys = ConfigHostKeys {
            state: &view,
            directory: self.directory.as_ref(),
        };
        let mut head = ChainHead {
            position: Position::new(base),
            hash: if base == 0 {
                None
            } else {
                Some(st.facts[base as usize - 1].hash)
            },
            epoch: if base == 0 {
                0
            } else {
                st.facts[base as usize - 1].epoch
            },
        };
        let mut out = Vec::new();
        for e in entries {
            match e {
                WireEntry::Clear { nquads } => {
                    let entry = entry_from_nquads(nquads).map_err(|e| e.to_string())?;
                    head = verify_entry(&self.object, head, &entry, &keys)
                        .map_err(|f| f.to_string())?;
                    out.push(e.facts().map_err(|e| e.to_string())?);
                }
                WireEntry::Sealed(k) => {
                    let f = e.facts().map_err(|e| e.to_string())?;
                    let pos = head.position.next();
                    if f.position != pos.get() || k.header.object != self.object.as_str() {
                        return Err(format!("entry {}: malformed", pos.get()));
                    }
                    if f.prev != head.hash {
                        return Err(format!("entry {}: hash-mismatch", pos.get()));
                    }
                    if f.epoch < head.epoch {
                        return Err(format!("entry {}: epoch-regression", pos.get()));
                    }
                    let Some(pk) = keys.host_keys(&self.object, f.epoch, &f.writer) else {
                        return Err(format!("entry {}: not-a-host", pos.get()));
                    };
                    if !verify_keeper_authorship(k, &pk) {
                        return Err(format!("entry {}: bad-signature", pos.get()));
                    }
                    let sealer_is_host =
                        view.config().is_host(&k.sealer) || batch_config.is_host(&k.sealer);
                    let sealed_ok = sealer_is_host
                        && self
                            .directory
                            .verification_keys(&k.sealer)
                            .is_some_and(|sk| verify_keeper_seal(k, &sk));
                    if !sealed_ok {
                        return Err(format!("entry {}: bad-seal", pos.get()));
                    }
                    head = ChainHead {
                        position: pos,
                        hash: Some(f.hash),
                        epoch: f.epoch,
                    };
                    out.push(f);
                }
            }
        }
        Ok(out)
    }

    fn signed_by_writer(&self, entry: &WireEntry, writer: &str) -> bool {
        let Some(keys) = self.directory.verification_keys(writer) else {
            return false;
        };
        match entry {
            WireEntry::Sealed(k) => verify_keeper_authorship(k, &keys),
            WireEntry::Clear { nquads } => entry_from_nquads(nquads)
                .ok()
                .and_then(|e| crate::chain::read_link(&e).ok().map(|l| (e, l)))
                .is_some_and(|(e, l)| {
                    crate::chain::content_hash(&e) == l.hash
                        && keys
                            .iter()
                            .any(|k| k.verify(&crate::chain::signing_input(&l.hash), &l.signature))
                }),
        }
    }

    // @tg: implements:: [[openspec:object-custody#Journal Replication Over TSP]] {confidence: partial, note: "the custodian side, transport-neutral"}
    // @tg: implements:: [[openspec:object-custody#Equivocation Evidence]]
    // @tg: implements:: [[openspec:durable-objects#Single Writer Per Object]]
    fn append(&self, batch: &AppendBatch) -> CustodyResponse {
        let mut st = self.lock();
        if st.refused.contains(&batch.epoch) {
            return CustodyResponse::Refused {
                highest: st.promised,
                reason: format!("epoch {} is refused after equivocation", batch.epoch),
            };
        }
        if batch.epoch < st.promised {
            return CustodyResponse::Refused {
                highest: st.promised,
                reason: "a higher epoch was promised".into(),
            };
        }
        if batch.config.version < st.config().version {
            return CustodyResponse::Refused {
                highest: st.promised,
                reason: format!(
                    "configuration {} is older than {}",
                    batch.config.version,
                    st.config().version
                ),
            };
        }
        if batch.config.validate().is_err() {
            return CustodyResponse::Failed {
                reason: "invalid configuration".into(),
            };
        }
        let writer_known =
            st.config().is_host(&batch.writer) || batch.config.is_host(&batch.writer);
        let verified = writer_known
            && self
                .directory
                .verification_keys(&batch.writer)
                .is_some_and(|k| batch.verify(&k));
        if !verified {
            return CustodyResponse::Failed {
                reason: format!("batch not signed by a host ({})", batch.writer),
            };
        }
        let len = st.head();
        if batch.after > len {
            return CustodyResponse::Behind { head: len };
        }
        let incoming: Vec<WireFacts> = match batch
            .entries
            .iter()
            .map(WireEntry::facts)
            .collect::<Result<_, _>>()
        {
            Ok(f) => f,
            Err(e) => {
                return CustodyResponse::Failed {
                    reason: e.to_string(),
                };
            }
        };
        let newer = st.accepted_epoch < batch.epoch;
        let batch_end = batch.after + batch.entries.len() as u64;
        // Find where the batch departs from what the replica holds.
        let mut base = len.min(batch_end);
        for (i, f) in incoming.iter().enumerate() {
            let pos = batch.after + 1 + i as u64;
            if pos > len {
                base = pos - 1;
                break;
            }
            let held = &st.facts[pos as usize - 1];
            if held.hash == f.hash {
                continue;
            }
            if newer {
                base = pos - 1;
                break;
            }
            // A different entry from the same epoch we accepted in.
            let first = st.entries[pos as usize - 1].clone();
            let second = batch.entries[i].clone();
            let equivocation = held.epoch == f.epoch
                && held.writer == f.writer
                && self.signed_by_writer(&first, &held.writer)
                && self.signed_by_writer(&second, &f.writer);
            if equivocation {
                let evidence = Evidence {
                    object: self.object.as_str().to_owned(),
                    epoch: f.epoch,
                    position: pos,
                    writer: f.writer.clone(),
                    first,
                    second,
                };
                if self
                    .write(
                        &mut st,
                        LogRecord::Evidence {
                            evidence: evidence.clone(),
                        },
                    )
                    .is_ok()
                {
                    let sink = self
                        .sink
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    drop(st);
                    if let Some(s) = sink {
                        s.report(&evidence);
                    }
                }
                return CustodyResponse::Diverged {
                    position: pos,
                    evidence: true,
                };
            }
            return CustodyResponse::Diverged {
                position: pos,
                evidence: false,
            };
        }
        // Only a newer epoch's batch replaces a tail (it ends the journal at that writer's
        // head); a batch of the epoch already accepted only extends the journal, so a delayed,
        // shorter batch never drops entries.
        let base = if newer { base } else { base.max(len) };
        let replace = newer && len > base;
        let skip = (base - batch.after) as usize;
        let new: Vec<WireEntry> = batch.entries.iter().skip(skip).cloned().collect();
        let role = st.config().role_of(self.custodian());
        // A keeper must only ever hold sealed entries, a host clear ones.
        if role == Some(Role::Keeper) && new.iter().any(|e| matches!(e, WireEntry::Clear { .. })) {
            return CustodyResponse::Failed {
                reason: "a keeper stores encrypted entries only".into(),
            };
        }
        if let Err(reason) = self.verify_new(&st, base, &new, &batch.config) {
            return CustodyResponse::Failed { reason };
        }
        let config = (batch.config.version > st.config().version).then(|| batch.config.clone());
        let keys: Vec<KeyWrap> = batch
            .keys
            .iter()
            .filter(|k| !st.keys.contains(k))
            .cloned()
            .collect();
        let changes = !new.is_empty()
            || replace
            || config.is_some()
            || !keys.is_empty()
            || newer
            || batch.epoch > st.promised;
        if changes {
            let rec = LogRecord::Accepted {
                epoch: batch.epoch,
                base,
                entries: new,
                config,
                keys,
            };
            if let Err(e) = self.write(&mut st, rec) {
                return CustodyResponse::Failed {
                    reason: e.to_string(),
                };
            }
        }
        CustodyResponse::Ack {
            epoch: batch.epoch,
            head: st.head(),
        }
    }
}
