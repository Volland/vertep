//! The host side of custody: winning an epoch from a quorum of custodians, adopting the
//! committed chain, writing turns that count only once a quorum has accepted them, handing
//! an object over, and changing its custodian set.
//!
//! [`CustodyHost`] serves one object; [`CustodyStore`] is the journal store a runtime node
//! uses, routing durable objects to their custody hosts and every other actor to an ordinary
//! store. The runtime appends a turn and performs its effects only after the append returns,
//! so a turn whose entry no quorum accepted never has effects.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use oxrdf::{Graph, GraphName, Literal, NamedNode, Quad, Triple, vocab::xsd};
use serde::{Deserialize, Serialize};
use vertep_cluster::{ShipOutcome, ship_suffix};
use vertep_federation::{KeyManager, PublicKey};
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphStore, JournalCapabilities, JournalRecord, JournalStore,
    Position, StoreError, vocab as store_vocab,
};

use super::config::{CustodyConfig, Role};
use super::link::{CustodianLink, CustodyTransport, WriterBatch};
use super::replica::Replica;
use super::wire::{
    AppendBatch, ContentKey, CustodyRequest, CustodyResponse, EpochPromise, KeyDirectory, KeyWrap,
    WireEntry, WitnessRequest, entry_from_nquads, open_keeper_entry, record_rank, signing_bytes,
};
use super::witness::WitnessSource;
use crate::chain::{
    ChainHead, ChainLink, Digest, EntrySigner, HostKeys, content_hash, read_link, seal,
    verify_chain,
};
use crate::identity::DurableObjectId;
use crate::vocab;

/// Why a custody operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustodyError {
    /// Fewer custodians promised than the quorum.
    NoQuorum {
        /// Custodians that promised.
        promised: usize,
        /// The quorum.
        needed: usize,
        /// The highest epoch a custodian reported having promised.
        highest: u64,
    },
    /// This host lost the object to a higher epoch, or handed it over; it does not elect again
    /// until resumed.
    Deposed {
        /// The epoch that superseded it.
        highest: u64,
    },
    /// This host is not the object's writer.
    NotWriter,
    /// No quorum of custodians holds a chain that verifies.
    Corrupt(String),
    /// A request or configuration is invalid or not signed by whom it must be.
    Invalid(String),
    /// The custodians did not accept a change.
    NotAccepted(String),
    /// A local store or key failure.
    Store(String),
}

impl fmt::Display for CustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CustodyError::NoQuorum {
                promised,
                needed,
                highest,
            } => write!(
                f,
                "no custody quorum: {promised} of {needed} custodians promised (highest epoch seen {highest})"
            ),
            CustodyError::Deposed { highest } => {
                write!(f, "deposed by custody epoch {highest}")
            }
            CustodyError::NotWriter => write!(f, "not the object's writer"),
            CustodyError::Corrupt(m) => write!(f, "no verifiable chain: {m}"),
            CustodyError::Invalid(m) => write!(f, "invalid custody request: {m}"),
            CustodyError::NotAccepted(m) => write!(f, "custody change not accepted: {m}"),
            CustodyError::Store(m) => write!(f, "custody store: {m}"),
        }
    }
}

impl std::error::Error for CustodyError {}

/// How a candidate waits between elections it lost to a race.
#[derive(Clone)]
pub struct Backoff {
    /// Elections tried before giving up.
    pub attempts: u32,
    /// The first delay; each retry doubles it, with up to 50% random jitter.
    pub base: Duration,
    /// The longest delay.
    pub max: Duration,
    /// Waits (a thread sleep by default; tests pass a no-op).
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
}

impl fmt::Debug for Backoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backoff")
            .field("attempts", &self.attempts)
            .field("base", &self.base)
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff {
            attempts: 5,
            base: Duration::from_millis(20),
            max: Duration::from_secs(2),
            sleep: Arc::new(std::thread::sleep),
        }
    }
}

impl Backoff {
    /// No waiting (tests).
    pub fn immediate(attempts: u32) -> Self {
        Backoff {
            attempts,
            base: Duration::ZERO,
            max: Duration::ZERO,
            sleep: Arc::new(|_| {}),
        }
    }

    fn delay(&self, attempt: u32) -> Duration {
        let exp = self
            .base
            .saturating_mul(1u32 << attempt.min(16))
            .min(self.max);
        let mut b = [0u8; 2];
        let _ = getrandom::fill(&mut b);
        let jitter = u64::from(u16::from_le_bytes(b)) % 500; // 0..50.0 %
        exp + exp.mul_f64(jitter as f64 / 1000.0)
    }
}

/// Publishes a durable object's DID document update (a signed `did:webvh` log entry) to the
/// web host serving the log.
pub trait DocumentPublisher: Send + Sync {
    /// Publishes `log_entry` for `object`, collecting the witness proofs it needs from
    /// `witnesses` (the object's custodians).
    ///
    /// # Errors
    ///
    /// Why the entry could not be witnessed, or the web host refused or could not be reached.
    fn publish(
        &self,
        object: &str,
        log_entry: &serde_json::Value,
        witnesses: &dyn WitnessSource,
    ) -> Result<(), String>;
}

/// A publisher that records what it was given (tests, and deployments that publish out of
/// band).
#[derive(Debug, Default)]
pub struct RecordingPublisher {
    published: Mutex<Vec<(String, serde_json::Value)>>,
}

impl RecordingPublisher {
    /// What was published.
    pub fn published(&self) -> Vec<(String, serde_json::Value)> {
        self.published
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl DocumentPublisher for RecordingPublisher {
    fn publish(
        &self,
        object: &str,
        log_entry: &serde_json::Value,
        _witnesses: &dyn WitnessSource,
    ) -> Result<(), String> {
        self.published
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((object.to_owned(), log_entry.clone()));
        Ok(())
    }
}

/// A controller-signed request to change an object's custodian set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconfigurationRequest {
    /// The object.
    pub object: String,
    /// The new configuration; its version is the current one plus one.
    pub config: CustodyConfig,
    /// The `did:webvh` log entry that publishes the new set, signed by the controller.
    pub did_log_entry: Option<serde_json::Value>,
    /// The controller's signature over the other fields.
    pub signature: String,
}

impl ReconfigurationRequest {
    fn unsigned(&self) -> Self {
        ReconfigurationRequest {
            signature: String::new(),
            ..self.clone()
        }
    }

