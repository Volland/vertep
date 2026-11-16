//! Journal shipping: a follower pulls a journal's entries from the shard's leader, in order and
//! with their digests, over the cluster transport (M13 design D4).
//!
//! The protocol has one request and one answer:
//!
//! - **[`PullRequest`]** (follower to leader): the shard, the actor whose journal is shipped,
//!   the follower's last durable position `after`, and the `window` of bytes it has room for.
//!   `after` is also the follower's acknowledgement: every entry up to it is stored durably.
//! - **[`PullBatch`]** (leader to follower): the leader, its `(shard, epoch)` fence, the digest
//!   of its entry at `after` (`prev`, so the follower checks the batch extends *its* journal),
//!   the leader's head, and the entries after `after` in position order, each with its
//!   position and its SHA-256 digest ([`vertep_store::Entry::digest`]), at most `window` bytes
//!   of them; or a [`PullError`].
//!
//! The follower ([`ShipFollower::accept`]) checks every entry in order — digest, fence, position
//! — appends the verified prefix to its store under the leader's fencing epoch, and stops at the
//! first entry it refuses; its next pull starts again from its durable head. So:
//!
//! - **Ordering and integrity.** An entry whose digest does not match its content, whose
//!   position is not the follower's next, or which comes from a leader whose epoch is below one
//!   the follower already stored, is refused and never appended ([`Verdict`]).
//! - **Resumption.** The follower's position is its store's head, which survives a dropped
//!   connection and a restart of the follower; a pull from it can neither skip nor repeat.
//! - **Back-pressure.** The leader answers a pull with at most the window the follower asked
//!   for (and its own [`ShipLeader`] maximum), keeps for each follower only the batch it has not
//!   yet acknowledged, and reads its journal in bounded ranges
//!   ([`vertep_store::JournalStore::read_range`]); a slow follower slows its own pulls and grows
//!   nothing on the leader.
//!
//! [`ShipLink`] is the follower's way to the leader: [`LocalShipLink`] in process, and
//! `QuicShipLink` (feature `quic`, [`quic`]) over the cluster transport, where a pull travels as
//! a request [`crate::Frame`] answered by [`crate::Reply::Data`].
//!
//! [`Accepted::record`] writes a round into a fault-injection [`History`]: the follower's
//! store is the fenced store of the lease model, every shipped entry an append by the leader
//! under its epoch, so a shipping history replays through the Lean lease model like any other.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use vertep_store::oxrdf::NamedNode;
use vertep_store::{AppendError, AppendMeta, Entry, FencingToken, JournalStore, Position};

use crate::fencing::Fence;
use crate::history::{AppendDecision, History, HistoryEvent};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;

mod wire;

#[cfg(feature = "quic")]
pub mod quic;

pub use wire::{decode_reply, encode_reply, entry_from_nquads, entry_to_nquads};

/// The event name of a pull request frame on the cluster transport.
pub const PULL_EVENT: &str = "vertep.journal.pull";

/// The default window a follower asks for: 256 KiB.
pub const DEFAULT_WINDOW: u32 = 256 * 1024;

/// The most entries one batch carries, whatever the window.
pub const MAX_BATCH_ENTRIES: usize = 1024;

/// A digest of one entry ([`Entry::digest`]).
pub type EntryDigest = [u8; 32];

/// A follower's pull: the entries of `actor` after `after`, at most `window` bytes of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// The shard the actor belongs to.
    pub shard: ShardId,
    /// The actor whose journal is shipped.
    pub actor: NamedNode,
    /// The follower's last durable position, which acknowledges every entry up to it.
    pub after: Position,
    /// The bytes of entries the follower has room for.
    pub window: u32,
}

/// One shipped entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShippedEntry {
    /// Its position in the actor's journal.
    pub position: Position,
    /// The digest of the entry as the leader stores it.
    pub digest: EntryDigest,
    /// The entry, with the leader store's stamps.
    pub entry: Entry,
}

impl ShippedEntry {
    /// The bytes the entry takes in a batch, counted against the window.
    pub fn wire_size(&self) -> usize {
        8 + 32 + 4 + entry_to_nquads(&self.entry).len()
    }
}

