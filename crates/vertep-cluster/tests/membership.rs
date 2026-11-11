//! Membership through a lease authority: the static membership and the external membership's
//! in-memory stand-in, view versions, join, leave, failure detection and leases.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::{AuthorityHarness, RecordingDrain, authority_view, grant, meta, node, timing};
use vertep_cluster::testkit::{self, Harness};
use vertep_cluster::{
    ActivationGate, Authority, AuthorityLink, AuthorityMembership, ClusterId, DedupCache, Inbound,
    JoinError, JoinRequest, LeaseError, ManualClock, MemberStatus, Membership, MembershipError,
    MemoryAuthority, ShardId, ShardOwner, StaticConfig, View, ViewTracker, assignment,
};

// @tg: verifies:: [[openspec:cluster-membership#Membership Trait#Runtime runs unchanged on any implementation]] {level: integration}
#[test]
fn runtime_runs_unchanged_on_static_and_external_memberships() {
    testkit::run_all(&mut AuthorityHarness::static_cluster(3, 64));
    testkit::run_all(&mut AuthorityHarness::external_cluster(3, 64));
}

// @tg: verifies:: [[openspec:cluster-membership#Membership Trait#Subscribers receive view changes in order]] {level: integration}
#[test]
fn subscribers_receive_view_changes_in_order() {
    let mut h = AuthorityHarness::static_cluster(3, 64);
    testkit::subscribers_receive_changes_in_order(&mut h);
    // two joins one after the other also reach every subscriber, in order, none skipped
    let mut h = AuthorityHarness::external_cluster(1, 64);
    let subs: Vec<_> = (0..3).map(|_| h.nodes[0].subscribe()).collect();
    h.add_node("n1").join().unwrap();
    h.settle();
    h.add_node("n2").join().unwrap();
    h.settle();
    for sub in subs {
        let views = sub.drain();
        let sizes: Vec<usize> = views.iter().map(|v| v.members.len()).collect();
        assert_eq!(sizes, vec![2, 3]);
        assert!(views[0].version < views[1].version);
    }
}

fn view(version: u64) -> View {
    let mut v = View::empty(ClusterId::new("test").unwrap());
    v.version = version;
    v
}

// @tg: verifies:: [[openspec:cluster-membership#Membership View Versions#Newer view replaces older view]] {level: unit}
#[test]
fn newer_view_replaces_older_view() {
    let mut t = ViewTracker::new(view(7));
    let sub = t.subscribe();
    assert!(t.offer(view(8)));
    assert_eq!(t.current().version, 8);
    assert_eq!(
        sub.drain().iter().map(|v| v.version).collect::<Vec<_>>(),
        [8]
    );
}

// @tg: verifies:: [[openspec:cluster-membership#Membership View Versions#Stale view is ignored]] {level: unit}
#[test]
fn stale_view_is_ignored() {
    let mut t = ViewTracker::new(view(8));
    let sub = t.subscribe();
    assert!(!t.offer(view(6)));
    assert!(!t.offer(view(8)));
    assert_eq!(t.current().version, 8);
    assert!(sub.try_next().is_none());
}

