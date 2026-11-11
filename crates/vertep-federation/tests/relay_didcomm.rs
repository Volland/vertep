//! DIDComm relay mailboxes on runtime nodes: two organisations exchanging events through a test
//! mediator, with pickup, acknowledgement after the journal append, deduplication, activation on
//! delivery, shared and dedicated mailboxes, retries and failure reporting.
#![cfg(feature = "didcomm")]

mod world;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use vertep_core::datamodel::Value;
use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::directory::TransportKind;
use vertep_federation::testkit::TestMediator;
use vertep_federation::{
    ActorIdentity, Grant, OutboundOutcome, PickupSource, Recipients, RelayNetwork, RelayProtocol,
};
use vertep_runtime::{CrashPoint, NodeConfig};
use world::*;

struct Pair {
    w: World,
    m: Arc<TestMediator>,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
}

fn didcomm() -> Arc<dyn RelayProtocol> {
    Arc::new(DidCommProtocol)
}

/// Org A hosts a sender DID actor, org B a receiver DID actor whose mailbox is at the mediator.
fn pair_with(b_spec: impl FnOnce(&Arc<TestMediator>) -> OrgSpec) -> Pair {
    let w = World::new();
    let m = mediator(&w);
    let a = w.org(OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Shared)],
        ..OrgSpec::default()
    });
    let b = w.org(b_spec(&m));
    let rk = vec![m.routing_key()];
    let a_id = a.did_actor("sender", "a1", vec![didcomm_service(MEDIATOR, &rk)]);
    let b_id = b.did_actor("receiver", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*", "approve"],
    ));
    Pair {
        w,
        m,
        a,
        b,
        a_id,
        b_id,
    }
}

fn pair() -> Pair {
    pair_with(|m| OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(m, Recipients::Shared)],
        ..OrgSpec::default()
    })
}

