//! Scenarios of the `actor-model` capability.
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, EvalError, Event, SessionInfo, StateQuery, Value,
};
use vertep_core::null::NullDatamodel;
use vertep_runtime::actor::START_EVENT;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{
    Actor, ActorFactory, ActorInput, ActorStart, Address, ChartActor, FnActor, NodeConfig,
    OutTarget, Outgoing, RegisterError, Turn, TurnOutput,
};
use vertep_store::{GraphStore, JournalStore, MemoryStore, Position};

const SINK: &str = r##"<state id="s"><transition event="*"/></state>"##;

fn names(node: &vertep_runtime::Node, a: &Address) -> Vec<String> {
    node.journal(a)
        .unwrap()
        .iter()
        .filter_map(|e| match &e.body {
            EntryBody::Input { input, .. } => Some(input.event.name.clone()),
            _ => None,
        })
        .collect()
}

// @tg: verifies:: [[openspec:actor-model#Actor Trait#Chart is an actor]] {level: integration}
#[test]
fn chart_is_an_actor() {
    fn is_actor<A: Actor>() {}
    is_actor::<ChartActor>();
    let (mut node, _clock, _store) = fresh();
    // the same chart, reached by <send> as a virtual actor and by <invoke> as a child
    let child = ty("echo");
    node.register_type(
        &child,
        scxml(
            "s",
            r##"<state id="s"><transition event="ping" target="s"><send event="pong" target="#_parent"/></transition></state>"##,
        ),
    )
    .unwrap();
    node.register_type(
        &ty("parent"),
        scxml(
            "run",
            &format!(
                r##"<state id="run"><invoke type="scxml" src="{child}" id="kid"/>
                <onentry><send event="ping" target="#_kid"/></onentry>
                <transition event="pong" target="got"/></state><state id="got"/>"##
            ),
        ),
    )
    .unwrap();
    let echo = Address::virtual_actor(&child, "direct").unwrap();
    node.send(&echo, ev("ping")).unwrap();
    let p = va("parent", "p");
    node.send(&p, ev("start")).unwrap();
    node.run_until_idle();
    // reached by <send>; as a virtual actor it has no parent, so its #_parent send fails
    assert_eq!(names(&node, &echo), vec!["ping", "error.communication"]);
    assert_eq!(
        node.state(&p).unwrap().configuration,
        vec!["got"],
        "invoked as a child"
    );
}

/// A non-chart actor kind: answers `ping` with `pong` to its parent, completes on `stop`.
struct Echo {
    address: Address,
}

impl Actor for Echo {
    fn address(&self) -> &Address {
        &self.address
    }
    fn turn(&mut self, input: ActorInput) -> Turn {
        let mut out = TurnOutput::default();
        match input.event.name.as_str() {
            "ping" => out
                .sends
                .push(Outgoing::new(OutTarget::Parent, "pong", input.event.data)),
            "stop" => out.done = Some(Value::String("bye".into())),
            _ => {}
        }
        Turn::Ready(Ok(out))
    }
}

fn echo_kind() -> ActorFactory {
    Arc::new(|s: &ActorStart| {
        Ok(Box::new(Echo {
            address: s.address.clone(),
        }) as Box<dyn Actor>)
    })
}

// @tg: verifies:: [[openspec:actor-model#Actor Trait#Non-chart actor is invokable]] {level: integration}
#[test]
fn non_chart_actor_is_invokable() {
    let (mut node, _clock, _store) = fresh();
    node.register_actor_kind("urn:vertep:kind:echo", echo_kind());
    node.register_type(
        &ty("caller"),
        scxml(
            "run",
            r##"<state id="run"><invoke type="urn:vertep:kind:echo" id="e"/>
               <transition event="go"><send event="ping" target="#_e"/></transition>
               <transition event="pong"><send event="stop" target="#_e"/></transition>
               <transition event="done.invoke.e" target="done"/></state><state id="done"/>"##,
        ),
    )
    .unwrap();
    let a = va("caller", "c");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["go", "pong", "done.invoke.e"]);
    let j = node.journal(&a).unwrap();
    let EntryBody::Input { input, .. } = &j[2].body else {
        panic!()
    };
    assert_eq!(input.event.data, Value::String("bye".into()));
    assert_eq!(input.event.invokeid.as_deref(), Some("e"));
    assert_eq!(node.state(&a).unwrap().configuration, vec!["done"]);
}

