//! The mediator's durable mailboxes: one event-sourced journal per recipient VID in any
//! [`JournalStore`] (a directory of tiramemsu shard files in production, one writer per file).
//!
//! Every change is an entry appended before the change takes effect: a registration, a stored
//! message, an acknowledgement (`received`), an expiry, a revocation. The in-memory index is
//! rebuilt by replaying the journals when the store opens, so a stored message survives
//! restarts, and an M4 replicated journal under the store survives the loss of a node.
//!
//! An entry holds only what the mediator may persist (design D11, TSP 1.0 §5.4): the recipient
//! VID, the opaque sealed bytes, a message id, the arrival time and the size. Acknowledged and
//! expired copies leave the index at once; their ciphertext stays in the journal history until
//! [`MailboxStore::compact_into`] rewrites the store with live messages only.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term, Triple};
use sha2::{Digest, Sha256};
use vertep_federation::tsp_mailbox::{ExpiryReport, HeldMessage, MailboxStatus};
use vertep_store::{AppendError, AppendMeta, Entry, JournalStore, Position, StoreError};

/// The vocabulary of mailbox journal entries.
pub const VOCAB: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#";

/// The prefix of mailbox journal IRIs.
pub const MAILBOX_PREFIX: &str = "urn:vt:mailbox:";

fn term(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VOCAB}{local}"))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The journal IRI of `vid`'s mailbox: a hash of the VID, so the IRI itself names no one.
pub fn mailbox_iri(vid: &str) -> NamedNode {
    let h = Sha256::digest(vid.as_bytes());
    let hex: String = h.iter().take(20).map(|b| format!("{b:02x}")).collect();
    NamedNode::new_unchecked(format!("{MAILBOX_PREFIX}{hex}"))
}

/// The mediator's id of a stored copy: the digest of the message as the previous hop posted it,
/// so the same post retried by a previous hop is stored once (the stored bytes are re-sealed for
/// the recipient and differ on every retry).
pub fn message_id(data: &[u8]) -> String {
    let h = Sha256::digest(data);
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Why a store operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailboxStoreError {
    /// The journal store failed; nothing changed.
    Store(String),
    /// A journal entry could not be read.
    Corrupt(String),
}

impl fmt::Display for MailboxStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MailboxStoreError::Store(m) => write!(f, "mailbox store: {m}"),
            MailboxStoreError::Corrupt(m) => write!(f, "mailbox journal: {m}"),
        }
    }
}

impl std::error::Error for MailboxStoreError {}

impl From<StoreError> for MailboxStoreError {
    fn from(e: StoreError) -> Self {
        MailboxStoreError::Store(e.to_string())
    }
}

impl From<AppendError> for MailboxStoreError {
    fn from(e: AppendError) -> Self {
        MailboxStoreError::Store(e.to_string())
    }
}

/// One change to a mailbox, as journaled.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Record {
    Register,
    Unregister { revoked: bool },
    Stored(HeldMessage),
    Received(Vec<String>),
    Expired { ids: Vec<String>, at_ms: i64 },
    Reported,
}

