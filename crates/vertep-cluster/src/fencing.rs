//! Fencing: the epoch of the shard lease on every journal append and every outbound frame,
//! and the receivers that reject lower epochs.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use vertep_store::oxrdf::NamedNode;
use vertep_store::{
    AppendError, AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, Position,
    StoreError,
};

use crate::membership::Membership;
use crate::meta::ClusterMeta;
use crate::placement::ShardId;

/// The `(shard, epoch)` a virtual actor's output carries: the lease under which it was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fence {
    /// The shard of the sending actor.
    pub shard: ShardId,
    /// The epoch of the sender's lease on it.
    pub epoch: u64,
}

/// The journal store of one shard on its owner: every append carries the fencing token of the
/// node's current lease on the shard, and an append is refused when the node does not hold the
/// lease by its own clock.
///
/// When the store fences an append (a newer owner appended at a higher epoch), the lease is
/// dropped through [`Membership::forget`], so the node stops processing the shard and
/// deactivates its actors.
// @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
// @tg: implements:: [[openspec:shard-placement#Activation On Shard Owner Only]]
pub struct ShardJournal<J, M: ?Sized> {
    store: J,
    shard: ShardId,
    meta: ClusterMeta,
    membership: Arc<M>,
}

impl<J, M: ?Sized> fmt::Debug for ShardJournal<J, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardJournal")
            .field("shard", &self.shard)
            .finish_non_exhaustive()
    }
}

/// Why a [`ShardJournal`] cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShardJournalError {
    /// The store does not enforce fencing tokens, so the backstop would be missing.
    NoFencing,
}

impl fmt::Display for ShardJournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShardJournalError::NoFencing => {
                f.write_str("a cluster shard journal needs a store that enforces fencing tokens")
            }
        }
    }
}

impl std::error::Error for ShardJournalError {}

impl<J: JournalStore, M: Membership + ?Sized> ShardJournal<J, M> {
    /// The journal of `shard` in `store`, fenced by the leases of `membership`.
    ///
    /// # Errors
    ///
    /// [`ShardJournalError::NoFencing`] when `store` does not enforce fencing.
    pub fn new(store: J, shard: ShardId, membership: Arc<M>) -> Result<Self, ShardJournalError> {
        if !store.supports_fencing() {
            return Err(ShardJournalError::NoFencing);
        }
        let meta = membership.meta().clone();
        Ok(ShardJournal {
            store,
            shard,
            meta,
            membership,
        })
    }

    /// The shard.
    pub fn shard(&self) -> ShardId {
        self.shard
    }

    /// The underlying store.
    pub fn store(&self) -> &J {
        &self.store
    }

    /// The fence this node's output for the shard carries now, if it holds the lease.
    pub fn fence(&self) -> Option<Fence> {
        self.membership.held(self.shard).map(|l| Fence {
            shard: self.shard,
            epoch: l.epoch,
        })
    }

    /// The fencing scope of the shard: what appends' fencing tokens name.
    pub fn scope(&self) -> NamedNode {
        self.meta.shard_scope(self.shard)
    }

    /// Drops this node's lease on the shard after a replication receiver showed it
    /// superseded, as a fenced append does: the node stops processing the shard.
    pub fn superseded(&self) {
        self.membership.forget(self.shard);
    }
}

impl<J: JournalStore, M: Membership + ?Sized> JournalStore for ShardJournal<J, M> {
    fn capabilities(&self) -> JournalCapabilities {
        self.store.capabilities()
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        let shard = self.meta.shard_of(actor.as_str());
        if shard != self.shard {
            return Err(AppendError::Invalid(format!(
                "{actor} belongs to {shard}, not to {}",
                self.shard
            )));
        }
        let Some(lease) = self.membership.held(self.shard) else {
            return Err(AppendError::Storage(StoreError::Unavailable(format!(
                "{} does not hold the lease on {}",
                self.membership.local(),
                self.shard
            ))));
        };
        let meta = meta.clone().with_fencing(lease.fencing_token(&self.meta));
        let out = self.store.append(actor, expected, entries, &meta);
        if let Err(AppendError::Fenced { .. }) = &out {
            self.membership.forget(self.shard);
        }
        out
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.store.read(actor, from)
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.store.head(actor)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.store.actors()
    }
}

/// A message dropped because it was produced under a superseded lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FencedMessage {
    /// The message identifier.
    pub id: String,
    /// The sending actor.
    pub sender: String,
    /// The target actor.
    pub target: String,
    /// The event name.
    pub event: String,
    /// The fence the message carried.
    pub fence: Fence,
    /// The highest epoch the receiver had seen for the shard.
    pub highest: u64,
}

/// Where a receiver records fenced messages: the runtime's dead-letter graph.
pub trait DeadLetters: Send + Sync {
    /// Records `message` as dropped for fencing.
    fn fenced(&self, message: FencedMessage);
}

/// Dead letters kept in memory.
#[derive(Debug, Default)]
pub struct MemoryDeadLetters(Mutex<Vec<FencedMessage>>);

impl MemoryDeadLetters {
    /// Every fenced message recorded, oldest first.
    pub fn fenced_messages(&self) -> Vec<FencedMessage> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl DeadLetters for MemoryDeadLetters {
    fn fenced(&self, message: FencedMessage) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
    }
}

/// The highest epoch a receiver has seen per shard. A message carrying a lower epoch was
/// produced by an owner that lost its lease, and is dropped.
// @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
#[derive(Debug, Default)]
pub struct EpochFilter {
    highest: Mutex<BTreeMap<ShardId, u64>>,
}

impl EpochFilter {
    /// A filter that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits a message under `fence` and raises the shard's highest epoch to it, or returns
    /// the higher epoch already seen.
    ///
    /// # Errors
    ///
    /// The highest epoch seen, when `fence.epoch` is lower.
    pub fn admit(&self, fence: Fence) -> Result<(), u64> {
        let mut h = self
            .highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let e = h.entry(fence.shard).or_insert(0);
        if fence.epoch < *e {
            return Err(*e);
        }
        *e = fence.epoch;
        Ok(())
    }

    /// Raises the highest epoch known for `shard` without a message, for example from a view.
    pub fn observe(&self, fence: Fence) {
        let mut h = self
            .highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let e = h.entry(fence.shard).or_insert(0);
        *e = (*e).max(fence.epoch);
    }

    /// The highest epoch seen for `shard`.
    pub fn highest(&self, shard: ShardId) -> u64 {
        self.highest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&shard)
            .copied()
            .unwrap_or(0)
    }

    /// Admits a message, or records it in `dead` as fenced. Returns whether it was admitted.
    pub fn admit_or_dead_letter(
        &self,
        id: &str,
        sender: &str,
        target: &str,
        event: &str,
        fence: Fence,
        dead: &dyn DeadLetters,
    ) -> bool {
        match self.admit(fence) {
            Ok(()) => true,
            Err(highest) => {
                dead.fenced(FencedMessage {
                    id: id.to_owned(),
                    sender: sender.to_owned(),
                    target: target.to_owned(),
                    event: event.to_owned(),
                    fence,
                    highest,
                });
                false
            }
        }
    }
}