// @tg: verifies:: [[openspec:actor-model#Actor Trait#Unknown actor kind]] {level: integration}
#[test]
fn unknown_actor_kind() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("caller"),
        scxml(
            "run",
            r##"<state id="run"><invoke type="urn:vertep:kind:nope" id="x"/>
               <transition event="error.execution" target="failed"/></state><state id="failed"/>"##,
        ),
    )
    .unwrap();
    let a = va("caller", "c");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    assert!(
        !node.is_active(&Address::invoked(&a, "x")),
        "nothing started"
    );
}

// @tg: verifies:: [[openspec:actor-model#IRI Actor Addresses#Address is an IRI]] {level: integration}
#[test]
fn address_is_an_iri() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("who"),
        ecma_scxml(
            "s",
            r##"<datamodel><data id="me" expr="''"/></datamodel>
            <state id="s"><transition event="ask"><assign location="me" expr="_sessionid"/>
            <send event="hello" targetexpr="'#_scxml_' + _sessionid"/></transition>
            <transition event="hello" target="greeted"/></state><state id="greeted"/>"##,
        ),
    )
    .unwrap();
    let a = va("who", "w");
    assert!(a.as_str().starts_with("https://"));
    node.send(&a, ev("ask")).unwrap();
    node.run_until_idle();
    let s = node.state(&a).unwrap();
    assert_eq!(
        data_of(&s.datamodel, "me"),
        Some(Value::String(a.as_str().into()))
    );
    assert_eq!(
        s.configuration,
        vec!["greeted"],
        "#_scxml_<sessionid> resolved through the IRI"
    );
}

// @tg: verifies:: [[openspec:actor-model#IRI Actor Addresses#Address names the graph subject]] {level: integration}
#[test]
fn address_names_the_graph_subject() {
    let (mut node, _clock, store) = fresh();
    node.register_type(
        &ty("t"),
        scxml(
            "a",
            r##"<state id="a"><transition event="go" target="b"/></state><state id="b"/>"##,
        ),
    )
    .unwrap();
    let a = va("t", "x");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let q = format!(
        "SELECT ?s ?p ?o WHERE {{ GRAPH {a} {{ ?s ?p ?o }} }}",
        a = a.node()
    );
    let sol = store.query(&q).unwrap().into_solutions().unwrap();
    assert!(!sol.is_empty());
    for s in sol.values("s") {
        assert_eq!(s.to_string(), a.node().to_string());
    }
    let in_state = vertep_runtime::vocab::IN_STATE;
    assert!(
        sol.values("p")
            .iter()
            .any(|p| p.to_string() == format!("<{in_state}>"))
    );
}

