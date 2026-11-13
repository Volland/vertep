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

/// The standbys of a shard owned by `owner` under `view`: the next `rf - 1` active members
/// after the owner in identifier order, wrapping (fewer when fewer are active). A new owner
/// replicates to these, so after a failover a member replaces the failed one.
pub fn standbys_of(view: &View, owner: &NodeId, rf: u32) -> Vec<NodeId> {
    let mut members: Vec<NodeId> = view.active().map(|m| m.id.clone()).collect();
    if !members.contains(owner) {
        members.push(owner.clone());
        members.sort();
    }
    let at = members.iter().position(|m| m == owner).unwrap_or(0);
    let k = (rf.max(1) as usize).min(members.len());
    (1..k)
        .map(|i| members[(at + i) % members.len()].clone())
        .collect()
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

/// A link to one replica of a journal, generic over the batch it carries: the cluster's
/// standbys take [`ReplicationBatch`]es over the cluster transport, and a durable object's
/// custodians take custody batches over TSP (`vertep-objects`). The suffix shipping both use is
/// [`ship_suffix`].
pub trait ReplicaLink<B>: Send + Sync {
    /// The replica, for logs and acknowledgement bookkeeping.
    fn replica(&self) -> String;
    /// Ships `batch`; returns the replica's head for the batch's journal after storing it.
    ///
    /// # Errors
    ///
    /// [`ReplicaError`]; [`ReplicaError::Behind`] asks for a resend from the replica's head.
    fn replicate(&self, batch: &B) -> Result<Position, ReplicaError>;
}

/// What shipping a journal suffix to one replica achieved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShipOutcome {
    /// The replica stored the suffix; its head is now this position.
    Stored(Position),
    /// The replica has accepted a higher epoch than the batch's.
    Fenced(u64),
    /// The replica did not store the suffix (unreachable, diverged, store failure, or still
    /// behind after one resend).
    Failed(ReplicaError),
}

/// Ships to `link` the entries after `from` (the head it is believed to hold): reads them with
/// `read` (entries from a position on, inclusive), wraps them with `batch` (the position they
/// follow and the entries) and, when the replica answers [`ReplicaError::Behind`], resends once
/// from the head it reported.
///
/// # Errors
///
/// The errors of `read`.
pub fn ship_suffix<B, E, L: ReplicaLink<B> + ?Sized>(
    link: &L,
    from: Position,
    mut read: impl FnMut(Position) -> Result<Vec<Entry>, E>,
    mut batch: impl FnMut(Position, Vec<Entry>) -> B,
) -> Result<ShipOutcome, E> {
    let mut from = from;
    let mut last = ReplicaError::Unreachable(link.replica());
    for _ in 0..2 {
        let entries = read(from.next())?;
        match link.replicate(&batch(from, entries)) {
            Ok(p) => return Ok(ShipOutcome::Stored(p)),
            Err(ReplicaError::Behind { head }) => {
                last = ReplicaError::Behind { head };
                from = head;
            }
            Err(ReplicaError::Fenced { highest, .. }) => return Ok(ShipOutcome::Fenced(highest)),
            Err(e) => return Ok(ShipOutcome::Failed(e)),
        }
    }
    Ok(ShipOutcome::Failed(last))
}

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
    /// appended in order under the batch's epoch, which then becomes the replica's fence.
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
            // like a fenced journal store, the replica's fence rises with the entries it
            // stores; a batch that only checks the replica (an owner's pre-flight, a catch-up
            // with nothing new) does not fence anyone
            *highest = batch.fence.epoch;
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