/// The leader's answer to a pull.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullBatch {
    /// The leader that served it.
    pub leader: NodeId,
    /// The leader's shard and lease epoch.
    pub fence: Fence,
    /// The actor.
    pub actor: NamedNode,
    /// The position the first entry follows (the request's `after`).
    pub after: Position,
    /// The digest of the leader's entry at `after`; `None` when `after` is 0.
    pub prev: Option<EntryDigest>,
    /// The leader's head when it served the pull.
    pub head: Position,
    /// The entries, in position order.
    pub entries: Vec<ShippedEntry>,
}

impl PullBatch {
    /// The bytes of the batch's entries, counted against the window.
    pub fn wire_size(&self) -> usize {
        self.entries.iter().map(ShippedEntry::wire_size).sum()
    }
}

/// Why a pull got no batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullError {
    /// The node asked does not hold the shard's lease.
    NotLeader {
        /// The shard.
        shard: ShardId,
    },
    /// The follower is ahead of the leader: its position is past the leader's head.
    Behind {
        /// The leader's head.
        head: Position,
    },
    /// The next entry alone exceeds the window: pull again with a larger one.
    TooLarge {
        /// The entry's position.
        position: Position,
        /// Its size in a batch.
        size: usize,
        /// The window that was granted.
        window: usize,
    },
    /// The leader's store failed.
    Store(String),
    /// The leader cannot be reached (connection refused or dropped).
    Unreachable(String),
    /// The request or the answer could not be decoded.
    Malformed(String),
}

impl fmt::Display for PullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PullError::NotLeader { shard } => write!(f, "not the leader of {shard}"),
            PullError::Behind { head } => {
                write!(f, "the follower is past the leader's head {}", head.get())
            }
            PullError::TooLarge {
                position,
                size,
                window,
            } => write!(
                f,
                "entry {} takes {size} bytes, over the window of {window}",
                position.get()
            ),
            PullError::Store(m) => write!(f, "leader store: {m}"),
            PullError::Unreachable(m) => write!(f, "leader unreachable: {m}"),
            PullError::Malformed(m) => write!(f, "malformed shipping message: {m}"),
        }
    }
}

impl std::error::Error for PullError {}

/// What answers pulls: a [`ShipLeader`], or a test double.
pub trait PullSource: Send + Sync {
    /// The shard served.
    fn shard(&self) -> ShardId;
    /// Answers `follower`'s pull.
    ///
    /// # Errors
    ///
    /// [`PullError`].
    fn serve(&self, follower: &NodeId, request: &PullRequest) -> Result<PullBatch, PullError>;
}

#[derive(Clone, Copy, Debug, Default)]
struct Outstanding {
    /// The size of the batch sent and not yet acknowledged.
    bytes: usize,
}

#[derive(Default)]
struct LeaderState {
    fence: Option<Fence>,
    /// The batch each follower has not acknowledged yet, per actor.
    outstanding: BTreeMap<(NodeId, NamedNode), Outstanding>,
    /// The position each follower acknowledged, per actor.
    acked: BTreeMap<(NodeId, NamedNode), Position>,
    /// The most bytes ever held for each follower.
    peak: BTreeMap<NodeId, usize>,
}

/// The leader's side of shipping one shard: answers pulls from its journal store while it
/// holds the shard's lease.
// @tg: implements:: [[openspec:journal-shipping#Ordered Shipping]]
// @tg: implements:: [[openspec:journal-shipping#Back Pressure]]
pub struct ShipLeader<J> {
    node: NodeId,
    shard: ShardId,
    store: J,
    max_window: usize,
    state: Mutex<LeaderState>,
}

impl<J> fmt::Debug for ShipLeader<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShipLeader")
            .field("node", &self.node)
            .field("shard", &self.shard)
            .finish_non_exhaustive()
    }
}

