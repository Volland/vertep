//! External lease authorities: etcd (feature `etcd`) and the Kubernetes Lease API (feature
//! `kubernetes`) as the authority for members and shard leases.
//!
//! Each is an [`Authority`](crate::Authority), so a node's membership is
//! [`ExternalMembership`](crate::ExternalMembership) over it, exactly like the static
//! membership over its hosted authority; the runtime cannot tell them apart. The external
//! service alone admits members, grants, renews and expires leases; when it is unreachable
//! every call fails with `Unavailable`, nothing is granted or renewed, and holders stop at
//! their local deadline (request time + term − safety margin on their own clock).
//!
//! - [`etcd::EtcdAuthority`]: the etcd v3 JSON gateway. Shard leases are keys attached to etcd
//!   leases whose TTL is the lease term, so etcd itself expires them; grants and handovers are
//!   transactions comparing `mod_revision`s, and a permanent per-shard epoch key makes every
//!   grant's epoch strictly greater than the last. Members are keys attached to two etcd
//!   leases (suspicion time and failure time): the node's heartbeat keeps them alive.
//! - [`kubernetes::KubeAuthority`]: `coordination.k8s.io/v1` `Lease` objects, one per shard and
//!   one per member, updated only with their `resourceVersion` (optimistic concurrency).
//!   `leaseTransitions` is the epoch. Kubernetes does not expire leases itself, so a node takes
//!   a shard held by another only after it has seen the holder's record unchanged for a whole
//!   lease term on its own clock (as client-go leader election does), and marks a member
//!   suspect or failed after seeing its heartbeat unchanged for the suspicion or failure time.
//!
//! [`MemoryAuthority`](crate::MemoryAuthority) remains the in-memory stand-in with the same
//! rules, for deterministic tests.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::meta::{ClusterId, NodeId};
use crate::placement::ShardId;
use crate::view::{Member, MemberStatus, ShardOwner, View};

mod http;

#[cfg(feature = "etcd")]
pub mod etcd;

#[cfg(feature = "kubernetes")]
pub mod kubernetes;

/// Whole seconds covering `d`, at least one: etcd lease TTLs and Kubernetes
/// `leaseDurationSeconds` are seconds.
pub(crate) fn ceil_secs(d: Duration) -> u64 {
    let s = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    s.max(1)
}

type Snapshot = (
    BTreeMap<NodeId, (String, MemberStatus)>,
    BTreeMap<ShardId, ShardOwner>,
);

/// The views one client of an external authority publishes: a new version whenever members,
/// statuses or owners change, taken from the service's revision so that nodes reading the
/// same state agree, and never lower than the last one.
#[derive(Debug, Default)]
pub(crate) struct Publisher {
    last: Option<Snapshot>,
    version: u64,
}

impl Publisher {
    /// The view of `snapshot`, read from the service at `revision`.
    pub fn view(&mut self, cluster: &ClusterId, snapshot: Snapshot, revision: u64) -> View {
        if self.last.as_ref() != Some(&snapshot) {
            self.version = revision.max(self.version + 1);
            self.last = Some(snapshot.clone());
        }
        let (members, owners) = snapshot;
        View {
            cluster: cluster.clone(),
            version: self.version,
            members: members
                .into_iter()
                .map(|(id, (endpoint, status))| {
                    (
                        id.clone(),
                        Member {
                            id,
                            endpoint,
                            status,
                        },
                    )
                })
                .collect(),
            owners,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_seconds_round_up() {
        assert_eq!(ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(ceil_secs(Duration::from_secs(2)), 2);
        assert_eq!(ceil_secs(Duration::from_millis(2_500)), 3);
        assert_eq!(ceil_secs(Duration::ZERO), 1);
    }

    #[test]
    fn versions_rise_only_on_change() {
        let c = ClusterId::new("c").unwrap();
        let mut p = Publisher::default();
        let empty: Snapshot = (BTreeMap::new(), BTreeMap::new());
        assert_eq!(p.view(&c, empty.clone(), 5).version, 5);
        assert_eq!(
            p.view(&c, empty.clone(), 9).version,
            5,
            "same state, same version"
        );
        let mut one = empty.clone();
        one.0.insert(
            NodeId::new("n").unwrap(),
            ("e".into(), MemberStatus::Active),
        );
        assert_eq!(p.view(&c, one.clone(), 3).version, 6, "never lower");
        assert_eq!(p.view(&c, empty, 20).version, 20);
    }
}