    /// Signs the request as the controller.
    ///
    /// # Errors
    ///
    /// The signer's error.
    pub fn sign(
        mut self,
        controller: &dyn EntrySigner,
    ) -> Result<Self, vertep_federation::KeyError> {
        use base64::Engine as _;
        let sig = controller.sign(&signing_bytes(
            "vertep-custody-reconfigure-v1",
            &self.unsigned(),
        ))?;
        self.signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
        Ok(self)
    }

    fn verify(&self, keys: &[PublicKey]) -> bool {
        use base64::Engine as _;
        let Ok(sig) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&self.signature)
        else {
            return false;
        };
        let input = signing_bytes("vertep-custody-reconfigure-v1", &self.unsigned());
        keys.iter().any(|k| k.verify(&input, &sig))
    }
}

impl WitnessSource for CustodyHost {
    /// Asks this host's replica and every custodian it has a transport to.
    // @tg: implements:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries]] {confidence: partial, note: "witness requests over the custody transports"}
    fn request_witnesses(
        &self,
        request: &WitnessRequest,
    ) -> Vec<(String, Result<serde_json::Value, String>)> {
        let answer = |r: CustodyResponse| match r {
            CustodyResponse::Witnessed { proof, .. } => Ok(proof),
            CustodyResponse::Refused { reason, .. } | CustodyResponse::Failed { reason } => {
                Err(reason)
            }
            other => Err(format!("unexpected answer {other:?}")),
        };
        let wrapped = CustodyRequest::Witness(request.clone());
        let mut out = vec![(self.host().to_owned(), answer(self.local.handle(&wrapped)))];
        let transports: Vec<Arc<dyn CustodyTransport>> = self
            .transports
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for t in transports {
            if t.custodian() == self.host() {
                continue;
            }
            let r = t.call(&wrapped).map_err(|e| e.to_string()).and_then(answer);
            out.push((t.custodian().to_owned(), r));
        }
        out
    }
}

/// One epoch this host won: the promises that made it writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochRecord {
    /// The epoch.
    pub epoch: u64,
    /// The promises received.
    pub promises: Vec<EpochPromise>,
    /// The configuration it was won under.
    pub config: CustodyConfig,
}

/// The outcome of an election this host won.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elected {
    /// The epoch.
    pub epoch: u64,
    /// The adopted chain's head.
    pub head: Position,
    /// Whether a quorum has already accepted the adopted chain in this epoch.
    pub committed: bool,
    /// Custodians whose replica failed verification and was skipped.
    pub skipped: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Idle,
    Writer { epoch: u64 },
    Deposed { highest: u64 },
}

struct WriterState {
    mode: Mode,
    committed: Position,
    behind: bool,
    config: CustodyConfig,
    pending: Option<CustodyConfig>,
    content_keys: BTreeMap<u64, ContentKey>,
    current_key: Option<u64>,
    wraps: Vec<KeyWrap>,
    acked: BTreeMap<String, Position>,
    epochs: Vec<EpochRecord>,
    highest_seen: u64,
}

struct ConfigsHostKeys<'a> {
    configs: &'a [(u64, CustodyConfig)],
    directory: &'a dyn KeyDirectory,
}

impl HostKeys for ConfigsHostKeys<'_> {
    fn host_keys(&self, _object: &NamedNode, epoch: u64, writer: &str) -> Option<Vec<PublicKey>> {
        let cfg = self
            .configs
            .iter()
            .rev()
            .find(|(from, _)| *from <= epoch)
            .or_else(|| self.configs.first())
            .map(|(_, c)| c)?;
        if !cfg.is_host(writer) {
            return None;
        }
        self.directory.verification_keys(writer)
    }
}

/// A configuration as recorded in custody histories.
pub(crate) fn config_json(c: &CustodyConfig) -> serde_json::Value {
    serde_json::json!({
        "version": c.version,
        "custodians": c.custodians.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
        "quorum": c.quorum,
    })
}

/// An adopted chain and the configurations it was verified under.
type Adopted = (Vec<Entry>, Vec<(u64, CustodyConfig)>);

/// A host's custody of one durable object.
pub struct CustodyHost {
    object: DurableObjectId,
    signer: Arc<dyn EntrySigner>,
    keys: Arc<dyn KeyManager>,
    agreement_key: String,
    local: Arc<Replica>,
    transports: RwLock<BTreeMap<String, Arc<dyn CustodyTransport>>>,
    directory: Arc<dyn KeyDirectory>,
    publisher: RwLock<Option<Arc<dyn DocumentPublisher>>>,
    backoff: Backoff,
    standby: std::sync::atomic::AtomicBool,
    history: RwLock<Option<Arc<super::history::History>>>,
    state: Mutex<WriterState>,
}

impl fmt::Debug for CustodyHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustodyHost")
            .field("object", &self.object.as_str())
            .field("host", &self.signer.writer())
            .finish_non_exhaustive()
    }
}

fn store_err(e: impl fmt::Display) -> CustodyError {
    CustodyError::Store(e.to_string())
}

impl CustodyHost {
    /// The custody of `object` by the host `signer`, whose own replica is `local`; content keys
    /// are unwrapped with the key-agreement key `agreement_key` of `keys`.
    pub fn new(
        object: DurableObjectId,
        signer: Arc<dyn EntrySigner>,
        keys: Arc<dyn KeyManager>,
        agreement_key: impl Into<String>,
        local: Arc<Replica>,
        directory: Arc<dyn KeyDirectory>,
    ) -> Self {
        let config = local.config();
        let pending = local.next();
        CustodyHost {
            object,
            signer,
            keys,
            agreement_key: agreement_key.into(),
            local,
            transports: RwLock::new(BTreeMap::new()),
            directory,
            publisher: RwLock::new(None),
            backoff: Backoff::default(),
            standby: std::sync::atomic::AtomicBool::new(false),
            history: RwLock::new(None),
            state: Mutex::new(WriterState {
                mode: Mode::Idle,
                committed: Position::ZERO,
                behind: false,
                config,
                pending,
                content_keys: BTreeMap::new(),
                current_key: None,
                wraps: Vec::new(),
                acked: BTreeMap::new(),
                epochs: Vec::new(),
                highest_seen: 0,
            }),
        }
    }

    /// Reaches the custodian `transport` leads to.
    pub fn with_transport(self, transport: Arc<dyn CustodyTransport>) -> Self {
        self.add_transport(transport);
        self
    }

    /// Adds or replaces the transport to a custodian.
    pub fn add_transport(&self, transport: Arc<dyn CustodyTransport>) {
        self.transports
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(transport.custodian().to_owned(), transport);
    }

    /// Waits as `backoff` says between lost elections.
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Publishes custodian-set changes with `publisher`.
    pub fn with_publisher(self, publisher: Arc<dyn DocumentPublisher>) -> Self {
        self.set_publisher(publisher);
        self
    }