impl<J: JournalStore> ShipLeader<J> {
    /// The leader `node` of `shard`, serving from `store`, never sending more than
    /// `max_window` bytes in one batch whatever a follower asks for. It serves nothing until
    /// [`ShipLeader::set_fence`] gives it its lease.
    pub fn new(node: NodeId, shard: ShardId, store: J, max_window: usize) -> Self {
        ShipLeader {
            node,
            shard,
            store,
            max_window: max_window.max(1),
            state: Mutex::new(LeaderState::default()),
        }
    }

    /// The store served from.
    pub fn store(&self) -> &J {
        &self.store
    }

    /// The lease this node holds on the shard (`None` once it lost it): batches carry it, and
    /// pulls are refused without it.
    pub fn set_fence(&self, epoch: Option<u64>) {
        self.lock().fence = epoch.map(|epoch| Fence {
            shard: self.shard,
            epoch,
        });
    }

    fn lock(&self) -> MutexGuard<'_, LeaderState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The bytes this leader holds for `follower`: the batches it sent and the follower has
    /// not acknowledged yet (at most one per actor).
    pub fn buffered(&self, follower: &NodeId) -> usize {
        self.lock()
            .outstanding
            .iter()
            .filter(|((n, _), _)| n == follower)
            .map(|(_, o)| o.bytes)
            .sum()
    }

    /// The most bytes this leader ever held for `follower` at once.
    pub fn peak_buffered(&self, follower: &NodeId) -> usize {
        self.lock().peak.get(follower).copied().unwrap_or(0)
    }

    /// The position `follower` last acknowledged for `actor`.
    pub fn acknowledged(&self, follower: &NodeId, actor: &NamedNode) -> Position {
        self.lock()
            .acked
            .get(&(follower.clone(), actor.clone()))
            .copied()
            .unwrap_or(Position::ZERO)
    }

    fn store_err(e: impl fmt::Display) -> PullError {
        PullError::Store(e.to_string())
    }
}

impl<J: JournalStore> PullSource for ShipLeader<J> {
    fn shard(&self) -> ShardId {
        self.shard
    }

    fn serve(&self, follower: &NodeId, req: &PullRequest) -> Result<PullBatch, PullError> {
        if req.shard != self.shard {
            return Err(PullError::NotLeader { shard: req.shard });
        }
        let key = (follower.clone(), req.actor.clone());
        let fence = {
            let mut st = self.lock();
            // the pull acknowledges everything up to `after` and supersedes the batch before
            // it: either way the follower no longer waits for it
            st.outstanding.remove(&key);
            st.acked.insert(key.clone(), req.after);
            st.fence.ok_or(PullError::NotLeader { shard: self.shard })?
        };
        let head = self.store.head(&req.actor).map_err(Self::store_err)?;
        if req.after > head {
            return Err(PullError::Behind { head });
        }
        let prev = if req.after == Position::ZERO {
            None
        } else {
            self.store
                .read_range(&req.actor, req.after, 1)
                .map_err(Self::store_err)?
                .first()
                .map(|r| r.entry.digest())
        };
        let window = (req.window as usize).min(self.max_window);
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        let mut next = req.after.next();
        'fill: while next <= head && entries.len() < MAX_BATCH_ENTRIES {
            // a bounded read: a follower far behind costs one chunk per pull, not its lag
            let chunk = 16.min(MAX_BATCH_ENTRIES - entries.len());
            let records = self
                .store
                .read_range(&req.actor, next, chunk)
                .map_err(Self::store_err)?;
            if records.is_empty() {
                break;
            }
            for r in records {
                let e = ShippedEntry {
                    position: r.position,
                    digest: r.entry.digest(),
                    entry: r.entry,
                };
                let size = e.wire_size();
                if bytes + size > window {
                    if entries.is_empty() {
                        return Err(PullError::TooLarge {
                            position: e.position,
                            size,
                            window,
                        });
                    }
                    break 'fill;
                }
                bytes += size;
                next = e.position.next();
                entries.push(e);
            }
        }
        if let Some(last) = entries.last() {
            let mut st = self.lock();
            debug_assert!(last.position <= head);
            st.outstanding.insert(key, Outstanding { bytes });
            let held: usize = st
                .outstanding
                .iter()
                .filter(|((n, _), _)| n == follower)
                .map(|(_, o)| o.bytes)
                .sum();
            let peak = st.peak.entry(follower.clone()).or_default();
            *peak = (*peak).max(held);
        }
        Ok(PullBatch {
            leader: self.node.clone(),
            fence,
            actor: req.actor.clone(),
            after: req.after,
            prev,
            head,
            entries,
        })
    }
}

