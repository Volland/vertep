//! Scenarios of the `durable-timers` capability.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, Envelope, Node, NodeConfig, VirtualClock};
use vertep_store::MemoryStore;

const REMIND: &str = r##"<state id="s">
    <transition event="arm"><send event="remind" delay="5s" id="r1"/></transition>
    <transition event="stop"><cancel sendid="r1"/></transition>
    <transition event="remind"/>
  </state>"##;

fn names(node: &Node, a: &Address) -> Vec<String> {
    node.journal(a)
        .unwrap()
        .iter()
        .filter_map(|e| match &e.body {
            EntryBody::Input { input, .. } => Some(input.event.name.clone()),
            _ => None,
        })
        .collect()
}

fn armed() -> (Node, VirtualClock, Arc<MemoryStore>, Address) {
    let (mut node, clock, store) = fresh();
    node.register_type(&ty("r"), scxml("s", REMIND)).unwrap();
    let a = va("r", "a");
    node.send(&a, ev("arm")).unwrap();
    node.run_until_idle();
    (node, clock, store, a)
}

// @tg: verifies:: [[openspec:durable-timers#Delayed Send Is Journaled#Timer recorded with its macrostep]] {level: integration}
#[test]
fn timer_recorded_with_its_macrostep() {
    let (node, _clock, _store, a) = armed();
    let j = node.journal(&a).unwrap();
    let eff = j[0].effects().unwrap();
    assert_eq!(eff.timers.len(), 1);
    let t = &eff.timers[0];
    assert_eq!(t.send_id, "r1");
    assert_eq!(t.due_ms, j[0].timestamp_ms + 5_000);
    assert_eq!(t.event.name, "remind");
    assert_eq!(t.recipient, a.as_str());
    assert!(eff.sends.is_empty(), "held as a timer, not dispatched");
}

// @tg: verifies:: [[openspec:durable-timers#Delayed Send Is Journaled#Timer survives a crash]] {level: integration}
#[test]
fn timer_survives_a_crash() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut n1 = node_on(store.clone(), &clock, NodeConfig::default());
    n1.register_type(&ty("r"), scxml("s", REMIND)).unwrap();
    let a = va("r", "a");
    n1.send(&a, ev("arm")).unwrap();
    n1.run_until_idle();
    drop(n1); // the process dies before the timer is due
    clock.advance(Duration::from_secs(2));
    let mut n2 = node_on(store.clone(), &clock, NodeConfig::default());
    n2.register_type(&ty("r"), scxml("s", REMIND)).unwrap();
    n2.run_until_idle();
    assert_eq!(names(&n2, &a), vec!["arm"], "not due yet");
    clock.advance(Duration::from_secs(3));
    n2.run_until_idle();
    assert_eq!(names(&n2, &a), vec!["arm", "remind"]);
    assert_eq!(n2.journal(&a).unwrap()[1].timestamp_ms, T0 + 5_000);
}

// @tg: verifies:: [[openspec:durable-timers#Timer Fires As Input#Due timer delivered]] {level: integration}
#[test]
fn due_timer_delivered() {
    let (mut node, clock, _store, a) = armed();
    clock.advance(Duration::from_secs(5));
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    assert_eq!(j.len(), 2);
    let EntryBody::Input { input, .. } = &j[1].body else {
        panic!()
    };
    assert_eq!(input.event.name, "remind");
    let timer = &j[0].effects().unwrap().timers[0];
    assert_eq!(input.timer.as_deref(), Some(timer.iri.as_str()));
    assert_eq!(
        input.message_id, timer.iri,
        "message id derived from the send id"
    );
}

// @tg: verifies:: [[openspec:durable-timers#Timer Fires As Input#Timer fires once]] {level: integration}
#[test]
fn timer_fires_once() {
    let (mut node, clock, _store, a) = armed();
    let timer = node.journal(&a).unwrap()[0].effects().unwrap().timers[0].clone();
    clock.advance(Duration::from_secs(5));
    node.run_until_idle();
    // the timer service retries the same delivery
    let mut env = Envelope::new(timer.message_id(0), &a, timer.event.clone());
    env.timer = Some(timer.iri.clone());
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["arm", "remind"]);
}

// @tg: verifies:: [[openspec:durable-timers#Timer Cancellation#Cancel before due]] {level: integration}
#[test]
fn cancel_before_due() {
    let (mut node, clock, _store, a) = armed();
    node.send(&a, ev("stop")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    let timer = j[0].effects().unwrap().timers[0].iri.clone();
    assert_eq!(
        j[1].effects().unwrap().cancels,
        vec![timer],
        "the cancellation is journaled"
    );
    clock.advance(Duration::from_secs(60));
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["arm", "stop"], "never delivered");
    assert!(node.pending_timers().is_empty());
}

// @tg: verifies:: [[openspec:durable-timers#Timer Cancellation#Cancel after fired]] {level: integration}
#[test]
fn cancel_after_fired() {
    let (mut node, clock, _store, a) = armed();
    clock.advance(Duration::from_secs(5));
    node.run_until_idle();
    node.send(&a, ev("stop")).unwrap();
    node.run_until_idle();
    assert_eq!(
        names(&node, &a),
        vec!["arm", "remind", "stop"],
        "no error event"
    );
    assert!(
        node.journal(&a).unwrap()[2]
            .effects()
            .unwrap()
            .cancels
            .is_empty()
    );
}