// @tg: verifies:: [[openspec:actor-model#IRI Actor Addresses#Relative or malformed address rejected]] {level: integration}
#[test]
fn relative_or_malformed_address_rejected() {
    assert!(Address::parse("orders/42").is_err());
    assert!(Address::parse("urn:bad target").is_err());
    let (mut node, _clock, store) = fresh();
    assert!(matches!(
        node.register_type("charts/relative", scxml("s", SINK)),
        Err(RegisterError::BadType(_))
    ));
    node.register_type(
        &ty("sender"),
        scxml(
            "s",
            r##"<state id="s"><transition event="go"><send event="x" target="urn:bad target"/></transition>
               <transition event="error.communication" target="failed"/></state><state id="failed"/>"##,
        ),
    )
    .unwrap();
    let a = va("sender", "s");
    let entries_before = store.dataset().len();
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    assert!(store.dataset().len() > entries_before);
    assert_eq!(
        store.actors().unwrap().len(),
        1,
        "nothing was enqueued for any other actor"
    );
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Identity#Same type and key give the same address]] {level: integration}
#[test]
fn same_type_and_key_give_the_same_address() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let x = Address::virtual_actor(&ty("sink"), "order 7").unwrap();
    let y = Address::virtual_actor(&ty("sink"), "order 7").unwrap();
    assert_eq!(x, y);
    assert_ne!(x, Address::virtual_actor(&ty("sink"), "order 8").unwrap());
    let h1 = node.handle();
    let h2 = node.handle();
    h1.send(&x, ev("one")).unwrap();
    h2.send(&y, ev("two")).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &x), vec!["one", "two"]);
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Identity#First message to a never-used key]] {level: integration}
#[test]
fn first_message_to_a_never_used_key() {
    let (mut node, _clock, store) = fresh();
    node.register_type(
        &ty("t"),
        scxml(
            "a",
            r##"<state id="a"><transition event="go" target="b"/></state><state id="b"/>"##,
        ),
    )
    .unwrap();
    let a = va("t", "never-used");
    assert_eq!(store.head(a.node()).unwrap(), Position::ZERO);
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["b"]);
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Activation#Activation on message]] {level: integration}
#[test]
fn activation_on_message() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut node = node_on(
        store,
        &clock,
        NodeConfig {
            snapshot_interval: 2,
            ..NodeConfig::default()
        },
    );
    node.register_type(
        &ty("t"),
        scxml("a", r##"<state id="a"><transition event="go" target="b"/></state><state id="b"><transition event="go" target="a"/></state>"##),
    )
    .unwrap();
    let a = va("t", "x");
    for _ in 0..3 {
        node.send(&a, ev("go")).unwrap();
    }
    node.run_until_idle();
    node.deactivate(&a);
    assert!(!node.is_active(&a));
    let replayed = node.stats().replayed;
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.stats().activations[&a], 2);
    assert_eq!(
        node.stats().replayed - replayed,
        1,
        "snapshot at 2, then entry 3"
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["a"]);
    assert_eq!(node.journal(&a).unwrap().len(), 4);
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Activation#Concurrent messages during activation]] {level: integration}
#[test]
fn concurrent_messages_during_activation() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "x");
    let h = node.handle();
    for n in ["m1", "m2", "m3", "m4"] {
        h.send(&a, ev(n)).unwrap();
    }
    node.run_until_idle();
    assert_eq!(node.stats().activations[&a], 1);
    assert_eq!(names(&node, &a), vec!["m1", "m2", "m3", "m4"]);
}

fn idle_node(idle_ms: i64) -> (vertep_runtime::Node, vertep_runtime::VirtualClock) {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let node = node_on(
        store,
        &clock,
        NodeConfig {
            idle_timeout_ms: idle_ms,
            ..NodeConfig::default()
        },
    );
    (node, clock)
}

