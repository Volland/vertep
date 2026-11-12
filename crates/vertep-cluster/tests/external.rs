//! The external memberships against real services: etcd (feature `etcd`) and the Kubernetes
//! Lease API (feature `kubernetes`, on a k3s API server), each in a container.
//!
//! Every node runs its own authority client, as separate processes would, on the real
//! monotonic clock. The tests skip unless `VERTEP_CONTAINER_TESTS=1` (see
//! `common::containers`); `VERTEP_ETCD_ENDPOINT` and `VERTEP_KUBECONFIG` point them at an
//! existing etcd or cluster (kind, for example) instead of starting containers.
#![cfg(any(feature = "etcd", feature = "kubernetes"))]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{RecordingDrain, node};
use vertep_cluster::testkit::{self, Harness};
use vertep_cluster::{
    Authority, AuthorityConfig, AuthorityLink, AuthorityMembership, Clock, ClusterId, ClusterMeta,
    DetectorConfig, JoinError, JoinRequest, LeaseConfig, LeaseError, MemberStatus, Membership,
    MonotonicClock, RoutingTable, ShardId,
};

/// Lease 4 s with a 1 s margin; suspect after 2 s, failed after 4 s (etcd TTLs are seconds).
fn live_timing() -> AuthorityConfig {
    AuthorityConfig {
        lease: LeaseConfig {
            duration: Duration::from_secs(4),
            safety_margin: Duration::from_secs(1),
        },
        detector: DetectorConfig {
            suspicion_time: Duration::from_secs(2),
            failure_time: Duration::from_secs(4),
        },
    }
}

/// A running external service a test talks to.
trait Service: Send + Sync + 'static {
    type A: Authority + 'static;
    /// A fresh client of the service for cluster `meta`.
    fn client(&self, meta: &ClusterMeta, clock: Arc<dyn Clock>, timeout: Duration) -> Self::A;
    /// Freezes the service; false when it cannot be (an external one).
    fn pause(&self) -> bool;
    fn unpause(&self);
}

#[cfg(feature = "etcd")]
impl Service for common::containers::Etcd {
    type A = vertep_cluster::external::etcd::EtcdAuthority;
    fn client(&self, meta: &ClusterMeta, clock: Arc<dyn Clock>, timeout: Duration) -> Self::A {
        let mut config = vertep_cluster::external::etcd::EtcdConfig::new(&self.endpoint);
        config.timeout = timeout;
        Self::A::new(meta.clone(), live_timing(), &config, clock).unwrap()
    }
    fn pause(&self) -> bool {
        self.container().map(|c| c.pause()).is_some()
    }
    fn unpause(&self) {
        if let Some(c) = self.container() {
            c.unpause();
        }
    }
}

#[cfg(feature = "kubernetes")]
impl Service for common::containers::K3s {
    type A = vertep_cluster::external::kubernetes::KubeAuthority;
    fn client(&self, meta: &ClusterMeta, clock: Arc<dyn Clock>, timeout: Duration) -> Self::A {
        let mut config =
            vertep_cluster::external::kubernetes::KubeConfig::from_kubeconfig(&self.kubeconfig)
                .unwrap();
        config.timeout = timeout;
        Self::A::new(meta.clone(), live_timing(), &config, clock).unwrap()
    }
    fn pause(&self) -> bool {
        self.container().map(|c| c.pause()).is_some()
    }
    fn unpause(&self) {
        if let Some(c) = self.container() {
            c.unpause();
        }
    }
}

type Node<A> = AuthorityMembership<AuthorityLink<Arc<A>>>;

/// Nodes of one cluster on a real service, each with its own client.
struct Live<S: Service> {
    service: Arc<S>,
    meta: ClusterMeta,
    clock: Arc<MonotonicClock>,
    timeout: Duration,
    nodes: Vec<Arc<Node<S::A>>>,
    links: Vec<AuthorityLink<Arc<S::A>>>,
}

