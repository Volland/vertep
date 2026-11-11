//! The shared membership test kit: placement, activation, leasing and routing checks that
//! every [`Membership`] implementation must pass unchanged.
//!
//! An implementation's tests provide a [`Harness`] (its nodes and a way to let time pass) and
//! call [`run_all`]. Every check panics with a description on failure.

use std::sync::Arc;
use std::time::Duration;

use crate::lease::{Lease, LeaseError};
use crate::membership::Membership;
use crate::placement::ShardId;
use crate::routing::{ActivationGate, Inbound, RoutingTable};

/// A running cluster of at least three nodes for the kit.
pub trait Harness {
    /// The nodes, in a fixed order.
    fn nodes(&self) -> Vec<Arc<dyn Membership>>;

    /// Lets the cluster exchange messages and poll until it is quiet (a short time passes).
    fn settle(&mut self);

    /// Lets `d` of time pass for every node and the lease authority.
    fn advance(&mut self, d: Duration);
}

/// Acquires `shard` on node `i`, letting the cluster settle while the request is pending.
///
/// # Errors
///
/// The final lease error.
pub fn acquire(h: &mut dyn Harness, i: usize, shard: ShardId) -> Result<Lease, LeaseError> {
    let node = h.nodes()[i].clone();
    for _ in 0..50 {
        match node.acquire(shard) {
            Err(LeaseError::Pending { .. }) => h.settle(),
            other => {
                h.settle();
                return other;
            }
        }
    }
    Err(LeaseError::Pending { shard })
}

/// Releases `lease` on node `i` and lets the cluster settle.
pub fn release(h: &mut dyn Harness, i: usize, lease: &Lease) {
    let node = h.nodes()[i].clone();
    node.release(lease).expect("release");
    h.settle();
}

/// Two lease grants one after the other reach a subscriber as two views, in version order.
pub fn subscribers_receive_changes_in_order(h: &mut dyn Harness) {
    let observer = h.nodes()[2].clone();
    let sub = observer.subscribe();
    let a = acquire(h, 0, ShardId::new(0)).expect("first grant");
    let b = acquire(h, 1, ShardId::new(1)).expect("second grant");
    h.settle();
    let views = sub.drain();
    assert!(views.len() >= 2, "expected two view changes, got {views:?}");
    for w in views.windows(2) {
        assert!(w[0].version < w[1].version, "views out of order");
    }
    let first = views
        .iter()
        .position(|v| v.owner(a.shard).is_some_and(|o| o.epoch == a.epoch))
        .expect("the first change was delivered");
    let second = views
        .iter()
        .position(|v| v.owner(b.shard).is_some_and(|o| o.epoch == b.epoch))
        .expect("the second change was delivered");
    assert!(first <= second, "the first change arrived after the second");
    assert!(
        views[..second]
            .iter()
            .all(|v| v.owner(b.shard).is_none_or(|o| o.epoch < b.epoch)),
        "the second change appeared before it happened"
    );
}

/// A shard is leased to one node at a time, every new grant raises the epoch, and a renewal
/// keeps it.
pub fn leases_are_exclusive_with_rising_epochs(h: &mut dyn Harness) {
    let shard = ShardId::new(3);
    let first = acquire(h, 0, shard).expect("grant to node 0");
    match acquire(h, 1, shard) {
        Err(LeaseError::Held { owner, .. }) => assert_eq!(&owner, h.nodes()[0].local()),
        other => panic!("node 1 must not get a held shard: {other:?}"),
    }
    let renewed = {
        let n = h.nodes()[0].clone();
        let mut r = n.renew(&first);
        while matches!(r, Err(LeaseError::Pending { .. })) {
            h.settle();
            r = n.held(shard).ok_or(LeaseError::Pending { shard });
        }
        r.expect("renewal")
    };
    assert_eq!(renewed.epoch, first.epoch, "renewal keeps the epoch");
    release(h, 0, &renewed);
    let second = acquire(h, 1, shard).expect("grant to node 1 after release");
    assert!(second.epoch > first.epoch, "a new grant raises the epoch");
    assert!(h.nodes()[0].held(shard).is_none());
    assert!(h.nodes()[1].held(shard).is_some());
}

/// Only the lease holder activates an actor; every other node forwards to it.
pub fn activation_only_on_the_owner(h: &mut dyn Harness) {
    let nodes = h.nodes();
    let shard = ShardId::new(10);
    let actor = &actor_in(nodes[0].as_ref(), shard);
    acquire(h, 1, shard).expect("grant");
    h.settle();
    for (i, n) in h.nodes().iter().enumerate() {
        let gate = ActivationGate::new(n.clone(), 3);
        match (i, gate.inbound(actor, 0)) {
            (1, Inbound::Deliver(l)) => assert_eq!(l.shard, shard),
            (1, other) => panic!("the owner must deliver: {other:?}"),
            (_, Inbound::Forward { to, redirect, .. }) => {
                assert_eq!(&to, nodes[1].local());
                assert_eq!(redirect.shard, shard);
            }
            (_, other) => panic!("node {i} must forward: {other:?}"),
        }
        if i != 1 {
            assert!(gate.may_activate(actor).is_err());
        }
    }
}

/// An actor IRI whose shard is `shard`.
pub fn actor_in(m: &dyn Membership, shard: ShardId) -> String {
    (0u64..)
        .map(|i| format!("urn:vt:actor:kit-{i}"))
        .find(|iri| m.meta().shard_of(iri) == shard)
        .unwrap_or_default()
}

/// Every node routes a shard to its owner from its own view.
pub fn routing_follows_the_view(h: &mut dyn Harness) {
    let shard = ShardId::new(5);
    let lease = acquire(h, 2, shard).expect("grant");
    h.settle();
    for n in h.nodes() {
        let mut table = RoutingTable::new(n.meta().clone());
        table.apply_view(&n.view());
        let o = table.owner(shard).expect("an owner");
        assert_eq!(&o.node, h.nodes()[2].local());
        assert_eq!(o.epoch, lease.epoch);
    }
}

/// Every check of the kit, in order, on fresh shards.
pub fn run_all(h: &mut dyn Harness) {
    subscribers_receive_changes_in_order(h);
    leases_are_exclusive_with_rising_epochs(h);
    activation_only_on_the_owner(h);
    routing_follows_the_view(h);
}
