//! Scenarios of the `messaging` capability.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_runtime::journal::EntryBody;
use vertep_runtime::mailbox::{Mailbox, MemoryMailbox};
use vertep_runtime::{
    Address, AskError, Envelope, Node, NodeConfig, Resolver, Shell, StoreMailboxes, VirtualClock,
};
use vertep_store::{GraphStore, JournalStore, MemoryStore, Position};

const SINK: &str = r##"<state id="s"><transition event="*"/></state>"##;

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

fn last_input(node: &Node, a: &Address) -> Event {
    let j = node.journal(a).unwrap();
    match &j.last().unwrap().body {
        EntryBody::Input { input, .. } => input.event.clone(),
        _ => panic!("input entry"),
    }
}

fn shell(clock: &VirtualClock) -> Shell {
    Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    }
}

// @tg: verifies:: [[openspec:messaging#Pluggable Mailboxes#Swapping the mailbox implementation]] {level: integration}
#[test]
fn swapping_the_mailbox_implementation() {
    let run = |store_backed: bool| {
        let store = Arc::new(MemoryStore::new());
        let clock = VirtualClock::new(T0);
        let mut b = Node::builder(store.clone(), shell(&clock));
        if store_backed {
            b = b.mailboxes_for_scheme("https", Arc::new(StoreMailboxes::new(store.clone())));
        }
        let mut node = b.build();
        node.register_type(
            &ty("t"),
            scxml(
                "a",
                r##"<state id="a"><transition event="x" target="b"/></state>
                   <state id="b"><transition event="y" target="a"/></state>"##,
            ),
        )
        .unwrap();
        let a = va("t", "k");
        for n in ["x", "y", "x", "z"] {
            node.send(&a, ev(n)).unwrap();
        }
        node.run_until_idle();
        (names(&node, &a), node.state(&a).unwrap().configuration)
    };
    let mem = run(false);
    let stored = run(true);
    assert_eq!(mem, stored);
    assert_eq!(mem.0, vec!["x", "y", "x", "z"]);
}

static FAIL_ONCE: AtomicBool = AtomicBool::new(true);

// @tg: verifies:: [[openspec:messaging#Pluggable Mailboxes#Unacknowledged message is redelivered]] {level: integration}
#[test]
fn unacknowledged_message_is_redelivered() {
    use vertep_core::datamodel::{Datamodel, DatamodelRegistry};
    struct FailOnce;
    impl DatamodelRegistry for FailOnce {
        fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
            if name == "null" && FAIL_ONCE.swap(false, Ordering::SeqCst) {
                panic!("the first activation fails");
            }
            vertep_core::null::NullRegistry.create(name)
        }
    }
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = Node::builder(store.clone(), shell(&clock))
        .datamodels(Arc::new(FailOnce))
        .config(NodeConfig {
            failure_backoff_ms: 500,
            ..NodeConfig::default()
        })
        .build();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "k");
    node.send(&a, ev("hello")).unwrap();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    node.run_until_idle();
    std::panic::set_hook(prev);
    assert_eq!(store.head(a.node()).unwrap(), Position::ZERO);
    assert_eq!(node.mailbox(&a).len(), 1, "not acknowledged");
    clock.advance(Duration::from_millis(500));
    node.run_until_idle();
    assert_eq!(
        names(&node, &a),
        vec!["hello"],
        "delivered again on the next activation"
    );
    assert!(node.mailbox(&a).is_empty());
}

// @tg: verifies:: [[openspec:messaging#Mailbox Ordering#Per-sender order]] {level: integration}
#[test]
fn per_sender_order() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    node.register_type(
        &ty("a"),
        scxml(
            "s",
            &format!(
                r##"<state id="s"><transition event="go"><send event="e1" target="{b}"/><send event="e2" target="{b}"/></transition>
                <transition event="more"><send event="e3" target="{b}"/></transition></state>"##
            ),
        ),
    )
    .unwrap();
    let a = va("a", "a");
    node.send(&a, ev("go")).unwrap();
    node.send(&a, ev("more")).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &b), vec!["e1", "e2", "e3"]);
}

