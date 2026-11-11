//! Journal replication to standbys.
//!
//! With a replication factor `rf` above one, every shard has `rf - 1` standbys besides its
//! owner: the members that follow the owner in the sorted list of active members
//! ([`replica_set`]). The owner's [`ReplicatedJournal`] appends to its own journal, then ships
//! the actor's journal suffix each standby lacks, and acknowledges the append only after the
//! configured number of standbys stored it. A standby's [`StandbyReplica`] keeps the highest
//! epoch it has accepted for the shard and refuses a stream from a lower one, so a stale
//! owner cannot extend a replica either.
//!
//! When the quorum is not reached the append is refused with `Unavailable` and the actor's
//! journal on the owner is marked behind: it can be neither read nor extended until a later
//! shipment brings enough standbys level again ([`ReplicatedJournal::catch_up`]). An entry
//! that never reached the quorum therefore never produces effects, so a standby that takes
//! over after the owner fails holds every entry whose effects anyone saw.
//!
//! With a replication factor of one there is no standby: a failed owner's shards stay
//! unavailable until its disk comes back, and [`may_take`] keeps every other node from
//! taking such a shard with an empty journal.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use vertep_store::oxrdf::NamedNode;
use vertep_store::{
    AppendError, AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, Position,
    StoreError,
};

use crate::fencing::{Fence, ShardJournal};
use crate::membership::Membership;
use crate::meta::{ClusterMeta, NodeId, ShardCount};
use crate::placement::{ShardId, assignment};
use crate::view::View;

/// The owner and standbys of every shard under `view`: the owner the deterministic
/// assignment gives, then the next `rf - 1` active members in identifier order, wrapping.
pub fn replica_set(view: &View, count: ShardCount, rf: u32) -> BTreeMap<ShardId, Vec<NodeId>> {
    let members: Vec<NodeId> = view.active().map(|m| m.id.clone()).collect();
    let mut out = BTreeMap::new();
    for (shard, owner) in assignment(view, count) {
        let Some(at) = members.iter().position(|m| m == &owner) else {
            continue;
        };
        let k = (rf.max(1) as usize).min(members.len());
        let set: Vec<NodeId> = (0..k)
            .map(|i| members[(at + i) % members.len()].clone())
            .collect();
        out.insert(shard, set);
    }
    out
}

/// Whether a node may take a free shard: only when it holds the shard's journal (it owned
/// the shard, received it by shipping, or keeps a replica of it) or the shard was never
/// owned. With a replication factor of one this keeps a failed owner's shards unavailable
/// instead of letting another node start them from an empty journal.
pub fn may_take(ever_owned: bool, holds_journal: bool) -> bool {
    !ever_owned || holds_journal
}

/// One actor's journal suffix shipped from a shard's owner to a standby.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplicationBatch {
    /// The shard and the epoch of the owner's lease.
    pub fence: Fence,
    /// The actor.
    pub actor: NamedNode,
    /// The position the first record follows.
    pub after: Position,
    /// The entries, in order.
    pub entries: Vec<Entry>,
}

/// Why a standby did not store a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaError {
    /// The standby cannot be reached.
    Unreachable(String),
    /// The batch's epoch is below the highest the standby has accepted for the shard.
    Fenced {
        /// The batch's epoch.
        epoch: u64,
        /// The highest accepted.
        highest: u64,
    },
    /// The standby's journal of the actor ends before the batch starts: resend from `head`.
    Behind {
        /// The standby's head.
        head: Position,
    },
    /// The standby holds a different entry at a position the batch covers.
    Diverged(Position),
    /// The standby's store failed.
    Store(String),
}

impl fmt::Display for ReplicaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplicaError::Unreachable(m) => write!(f, "standby unreachable: {m}"),
            ReplicaError::Fenced { epoch, highest } => {
                write!(f, "replication at epoch {epoch} fenced by epoch {highest}")
            }
            ReplicaError::Behind { head } => write!(f, "standby is behind at {}", head.get()),
            ReplicaError::Diverged(p) => write!(f, "standby diverges at {}", p.get()),
            ReplicaError::Store(m) => write!(f, "standby store: {m}"),
        }
    }
}

impl std::error::Error for ReplicaError {}

/// Where an owner ships a shard's appends: one standby.
pub trait StandbyLink: Send + Sync {
    /// The standby node.
    fn node(&self) -> &NodeId;
    /// Ships `batch`; returns the standby's head for the actor after storing it.
    ///
    /// # Errors
    ///
    /// [`ReplicaError`].
    fn replicate(&self, batch: &ReplicationBatch) -> Result<Position, ReplicaError>;
}

