//! Scenarios of the `event-journal` capability.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;
use vertep_core::datamodel::Value;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{
    ActivationError, Address, CrashPoint, Envelope, FnActor, NodeConfig, StoreMailboxes,
};
use vertep_store::{
    AppendError, AppendMeta, Entry, FencingToken, JournalStore, MemoryStore, Position,
};

const TOGGLE: &str = r#"<state id="idle"><transition event="go" target="busy"/></state>
    <state id="busy"><transition event="go" target="idle"/></state>"#;

/// A chart type that accepts every event without leaving its state.
const SINK: &str = r#"<state id="s"><transition event="*"/></state>"#;

fn setup_toggle() -> (vertep_runtime::Node, (), Arc<MemoryStore>, Address) {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("toggle"), scxml("idle", TOGGLE))
        .unwrap();
    (node, (), store, va("toggle", "k"))
}

// @tg: verifies:: [[openspec:event-journal#Journal Of Accepted Inputs#One entry per turn]] {level: integration}
#[test]
fn one_entry_per_turn() {
    let (mut node, _, _store, a) = setup_toggle();
    let v = node.latest(&ty("toggle")).unwrap().version.clone();
    let id = node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0].message_id, id);
    assert_eq!(j[0].version, v.as_str());
    let EntryBody::Input { input, .. } = &j[0].body else {
        panic!("input entry")
    };
    assert_eq!(input.event.name, "go");
    assert_eq!(input.message_id, id);
}

// @tg: verifies:: [[openspec:event-journal#Journal Of Accepted Inputs#Entries are never rewritten]] {level: integration}
#[test]
fn entries_are_never_rewritten() {
    let (mut node, _, store, a) = setup_toggle();
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let first = store.read(a.node(), Position::ZERO).unwrap()[0]
        .entry
        .clone();
    for _ in 0..3 {
        node.send(&a, ev("go")).unwrap();
        node.run_until_idle();
    }
    let again = store.read(a.node(), Position::ZERO).unwrap();
    assert_eq!(again.len(), 4);
    assert!(again[0].entry.is_isomorphic(&first));
    // an append over an existing position is refused and writes nothing
    let forged = Entry::new(a.entry_iri(99), []);
    let r = store.append(a.node(), Position::ZERO, &[forged], &AppendMeta::none());
    assert!(matches!(r, Err(AppendError::Conflict { .. })));
    assert_eq!(store.head(a.node()).unwrap(), Position::new(4));
}

// @tg: verifies:: [[openspec:event-journal#Journal Entry Content#Entry carries its effects]] {level: integration}
#[test]
fn entry_carries_its_effects() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    let c = va("sink", "c");
    node.register_type(
        &ty("fan"),
        scxml(
            "idle",
            &format!(
                r#"<state id="idle"><transition event="go" target="done">
                     <send event="a" target="{b}"/><send event="b" target="{c}"/>
                     <send event="remind" delay="5s" id="r1"/>
                   </transition></state><state id="done"/>"#
            ),
        ),
    )
    .unwrap();
    let a = va("fan", "x");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    let eff = j[0].effects().unwrap();
    assert_eq!(eff.sends.len(), 2);
    assert_eq!(eff.sends[0].event.name, "a");
    assert_eq!(eff.sends[0].recipient.as_deref(), Some(b.as_str()));
    assert_eq!(eff.sends[1].event.name, "b");
    assert_ne!(eff.sends[0].iri, eff.sends[1].iri);
    assert!(eff.sends[0].iri.starts_with(a.entry_iri(1).as_str()));
    assert_eq!(eff.timers.len(), 1);
    assert_eq!(eff.timers[0].send_id, "r1");
    assert_eq!(
        eff.timers[0].iri,
        format!("{}/timer/r1", a.entry_iri(1).as_str())
    );
    // the receivers saw the sends under the same stable ids
    assert_eq!(node.journal(&b).unwrap()[0].message_id, eff.sends[0].iri);
    assert_eq!(node.journal(&c).unwrap()[0].message_id, eff.sends[1].iri);
}