// @tg: verifies:: [[openspec:durable-timers#Timers Reactivate Virtual Actors#Reminder wakes an idle actor]] {level: integration}
#[test]
fn reminder_wakes_an_idle_actor() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = node_on(
        store,
        &clock,
        NodeConfig {
            idle_timeout_ms: 60_000,
            ..NodeConfig::default()
        },
    );
    node.register_type(
        &ty("sleeper"),
        scxml(
            "s",
            r##"<state id="s"><transition event="arm"><send event="wake" delay="3600s"/></transition>
               <transition event="wake" target="awake"/></state><state id="awake"/>"##,
        ),
    )
    .unwrap();
    let a = va("sleeper", "a");
    node.send(&a, ev("arm")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_secs(120));
    node.run_until_idle();
    assert!(!node.is_active(&a), "deactivated for idleness");
    clock.advance(Duration::from_secs(3600 - 120));
    node.run_until_idle();
    assert!(node.is_active(&a));
    assert_eq!(node.state(&a).unwrap().configuration, vec!["awake"]);
}

// @tg: verifies:: [[openspec:durable-timers#Timer Time Source#Replay reproduces due times]] {level: integration}
#[test]
fn replay_reproduces_due_times() {
    let (mut node, clock, _store, a) = armed();
    let due = node.pending_timers()[0].0;
    clock.advance(Duration::from_secs(2));
    node.deactivate(&a);
    node.activate_actor(&a).unwrap();
    assert_eq!(node.pending_timers().len(), 1);
    assert_eq!(
        node.pending_timers()[0].0,
        due,
        "computed from the journaled timestamp"
    );
    clock.advance(Duration::from_secs(3));
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["arm", "remind"]);
    // replay after the firing delivers nothing again
    node.deactivate(&a);
    node.activate_actor(&a).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["arm", "remind"]);
    assert!(node.pending_timers().is_empty());
}

const POLLER: &str = r##"<state id="polling">
    <onentry><vt:reminder name="poll" period="10min" event="poll.tick"/></onentry>
    <transition event="poll.tick"/>
    <transition event="stop" target="done"/></state>
  <final id="done"/>"##;

// @tg: verifies:: [[openspec:durable-timers#Recurring Reminders#Periodic reminder]] {level: integration}
#[test]
fn periodic_reminder() {
    let (mut node, clock, _store) = fresh();
    node.register_type(&ty("poller"), scxml("polling", POLLER))
        .unwrap();
    let a = va("poller", "a");
    node.send(&a, ev("start")).unwrap();
    node.run_until_idle();
    for _ in 0..3 {
        clock.advance(Duration::from_secs(600));
        node.run_until_idle();
    }
    assert_eq!(
        names(&node, &a),
        vec!["start", "poll.tick", "poll.tick", "poll.tick"]
    );
    let j = node.journal(&a).unwrap();
    let stamps: Vec<i64> = j[1..].iter().map(|e| e.timestamp_ms).collect();
    assert_eq!(stamps, vec![T0 + 600_000, T0 + 1_200_000, T0 + 1_800_000]);
    let ids: std::collections::BTreeSet<String> =
        j[1..].iter().map(|e| e.message_id.clone()).collect();
    assert_eq!(ids.len(), 3, "each occurrence is a separate input");
}

// @tg: verifies:: [[openspec:durable-timers#Recurring Reminders#Reminder stops at final]] {level: integration}
#[test]
fn reminder_stops_at_final() {
    let (mut node, clock, _store) = fresh();
    node.register_type(&ty("poller"), scxml("polling", POLLER))
        .unwrap();
    let a = va("poller", "a");
    node.send(&a, ev("start")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_secs(600));
    node.run_until_idle();
    node.send(&a, ev("stop")).unwrap();
    node.run_until_idle();
    assert!(node.state(&a).unwrap().terminated);
    assert!(
        !node.journal(&a).unwrap()[2]
            .effects()
            .unwrap()
            .cancels
            .is_empty()
    );
    clock.advance(Duration::from_secs(3_600));
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["start", "poll.tick", "stop"]);
    assert!(node.pending_timers().is_empty());
}

// @tg: verifies:: [[openspec:durable-timers#Missed Timers After Downtime#Catching up after downtime]] {level: integration}
#[test]
fn catching_up_after_downtime() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let chart = || {
        scxml(
            "s",
            r##"<state id="s"><onentry>
                <vt:reminder name="beat" period="2s" event="beat"/>
                <send event="second" delay="7s"/><send event="first" delay="5s"/></onentry>
              <transition event="*"/></state>"##,
        )
    };
    let mut n1 = node_on(store.clone(), &clock, NodeConfig::default());
    n1.register_type(&ty("c"), chart()).unwrap();
    let a = va("c", "a");
    n1.send(&a, ev("start")).unwrap();
    n1.run_until_idle();
    drop(n1);
    // down while the beat was due at 2, 4, 6, 8 and 10 s and both timers became due
    clock.advance(Duration::from_millis(10_500));
    let mut n2 = node_on(store.clone(), &clock, NodeConfig::default());
    n2.register_type(&ty("c"), chart()).unwrap();
    n2.run_until_idle();
    assert_eq!(names(&n2, &a), vec!["start", "first", "second", "beat"]);
    clock.advance(Duration::from_millis(1_500));
    n2.run_until_idle();
    assert_eq!(
        names(&n2, &a),
        vec!["start", "first", "second", "beat", "beat"],
        "back on period"
    );
}
