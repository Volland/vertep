//! The refusal path through a running node ([[openspec:rule-invariants#Refusal Is Visible And Writes No Entry]]).
//! Needs the native engine library; see `tests/invariants.rs`.
#![cfg(feature = "native")]

use std::sync::Arc;

use vertep_core::datamodel::{Event, EventType};
use vertep_rules::{enforce, put_program};
use vertep_runtime::{Address, Node, NodeConfig, Shell, VirtualClock};
use vertep_store::{JournalStore, Position};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value};

const T0: i64 = 1_794_355_200_000;
const CHART_TYPE: &str = "https://example.org/charts/counter";
const PROGRAM: &str = "urn:vt:program:no-blocked-actor";

const INVARIANT: &str = r#"
tm.violation-delta "blocked_actor" "the actor is blocked" [E1, E2] :-
  tm.added _ (iri "https://w3id.org/vertep/vt#actor") A E1,
  tm.triple A (iri "urn:vt:status") (str "blocked") E2.
tm.violation error "blocked_actor" "the actor is blocked" [E1, E2] :-
  tm.triple _ (iri "https://w3id.org/vertep/vt#actor") A E1,
  tm.triple A (iri "urn:vt:status") (str "blocked") E2.
"#;

fn node(shard: Arc<TiramemsuShard>, clock: &VirtualClock) -> Node {
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    Node::builder(shard, shell)
        .config(NodeConfig::default())
        .build()
}

fn chart() -> vertep_core::ir::Chart {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <state id="a"><transition event="go" target="b"/></state>
             <state id="b"><transition event="back" target="a"/></state>
           </scxml>"#,
        &opts,
    )
    .unwrap()
    .chart
}

// @tg: verifies:: [[openspec:rule-invariants#Refusal Is Visible And Writes No Entry#Dead letter]] {level: integration}
// @tg: verifies:: [[openspec:rule-invariants#Refusal Is Visible And Writes No Entry#No entry]] {level: integration}
#[test]
fn a_refused_append_is_dead_lettered_and_the_actor_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    let clock = VirtualClock::new(T0);
    let mut node = node(shard.clone(), &clock);
    node.register_type(CHART_TYPE, chart()).unwrap();
    let actor = Address::virtual_actor(CHART_TYPE, "k1").unwrap();

    // the actor takes one turn before the rule exists
    node.send(&actor, Event::named("go", EventType::External))
        .unwrap();
    node.run_until_idle();
    let before = shard.read(actor.node(), Position::ZERO).unwrap().len();
    assert!(before >= 1, "the first turn is journaled: {before}");

    // a second actor is blocked, and the invariant is enforced
    put_program(&shard, PROGRAM, INVARIANT).unwrap();
    let other = Address::virtual_actor(CHART_TYPE, "k2").unwrap();
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.assert(
                Value::iri(other.node().as_str()),
                Value::iri("urn:vt:status"),
                Value::str("blocked"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    enforce(&shard, PROGRAM, true).unwrap();

    node.send(&other, Event::named("go", EventType::External))
        .unwrap();
    node.run_until_idle();

    // nothing was written for the blocked actor, and the message is a dead letter naming the rule
    assert!(shard.read(other.node(), Position::ZERO).unwrap().is_empty());
    let dead = node.dead_letters(&other).unwrap();
    assert_eq!(dead.len(), 1, "{dead:?}");
    assert_eq!(dead[0].event, "go");
    assert!(
        dead[0].reason.starts_with("invariant: blocked_actor"),
        "{}",
        dead[0].reason
    );
    // the other actor is unaffected
    node.send(&actor, Event::named("back", EventType::External))
        .unwrap();
    node.run_until_idle();
    assert!(shard.read(actor.node(), Position::ZERO).unwrap().len() > before);
}
