//! Transport selection between TSP and DIDComm from the recipient's DID document, fallback, and
//! the absence of ordering across relays.
#![cfg(all(feature = "tsp", feature = "didcomm"))]

mod world;

use std::sync::Arc;

use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::directory::TransportKind;
use vertep_federation::tsp::TspProtocol;
use vertep_federation::vocab::{DIDCOMM_PROCESSOR, TSP_PROCESSOR};
use vertep_federation::{
    ActorIdentity, DropBox, Grant, OutboundOutcome, Recipients, RelayProtocol,
};
use world::*;

const A_TSP: &str = "https://a.example/tsp";
const B_TSP: &str = "https://b.example/tsp";

struct Both {
    w: World,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
    m: Arc<vertep_federation::testkit::TestMediator>,
}

/// B lists a TSP service and a DIDComm service (through the mediator); A prefers TSP. B's
/// pickups run DIDComm first, then TSP.
fn both() -> Both {
    let w = World::new();
    let m = mediator(&w);
    let a = w.org(OrgSpec {
        protocols: vec![
            Arc::new(TspProtocol::new(true)) as Arc<dyn RelayProtocol>,
            Arc::new(DidCommProtocol),
        ],
        preference: vec![TransportKind::Tsp, TransportKind::DidComm],
        ..OrgSpec::default()
    });
    let b_tsp = Arc::new(TspProtocol::new(true));
    let inbound = DropBox::new("b-inbound");
    let b = w.org(OrgSpec {
        protocols: vec![
            b_tsp.clone() as Arc<dyn RelayProtocol>,
            Arc::new(DidCommProtocol),
        ],
        bindings: vec![
            mediator_binding(&m, Recipients::Shared),
            drop_binding("tsp", &inbound, b_tsp.clone() as Arc<dyn RelayProtocol>),
        ],
        ..OrgSpec::default()
    });
    w.net.serve(
        B_TSP,
        Arc::new(b_tsp.endpoint(b.fed.identities().clone(), inbound, w.registry.clone())),
    );
    let rk = vec![m.routing_key()];
    let a_id = a.did_actor(
        "sender",
        "a1",
        vec![tsp_service(A_TSP, &[]), didcomm_service(MEDIATOR, &rk)],
    );
    let b_id = b.did_actor(
        "receiver",
        "b1",
        vec![tsp_service(B_TSP, &[]), didcomm_service(MEDIATOR, &rk)],
    );
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed
        .grant_inbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    Both {
        w,
        a,
        b,
        a_id,
        b_id,
        m,
    }
}

fn sent_with(b: &Both) -> Vec<(TransportKind, usize)> {
    b.a.fed
        .outbound_records()
        .into_iter()
        .filter_map(|r| match r.outcome {
            OutboundOutcome::Sent {
                transport,
                fell_back_from,
                ..
            } => Some((transport, fell_back_from.len())),
            _ => None,
        })
        .collect()
}

// @tg: verifies:: [[openspec:relay-mailboxes#Transport Selection From The DID Document#Recipient supports both protocols]] {level: e2e}
#[test]
fn recipient_supports_both_protocols() {
    let mut t = both();
    t.a.node
        .send(&addr(&t.a_id.did), ev("go", obj(&[("to", s(&t.b_id.did))])))
        .expect("send");
    settle(&mut [&mut t.a, &mut t.b]);
    assert_eq!(sent_with(&t), vec![(TransportKind::Tsp, 0)]);
    let got = inputs(&t.b.node, &addr(&t.b_id.did));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origintype.as_deref(), Some(TSP_PROCESSOR));
    assert!(
        t.m.store().acknowledged().is_empty(),
        "nothing went through the mediator"
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#Transport Selection From The DID Document#Fallback to the next service]] {level: e2e}
#[test]
fn fallback_to_the_next_service() {
    let mut t = both();
    t.w.net.set_down(B_TSP, true);
    t.a.node
        .send(&addr(&t.a_id.did), ev("go", obj(&[("to", s(&t.b_id.did))])))
        .expect("send");
    settle(&mut [&mut t.a, &mut t.b]);
    // Sent over DIDComm, with the TSP failure recorded as the fallback.
    assert_eq!(sent_with(&t), vec![(TransportKind::DidComm, 1)]);
    let got = inputs(&t.b.node, &addr(&t.b_id.did));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origintype.as_deref(), Some(DIDCOMM_PROCESSOR));
    assert_eq!(got[0].origin.as_deref(), Some(t.a_id.did.as_str()));
}

// @tg: verifies:: [[openspec:relay-mailboxes#Mailbox Ordering#No cross-relay ordering]] {level: e2e}
#[test]
fn no_cross_relay_ordering() {
    let mut t = both();
    let a = addr(&t.a_id.did);
    for (event, ty) in [("m1", "tsp"), ("m2", "didcomm")] {
        t.a.node
            .send(
                &a,
                ev(
                    "typed",
                    obj(&[("to", s(&t.b_id.did)), ("event", s(event)), ("type", s(ty))]),
                ),
            )
            .expect("send");
        t.a.node.run_until_idle();
    }
    t.a.fed.pump_outbox();
    assert_eq!(
        sent_with(&t),
        vec![(TransportKind::Tsp, 0), (TransportKind::DidComm, 0)],
        "m1 left first, over TSP"
    );
    settle(&mut [&mut t.a, &mut t.b]);
    // B picks up from the mediator before its TSP queue, so m2 is journaled first.
    assert_eq!(names(&t.b.node, &addr(&t.b_id.did)), vec!["m2", "m1"]);
}