impl Record {
    fn kind(&self) -> &'static str {
        match self {
            Record::Register => "register",
            Record::Unregister { .. } => "unregister",
            Record::Stored(_) => "stored",
            Record::Received(_) => "received",
            Record::Expired { .. } => "expired",
            Record::Reported => "reported",
        }
    }

    fn to_entry(&self, mailbox: &NamedNode, vid: &str, position: Position) -> Entry {
        let e = NamedNode::new_unchecked(format!("{}/e{}", mailbox.as_str(), position.get()));
        let int = |n: i64| Literal::new_typed_literal(n.to_string(), xsd::INTEGER);
        let mut t = vec![
            Triple::new(e.clone(), term("kind"), Literal::from(self.kind())),
            Triple::new(e.clone(), term("recipient"), Literal::from(vid)),
        ];
        match self {
            Record::Register | Record::Reported => {}
            Record::Unregister { revoked } => {
                t.push(Triple::new(
                    e.clone(),
                    term("revoked"),
                    Literal::from(*revoked),
                ));
            }
            Record::Stored(h) => {
                t.push(Triple::new(
                    e.clone(),
                    term("messageId"),
                    Literal::from(h.id.as_str()),
                ));
                t.push(Triple::new(
                    e.clone(),
                    term("data"),
                    Literal::new_typed_literal(STANDARD.encode(&h.data), xsd::BASE_64_BINARY),
                ));
                t.push(Triple::new(e.clone(), term("arrival"), int(h.arrival_ms)));
                t.push(Triple::new(
                    e.clone(),
                    term("size"),
                    int(i64::try_from(h.size).unwrap_or(i64::MAX)),
                ));
            }
            Record::Received(ids) => {
                for id in ids {
                    t.push(Triple::new(
                        e.clone(),
                        term("messageId"),
                        Literal::from(id.as_str()),
                    ));
                }
            }
            Record::Expired { ids, at_ms } => {
                for id in ids {
                    t.push(Triple::new(
                        e.clone(),
                        term("messageId"),
                        Literal::from(id.as_str()),
                    ));
                }
                t.push(Triple::new(e.clone(), term("at"), int(*at_ms)));
            }
        }
        Entry::new(e, t)
    }

    fn from_entry(entry: &Entry) -> Result<(String, Record), MailboxStoreError> {
        let s = entry.name();
        let values = |p: &str| -> Vec<String> {
            entry
                .triples()
                .objects_for_subject_predicate(s, &term(p))
                .filter_map(|o| match o.into_owned() {
                    Term::Literal(l) => Some(l.value().to_owned()),
                    _ => None,
                })
                .collect()
        };
        let one = |p: &str| -> Result<String, MailboxStoreError> {
            values(p)
                .into_iter()
                .next()
                .ok_or_else(|| MailboxStoreError::Corrupt(format!("{s}: no {p}")))
        };
        let int = |p: &str| -> Result<i64, MailboxStoreError> {
            one(p)?
                .parse::<i64>()
                .map_err(|e| MailboxStoreError::Corrupt(format!("{s}: {p}: {e}")))
        };
        let vid = one("recipient")?;
        let record = match one("kind")?.as_str() {
            "register" => Record::Register,
            "reported" => Record::Reported,
            "unregister" => Record::Unregister {
                revoked: values("revoked").iter().any(|v| v == "true"),
            },
            "stored" => {
                let data = STANDARD
                    .decode(one("data")?)
                    .map_err(|e| MailboxStoreError::Corrupt(format!("{s}: data: {e}")))?;
                Record::Stored(HeldMessage {
                    id: one("messageId")?,
                    size: u64::try_from(int("size")?).unwrap_or(0),
                    arrival_ms: int("arrival")?,
                    data,
                })
            }
            "received" => Record::Received(values("messageId")),
            "expired" => Record::Expired {
                ids: values("messageId"),
                at_ms: int("at")?,
            },
            other => {
                return Err(MailboxStoreError::Corrupt(format!(
                    "{s}: unknown kind {other}"
                )));
            }
        };
        Ok((vid, record))
    }
}

/// One mailbox's state.
#[derive(Clone, Debug)]
struct BoxState {
    iri: NamedNode,
    head: Position,
    registered: bool,
    revoked: bool,
    held: VecDeque<HeldMessage>,
    total_bytes: u64,
    expired: ExpiryReport,
}

impl BoxState {
    fn new(vid: &str) -> Self {
        BoxState {
            iri: mailbox_iri(vid),
            head: Position::ZERO,
            registered: false,
            revoked: false,
            held: VecDeque::new(),
            total_bytes: 0,
            expired: ExpiryReport::default(),
        }
    }

    fn apply(&mut self, r: &Record) {
        match r {
            Record::Register => {
                self.registered = true;
                self.revoked = false;
            }
            Record::Unregister { revoked } => {
                self.registered = false;
                self.revoked = *revoked;
                self.held.clear();
                self.total_bytes = 0;
            }
            Record::Stored(h) => {
                if !self.held.iter().any(|x| x.id == h.id) {
                    self.total_bytes += h.size;
                    self.held.push_back(h.clone());
                }
            }
            Record::Received(ids) => self.remove(ids),
            Record::Expired { ids, at_ms } => {
                let n = self.held.iter().filter(|h| ids.contains(&h.id)).count() as u64;
                self.remove(ids);
                if n > 0 {
                    if self.expired.count == 0 || *at_ms < self.expired.oldest_expiry_ms {
                        self.expired.oldest_expiry_ms = *at_ms;
                    }
                    self.expired.count += n;
                }
            }
            Record::Reported => self.expired = ExpiryReport::default(),
        }
    }