// @tg: verifies:: [[openspec:event-journal#Journal Entry Content#Gap-free positions]] {level: integration}
#[test]
fn gap_free_positions() {
    let (mut node, _, _store, a) = setup_toggle();
    for _ in 0..3 {
        node.send(&a, ev("go")).unwrap();
    }
    node.run_until_idle();
    let pos: Vec<u64> = node
        .journal(&a)
        .unwrap()
        .iter()
        .map(|e| e.position)
        .collect();
    assert_eq!(pos, vec![1, 2, 3]);
}

// @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Append at the expected position]] {level: integration}
#[test]
fn append_at_the_expected_position() {
    let (mut node, _, store, a) = setup_toggle();
    for _ in 0..7 {
        node.send(&a, ev("go")).unwrap();
    }
    node.run_until_idle();
    assert_eq!(store.head(a.node()).unwrap(), Position::new(7));
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let j = node.journal(&a).unwrap();
    assert_eq!(j.last().unwrap().position, 8);
    assert_eq!(store.head(a.node()).unwrap(), Position::new(8));
}

fn two_nodes() -> (
    vertep_runtime::Node,
    vertep_runtime::Node,
    Arc<MemoryStore>,
    Address,
) {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut n1 = node_on(store.clone(), &clock, NodeConfig::default());
    let mut n2 = node_on(store.clone(), &clock, NodeConfig::default());
    for n in [&mut n1, &mut n2] {
        n.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
        n.register_type(
            &ty("tell"),
            scxml(
                "s",
                &format!(
                    r#"<state id="s"><transition event="*"><send event="told" target="{}"/></transition></state>"#,
                    va("sink", "log")
                ),
            ),
        )
        .unwrap();
    }
    (n1, n2, store, va("tell", "k"))
}

// @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Conflicting append rejected]] {level: integration}
#[test]
fn conflicting_append_rejected() {
    // at the store: two appends at expected position 7, one wins
    let store = MemoryStore::new();
    let a = Address::parse("urn:vertep:test:cas").unwrap();
    for i in 1..=7 {
        store
            .append(
                a.node(),
                Position::new(i - 1),
                &[Entry::new(a.entry_iri(i), [])],
                &AppendMeta::none(),
            )
            .unwrap();
    }
    let x = store.append(
        a.node(),
        Position::new(7),
        &[Entry::new(a.entry_iri(8), [])],
        &AppendMeta::none(),
    );
    let y = store.append(
        a.node(),
        Position::new(7),
        &[Entry::new(
            Address::parse("urn:vertep:test:other")
                .unwrap()
                .entry_iri(8),
            [],
        )],
        &AppendMeta::none(),
    );
    assert!(x.is_ok());
    assert!(
        matches!(y, Err(AppendError::Conflict { expected, actual }) if expected == Position::new(7) && actual == Position::new(8))
    );
    assert_eq!(store.read(a.node(), Position::ZERO).unwrap().len(), 8);

    // two activations of one actor on two nodes
    let (mut n1, mut n2, store, a) = two_nodes();
    n1.activate_actor(&a).unwrap();
    n2.activate_actor(&a).unwrap();
    n1.send(&a, ev("one")).unwrap();
    n1.run_until_idle();
    n2.send(&a, ev("two")).unwrap();
    n2.pump();
    assert_eq!(
        store.head(a.node()).unwrap(),
        Position::new(1),
        "the stale append left no trace"
    );
    let j = n1.journal(&a).unwrap();
    assert_eq!(j.len(), 1);
}