impl<S: Service> Live<S> {
    /// A cluster of `n` joined nodes with `shards` shards and a fresh cluster identifier.
    fn new(service: Arc<S>, n: usize, shards: u64) -> Self {
        let cluster = ClusterId::new(common::containers::unique("c")).unwrap();
        let meta = ClusterMeta::new(cluster, shards).unwrap();
        let mut live = Live {
            service,
            meta,
            clock: Arc::new(MonotonicClock::new()),
            timeout: Duration::from_secs(2),
            nodes: Vec::new(),
            links: Vec::new(),
        };
        for i in 0..n {
            live.add(&format!("n{i}")).join().unwrap();
        }
        live.tick_all();
        live
    }

    /// A client of the service for this cluster, outside every node.
    fn admin(&self) -> S::A {
        self.service
            .client(&self.meta, self.clock.clone(), self.timeout)
    }

    fn add(&mut self, id: &str) -> Arc<Node<S::A>> {
        let link = AuthorityLink::new(Arc::new(self.admin()));
        let n = Arc::new(AuthorityMembership::new(
            link.clone(),
            self.meta.clone(),
            node(id),
            format!("{id}.local:7400"),
            self.clock.clone(),
            live_timing().lease,
        ));
        self.nodes.push(n.clone());
        self.links.push(link);
        n
    }

    fn tick_all(&self) {
        for n in &self.nodes {
            let _ = n.tick();
        }
    }