// @tg: verifies:: [[openspec:messaging#Mailboxes Outlive Activations#Message to a deactivated actor]] {level: integration}
#[test]
fn message_to_a_deactivated_actor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "k");
    node.send(&a, ev("one")).unwrap();
    node.run_until_idle();
    node.deactivate(&a);
    node.send(&a, ev("two")).unwrap();
    assert!(!node.is_active(&a));
    assert_eq!(node.mailbox(&a).len(), 1, "stored while inactive");
    node.run_until_idle();
    assert!(node.is_active(&a), "its arrival activated the actor");
    assert_eq!(names(&node, &a), vec!["one", "two"]);
}

// @tg: verifies:: [[openspec:messaging#Mailboxes Outlive Activations#Durable mailbox survives a restart]] {level: integration}
#[test]
fn durable_mailbox_survives_a_restart() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let build = || {
        let mut n = Node::builder(store.clone(), shell(&clock))
            .mailboxes(Arc::new(StoreMailboxes::new(store.clone())))
            .build();
        n.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
        n
    };
    let n1 = build();
    let a = va("sink", "k");
    for e in ["e1", "e2", "e3"] {
        n1.send(&a, ev(e)).unwrap();
    }
    drop(n1);
    let mut n2 = build();
    n2.run_until_idle();
    assert_eq!(names(&n2, &a), vec!["e1", "e2", "e3"]);
    assert!(n2.mailbox(&a).is_empty());
}

// @tg: verifies:: [[openspec:messaging#Pluggable Transports#Local transport as Event I/O Processor]] {level: integration}
#[test]
fn local_transport_as_event_io_processor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    node.register_type(
        &ty("io"),
        ecma_scxml(
            "s",
            &format!(
                r##"<datamodel><data id="loc" expr="''"/></datamodel>
                <state id="s"><transition event="go">
                  <assign location="loc" expr="_ioprocessors['http://www.w3.org/TR/scxml/#SCXMLEventProcessor'].location"/>
                  <send event="hi" target="{b}"/>
                  <send event="hi2" target="{b}" type="http://www.w3.org/TR/scxml/#SCXMLEventProcessor"/>
                </transition></state>"##
            ),
        ),
    )
    .unwrap();
    let a = va("io", "a");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(
        data_of(&node.state(&a).unwrap().datamodel, "loc"),
        Some(Value::String(a.as_str().into()))
    );
    assert_eq!(names(&node, &b), vec!["hi", "hi2"]);
    assert_eq!(
        last_input(&node, &b).origintype.as_deref(),
        Some("http://www.w3.org/TR/scxml/#SCXMLEventProcessor")
    );
}

// @tg: verifies:: [[openspec:messaging#Pluggable Transports#Unsupported send type]] {level: integration}
#[test]
fn unsupported_send_type() {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    node.register_type(
        &ty("a"),
        scxml(
            "s",
            &format!(
                r##"<state id="s"><transition event="go"><send event="x" target="{b}" type="urn:vertep:no-such-transport"/></transition>
                <transition event="error.execution" target="failed"/></state><state id="failed"/>"##
            ),
        ),
    )
    .unwrap();
    let a = va("a", "a");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    assert_eq!(
        store.head(b.node()).unwrap(),
        Position::ZERO,
        "nothing delivered"
    );
}

// @tg: verifies:: [[openspec:messaging#Address Resolution#Resolve a local actor]] {level: integration}
#[test]
fn resolve_a_local_actor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    node.send(&b, ev("hello")).unwrap();
    let queued = node.mailbox(&b);
    assert_eq!(queued.len(), 1, "enqueued in the local mailbox");
    assert_eq!(queued[0].to, b.as_str());
    node.run_until_idle();
    assert_eq!(names(&node, &b), vec!["hello"]);
}