impl Pair {
    fn send(&mut self, n: usize) {
        for i in 0..n {
            self.a
                .node
                .send(
                    &addr(&self.a_id.did),
                    ev(
                        "send",
                        obj(&[
                            ("to", s(&self.b_id.did)),
                            ("event", s(&format!("order.m{}", i + 1))),
                            ("payload", obj(&[("n", Value::Number((i + 1) as f64))])),
                        ]),
                    ),
                )
                .expect("send");
            self.a.node.run_until_idle();
        }
        self.a.fed.pump_outbox();
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Mediator Mailbox#Message stored while the actor is deactivated]] {level: integration}
#[test]
fn message_stored_while_the_actor_is_deactivated() {
    let mut p = pair();
    p.send(1);
    // The sender's send is complete: nothing pending in its relay outbox, the mediator holds it.
    assert!(p.a.fed.outbox().pending().is_empty());
    assert!(matches!(
        p.a.fed.outbound_records().last().map(|r| &r.outcome),
        Some(OutboundOutcome::Sent { .. })
    ));
    assert_eq!(p.m.store().held_for(&p.b_id.did).len(), 1);
    let b = addr(&p.b_id.did);
    assert!(!p.b.node.is_active(&b));
    assert!(names(&p.b.node, &b).is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#Activation On Delivery#Message wakes an actor]] {level: integration}
#[test]
fn message_wakes_an_actor() {
    let mut p = pair();
    let b = addr(&p.b_id.did);
    p.send(1);
    settle(&mut [&mut p.a, &mut p.b]);
    assert_eq!(names(&p.b.node, &b), vec!["order.m1"]);
    p.b.node.deactivate(&b);
    assert!(!p.b.node.is_active(&b));
    p.send(1);
    let report = p.b.fed.pickup();
    assert_eq!(report.delivered.get(&p.b_id.did), Some(&1));
    p.b.node.run_until_idle();
    assert!(p.b.node.is_active(&b), "the picked-up message activated it");
    assert_eq!(names(&p.b.node, &b), vec!["order.m1", "order.m1"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Mediator Mailbox#Messages survive a node restart]] {level: integration}
#[test]
fn messages_survive_a_node_restart() {
    let mut p = pair();
    p.send(2);
    assert_eq!(p.m.store().held_for(&p.b_id.did).len(), 2);
    // B's node restarts: a new node and federation layer on the same store and keys.
    let store = p.b.store.clone();
    let km = p.b.km.clone();
    drop(std::mem::replace(
        &mut p.b,
        p.w.org(OrgSpec {
            protocols: vec![didcomm()],
            bindings: vec![mediator_binding(&p.m, Recipients::Shared)],
            store: Some(store),
            km: Some(km),
            ..OrgSpec::default()
        }),
    ));
    p.b.rehost("receiver", "b1", &p.b_id);
    p.b.fed.grant_inbound(Grant::new(
        p.a_id.did.as_str(),
        p.b_id.did.as_str(),
        ["order.*"],
    ));
    // The first pump recovers: the mailbox factory picks up what the mediator held.
    p.b.node.run_until_idle();
    let b = addr(&p.b_id.did);
    assert_eq!(names(&p.b.node, &b), vec!["order.m1", "order.m2"]);
    assert!(p.m.store().held_for(&p.b_id.did).is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Message Pickup#Pickup batch]] {level: integration}
#[test]
fn pickup_batch() {
    let mut p = pair();
    p.send(3);
    let held: Vec<String> = p.m.store().held().into_iter().map(|m| m.id).collect();
    assert_eq!(held.len(), 3);
    let report = p.b.fed.pickup();
    assert_eq!(report.fetches, 1);
    assert_eq!(report.delivered.get(&p.b_id.did), Some(&3));
    // Nothing is acknowledged before the journal append.
    assert!(p.m.store().acknowledged().is_empty());
    p.b.node.run_until_idle();
    let b = addr(&p.b_id.did);
    assert_eq!(
        names(&p.b.node, &b),
        vec!["order.m1", "order.m2", "order.m3"]
    );
    assert_eq!(
        p.m.store().acknowledged(),
        held,
        "acknowledged in order, after journaling"
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Message Pickup#Crash before acknowledgement]] {level: integration}
#[test]
fn crash_before_acknowledgement() {
    let mut p = pair_with(|m| OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(m, Recipients::Shared)],
        config: NodeConfig {
            crash_at: Some(CrashPoint::AfterAppend),
            ..NodeConfig::default()
        },
        ..OrgSpec::default()
    });
    p.send(1);
    p.b.fed.pickup();
    p.b.node.run_until_idle();
    let b = addr(&p.b_id.did);
    assert_eq!(names(&p.b.node, &b), vec!["order.m1"], "journaled");
    assert_eq!(
        p.m.store().held_for(&p.b_id.did).len(),
        1,
        "not acknowledged"
    );
    // Recovery: a new node on the same store picks the message up again.
    let store = p.b.store.clone();
    let km = p.b.km.clone();
    p.b = p.w.org(OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&p.m, Recipients::Shared)],
        store: Some(store),
        km: Some(km),
        ..OrgSpec::default()
    });
    p.b.rehost("receiver", "b1", &p.b_id);
    p.b.fed.grant_inbound(Grant::new(
        p.a_id.did.as_str(),
        p.b_id.did.as_str(),
        ["order.*"],
    ));
    p.b.node.run_until_idle();
    assert_eq!(names(&p.b.node, &b), vec!["order.m1"], "processed once");
    assert!(
        p.m.store().held_for(&p.b_id.did).is_empty(),
        "the duplicate was acknowledged"
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Message Pickup#Live delivery]] {level: integration}
#[test]
fn live_delivery() {
    let mut p = pair();
    assert_eq!(p.b.fed.go_live(), vec!["mediator".to_owned()]);
    p.send(1);
    // No poll: the mediator's live session delivered it into the mailbox and woke the actor.
    p.b.node.run_until_idle();
    let b = addr(&p.b_id.did);
    assert_eq!(names(&p.b.node, &b), vec!["order.m1"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#At-Least-Once Delivery With Deduplication#Relay retries delivery]] {level: integration}
#[test]
fn relay_retries_delivery() {
    let mut p = pair();
    p.send(1);
    let copy = p.m.store().held_for(&p.b_id.did)[0].clone();
    settle(&mut [&mut p.a, &mut p.b]);
    // The acknowledgement was lost: the relay delivers the same message again.
    p.m.store().store(&copy.recipient, copy.bytes.clone());
    settle(&mut [&mut p.a, &mut p.b]);
    let b = addr(&p.b_id.did);
    assert_eq!(names(&p.b.node, &b), vec!["order.m1"], "processed once");
    assert!(
        p.m.store().held().is_empty(),
        "the second copy was acknowledged"
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#At-Least-Once Delivery With Deduplication#Sender retries after timeout]] {level: integration}
#[test]
fn sender_retries_after_timeout() {
    let w = World::new();
    let m = mediator(&w);
    let flaky = Arc::new(LostResponses {
        inner: w.net.clone(),
        uri: MEDIATOR.into(),
        remaining: AtomicUsize::new(1),
    });
    let mut a = w.org(OrgSpec {
        protocols: vec![didcomm()],
        net: Some(flaky as Arc<dyn RelayNetwork>),
        ..OrgSpec::default()
    });
    let mut b = w.org(OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Shared)],
        ..OrgSpec::default()
    });
    let rk = vec![m.routing_key()];
    let a_id = a.did_actor("sender", "a1", vec![didcomm_service(MEDIATOR, &rk)]);
    let b_id = b.did_actor("receiver", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*"],
    ));
    a.node
        .send(&addr(&a_id.did), ev("go", obj(&[("to", s(&b_id.did))])))
        .expect("send");
    a.node.run_until_idle();
    a.fed.pump_outbox();
    // The first copy arrived but the sender saw a timeout and will retry.
    assert_eq!(m.store().held_for(&b_id.did).len(), 1);
    assert_eq!(a.fed.outbox().pending().len(), 1);
    w.clock.advance(std::time::Duration::from_secs(2));
    a.fed.pump_outbox();
    assert_eq!(m.store().held_for(&b_id.did).len(), 2, "two copies");
    settle(&mut [&mut a, &mut b]);
    assert_eq!(names(&b.node, &addr(&b_id.did)), vec!["order.placed"]);
    assert!(m.store().held().is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#Shared And Per-Actor Mediator Mailboxes#Shared mailbox for a shard]] {level: integration}
#[test]
fn shared_mailbox_for_a_shard() {
    let mut p = pair();
    let rk = vec![p.m.routing_key()];
    let b2 =
        p.b.did_actor("receiver", "b2", vec![didcomm_service(MEDIATOR, &rk)]);
    let b3 =
        p.b.did_actor("receiver", "b3", vec![didcomm_service(MEDIATOR, &rk)]);
    for t in [&b2, &b3] {
        p.a.fed
            .grant_outbound(Grant::new(p.a_id.did.as_str(), t.did.as_str(), ["*"]));
        p.b.fed
            .grant_inbound(Grant::new(p.a_id.did.as_str(), t.did.as_str(), ["*"]));
    }
    for (to, name) in [
        (&p.b_id.did, "x.one"),
        (&b2.did, "x.two"),
        (&b3.did, "x.three"),
    ] {
        p.a.node
            .send(
                &addr(&p.a_id.did),
                ev(
                    "send",
                    obj(&[("to", s(to)), ("event", s(name)), ("payload", Value::Null)]),
                ),
            )
            .expect("send");
    }
    p.a.node.run_until_idle();
    p.a.fed.pump_outbox();
    p.b.fed.grant_inbound(Grant::new(
        p.a_id.did.as_str(),
        p.b_id.did.as_str(),
        ["x.*"],
    ));
    let before = p.m.store().fetches();
    let report = p.b.fed.pickup();
    assert_eq!(p.m.store().fetches() - before, 1, "one pickup session");
    assert_eq!(report.delivered.len(), 3);
    p.b.node.run_until_idle();
    assert_eq!(names(&p.b.node, &addr(&p.b_id.did)), vec!["x.one"]);
    assert_eq!(names(&p.b.node, &addr(&b2.did)), vec!["x.two"]);
    assert_eq!(names(&p.b.node, &addr(&b3.did)), vec!["x.three"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#Shared And Per-Actor Mediator Mailboxes#Dedicated mailbox]] {level: integration}
#[test]
fn dedicated_mailbox() {
    let w = World::new();
    let m = mediator(&w);
    let mut a = w.org(OrgSpec {
        protocols: vec![didcomm()],
        ..OrgSpec::default()
    });
    let km = Arc::new(vertep_federation::InMemoryKeyManager::new());
    let b = w.org(OrgSpec {
        protocols: vec![didcomm()],
        km: Some(km),
        ..OrgSpec::default()
    });
    let rk = vec![m.routing_key()];
    let a_id = a.did_actor("sender", "a1", vec![didcomm_service(MEDIATOR, &rk)]);
    let b1 = b.did_actor("receiver", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    let b2 = b.did_actor("receiver", "b2", vec![didcomm_service(MEDIATOR, &rk)]);
    // A node whose only binding is b1's dedicated mailbox at the mediator.
    let mut dedicated = w.org(OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Dedicated(b1.did.clone()))],
        km: Some(b.km.clone()),
        ..OrgSpec::default()
    });
    dedicated.rehost("receiver", "b1", &b1);
    dedicated.rehost("receiver", "b2", &b2);
    for t in [&b1, &b2] {
        a.fed
            .grant_outbound(Grant::new(a_id.did.as_str(), t.did.as_str(), ["*"]));
        dedicated
            .fed
            .grant_inbound(Grant::new(a_id.did.as_str(), t.did.as_str(), ["*"]));
    }
    for to in [&b1.did, &b2.did] {
        a.node
            .send(&addr(&a_id.did), ev("go", obj(&[("to", s(to))])))
            .expect("send");
    }
    a.node.run_until_idle();
    a.fed.pump_outbox();
    let fetched = m.fetch(std::slice::from_ref(&b1.did), 10).expect("fetch");
    assert!(fetched.iter().all(|x| x.recipient == b1.did));
    settle(&mut [&mut a, &mut dedicated]);
    assert_eq!(names(&dedicated.node, &addr(&b1.did)), vec!["order.placed"]);
    assert!(names(&dedicated.node, &addr(&b2.did)).is_empty());
    assert_eq!(
        m.store().held_for(&b2.did).len(),
        1,
        "b2's message was not fetched"
    );
    assert!(m.store().held_for(&b1.did).is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#Activation On Delivery#Activation fails]] {level: integration}
#[test]
fn activation_fails() {
    let mut p = pair();
    // A second receiver whose chart type is not registered on B's node: activation fails.
    let rk = vec![p.m.routing_key()];
    let broken =
        p.b.did_actor("unregistered", "x", vec![didcomm_service(MEDIATOR, &rk)]);
    p.a.fed
        .grant_outbound(Grant::new(p.a_id.did.as_str(), broken.did.as_str(), ["*"]));
    p.b.fed
        .grant_inbound(Grant::new(p.a_id.did.as_str(), broken.did.as_str(), ["*"]));
    p.a.node
        .send(&addr(&p.a_id.did), ev("go", obj(&[("to", s(&broken.did))])))
        .expect("send");
    p.a.node.run_until_idle();
    p.a.fed.pump_outbox();
    p.b.fed.pickup();
    p.b.node.run_until_idle();
    let x = addr(&broken.did);
    assert!(!p.b.node.is_active(&x));
    assert!(p.b.node.last_error(&x).is_some(), "activation failed");
    assert_eq!(
        p.m.store().held_for(&broken.did).len(),
        1,
        "not acknowledged: still at the mediator"
    );
    assert!(p.m.store().acknowledged().is_empty());
}

// @tg: verifies:: [[openspec:relay-mailboxes#Mailbox Ordering#Order preserved within one mailbox]] {level: integration}
#[test]
fn order_preserved_within_one_mailbox() {
    let mut p = pair();
    p.send(2);
    let held = p.m.store().held_for(&p.b_id.did);
    assert_eq!(held.len(), 2);
    settle(&mut [&mut p.a, &mut p.b]);
    assert_eq!(
        names(&p.b.node, &addr(&p.b_id.did)),
        vec!["order.m1", "order.m2"]
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#Undeliverable Message Reporting#Recipient unreachable]] {level: integration}
#[test]
fn recipient_unreachable() {
    let mut p = pair();
    p.w.net.set_down(MEDIATOR, true);
    let a = addr(&p.a_id.did);
    p.a.node
        .send(&a, ev("go", obj(&[("to", s(&p.b_id.did))])))
        .expect("send");
    for _ in 0..5 {
        p.a.node.run_until_idle();
        p.a.fed.pump_outbox();
        p.w.clock.advance(std::time::Duration::from_secs(20));
    }
    p.a.node.run_until_idle();
    let errors: Vec<_> = inputs(&p.a.node, &a)
        .into_iter()
        .filter(|e| e.name == "error.communication")
        .collect();
    assert_eq!(errors.len(), 1, "raised once the retry policy is exhausted");
    assert_eq!(errors[0].sendid.as_deref(), Some("s-17"));
    let Value::Object(data) = &errors[0].data else {
        panic!("error payload");
    };
    let message = data
        .iter()
        .find(|(k, _)| k == "message")
        .map(|(_, v)| format!("{v:?}"))
        .unwrap_or_default();
    assert!(message.contains("unreachable"), "{message}");
    assert!(p.a.fed.outbox().pending().is_empty());
    assert_eq!(p.w.net.posts_to(MEDIATOR), 0);
}

// @tg: verifies:: [[openspec:relay-mailboxes#Undeliverable Message Reporting#Chart keeps running while retrying]] {level: integration}
#[test]
fn chart_keeps_running_while_retrying() {
    let mut p = pair();
    p.w.net.set_down(MEDIATOR, true);
    let a = addr(&p.a_id.did);
    p.a.node
        .send(&a, ev("go", obj(&[("to", s(&p.b_id.did))])))
        .expect("send");
    p.a.node.run_until_idle();
    p.a.fed.pump_outbox();
    assert_eq!(
        p.a.fed.outbox().pending().len(),
        1,
        "pending while retrying"
    );
    // The chart keeps processing other events meanwhile.
    p.a.node.send(&a, ev("other", Value::Null)).expect("send");
    p.a.node.run_until_idle();
    assert_eq!(names(&p.a.node, &a), vec!["go", "other"]);
    assert_eq!(p.a.fed.outbox().pending().len(), 1);
    p.w.net.set_down(MEDIATOR, false);
    p.w.clock.advance(std::time::Duration::from_secs(2));
    settle(&mut [&mut p.a, &mut p.b]);
    assert!(p.a.fed.outbox().pending().is_empty(), "sent after recovery");
    assert_eq!(names(&p.b.node, &addr(&p.b_id.did)), vec!["order.placed"]);
    assert!(!names(&p.a.node, &a).contains(&"error.communication".to_owned()));
}

#[test]
fn didcomm_transport_is_an_event_io_processor() {
    let p = pair();
    let t = p.a.fed.transport(TransportKind::DidComm);
    assert_eq!(t.type_iri(), vertep_federation::vocab::DIDCOMM_PROCESSOR);
    assert_eq!(t.aliases(), vec!["didcomm".to_owned()]);
}
