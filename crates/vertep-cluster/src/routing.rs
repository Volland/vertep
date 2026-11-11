//! Routing between nodes: the shard table each node keeps, redirects that correct it,
//! activation only on the shard owner, forwarding with a hop limit, and deduplication of
//! retried messages.

use core::fmt;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::lease::Lease;
use crate::membership::Membership;
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;
use crate::view::{ShardOwner, View};

/// A node's answer to a message for a shard it does not own: the owner and epoch it knows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Redirect {
    /// The shard.
    pub shard: ShardId,
    /// The owner the redirecting node knows.
    pub owner: NodeId,
    /// The epoch of that owner's lease.
    pub epoch: u64,
}

/// A node's copy of the shard table: for each shard, the owner and epoch it knows. It is fed
/// by membership views and by redirects, and never goes back to a lower epoch.
// @tg: implements:: [[openspec:cluster-transport#Routing Via The Shard Table]]
// @tg: implements:: [[openspec:cluster-transport#Redirect On Stale Routing]]
#[derive(Clone, Debug)]
pub struct RoutingTable {
    meta: ClusterMeta,
    view_version: u64,
    entries: BTreeMap<ShardId, ShardOwner>,
}

impl RoutingTable {
    /// An empty table.
    pub fn new(meta: ClusterMeta) -> Self {
        RoutingTable {
            meta,
            view_version: 0,
            entries: BTreeMap::new(),
        }
    }

    fn learn(&mut self, shard: ShardId, owner: &ShardOwner) -> bool {
        match self.entries.get(&shard) {
            Some(known) if known.epoch >= owner.epoch => false,
            _ => {
                self.entries.insert(shard, owner.clone());
                true
            }
        }
    }

    /// Learns the owners of a view newer than the last one applied. Returns whether any entry
    /// changed.
    pub fn apply_view(&mut self, view: &View) -> bool {
        if view.version <= self.view_version {
            return false;
        }
        self.view_version = view.version;
        let mut changed = false;
        for (s, o) in &view.owners {
            changed |= self.learn(*s, o);
        }
        changed
    }

    /// Learns a redirect, unless the table already knows the shard at an equal or higher
    /// epoch. Returns whether the entry changed.
    pub fn apply_redirect(&mut self, r: &Redirect) -> bool {
        self.learn(
            r.shard,
            &ShardOwner {
                node: r.owner.clone(),
                epoch: r.epoch,
            },
        )
    }

    /// The known owner of `shard`.
    pub fn owner(&self, shard: ShardId) -> Option<&ShardOwner> {
        self.entries.get(&shard)
    }

    /// The shard of `actor` and its known owner: where to send a message for it.
    pub fn route(&self, actor: &str) -> (ShardId, Option<&ShardOwner>) {
        let s = self.meta.shard_of(actor);
        (s, self.owner(s))
    }
}

/// What a node does with a message for a virtual actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    /// This node holds the shard's lease: activate the actor if needed and deliver.
    Deliver(Lease),
    /// Another node owns the shard: forward there, and tell the sender with the redirect.
    Forward {
        /// The owner to forward to.
        to: NodeId,
        /// The hop count the forwarded message carries.
        hops: u8,
        /// The redirect for the sender.
        redirect: Redirect,
    },
    /// No node can take the message now (the shard is unowned, or handed to this node but not
    /// yet renewed by it): keep it in the mailbox and retry.
    Wait(ShardId),
    /// The message was forwarded as often as the hop limit allows: report a delivery error.
    HopLimit {
        /// The shard.
        shard: ShardId,
        /// The hops already taken.
        hops: u8,
    },
}

/// Why a node refuses to activate an actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotOwner {
    /// The actor's shard.
    pub shard: ShardId,
    /// The owner this node knows, if any.
    pub owner: Option<ShardOwner>,
}

impl fmt::Display for NotOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.owner {
            Some(o) => write!(
                f,
                "this node does not lease {}; its owner is {} at epoch {}",
                self.shard, o.node, o.epoch
            ),
            None => write!(f, "this node does not lease {}", self.shard),
        }
    }
}

impl std::error::Error for NotOwner {}

/// The activation check of a node: an actor is activated only while the node holds an
/// unexpired lease on its shard, and a message for any other shard is forwarded to the owner
/// up to a hop limit.
// @tg: implements:: [[openspec:shard-placement#Activation On Shard Owner Only]]
// @tg: implements:: [[openspec:shard-placement#Message Forwarding To Shard Owner]]
// @tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]
pub struct ActivationGate<M: ?Sized> {
    membership: Arc<M>,
    hop_limit: u8,
}

impl<M: ?Sized> fmt::Debug for ActivationGate<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivationGate")
            .field("hop_limit", &self.hop_limit)
            .finish_non_exhaustive()
    }
}

impl<M: Membership + ?Sized> ActivationGate<M> {
    /// The gate of `membership`'s node, forwarding at most `hop_limit` times.
    pub fn new(membership: Arc<M>, hop_limit: u8) -> Self {
        ActivationGate {
            membership,
            hop_limit,
        }
    }

    /// The lease under which `actor` may be activated here.
    ///
    /// # Errors
    ///
    /// [`NotOwner`] when this node does not hold its shard's lease.
    pub fn may_activate(&self, actor: &str) -> Result<Lease, NotOwner> {
        let shard = self.membership.meta().shard_of(actor);
        self.membership.held(shard).ok_or_else(|| NotOwner {
            shard,
            owner: self.membership.view().owner(shard).cloned(),
        })
    }

    /// What to do with a message for `actor` that has already been forwarded `hops` times.
    pub fn inbound(&self, actor: &str, hops: u8) -> Inbound {
        let refusal = match self.may_activate(actor) {
            Ok(lease) => return Inbound::Deliver(lease),
            Err(e) => e,
        };
        let Some(owner) = refusal.owner.filter(|o| &o.node != self.membership.local()) else {
            return Inbound::Wait(refusal.shard);
        };
        if hops >= self.hop_limit {
            return Inbound::HopLimit {
                shard: refusal.shard,
                hops,
            };
        }
        Inbound::Forward {
            to: owner.node.clone(),
            hops: hops + 1,
            redirect: Redirect {
                shard: refusal.shard,
                owner: owner.node,
                epoch: owner.epoch,
            },
        }
    }
}

/// A bounded set of the message identifiers a receiver has accepted per target, so a retried
/// message is acknowledged and discarded instead of processed twice. Journaled actors also
/// deduplicate in their journal; this covers mailboxes that are not journaled.
// @tg: implements:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication]] {confidence: partial}
#[derive(Debug)]
pub struct DedupCache {
    capacity: usize,
    seen: HashSet<(String, String)>,
    order: VecDeque<(String, String)>,
}

impl DedupCache {
    /// A cache remembering the last `capacity` messages.
    pub fn new(capacity: usize) -> Self {
        DedupCache {
            capacity: capacity.max(1),
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// True when message `id` was already accepted for `target`.
    pub fn contains(&self, target: &str, id: &str) -> bool {
        self.seen.contains(&(target.to_owned(), id.to_owned()))
    }

    /// Records message `id` for `target`; false when it was already accepted.
    pub fn first_time(&mut self, target: &str, id: &str) -> bool {
        let key = (target.to_owned(), id.to_owned());
        if self.seen.contains(&key) {
            return false;
        }
        if self.order.len() == self.capacity
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        self.seen.insert(key.clone());
        self.order.push_back(key);
        true
    }
}