impl<S: PullSource + ?Sized> PullSource for Arc<S> {
    fn shard(&self) -> ShardId {
        (**self).shard()
    }
    fn serve(&self, follower: &NodeId, request: &PullRequest) -> Result<PullBatch, PullError> {
        (**self).serve(follower, request)
    }
}

/// What the follower decided about one shipped entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Verified and stored durably.
    Accepted,
    /// The batch's epoch is below the highest the follower stored for the shard: a superseded
    /// leader.
    Fenced {
        /// The highest epoch stored.
        highest: u64,
    },
    /// The entry's position is not the follower's next (a gap, or a repeat of an entry it
    /// holds).
    OutOfOrder {
        /// The position the follower expected.
        expected: Position,
    },
    /// The follower's journal already holds an entry with this IRI.
    Duplicate,
    /// The digest does not match the entry's content.
    BadDigest,
    /// The batch's `prev` digest is not the digest of the follower's entry at `after`: the
    /// leader's journal does not extend the follower's.
    Diverged,
}

impl Verdict {
    /// The decision of the fenced store the lease model sees, or `None` for a refusal that
    /// never reaches the store (a bad digest, a diverged batch).
    pub fn append_decision(self) -> Option<AppendDecision> {
        match self {
            Verdict::Accepted => Some(AppendDecision::Accept),
            Verdict::Fenced { .. } => Some(AppendDecision::Fenced),
            Verdict::OutOfOrder { .. } | Verdict::Duplicate => Some(AppendDecision::Conflict),
            Verdict::BadDigest | Verdict::Diverged => None,
        }
    }
}

/// The follower's decision about one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryVerdict {
    /// The entry's stated position.
    pub position: Position,
    /// The entry IRI.
    pub entry: NamedNode,
    /// The decision.
    pub verdict: Verdict,
}

/// What the follower did with one batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The follower's durable head afterwards: the position its next pull starts after.
    pub head: Position,
    /// A decision for every entry up to and including the first refused one; the entries
    /// after a refusal are not looked at.
    pub verdicts: Vec<EntryVerdict>,
}

impl Accepted {
    /// The entries stored.
    pub fn stored(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|v| v.verdict == Verdict::Accepted)
            .count()
    }

    /// The refusal that ended the batch, if any.
    pub fn refusal(&self) -> Option<&EntryVerdict> {
        self.verdicts
            .iter()
            .find(|v| v.verdict != Verdict::Accepted)
    }

    /// Writes the round into `history`: every decision the store took as an append by the
    /// batch's leader under its epoch (version = the entry's position, message = the entry
    /// IRI), and an acknowledgement for every stored entry.
    pub fn record(&self, batch: &PullBatch, history: &mut History) {
        for v in &self.verdicts {
            if let Some(decision) = v.verdict.append_decision() {
                history.record(HistoryEvent::Append {
                    node: batch.leader.clone(),
                    shard: batch.fence.shard,
                    epoch: batch.fence.epoch,
                    actor: batch.actor.as_str().to_owned(),
                    version: v.position.get(),
                    message: v.entry.as_str().to_owned(),
                    decision,
                });
            }
        }
        for v in &self.verdicts {
            if v.verdict == Verdict::Accepted {
                history.record(HistoryEvent::Ack {
                    message: v.entry.as_str().to_owned(),
                });
            }
        }
    }
}

/// The follower's way to a leader.
pub trait ShipLink: Send + Sync {
    /// The leader, for logs.
    fn leader(&self) -> String;
    /// Sends `request` and returns the leader's answer.
    ///
    /// # Errors
    ///
    /// [`PullError`]; [`PullError::Unreachable`] when the connection fails.
    fn pull(&self, request: &PullRequest) -> Result<PullBatch, PullError>;
}