impl<T: StandbyLink + ?Sized> ReplicaLink<ReplicationBatch> for T {
    fn replica(&self) -> String {
        self.node().to_string()
    }
    fn replicate(&self, batch: &ReplicationBatch) -> Result<Position, ReplicaError> {
        StandbyLink::replicate(self, batch)
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
    standbys: Mutex<Vec<Arc<dyn StandbyLink>>>,
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
            standbys: Mutex::new(standbys),
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

    /// Replicates to `standbys` from now on: what a new owner does when it takes the shard
    /// (the shard's current standbys, see [`standbys_of`]). A standby new to the journal gets
    /// the whole journal with the next shipment.
    pub fn set_standbys(&self, standbys: Vec<Arc<dyn StandbyLink>>) {
        let nodes: BTreeSet<NodeId> = standbys.iter().map(|l| l.node().clone()).collect();
        self.lock().acked.retain(|(n, _), _| nodes.contains(n));
        *self
            .standbys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = standbys;
    }

    /// The standbys this journal replicates to.
    pub fn standbys(&self) -> Vec<NodeId> {
        self.links().iter().map(|l| l.node().clone()).collect()
    }

    fn links(&self) -> Vec<Arc<dyn StandbyLink>> {
        self.standbys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn lock(&self) -> MutexGuard<'_, Shipping> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ships `actor`'s suffix to every standby; returns how many hold the whole journal.
    fn ship(&self, actor: &NamedNode) -> Result<usize, StoreError> {
        Ok(self.ship_fenced(actor)?.0)
    }

    /// Ships `actor`'s suffix to every standby; returns how many hold the whole journal, and
    /// the highest epoch of a standby that fenced the batch, if one did.
    fn ship_fenced(&self, actor: &NamedNode) -> Result<(usize, Option<u64>), StoreError> {
        let Some(fence) = self.inner.fence() else {
            return Ok((0, None));
        };
        let head = self.inner.store().head(actor)?;
        let mut acks = 0;
        let mut fenced = None;
        for link in &self.links() {
            let key = (link.node().clone(), actor.clone());
            let from = self
                .lock()
                .acked
                .get(&key)
                .copied()
                .unwrap_or(Position::ZERO);
            let outcome = ship_suffix(
                link.as_ref(),
                from,
                |p| {
                    Ok::<_, StoreError>(
                        self.inner
                            .store()
                            .read(actor, p)?
                            .into_iter()
                            .map(|r| r.entry)
                            .collect(),
                    )
                },
                |after, entries| ReplicationBatch {
                    fence,
                    actor: actor.clone(),
                    after,
                    entries,
                },
            )?;
            match outcome {
                ShipOutcome::Stored(p) => {
                    self.lock().acked.insert(key, p);
                    if p >= head {
                        acks += 1;
                    }
                }
                ShipOutcome::Fenced(highest) => {
                    fenced = Some(fenced.map_or(highest, |h: u64| h.max(highest)));
                }
                ShipOutcome::Failed(_) => {}
            }
        }
        Ok((acks, fenced))
    }

    /// Before the owner writes its own copy: brings the standbys level with it and checks that
    /// `required` of them hold the whole journal under the owner's epoch. A standby that has
    /// seen a newer epoch fences the owner, and an unreachable quorum stops the append, both
    /// before anything is written, so the owner's copy never gets an entry the standbys refused.
    fn prepare(&self, actor: &NamedNode) -> Result<(), AppendError> {
        if self.required == 0 {
            return Ok(());
        }
        let Some(fence) = self.inner.fence() else {
            return Ok(()); // no lease: the fenced journal refuses the append itself
        };
        let (acks, fenced) = self.ship_fenced(actor).map_err(AppendError::Storage)?;
        if let Some(highest) = fenced {
            self.inner.superseded();
            return Err(AppendError::Fenced {
                scope: self.inner.scope(),
                epoch: fence.epoch,
                highest,
            });
        }
        if acks < self.required {
            return Err(AppendError::Storage(StoreError::Unavailable(format!(
                "replication quorum unreachable: {acks} of {} standbys hold the journal of {actor}",
                self.required
            ))));
        }
        Ok(())
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
        self.prepare(actor)?;
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