// @tg: verifies:: [[openspec:messaging#Address Resolution#Unresolvable address]] {level: integration}
#[test]
fn unresolvable_address() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("a"),
        ecma_scxml(
            "s",
            r##"<datamodel><data id="failed" expr="''"/></datamodel>
            <state id="s"><transition event="go"><send id="s1" event="x" target="urn:nowhere:z"/></transition>
            <transition event="error.communication"><assign location="failed" expr="_event.sendid"/></transition></state>"##,
        ),
    )
    .unwrap();
    let a = va("a", "a");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(
        data_of(&node.state(&a).unwrap().datamodel, "failed"),
        Some(Value::String("s1".into()))
    );
    let z = Address::parse("urn:nowhere:z").unwrap();
    let dl = node.dead_letters(&z).unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0].reason, "unresolvable");
    assert_eq!(dl[0].sender.as_deref(), Some(a.as_str()));
}

struct Fixed(Arc<MemoryMailbox>);

impl Resolver for Fixed {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        (address.as_str() == "test:inbox").then(|| self.0.clone() as Arc<dyn Mailbox>)
    }
}

// @tg: verifies:: [[openspec:messaging#Address Resolution#Pluggable resolvers]] {level: integration}
#[test]
fn pluggable_resolvers() {
    let (mut node, _clock, _store) = fresh();
    let inbox = Arc::new(MemoryMailbox::default());
    node.handle()
        .register_resolver("test", Arc::new(Fixed(inbox.clone())));
    node.register_type(
        &ty("a"),
        scxml(
            "s",
            r##"<state id="s"><transition event="go"><send event="out" target="test:inbox"/></transition></state>"##,
        ),
    )
    .unwrap();
    let a = va("a", "a");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let got = inbox.pending();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].event.name, "out");
    assert_eq!(got[0].event.origin.as_deref(), Some(a.as_str()));
}

const PONG: &str = r##"<state id="s"><transition event="ping"><send event="pong" targetexpr="_event.origin"/></transition></state>"##;

// @tg: verifies:: [[openspec:messaging#Event Origin#Reply to origin]] {level: integration}
#[test]
fn reply_to_origin() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("b"), ecma_scxml("s", PONG)).unwrap();
    let b = va("b", "b");
    node.register_type(
        &ty("a"),
        scxml(
            "s",
            &format!(
                r##"<state id="s"><transition event="go"><send event="ping" target="{b}"/></transition>
                <transition event="pong" target="done"/></state><state id="done"/>"##
            ),
        ),
    )
    .unwrap();
    let a = va("a", "a");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["done"]);
    let pong = last_input(&node, &a);
    assert_eq!(pong.origin.as_deref(), Some(b.as_str()));
    let ping = last_input(&node, &b);
    assert_eq!(ping.origin.as_deref(), Some(a.as_str()));
}

const PRICER: &str = r##"<state id="s"><transition event="price.get">
    <send event="vt:reply" targetexpr="_event.origin"><content expr="({price: 42})"/></send></transition></state>"##;

fn requester(target: &Address, timeout: &str) -> vertep_core::ir::Chart {
    scxml(
        "idle",
        &format!(
            r##"<state id="idle"><transition event="quote" target="quoting"/></state>
            <state id="quoting"><vt:request target="{target}" event="price.get" timeout="{timeout}" reply="priced" ontimeout="failed"/>
              <transition event="ping"/></state>
            <state id="priced"/><state id="failed"/>"##
        ),
    )
}

// @tg: verifies:: [[openspec:messaging#Non-Blocking Request#Request and reply]] {level: integration}
#[test]
fn request_and_reply() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("pricer"), ecma_scxml("s", PRICER))
        .unwrap();
    let b = va("pricer", "b");
    node.register_type(&ty("buyer"), requester(&b, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["priced"]);
    let reply = last_input(&node, &a);
    assert_eq!(reply.name, "vt:reply");
    assert_eq!(field(&reply.data, "price"), Some(Value::Number(42.0)));
    let j = node.journal(&a).unwrap();
    assert_eq!(
        j[0].effects().unwrap().requests,
        vec![a.correlation_id(1, 0)]
    );
    assert_eq!(
        j[1].effects().unwrap().answers,
        Some(a.correlation_id(1, 0))
    );
}

