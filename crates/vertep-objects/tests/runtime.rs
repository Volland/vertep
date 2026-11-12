//! Durable objects in the actor runtime: a virtual actor at a `did:webvh` address whose
//! journal is a custody journal.

use std::sync::Arc;
use std::time::Duration;

use vertep_core::datamodel::{Event, EventType, Value};
use vertep_objects::ObjectStorage;
use vertep_objects::custody::CustodyHost;
use vertep_objects::testkit::CustodyFixture;
use vertep_runtime::{Address, Node, Shell, VirtualClock};
use vertep_store::MemoryStore;

const TICKET: &str = "https://example.org/charts/ticket";
const SINK: &str = "https://example.org/charts/sink";
const T0: i64 = 1_794_355_200_000;

const TICKET_XML: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="open">
  <datamodel><data id="count" expr="0"/><data id="fired" expr="0"/></datamodel>
  <state id="open">
    <transition event="work"><assign location="count" expr="count + 1"/><send event="done" target="https://example.org/charts/sink/1"/></transition>
    <transition event="arm"><send event="alarm" delay="600s"/></transition>
    <transition event="alarm"><assign location="fired" expr="fired + 1"/><send event="alarm.fired" target="https://example.org/charts/sink/1"/></transition>
  </state>
</scxml>"#;

const SINK_XML: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="s">
  <datamodel><data id="done" expr="0"/><data id="alarms" expr="0"/></datamodel>
  <state id="s">
    <transition event="done"><assign location="done" expr="done + 1"/></transition>
    <transition event="alarm.fired"><assign location="alarms" expr="alarms + 1"/></transition>
  </state>
</scxml>"#;

fn chart(xml: &str) -> vertep_core::ir::Chart {
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    vertep_scxml::read(xml, &opts).unwrap().chart
}

fn node_for(f: &CustodyFixture, host: &Arc<CustodyHost>, shell: Shell) -> Node {
    let storage = Arc::new(ObjectStorage::new(Arc::new(MemoryStore::new())));
    storage.custody().add(host.clone());
    let mut node = Node::builder(storage, shell)
        .datamodels(Arc::new(vertep_dm_ecma::EcmaRegistry))
        .build();
    node.register_type(TICKET, chart(TICKET_XML)).unwrap();
    node.register_type(SINK, chart(SINK_XML)).unwrap();
    node.handle()
        .bind_address(&object(f), TICKET, "8812");
    node
}

fn object(f: &CustodyFixture) -> Address {
    Address::parse(f.object.as_str()).unwrap()
}

fn sink() -> Address {
    Address::parse("https://example.org/charts/sink/1").unwrap()
}

fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

/// The number held by datamodel variable `name` anywhere in a snapshot.
fn var(v: &Value, name: &str) -> Option<f64> {
    match v {
        Value::Object(kv) => kv.iter().find_map(|(k, x)| {
            if k == name
                && let Value::Number(n) = x {
                    return Some(*n);
                }
            var(x, name)
        }),
        Value::Array(a) => match a.as_slice() {
            [Value::String(k), Value::Number(n), ..] if k == name => Some(*n),
            _ => a.iter().find_map(|x| var(x, name)),
        },
        _ => None,
    }
}

fn read(node: &mut Node, a: &Address, name: &str) -> f64 {
    let s = node.state(a).unwrap();
    var(&s.datamodel, name).unwrap_or_else(|| panic!("no {name} in {:?}", s.datamodel))
}

fn five() -> CustodyFixture {
    CustodyFixture::new(
        &["did:web:a.example.org", "did:web:b.example.org", "did:web:c.example.org"],
        &["did:web:k1.example.net", "did:web:k2.example.net"],
        None,
    )
}

// @tg: verifies:: [[openspec:durable-objects#Effects After Durable Commit#No quorum, no effects]] {level: integration}
#[test]
fn no_quorum_no_effects() {
    let f = five();
    let (shell, clock) = Shell::virtual_time(T0);
    let a = f.custody_host("did:web:a.example.org");
    let mut node = node_for(&f, &a, shell);
    node.send(&object(&f), ev("work")).unwrap();
    node.run_until_idle();
    assert_eq!(read(&mut node, &sink(), "done"), 1.0);
    for d in ["did:web:b.example.org", "did:web:c.example.org", "did:web:k1.example.net"] {
        f.set_up(d, false);
    }
    node.send(&object(&f), ev("work")).unwrap();
    node.run_until_idle();
    assert_eq!(
        read(&mut node, &sink(), "done"),
        1.0,
        "no message from a turn no quorum accepted"
    );
    assert_eq!(a.committed().get(), 1, "the turn no quorum accepted is not readable");
    for d in ["did:web:b.example.org", "did:web:c.example.org", "did:web:k1.example.net"] {
        f.set_up(d, true);
    }
    // The failed turn's message was released and the actor parked; after the back-off it is
    // redelivered. The entry the quorum now accepts is replayed (its message id deduplicates
    // the redelivery) and its send dispatched once.
    node.send(&object(&f), ev("work")).unwrap();
    clock.advance(Duration::from_secs(60));
    node.run_until_idle();
    let done = read(&mut node, &sink(), "done");
    let count = read(&mut node, &object(&f), "count");
    assert_eq!(count, 3.0, "three distinct work events, each applied once");
    assert_eq!(done, 3.0, "every committed turn had its effects exactly once");
}