// @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Losing activation steps down]] {level: integration}
#[test]
fn losing_activation_steps_down() {
    let (mut n1, mut n2, store, a) = two_nodes();
    n1.activate_actor(&a).unwrap();
    n2.activate_actor(&a).unwrap();
    n1.send(&a, ev("one")).unwrap();
    n1.run_until_idle();
    let log = va("sink", "log");
    assert_eq!(store.head(log.node()).unwrap(), Position::new(1));
    n2.send(&a, ev("two")).unwrap();
    n2.pump();
    assert!(!n2.is_active(&a), "the loser is deactivated");
    assert_eq!(
        store.head(log.node()).unwrap(),
        Position::new(1),
        "the loser dispatched nothing"
    );
    assert_eq!(n2.mailbox(&a).len(), 1, "its message is not acknowledged");
    // the message is processed by a fresh activation that replays the winner's entry
    n2.run_until_idle();
    let j = n2.journal(&a).unwrap();
    assert_eq!(j.len(), 2);
    assert_eq!(j[1].effects().unwrap().sends.len(), 1);
}

// @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Fencing token passed through]] {level: integration}
#[test]
fn fencing_token_passed_through() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let shard = vertep_store::oxrdf::NamedNode::new("urn:vertep:shard:0").unwrap();
    let mut n1 = node_on(
        store.clone(),
        &clock,
        NodeConfig {
            fencing: Some(FencingToken::new(shard.clone(), 5)),
            ..NodeConfig::default()
        },
    );
    n1.register_type(&ty("toggle"), scxml("idle", TOGGLE))
        .unwrap();
    let a = va("toggle", "k");
    n1.send(&a, ev("go")).unwrap();
    n1.run_until_idle();
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
    // the store saw epoch 5: a lower epoch is now fenced
    let stale = store.append(
        a.node(),
        Position::new(1),
        &[Entry::new(a.entry_iri(2), [])],
        &AppendMeta::none().with_fencing(FencingToken::new(shard.clone(), 4)),
    );
    assert!(matches!(stale, Err(AppendError::Fenced { highest: 5, .. })));
    // a node holding an old epoch cannot append, compare-and-set notwithstanding
    let mut n2 = node_on(
        store.clone(),
        &clock,
        NodeConfig {
            fencing: Some(FencingToken::new(shard, 3)),
            ..NodeConfig::default()
        },
    );
    n2.register_type(&ty("toggle"), scxml("idle", TOGGLE))
        .unwrap();
    n2.send(&a, ev("go")).unwrap();
    n2.pump();
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
    assert!(!n2.is_active(&a));
}

// @tg: verifies:: [[openspec:event-journal#Message Deduplication#Redelivered message dropped]] {level: integration}
#[test]
fn redelivered_message_dropped() {
    let (mut node, _, _store, a) = setup_toggle();
    let env = Envelope::new("urn:vertep:test:msg-1", &a, ev("go"));
    node.send_envelope(env.clone()).unwrap();
    node.run_until_idle();
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(node.journal(&a).unwrap().len(), 1);
    assert!(
        node.mailbox(&a).is_empty(),
        "the duplicate was acknowledged"
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["busy"]);
}

// @tg: verifies:: [[openspec:event-journal#Message Deduplication#Same id to different actors]] {level: integration}
#[test]
fn same_id_to_different_actors() {
    let (mut node, _, _store, a) = setup_toggle();
    let b = va("toggle", "other");
    node.send_envelope(Envelope::new("urn:vertep:test:same", &a, ev("go")))
        .unwrap();
    node.send_envelope(Envelope::new("urn:vertep:test:same", &b, ev("go")))
        .unwrap();
    node.run_until_idle();
    assert_eq!(node.journal(&a).unwrap().len(), 1);
    assert_eq!(node.journal(&b).unwrap().len(), 1);
}

fn fan_out(node: &mut vertep_runtime::Node) -> (Address, Address) {
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    node.register_type(
        &ty("fan"),
        scxml(
            "s",
            &format!(
                r#"<state id="s"><transition event="go"><send event="one" target="{b}"/><send event="two" target="{b}"/></transition></state>"#
            ),
        ),
    )
    .unwrap();
    (va("fan", "a"), b)
}