// @tg: verifies:: [[openspec:messaging#Non-Blocking Request#Other events processed while waiting]] {level: integration}
#[test]
fn other_events_processed_while_waiting() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let silent = va("sink", "silent");
    node.register_type(&ty("buyer"), requester(&silent, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["quoting", "quoting.vt-waiting"]
    );
    node.send(&a, ev("ping")).unwrap();
    node.run_until_idle();
    assert_eq!(
        names(&node, &a),
        vec!["quote", "ping"],
        "a normal turn while waiting"
    );
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["quoting", "quoting.vt-waiting"]
    );
}

// @tg: verifies:: [[openspec:messaging#Non-Blocking Request#Mutual requests do not deadlock]] {level: integration}
#[test]
fn mutual_requests_do_not_deadlock() {
    let (mut node, _clock, _store) = fresh();
    let a = va("peer", "a");
    let b = va("peer", "b");
    let peer = |other: &Address| {
        ecma_scxml(
            "idle",
            &format!(
                r##"<state id="root" initial="idle">
                  <transition event="price.get"><send event="vt:reply" targetexpr="_event.origin"><content expr="1"/></send></transition>
                  <state id="idle"><transition event="quote" target="quoting"/></state>
                  <state id="quoting"><vt:request target="{other}" event="price.get" timeout="5s" reply="priced" ontimeout="failed"/></state>
                  <state id="priced"/><state id="failed"/></state>"##
            ),
        )
    };
    // two versions of one type, one per direction; each actor is pinned to the one it started on
    node.register_type(&ty("peer"), peer(&b)).unwrap();
    node.send(&a, ev("quote")).unwrap();
    node.activate_actor(&a).unwrap();
    node.register_type(&ty("peer"), peer(&a)).unwrap();
    node.send(&b, ev("quote")).unwrap();
    node.run_until_idle();
    assert!(
        node.state(&a)
            .unwrap()
            .configuration
            .contains(&"priced".to_owned())
    );
    assert!(
        node.state(&b)
            .unwrap()
            .configuration
            .contains(&"priced".to_owned())
    );
}

// @tg: verifies:: [[openspec:messaging#Deterministic Correlation Ids#Replay reproduces correlation ids]] {level: integration}
#[test]
fn replay_reproduces_correlation_ids() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "silent");
    node.register_type(&ty("buyer"), requester(&b, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    let corr = node.journal(&a).unwrap()[0].effects().unwrap().requests[0].clone();
    assert_eq!(corr, a.correlation_id(1, 0));
    // reactivate by replay, then answer with the recorded id: it still matches
    node.deactivate(&a);
    node.activate_actor(&a).unwrap();
    let mut env = Envelope::new("urn:vertep:test:reply", &a, ev("vt:reply"));
    env.correlation = Some(corr.clone());
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["priced"]);
    assert_eq!(
        node.journal(&a).unwrap()[1].effects().unwrap().answers,
        Some(corr)
    );
}

// @tg: verifies:: [[openspec:messaging#Deterministic Correlation Ids#Ids are unique per actor]] {level: integration}
#[test]
fn ids_are_unique_per_actor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "s");
    node.register_type(
        &ty("multi"),
        scxml(
            "s",
            &format!(
                r##"<state id="s"><transition event="go" target="two"/></state>
                <state id="two"><vt:request target="{b}" event="q1"/><vt:request target="{b}" event="q2"/>
                  <transition event="again" target="one"/></state>
                <state id="one"><vt:request target="{b}" event="q3"/></state>"##
            ),
        ),
    )
    .unwrap();
    let a = va("multi", "a");
    node.send(&a, ev("go")).unwrap();
    node.send(&a, ev("again")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    let mut all: Vec<String> = j
        .iter()
        .flat_map(|e| e.effects().unwrap().requests.clone())
        .collect();
    assert_eq!(all.len(), 3);
    all.sort();
    all.dedup();
    assert_eq!(
        all.len(),
        3,
        "two in one macrostep and one in another, all distinct"
    );
}

// @tg: verifies:: [[openspec:messaging#Request Timeout#Timeout fires]] {level: integration}
#[test]
fn timeout_fires() {
    let (mut node, clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "silent");
    node.register_type(&ty("buyer"), requester(&b, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_millis(4_999));
    node.run_until_idle();
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["quoting", "quoting.vt-waiting"]
    );
    clock.advance(Duration::from_millis(1));
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    let t = last_input(&node, &a);
    assert_eq!(t.name, "error.timeout");
    assert_eq!(
        field(&t.data, "correlation"),
        Some(Value::String(a.correlation_id(1, 0)))
    );
}