const COUNTER: &str = r##"<datamodel><data id="n" expr="0"/></datamodel>
  <state id="s"><transition event="inc"><assign location="n" expr="n + 1"/></transition>
  <transition event="flip" target="t"/></state><state id="t"/>"##;

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Deactivation#Idle actor is deactivated]] {level: integration}
#[test]
fn idle_actor_is_deactivated() {
    let (mut node, clock) = idle_node(1_000);
    node.register_type(&ty("c"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("c", "x");
    node.send(&a, ev("inc")).unwrap();
    node.send(&a, ev("inc")).unwrap();
    node.run_until_idle();
    let before = node.state(&a).unwrap();
    clock.advance(Duration::from_millis(1_000));
    node.run_until_idle();
    assert!(!node.is_active(&a));
    node.send(&a, ev("inc")).unwrap();
    node.run_until_idle();
    assert!(node.is_active(&a));
    let after = node.state(&a).unwrap();
    assert_eq!(after.configuration, before.configuration);
    assert_eq!(data_of(&after.datamodel, "n"), Some(Value::Number(3.0)));
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actor Deactivation#No deactivation with pending work]] {level: integration}
#[tokio::test]
async fn no_deactivation_with_pending_work() {
    let (mut node, clock) = idle_node(1_000);
    node.register_type(&ty("c"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("c", "x");
    node.send(&a, ev("inc")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_millis(5_000));
    // a message is queued when the idle timeout has passed: it is processed, not dropped
    node.send(&a, ev("inc")).unwrap();
    node.pump();
    assert!(node.is_active(&a));
    assert_eq!(node.journal(&a).unwrap().len(), 2);

    // a running invocation (a turn of the child in progress) keeps the parent active
    let gate = Arc::new(tokio::sync::Notify::new());
    let g = gate.clone();
    node.register_actor_kind(
        "urn:vertep:kind:slow",
        FnActor::invokable(move |_| {
            let g = g.clone();
            async move {
                g.notified().await;
                Ok(Value::Null)
            }
        }),
    );
    node.register_type(
        &ty("p"),
        scxml(
            "w",
            r##"<state id="w"><invoke type="urn:vertep:kind:slow" id="s"/>
               <transition event="done.invoke.s" target="d"/></state><state id="d"/>"##,
        ),
    )
    .unwrap();
    let p = va("p", "y");
    node.send(&p, ev("go")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_millis(5_000));
    node.run_until_idle();
    assert!(
        node.is_active(&p),
        "a running turn of its child blocks deactivation"
    );
    gate.notify_one();
    node.settle().await;
    assert_eq!(node.state(&p).unwrap().configuration, vec!["d"]);
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actors Are Not Invokable#Invoke of a virtual actor rejected]] {level: integration}
#[test]
fn invoke_of_a_virtual_actor_rejected() {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let target = va("sink", "shared");
    // at validation: a static src naming a virtual actor
    let r = node.register_type(
        &ty("bad"),
        scxml(
            "s",
            &format!(r##"<state id="s"><invoke type="scxml" src="{target}" id="v"/></state>"##),
        ),
    );
    assert!(matches!(r, Err(RegisterError::InvokesVirtualActor { .. })));
    // at runtime: a computed src
    node.register_type(
        &ty("sneaky"),
        ecma_scxml(
            "s",
            &format!(
                r##"<state id="s"><invoke type="scxml" srcexpr="'{target}'" id="v"/>
                <transition event="error.execution" target="refused"/></state><state id="refused"/>"##
            ),
        ),
    )
    .unwrap();
    let a = va("sneaky", "x");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["refused"]);
    assert_eq!(
        store.head(target.node()).unwrap(),
        Position::ZERO,
        "neither started"
    );
    assert!(!node.is_active(&target));
}

// @tg: verifies:: [[openspec:actor-model#Virtual Actors Are Not Invokable#Cancelling a parent does not touch a virtual actor]] {level: integration}
#[test]
fn cancelling_a_parent_does_not_touch_a_virtual_actor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("c"), ecma_scxml("s", COUNTER))
        .unwrap();
    let shared = va("c", "shared");
    node.register_type(
        &ty("p"),
        scxml(
            "talk",
            &format!(
                r##"<state id="talk"><onentry><send event="inc" target="{shared}"/></onentry>
                <transition event="leave" target="gone"/></state><final id="gone"/>"##
            ),
        ),
    )
    .unwrap();
    let p = va("p", "x");
    node.send(&p, ev("hello")).unwrap();
    node.run_until_idle();
    let before = node.state(&shared).unwrap();
    node.send(&p, ev("leave")).unwrap();
    node.run_until_idle();
    assert!(node.state(&p).unwrap().terminated);
    assert!(node.is_active(&shared));
    assert_eq!(node.state(&shared).unwrap(), before);
}

// @tg: verifies:: [[openspec:actor-model#Invoked Actor Lifecycle#Started with the state]] {level: integration}
#[test]
fn started_with_the_state() {
    let (mut node, _clock, _store) = fresh();
    node.register_actor_kind("urn:vertep:kind:echo", echo_kind());
    node.register_type(
        &ty("p"),
        scxml(
            "idle",
            r##"<state id="idle"><transition event="go" target="searching"/></state>
               <state id="searching"><invoke type="urn:vertep:kind:echo" id="search"/></state>"##,
        ),
    )
    .unwrap();
    let p = va("p", "x");
    let child = Address::invoked(&p, "search");
    node.send(&p, ev("noop")).unwrap();
    node.run_until_idle();
    assert!(!node.is_active(&child));
    node.send(&p, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(child.as_str(), format!("{p}/invoke/search"));
    assert!(node.is_active(&child));
}

// @tg: verifies:: [[openspec:actor-model#Invoked Actor Lifecycle#Cancelled on state exit]] {level: integration}
#[tokio::test]
async fn cancelled_on_state_exit() {
    let (mut node, _clock, _store) = fresh();
    let gate = Arc::new(tokio::sync::Notify::new());
    let g = gate.clone();
    node.register_actor_kind(
        "urn:vertep:kind:slow",
        FnActor::invokable(move |_| {
            let g = g.clone();
            async move {
                g.notified().await;
                Ok(Value::String("late".into()))
            }
        }),
    );
    node.register_type(
        &ty("p"),
        scxml(
            "searching",
            r##"<state id="searching"><invoke type="urn:vertep:kind:slow" id="search"/>
               <transition event="leave" target="away"/>
               <transition event="done.invoke.search" target="found"/></state>
               <state id="away"><transition event="*"/></state><state id="found"/>"##,
        ),
    )
    .unwrap();
    let p = va("p", "x");
    let child = Address::invoked(&p, "search");
    node.send(&p, ev("go")).unwrap();
    node.run_until_idle();
    assert!(node.is_active(&child));
    node.send(&p, ev("leave")).unwrap();
    node.run_until_idle();
    assert!(!node.is_active(&child), "cancelled");
    gate.notify_one();
    node.settle().await;
    assert!(!names(&node, &p).iter().any(|n| n == "done.invoke.search"));
    assert_eq!(node.state(&p).unwrap().configuration, vec!["away"]);
}

// @tg: verifies:: [[openspec:actor-model#Invoked Actor Lifecycle#Completion reported to the parent]] {level: integration}
#[test]
fn completion_reported_to_the_parent() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("p"),
        ecma_scxml(
            "searching",
            r##"<state id="searching">
                 <invoke type="scxml" id="search"><content><scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="f"><final id="f"><donedata><param name="hits" expr="3"/></donedata></final></scxml></content></invoke>
                 <transition event="done.invoke.search" target="found"/></state>
               <state id="found"/>"##,
        ),
    )
    .unwrap();
    let p = va("p", "x");
    node.send(&p, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&p).unwrap().configuration, vec!["found"]);
    let j = node.journal(&p).unwrap();
    let EntryBody::Input { input, .. } = &j.last().unwrap().body else {
        panic!()
    };
    assert_eq!(input.event.name, "done.invoke.search");
    assert_eq!(field(&input.event.data, "hits"), Some(Value::Number(3.0)));
}

// @tg: verifies:: [[openspec:actor-model#Stateless Worker Lifecycle#Parallel activations]] {level: integration}
#[tokio::test]
async fn parallel_activations() {
    let (mut node, _clock, _store) = fresh();
    let w = Address::parse("urn:vertep:worker:square").unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (r, s) = (running.clone(), seen.clone());
    node.register_worker(
        w.clone(),
        4,
        FnActor::handler(move |e: Event| {
            let (r, s) = (r.clone(), s.clone());
            async move {
                r.fetch_add(1, Ordering::SeqCst);
                for _ in 0..5 {
                    tokio::task::yield_now().await;
                }
                s.lock().unwrap().push(e.name.clone());
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(None)
            }
        }),
    );
    for i in 0..10 {
        node.send(&w, ev(&format!("job{i}"))).unwrap();
    }
    node.settle().await;
    assert_eq!(node.stats().worker_concurrency[&w], 4);
    let mut got = seen.lock().unwrap().clone();
    got.sort();
    let mut want: Vec<String> = (0..10).map(|i| format!("job{i}")).collect();
    want.sort();
    assert_eq!(
        got, want,
        "each message processed by exactly one activation"
    );
}

// @tg: verifies:: [[openspec:actor-model#Stateless Worker Lifecycle#Worker keeps no journal]] {level: integration}
#[tokio::test]
async fn worker_keeps_no_journal() {
    let (mut node, _clock, store) = fresh();
    let w = Address::parse("urn:vertep:worker:double").unwrap();
    node.register_worker(
        w.clone(),
        2,
        FnActor::handler(|e: Event| async move {
            let n = match e.data {
                Value::Number(n) => n,
                _ => 0.0,
            };
            Ok(Some(Value::Number(n * 2.0)))
        }),
    );
    node.register_type(
        &ty("caller"),
        scxml(
            "s",
            &format!(
                r##"<state id="s"><vt:request target="{w}" event="double" reply="got"/></state><state id="got"/>"##
            ),
        ),
    )
    .unwrap();
    let a = va("caller", "x");
    node.send(&a, ev("start")).unwrap();
    node.settle().await;
    assert_eq!(node.state(&a).unwrap().configuration, vec!["got"]);
    assert_eq!(store.head(w.node()).unwrap(), Position::ZERO);
    assert!(
        !store.actors().unwrap().contains(w.node()),
        "no journal for the worker"
    );
    let j = node.journal(&a).unwrap();
    let EntryBody::Input { input, .. } = &j.last().unwrap().body else {
        panic!()
    };
    assert_eq!(
        input.event.name, "vt:reply",
        "the reply is journaled by the caller"
    );
}

const LOGGER: &str = r##"<datamodel><data id="log" expr="[]"/></datamodel>
  <state id="s">
    <transition event="e1"><script>log.push('e1')</script><raise event="r"/></transition>
    <transition event="r"><script>log.push('r')</script></transition>
    <transition event="e2"><script>log.push('e2')</script></transition>
  </state>"##;

// @tg: verifies:: [[openspec:actor-model#Turn Is One Macrostep#No interleaving within a turn]] {level: integration}
#[test]
fn no_interleaving_within_a_turn() {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("log"), ecma_scxml("s", LOGGER))
        .unwrap();
    let a = va("log", "x");
    node.send(&a, ev("e1")).unwrap();
    node.send(&a, ev("e2")).unwrap();
    // one pump runs exactly one turn for the actor: e1 is durable before e2 is taken
    node.pump();
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
    assert_eq!(node.mailbox(&a).len(), 1);
    node.run_until_idle();
    let log = data_of(&node.state(&a).unwrap().datamodel, "log.join(',')");
    assert_eq!(log, Some(Value::String("e1,r,e2".into())));
    assert_eq!(names(&node, &a), vec!["e1", "e2"]);
}

// @tg: verifies:: [[openspec:actor-model#Turn Is One Macrostep#Internal events stay within the turn]] {level: integration}
#[test]
fn internal_events_stay_within_the_turn() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("log"), ecma_scxml("s", LOGGER))
        .unwrap();
    let a = va("log", "x");
    node.send(&a, ev("e1")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    assert_eq!(j.len(), 1, "the raised event has no entry of its own");
    let t = &j[0].effects().unwrap().transitions;
    assert_eq!(
        t.len(),
        2,
        "the transitions of e1 and of the raised r are in one macrostep"
    );
    assert_eq!(
        data_of(&node.state(&a).unwrap().datamodel, "log.join(',')"),
        Some(Value::String("e1,r".into()))
    );
}