// @tg: verifies:: [[openspec:durable-objects#Effects After Durable Commit#Single-host object]] {level: integration}
#[test]
fn single_host_object_in_the_runtime() {
    let f = CustodyFixture::new(&["did:web:a.example.org"], &[], Some(1));
    let (shell, _clock) = Shell::virtual_time(T0);
    let a = f.custody_host("did:web:a.example.org");
    let mut node = node_for(&f, &a, shell);
    for _ in 0..3 {
        node.send(&object(&f), ev("work")).unwrap();
    }
    node.run_until_idle();
    assert_eq!(read(&mut node, &sink(), "done"), 3.0);
}

// @tg: verifies:: [[openspec:durable-objects#Durable Object Identity#Address survives a host change]] {level: integration}
// @tg: verifies:: [[openspec:durable-objects#Activation From A Verified Replica#Activation on a new host]] {level: integration}
#[test]
fn address_survives_a_host_change() {
    let f = five();
    let (shell, _clock) = Shell::virtual_time(T0);
    let a = f.custody_host("did:web:a.example.org");
    let before = {
        let mut node_a = node_for(&f, &a, shell.clone());
        for _ in 0..2 {
            node_a.send(&object(&f), ev("work")).unwrap();
        }
        node_a.run_until_idle();
        let s = node_a.state(&object(&f)).unwrap();
        a.hand_off().unwrap();
        s
    };
    let b = f.custody_host("did:web:b.example.org");
    let mut node_b = node_for(&f, &b, shell);
    let replayed = node_b.state(&object(&f)).unwrap();
    assert_eq!(replayed.configuration, before.configuration);
    assert_eq!(var(&replayed.datamodel, "count"), var(&before.datamodel, "count"));
    // The sender keeps the same address; the event reaches the object on host B.
    node_b.send(&object(&f), ev("work")).unwrap();
    node_b.run_until_idle();
    assert_eq!(read(&mut node_b, &object(&f), "count"), 3.0);
    // Outbox progress is per node, so B re-dispatches A's two sends with their original
    // message ids (receivers deduplicate them) and then its own.
    assert_eq!(read(&mut node_b, &sink(), "done"), 3.0);
}

fn arm_then_fail_over(f: &CustodyFixture, clock: &VirtualClock, shell: &Shell, fire_on_a: bool) -> Node {
    let a = f.custody_host("did:web:a.example.org");
    {
        let mut node_a = node_for(f, &a, shell.clone());
        node_a.send(&object(f), ev("arm")).unwrap();
        node_a.run_until_idle();
        if fire_on_a {
            clock.advance(Duration::from_secs(600));
            node_a.run_until_idle();
            assert_eq!(read(&mut node_a, &object(f), "fired"), 1.0);
        } else {
            clock.advance(Duration::from_secs(120));
            node_a.run_until_idle();
        }
    } // host A's node is gone
    f.set_up("did:web:a.example.org", false);
    let b = f.custody_host("did:web:b.example.org");
    let mut node_b = node_for(f, &b, shell.clone());
    vertep_objects::serve_object(&mut node_b, &object(f)).unwrap();
    node_b
}

// @tg: verifies:: [[openspec:durable-objects#Durable Alarms#Alarm fires after failover]] {level: integration}
#[test]
fn alarm_fires_after_failover() {
    let f = five();
    let (shell, clock) = Shell::virtual_time(T0);
    let mut node_b = arm_then_fail_over(&f, &clock, &shell, false);
    assert_eq!(read(&mut node_b, &object(&f), "fired"), 0.0, "not yet due");
    assert_eq!(node_b.pending_timers().len(), 1, "B re-armed A's alarm");
    clock.advance(Duration::from_secs(480));
    node_b.run_until_idle();
    assert_eq!(read(&mut node_b, &object(&f), "fired"), 1.0, "fired once on host B");
    clock.advance(Duration::from_secs(3600));
    node_b.run_until_idle();
    assert_eq!(read(&mut node_b, &object(&f), "fired"), 1.0);
}

// @tg: verifies:: [[openspec:durable-objects#Durable Alarms#Alarm not fired twice]] {level: integration}
#[test]
fn alarm_not_fired_twice() {
    let f = five();
    let (shell, clock) = Shell::virtual_time(T0);
    let mut node_b = arm_then_fail_over(&f, &clock, &shell, true);
    assert!(node_b.pending_timers().is_empty(), "a fired timer is not armed again");
    clock.advance(Duration::from_secs(3600));
    node_b.run_until_idle();
    assert_eq!(read(&mut node_b, &object(&f), "fired"), 1.0, "B does not fire it again");
}