// @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash after append, before dispatch]] {level: integration}
#[test]
fn crash_after_append_before_dispatch() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut n1 = node_on(
        store.clone(),
        &clock,
        NodeConfig {
            crash_at: Some(CrashPoint::AfterAppend),
            ..NodeConfig::default()
        },
    );
    let (a, b) = fan_out(&mut n1);
    n1.send(&a, ev("go")).unwrap();
    n1.run_until_idle();
    assert_eq!(store.head(a.node()).unwrap(), Position::new(1));
    assert_eq!(
        store.head(b.node()).unwrap(),
        Position::ZERO,
        "nothing dispatched before the crash"
    );
    let recorded: Vec<String> = n1.journal(&a).unwrap()[0]
        .effects()
        .unwrap()
        .sends
        .iter()
        .map(|s| s.iri.clone())
        .collect();
    drop(n1);
    let mut n2 = node_on(store.clone(), &clock, NodeConfig::default());
    fan_out(&mut n2);
    n2.run_until_idle();
    let got: Vec<String> = n2
        .journal(&b)
        .unwrap()
        .iter()
        .map(|e| e.message_id.clone())
        .collect();
    assert_eq!(got, recorded, "both sends, with their original ids");
}

// @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash before append]] {level: integration}
#[test]
fn crash_before_append() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let build = |crash| {
        let shell = vertep_runtime::Shell {
            spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
            clock: Arc::new(clock.clone()),
            sleep: Arc::new(clock.clone()),
        };
        vertep_runtime::Node::builder(store.clone(), shell)
            .mailboxes(Arc::new(StoreMailboxes::new(store.clone())))
            .config(NodeConfig {
                crash_at: crash,
                ..NodeConfig::default()
            })
            .build()
    };
    let mut n1 = build(Some(CrashPoint::BeforeAppend));
    let (a, b) = fan_out(&mut n1);
    n1.send(&a, ev("go")).unwrap();
    n1.run_until_idle();
    assert_eq!(store.head(a.node()).unwrap(), Position::ZERO);
    assert_eq!(
        store.head(b.node()).unwrap(),
        Position::ZERO,
        "no send of the lost macrostep"
    );
    drop(n1);
    let mut n2 = build(None);
    fan_out(&mut n2);
    n2.run_until_idle();
    assert_eq!(
        store.head(a.node()).unwrap(),
        Position::new(1),
        "the message was redelivered"
    );
    assert_eq!(n2.journal(&a).unwrap()[0].effects().unwrap().sends.len(), 2);
    assert_eq!(store.head(b.node()).unwrap(), Position::new(2));
}

// @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Stable send ids enable receiver dedup]] {level: integration}
#[test]
fn stable_send_ids_enable_receiver_dedup() {
    let (mut node, _clock, store) = fresh();
    let (a, b) = fan_out(&mut node);
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let s = node.journal(&a).unwrap()[0].effects().unwrap().sends[0].clone();
    // a retry delivers the same send again: same message id
    let mut env = Envelope::new(s.iri.clone(), &b, s.event.clone());
    env.correlation = s.correlation.clone();
    node.send_envelope(env).unwrap();
    node.run_until_idle();
    assert_eq!(
        store.head(b.node()).unwrap(),
        Position::new(2),
        "processed once"
    );
    let ids: Vec<String> = node
        .journal(&b)
        .unwrap()
        .iter()
        .map(|e| e.message_id.clone())
        .collect();
    assert_eq!(ids.iter().filter(|i| **i == s.iri).count(), 1);
}

const COUNTER: &str = r#"<datamodel><data id="n" expr="0"/></datamodel>
  <state id="s"><transition event="inc"><assign location="n" expr="n + 1"/></transition></state>"#;

// @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay equals recorded state]] {level: integration}
#[test]
fn replay_equals_recorded_state() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    node.register_type(
        &ty("counter"),
        ecma_scxml(
            "s",
            &format!(
                r#"<datamodel><data id="n" expr="0"/></datamodel>
                <state id="s"><transition event="inc"><assign location="n" expr="n + 1"/>
                <send event="tick" target="{}"/></transition></state>"#,
                va("sink", "t")
            ),
        ),
    )
    .unwrap();
    let a = va("counter", "c");
    for _ in 0..3 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    let live = node.state(&a).unwrap();
    assert_eq!(data_of(&live.datamodel, "n"), Some(Value::Number(3.0)));
    node.deactivate(&a);
    let replayed_before = node.stats().replayed;
    let replayed = node.state(&a).unwrap();
    assert_eq!(replayed, live);
    assert_eq!(node.stats().replayed - replayed_before, 3);
    // a reactivation also checks every replayed send against the journal
    node.activate_actor(&a).unwrap();
}

