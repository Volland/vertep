//! TSP on runtime nodes: direct mode with relationship setup, routed mode through a test
//! intermediary (forwarding and store-and-forward), and refusals reported to the sending chart.
#![cfg(feature = "tsp")]

mod world;

use std::sync::Arc;

use vertep_federation::testkit::TestIntermediary;
use vertep_federation::tsp::TspProtocol;
use vertep_federation::vocab::TSP_PROCESSOR;
use vertep_federation::{
    ActorIdentity, DropBox, Grant, InMemoryKeyManager, OutboundOutcome, PickupBinding, Recipients,
    RelayProtocol,
};
use world::*;

const A_TSP: &str = "https://a.example/tsp";
const B_TSP: &str = "https://b.example/tsp";

struct Tsp {
    w: World,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
    b_tsp: Arc<TspProtocol>,
}

/// Org A's sender and org B's receiver, both DID actors with TSP endpoints. B's service routes
/// through `intermediaries` when given.
fn tsp_pair(w: World, intermediaries: &[String], extra: Vec<PickupBinding>) -> Tsp {
    let a_tsp = Arc::new(TspProtocol::new(true));
    let b_tsp = Arc::new(TspProtocol::new(true));
    let inbound = DropBox::new("b-inbound");
    let a = w.org(OrgSpec {
        protocols: vec![a_tsp.clone() as Arc<dyn RelayProtocol>],
        ..OrgSpec::default()
    });
    let mut bindings = vec![drop_binding(
        "tsp",
        &inbound,
        b_tsp.clone() as Arc<dyn RelayProtocol>,
    )];
    bindings.extend(extra);
    let b = w.org(OrgSpec {
        protocols: vec![b_tsp.clone() as Arc<dyn RelayProtocol>],
        bindings,
        ..OrgSpec::default()
    });
    w.net.serve(
        B_TSP,
        Arc::new(b_tsp.endpoint(b.fed.identities().clone(), inbound, w.registry.clone())),
    );
    let a_id = a.did_actor("sender", "a1", vec![tsp_service(A_TSP, &[])]);
    let b_id = b.did_actor("receiver", "b1", vec![tsp_service(B_TSP, intermediaries)]);
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*"],
    ));
    Tsp {
        w,
        a,
        b,
        a_id,
        b_id,
        b_tsp,
    }
}

impl Tsp {
    fn go(&mut self) {
        self.a
            .node
            .send(
                &addr(&self.a_id.did),
                ev("go", obj(&[("to", s(&self.b_id.did))])),
            )
            .expect("send");
        settle(&mut [&mut self.a, &mut self.b]);
    }

    fn errors(&self) -> Vec<vertep_core::datamodel::Event> {
        inputs(&self.a.node, &addr(&self.a_id.did))
            .into_iter()
            .filter(|e| e.name == "error.communication")
            .collect()
    }
}

fn intermediary(w: &World, store_and_forward: bool) -> Arc<TestIntermediary> {
    let i = TestIntermediary::new(
        Arc::new(InMemoryKeyManager::new()),
        INTERMEDIARY,
        w.net.clone(),
        w.registry.clone(),
        store_and_forward,
    )
    .expect("intermediary");
    w.net.serve(INTERMEDIARY, i.clone());
    i
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Direct Transport#Direct exchange]] {level: e2e}
#[test]
fn direct_exchange_between_runtime_nodes() {
    let mut t = tsp_pair(World::new(), &[], Vec::new());
    t.go();
    let got = inputs(&t.b.node, &addr(&t.b_id.did));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "order.placed");
    assert_eq!(got[0].origin.as_deref(), Some(t.a_id.did.as_str()));
    assert_eq!(got[0].origintype.as_deref(), Some(TSP_PROCESSOR));
    assert_eq!(got[0].sendid.as_deref(), Some("s-17"));
    // The relationship was set up before the event: two posts (request, event).
    assert_eq!(t.w.net.posts_to(B_TSP), 2);
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Direct Transport#Relationship required]] {level: e2e}
#[test]
fn refused_relationship_raises_error_communication() {
    let mut t = tsp_pair(World::new(), &[], Vec::new());
    t.b_tsp.refuse_relationships_from(&t.a_id.did);
    t.go();
    let errors = t.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].sendid.as_deref(), Some("s-17"));
    assert!(format!("{:?}", errors[0].data).contains("refused"));
    assert!(names(&t.b.node, &addr(&t.b_id.did)).is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Routed Transport Through Intermediaries#Routed through one intermediary]] {level: e2e}
#[test]
fn routed_through_one_intermediary() {
    let w = World::new();
    let i = intermediary(&w, false);
    let mut t = tsp_pair(w, &[i.did().to_owned()], Vec::new());
    t.go();
    let got = inputs(&t.b.node, &addr(&t.b_id.did));
    assert_eq!(got.len(), 1, "{:?}", t.a.fed.outbound_records());
    assert_eq!(got[0].origin.as_deref(), Some(t.a_id.did.as_str()));
    // What the intermediary received never shows the event.
    let seen: Vec<Vec<u8>> =
        t.w.net
            .posts()
            .into_iter()
            .filter(|(u, _)| u == INTERMEDIARY)
            .map(|(_, b)| b)
            .collect();
    assert_eq!(seen.len(), 1);
    for body in seen {
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("order.placed") && !text.contains("o-1"));
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Routed Transport Through Intermediaries#Intermediary refuses to route]] {level: e2e}
#[test]
fn intermediary_refuses_to_route() {
    let w = World::new();
    let i = intermediary(&w, false);
    let mut t = tsp_pair(w, &[i.did().to_owned()], Vec::new());
    i.set_refusing(true);
    t.go();
    let errors = t.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].sendid.as_deref(), Some("s-17"));
    assert!(matches!(
        t.a.fed.outbound_records().last().map(|r| &r.outcome),
        Some(OutboundOutcome::Failed { .. })
    ));
}

/// Task 8.3: the intermediary holds the message for the recipient, which picks it up.
#[test]
fn intermediary_mailbox_stores_and_forwards() {
    let w = World::new();
    let i = intermediary(&w, true);
    let binding = PickupBinding {
        name: "intermediary".into(),
        source: i.clone(),
        protocol: Arc::new(TspProtocol::new(true)),
        recipients: Recipients::Shared,
    };
    let mut t = tsp_pair(w, &[i.did().to_owned()], vec![binding]);
    t.a.node
        .send(&addr(&t.a_id.did), ev("go", obj(&[("to", s(&t.b_id.did))])))
        .expect("send");
    t.a.node.run_until_idle();
    t.a.fed.pump_outbox();
    assert_eq!(i.store().held_for(&t.b_id.did).len(), 1, "held for pickup");
    settle(&mut [&mut t.a, &mut t.b]);
    assert_eq!(names(&t.b.node, &addr(&t.b_id.did)), vec!["order.placed"]);
    assert!(i.store().held().is_empty(), "acknowledged after journaling");
}