// @tg: verifies:: [[openspec:messaging#Request Timeout#Late reply after timeout]] {level: integration}
#[test]
fn late_reply_after_timeout() {
    let (mut node, clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "silent");
    node.register_type(&ty("buyer"), requester(&b, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_secs(5));
    node.run_until_idle();
    let n = node.journal(&a).unwrap().len();
    let mut env = Envelope::new("urn:vertep:test:late", &a, ev("vt:reply"));
    env.correlation = Some(a.correlation_id(1, 0));
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(
        node.journal(&a).unwrap().len(),
        n,
        "not matched to a waiting state"
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["failed"]);
    let dl = node.dead_letters(&a).unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(
        (dl[0].event.as_str(), dl[0].reason.as_str()),
        ("vt:reply", "late-reply")
    );
}

// @tg: verifies:: [[openspec:messaging#Duplicate Reply Suppression#Redelivered reply dropped]] {level: integration}
#[test]
fn redelivered_reply_dropped() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("pricer"), ecma_scxml("s", PRICER))
        .unwrap();
    let b = va("pricer", "b");
    node.register_type(&ty("buyer"), requester(&b, "5s"))
        .unwrap();
    let a = va("buyer", "a");
    node.send(&a, ev("quote")).unwrap();
    node.run_until_idle();
    assert_eq!(node.journal(&a).unwrap().len(), 2);
    // a relay delivers the same reply again (under another message id)
    let mut env = Envelope::new("urn:vertep:test:relayed-copy", &a, ev("vt:reply"));
    env.correlation = Some(a.correlation_id(1, 0));
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(
        node.journal(&a).unwrap().len(),
        2,
        "no turn for the second copy"
    );
    assert!(node.mailbox(&a).is_empty(), "acknowledged");
    assert!(node.dead_letters(&a).unwrap().is_empty());
}

const DEFER: &str = r##"<state id="busy" vt:defer="order.*">
    <transition event="finish" target="idle"/></state>
  <state id="idle"><transition event="*"/></state>"##;

// @tg: verifies:: [[openspec:messaging#Deferred Events#Event deferred while busy]] {level: integration}
#[test]
fn event_deferred_while_busy() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("d"), scxml("busy", DEFER)).unwrap();
    let a = va("d", "a");
    node.send(&a, ev("hello")).unwrap();
    node.run_until_idle();
    node.send(&a, ev("order.cancel")).unwrap();
    node.run_until_idle();
    assert_eq!(names(&node, &a), vec!["hello"], "no transition ran for it");
    assert_eq!(node.mailbox(&a).len(), 1, "it remains queued");
    assert_eq!(node.mailbox(&a)[0].event.name, "order.cancel");
}

// @tg: verifies:: [[openspec:messaging#Deferred Events#Redelivery after leaving the deferring state]] {level: integration}
#[test]
fn redelivery_after_leaving_the_deferring_state() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("d"), scxml("busy", DEFER)).unwrap();
    let a = va("d", "a");
    node.send(&a, ev("hello")).unwrap();
    node.run_until_idle();
    for e in ["order.cancel", "order.update", "finish", "ping"] {
        node.send(&a, ev(e)).unwrap();
    }
    node.run_until_idle();
    assert_eq!(
        names(&node, &a),
        vec!["hello", "finish", "order.cancel", "order.update", "ping"]
    );
}

// @tg: verifies:: [[openspec:messaging#Deferred Events#Deferral survives deactivation]] {level: integration}
#[test]
fn deferral_survives_deactivation() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = node_on(
        store,
        &clock,
        NodeConfig {
            idle_timeout_ms: 1_000,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("d"), scxml("busy", DEFER)).unwrap();
    let a = va("d", "a");
    node.send(&a, ev("hello")).unwrap();
    node.send(&a, ev("order.a")).unwrap();
    node.send(&a, ev("order.b")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_secs(2));
    node.run_until_idle();
    assert!(
        !node.is_active(&a),
        "deactivated with deferred events queued"
    );
    let queued: Vec<String> = node
        .mailbox(&a)
        .iter()
        .map(|e| e.event.name.clone())
        .collect();
    assert_eq!(queued, vec!["order.a", "order.b"]);
    node.send(&a, ev("finish")).unwrap();
    node.run_until_idle();
    assert_eq!(
        names(&node, &a),
        vec!["hello", "finish", "order.a", "order.b"]
    );
}