/// A link to a leader in the same process, which can be cut (fault injection).
pub struct LocalShipLink<S> {
    follower: NodeId,
    source: S,
    up: Mutex<bool>,
}

impl<S> fmt::Debug for LocalShipLink<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalShipLink")
            .field("follower", &self.follower)
            .finish_non_exhaustive()
    }
}

impl<S: PullSource> LocalShipLink<S> {
    /// `follower`'s link to `source`.
    pub fn new(follower: NodeId, source: S) -> Self {
        LocalShipLink {
            follower,
            source,
            up: Mutex::new(true),
        }
    }

    /// Connects or cuts the link.
    pub fn set_up(&self, up: bool) {
        *self
            .up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = up;
    }
}

impl<S: PullSource> ShipLink for LocalShipLink<S> {
    fn leader(&self) -> String {
        format!("{} (in process)", self.source.shard())
    }

    fn pull(&self, request: &PullRequest) -> Result<PullBatch, PullError> {
        if !*self
            .up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(PullError::Unreachable("the link is cut".into()));
        }
        self.source.serve(&self.follower, request)
    }
}

/// What a catch-up achieved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatchUp {
    /// The follower's head at the end.
    pub head: Position,
    /// The pulls made.
    pub rounds: usize,
    /// The entries stored.
    pub stored: usize,
    /// The entries refused (and pulled again).
    pub refused: usize,
}

/// The follower's side of shipping one shard: pulls from its durable position, verifies, and
/// appends under the leader's fencing epoch.
// @tg: implements:: [[openspec:journal-shipping#Ordered Shipping]]
// @tg: implements:: [[openspec:journal-shipping#Resumption]]
// @tg: implements:: [[openspec:journal-shipping#Back Pressure]]
pub struct ShipFollower<J> {
    node: NodeId,
    meta: ClusterMeta,
    shard: ShardId,
    store: J,
    window: u32,
    highest: Mutex<u64>,
}

impl<J> fmt::Debug for ShipFollower<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShipFollower")
            .field("node", &self.node)
            .field("shard", &self.shard)
            .finish_non_exhaustive()
    }
}

impl<J: JournalStore> ShipFollower<J> {
    /// The follower `node` of `shard`, storing into `store` and asking for `window` bytes per
    /// pull. Its position is the store's head, so a follower made over a store that survived
    /// a restart resumes where the store ends; a store that enforces fencing also keeps the
    /// highest leader epoch across the restart.
    pub fn new(node: NodeId, meta: ClusterMeta, shard: ShardId, store: J, window: u32) -> Self {
        ShipFollower {
            node,
            meta,
            shard,
            store,
            window: window.max(1),
            highest: Mutex::new(0),
        }
    }

    /// The node.
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    /// The store.
    pub fn store(&self) -> &J {
        &self.store
    }

    /// The store, by value (a restart drops the follower and reopens it).
    pub fn into_store(self) -> J {
        self.store
    }