// @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay divergence detected]] {level: integration}
#[test]
fn replay_divergence_detected() {
    let (mut node, _, store, a) = setup_toggle();
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    // a second entry whose recorded digest does not match what the chart produces
    let mut forged = node.journal(&a).unwrap()[0].clone();
    forged.position = 2;
    forged.message_id = "urn:vertep:test:forged".into();
    forged.digest = "0000".into();
    store
        .append(
            a.node(),
            Position::new(1),
            &[forged.to_entry()],
            &AppendMeta::none(),
        )
        .unwrap();
    node.deactivate(&a);
    let err = node.activate_actor(&a).unwrap_err();
    assert_eq!(
        err,
        ActivationError::ReplayDivergence {
            actor: a.clone(),
            position: 2
        }
    );
    assert!(!node.is_active(&a));
    assert!(err.to_string().contains(a.as_str()));
}

// @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay produces no external effects]] {level: integration}
#[test]
fn replay_produces_no_external_effects() {
    let (mut node, _clock, store) = fresh();
    let (a, b) = fan_out(&mut node);
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(store.head(b.node()).unwrap(), Position::new(2));
    node.deactivate(&a);
    node.deactivate(&b);
    node.activate_actor(&a).unwrap();
    node.run_until_idle();
    assert!(node.mailbox(&b).is_empty());
    assert_eq!(
        store.head(b.node()).unwrap(),
        Position::new(2),
        "replay sent nothing"
    );
}

// @tg: verifies:: [[openspec:event-journal#Inputs Only From The Journal#Time is a journaled value]] {level: integration}
#[test]
fn time_is_a_journaled_value() {
    let (mut node, clock, _store) = fresh();
    node.register_type(
        &ty("clock"),
        ecma_scxml(
            "s",
            r#"<datamodel><data id="t" expr="0"/></datamodel>
            <state id="s"><transition event="stamp"><assign location="t" expr="Date.now()"/></transition></state>"#,
        ),
    )
    .unwrap();
    let a = va("clock", "c");
    clock.advance(Duration::from_millis(1234));
    node.send(&a, ev("stamp")).unwrap();
    node.run_until_idle();
    let ts = node.journal(&a).unwrap()[0].timestamp_ms;
    assert_eq!(ts, T0 + 1234);
    let t = data_of(&node.state(&a).unwrap().datamodel, "t");
    assert_eq!(t, Some(Value::Number(ts as f64)));
    clock.advance(Duration::from_secs(3600));
    node.deactivate(&a);
    let t2 = data_of(&node.state(&a).unwrap().datamodel, "t");
    assert_eq!(t2, t, "replay reads the journaled time, not the clock");
}

// @tg: verifies:: [[openspec:event-journal#Inputs Only From The Journal#Random values are recorded]] {level: integration}
#[test]
fn random_values_are_recorded() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("dice"),
        ecma_scxml(
            "s",
            r#"<datamodel><data id="r" expr="0"/></datamodel>
            <state id="s"><transition event="roll"><assign location="r" expr="Math.random()"/></transition></state>"#,
        ),
    )
    .unwrap();
    let a = va("dice", "a");
    let b = va("dice", "b");
    node.send(&a, ev("roll")).unwrap();
    node.send(&b, ev("roll")).unwrap();
    node.run_until_idle();
    let ra = data_of(&node.state(&a).unwrap().datamodel, "r").unwrap();
    let rb = data_of(&node.state(&b).unwrap().datamodel, "r").unwrap();
    assert_ne!(ra, rb, "seeded per entry");
    // the value is part of the recorded state: the entry's digest covers it and replay agrees
    let digest = node.journal(&a).unwrap()[0].digest.clone();
    node.deactivate(&a);
    assert_eq!(data_of(&node.state(&a).unwrap().datamodel, "r"), Some(ra));
    node.activate_actor(&a).unwrap();
    assert_eq!(node.journal(&a).unwrap()[0].digest, digest);
}