// @tg: verifies:: [[openspec:messaging#Unhandled Event Policy#Discard keeps SCXML conformance]] {level: integration}
#[test]
fn discard_keeps_scxml_conformance() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("plain"),
        chart(
            r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" initial="a" vt:unhandled="discard">
               <state id="a"><transition event="go" target="b"/></state><state id="b"/></scxml>"##,
        ),
    )
    .unwrap();
    let a = va("plain", "a");
    node.send(&a, ev("unknown")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["a"]);
    assert!(
        node.dead_letters(&a).unwrap().is_empty(),
        "dropped, nothing recorded"
    );
    let j = node.journal(&a).unwrap();
    assert_eq!(
        j[0].effects().unwrap().unhandled,
        Some(vertep_runtime::UnhandledPolicy::Discard)
    );
}

// @tg: verifies:: [[openspec:messaging#Unhandled Event Policy#Dead letter for a virtual actor]] {level: integration}
#[test]
fn dead_letter_for_a_virtual_actor() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("t"),
        scxml(
            "a",
            r##"<state id="a"><transition event="go" target="b"/></state><state id="b"/>"##,
        ),
    )
    .unwrap();
    let a = va("t", "a");
    node.send(
        &a,
        Event {
            name: "unknown".into(),
            data: obj(&[("x", Value::Number(1.0))]),
            ..Event::default()
        },
    )
    .unwrap();
    node.run_until_idle();
    assert_eq!(
        node.state(&a).unwrap().configuration,
        vec!["a"],
        "configuration unchanged"
    );
    let dl = node.dead_letters(&a).unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0].event, "unknown");
    assert_eq!(dl[0].reason, "unhandled");
    assert_eq!(dl[0].payload, r#"{"x":1.0}"#);
}

// @tg: verifies:: [[openspec:messaging#Unhandled Event Policy#Error policy]] {level: integration}
#[test]
fn error_policy() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("strict"),
        chart(
            r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="ecmascript" initial="a" vt:unhandled="error">
               <datamodel><data id="what" expr="''"/></datamodel>
               <state id="a"><transition event="error.unhandled" target="oops"><assign location="what" expr="_event.data.name"/></transition></state>
               <state id="oops"/></scxml>"##,
        ),
    )
    .unwrap();
    let a = va("strict", "a");
    node.send(&a, ev("mystery")).unwrap();
    node.run_until_idle();
    let s = node.state(&a).unwrap();
    assert_eq!(s.configuration, vec!["oops"]);
    assert_eq!(
        data_of(&s.datamodel, "what"),
        Some(Value::String("mystery".into()))
    );
    assert!(node.dead_letters(&a).unwrap().is_empty());
}