    /// Ticks the nodes whose index `keep` accepts for `d`, every 200 ms.
    fn run_for(&self, d: Duration, keep: impl Fn(usize) -> bool) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            for (i, n) in self.nodes.iter().enumerate() {
                if keep(i) {
                    let _ = n.tick();
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Ticks the nodes `keep` accepts until `cond` holds; the time it took.
    fn until(
        &self,
        limit: Duration,
        keep: impl Fn(usize) -> bool,
        mut cond: impl FnMut(&Self) -> bool,
    ) -> Duration {
        let start = Instant::now();
        loop {
            if cond(self) {
                return start.elapsed();
            }
            assert!(start.elapsed() < limit, "not reached within {limit:?}");
            for (i, n) in self.nodes.iter().enumerate() {
                if keep(i) {
                    let _ = n.tick();
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl<S: Service> Harness for Live<S> {
    fn nodes(&self) -> Vec<Arc<dyn Membership>> {
        self.nodes
            .iter()
            .map(|n| n.clone() as Arc<dyn Membership>)
            .collect()
    }

    fn settle(&mut self) {
        self.tick_all();
        std::thread::sleep(Duration::from_millis(20));
    }

    fn advance(&mut self, d: Duration) {
        self.run_for(d, |_| true);
    }
}

// ---------------------------------------------------------------------------------------------
// The scenarios, generic over the service.

/// The shared membership test kit, unchanged.
fn kit_passes<S: Service>(service: Arc<S>) {
    let mut live = Live::new(service, 3, 64);
    testkit::run_all(&mut live);
}

/// A lease the external service grants makes its node the owner everywhere.
fn leases_follow_the_service<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 3, 64);
    let shard = ShardId::new(2);
    // granted by the service, outside every node
    let lease = live.admin().acquire(shard, &node("n1")).unwrap();
    live.tick_all();
    live.tick_all();
    for n in &live.nodes {
        let o = n.view().owner(shard).cloned().expect("an owner");
        assert_eq!((o.node, o.epoch), (node("n1"), lease.epoch));
        let mut table = RoutingTable::new(live.meta.clone());
        table.apply_view(&n.view());
        assert_eq!(table.owner(shard).unwrap().node, node("n1"));
    }
    assert!(live.nodes[1].held(shard).is_some(), "the owner holds it");
    assert!(live.nodes[0].held(shard).is_none());
    // and a grant it refuses makes no owner
    assert!(matches!(
        live.admin().acquire(shard, &node("n2")),
        Err(LeaseError::Held { .. })
    ));
}

/// With the service unreachable nothing is granted or renewed, and the holder stops at its
/// local deadline.
fn loss_stops_new_leases<S: Service>(service: Arc<S>) {
    let mut live = Live::new(service, 2, 64);
    live.timeout = Duration::from_millis(500);
    let shard = ShardId::new(3);
    let sent = live.clock.now();
    let lease = live.nodes[0].acquire(shard).unwrap();
    let deadline = live.nodes[0].held_until(shard).unwrap();
    assert!(
        deadline
            <= sent
                .after(lease.term)
                .before(live_timing().lease.safety_margin),
        "the local deadline is request time + term - margin"
    );
    if !live.service.pause() {
        eprintln!("skipped: an external service cannot be paused");
        return;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(matches!(
            live.nodes[1].acquire(ShardId::new(4)),
            Err(LeaseError::Unavailable(_))
        ));
        assert!(matches!(
            live.nodes[0].renew(&lease),
            Err(LeaseError::Unavailable(_))
        ));
        // the holder keeps working until its deadline, and stops there
        while live.clock.now() < deadline.before(Duration::from_millis(300)) {
            assert!(
                live.nodes[0].held(shard).is_some(),
                "held before the deadline"
            );
            let _ = live.nodes[0].tick();
            std::thread::sleep(Duration::from_millis(100));
        }
        while live.clock.now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(live.nodes[0].held(shard).is_none(), "stops at the deadline");
        let _ = live.nodes[0].tick();
        assert!(live.nodes[0].held(shard).is_none(), "and does not renew");
    }));
    live.service.unpause();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// A silent node becomes suspect, then failed, each in a new view version.
fn crashed_node_is_marked_failed<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 3, 64);
    let t = live_timing().detector;
    let v0 = live.nodes[0].view().version;
    let alive = |i: usize| i != 2;
    let suspect = live.until(Duration::from_secs(10), alive, |l| {
        l.nodes[0].view().status(&node("n2")) != Some(MemberStatus::Active)
    });
    let v1 = live.nodes[0].view().version;
    let failed = live.until(Duration::from_secs(15), alive, |l| {
        l.nodes[0].view().status(&node("n2")) == Some(MemberStatus::Failed)
    });
    let v2 = live.nodes[0].view().version;
    // etcd expires TTLs in whole seconds with a sweep of its own; allow 2 s on top
    let slack = Duration::from_secs(2);
    assert!(suspect >= t.suspicion_time.saturating_sub(Duration::from_millis(500)));
    assert!(
        suspect <= t.suspicion_time + slack,
        "suspect after {suspect:?}"
    );
    assert!(
        suspect + failed <= t.failure_time + slack,
        "failed after {:?}",
        suspect + failed
    );
    assert!(v0 < v1 && v1 < v2, "each change is a new view version");
    // the others stayed active
    let v = live.nodes[0].view();
    assert_eq!(v.status(&node("n0")), Some(MemberStatus::Active));
    assert_eq!(v.status(&node("n1")), Some(MemberStatus::Active));
}

/// A failed node's shard goes to another node only after its lease expired.
fn leases_of_a_failed_node_wait_for_expiry<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 3, 64);
    let shard = ShardId::new(7);
    live.nodes[2].acquire(shard).unwrap();
    // its last renewal: the lease runs until (send time + term) at the earliest
    let _ = live.nodes[2].tick();
    let earliest = live.nodes[2]
        .held_until(shard)
        .unwrap()
        .after(live_timing().lease.safety_margin);
    // node 2 crashes; node 0 keeps asking for the shard
    let mut got = None;
    let start = Instant::now();
    while got.is_none() {
        assert!(start.elapsed() < Duration::from_secs(20), "never regranted");
        for n in &live.nodes[..2] {
            let _ = n.tick();
        }
        match live.nodes[0].acquire(shard) {
            Ok(l) => got = Some((live.clock.now(), l)),
            Err(LeaseError::Held { .. } | LeaseError::NotEligible { .. }) => {}
            Err(e) => panic!("unexpected: {e}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let (at, lease) = got.unwrap();
    assert!(
        at >= earliest,
        "regranted at {at}, before the lease ran out at {earliest}"
    );
    assert!(lease.epoch > 1, "at a higher epoch");
}

/// Joins are checked against the cluster's recorded shard count, and duplicates refused.
fn incompatible_and_duplicate_joins_are_refused<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 2, 64);
    let admin = live.admin();
    let other = ClusterMeta::new(live.meta.cluster.clone(), 128).unwrap();
    // a client configured with 128 shards asks to join the 64-shard cluster
    let wrong = live
        .service
        .client(&other, live.clock.clone(), live.timeout);
    let req = JoinRequest::new(&other, node("n9"), "n9.local:7400");
    match wrong.join(&req) {
        Err(JoinError::ShardCountMismatch { cluster, node: n }) => {
            assert_eq!((cluster.get(), n.get()), (64, 128));
        }
        other => panic!("expected a shard count refusal: {other:?}"),
    }
    let req = JoinRequest::new(&live.meta, node("n0"), "elsewhere:7400");
    assert!(matches!(
        admin.join(&req),
        Err(JoinError::DuplicateNode(n)) if n == node("n0")
    ));
    let v = admin.view().unwrap();
    assert!(v.member(&node("n9")).is_none());
    assert_eq!(v.member(&node("n0")).unwrap().endpoint, "n0.local:7400");
}

/// A leaving node drains and hands over every shard before it leaves the view.
fn leaving_node_hands_over_its_shards<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 3, 16);
    let shards: Vec<ShardId> = (0..4).map(ShardId::new).collect();
    let epochs: Vec<u64> = shards
        .iter()
        .map(|s| live.nodes[2].acquire(*s).unwrap().epoch)
        .collect();
    let mut drain = RecordingDrain::default();
    live.nodes[2].leave(&mut drain).unwrap();
    assert_eq!(drain.0, shards, "every shard drained before its handover");
    live.tick_all();
    live.tick_all();
    let v = live.nodes[0].view();
    assert!(v.member(&node("n2")).is_none(), "gone from the view");
    for (s, e) in shards.iter().zip(epochs) {
        let o = v.owner(*s).expect("an owner");
        assert_ne!(o.node, node("n2"));
        assert!(o.epoch > e, "handed over at a higher epoch");
        let to = live.nodes.iter().find(|n| n.local() == &o.node).unwrap();
        assert!(to.held(*s).is_some(), "{} holds {s}", o.node);
    }
}