const LLM: &str = "urn:vertep:kind:llm";

fn llm_node(calls: Arc<AtomicUsize>) -> (vertep_runtime::Node, Address) {
    let (mut node, _clock, _store) = fresh();
    node.register_actor_kind(
        LLM,
        FnActor::invokable(move |_params| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(obj(&[
                    ("text", Value::String("The answer is 42.".into())),
                    ("model", Value::String("m-1".into())),
                ]))
            }
        }),
    );
    node.register_type(
        &ty("agent"),
        scxml(
            "idle",
            &format!(
                r#"<state id="idle"><transition event="ask" target="thinking"/></state>
                <state id="thinking"><invoke type="{LLM}" id="llm"/>
                  <transition event="done.invoke.llm" target="answered"/></state>
                <state id="answered"/>"#
            ),
        ),
    )
    .unwrap();
    (node, va("agent", "a"))
}

// @tg: verifies:: [[openspec:event-journal#External Results As Inputs#LLM result is journaled]] {level: integration}
#[tokio::test]
async fn llm_result_is_journaled() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut node, a) = llm_node(calls.clone());
    node.send(&a, ev("ask")).unwrap();
    node.settle().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let j = node.journal(&a).unwrap();
    let EntryBody::Input { input, .. } = &j[1].body else {
        panic!("input entry")
    };
    assert_eq!(input.event.name, "done.invoke.llm");
    assert_eq!(
        field(&input.event.data, "text"),
        Some(Value::String("The answer is 42.".into()))
    );
    assert_eq!(
        field(&input.event.data, "model"),
        Some(Value::String("m-1".into()))
    );
    assert_eq!(node.state(&a).unwrap().configuration, vec!["answered"]);
}

// @tg: verifies:: [[openspec:event-journal#External Results As Inputs#Replay does not call the model]] {level: integration}
#[tokio::test]
async fn replay_does_not_call_the_model() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut node, a) = llm_node(calls.clone());
    node.send(&a, ev("ask")).unwrap();
    node.settle().await;
    node.deactivate(&a);
    node.activate_actor(&a).unwrap();
    node.settle().await;
    assert_eq!(node.state(&a).unwrap().configuration, vec!["answered"]);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the completion came from the journal"
    );
}