fn flaky(
    fail_times: usize,
    calls: Arc<Mutex<Vec<i64>>>,
    clock: vertep_runtime::VirtualClock,
) -> ActorFactory {
    let n = Arc::new(AtomicUsize::new(0));
    FnActor::invokable(move |_| {
        let (n, calls, clock) = (n.clone(), calls.clone(), clock.clone());
        async move {
            use vertep_runtime::Clock;
            calls.lock().unwrap().push(clock.now_ms());
            if n.fetch_add(1, Ordering::SeqCst) < fail_times {
                Err("boom".to_owned())
            } else {
                Ok(Value::String("ok".into()))
            }
        }
    })
}

fn supervised(restart: &str) -> vertep_core::ir::Chart {
    scxml(
        "w",
        &format!(
            r##"<state id="w"><invoke type="urn:vertep:kind:flaky" id="job" {restart}/>
               <transition event="done.invoke.job" target="ok"/>
               <transition event="error.execution" target="failed"/></state>
               <state id="ok"/><state id="failed"/>"##
        ),
    )
}

// @tg: verifies:: [[openspec:actor-model#Supervision With Restart#Restart with backoff]] {level: integration}
#[tokio::test]
async fn restart_with_backoff() {
    let (mut node, clock, _store) = fresh();
    let calls = Arc::new(Mutex::new(Vec::new()));
    node.register_actor_kind(
        "urn:vertep:kind:flaky",
        flaky(2, calls.clone(), clock.clone()),
    );
    node.register_type(
        &ty("sup"),
        supervised(r##"vt:restart="on-failure" vt:restart-limit="3" vt:restart-backoff="100ms""##),
    )
    .unwrap();
    let a = va("sup", "x");
    node.send(&a, ev("go")).unwrap();
    node.settle().await;
    clock.advance(Duration::from_millis(100));
    node.settle().await;
    clock.advance(Duration::from_millis(200));
    node.settle().await;
    assert_eq!(*calls.lock().unwrap(), vec![T0, T0 + 100, T0 + 300]);
    assert_eq!(node.state(&a).unwrap().configuration, vec!["ok"]);
    assert!(!names(&node, &a).iter().any(|n| n == "error.execution"));
}

// @tg: verifies:: [[openspec:actor-model#Supervision With Restart#Restart limit exceeded]] {level: integration}
#[tokio::test]
async fn restart_limit_exceeded() {
    let (mut node, clock, _store) = fresh();
    let calls = Arc::new(Mutex::new(Vec::new()));
    node.register_actor_kind(
        "urn:vertep:kind:flaky",
        flaky(usize::MAX, calls.clone(), clock.clone()),
    );
    node.register_type(
        &ty("sup"),
        supervised(r##"vt:restart="on-failure" vt:restart-limit="3" vt:restart-backoff="100ms""##),
    )
    .unwrap();
    let a = va("sup", "x");
    node.send(&a, ev("go")).unwrap();
    node.settle().await;
    for step in [100, 200, 400] {
        clock.advance(Duration::from_millis(step));
        node.settle().await;
    }
    assert_eq!(
        calls.lock().unwrap().len(),
        4,
        "three restarts, then the fourth failure gives up"
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    let j = node.journal(&a).unwrap();
    let EntryBody::Input { input, .. } = &j.last().unwrap().body else {
        panic!()
    };
    assert_eq!(input.event.name, "error.execution");
    assert_eq!(
        field(&input.event.data, "invokeid"),
        Some(Value::String("job".into()))
    );
    assert_eq!(
        field(&input.event.data, "cause"),
        Some(Value::String("boom".into()))
    );
}

// @tg: verifies:: [[openspec:actor-model#Supervision With Restart#No policy means SCXML default]] {level: integration}
#[tokio::test]
async fn no_policy_means_scxml_default() {
    let (mut node, clock, _store) = fresh();
    let calls = Arc::new(Mutex::new(Vec::new()));
    node.register_actor_kind(
        "urn:vertep:kind:flaky",
        flaky(1, calls.clone(), clock.clone()),
    );
    node.register_type(&ty("sup"), supervised("")).unwrap();
    let a = va("sup", "x");
    node.send(&a, ev("go")).unwrap();
    node.settle().await;
    clock.advance(Duration::from_secs(10));
    node.settle().await;
    assert_eq!(calls.lock().unwrap().len(), 1, "not restarted");
    assert!(node.stats().restarts.is_empty());
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
}

/// A datamodel that panics when it runs the script `panic`.
struct Panicky(NullDatamodel);

impl Datamodel for Panicky {
    fn name(&self) -> &str {
        "panicky"
    }
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.0.init_session(info)
    }
    fn set_environment(&mut self, env: &Environment) {
        self.0.set_environment(env)
    }
    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        self.0.declare(id)
    }
    fn set_event(&mut self, event: &Event) {
        self.0.set_event(event)
    }
    fn eval_cond(&mut self, e: &str, s: &dyn StateQuery) -> Result<bool, EvalError> {
        self.0.eval_cond(e, s)
    }
    fn eval_value(&mut self, e: &str, s: &dyn StateQuery) -> Result<Value, EvalError> {
        self.0.eval_value(e, s)
    }
    fn eval_content(&mut self, c: &str) -> Result<Value, EvalError> {
        self.0.eval_content(c)
    }
    fn assign_expr(&mut self, l: &str, e: &str, s: &dyn StateQuery) -> Result<(), EvalError> {
        self.0.assign_expr(l, e, s)
    }
    fn assign_value(&mut self, l: &str, v: Value) -> Result<(), EvalError> {
        self.0.assign_value(l, v)
    }
    fn bind_var(&mut self, n: &str, v: Value) -> Result<(), EvalError> {
        self.0.bind_var(n, v)
    }
    fn is_location(&mut self, l: &str) -> bool {
        self.0.is_location(l)
    }
    fn exec_script(&mut self, script: &str, _s: &dyn StateQuery) -> Result<(), EvalError> {
        if script.trim() == "panic" {
            panic!("an action implementation panicked");
        }
        Ok(())
    }
    fn foreach_items(&mut self, a: &str, s: &dyn StateQuery) -> Result<Vec<Value>, EvalError> {
        self.0.foreach_items(a, s)
    }
    fn snapshot(&self) -> Value {
        self.0.snapshot()
    }
    fn restore(&mut self, v: &Value) -> Result<(), EvalError> {
        self.0.restore(v)
    }
}

struct PanickyRegistry;

impl DatamodelRegistry for PanickyRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        (name == "panicky").then(|| Box::new(Panicky(NullDatamodel::new())) as Box<dyn Datamodel>)
    }
}