/// A node cut off from the service learns the newer epochs when it reconnects and does not
/// process the shards it lost.
fn old_owner_rejoins_after_a_partition<S: Service>(service: Arc<S>) {
    let live = Live::new(service, 3, 64);
    let shard = ShardId::new(9);
    let old = live.nodes[0].acquire(shard).unwrap();
    live.links[0].set_connected(false);
    let next = {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < Duration::from_secs(20), "never regranted");
            let _ = live.nodes[0].tick();
            for n in &live.nodes[1..] {
                let _ = n.tick();
            }
            if let Ok(l) = live.nodes[1].acquire(shard) {
                break l;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert!(next.epoch > old.epoch);
    assert!(live.nodes[0].held(shard).is_none(), "stopped while cut off");
    live.links[0].set_connected(true);
    live.until(
        Duration::from_secs(10),
        |_| true,
        |l| {
            l.nodes[0]
                .view()
                .owner(shard)
                .is_some_and(|o| o.epoch == next.epoch)
        },
    );
    assert!(
        live.nodes[0].held(shard).is_none(),
        "does not process the shard"
    );
    let o = live.nodes[0].view().owner(shard).cloned().unwrap();
    assert_eq!(o.node, node("n1"));
}

// ---------------------------------------------------------------------------------------------
// etcd

#[cfg(feature = "etcd")]
mod etcd {
    use super::*;
    use common::containers::{self, Etcd};

    fn etcd(what: &str) -> Option<Arc<Etcd>> {
        containers::enabled(what).then(|| Arc::new(Etcd::start()))
    }

    // @tg: verifies:: [[openspec:cluster-membership#Membership Trait#Runtime runs unchanged on any implementation]] {level: integration}
    #[test]
    fn runtime_runs_unchanged_on_etcd() {
        if let Some(s) = etcd("the membership kit on etcd") {
            kit_passes(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#External Membership#Leases follow the external service]] {level: integration}
    #[test]
    fn leases_follow_etcd() {
        if let Some(s) = etcd("leases on etcd") {
            leases_follow_the_service(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#External Membership#Loss of the external service stops new leases]] {level: integration}
    #[test]
    fn loss_of_etcd_stops_new_leases() {
        if let Some(s) = etcd("losing etcd") {
            loss_stops_new_leases(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Crashed node is marked failed]] {level: integration}
    #[test]
    fn crashed_node_is_marked_failed_by_etcd() {
        if let Some(s) = etcd("failure detection on etcd") {
            crashed_node_is_marked_failed(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Leases of a failed node are not reassigned early]] {level: integration}
    #[test]
    fn leases_of_a_failed_node_wait_for_etcd_expiry() {
        if let Some(s) = etcd("lease expiry on etcd") {
            leases_of_a_failed_node_wait_for_expiry(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Graceful Join#Node with a different shard count is refused]] {level: integration}
    #[test]
    fn etcd_refuses_incompatible_and_duplicate_joins() {
        if let Some(s) = etcd("joins on etcd") {
            incompatible_and_duplicate_joins_are_refused(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Graceful Leave With Drain#Leaving node hands over its shards]] {level: integration}
    #[test]
    fn leaving_node_hands_over_its_shards_on_etcd() {
        if let Some(s) = etcd("leaving on etcd") {
            leaving_node_hands_over_its_shards(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Lease Expiry And Handover#Old owner rejoins after a partition]] {level: integration}
    #[test]
    fn old_owner_rejoins_after_a_partition_from_etcd() {
        if let Some(s) = etcd("partitions from etcd") {
            old_owner_rejoins_after_a_partition(s);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Kubernetes

#[cfg(feature = "kubernetes")]
mod kubernetes {
    use super::*;
    use common::containers::{self, K3s};

    fn k3s(what: &str) -> Option<Arc<K3s>> {
        containers::enabled(what).then(K3s::shared)
    }

    // @tg: verifies:: [[openspec:cluster-membership#Membership Trait#Runtime runs unchanged on any implementation]] {level: integration}
    #[test]
    fn runtime_runs_unchanged_on_kubernetes_leases() {
        if let Some(s) = k3s("the membership kit on Kubernetes") {
            kit_passes(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#External Membership#Leases follow the external service]] {level: integration}
    #[test]
    fn leases_follow_kubernetes() {
        if let Some(s) = k3s("leases on Kubernetes") {
            leases_follow_the_service(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#External Membership#Loss of the external service stops new leases]] {level: integration}
    #[test]
    fn loss_of_kubernetes_stops_new_leases() {
        // a dedicated API server: pausing the shared one would stall the other tests
        if containers::enabled("losing Kubernetes") {
            loss_stops_new_leases(Arc::new(K3s::start()));
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Crashed node is marked failed]] {level: integration}
    #[test]
    fn crashed_node_is_marked_failed_in_kubernetes() {
        if let Some(s) = k3s("failure detection on Kubernetes") {
            crashed_node_is_marked_failed(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Leases of a failed node are not reassigned early]] {level: integration}
    #[test]
    fn leases_of_a_failed_node_wait_for_kubernetes_expiry() {
        if let Some(s) = k3s("lease expiry on Kubernetes") {
            leases_of_a_failed_node_wait_for_expiry(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Graceful Join#Node with a different shard count is refused]] {level: integration}
    #[test]
    fn kubernetes_refuses_incompatible_and_duplicate_joins() {
        if let Some(s) = k3s("joins on Kubernetes") {
            incompatible_and_duplicate_joins_are_refused(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Graceful Leave With Drain#Leaving node hands over its shards]] {level: integration}
    #[test]
    fn leaving_node_hands_over_its_shards_in_kubernetes() {
        if let Some(s) = k3s("leaving on Kubernetes") {
            leaving_node_hands_over_its_shards(s);
        }
    }

    // @tg: verifies:: [[openspec:cluster-membership#Lease Expiry And Handover#Old owner rejoins after a partition]] {level: integration}
    #[test]
    fn old_owner_rejoins_after_a_partition_from_kubernetes() {
        if let Some(s) = k3s("partitions from Kubernetes") {
            old_owner_rejoins_after_a_partition(s);
        }
    }
}