// @tg: verifies:: [[openspec:messaging#Queryable Dead Letters#Query what an actor ignored]] {level: integration}
#[test]
fn query_what_an_actor_ignored() {
    let (mut node, _clock, store) = fresh();
    node.register_type(
        &ty("t"),
        scxml(
            "a",
            r##"<state id="a"><transition event="go"><send event="lost" target="urn:nowhere:gone"/></transition></state>"##,
        ),
    )
    .unwrap();
    let a = va("t", "a");
    let mut e = ev("ignored.one");
    e.origin = Some("urn:vertep:test:operator".into());
    node.send(&a, e).unwrap();
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let query = |receiver: &str| {
        let q = format!(
            "PREFIX vt: <https://w3id.org/vertep/vt#>
             SELECT ?name ?sender ?reason WHERE {{ GRAPH <{receiver}/dead-letters> {{
               ?d a vt:DeadLetter ; vt:receiver <{receiver}> ; vt:eventName ?name ; vt:reason ?reason .
               OPTIONAL {{ ?d vt:sender ?sender }} }} }} ORDER BY ?name"
        );
        store.query(&q).unwrap().into_solutions().unwrap()
    };
    // the ignored event, and the error.communication about the lost send, which `a` also
    // does not handle
    let mine = query(a.as_str());
    assert_eq!(mine.len(), 2);
    assert_eq!(
        mine.get(0, "name").unwrap().to_string(),
        "\"error.communication\""
    );
    assert_eq!(mine.get(1, "name").unwrap().to_string(), "\"ignored.one\"");
    assert_eq!(mine.get(1, "reason").unwrap().to_string(), "\"unhandled\"");
    assert_eq!(
        mine.get(1, "sender").unwrap().to_string(),
        "\"urn:vertep:test:operator\""
    );
    let gone = query("urn:nowhere:gone");
    assert_eq!(gone.len(), 1);
    assert_eq!(
        gone.get(0, "reason").unwrap().to_string(),
        "\"unresolvable\""
    );
    assert_eq!(
        gone.get(0, "sender").unwrap().to_string(),
        format!("\"{a}\"")
    );
}

const SLOW_PRICER: &str = r##"<state id="s"><transition event="price.get">
    <send event="vt:reply" targetexpr="_event.origin" delay="100ms"><content expr="({price: 7})"/></send></transition></state>"##;

// @tg: verifies:: [[openspec:messaging#External Blocking Ask#Ask from outside]] {level: integration}
#[tokio::test]
async fn ask_from_outside() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (mut node, clock, _store) = fresh();
            node.register_type(&ty("pricer"), ecma_scxml("s", SLOW_PRICER))
                .unwrap();
            let b = va("pricer", "b");
            let h = node.handle();
            let stop = node.handle();
            let runner = tokio::task::spawn_local(async move {
                node.run().await;
                node
            });
            let ask = tokio::task::spawn_local({
                let h = h.clone();
                let b = b.clone();
                async move { h.ask(&b, ev("price.get"), Duration::from_secs(2)).await }
            });
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            clock.advance(Duration::from_millis(100));
            let reply = ask.await.unwrap().unwrap();
            assert_eq!(reply.name, "vt:reply");
            assert_eq!(field(&reply.data, "price"), Some(Value::Number(7.0)));
            stop.shutdown();
            let node = runner.await.unwrap();
            assert_eq!(names(&node, &b), vec!["price.get"]);
        })
        .await;
}

// @tg: verifies:: [[openspec:messaging#External Blocking Ask#Ask times out]] {level: integration}
#[tokio::test]
async fn ask_times_out() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (mut node, clock, _store) = fresh();
            node.register_type(&ty("pricer"), ecma_scxml("s", SLOW_PRICER))
                .unwrap();
            let b = va("pricer", "b");
            let h = node.handle();
            let runner = tokio::task::spawn_local(async move {
                node.run().await;
                node
            });
            let ask = tokio::task::spawn_local({
                let (h, b) = (h.clone(), b.clone());
                async move { h.ask(&b, ev("price.get"), Duration::from_millis(50)).await }
            });
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            clock.advance(Duration::from_millis(50));
            assert_eq!(ask.await.unwrap(), Err(AskError::Timeout));
            // the reply comes after the timeout and is dropped
            clock.advance(Duration::from_millis(50));
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            h.shutdown();
            let node = runner.await.unwrap();
            assert_eq!(
                names(&node, &b),
                vec!["price.get"],
                "no error came back to the actor"
            );
            let _ = EventType::External;
        })
        .await;
}

#[test]
fn store_backed_mailbox_rows_are_removed_on_ack() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = Node::builder(store.clone(), shell(&clock))
        .mailboxes(Arc::new(StoreMailboxes::new(store.clone())))
        .build();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "k");
    node.send(&a, ev("x")).unwrap();
    let q = format!("ASK {{ GRAPH <{a}/mailbox> {{ ?m ?p ?o }} }}");
    assert!(store.query(&q).unwrap().into_boolean().unwrap());
    node.run_until_idle();
    assert!(!store.query(&q).unwrap().into_boolean().unwrap());
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
}