    /// The follower's durable position in `actor`'s journal.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub fn position(&self, actor: &NamedNode) -> Result<Position, vertep_store::StoreError> {
        self.store.head(actor)
    }

    /// The pull from the follower's durable position.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub fn request(&self, actor: &NamedNode) -> Result<PullRequest, vertep_store::StoreError> {
        Ok(PullRequest {
            shard: self.shard,
            actor: actor.clone(),
            after: self.position(actor)?,
            window: self.window,
        })
    }

    fn highest(&self) -> MutexGuard<'_, u64> {
        self.highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Verifies `batch` entry by entry — digest, fence, position — and appends the verified
    /// prefix under the leader's fencing epoch, stopping at the first refusal.
    ///
    /// # Errors
    ///
    /// [`PullError::Malformed`] for a batch of another shard, [`PullError::Store`] when the
    /// follower's store fails.
    pub fn accept(&self, batch: &PullBatch) -> Result<Accepted, PullError> {
        if batch.fence.shard != self.shard {
            return Err(PullError::Malformed(format!(
                "a batch of {} sent to the follower of {}",
                batch.fence.shard, self.shard
            )));
        }
        let store_err = |e: vertep_store::StoreError| PullError::Store(e.to_string());
        let head = self.store.head(&batch.actor).map_err(store_err)?;
        let mut verdicts = Vec::new();
        let refuse = |verdicts: &mut Vec<EntryVerdict>, e: &ShippedEntry, verdict| {
            verdicts.push(EntryVerdict {
                position: e.position,
                entry: e.entry.name().clone(),
                verdict,
            });
        };
        // the batch must extend the follower's journal, not just follow its position
        if batch.after == head
            && head > Position::ZERO
            && let Some(first) = batch.entries.first()
        {
            let own = self
                .store
                .read_range(&batch.actor, head, 1)
                .map_err(store_err)?
                .first()
                .map(|r| r.entry.digest());
            if own != batch.prev {
                refuse(&mut verdicts, first, Verdict::Diverged);
                return Ok(Accepted { head, verdicts });
            }
        }
        let highest = *self.highest();
        let mut good: Vec<&ShippedEntry> = Vec::new();
        // the first refused entry; `None` as its verdict leaves the decision to the store
        let mut refused: Option<(&ShippedEntry, Option<Verdict>)> = None;
        for e in &batch.entries {
            let expected = head.advance(good.len() as u64 + 1);
            let verdict = if e.entry.digest() != e.digest {
                Some(Some(Verdict::BadDigest))
            } else if batch.fence.epoch < highest {
                Some(Some(Verdict::Fenced { highest }))
            } else if e.entry.position() != Some(e.position) || e.position == Position::ZERO {
                Some(Some(Verdict::OutOfOrder { expected }))
            } else if e.position != expected {
                // not the follower's next: the store decides, so that a fence it keeps across
                // a restart of the follower is checked before the position, as on any append
                Some(None)
            } else {
                None
            };
            if let Some(v) = verdict {
                refused = Some((e, v));
                break;
            }
            good.push(e);
        }
        let mut new_head = head;
        if !good.is_empty() {
            match self.append(&batch.actor, head, &good, batch.fence.epoch) {
                Ok(h) => {
                    new_head = h;
                    for e in &good {
                        refuse(&mut verdicts, e, Verdict::Accepted);
                    }
                }
                Err(AppendError::Fenced { highest, .. }) => {
                    {
                        let mut h = self.highest();
                        *h = (*h).max(highest);
                    }
                    refuse(&mut verdicts, good[0], Verdict::Fenced { highest });
                    return Ok(Accepted { head, verdicts });
                }
                Err(AppendError::Conflict { actual, .. }) => {
                    // another writer moved the follower's journal: not the follower's next
                    refuse(
                        &mut verdicts,
                        good[0],
                        Verdict::OutOfOrder {
                            expected: actual.next(),
                        },
                    );
                    return Ok(Accepted { head, verdicts });
                }
                Err(AppendError::Invalid(_)) => {
                    // an entry IRI the follower already holds: find which, one entry at a time
                    let (h, v) = self.append_one_by_one(&batch.actor, head, &good, batch)?;
                    return Ok(Accepted {
                        head: h,
                        verdicts: v,
                    });
                }
                Err(other) => return Err(PullError::Store(other.to_string())),
            }
        }
        if let Some((e, v)) = refused {
            let v = match v {
                Some(v) => v,
                None => self.probe(&batch.actor, e, batch.fence.epoch, &mut new_head)?,
            };
            refuse(&mut verdicts, e, v);
        }
        Ok(Accepted {
            head: new_head,
            verdicts,
        })
    }

    /// The store's decision on an entry that is not the follower's next, appended where it
    /// says it belongs: fenced, or a conflict at the follower's head.
    fn probe(
        &self,
        actor: &NamedNode,
        e: &ShippedEntry,
        epoch: u64,
        head: &mut Position,
    ) -> Result<Verdict, PullError> {
        let at = Position::new(e.position.get() - 1);
        Ok(match self.append(actor, at, &[e], epoch) {
            // the follower's journal moved under it: the entry did extend it
            Ok(h) => {
                *head = h;
                Verdict::Accepted
            }
            Err(AppendError::Fenced { highest, .. }) => {
                let mut h = self.highest();
                *h = (*h).max(highest);
                Verdict::Fenced { highest }
            }
            Err(AppendError::Conflict { actual, .. }) => Verdict::OutOfOrder {
                expected: actual.next(),
            },
            Err(AppendError::Invalid(_)) => Verdict::Duplicate,
            Err(other) => return Err(PullError::Store(other.to_string())),
        })
    }

    fn append(
        &self,
        actor: &NamedNode,
        head: Position,
        entries: &[&ShippedEntry],
        epoch: u64,
    ) -> Result<Position, AppendError> {
        let owned: Vec<Entry> = entries.iter().map(|e| e.entry.clone()).collect();
        let meta = if self.store.supports_fencing() {
            AppendMeta::none()
                .with_fencing(FencingToken::new(self.meta.shard_scope(self.shard), epoch))
        } else {
            AppendMeta::none()
        };
        let out = self.store.append(actor, head, &owned, &meta)?;
        // like a standby, the follower's fence rises only with entries it stores
        let mut h = self.highest();
        *h = (*h).max(epoch);
        Ok(out)
    }

    fn append_one_by_one(
        &self,
        actor: &NamedNode,
        mut head: Position,
        entries: &[&ShippedEntry],
        batch: &PullBatch,
    ) -> Result<(Position, Vec<EntryVerdict>), PullError> {
        let mut verdicts = Vec::new();
        for e in entries {
            let verdict = match self.append(actor, head, &[e], batch.fence.epoch) {
                Ok(h) => {
                    head = h;
                    Verdict::Accepted
                }
                Err(AppendError::Invalid(_)) => Verdict::Duplicate,
                Err(AppendError::Fenced { highest, .. }) => Verdict::Fenced { highest },
                Err(AppendError::Conflict { actual, .. }) => Verdict::OutOfOrder {
                    expected: actual.next(),
                },
                Err(other) => return Err(PullError::Store(other.to_string())),
            };
            verdicts.push(EntryVerdict {
                position: e.position,
                entry: e.entry.name().clone(),
                verdict,
            });
            if verdict != Verdict::Accepted {
                break;
            }
        }
        Ok((head, verdicts))
    }

    /// One pull over `link` from the durable position, and its verification.
    ///
    /// # Errors
    ///
    /// The link's [`PullError`]s and the store's.
    pub fn pull_once(
        &self,
        link: &dyn ShipLink,
        actor: &NamedNode,
    ) -> Result<(PullBatch, Accepted), PullError> {
        let req = self
            .request(actor)
            .map_err(|e| PullError::Store(e.to_string()))?;
        let batch = link.pull(&req)?;
        let accepted = self.accept(&batch)?;
        Ok((batch, accepted))
    }

    /// Pulls until the follower holds the leader's head, re-pulling from its durable position
    /// after every refusal, at most `max_rounds` times; `observe` sees every round. The last
    /// pull, which finds nothing new, acknowledges the whole journal to the leader.
    ///
    /// # Errors
    ///
    /// The first [`PullError`] of the link (a dropped connection ends the catch-up; calling it
    /// again resumes from the durable position).
    pub fn catch_up(
        &self,
        link: &dyn ShipLink,
        actor: &NamedNode,
        max_rounds: usize,
        observe: &mut dyn FnMut(&PullBatch, &Accepted),
    ) -> Result<CatchUp, PullError> {
        let mut out = CatchUp::default();
        while out.rounds < max_rounds {
            let (batch, accepted) = self.pull_once(link, actor)?;
            out.rounds += 1;
            out.stored += accepted.stored();
            out.head = accepted.head;
            observe(&batch, &accepted);
            if accepted.refusal().is_some() {
                out.refused += 1;
                continue;
            }
            if batch.entries.is_empty() && accepted.head >= batch.head {
                break;
            }
        }
        Ok(out)
    }
}
