//! The worker pool dispatcher: activations on demand, placement on declaring members,
//! balancing policies, limits and the bounded queue.

use vertep_cluster::{NodeId, Policy, Refusal, Submit, WorkerPool, WorkerType};

fn node(s: &str) -> NodeId {
    NodeId::new(s).unwrap()
}

fn pool(ty: WorkerType, local: &str, nodes: &[&str]) -> WorkerPool<u32> {
    let mut p = WorkerPool::new(ty, node(local));
    for n in nodes {
        p.declare(node(n));
    }
    p
}

fn ty() -> WorkerType {
    WorkerType::new("urn:vertep:worker:llm")
}

fn assigned(s: Submit<u32>) -> vertep_cluster::Assignment<u32> {
    match s {
        Submit::Assigned(a) => a,
        other => panic!("expected an assignment, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:worker-pools#Stateless Worker Activations#Activations created on demand]] {level: unit}
#[test]
fn activations_created_on_demand() {
    let mut p = pool(ty(), "n0", &["n1"]);
    assert!(p.activations().is_empty(), "no activation before a message");
    let a = assigned(p.submit("m1", 1));
    assert!(a.created, "an activation was created for the first message");
    assert_eq!(a.node, node("n1"));
    assert_eq!(p.activations().len(), 1);
    p.complete("m1");
    let b = assigned(p.submit("m2", 2));
    assert!(!b.created, "the idle activation handles the next one");
    assert_eq!(b.activation, a.activation);
}

// @tg: verifies:: [[openspec:worker-pools#Pool Placement Across Nodes#Workers spread across nodes]] {level: unit}
#[test]
fn workers_spread_across_nodes() {
    let mut p = pool(ty().with_limits(4, 1, 16), "n0", &["n1", "n2", "n3"]);
    let nodes: std::collections::BTreeSet<NodeId> = (0..6)
        .map(|i| assigned(p.submit(format!("m{i}"), i)).node)
        .collect();
    assert!(
        nodes.len() > 1,
        "activations on more than one node: {nodes:?}"
    );
    assert_eq!(nodes.len(), 3, "load spreads over all three");
    let per_node: Vec<usize> = ["n1", "n2", "n3"]
        .iter()
        .map(|n| p.activations().iter().filter(|a| a.1 == node(n)).count())
        .collect();
    assert_eq!(per_node, vec![2, 2, 2]);
}

// @tg: verifies:: [[openspec:worker-pools#Pool Placement Across Nodes#Node without the worker type is not used]] {level: unit}
#[test]
fn node_without_the_worker_type_is_not_used() {
    // n0 (the sender) and n2 do not declare the type
    let mut p = pool(
        ty().with_limits(8, 1, 64).with_policy(Policy::PreferLocal),
        "n0",
        &["n1", "n3"],
    );
    for i in 0..10 {
        let a = assigned(p.submit(format!("m{i}"), i));
        assert!(
            a.node == node("n1") || a.node == node("n3"),
            "placed on {}",
            a.node
        );
    }
    assert!(
        p.activations()
            .iter()
            .all(|a| a.1 != node("n0") && a.1 != node("n2"))
    );
    // with no declaring member at all, a request is refused
    let mut empty = pool(ty(), "n0", &[]);
    assert_eq!(empty.submit("m", 0), Submit::Refused(Refusal::NoNode));
}

// @tg: verifies:: [[openspec:worker-pools#Load Balancing Policies#Least outstanding requests]] {level: unit}
#[test]
fn least_outstanding_requests() {
    let mut p = pool(
        ty().with_limits(1, 10, 16)
            .with_policy(Policy::LeastOutstanding),
        "n0",
        &["n1", "n2"],
    );
    // two activations: one with five requests in progress, one with one
    let a = assigned(p.submit("a0", 0));
    let mut busy = Vec::new();
    for i in 1..5 {
        busy.push(assigned(p.submit(format!("a{i}"), i)).activation);
    }
    assert!(busy.iter().all(|x| *x == a.activation));
    // force a second activation by filling the first up to its limit, then free all but one
    for i in 5..10 {
        p.submit(format!("a{i}"), i);
    }
    let second = assigned(p.submit("b0", 100));
    assert_ne!(second.activation, a.activation);
    for i in 5..10 {
        p.complete(&format!("a{i}"));
    }
    assert_eq!(p.in_flight(a.activation), 5);
    assert_eq!(p.in_flight(second.activation), 1);
    let next = assigned(p.submit("next", 7));
    assert_eq!(
        next.activation, second.activation,
        "the one with one in progress"
    );
}

// @tg: verifies:: [[openspec:worker-pools#Load Balancing Policies#Prefer local node]] {level: unit}
#[test]
fn prefer_local_node() {
    let local = ty().with_limits(1, 1, 16).with_policy(Policy::PreferLocal);
    let mut p = pool(local, "n0", &["n0", "n1"]);
    // one activation on each node, both idle again
    let a = assigned(p.submit("m1", 1));
    let b = assigned(p.submit("m2", 2));
    assert_eq!(
        a.node,
        node("n0"),
        "the first activation goes to the sender's node"
    );
    assert_eq!(
        b.node,
        node("n1"),
        "the local one is busy and at the node limit"
    );
    p.complete("m1");
    p.complete("m2");
    for i in 3..7 {
        let x = assigned(p.submit(format!("m{i}"), i));
        assert_eq!(
            x.node,
            node("n0"),
            "handled on the sender's node, no network hop"
        );
        assert_eq!(x.activation, a.activation);
        p.complete(&format!("m{i}"));
    }
    // round robin, by contrast, alternates between the two idle activations
    let mut rr = pool(ty().with_limits(1, 1, 16), "n0", &["n0", "n1"]);
    rr.submit("m1", 1);
    rr.submit("m2", 2);
    rr.complete("m1");
    rr.complete("m2");
    let x = assigned(rr.submit("m3", 3)).node;
    rr.complete("m3");
    let y = assigned(rr.submit("m4", 4)).node;
    assert_ne!(x, y);
}

// @tg: verifies:: [[openspec:worker-pools#Worker Concurrency Limits#Limit reached]] {level: unit}
#[test]
fn limit_reached() {
    // two nodes, one activation each, two requests per activation: four in progress at most
    let mut p = pool(ty().with_limits(1, 2, 8), "n0", &["n1", "n2"]);
    for i in 0..4 {
        assigned(p.submit(format!("m{i}"), i));
    }
    assert_eq!(
        p.activations().len(),
        2,
        "the per-node activation limit holds"
    );
    assert_eq!(p.submit("m4", 4), Submit::Queued, "the fifth waits");
    assert_eq!(p.submit("m5", 5), Submit::Queued);
    assert_eq!(p.queued(), 2);
    let freed = p.complete("m0");
    assert_eq!(
        freed.len(),
        1,
        "capacity freed: the oldest waiting request runs"
    );
    assert_eq!(freed[0].request, "m4");
    assert_eq!(p.queued(), 1);
}

// @tg: verifies:: [[openspec:worker-pools#Worker Concurrency Limits#Queue bound reached]] {level: unit}
#[test]
fn queue_bound_reached() {
    let mut p = pool(ty().with_limits(1, 1, 2), "n0", &["n1"]);
    assigned(p.submit("m0", 0));
    assert_eq!(p.submit("m1", 1), Submit::Queued);
    assert_eq!(p.submit("m2", 2), Submit::Queued);
    assert_eq!(
        p.submit("m3", 3),
        Submit::Refused(Refusal::QueueFull { bound: 2 }),
        "refused at once"
    );
}

#[test]
fn a_crashed_activation_is_retried_once_only_when_idempotent() {
    let mut p = pool(ty().with_limits(1, 1, 8).idempotent(), "n0", &["n1", "n2"]);
    let a = assigned(p.submit("m", 7));
    let out = p.activation_crashed("m");
    let retry = match &out[0] {
        vertep_cluster::Outcome::Retried(r) => r.clone(),
        other => panic!("{other:?}"),
    };
    assert_ne!(retry.activation, a.activation);
    // a second crash is reported
    let out = p.activation_crashed("m");
    assert!(matches!(out[0], vertep_cluster::Outcome::Failed { .. }));

    let mut q = pool(ty().with_limits(1, 1, 8), "n0", &["n1", "n2"]);
    assigned(q.submit("m", 7));
    assert!(matches!(
        q.activation_crashed("m")[0],
        vertep_cluster::Outcome::Failed { .. }
    ));
}