    fn remove(&mut self, ids: &[String]) {
        let before = self.total_bytes;
        let mut removed = 0;
        self.held.retain(|h| {
            if ids.contains(&h.id) {
                removed += h.size;
                false
            } else {
                true
            }
        });
        self.total_bytes = before.saturating_sub(removed);
    }

    fn status(&self, now_ms: i64) -> MailboxStatus {
        MailboxStatus {
            message_count: self.held.len() as u64,
            total_bytes: self.total_bytes,
            oldest_age_ms: self
                .held
                .front()
                .map_or(0, |h| (now_ms - h.arrival_ms).max(0)),
            expired: self.expired,
        }
    }
}

/// What [`MailboxStore::store`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    /// The stored copy's id.
    pub id: String,
    /// True when the same bytes were already held (nothing was appended).
    pub duplicate: bool,
}

/// Per-mailbox figures for metrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxFigures {
    /// The recipient VID.
    pub vid: String,
    /// Whether the VID is registered.
    pub registered: bool,
    /// Waiting messages, their bytes and the oldest's age.
    pub status: MailboxStatus,
}

/// The mediator's mailboxes over a journal store. Any `JournalStore` serves: tiramemsu shard
/// files on one node, or an M4 `ReplicatedJournal` whose appends complete only after a standby
/// stored them, so a node's loss loses no accepted message.
// @tg: implements:: [[openspec:tsp-mediator#Durable Store-And-Forward]]
// @tg: implements:: [[openspec:tsp-mediator#Replicated Mediator Deployment]] {note: "a ReplicatedJournal under the store; the binary runs one node"}
pub struct MailboxStore {
    journal: Arc<dyn JournalStore>,
    boxes: Mutex<BTreeMap<String, BoxState>>,
}

impl fmt::Debug for MailboxStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailboxStore")
            .field("mailboxes", &lock(&self.boxes).len())
            .finish()
    }
}

impl MailboxStore {
    /// Opens the mailboxes journaled in `journal`, replaying every mailbox journal.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the store fails or an entry cannot be read.
    pub fn open(journal: Arc<dyn JournalStore>) -> Result<Self, MailboxStoreError> {
        let mut boxes: BTreeMap<String, BoxState> = BTreeMap::new();
        for actor in journal.actors()? {
            if !actor.as_str().starts_with(MAILBOX_PREFIX) {
                continue;
            }
            let records = journal.read(&actor, Position::ZERO)?;
            for rec in records {
                let (vid, r) = Record::from_entry(&rec.entry)?;
                let b = boxes
                    .entry(vid.clone())
                    .or_insert_with(|| BoxState::new(&vid));
                b.apply(&r);
                b.head = rec.position;
            }
        }
        Ok(MailboxStore {
            journal,
            boxes: Mutex::new(boxes),
        })
    }

    fn append(
        &self,
        boxes: &mut BTreeMap<String, BoxState>,
        vid: &str,
        r: Record,
    ) -> Result<(), MailboxStoreError> {
        let b = boxes
            .entry(vid.to_owned())
            .or_insert_with(|| BoxState::new(vid));
        let entry = r.to_entry(&b.iri, vid, b.head.next());
        let head = self
            .journal
            .append(&b.iri, b.head, &[entry], &AppendMeta::none())?;
        b.head = head;
        b.apply(&r);
        Ok(())
    }

    /// True when `vid` has a mailbox.
    pub fn is_registered(&self, vid: &str) -> bool {
        lock(&self.boxes).get(vid).is_some_and(|b| b.registered)
    }

    /// True when the operator revoked `vid`'s mailbox.
    pub fn is_revoked(&self, vid: &str) -> bool {
        lock(&self.boxes).get(vid).is_some_and(|b| b.revoked)
    }