// @tg: verifies:: [[openspec:event-journal#Snapshots#Activation from snapshot]] {level: integration}
#[test]
fn activation_from_snapshot() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut node = node_on(
        store,
        &clock,
        NodeConfig {
            snapshot_interval: 1000,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("counter"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("counter", "big");
    for _ in 0..1010 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    node.deactivate(&a);
    let before = node.stats().replayed;
    node.activate_actor(&a).unwrap();
    assert_eq!(
        node.stats().replayed - before,
        10,
        "only entries 1001 to 1010 replayed"
    );
    assert_eq!(
        data_of(&node.state(&a).unwrap().datamodel, "n"),
        Some(Value::Number(1010.0))
    );
}

// @tg: verifies:: [[openspec:event-journal#Snapshots#Snapshot is checked against replay]] {level: integration}
#[test]
fn snapshot_is_checked_against_replay() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut node = node_on(
        store.clone(),
        &clock,
        NodeConfig {
            snapshot_interval: 2,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("counter"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("counter", "c");
    for _ in 0..5 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    // corrupt the newest snapshot (position 4): wrong digest
    use vertep_store::GraphStore;
    let g = a.snapshot_iri(4);
    let digest =
        vertep_store::oxrdf::NamedNode::new(vertep_runtime::vocab::CONFIGURATION_DIGEST).unwrap();
    let old: Vec<_> = store
        .dataset()
        .iter()
        .filter(|q| {
            q.graph_name == vertep_store::oxrdf::GraphNameRef::NamedNode(g.as_ref())
                && q.predicate == digest.as_ref()
        })
        .map(|q| q.into_owned())
        .collect();
    assert_eq!(old.len(), 1);
    store.remove_quads(&old).unwrap();
    store
        .insert_quads(&[vertep_store::oxrdf::Quad::new(
            g.clone(),
            digest,
            vertep_store::oxrdf::Literal::new_simple_literal("bogus"),
            g,
        )])
        .unwrap();
    node.deactivate(&a);
    let before = node.stats().replayed;
    node.activate_actor(&a).unwrap();
    assert_eq!(
        node.stats().replayed - before,
        3,
        "fell back to the snapshot at 2"
    );
    assert_eq!(
        data_of(&node.state(&a).unwrap().datamodel, "n"),
        Some(Value::Number(5.0))
    );
}

// @tg: verifies:: [[openspec:event-journal#Snapshots#Snapshot policy is configurable]] {level: integration}
#[test]
fn snapshot_policy_is_configurable() {
    let store = Arc::new(MemoryStore::new());
    let clock = vertep_runtime::VirtualClock::new(T0);
    let mut node = node_on(
        store.clone(),
        &clock,
        NodeConfig {
            snapshot_interval: 3,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("counter"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("counter", "c");
    for _ in 0..7 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    let names: std::collections::BTreeSet<String> = store
        .dataset()
        .iter()
        .filter_map(|q| match q.graph_name {
            vertep_store::oxrdf::GraphNameRef::NamedNode(n)
                if n.as_str().contains("/snapshot/") =>
            {
                Some(n.as_str().to_owned())
            }
            _ => None,
        })
        .collect();
    let want: std::collections::BTreeSet<String> = [3, 6]
        .iter()
        .map(|p| a.snapshot_iri(*p).as_str().to_owned())
        .collect();
    assert_eq!(names, want);
}

// @tg: verifies:: [[openspec:event-journal#Journal Read Access#Time travel to a position]] {level: integration}
#[test]
fn time_travel_to_a_position() {
    let (mut node, _, _store, a) = setup_toggle();
    for _ in 0..5 {
        node.send(&a, ev("go")).unwrap();
    }
    node.run_until_idle();
    let at3 = node.state_at(&a, 3).unwrap();
    assert_eq!(at3.position, 3);
    assert_eq!(at3.configuration, vec!["busy"]);
    let at2 = node.state_at(&a, 2).unwrap();
    assert_eq!(at2.configuration, vec!["idle"]);
    assert!(node.is_active(&a), "the live actor is unaffected");
    let live = node.state(&a).unwrap();
    assert_eq!(
        (live.position, live.configuration),
        (5, vec!["busy".to_owned()])
    );
}

// @tg: verifies:: [[openspec:event-journal#Journal Read Access#Fork from a position]] {level: integration}
#[test]
fn fork_from_a_position() {
    let (mut node, _, _store, a) = setup_toggle();
    for _ in 0..3 {
        node.send(&a, ev("go")).unwrap();
    }
    node.run_until_idle();
    let f = Address::parse("urn:vertep:test:fork-of-k").unwrap();
    node.fork(&a, 2, &f).unwrap();
    let j = node.journal(&f).unwrap();
    let EntryBody::Fork {
        source, position, ..
    } = &j[0].body
    else {
        panic!("fork entry")
    };
    assert_eq!((source.as_str(), *position), (a.as_str(), 2));
    assert_eq!(node.state(&f).unwrap().configuration, vec!["idle"]);
    node.send(&f, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&f).unwrap().configuration, vec!["busy"]);
    assert_eq!(node.state(&a).unwrap().configuration, vec!["busy"]);
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    assert_eq!(node.state(&a).unwrap().configuration, vec!["idle"]);
    assert_eq!(
        node.state(&f).unwrap().configuration,
        vec!["busy"],
        "both evolve independently"
    );
    assert_eq!(node.journal(&a).unwrap().len(), 4);
    assert_eq!(node.journal(&f).unwrap().len(), 2);
}
