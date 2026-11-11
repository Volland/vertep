//! The SWIM gossip membership over the simulated network.
#![cfg(feature = "gossip")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{RecordingDrain, meta, node};
use vertep_cluster::gossip::{GossipConfig, GossipMembership, JoinState, SimNetwork};
use vertep_cluster::testkit::{self, Harness};
use vertep_cluster::{JoinError, LeaseError, MemberStatus, Membership, ShardId, View};

struct GossipHarness {
    net: SimNetwork,
    nodes: Vec<GossipMembership>,
}

fn endpoint(i: usize) -> String {
    format!("n{i}:7946")
}

impl GossipHarness {
    fn new(n: usize) -> Self {
        let mut net = SimNetwork::new(Duration::from_millis(10), Duration::from_millis(5));
        let m = meta(64);
        let mut nodes = Vec::new();
        for i in 0..n {
            let h = net.add_node(
                &m,
                node(&format!("n{i}")),
                &endpoint(i),
                GossipConfig::default(),
            );
            h.node().join(endpoint(0));
            nodes.push(h);
        }
        // let the cluster form and the first leader's grace period pass
        net.run_for(Duration::from_secs(3));
        GossipHarness { net, nodes }
    }
}

impl Harness for GossipHarness {
    fn nodes(&self) -> Vec<Arc<dyn Membership>> {
        self.nodes
            .iter()
            .map(|n| Arc::new(n.clone()) as Arc<dyn Membership>)
            .collect()
    }

    fn settle(&mut self) {
        self.net.run_for(Duration::from_secs(1));
    }

    fn advance(&mut self, d: Duration) {
        self.net.run_for(d);
    }
}

fn statuses(v: &View) -> Vec<MemberStatus> {
    v.members.values().map(|m| m.status).collect()
}

// @tg: verifies:: [[openspec:cluster-membership#Membership Trait#Runtime runs unchanged on any implementation]] {level: integration}
#[test]
fn runtime_runs_unchanged_on_gossip_membership() {
    testkit::run_all(&mut GossipHarness::new(3));
}

// @tg: verifies:: [[openspec:cluster-membership#Gossip Membership#Cluster forms from one seed]] {level: integration}
#[test]
fn cluster_forms_from_one_seed() {
    let h = GossipHarness::new(5);
    let views: Vec<View> = h.nodes.iter().map(|n| n.view()).collect();
    for v in &views {
        assert_eq!(v.members.len(), 5);
        assert_eq!(statuses(v), vec![MemberStatus::Active; 5]);
        assert_eq!(v.membership(), views[0].membership());
    }
    for n in &h.nodes {
        assert_eq!(n.node().join_state(), &JoinState::Joined);
        assert_eq!(n.node().leader(), Some(node("n0")));
    }
}

// @tg: verifies:: [[openspec:cluster-membership#Gossip Membership#Indirect probe avoids a false failure]] {level: integration}
#[test]
fn indirect_probe_avoids_a_false_failure() {
    let mut h = GossipHarness::new(3);
    let subs: Vec<_> = h.nodes.iter().map(|n| n.subscribe()).collect();
    h.net.cut(&node("n0"), &node("n1"));
    h.net.run_for(Duration::from_secs(10));
    for sub in subs {
        for v in sub.drain() {
            assert_eq!(
                statuses(&v),
                vec![MemberStatus::Active; 3],
                "a member was suspected in view {}",
                v.version
            );
        }
    }
}

#[test]
fn crashed_gossip_member_is_suspected_then_failed() {
    let mut h = GossipHarness::new(4);
    h.net.crash(&node("n3"));
    let sub = h.nodes[0].subscribe();
    h.net.run_for(Duration::from_secs(5));
    let views = sub.drain();
    let suspect = views
        .iter()
        .find(|v| v.status(&node("n3")) == Some(MemberStatus::Suspect))
        .map(|v| v.version);
    let failed = views
        .iter()
        .find(|v| v.status(&node("n3")) == Some(MemberStatus::Failed))
        .map(|v| v.version);
    assert!(suspect.is_some() && failed.is_some() && suspect < failed);
    for n in &h.nodes[..3] {
        assert_eq!(n.view().status(&node("n3")), Some(MemberStatus::Failed));
    }
}