#[test]
fn every_member_change_raises_the_view_version() {
    let mut h = AuthorityHarness::external_cluster(2, 64);
    let mut last = authority_view(&h.authority).version;
    let n2 = h.add_node("n2");
    for step in 0..3 {
        match step {
            0 => {
                n2.join().unwrap();
            }
            1 => {
                n2.acquire(ShardId::new(1)).unwrap();
            }
            _ => h.advance(Duration::from_secs(4)),
        }
        let v = authority_view(&h.authority).version;
        assert!(v > last, "step {step} did not raise the version");
        last = v;
    }
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Join#Compatible node joins]] {level: integration}
#[test]
fn compatible_node_joins() {
    let mut h = AuthorityHarness::external_cluster(2, 64);
    let before = authority_view(&h.authority).version;
    let n2 = h.add_node("n2");
    let v = n2.join().unwrap();
    assert!(v.version > before);
    assert_eq!(v.status(&node("n2")), Some(MemberStatus::Active));
    assert!(n2.acquire(ShardId::new(9)).is_ok(), "eligible for leases");
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Join#Node with a different shard count is refused]] {level: integration}
#[test]
fn node_with_a_different_shard_count_is_refused() {
    let clock = ManualClock::new();
    let authority = MemoryAuthority::new(meta(256), timing(), Arc::new(clock));
    let sub_view_before = authority.view().unwrap();
    let req = JoinRequest::new(&meta(128), node("n9"), "n9:7400");
    let err = authority.join(&req).unwrap_err();
    assert!(matches!(err, JoinError::ShardCountMismatch { .. }));
    let msg = err.to_string();
    assert!(msg.contains("128") && msg.contains("256"), "{msg}");
    let after = authority.view().unwrap();
    assert!(after.member(&node("n9")).is_none());
    assert_eq!(after.version, sub_view_before.version);
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Join#Duplicate node identifier is refused]] {level: integration}
#[test]
fn duplicate_node_identifier_is_refused() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let lease = grant(&h.nodes[0], ShardId::new(4));
    let before = authority_view(&h.authority);
    let req = JoinRequest::new(&meta(64), node("n0"), "elsewhere:7400");
    assert_eq!(
        h.authority.join(&req).unwrap_err(),
        JoinError::DuplicateNode(node("n0"))
    );
    let after = authority_view(&h.authority);
    assert_eq!(after, before, "the active member is unaffected");
    assert_eq!(h.nodes[0].held(ShardId::new(4)), Some(lease));
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Leave With Drain#Leaving node hands over its shards]] {level: integration}
#[test]
fn leaving_node_hands_over_its_shards() {
    let h = AuthorityHarness::external_cluster(3, 64);
    let leaving = h.nodes[0].clone();
    let shards: Vec<ShardId> = (0..12).map(ShardId::new).collect();
    let old: Vec<_> = shards.iter().map(|s| grant(&leaving, *s)).collect();
    h.tick_all();
    let sub = leaving.subscribe();
    let mut drain = RecordingDrain::default();
    leaving.leave(&mut drain).unwrap();
    h.tick_all();

    assert_eq!(drain.0, shards, "every shard drained before its handover");
    let views = sub.drain();
    let gone = views
        .iter()
        .position(|v| v.member(&node("n0")).is_none())
        .expect("the node left the view");
    // before it disappears every shard is already owned by another member
    let last_with = &views[gone - 1];
    for (s, l) in shards.iter().zip(&old) {
        let o = last_with.owner(*s).unwrap();
        assert_ne!(o.node, node("n0"));
        assert!(o.epoch > l.epoch);
    }
    for v in &views[gone..] {
        assert_eq!(v.shards_of(&node("n0")).count(), 0);
    }
    // the new owners hold them
    for s in &shards {
        let owner = &views.last().unwrap().owner(*s).unwrap().node;
        let n = h.nodes.iter().find(|n| n.local() == owner).unwrap();
        assert!(n.held(*s).is_some(), "{owner} holds {s}");
    }
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Leave With Drain#Messages arriving during drain are forwarded]] {level: integration}
#[test]
fn messages_arriving_during_drain_are_forwarded() {
    let h = AuthorityHarness::external_cluster(3, 64);
    let m = meta(64);
    let actor = common::actor_in(&m, ShardId::new(5), "drain");
    grant(&h.nodes[0], ShardId::new(5));
    h.nodes[0].leave(&mut RecordingDrain::default()).unwrap();
    h.tick_all();

    // the message reaches the old owner after the handover started
    let old_gate = ActivationGate::new(h.nodes[0].clone(), 3);
    let Inbound::Forward { to, hops, redirect } = old_gate.inbound(actor.as_str(), 0) else {
        panic!("the old owner must forward");
    };
    assert_eq!(hops, 1);
    assert_eq!(redirect.owner, to);
    let new_owner = h.nodes.iter().find(|n| n.local() == &to).unwrap();
    let gate = ActivationGate::new(new_owner.clone(), 3);
    let mut processed = Vec::new();
    let mut dedup = DedupCache::new(16);
    // the forwarded copy and a retry of the original both arrive: processed once
    for _ in 0..2 {
        match gate.inbound(actor.as_str(), hops) {
            Inbound::Deliver(_) => {
                if dedup.first_time(actor.as_str(), "m1") {
                    processed.push("m1");
                }
            }
            other => panic!("the new owner must deliver: {other:?}"),
        }
    }
    assert_eq!(processed, ["m1"]);
}

#[test]
fn a_shard_that_cannot_be_drained_keeps_its_lease() {
    struct Stuck;
    impl vertep_cluster::Drain for Stuck {
        fn drain_shard(&mut self, _: ShardId) -> Result<(), String> {
            Err("macrostep in progress".into())
        }
    }
    let h = AuthorityHarness::external_cluster(2, 64);
    grant(&h.nodes[0], ShardId::new(1));
    assert!(matches!(
        h.nodes[0].leave(&mut Stuck),
        Err(MembershipError::Drain { .. })
    ));
    assert_eq!(
        authority_view(&h.authority)
            .owner(ShardId::new(1))
            .unwrap()
            .node,
        node("n0")
    );
}

/// Steps the clock in `step_ms` increments, ticking every node but `silent`, until `until`
/// returns true; returns the elapsed milliseconds.
fn run_until(
    h: &AuthorityHarness,
    silent: usize,
    step_ms: u64,
    until: impl Fn(&View) -> bool,
) -> u64 {
    let mut elapsed = 0;
    loop {
        h.clock.advance_ms(step_ms);
        elapsed += step_ms;
        for (i, n) in h.nodes.iter().enumerate() {
            if i != silent {
                let _ = n.tick();
            }
        }
        if until(&h.nodes[0].view()) {
            return elapsed;
        }
        assert!(elapsed < 60_000, "condition never reached");
    }
}

// @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Crashed node is marked failed]] {level: integration}
#[test]
fn crashed_node_is_marked_failed() {
    let h = AuthorityHarness::static_cluster(3, 64);
    let sub = h.nodes[0].subscribe();
    let n2 = node("n2");
    let suspect = run_until(&h, 2, 250, |v| v.status(&n2) == Some(MemberStatus::Suspect));
    assert!(suspect <= 3_000 + 250, "suspect after {suspect} ms");
    let failed = suspect + run_until(&h, 2, 250, |v| v.status(&n2) == Some(MemberStatus::Failed));
    assert!(failed <= 10_000 + 250, "failed after {failed} ms");
    let views = sub.drain();
    let vs = views
        .iter()
        .find(|v| v.status(&n2) == Some(MemberStatus::Suspect))
        .unwrap()
        .version;
    let vf = views
        .iter()
        .find(|v| v.status(&n2) == Some(MemberStatus::Failed))
        .unwrap()
        .version;
    assert!(vs < vf, "each status in its own, newer view version");
}

// @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Suspect node that answers is restored]] {level: integration}
#[test]
fn suspect_node_that_answers_is_restored() {
    let h = AuthorityHarness::static_cluster(3, 64);
    let lease = grant(&h.nodes[2], ShardId::new(8));
    let n2 = node("n2");
    run_until(&h, 2, 250, |v| v.status(&n2) == Some(MemberStatus::Suspect));
    let suspect_version = h.nodes[0].view().version;
    h.nodes[2].tick().unwrap();
    h.nodes[0].tick().unwrap();
    let v = h.nodes[0].view();
    assert_eq!(v.status(&n2), Some(MemberStatus::Active));
    assert!(v.version > suspect_version);
    let kept = h.nodes[2].held(ShardId::new(8)).expect("keeps its lease");
    assert_eq!(kept.epoch, lease.epoch);
}

// @tg: verifies:: [[openspec:cluster-membership#Failure Detection#Leases of a failed node are not reassigned early]] {level: integration}
#[test]
fn leases_of_a_failed_node_are_not_reassigned_early() {
    let clock = ManualClock::new();
    let mut t = timing();
    t.lease.duration = Duration::from_secs(20);
    let config = StaticConfig {
        meta: meta(64),
        members: vec![(node("n0"), "a".into()), (node("n1"), "b".into())],
        timing: t,
    };
    let authority = Arc::new(config.authority(Arc::new(clock.clone())));
    let n0 = config
        .membership(&node("n0"), authority.clone(), Arc::new(clock.clone()))
        .unwrap();
    let n1 = config
        .membership(&node("n1"), authority.clone(), Arc::new(clock.clone()))
        .unwrap();
    let lease = n1.acquire(ShardId::new(7)).unwrap();
    // n1 crashes: only n0 keeps ticking
    for _ in 0..11 {
        clock.advance(Duration::from_secs(1));
        n0.tick().unwrap();
    }
    assert_eq!(n0.view().status(&node("n1")), Some(MemberStatus::Failed));
    assert!(matches!(
        n0.acquire(ShardId::new(7)),
        Err(LeaseError::Held { .. })
    ));
    clock.set(20_000);
    n0.tick().unwrap();
    let next = n0.acquire(ShardId::new(7)).unwrap();
    assert!(next.epoch > lease.epoch);
}

// @tg: verifies:: [[openspec:cluster-membership#Static Membership#Members from configuration]] {level: integration}
#[test]
fn members_from_configuration() {
    let h = AuthorityHarness::static_cluster(3, 64);
    for n in &h.nodes {
        n.claim_assigned();
    }
    h.tick_all();
    let views: Vec<View> = h.nodes.iter().map(|n| n.view()).collect();
    assert!(
        views.windows(2).all(|w| w[0] == w[1]),
        "same view everywhere"
    );
    let expected = assignment(&views[0], meta(64).shard_count);
    let table: Vec<(ShardId, ShardOwner)> = views[0].owners.clone().into_iter().collect();
    assert_eq!(table.len(), 64);
    for (s, o) in table {
        assert_eq!(expected[&s], o.node);
    }
    assert_eq!(views[0].active().count(), 3);
}

// @tg: verifies:: [[openspec:cluster-membership#Static Membership#Unlisted node cannot join]] {level: integration}
#[test]
fn unlisted_node_cannot_join() {
    let h = AuthorityHarness::static_cluster(3, 64);
    let before = authority_view(&h.authority);
    let req = JoinRequest::new(&meta(64), node("intruder"), "x:7400");
    assert_eq!(
        h.authority.join(&req).unwrap_err(),
        JoinError::NotListed(node("intruder"))
    );
    let config = StaticConfig {
        meta: meta(64),
        members: vec![(node("n0"), "a".into())],
        timing: timing(),
    };
    assert!(
        config
            .membership(
                &node("intruder"),
                h.authority.clone(),
                Arc::new(h.clock.clone())
            )
            .is_err()
    );
    assert_eq!(authority_view(&h.authority), before);
}

// The external scenarios run against the in-memory stand-in of the external authority here;
// runs against containerised etcd and kind are not written yet (task 2.8 stays open).
// @tg: verifies:: [[openspec:cluster-membership#External Membership#Leases follow the external service]] {level: integration}
#[test]
fn leases_follow_the_external_service_stand_in() {
    let h = AuthorityHarness::external_cluster(3, 64);
    let lease = h.authority.acquire(ShardId::new(2), &node("n1")).unwrap();
    h.tick_all();
    for n in &h.nodes {
        let o = n.view().owner(ShardId::new(2)).cloned().unwrap();
        assert_eq!((o.node, o.epoch), (node("n1"), lease.epoch));
    }
    assert!(h.nodes[1].held(ShardId::new(2)).is_some());
}

// @tg: verifies:: [[openspec:cluster-membership#External Membership#Loss of the external service stops new leases]] {level: integration}
#[test]
fn loss_of_the_external_service_stops_new_leases_stand_in() {
    let mut h = AuthorityHarness::external_cluster(2, 64);
    let lease = grant(&h.nodes[0], ShardId::new(3));
    h.authority.set_reachable(false);
    assert!(matches!(
        h.nodes[1].acquire(ShardId::new(4)),
        Err(LeaseError::Unavailable(_))
    ));
    assert!(matches!(
        h.nodes[0].renew(&lease),
        Err(LeaseError::Unavailable(_))
    ));
    h.advance(Duration::from_millis(7_900));
    assert!(h.nodes[0].held(ShardId::new(3)).is_some());
    h.advance(Duration::from_millis(100));
    assert!(
        h.nodes[0].held(ShardId::new(3)).is_none(),
        "stops at expiry minus margin"
    );
}

// @tg: verifies:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs#New grant increases the epoch]] {level: integration}
#[test]
fn new_grant_increases_the_epoch() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let s = ShardId::new(42);
    let mut lease = grant(&h.nodes[0], s);
    while lease.epoch < 5 {
        h.nodes[0].release(&lease).unwrap();
        lease = grant(&h.nodes[0], s);
    }
    assert_eq!(lease.epoch, 5);
    h.nodes[0].release(&lease).unwrap();
    let other = grant(&h.nodes[1], s);
    assert!(other.epoch > 5);
}

// @tg: verifies:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs#Renewal keeps the epoch]] {level: integration}
#[test]
fn renewal_keeps_the_epoch() {
    let h = AuthorityHarness::external_cluster(1, 64);
    let lease = grant(&h.nodes[0], ShardId::new(42));
    h.clock.advance(Duration::from_secs(4));
    let renewed = h.nodes[0].renew(&lease).unwrap();
    assert_eq!(renewed.epoch, lease.epoch);
    assert!(renewed.expires_at > lease.expires_at);
    assert!(
        h.nodes[0].held_until(ShardId::new(42))
            > Some(lease.expires_at.before(timing().lease.safety_margin))
    );
}

// @tg: verifies:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs#At most one valid lease per shard]] {level: integration}
#[test]
fn at_most_one_valid_lease_per_shard() {
    let h = AuthorityHarness::external_cluster(4, 64);
    // every node races for every shard at once
    let wins: Vec<Vec<ShardId>> = std::thread::scope(|scope| {
        let handles: Vec<_> = h
            .nodes
            .iter()
            .map(|n| {
                let n = n.clone();
                scope.spawn(move || {
                    (0..64)
                        .map(ShardId::new)
                        .filter(|s| n.acquire(*s).is_ok())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().map(|j| j.join().unwrap()).collect()
    });
    let mut all: Vec<ShardId> = wins.concat();
    all.sort();
    let unique: BTreeSet<ShardId> = all.iter().copied().collect();
    assert_eq!(all.len(), 64);
    assert_eq!(unique.len(), 64, "each shard granted exactly once");
    for s in 0..64 {
        let holders = h
            .nodes
            .iter()
            .filter(|n| n.held(ShardId::new(s)).is_some())
            .count();
        assert_eq!(holders, 1);
    }
    assert_eq!(h.authority.live_leases().len(), 64);
}

// @tg: verifies:: [[openspec:cluster-membership#Lease Expiry And Handover#Owner stops at expiry]] {level: integration}
#[test]
fn owner_stops_at_expiry() {
    let clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        meta(64),
        timing(),
        Arc::new(clock.clone()),
    ));
    let link = AuthorityLink::new(authority.clone());
    let n0 = AuthorityMembership::new(
        link.clone(),
        meta(64),
        node("n0"),
        "a",
        Arc::new(clock.clone()),
        timing().lease,
    );
    n0.join().unwrap();
    let s = ShardId::new(1);
    let lease = n0.acquire(s).unwrap();
    link.set_connected(false);
    clock.advance(Duration::from_secs(6));
    assert!(n0.tick().is_err());
    assert!(matches!(n0.renew(&lease), Err(LeaseError::Unavailable(_))));
    // lease 10 s, safety margin 2 s: macrosteps stop at 8 s
    clock.set(7_999);
    assert!(n0.held(s).is_some());
    clock.set(8_000);
    assert!(n0.held(s).is_none());
    let gate = ActivationGate::new(Arc::new(n0), 3);
    assert!(
        gate.may_activate(common::actor_in(&meta(64), s, "x").as_str())
            .is_err()
    );
}

// @tg: verifies:: [[openspec:cluster-membership#Lease Expiry And Handover#Planned handover]] {level: integration}
#[test]
fn planned_handover() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let s = ShardId::new(6);
    let actor = common::actor_in(&meta(64), s, "handover");
    let a = grant(&h.nodes[0], s);
    let mut drain = RecordingDrain::default();
    let b =
        vertep_cluster::move_by_lease_transfer(h.nodes[0].as_ref(), &a, &node("n1"), &mut drain)
            .unwrap();
    assert_eq!(
        drain.0,
        [s],
        "A drained (finished macrosteps, flushed outbox) first"
    );
    assert!(b.epoch > a.epoch);
    assert!(h.nodes[0].held(s).is_none());
    h.tick_all();
    let gate = ActivationGate::new(h.nodes[1].clone(), 3);
    match gate.inbound(actor.as_str(), 0) {
        Inbound::Deliver(l) => assert_eq!(l.epoch, b.epoch),
        other => panic!("B processes the next message: {other:?}"),
    }
}

// @tg: verifies:: [[openspec:cluster-membership#Lease Expiry And Handover#Old owner rejoins after a partition]] {level: integration}
#[test]
fn old_owner_rejoins_after_a_partition() {
    let clock = ManualClock::new();
    let a_clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        meta(64),
        timing(),
        Arc::new(clock.clone()),
    ));
    let a_link = AuthorityLink::new(authority.clone());
    let a = Arc::new(AuthorityMembership::new(
        a_link.clone(),
        meta(64),
        node("a"),
        "a",
        Arc::new(a_clock.clone()),
        timing().lease,
    ));
    let b = Arc::new(AuthorityMembership::new(
        authority.clone(),
        meta(64),
        node("b"),
        "b",
        Arc::new(clock.clone()),
        timing().lease,
    ));
    a.join().unwrap();
    b.join().unwrap();
    let s = ShardId::new(9);
    let old = a.acquire(s).unwrap();
    // partition: A is cut off and paused (its clock stands still)
    a_link.set_connected(false);
    for _ in 0..25 {
        clock.advance(Duration::from_secs(1));
        b.tick().unwrap();
    }
    let new = b.acquire(s).unwrap();
    assert!(new.epoch > old.epoch);
    assert!(
        a.held(s).is_some(),
        "A's paused clock still believes in its lease"
    );
    // A reconnects: its next tick rejoins and learns the newer epoch from the view
    a_link.set_connected(true);
    a.tick().unwrap();
    assert!(a.held(s).is_none());
    let gate = ActivationGate::new(a.clone(), 3);
    match gate.inbound(common::actor_in(&meta(64), s, "p").as_str(), 0) {
        Inbound::Forward { to, redirect, .. } => {
            assert_eq!(to, node("b"));
            assert_eq!(redirect.epoch, new.epoch);
        }
        other => panic!("A must not process the shard: {other:?}"),
    }
}
