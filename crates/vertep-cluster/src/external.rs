//! External lease authorities: how cluster members and shard leases map onto an etcd key space
//! and onto Kubernetes `Lease` objects.
//!
//! Only the mapping is here. There is no live etcd or Kubernetes client yet; tests run the
//! external membership against [`crate::MemoryAuthority`], which follows the same rules (the
//! external service alone grants, renews and expires leases; when it is unreachable nothing is
//! granted or renewed and holders stop at their local deadline).

use core::fmt;

use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;

/// A shard lease as stored in the external service.
#[derive(Clone, Debug, PartialEq, Eq)]
// @tg: implements:: [[openspec:cluster-membership#External Membership]] {confidence: partial}
pub struct LeaseRecord {
    /// The holder.
    pub owner: NodeId,
    /// The fencing epoch: the etcd revision of the grant, or the Kubernetes `leaseTransitions`
    /// count, both strictly increasing per shard.
    pub epoch: u64,
    /// The lease term in milliseconds (etcd lease TTL, Kubernetes `leaseDurationSeconds`).
    pub term_ms: u64,
}

/// Why a stored lease record cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordError(pub String);

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed lease record: {}", self.0)
    }
}

impl std::error::Error for RecordError {}

impl LeaseRecord {
    /// The record as the value stored under the shard's key: `owner;epoch;term_ms`.
    pub fn encode(&self) -> String {
        format!("{};{};{}", self.owner, self.epoch, self.term_ms)
    }

    /// The record in `value`.
    ///
    /// # Errors
    ///
    /// [`RecordError`] for anything [`LeaseRecord::encode`] does not produce.
    pub fn decode(value: &str) -> Result<Self, RecordError> {
        let mut it = value.split(';');
        let (Some(o), Some(e), Some(t), None) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err(RecordError(value.to_owned()));
        };
        Ok(LeaseRecord {
            owner: NodeId::new(o).map_err(|e| RecordError(e.to_string()))?,
            epoch: e.parse().map_err(|_| RecordError(format!("epoch {e}")))?,
            term_ms: t.parse().map_err(|_| RecordError(format!("term {t}")))?,
        })
    }
}

/// The etcd key space of a cluster.
#[cfg(feature = "etcd")]
pub mod etcd {
    use super::*;

    /// The prefix of every key of the cluster: `/vertep/<cluster>/`.
    pub fn prefix(meta: &ClusterMeta) -> String {
        format!("/vertep/{}/", meta.cluster)
    }

    /// The key of a member, attached to the member's etcd lease (its heartbeat).
    pub fn member_key(meta: &ClusterMeta, node: &NodeId) -> String {
        format!("{}members/{node}", prefix(meta))
    }

    /// The key of a shard's lease record, attached to the holder's etcd lease, written only by
    /// a transaction comparing the key's `mod_revision`.
    pub fn shard_key(meta: &ClusterMeta, shard: ShardId) -> String {
        format!(
            "{}shards/{}-of-{}",
            prefix(meta),
            shard.get(),
            meta.shard_count
        )
    }
}

/// The Kubernetes `Lease` objects of a cluster.
#[cfg(feature = "kubernetes")]
pub mod kubernetes {
    use super::*;

    /// The name of a shard's `Lease` object: `vertep-<cluster>-shard-<s>-of-<n>`. Its
    /// `holderIdentity` is the node, `leaseTransitions` the epoch, and updates are guarded by
    /// the object's `resourceVersion`.
    pub fn lease_name(meta: &ClusterMeta, shard: ShardId) -> String {
        format!(
            "vertep-{}-shard-{}-of-{}",
            meta.cluster,
            shard.get(),
            meta.shard_count
        )
    }

    /// The name of a member's heartbeat `Lease` object: `vertep-<cluster>-member-<node>`.
    pub fn member_lease_name(meta: &ClusterMeta, node: &NodeId) -> String {
        format!("vertep-{}-member-{node}", meta.cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_record_round_trip() {
        let r = LeaseRecord {
            owner: NodeId::new("n1").unwrap(),
            epoch: 42,
            term_ms: 10_000,
        };
        assert_eq!(LeaseRecord::decode(&r.encode()).unwrap(), r);
        assert!(LeaseRecord::decode("n1;x;1").is_err());
    }
}
