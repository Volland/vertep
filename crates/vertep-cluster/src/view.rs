//! Membership views: versioned snapshots of the members, their status and the shard owners,
//! with ordered delivery to subscribers.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use crate::meta::{ClusterId, NodeId};
use crate::placement::ShardId;

/// The status of a member in a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemberStatus {
    /// Responding and eligible for shard leases.
    Active,
    /// Missed probes; keeps its leases, gets no new ones, returns to active if it answers.
    Suspect,
    /// Leaving gracefully: gets no new leases while it hands its shards over.
    Draining,
    /// Did not answer within the failure time. Its leases are reassigned only after they
    /// expire.
    Failed,
}

impl fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MemberStatus::Active => "active",
            MemberStatus::Suspect => "suspect",
            MemberStatus::Draining => "draining",
            MemberStatus::Failed => "failed",
        })
    }
}

/// A member of a cluster view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Member {
    /// The node identifier.
    pub id: NodeId,
    /// The node's cluster transport endpoint, such as `10.0.0.5:7400`.
    pub endpoint: String,
    /// The member's status.
    pub status: MemberStatus,
}

/// The owner of a shard as last granted by the lease authority, with the lease's fencing epoch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShardOwner {
    /// The node the lease was granted to.
    pub node: NodeId,
    /// The fencing epoch of the grant.
    pub epoch: u64,
}

/// A membership view: who is a member, in which status, and which node was last granted each
/// shard, under a version number that increases strictly with every change.
#[derive(Clone, Debug, PartialEq, Eq)]
// @tg: implements:: [[openspec:cluster-membership#Membership View Versions]]
pub struct View {
    /// The cluster the view belongs to.
    pub cluster: ClusterId,
    /// The version; a node adopts a view only if its version is greater than the one it holds.
    pub version: u64,
    /// The members, by identifier. A member that left is absent.
    pub members: BTreeMap<NodeId, Member>,
    /// The last granted owner of each shard that was ever leased.
    pub owners: BTreeMap<ShardId, ShardOwner>,
}

impl View {
    /// The empty view of `cluster` at version 0.
    pub fn empty(cluster: ClusterId) -> Self {
        View {
            cluster,
            version: 0,
            members: BTreeMap::new(),
            owners: BTreeMap::new(),
        }
    }

    /// The member `id`, if present.
    pub fn member(&self, id: &NodeId) -> Option<&Member> {
        self.members.get(id)
    }

    /// The status of member `id`, if present.
    pub fn status(&self, id: &NodeId) -> Option<MemberStatus> {
        self.member(id).map(|m| m.status)
    }

    /// The active members, sorted by identifier.
    pub fn active(&self) -> impl Iterator<Item = &Member> {
        self.members
            .values()
            .filter(|m| m.status == MemberStatus::Active)
    }

    /// True when `id` is a member that may connect and send: active, suspect or draining.
    pub fn is_live(&self, id: &NodeId) -> bool {
        matches!(
            self.status(id),
            Some(MemberStatus::Active | MemberStatus::Suspect | MemberStatus::Draining)
        )
    }

    /// The last granted owner of `shard`.
    pub fn owner(&self, shard: ShardId) -> Option<&ShardOwner> {
        self.owners.get(&shard)
    }

    /// The shards last granted to `node`.
    pub fn shards_of<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = ShardId> + 'a {
        self.owners
            .iter()
            .filter(move |(_, o)| &o.node == node)
            .map(|(s, _)| *s)
    }

    /// The members and their statuses, ignoring the version and the owners: two nodes "see the
    /// same members" when these are equal.
    pub fn membership(&self) -> BTreeMap<NodeId, MemberStatus> {
        self.members
            .iter()
            .map(|(k, m)| (k.clone(), m.status))
            .collect()
    }
}

/// A subscription to view changes: every view the node adopts, in version order.
#[derive(Debug)]
pub struct Subscription {
    rx: mpsc::Receiver<View>,
}

impl Subscription {
    /// The next view if one is waiting.
    pub fn try_next(&self) -> Option<View> {
        self.rx.try_recv().ok()
    }

    /// Every view waiting, oldest first.
    pub fn drain(&self) -> Vec<View> {
        self.rx.try_iter().collect()
    }

    /// The next view, waiting up to `timeout`.
    pub fn next_timeout(&self, timeout: Duration) -> Option<View> {
        self.rx.recv_timeout(timeout).ok()
    }
}

/// A node's current view and its subscribers. A view is adopted only when its version is
/// greater than the current one, and every adopted view goes to every subscriber, in order.
#[derive(Debug)]
// @tg: implements:: [[openspec:cluster-membership#Membership View Versions]]
pub struct ViewTracker {
    current: View,
    subscribers: Vec<mpsc::Sender<View>>,
}

impl ViewTracker {
    /// A tracker holding `initial`.
    pub fn new(initial: View) -> Self {
        ViewTracker {
            current: initial,
            subscribers: Vec::new(),
        }
    }

    /// The current view.
    pub fn current(&self) -> &View {
        &self.current
    }

    /// Adopts `view` if its version is greater than the current one and notifies subscribers;
    /// a stale or equal view is ignored. Returns whether the view was adopted.
    pub fn offer(&mut self, view: View) -> bool {
        if view.version <= self.current.version || view.cluster != self.current.cluster {
            return false;
        }
        self.subscribers.retain(|s| s.send(view.clone()).is_ok());
        self.current = view;
        true
    }

    /// A new subscription; it receives the views adopted from now on.
    pub fn subscribe(&mut self) -> Subscription {
        let (tx, rx) = mpsc::channel();
        self.subscribers.push(tx);
        Subscription { rx }
    }
}