    /// Creates `vid`'s mailbox (idempotent).
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails.
    pub fn register(&self, vid: &str) -> Result<(), MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        if boxes.get(vid).is_some_and(|b| b.registered) {
            return Ok(());
        }
        self.append(&mut boxes, vid, Record::Register)
    }

    /// Removes `vid`'s mailbox and drops what it held; `revoked` marks an operator revocation.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails.
    pub fn unregister(&self, vid: &str, revoked: bool) -> Result<(), MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        self.append(&mut boxes, vid, Record::Unregister { revoked })
    }

    /// Stores `data` for `vid`, durably, before returning.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails; nothing is stored then.
    pub fn store(
        &self,
        vid: &str,
        data: Vec<u8>,
        now_ms: i64,
    ) -> Result<Stored, MailboxStoreError> {
        let id = message_id(&data);
        self.store_as(vid, id, data, now_ms)
    }

    /// Stores `data` for `vid` under `id` (see [`message_id`]); a copy with that id already held
    /// is not stored again.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails; nothing is stored then.
    pub fn store_as(
        &self,
        vid: &str,
        id: String,
        data: Vec<u8>,
        now_ms: i64,
    ) -> Result<Stored, MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        if boxes
            .get(vid)
            .is_some_and(|b| b.held.iter().any(|h| h.id == id))
        {
            return Ok(Stored {
                id,
                duplicate: true,
            });
        }
        let held = HeldMessage {
            id: id.clone(),
            size: data.len() as u64,
            arrival_ms: now_ms,
            data,
        };
        self.append(&mut boxes, vid, Record::Stored(held))?;
        Ok(Stored {
            id,
            duplicate: false,
        })
    }

    /// Up to `limit` of `vid`'s waiting messages, oldest first.
    pub fn held(&self, vid: &str, limit: usize) -> Vec<HeldMessage> {
        lock(&self.boxes)
            .get(vid)
            .map(|b| b.held.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// The message `id` of `vid`, if held.
    pub fn get(&self, vid: &str, id: &str) -> Option<HeldMessage> {
        lock(&self.boxes)
            .get(vid)
            .and_then(|b| b.held.iter().find(|h| h.id == id).cloned())
    }

    /// `vid`'s status (including unreported expiries).
    pub fn status(&self, vid: &str, now_ms: i64) -> MailboxStatus {
        lock(&self.boxes)
            .get(vid)
            .map(|b| b.status(now_ms))
            .unwrap_or_default()
    }

    /// Deletes `ids` from `vid`'s mailbox (ids of other mailboxes are ignored). Returns how
    /// many were removed.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails.
    pub fn received(&self, vid: &str, ids: &[String]) -> Result<usize, MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        let mine: Vec<String> = boxes
            .get(vid)
            .map(|b| {
                b.held
                    .iter()
                    .filter(|h| ids.contains(&h.id))
                    .map(|h| h.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if mine.is_empty() {
            return Ok(0);
        }
        let n = mine.len();
        self.append(&mut boxes, vid, Record::Received(mine))?;
        Ok(n)
    }

    /// Expires every message that arrived more than `retention_ms` before `now_ms`. Returns the
    /// number expired per VID.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when an append fails.
    pub fn expire(
        &self,
        retention_ms: i64,
        now_ms: i64,
    ) -> Result<BTreeMap<String, usize>, MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        let due: Vec<(String, Vec<String>)> = boxes
            .iter()
            .filter_map(|(vid, b)| {
                let ids: Vec<String> = b
                    .held
                    .iter()
                    .filter(|h| now_ms - h.arrival_ms > retention_ms)
                    .map(|h| h.id.clone())
                    .collect();
                (!ids.is_empty()).then(|| (vid.clone(), ids))
            })
            .collect();
        let mut out = BTreeMap::new();
        for (vid, ids) in due {
            out.insert(vid.clone(), ids.len());
            self.append(&mut boxes, &vid, Record::Expired { ids, at_ms: now_ms })?;
        }
        Ok(out)
    }

    /// Returns `vid`'s unreported expiries and marks them reported.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the append fails.
    pub fn take_expiry_report(&self, vid: &str) -> Result<ExpiryReport, MailboxStoreError> {
        let mut boxes = lock(&self.boxes);
        let report = boxes.get(vid).map(|b| b.expired).unwrap_or_default();
        if report.count > 0 {
            self.append(&mut boxes, vid, Record::Reported)?;
        }
        Ok(report)
    }

    /// Figures of every mailbox.
    pub fn figures(&self, now_ms: i64) -> Vec<MailboxFigures> {
        lock(&self.boxes)
            .iter()
            .map(|(vid, b)| MailboxFigures {
                vid: vid.clone(),
                registered: b.registered,
                status: b.status(now_ms),
            })
            .collect()
    }

    /// Writes the live state (registrations, revocations and waiting messages) into `target`,
    /// an empty store: what an offline compaction keeps. Acknowledged and expired ciphertext
    /// does not reach the target.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the target fails.
    pub fn compact_into(&self, target: Arc<dyn JournalStore>) -> Result<(), MailboxStoreError> {
        let boxes = lock(&self.boxes).clone();
        let out = MailboxStore {
            journal: target,
            boxes: Mutex::new(BTreeMap::new()),
        };
        for (vid, b) in boxes {
            if b.revoked {
                out.unregister(&vid, true)?;
            }
            if b.registered {
                out.register(&vid)?;
            }
            for h in b.held {
                let mut g = lock(&out.boxes);
                out.append(&mut g, &vid, Record::Stored(h))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vertep_store::MemoryStore;

    #[test]
    fn replay_rebuilds_the_mailboxes() {
        let j: Arc<dyn JournalStore> = Arc::new(MemoryStore::new());
        let s = MailboxStore::open(j.clone()).unwrap();
        s.register("did:peer:a").unwrap();
        let one = s.store("did:peer:a", b"one".to_vec(), 10).unwrap();
        s.store("did:peer:a", b"two".to_vec(), 20).unwrap();
        assert!(
            s.store("did:peer:a", b"one".to_vec(), 30)
                .unwrap()
                .duplicate
        );
        s.received("did:peer:a", std::slice::from_ref(&one.id))
            .unwrap();
        let again = MailboxStore::open(j).unwrap();
        assert!(again.is_registered("did:peer:a"));
        let held = again.held("did:peer:a", 10);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].data, b"two");
        assert_eq!(again.status("did:peer:a", 25).oldest_age_ms, 5);
    }

    #[test]
    fn expiry_is_reported_once() {
        let j: Arc<dyn JournalStore> = Arc::new(MemoryStore::new());
        let s = MailboxStore::open(j.clone()).unwrap();
        s.register("v").unwrap();
        s.store("v", b"old".to_vec(), 0).unwrap();
        s.store("v", b"new".to_vec(), 900).unwrap();
        assert_eq!(s.expire(500, 1_000).unwrap().get("v"), Some(&1));
        let r = MailboxStore::open(j.clone())
            .unwrap()
            .take_expiry_report("v")
            .unwrap();
        assert_eq!(r.count, 1);
        assert_eq!(r.oldest_expiry_ms, 1_000);
        assert_eq!(
            MailboxStore::open(j)
                .unwrap()
                .status("v", 1_000)
                .expired
                .count,
            0
        );
    }

    #[test]
    fn compaction_keeps_only_live_messages() {
        let j: Arc<dyn JournalStore> = Arc::new(MemoryStore::new());
        let s = MailboxStore::open(j).unwrap();
        s.register("v").unwrap();
        let a = s.store("v", b"acked".to_vec(), 1).unwrap();
        s.store("v", b"waiting".to_vec(), 2).unwrap();
        s.received("v", &[a.id]).unwrap();
        let target: Arc<dyn JournalStore> = Arc::new(MemoryStore::new());
        s.compact_into(target.clone()).unwrap();
        let c = MailboxStore::open(target.clone()).unwrap();
        assert_eq!(c.held("v", 10).len(), 1);
        let all = target.read(&mailbox_iri("v"), Position::ZERO).unwrap();
        let text = format!("{all:?}");
        assert!(
            !text.contains(&STANDARD.encode(b"acked")),
            "acked ciphertext is gone"
        );
    }
}
