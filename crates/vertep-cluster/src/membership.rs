//! The membership interface every implementation provides, and what joining and leaving share.

use core::fmt;
use std::time::Duration;

use crate::lease::{Lease, LeaseError};
use crate::meta::{ClusterId, ClusterMeta, NodeId, ShardCount};
use crate::placement::ShardId;
use crate::view::{Subscription, View};

/// Cluster membership: the current view, view change notifications, and the shard leases
/// with their fencing epochs.
///
/// The runtime and the rest of this crate use only this trait, so the static, gossip and
/// external implementations are interchangeable. Lease operations are non-blocking: an
/// implementation whose lease authority is remote may answer [`LeaseError::Pending`] after
/// sending the request, and the caller asks again later.
// @tg: implements:: [[openspec:cluster-membership#Membership Trait]]
pub trait Membership: Send + Sync {
    /// This node.
    fn local(&self) -> &NodeId;

    /// The cluster metadata this node runs with.
    fn meta(&self) -> &ClusterMeta;

    /// The current view.
    fn view(&self) -> View;

    /// A subscription to the views this node adopts from now on, in version order.
    fn subscribe(&self) -> Subscription;

    /// Asks for the lease on `shard`. Acquiring a shard this node already holds renews it.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the lease authority.
    fn acquire(&self, shard: ShardId) -> Result<Lease, LeaseError>;

    /// Renews `lease`, keeping its epoch.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the lease authority.
    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError>;

    /// Gives `lease` back; the shard is free at once.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the lease authority.
    fn release(&self, lease: &Lease) -> Result<(), LeaseError>;

    /// Revokes the current lease of `shard`, whoever holds it.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the lease authority.
    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError>;

    /// Hands `lease` to `to` at a higher epoch, after this node drained the shard; this node
    /// stops holding it at once. `to` holds it once it renews it (on its next tick).
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the lease authority.
    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError>;

    /// The lease on `shard` if this node holds it now by its own clock, a safety margin before
    /// its expiry. A node starts macrosteps for a shard's actors only while this is `Some`.
    fn held(&self, shard: ShardId) -> Option<Lease>;

    /// Drops the local record of the lease on `shard`, after a fenced append showed it
    /// superseded: the node deactivates the shard's actors and stops processing it.
    fn forget(&self, shard: ShardId);
}

/// What a node presents to join: the cluster identifier, its own identifier and endpoint, and
/// the shard count and hash version it was configured with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequest {
    /// The cluster the node wants to join.
    pub cluster: ClusterId,
    /// The node.
    pub node: NodeId,
    /// The node's transport endpoint.
    pub endpoint: String,
    /// The shard count the node is configured with.
    pub shard_count: ShardCount,
    /// The number of the hash version the node is built with ([`crate::HashVersion::number`]).
    pub hash_version: u32,
}

impl JoinRequest {
    /// The request of `node` at `endpoint`, configured with `meta`.
    pub fn new(meta: &ClusterMeta, node: NodeId, endpoint: impl Into<String>) -> Self {
        JoinRequest {
            cluster: meta.cluster.clone(),
            node,
            endpoint: endpoint.into(),
            shard_count: meta.shard_count,
            hash_version: meta.hash_version.number(),
        }
    }
}

/// Why a join was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    /// The node asked to join another cluster.
    ClusterMismatch {
        /// The cluster's identifier.
        cluster: ClusterId,
        /// The identifier the node presented.
        requested: ClusterId,
    },
    /// The node is configured with another shard count.
    ShardCountMismatch {
        /// The cluster's recorded shard count.
        cluster: ShardCount,
        /// The node's configured shard count.
        node: ShardCount,
    },
    /// The node is built with another IRI hash version.
    HashVersionMismatch {
        /// The cluster's recorded hash version number.
        cluster: u32,
        /// The node's hash version number.
        node: u32,
    },
    /// A member with this identifier is still active.
    DuplicateNode(NodeId),
    /// The node is not in the static member list.
    NotListed(NodeId),
    /// The membership authority cannot be reached.
    Unavailable(String),
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::ClusterMismatch { cluster, requested } => write!(
                f,
                "join refused: the node asked for cluster {requested}, this is cluster {cluster}"
            ),
            JoinError::ShardCountMismatch { cluster, node } => write!(
                f,
                "join refused: the node is configured with {node} shards, the cluster was \
                 created with {cluster} shards"
            ),
            JoinError::HashVersionMismatch { cluster, node } => write!(
                f,
                "join refused: the node uses shard hash version {node}, the cluster uses \
                 version {cluster}"
            ),
            JoinError::DuplicateNode(n) => {
                write!(f, "join refused: member {n} is still active")
            }
            JoinError::NotListed(n) => {
                write!(f, "join refused: {n} is not in the static member list")
            }
            JoinError::Unavailable(m) => write!(f, "join failed: {m}"),
        }
    }
}