// @tg: verifies:: [[openspec:actor-model#Actor Failure Isolation#Panic during a turn]] {level: integration}
#[test]
fn panic_during_a_turn() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let shell = vertep_runtime::Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    let mut node = vertep_runtime::Node::builder(store.clone(), shell)
        .datamodels(Arc::new(PanickyRegistry))
        .build();
    node.register_type(
        &ty("fragile"),
        common::chart(
            r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="panicky" initial="s">
               <state id="s"><transition event="boom" target="t"><script>panic</script></transition></state>
               <state id="t"/></scxml>"##,
        ),
    )
    .unwrap();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("fragile", "x");
    let b = va("sink", "y");
    node.send(&a, ev("boom")).unwrap();
    node.send(&b, ev("hello")).unwrap();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    node.run_until_idle();
    std::panic::set_hook(prev);
    assert_eq!(
        store.head(a.node()).unwrap(),
        Position::ZERO,
        "nothing appended"
    );
    assert!(!node.is_active(&a), "deactivated");
    assert_eq!(
        node.mailbox(&a).len(),
        1,
        "the message stays for redelivery"
    );
    assert_eq!(names(&node, &b), vec!["hello"], "other actors keep running");
    let _ = START_EVENT;
}

#[tokio::test]
async fn a_stream_is_an_invokable_actor() {
    use futures::stream;
    let (mut node, _clock, _store) = fresh();
    node.register_actor_kind(
        "urn:vertep:kind:ticks",
        vertep_runtime::StreamActor::invokable("tick", |_params| {
            Box::pin(stream::iter(vec![
                Ok(Value::Number(1.0)),
                Ok(Value::Number(2.0)),
                Ok(Value::Number(3.0)),
            ]))
        }),
    );
    node.register_type(
        &ty("listener"),
        scxml(
            "on",
            r##"<state id="on"><invoke type="urn:vertep:kind:ticks" id="t"/>
               <transition event="tick"/><transition event="done.invoke.t" target="off"/></state><state id="off"/>"##,
        ),
    )
    .unwrap();
    let a = va("listener", "x");
    node.send(&a, ev("go")).unwrap();
    node.settle().await;
    assert_eq!(
        names(&node, &a),
        vec!["go", "tick", "tick", "tick", "done.invoke.t"]
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["off"]);
}