/// A standby's copy of one shard's journal: accepts batches in order from the current owner
/// and refuses batches under a lower epoch than it has accepted.
// @tg: implements:: [[openspec:shard-placement#Journal Replication To Standbys]]
// @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
pub struct StandbyReplica<J> {
    shard: ShardId,
    meta: ClusterMeta,
    store: J,
    highest: Mutex<u64>,
}

impl<J> fmt::Debug for StandbyReplica<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandbyReplica")
            .field("shard", &self.shard)
            .finish_non_exhaustive()
    }
}

impl<J: JournalStore> StandbyReplica<J> {
    /// The replica of `shard` kept in `store`.
    pub fn new(meta: ClusterMeta, shard: ShardId, store: J) -> Self {
        StandbyReplica {
            shard,
            meta,
            store,
            highest: Mutex::new(0),
        }
    }

    /// The replica's store, which becomes the shard's journal when this node takes over.
    pub fn store(&self) -> &J {
        &self.store
    }

    /// The replica's store, by value.
    pub fn into_store(self) -> J {
        self.store
    }

    /// The highest epoch accepted.
    pub fn highest_epoch(&self) -> u64 {
        *self
            .highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Stores `batch`: entries the replica already holds are checked and skipped, the rest
    /// appended in order under the batch's epoch.
    ///
    /// # Errors
    ///
    /// [`ReplicaError::Fenced`] for a lower epoch, [`ReplicaError::Behind`] when the replica
    /// lacks entries before the batch, [`ReplicaError::Diverged`] for a different entry at a
    /// covered position, [`ReplicaError::Store`] when the store fails.
    pub fn receive(&self, batch: &ReplicationBatch) -> Result<Position, ReplicaError> {
        if batch.fence.shard != self.shard {
            return Err(ReplicaError::Store(format!(
                "batch for {} sent to the replica of {}",
                batch.fence.shard, self.shard
            )));
        }
        let mut highest = self
            .highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if batch.fence.epoch < *highest {
            return Err(ReplicaError::Fenced {
                epoch: batch.fence.epoch,
                highest: *highest,
            });
        }
        *highest = batch.fence.epoch;
        let store_err = |e: StoreError| ReplicaError::Store(e.to_string());
        let mut head = self.store.head(&batch.actor).map_err(store_err)?;
        if head < batch.after {
            return Err(ReplicaError::Behind { head });
        }
        let held = head.get() - batch.after.get();
        if held > 0 {
            let existing = self
                .store
                .read(&batch.actor, batch.after.next())
                .map_err(store_err)?;
            for (e, r) in batch.entries.iter().zip(&existing) {
                if !e.is_isomorphic(&r.entry) {
                    return Err(ReplicaError::Diverged(r.position));
                }
            }
        }
        let rest: Vec<Entry> = batch.entries.iter().skip(held as usize).cloned().collect();
        if !rest.is_empty() {
            let meta = AppendMeta::none().with_fencing(vertep_store::FencingToken::new(
                self.meta.shard_scope(self.shard),
                batch.fence.epoch,
            ));
            head = self
                .store
                .append(&batch.actor, head, &rest, &meta)
                .map_err(|e| match e {
                    AppendError::Fenced { epoch, highest, .. } => {
                        ReplicaError::Fenced { epoch, highest }
                    }
                    other => ReplicaError::Store(other.to_string()),
                })?;
        }
        Ok(head)
    }
}

/// A standby reached in memory (tests, and a node that is its own standby).
pub struct LocalStandby<J> {
    node: NodeId,
    replica: Arc<StandbyReplica<J>>,
    up: Mutex<bool>,
}

impl<J> fmt::Debug for LocalStandby<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalStandby")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl<J: JournalStore> LocalStandby<J> {
    /// The link to `replica` on `node`.
    pub fn new(node: NodeId, replica: Arc<StandbyReplica<J>>) -> Self {
        LocalStandby {
            node,
            replica,
            up: Mutex::new(true),
        }
    }

    /// Connects or disconnects the link (fault injection).
    pub fn set_up(&self, up: bool) {
        *self
            .up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = up;
    }
}

impl<J: JournalStore> StandbyLink for LocalStandby<J> {
    fn node(&self) -> &NodeId {
        &self.node
    }

    fn replicate(&self, batch: &ReplicationBatch) -> Result<Position, ReplicaError> {
        if !*self
            .up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(ReplicaError::Unreachable(format!(
                "{} is disconnected",
                self.node
            )));
        }
        self.replica.receive(batch)
    }
}