impl std::error::Error for JoinError {}

/// The checks every implementation makes before admitting a node: same cluster, same shard
/// count, same hash version.
///
/// # Errors
///
/// The first mismatch found.
// @tg: implements:: [[openspec:cluster-membership#Graceful Join]]
// @tg: implements:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash]]
pub fn check_compatible(meta: &ClusterMeta, req: &JoinRequest) -> Result<(), JoinError> {
    if req.cluster != meta.cluster {
        return Err(JoinError::ClusterMismatch {
            cluster: meta.cluster.clone(),
            requested: req.cluster.clone(),
        });
    }
    if req.shard_count != meta.shard_count {
        return Err(JoinError::ShardCountMismatch {
            cluster: meta.shard_count,
            node: req.shard_count,
        });
    }
    if req.hash_version != meta.hash_version.number() {
        return Err(JoinError::HashVersionMismatch {
            cluster: meta.hash_version.number(),
            node: req.hash_version,
        });
    }
    Ok(())
}

/// What a leaving node asks of its runtime for each shard before handing it over: refuse new
/// activations, finish the macrostep in progress of each actor, and flush the outboxes.
pub trait Drain {
    /// Drains `shard`; when this returns `Ok`, nothing of the shard is in flight on this node.
    ///
    /// # Errors
    ///
    /// A reason the shard could not be drained; the leave stops and the lease stays.
    fn drain_shard(&mut self, shard: ShardId) -> Result<(), String>;

    /// Gives `to` what it needs to take over `shard`, after the drain and before the lease
    /// moves: where shard journals are local files, ships the shard's journal to `to` and
    /// verifies that it ends at the same version. Nothing by default (a shared or replicated
    /// journal needs nothing).
    ///
    /// # Errors
    ///
    /// A reason the shard could not be shipped; the leave stops and the lease stays.
    fn ship_shard(&mut self, shard: ShardId, to: &NodeId) -> Result<(), String> {
        let _ = (shard, to);
        Ok(())
    }
}

/// A membership error outside the lease and join protocols.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MembershipError {
    /// The membership authority cannot be reached.
    Unavailable(String),
    /// The node is not a member (it never joined, left, or was marked failed).
    NotMember(NodeId),
    /// The node cannot leave the view while it holds leases.
    HoldsLeases {
        /// The node.
        node: NodeId,
        /// The shards it still holds.
        shards: Vec<ShardId>,
    },
    /// A shard could not be drained.
    Drain {
        /// The shard.
        shard: ShardId,
        /// The runtime's reason.
        reason: String,
    },
    /// No other active member can take the shard.
    NoSuccessor(ShardId),
    /// A join error.
    Join(JoinError),
    /// A lease error.
    Lease(LeaseError),
}

impl fmt::Display for MembershipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MembershipError::Unavailable(m) => write!(f, "membership unavailable: {m}"),
            MembershipError::NotMember(n) => write!(f, "{n} is not a member"),
            MembershipError::HoldsLeases { node, shards } => {
                write!(f, "{node} still holds {} shard leases", shards.len())
            }
            MembershipError::Drain { shard, reason } => {
                write!(f, "{shard} could not be drained: {reason}")
            }
            MembershipError::NoSuccessor(s) => write!(f, "no active member can take {s}"),
            MembershipError::Join(e) => e.fmt(f),
            MembershipError::Lease(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MembershipError {}

impl From<LeaseError> for MembershipError {
    fn from(e: LeaseError) -> Self {
        MembershipError::Lease(e)
    }
}

impl From<JoinError> for MembershipError {
    fn from(e: JoinError) -> Self {
        MembershipError::Join(e)
    }
}

/// When a silent member becomes suspect, then failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// @tg: implements:: [[openspec:cluster-membership#Failure Detection]]
pub struct DetectorConfig {
    /// Silence after which an active member is marked suspect.
    pub suspicion_time: Duration,
    /// Silence after which a member is marked failed.
    pub failure_time: Duration,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        DetectorConfig {
            suspicion_time: Duration::from_secs(3),
            failure_time: Duration::from_secs(10),
        }
    }
}