#[test]
fn suspected_gossip_member_refutes_and_keeps_its_lease() {
    let mut h = GossipHarness::new(4);
    let lease = testkit::acquire(&mut h, 3, ShardId::new(1)).unwrap();
    // a pause longer than a probe period but shorter than the suspicion timeout
    h.net.pause(&node("n3"));
    h.net.run_for(Duration::from_millis(600));
    h.net.resume(&node("n3"));
    h.net.run_for(Duration::from_secs(2));
    for n in &h.nodes {
        assert_eq!(n.view().status(&node("n3")), Some(MemberStatus::Active));
    }
    assert_eq!(
        h.nodes[3].held(ShardId::new(1)).map(|l| l.epoch),
        Some(lease.epoch)
    );
}

#[test]
fn gossip_join_with_another_shard_count_is_refused() {
    let mut h = GossipHarness::new(2);
    let other = h
        .net
        .add_node(&meta(128), node("n9"), "n9:7946", GossipConfig::default());
    other.node().join(endpoint(0));
    h.net.run_for(Duration::from_secs(1));
    assert!(matches!(
        other.node().join_state(),
        JoinState::Refused(JoinError::ShardCountMismatch { .. })
    ));
    assert!(h.nodes[0].view().member(&node("n9")).is_none());
}

#[test]
fn gossip_leave_hands_over_before_leaving() {
    let mut h = GossipHarness::new(3);
    let shards: Vec<ShardId> = (20..24).map(ShardId::new).collect();
    let old: Vec<_> = shards
        .iter()
        .map(|s| testkit::acquire(&mut h, 2, *s).unwrap())
        .collect();
    let mut drain = RecordingDrain::default();
    h.nodes[2].node().leave(&mut drain).unwrap();
    h.net.run_for(Duration::from_secs(3));
    assert_eq!(drain.0, shards);
    assert!(h.nodes[2].node().has_left());
    for n in &h.nodes[..2] {
        let v = n.view();
        assert!(v.member(&node("n2")).is_none());
        for (s, l) in shards.iter().zip(&old) {
            let o = v.owner(*s).unwrap();
            assert_ne!(o.node, node("n2"));
            assert!(o.epoch > l.epoch);
        }
    }
}

#[test]
fn a_new_leader_waits_out_its_predecessors_leases() {
    let mut h = GossipHarness::new(3);
    let lease = testkit::acquire(&mut h, 1, ShardId::new(2)).unwrap();
    // the leader crashes; n1 becomes leader once n0 is failed
    h.net.crash(&node("n0"));
    h.net.run_for(Duration::from_secs(2));
    assert_eq!(h.nodes[1].node().leader(), Some(node("n1")));
    // n2 cannot take the shard while n1's lease (renewed with the new leader) lives
    let r = testkit::acquire(&mut h, 2, ShardId::new(2));
    assert!(
        matches!(r, Err(LeaseError::Held { .. } | LeaseError::Unavailable(_))),
        "{r:?}"
    );
    // n1 keeps renewing through the new leader, at the same epoch
    h.net.run_for(Duration::from_secs(3));
    assert_eq!(
        h.nodes[1].held(ShardId::new(2)).map(|l| l.epoch),
        Some(lease.epoch)
    );
}

#[test]
fn without_indirect_probes_a_cut_link_causes_suspicion() {
    let mut net = SimNetwork::new(Duration::from_millis(10), Duration::from_millis(5));
    let config = GossipConfig {
        indirect_probes: 0,
        ..GossipConfig::default()
    };
    let nodes: Vec<GossipMembership> = (0..3)
        .map(|i| {
            let h = net.add_node(&meta(64), node(&format!("n{i}")), &endpoint(i), config);
            h.node().join(endpoint(0));
            h
        })
        .collect();
    net.run_for(Duration::from_secs(3));
    let subs: Vec<_> = nodes.iter().map(|n| n.subscribe()).collect();
    net.cut(&node("n0"), &node("n1"));
    net.run_for(Duration::from_secs(10));
    let suspected = subs.iter().any(|s| {
        s.drain()
            .iter()
            .any(|v| statuses(v).iter().any(|st| *st != MemberStatus::Active))
    });
    assert!(suspected, "the control run must see a false suspicion");
}