struct Shipping {
    /// The head each standby is known to hold, per actor.
    acked: BTreeMap<(NodeId, NamedNode), Position>,
    /// Actors whose last append did not reach the quorum.
    behind: BTreeSet<NamedNode>,
}

/// The owner's journal of one shard with replication: an append is acknowledged only after
/// `required` standbys have stored it.
// @tg: implements:: [[openspec:shard-placement#Journal Replication To Standbys]]
pub struct ReplicatedJournal<J, M: ?Sized> {
    inner: ShardJournal<J, M>,
    standbys: Vec<Arc<dyn StandbyLink>>,
    required: usize,
    state: Mutex<Shipping>,
}

impl<J, M: ?Sized> fmt::Debug for ReplicatedJournal<J, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicatedJournal")
            .field("required", &self.required)
            .finish_non_exhaustive()
    }
}

impl<J: JournalStore, M: Membership + ?Sized> ReplicatedJournal<J, M> {
    /// The fenced journal `inner`, replicated to `standbys`, acknowledging an append once
    /// `required` of them stored it (the cluster's replication factor minus one).
    pub fn new(
        inner: ShardJournal<J, M>,
        standbys: Vec<Arc<dyn StandbyLink>>,
        required: usize,
    ) -> Self {
        ReplicatedJournal {
            inner,
            standbys,
            required,
            state: Mutex::new(Shipping {
                acked: BTreeMap::new(),
                behind: BTreeSet::new(),
            }),
        }
    }

    /// The fenced journal underneath.
    pub fn inner(&self) -> &ShardJournal<J, M> {
        &self.inner
    }

    fn lock(&self) -> MutexGuard<'_, Shipping> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ships `actor`'s suffix to every standby; returns how many hold the whole journal.
    fn ship(&self, actor: &NamedNode) -> Result<usize, StoreError> {
        let Some(fence) = self.inner.fence() else {
            return Ok(0);
        };
        let head = self.inner.store().head(actor)?;
        let mut acks = 0;
        for link in &self.standbys {
            let key = (link.node().clone(), actor.clone());
            let mut from = self
                .lock()
                .acked
                .get(&key)
                .copied()
                .unwrap_or(Position::ZERO);
            for _ in 0..2 {
                let entries: Vec<Entry> = self
                    .inner
                    .store()
                    .read(actor, from.next())?
                    .into_iter()
                    .map(|r| r.entry)
                    .collect();
                let batch = ReplicationBatch {
                    fence,
                    actor: actor.clone(),
                    after: from,
                    entries,
                };
                match link.replicate(&batch) {
                    Ok(p) => {
                        self.lock().acked.insert(key.clone(), p);
                        if p >= head {
                            acks += 1;
                        }
                        break;
                    }
                    Err(ReplicaError::Behind { head }) => from = head,
                    Err(_) => break,
                }
            }
        }
        Ok(acks)
    }

    /// Ships what the standbys lack of `actor`'s journal; once the quorum holds all of it the
    /// actor's journal is usable again. Returns whether it is.
    ///
    /// # Errors
    ///
    /// The local store's read errors.
    pub fn catch_up(&self, actor: &NamedNode) -> Result<bool, StoreError> {
        let ok = self.ship(actor)? >= self.required;
        if ok {
            self.lock().behind.remove(actor);
        }
        Ok(ok)
    }

    fn usable(&self, actor: &NamedNode) -> Result<(), StoreError> {
        if !self.lock().behind.contains(actor) || self.catch_up(actor)? {
            Ok(())
        } else {
            Err(StoreError::Unavailable(format!(
                "the journal of {actor} is not yet stored by {} standbys",
                self.required
            )))
        }
    }
}

impl<J: JournalStore, M: Membership + ?Sized> JournalStore for ReplicatedJournal<J, M> {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.usable(actor).map_err(AppendError::Storage)?;
        let pos = self.inner.append(actor, expected, entries, meta)?;
        let acks = self.ship(actor).map_err(AppendError::Storage)?;
        if acks < self.required {
            self.lock().behind.insert(actor.clone());
            return Err(AppendError::Storage(StoreError::Unavailable(format!(
                "replication quorum not reached: {acks} of {} standbys stored the entry",
                self.required
            ))));
        }
        Ok(pos)
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.usable(actor)?;
        self.inner.read(actor, from)
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.usable(actor)?;
        self.inner.head(actor)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.inner.actors()
    }
}