    /// Sets or replaces the publisher of custodian-set changes.
    pub fn set_publisher(&self, publisher: Arc<dyn DocumentPublisher>) {
        *self
            .publisher
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(publisher);
    }

    /// Keeps this host from electing on first use: a standby host serves the object only
    /// while it is the writer (after an explicit [`CustodyHost::activate`], for a failover). Any
    /// host with the object's keys can open its messages, but a standby's journal appends fail,
    /// so it never acknowledges a pickup.
    pub fn set_standby(&self, standby: bool) {
        self.standby
            .store(standby, std::sync::atomic::Ordering::SeqCst);
    }

    /// Records this host's elections and commits in `history`.
    pub fn set_history(&self, history: Arc<super::history::History>) {
        *self
            .history
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(history);
    }

    fn record(&self, event: serde_json::Value) {
        if let Some(h) = self
            .history
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            h.record(event);
        }
    }

    fn record_commit(&self, epoch: u64, committed: Position) {
        let log: Vec<String> = self
            .local
            .entries()
            .iter()
            .take(committed.get() as usize)
            .filter_map(|e| e.facts().ok().map(|f| f.hash.to_hex()))
            .collect();
        let (config, next) = (self.local.config(), self.local.next());
        self.record(serde_json::json!({
            "kind": "committed", "host": self.host(), "epoch": epoch, "log": log,
            "config": config_json(&config), "next": next.as_ref().map(config_json),
        }));
    }

    /// Whether this host is a standby.
    pub fn is_standby(&self) -> bool {
        self.standby.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The object.
    pub fn object(&self) -> &DurableObjectId {
        &self.object
    }

    /// This host's DID.
    pub fn host(&self) -> &str {
        self.signer.writer()
    }

    /// The host's own replica.
    pub fn replica(&self) -> &Arc<Replica> {
        &self.local
    }

    fn lock(&self) -> MutexGuard<'_, WriterState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The epoch this host writes in, if it is the writer.
    pub fn writer_epoch(&self) -> Option<u64> {
        match self.lock().mode {
            Mode::Writer { epoch } => Some(epoch),
            _ => None,
        }
    }

    /// The committed head.
    pub fn committed(&self) -> Position {
        self.lock().committed
    }

    /// The configuration in force.
    pub fn config(&self) -> CustodyConfig {
        self.lock().config.clone()
    }

    /// The epochs this host won, with their promises.
    pub fn epochs(&self) -> Vec<EpochRecord> {
        self.lock().epochs.clone()
    }

    /// Recovers from a crash of the host process: everything the host kept in memory (writer
    /// role, commit point, shipping progress, unwrapped keys) is gone; its replica is durable.
    pub fn restart(&self) {
        let mut st = self.lock();
        st.mode = Mode::Idle;
        st.committed = Position::ZERO;
        st.behind = false;
        st.pending = self.local.next();
        st.content_keys.clear();
        st.current_key = None;
        st.wraps.clear();
        st.acked.clear();
        st.config = self.local.config();
    }

    /// Lets a deposed host elect again.
    pub fn resume(&self) {
        let mut st = self.lock();
        if matches!(st.mode, Mode::Deposed { .. }) {
            st.mode = Mode::Idle;
        }
    }

    fn transport(&self, did: &str) -> Option<Arc<dyn CustodyTransport>> {
        self.transports
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(did)
            .cloned()
    }

    fn call(&self, did: &str, req: &CustodyRequest) -> Option<CustodyResponse> {
        if did == self.host() {
            return Some(self.local.handle(req));
        }
        self.transport(did)?.call(req).ok()
    }

    fn content_key(&self, st: &mut WriterState, generation: u64) -> Option<ContentKey> {
        if let Some(k) = st.content_keys.get(&generation) {
            return Some(k.clone());
        }
        let wrap = st
            .wraps
            .iter()
            .find(|w| w.generation == generation && w.host == self.host())?
            .clone();
        let key = ContentKey::unwrap(
            self.object.as_str(),
            &wrap,
            self.keys.as_ref(),
            &self.agreement_key,
        )
        .ok()?;
        st.content_keys.insert(generation, key.clone());
        Some(key)
    }

    fn open_wire(&self, st: &mut WriterState, e: &WireEntry) -> Result<Entry, String> {
        match e {
            WireEntry::Clear { nquads } => entry_from_nquads(nquads).map_err(|e| e.to_string()),
            WireEntry::Sealed(k) => {
                let key = self.content_key(st, k.header.generation).ok_or_else(|| {
                    format!("no content key {} for this host", k.header.generation)
                })?;
                open_keeper_entry(k, &key).map_err(|e| e.to_string())
            }
        }
    }

    /// Wins epoch `epoch`: asks every custodian for a promise, adopts the configuration and
    /// chain accepted in the highest epoch (skipping replicas that do not verify, as long as a
    /// quorum of promises remains), stores it as its own replica and ships it to the
    /// custodians under `epoch`.
    ///
    /// # Errors
    ///
    /// [`CustodyError::NoQuorum`] when fewer than a quorum promise; [`CustodyError::Corrupt`]
    /// when no quorum of verifiable replicas remains; store and key failures.
    // @tg: implements:: [[openspec:object-custody#Quorum-Promised Custody Epochs]]
    // @tg: implements:: [[openspec:durable-objects#Activation From A Verified Replica]]
    // @tg: implements:: [[openspec:object-custody#Failover Without A Shared Authority]]
    pub fn elect(&self, epoch: u64) -> Result<Elected, CustodyError> {
        let mut st = self.lock();
        let mut epoch = epoch;
        // The configuration record prepared with: the effective set and, while a change is in
        // progress, the pending one (joint consensus: quorums of both are needed).
        let mut config = st.config.clone();
        let mut next = st.pending.clone();
        let object = self.object.as_str().to_owned();
        let mut promises: Vec<EpochPromise>;
        let mut rounds = 0;
        loop {
            rounds += 1;
            promises = Vec::new();
            let mut highest = 0;
            let mut targets: Vec<String> = config
                .custodians
                .iter()
                .chain(next.iter().flat_map(|n| n.custodians.iter()))
                .map(|c| c.id.clone())
                .collect();
            targets.sort();
            targets.dedup();
            for c in &targets {
                let req = CustodyRequest::Prepare {
                    object: object.clone(),
                    epoch,
                    candidate: self.host().to_owned(),
                };
                match self.call(c, &req) {
                    Some(CustodyResponse::Promise(p))
                        if p.epoch == epoch
                            && &p.custodian == c
                            && self
                                .directory
                                .verification_keys(c)
                                .is_some_and(|k| p.verify(&k)) =>
                    {
                        promises.push(p);
                    }
                    Some(CustodyResponse::Refused { highest: h, .. }) => highest = highest.max(h),
                    _ => {}
                }
            }
            st.highest_seen = st.highest_seen.max(highest).max(epoch);
            // A newer configuration record reported by a custodian: prepare again with it.
            if let Some(p) = promises
                .iter()
                .max_by_key(|p| record_rank(&p.config, p.next.as_ref()))
                && record_rank(&p.config, p.next.as_ref()) > record_rank(&config, next.as_ref())
                && rounds < 4
            {
                config = p.config.clone();
                next = p.next.clone();
                epoch = st.highest_seen + 1;
                continue;
            }
            let ids = || promises.iter().map(|p| p.custodian.as_str());
            if !(config.is_quorum(ids()) && next.as_ref().is_none_or(|n| n.is_quorum(ids()))) {
                return Err(CustodyError::NoQuorum {
                    promised: promises.len(),
                    needed: config.quorum,
                    highest,
                });
            }
            break;
        }
        let is_quorum = |ids: &mut dyn Iterator<Item = &str>| {
            let ids: Vec<&str> = ids.collect();
            config.is_quorum(ids.iter().copied())
                && next
                    .as_ref()
                    .is_none_or(|n| n.is_quorum(ids.iter().copied()))
        };
        // Candidates in adoption order: highest accepted epoch, then longest.
        let mut order: Vec<&EpochPromise> = promises.iter().collect();
        order.sort_by_key(|p| std::cmp::Reverse((p.accepted_epoch, p.head)));
        let mut excluded: BTreeSet<String> = BTreeSet::new();
        let mut skipped = Vec::new();
        let mut adopted: Option<Adopted> = None;
        for p in order {
            let mut remaining = promises
                .iter()
                .filter(|q| !excluded.contains(&q.custodian))
                .map(|q| q.custodian.as_str());
            if !is_quorum(&mut remaining) {
                break;
            }
            let read = CustodyRequest::Read {
                object: object.clone(),
                from: 1,
            };
            let Some(CustodyResponse::Entries {
                entries,
                keys,
                configs,
                ..
            }) = self.call(&p.custodian, &read)
            else {
                excluded.insert(p.custodian.clone());
                skipped.push((p.custodian.clone(), "unreachable".into()));
                continue;
            };
            for k in keys {
                if !st.wraps.contains(&k) {
                    st.wraps.push(k);
                }
            }
            let opened: Result<Vec<Entry>, String> =
                entries.iter().map(|e| self.open_wire(&mut st, e)).collect();
            let verdict = opened.and_then(|chain| {
                let records: Vec<JournalRecord> = chain
                    .iter()
                    .enumerate()
                    .map(|(i, e)| JournalRecord {
                        position: Position::new(i as u64 + 1),
                        entry: e.clone(),
                    })
                    .collect();
                let hk = ConfigsHostKeys {
                    configs: &configs,
                    directory: self.directory.as_ref(),
                };
                let head = verify_chain(&self.object.iri(), ChainHead::EMPTY, &records, &hk)
                    .map_err(|f| f.to_string())?;
                if head.position.get() < p.head || head.hash != p.head_digest() {
                    return Err(format!(
                        "replica ends at {} but promised head {}",
                        head.position.get(),
                        p.head
                    ));
                }
                Ok(chain)
            });
            match verdict {
                Ok(chain) => {
                    adopted = Some((chain, configs));
                    break;
                }
                Err(reason) => {
                    excluded.insert(p.custodian.clone());
                    skipped.push((p.custodian.clone(), reason));
                }
            }
        }
        let Some((chain, _configs)) = adopted else {
            return Err(CustodyError::Corrupt(format!(
                "skipped replicas: {skipped:?}"
            )));
        };
        // Content keys: unwrap what is wrapped to this host, or start generation 1.
        for w in self.local.keys() {
            if !st.wraps.contains(&w) {
                st.wraps.push(w);
            }
        }
        let latest = st
            .wraps
            .iter()
            .filter(|w| w.host == self.host())
            .map(|w| w.generation)
            .max();
        match latest {
            Some(g) => {
                if self.content_key(&mut st, g).is_none() {
                    return Err(CustodyError::Store(format!(
                        "content key generation {g} does not open"
                    )));
                }
                st.current_key = Some(g);
            }
            None => {
                let g = st.wraps.iter().map(|w| w.generation).max().unwrap_or(0) + 1;
                let key = ContentKey::generate(g).map_err(store_err)?;
                let mut hosts: Vec<&str> = config
                    .hosts()
                    .chain(next.iter().flat_map(|n| n.hosts()))
                    .collect();
                hosts.sort_unstable();
                hosts.dedup();
                for h in hosts {
                    let agree = self.directory.agreement_key(h).ok_or_else(|| {
                        CustodyError::Invalid(format!("no key-agreement key for host {h}"))
                    })?;
                    let w = key
                        .wrap_for(self.object.as_str(), h, &agree)
                        .map_err(store_err)?;
                    st.wraps.push(w);
                }
                st.content_keys.insert(g, key);
                st.current_key = Some(g);
            }
        }
        // Store the adopted chain as this host's replica under the new epoch.
        let head = Position::new(chain.len() as u64);
        let batch = AppendBatch {
            object: object.clone(),
            epoch,
            writer: self.host().to_owned(),
            after: 0,
            entries: chain.iter().map(WireEntry::clear).collect(),
            config: config.clone(),
            next: next.clone(),
            keys: st.wraps.clone(),
            signature: String::new(),
        }
        .sign(self.signer.as_ref())
        .map_err(store_err)?;
        // The election is won here (a quorum promised); the own replica's accept follows.
        self.record(serde_json::json!({
            "kind": "elected", "host": self.host(), "epoch": epoch,
            "promised_by": promises.iter().map(|p| p.custodian.as_str()).collect::<Vec<_>>(),
            "adopted": chain.iter().map(|e| content_hash(e).to_hex()).collect::<Vec<_>>(),
            "config": config_json(&config), "next": next.as_ref().map(config_json),
        }));
        match self.local.handle(&CustodyRequest::Append(batch)) {
            CustodyResponse::Ack { .. } => {}
            CustodyResponse::Refused { highest, .. } => {
                st.highest_seen = st.highest_seen.max(highest);
                return Err(CustodyError::NoQuorum {
                    promised: promises.len(),
                    needed: config.quorum,
                    highest,
                });
            }
            other => return Err(CustodyError::Store(format!("own replica: {other:?}"))),
        }
        st.mode = Mode::Writer { epoch };
        st.config = config.clone();
        st.pending = next.clone();
        st.committed = Position::ZERO;
        st.behind = true;
        // Start shipping each custodian from the head it promised when that is a prefix of
        // the adopted chain.
        st.acked.clear();
        for p in &promises {
            let at = p.head as usize;
            let prefix = at <= chain.len()
                && (at == 0 || Some(content_hash(&chain[at - 1])) == p.head_digest());
            if prefix && p.accepted_epoch > 0 {
                st.acked.insert(p.custodian.clone(), Position::new(p.head));
            }
        }
        st.epochs.push(EpochRecord {
            epoch,
            promises: promises.clone(),
            config,
        });
        let committed = self.catch_up_locked(&mut st)?;
        Ok(Elected {
            epoch,
            head,
            committed,
            skipped,
        })
    }

    /// Becomes the writer if it is not: elects with an epoch above every epoch seen, retrying
    /// with back-off when another candidate wins a race.
    ///
    /// # Errors
    ///
    /// [`CustodyError::Deposed`] for a host that lost the object until [`CustodyHost::resume`];
    /// the last election's error after the back-off's attempts.
    pub fn activate(&self) -> Result<u64, CustodyError> {
        let mut last = CustodyError::NotWriter;
        for attempt in 0..self.backoff.attempts.max(1) {
            let next = {
                let st = self.lock();
                match st.mode {
                    Mode::Writer { epoch } => return Ok(epoch),
                    Mode::Deposed { highest } => return Err(CustodyError::Deposed { highest }),
                    Mode::Idle => {}
                }
                st.highest_seen
                    .max(self.local.promised())
                    .max(self.local.accepted_epoch())
                    + 1
            };
            match self.elect(next) {
                Ok(e) => return Ok(e.epoch),
                Err(e @ (CustodyError::NoQuorum { .. } | CustodyError::Corrupt(_))) => {
                    last = e;
                    (self.backoff.sleep)(self.backoff.delay(attempt));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Ships every custodian what it lacks under the writer's epoch. Returns the custodians
    /// that hold the whole journal and the highest epoch a custodian was fenced by.
    fn ship(&self, st: &mut WriterState, epoch: u64) -> (BTreeSet<String>, Option<u64>) {
        let head = self.local.head();
        let mut acks = BTreeSet::new();
        let mut fenced = None;
        // The own replica counts only while it has promised nothing higher: a candidate for a
        // later epoch may have won its promise, and then this writer is deposed.
        let own = self.local.promised();
        if own > epoch {
            return (acks, Some(own));
        }
        let config = st.config.clone();
        let next = st.pending.clone();
        // The own replica takes the writer's configuration record like any custodian (an empty
        // batch at its head) and counts only once it holds that record in this epoch: a joint
        // quorum needs every counted member to know the change.
        if self.local.config() != config || self.local.next() != next {
            let sync = AppendBatch {
                object: self.object.as_str().to_owned(),
                epoch,
                writer: self.host().to_owned(),
                after: head.get(),
                entries: Vec::new(),
                config: config.clone(),
                next: next.clone(),
                keys: st.wraps.clone(),
                signature: String::new(),
            }
            .sign(self.signer.as_ref());
            if let Ok(batch) = sync
                && let CustodyResponse::Refused { highest, .. } =
                    self.local.handle(&CustodyRequest::Append(batch))
                && highest > epoch
            {
                return (acks, Some(highest));
            }
        }
        if self.local.accepted_epoch() == epoch
            && self.local.config() == config
            && self.local.next() == next
        {
            acks.insert(self.host().to_owned());
        }
        let mut targets: Vec<(String, Role)> = config
            .custodians
            .iter()
            .chain(next.iter().flat_map(|n| n.custodians.iter()))
            .map(|c| (c.id.clone(), c.role))
            .collect();
        targets.sort();
        targets.dedup_by(|a, b| a.0 == b.0);
        let key = st
            .current_key
            .and_then(|g| st.content_keys.get(&g).cloned());
        let local_entries = self.local.entries();
        for (did, role) in targets {
            if did == self.host() {
                continue;
            }
            let Some(transport) = self.transport(&did) else {
                continue;
            };
            let link = CustodianLink {
                object: self.object.as_str().to_owned(),
                role,
                transport,
                signer: self.signer.clone(),
                content_key: key.clone(),
            };
            let from = st.acked.get(&did).copied().unwrap_or(Position::ZERO);
            let outcome = ship_suffix(
                &link,
                from,
                |p| {
                    local_entries
                        .iter()
                        .skip(p.get().saturating_sub(1) as usize)
                        .map(|e| match e {
                            WireEntry::Clear { nquads } => entry_from_nquads(nquads),
                            WireEntry::Sealed(_) => Err(super::wire::WireError(
                                "a host's replica holds clear entries".into(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()
                },
                |after, entries| WriterBatch {
                    epoch,
                    after,
                    entries,
                    config: config.clone(),
                    next: next.clone(),
                    keys: st.wraps.clone(),
                },
            );
            match outcome {
                Ok(ShipOutcome::Stored(p)) => {
                    st.acked.insert(did.clone(), p);
                    if p >= head {
                        acks.insert(did);
                    }
                }
                Ok(ShipOutcome::Fenced(h)) => fenced = Some(fenced.map_or(h, |x: u64| x.max(h))),
                Ok(ShipOutcome::Failed(_)) | Err(_) => {}
            }
        }
        (acks, fenced)
    }

    fn quorum_reached(st: &WriterState, acks: &BTreeSet<String>) -> bool {
        let ids = || acks.iter().map(String::as_str);
        st.config.is_quorum(ids()) && st.pending.as_ref().is_none_or(|p| p.is_quorum(ids()))
    }

    /// Ships what the custodians lack; once a quorum holds the whole local journal in the
    /// writer's epoch, it is committed. Returns whether it is.
    fn catch_up_locked(&self, st: &mut WriterState) -> Result<bool, CustodyError> {
        let Mode::Writer { epoch } = st.mode else {
            return Err(CustodyError::NotWriter);
        };
        let (acks, fenced) = self.ship(st, epoch);
        if let Some(h) = fenced {
            st.mode = Mode::Deposed { highest: h };
            st.highest_seen = st.highest_seen.max(h);
            return Err(CustodyError::Deposed { highest: h });
        }
        if Self::quorum_reached(st, &acks) {
            st.committed = self.local.head();
            st.behind = false;
            self.record_commit(epoch, st.committed);
            Ok(true)
        } else {
            st.behind = true;
            Ok(false)
        }
    }

    /// Brings the custodians level with the writer; true when the whole journal is committed.
    ///
    /// # Errors
    ///
    /// [`CustodyError::NotWriter`] or [`CustodyError::Deposed`].
    pub fn catch_up(&self) -> Result<bool, CustodyError> {
        let mut st = self.lock();
        self.catch_up_locked(&mut st)
    }

    /// Appends a runtime turn: seals `entries` at `expected + 1 ..` under the writer's epoch,
    /// stores them in the own replica, ships them, and succeeds only when a quorum has
    /// accepted them.
    ///
    /// # Errors
    ///
    /// [`AppendError::Fenced`] when a custodian has promised a higher epoch (the host is then
    /// deposed); [`AppendError::Conflict`] when `expected` is not the committed head;
    /// [`AppendError::Storage`] with [`StoreError::Unavailable`] when no quorum accepted — the
    /// entries stay unreadable until a later catch-up reaches a quorum.
    // @tg: implements:: [[openspec:durable-objects#Effects After Durable Commit]]
    // @tg: implements:: [[openspec:object-custody#Quorum Before Effects]]
    // @tg: implements:: [[openspec:durable-objects#Single Writer Per Object]]
    pub fn append_turn(
        &self,
        expected: Position,
        entries: &[Entry],
    ) -> Result<Position, AppendError> {
        let mut st = self.lock();
        let epoch = match st.mode {
            Mode::Writer { epoch } => epoch,
            Mode::Deposed { highest } => {
                return Err(AppendError::Fenced {
                    scope: self.object.iri(),
                    epoch: 0,
                    highest,
                });
            }
            Mode::Idle => {
                return Err(AppendError::Storage(StoreError::Unavailable(
                    "not the object's writer".into(),
                )));
            }
        };
        let fenced = |st: &mut WriterState, h: u64| {
            st.mode = Mode::Deposed { highest: h };
            AppendError::Fenced {
                scope: self.object.iri(),
                epoch,
                highest: h,
            }
        };
        if st.behind {
            match self.catch_up_locked(&mut st) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(AppendError::Storage(StoreError::Unavailable(
                        "the object's journal is not yet accepted by a custody quorum".into(),
                    )));
                }
                Err(CustodyError::Deposed { highest }) => return Err(fenced(&mut st, highest)),
                Err(e) => return Err(AppendError::Storage(StoreError::Unavailable(e.to_string()))),
            }
        }
        if expected != st.committed {
            return Err(AppendError::Conflict {
                expected,
                actual: st.committed,
            });
        }
        if entries.is_empty() {
            return Err(AppendError::Invalid(
                "an append has at least one entry".into(),
            ));
        }
        let mut prev: Option<Digest> = self
            .local
            .entries()
            .last()
            .and_then(|e| e.facts().ok())
            .map(|f| f.hash);
        let mut sealed = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let link = ChainLink {
                object: self.object.iri(),
                position: expected.advance(i as u64 + 1),
                epoch,
                writer: self.host().to_owned(),
                prev,
            };
            let s = seal(e, &link, self.signer.as_ref())
                .map_err(|e| AppendError::Invalid(e.to_string()))?;
            prev = Some(content_hash(&s));
            sealed.push(s);
        }
        let batch = AppendBatch {
            object: self.object.as_str().to_owned(),
            epoch,
            writer: self.host().to_owned(),
            after: expected.get(),
            entries: sealed.iter().map(WireEntry::clear).collect(),
            config: st.config.clone(),
            next: st.pending.clone(),
            keys: st.wraps.clone(),
            signature: String::new(),
        }
        .sign(self.signer.as_ref())
        .map_err(|e| AppendError::Storage(StoreError::Backend(e.to_string())))?;
        match self.local.handle(&CustodyRequest::Append(batch)) {
            CustodyResponse::Ack { .. } => {}
            CustodyResponse::Refused { highest, .. } => return Err(fenced(&mut st, highest)),
            other => {
                return Err(AppendError::Storage(StoreError::Backend(format!(
                    "own replica: {other:?}"
                ))));
            }
        }
        let (acks, fence) = self.ship(&mut st, epoch);
        if let Some(h) = fence {
            return Err(fenced(&mut st, h));
        }
        if Self::quorum_reached(&st, &acks) {
            st.committed = self.local.head();
            self.record_commit(epoch, st.committed);
            Ok(st.committed)
        } else {
            st.behind = true;
            Err(AppendError::Storage(StoreError::Unavailable(format!(
                "custody quorum not reached: {} of {} custodians accepted the entry",
                acks.len(),
                st.config.quorum
            ))))
        }
    }

    /// The committed entries from position `from` (inclusive; 0 reads from the start), with
    /// the store's stamps.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unavailable`] when this host is not the writer or the journal is not yet
    /// accepted by a quorum.
    pub fn read_committed(&self, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        let mut st = self.lock();
        if !matches!(st.mode, Mode::Writer { .. }) {
            return Err(StoreError::Unavailable(format!(
                "{} is not the writer of {}",
                self.host(),
                self.object
            )));
        }
        if st.behind && !self.catch_up_locked(&mut st).unwrap_or(false) {
            return Err(StoreError::Unavailable(
                "the object's journal is not yet accepted by a custody quorum".into(),
            ));
        }
        let committed = st.committed.get() as usize;
        let from = from.get().max(1) as usize;
        self.local
            .entries()
            .iter()
            .take(committed)
            .enumerate()
            .skip(from - 1)
            .map(|(i, e)| {
                let pos = Position::new(i as u64 + 1);
                let entry = match e {
                    WireEntry::Clear { nquads } => {
                        entry_from_nquads(nquads).map_err(|e| StoreError::invalid(e.to_string()))?
                    }
                    WireEntry::Sealed(_) => {
                        return Err(StoreError::invalid("a host holds clear entries"));
                    }
                };
                Ok(JournalRecord {
                    position: pos,
                    entry: entry.stamp(&self.object.iri(), pos)?,
                })
            })
            .collect()
    }

    /// Hands the object over: brings the custodians level, then stops writing, so the next
    /// host to elect (with a higher epoch) adopts every committed entry and nothing else.
    /// Returns the committed head.
    ///
    /// # Errors
    ///
    /// [`CustodyError::NotWriter`]; [`CustodyError::NotAccepted`] when the journal cannot be
    /// committed first.
    // @tg: implements:: [[openspec:object-custody#Custody Handoff]]
    pub fn hand_off(&self) -> Result<Position, CustodyError> {
        let mut st = self.lock();
        let Mode::Writer { epoch } = st.mode else {
            return Err(CustodyError::NotWriter);
        };
        if !self.catch_up_locked(&mut st)? {
            return Err(CustodyError::NotAccepted(
                "the journal is not accepted by a quorum".into(),
            ));
        }
        st.mode = Mode::Deposed { highest: epoch };
        Ok(st.committed)
    }

    /// Changes the custodian set as the controller asked: checks the controller's signature,
    /// re-wraps content keys (a new generation when a host leaves), ships the journal and the
    /// new configuration to old and new custodians, and switches once a quorum of the old set
    /// and every new custodian accepted; then publishes the DID document update.
    ///
    /// # Errors
    ///
    /// [`CustodyError::Invalid`] for an unsigned or invalid request; [`CustodyError::NotWriter`];
    /// [`CustodyError::NotAccepted`] when the custodians did not all accept yet (retry with
    /// [`CustodyHost::catch_up`] and this method).
    // @tg: implements:: [[openspec:object-custody#Custodian Set Changes]]
    pub fn reconfigure(&self, request: &ReconfigurationRequest) -> Result<(), CustodyError> {
        if request.object != self.object.as_str() {
            return Err(CustodyError::Invalid("request for another object".into()));
        }
        let controller = self
            .directory
            .controller_keys(self.object.as_str())
            .unwrap_or_default();
        if !request.verify(&controller) {
            return Err(CustodyError::Invalid(
                "the reconfiguration is not signed by the object's controller".into(),
            ));
        }
        request
            .config
            .validate()
            .map_err(|e| CustodyError::Invalid(e.to_string()))?;
        let mut st = self.lock();
        let Mode::Writer { epoch } = st.mode else {
            return Err(CustodyError::NotWriter);
        };
        let new = request.config.clone();
        let starting = st.pending.is_none() && st.config.version + 1 == new.version;
        let joint = st.pending.as_ref() == Some(&new);
        let switched = st.pending.is_none() && st.config == new;
        if !(starting || joint || switched) {
            return Err(CustodyError::Invalid(format!(
                "configuration version {} does not follow {}",
                new.version, st.config.version
            )));
        }
        if !new.is_host(self.host()) {
            return Err(CustodyError::Invalid(
                "the writer must stay a host; hand the object off first".into(),
            ));
        }
        if starting {
            let old_hosts: BTreeSet<String> = st.config.hosts().map(str::to_owned).collect();
            let new_hosts: BTreeSet<String> = new.hosts().map(str::to_owned).collect();
            let removed = !old_hosts.is_subset(&new_hosts);
            let wrap_to = |st: &mut WriterState, key: &ContentKey, hosts: &BTreeSet<String>| {
                for h in hosts {
                    let agree = self.directory.agreement_key(h).ok_or_else(|| {
                        CustodyError::Invalid(format!("no key-agreement key for host {h}"))
                    })?;
                    let w = key
                        .wrap_for(self.object.as_str(), h, &agree)
                        .map_err(store_err)?;
                    st.wraps.push(w);
                }
                Ok::<_, CustodyError>(())
            };
            // Earlier generations go to new hosts, so they can read keeper replicas.
            let added: BTreeSet<String> = new_hosts.difference(&old_hosts).cloned().collect();
            let gens: Vec<u64> = st.content_keys.keys().copied().collect();
            for g in gens {
                if let Some(k) = st.content_keys.get(&g).cloned() {
                    wrap_to(&mut st, &k, &added)?;
                }
            }
            if removed {
                let g = st.content_keys.keys().max().copied().unwrap_or(0) + 1;
                let key = ContentKey::generate(g).map_err(store_err)?;
                wrap_to(&mut st, &key, &new_hosts)?;
                st.content_keys.insert(g, key);
                st.current_key = Some(g);
            }
            st.pending = Some(new.clone());
        }
        // Phase 1 (joint consensus): a quorum of the old set and a quorum of the new set accept
        // the whole journal with the new set pending; meanwhile every commit needs both.
        let head = self.local.head();
        if st.pending.is_some() {
            let (acks, fenced) = self.ship(&mut st, epoch);
            if let Some(h) = fenced {
                st.mode = Mode::Deposed { highest: h };
                return Err(CustodyError::Deposed { highest: h });
            }
            if !Self::quorum_reached(&st, &acks) {
                return Err(CustodyError::NotAccepted(format!(
                    "joint phase: {} custodians accepted; quorums of the old and the new set are needed",
                    acks.len()
                )));
            }
            st.committed = head;
            st.behind = false;
            self.record_commit(epoch, head);
            // Phase 2: the new set takes effect.
            st.config = new;
            st.pending = None;
        }
        // The new set accepts the switch; only then is the change published.
        let (acks, fenced) = self.ship(&mut st, epoch);
        if let Some(h) = fenced {
            st.mode = Mode::Deposed { highest: h };
            return Err(CustodyError::Deposed { highest: h });
        }
        if !st.config.is_quorum(acks.iter().map(String::as_str)) {
            st.behind = true;
            return Err(CustodyError::NotAccepted(format!(
                "the switch is accepted by {} custodians, not yet a quorum of the new set",
                acks.len()
            )));
        }
        st.committed = head;
        st.behind = false;
        self.record_commit(epoch, head);
        drop(st);
        if let Some(entry) = &request.did_log_entry {
            let publisher = self
                .publisher
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(p) = publisher {
                p.publish(self.object.as_str(), entry, self)
                    .map_err(CustodyError::NotAccepted)?;
            }
        }
        Ok(())
    }

    /// The custody lineage of the object as RDF: the object, every epoch found in its
    /// committed chain with its host and first and last position, the custodians of the
    /// configuration in force in it, the promises of the epochs this host won, and recorded
    /// equivocation evidence.
    // @tg: implements:: [[openspec:durable-objects#Durable Object Lineage]]
    pub fn lineage(&self) -> Graph {
        let st = self.lock();
        let committed = st.committed.get() as usize;
        let mut g = Graph::new();
        let obj = self.object.iri();
        let n = |s: &str| vocab::node(s);
        let int = |v: u64| Literal::new_typed_literal(v.to_string(), xsd::INTEGER);
        g.insert(&Triple::new(
            obj.clone(),
            store_vocab::node(store_vocab::RDF_TYPE),
            n(vocab::DURABLE_OBJECT),
        ));
        // (epoch, writer, first, last)
        let mut spans: Vec<(u64, String, u64, u64)> = Vec::new();
        for (i, e) in self.local.entries().iter().take(committed).enumerate() {
            let Ok(f) = e.facts() else { continue };
            let pos = i as u64 + 1;
            match spans.last_mut() {
                Some(s) if s.0 == f.epoch && s.1 == f.writer => s.3 = pos,
                _ => spans.push((f.epoch, f.writer.clone(), pos, pos)),
            }
        }
        let configs = self.local.configs();
        for (epoch, writer, first, last) in &spans {
            let e = NamedNode::new_unchecked(format!("{}#epoch-{epoch}", obj.as_str()));
            g.insert(&Triple::new(
                e.clone(),
                store_vocab::node(store_vocab::RDF_TYPE),
                n(vocab::CUSTODY_EPOCH_CLASS),
            ));
            g.insert(&Triple::new(e.clone(), n(vocab::EPOCH_OF), obj.clone()));
            g.insert(&Triple::new(e.clone(), n(vocab::EPOCH_NUMBER), int(*epoch)));
            if let Ok(w) = NamedNode::new(writer.clone()) {
                g.insert(&Triple::new(e.clone(), n(vocab::EPOCH_HOST), w));
            }
            g.insert(&Triple::new(
                e.clone(),
                n(vocab::FIRST_POSITION),
                int(*first),
            ));
            g.insert(&Triple::new(e.clone(), n(vocab::LAST_POSITION), int(*last)));
            let cfg = configs
                .iter()
                .rev()
                .find(|(from, _)| from <= epoch)
                .or_else(|| configs.first())
                .map(|(_, c)| c);
            for c in cfg.map(|c| c.custodians.as_slice()).unwrap_or_default() {
                if let Ok(id) = NamedNode::new(c.id.clone()) {
                    g.insert(&Triple::new(e.clone(), n(vocab::CUSTODIAN), id));
                }
            }
            if let Some(rec) = st.epochs.iter().find(|r| r.epoch == *epoch) {
                for p in &rec.promises {
                    if let Ok(id) = NamedNode::new(p.custodian.clone()) {
                        g.insert(&Triple::new(e.clone(), n(vocab::PROMISED_BY), id));
                    }
                }
            }
        }
        for (k, ev) in self.local.evidence().iter().enumerate() {
            let e = NamedNode::new_unchecked(format!("{}#equivocation-{}", obj.as_str(), k + 1));
            g.insert(&Triple::new(
                e.clone(),
                store_vocab::node(store_vocab::RDF_TYPE),
                n(vocab::EQUIVOCATION),
            ));
            g.insert(&Triple::new(e.clone(), n(vocab::EPOCH_OF), obj.clone()));
            g.insert(&Triple::new(
                e.clone(),
                n(vocab::EPOCH_NUMBER),
                int(ev.epoch),
            ));
            g.insert(&Triple::new(
                e.clone(),
                store_vocab::node(store_vocab::POSITION),
                int(ev.position),
            ));
            for w in [&ev.first, &ev.second] {
                if let Ok(f) = w.facts() {
                    g.insert(&Triple::new(
                        e.clone(),
                        n(vocab::CONFLICTING_ENTRY),
                        Literal::new_simple_literal(f.hash.to_hex()),
                    ));
                }
            }
        }
        g
    }

    /// Writes [`CustodyHost::lineage`] into the named graph `<object>/custody` of `store`.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub fn record_lineage(&self, store: &dyn GraphStore) -> Result<(), StoreError> {
        let name = GraphName::NamedNode(NamedNode::new_unchecked(format!(
            "{}/custody",
            self.object.as_str()
        )));
        let quads: Vec<Quad> = self
            .lineage()
            .iter()
            .map(|t| t.into_owned().in_graph(name.clone()))
            .collect();
        store.insert_quads(&quads)
    }
}

/// The journal store of a node that hosts durable objects: appends, reads and heads of a
/// durable object go to its [`CustodyHost`] (which elects on first use), every other actor's to
/// `inner`.
pub struct CustodyStore {
    inner: Arc<dyn JournalStore>,
    hosts: RwLock<BTreeMap<String, Arc<CustodyHost>>>,
}

impl fmt::Debug for CustodyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustodyStore").finish_non_exhaustive()
    }
}

impl CustodyStore {
    /// A store over `inner` with no durable objects yet.
    pub fn new(inner: Arc<dyn JournalStore>) -> Self {
        CustodyStore {
            inner,
            hosts: RwLock::new(BTreeMap::new()),
        }
    }

    /// Serves the durable object of `host`.
    pub fn add(&self, host: Arc<CustodyHost>) {
        self.hosts
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(host.object().as_str().to_owned(), host);
    }

    /// The custody host of `actor`, if it is a durable object here.
    pub fn host(&self, actor: &NamedNode) -> Option<Arc<CustodyHost>> {
        self.hosts
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(actor.as_str())
            .cloned()
    }

    fn active(&self, actor: &NamedNode) -> Result<Option<Arc<CustodyHost>>, StoreError> {
        let Some(h) = self.host(actor) else {
            return Ok(None);
        };
        if h.writer_epoch().is_none() && h.is_standby() {
            return Err(StoreError::Unavailable(format!(
                "{} is a standby host of {actor}",
                h.host()
            )));
        }
        h.activate()
            .map_err(|e| StoreError::Unavailable(e.to_string()))?;
        Ok(Some(h))
    }
}

impl JournalStore for CustodyStore {
    fn capabilities(&self) -> JournalCapabilities {
        JournalCapabilities::default()
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        if meta.fencing.is_some() || meta.state.is_some() {
            return Err(AppendError::Unsupported(
                "a custody store takes neither fencing tokens nor state deltas".into(),
            ));
        }
        match self.active(actor).map_err(AppendError::Storage)? {
            Some(h) => h.append_turn(expected, entries),
            None => self.inner.append(actor, expected, entries, meta),
        }
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        match self.active(actor)? {
            Some(h) => h.read_committed(from),
            None => self.inner.read(actor, from),
        }
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        match self.active(actor)? {
            Some(h) => {
                h.read_committed(Position::new(u64::MAX))?;
                Ok(h.committed())
            }
            None => self.inner.head(actor),
        }
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        let mut out = self.inner.actors()?;
        out.retain(|a| !a.as_str().ends_with("/custody-log"));
        for k in self
            .hosts
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
        {
            out.push(NamedNode::new_unchecked(k));
        }
        Ok(out)
    }
}

/// The custody facts of a sealed entry, for inspection.
///
/// # Errors
///
/// The chain error when the entry is not sealed.
pub fn entry_epoch(entry: &Entry) -> Result<(u64, String), crate::chain::ChainError> {
    let l = read_link(entry)?;
    Ok((l.link.epoch, l.link.writer))
}
