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

// @tg: verifies:: [[openspec:rule-decider#Offer And State Are Statements#One record]] {level: integration}
#[test]
fn the_explanation_shows_the_derivation_and_the_offer_the_rule_read() {
    use vertep_agent::host::AgentHost;
    use vertep_core::datamodel::{Event, EventType};
    use vertep_rules::{RuleDecider, RulePolicy};

    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    put_program(
        &shard,
        "urn:vt:program:p",
        r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:reject")) [X, Eo] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:reject") Eo.
"#,
    )
    .unwrap();
    let decider = Arc::new(
        RuleDecider::new(shard.clone(), "urn:vt:program:p", RulePolicy::default()).unwrap(),
    );
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .decider("d", decider.clone())
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let chart = vertep_scxml::read(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="idle">
             <state id="idle"><transition event="start" target="review"/></state>
             <state id="review" vt:decide="d" vt:candidates="approve reject">
               <transition event="approve" cond="false" target="approved"/>
               <transition event="reject" target="rejected"/>
             </state>
             <state id="approved"/><state id="rejected"/>
           </scxml>"#,
        &opts,
    )
    .unwrap()
    .chart;
    let v = host.register(chart).unwrap();
    let actor = "https://example.org/agents/r1";
    host.spawn(actor, &v).unwrap();
    host.send(actor, Event::named("start", EventType::External));
    host.run_until_idle();
    let n = host.journal(actor).len() as u64;
    let x = host.explain(actor, n).unwrap();
    let d = x.decision.unwrap();
    assert_eq!(d.chosen.as_deref(), Some("reject"));
    assert!(d.claim.is_none(), "a rule's reason is not a claim");
    let derivation = d.derivation.expect("the derivation is on the entry");
    assert_eq!(derivation["event"], "reject");
    // the offer the explanation reports is the offer the rule read: both come from the request
    assert_eq!(d.offered, vec!["reject"]);
    assert_eq!(decider.derivations()[0].support.len(), 2);
    assert_eq!(derivation["support"].as_array().unwrap().len(), 2);
}
