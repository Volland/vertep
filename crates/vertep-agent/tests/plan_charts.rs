//! The `plan-charts` capability: plan submission, validation, the CEL-only restriction,
//! content addressing, sandboxed and bounded execution, results, lineage and
//! self-modification through versioning.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vertep_agent::host::AgentHost;
use vertep_agent::journal::AgentInput;
use vertep_agent::llm::ScriptedProvider;
use vertep_agent::machine::Record;
use vertep_agent::plan::{PlanErrorClass, PlanFormat, submit};
use vertep_agent::vocab;
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;

const ALLOWED: &str = "https://example.org/allowed";
const FORBIDDEN: &str = "https://example.org/forbidden";

/// The parent: `vt.plan.proposed` carries a plan into `running`, which invokes it as `trip`.
fn parent(bounds: &str) -> Chart {
    cel("idle", PARENT_HEAD, &parent_body(bounds))
}

const PARENT_HEAD: &str = r#"<datamodel><data id="plan" expr="''"/><data id="call" expr="''"/><data id="result" expr="null"/>
           <data id="class" expr="''"/><data id="bound" expr="''"/><data id="reason" expr="''"/>
           <data id="violations" expr="[]"/><data id="origin" expr="''"/></datamodel>"#;

fn parent_body(bounds: &str) -> String {
    format!(
        r##"<state id="idle">
    <transition event="vt.plan.proposed" target="running"><assign location="plan" expr="ev.data.plan"/><assign location="call" expr="has(ev.data.call) ? ev.data.call : ''"/></transition>
  </state>
  <state id="running">
    <invoke type="vt:plan" id="trip" {bounds} vt:grantTargets="#_parent {ALLOWED}">
      <param name="plan" expr="ctx.plan"/><param name="generatedBy" expr="ctx.call"/>
    </invoke>
    <transition event="hello"><assign location="origin" expr="ev.origin"/></transition>
    <transition event="done.invoke.trip" target="done"><assign location="result" expr="ev.data"/></transition>
    <transition event="error.plan" target="failed"><assign location="class" expr="ev.data['class']"/></transition>
    <transition event="leave" target="idle"/>
  </state>
  <state id="done"/>
  <state id="failed"/>"##
    )
}

fn host_with(chart: Chart) -> (AgentHost, vertep_runtime::VirtualClock) {
    let (b, clock) = builder();
    let mut host = b.build();
    spawn(&mut host, chart);
    (host, clock)
}

fn propose(host: &mut AgentHost, plan: &str) {
    host.send(
        AGENT,
        ev_data(
            vocab::PLAN_PROPOSED_EVENT,
            val(serde_json::json!({ "plan": plan })),
        ),
    );
    host.run_until_idle();
}

fn plan_records(host: &AgentHost) -> Vec<Record> {
    host.machine(AGENT)
        .unwrap()
        .records()
        .iter()
        .filter(|r| matches!(r, Record::Plan { .. } | Record::PlanExecution { .. }))
        .cloned()
        .collect()
}

const BOOKING: &str = r##"vertep: 1
name: booking
datamodel: cel
extensions:
  - { local: result, attribute: "{ booked: bool }" }
states:
  book:
    entry:
      - send: { event: hello, target: "#_parent" }
      - send: { event: confirmed }
    on: { confirmed: done }
  done:
    type: final
    donedata: { params: [ { name: booked, expr: "true" } ] }
"##;

// ----- Plan Submission ---------------------------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#Plan Submission#Plan in YAML accepted for validation]] {level: integration}
#[test]
fn plan_in_yaml_accepted_for_validation() {
    // an LLM call writes the plan; the chart proposes it with vt.plan.proposed
    let model = ScriptedProvider::new();
    model.push_text(BOOKING, 300, 120);
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(model)).build();
    spawn(&mut host, planning_chart());
    assert_eq!(config(&mut host), vec!["done"]);
    let recs = plan_records(&host);
    assert!(
        matches!(&recs[0], Record::Plan { accepted: true, generated_by: Some(g), .. } if g.contains("/llm-call/"))
    );
    assert_eq!(
        ctx(&mut host, "result"),
        Some(val(serde_json::json!({"booked": true})))
    );
}

const PLANNING: &str = r#"<state id="planning">
             <invoke type="vt:llm" id="planner" vt:provider="p" vt:model="planner-model" vt:template="Plan a trip."/>
             <transition event="done.invoke.planner" target="idle">
               <send event="vt.plan.proposed"><param name="plan" expr="ev.data.text"/><param name="call" expr="ev.data.call"/></send>
             </transition>
           </state>"#;

fn planning_chart() -> Chart {
    cel(
        "planning",
        PARENT_HEAD,
        &format!("{PLANNING}{}", parent_body("")),
    )
}

// @tg: verifies:: [[openspec:plan-charts#Plan Submission#Unparseable plan rejected]] {level: integration}
#[test]
fn unparseable_plan_rejected() {
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, "states: [ { unclosed");
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(ctx(&mut host, "class"), Some(Value::String("parse".into())));
    let err = submit("states: [ { unclosed", PlanFormat::Yaml, &BTreeSet::new()).unwrap_err();
    assert_eq!(err.class, PlanErrorClass::Parse);
    assert!(
        !err.violations.is_empty(),
        "the diagnostics travel with the error"
    );
    let recs = plan_records(&host);
    assert!(
        recs.iter()
            .all(|r| !matches!(r, Record::PlanExecution { .. })),
        "nothing is executed"
    );
}

// ----- Plan Validation ----------------------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#Plan Validation#Plan with dangling target rejected]] {level: integration}
#[test]
fn plan_with_dangling_target_rejected() {
    let plan = "vertep: 1\nname: p\ndatamodel: cel\nstates:\n  a: { on: { go: nowhere } }\n  b: { type: final }\n";
    let err = submit(plan, PlanFormat::Yaml, &BTreeSet::new()).unwrap_err();
    assert_eq!(err.class, PlanErrorClass::Invalid);
    assert!(
        err.violations.iter().any(|v| v.contains("nowhere")),
        "{err}"
    );
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, plan);
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("invalid".into()))
    );
}

// @tg: verifies:: [[openspec:plan-charts#Plan Validation#Valid plan accepted]] {level: integration}
#[test]
fn valid_plan_accepted() {
    let p = submit(BOOKING, PlanFormat::Yaml, &BTreeSet::new()).unwrap();
    assert_eq!(p.version, vertep_rdf::chart_version(&p.chart));
    assert!(p.version.as_str().starts_with("urn:vertep:chart:sha256:"));
    assert!(p.content_hash.starts_with("sha256:"));
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, BOOKING);
    assert_eq!(config(&mut host), vec!["done"]);
    assert!(
        matches!(&plan_records(&host)[0], Record::Plan { accepted: true, iri, .. } if iri == p.version.as_str())
    );
}

// ----- CEL-only Plans -------------------------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#CEL-only Plans#ECMAScript plan rejected]] {level: unit}
#[test]
fn ecmascript_plan_rejected() {
    for plan in [
        "vertep: 1\nname: p\ndatamodel: ecmascript\nstates:\n  a: { type: final }\n",
        "vertep: 1\nname: p\ndatamodel: cel\nstates:\n  a:\n    entry:\n      - script: \"x = 1\"\n",
    ] {
        let err = submit(plan, PlanFormat::Yaml, &BTreeSet::new()).unwrap_err();
        assert_eq!(err.class, PlanErrorClass::Invalid, "{err}");
        assert_eq!(err.reasons[0], "datamodel", "{err}");
    }
    // the null datamodel is allowed
    submit(
        "vertep: 1\nname: p\ndatamodel: \"null\"\nstates:\n  a: { type: final }\n",
        PlanFormat::Yaml,
        &BTreeSet::new(),
    )
    .unwrap();
}

// @tg: verifies:: [[openspec:plan-charts#CEL-only Plans#CEL expression type-checked]] {level: unit}
#[test]
fn cel_expression_type_checked() {
    let plan = "vertep: 1\nname: x\ndatamodel: cel\ncontext:\n  n: { type: string, init: \"'a'\" }\nstates:\n  s:\n    on:\n      go: { cond: \"ctx.n == 1\", target: t }\n  t: { type: final }\n";
    let err = submit(plan, PlanFormat::Yaml, &BTreeSet::new()).unwrap_err();
    assert_eq!(err.reasons, vec!["type"]);
    assert!(err.violations[0].contains("(string, int)"), "{err}");
}

// @tg: verifies:: [[openspec:plan-charts#CEL-only Plans#Rust-bound action rejected]] {level: unit}
#[test]
fn rust_bound_action_rejected() {
    let plan = "vertep: 1\nname: p\ndatamodel: cel\nstates:\n  a:\n    entry:\n      - call: \"vt:action/notify\"\n    on: { go: b }\n  b: { type: final }\n";
    let err = submit(plan, PlanFormat::Yaml, &BTreeSet::new()).unwrap_err();
    assert!(
        err.reasons.contains(&"ungranted-function".to_owned()),
        "{err}"
    );
    let granted: BTreeSet<String> = ["vt:action/notify".to_owned()].into();
    submit(plan, PlanFormat::Yaml, &granted).unwrap();
}

// ----- Content-addressed Plans ---------------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#Content-addressed Plans#Identical plans share an IRI]] {level: unit}
#[test]
fn identical_plans_share_an_iri() {
    let a = "vertep: 1\nname: p\ndatamodel: cel\nstates:\n  a: { on: { go: b }, entry: [ { log: { expr: \"'x'\" } } ] }\n  b: { type: final }\n";
    let b = "vertep: 1\ndatamodel: cel\nname: p\n\nstates:\n  a:\n    entry:\n      - log: { expr: \"'x'\" }\n    on:\n      go:   b\n  b:\n    type: final\n";
    let pa = submit(a, PlanFormat::Yaml, &BTreeSet::new()).unwrap();
    let pb = submit(b, PlanFormat::Yaml, &BTreeSet::new()).unwrap();
    assert_ne!(pa.content_hash, pb.content_hash, "the texts differ");
    assert_eq!(pa.version, pb.version);
}

// @tg: verifies:: [[openspec:plan-charts#Content-addressed Plans#Different plans get different IRIs]] {level: unit}
#[test]
fn different_plans_get_different_iris() {
    let a = "vertep: 1\nname: p\ndatamodel: cel\nstates:\n  a: { on: { go: b } }\n  b: { type: final }\n  c: { type: final }\n";
    let b = a.replace("go: b", "go: c");
    let pa = submit(a, PlanFormat::Yaml, &BTreeSet::new()).unwrap();
    let pb = submit(&b, PlanFormat::Yaml, &BTreeSet::new()).unwrap();
    assert_ne!(pa.version, pb.version);
}

// ----- Sandboxed Plan Execution -------------------------------------------------------------

const WAITING: &str = r##"vertep: 1
name: waiting
datamodel: cel
states:
  wait:
    entry:
      - send: { event: hello, target: "#_parent" }
    on: { next: step }
  step:
    entry:
      - send: { event: stepped, target: "#_parent" }
    on: { more: done }
  done: { type: final }
"##;

// @tg: verifies:: [[openspec:plan-charts#Sandboxed Plan Execution#Plan runs as invoked child]] {level: integration}
#[test]
fn plan_runs_as_invoked_child() {
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, WAITING);
    assert_eq!(
        config(&mut host),
        vec!["running"],
        "the start leaves the parent where it was"
    );
    assert_eq!(
        ctx(&mut host, "origin"),
        Some(Value::String(format!("{AGENT}#_trip")))
    );
    // the plan is never a virtual actor: no journal of its own
    assert!(host.journal(&format!("{AGENT}#_trip")).is_empty());
}

// @tg: verifies:: [[openspec:plan-charts#Sandboxed Plan Execution#Plan cancelled with its state]] {level: integration}
#[test]
fn plan_cancelled_with_its_state() {
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, WAITING);
    host.send(AGENT, ev("leave"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["idle"]);
    let recs = plan_records(&host);
    assert!(
        recs.iter()
            .any(|r| matches!(r, Record::PlanExecution { outcome, .. } if outcome == "cancelled"))
    );
    // its remaining steps never run: nothing it would send reaches the parent
    let stepped = host
        .journal(AGENT)
        .iter()
        .any(|e| matches!(&e.input, AgentInput::Event { event, .. } if event.name == "stepped"));
    assert!(!stepped);
}

// @tg: verifies:: [[openspec:plan-charts#Sandboxed Plan Execution#Plan cannot exceed grants]] {level: integration}
#[test]
fn plan_cannot_exceed_grants() {
    let plan = format!(
        r#"vertep: 1
name: sneaky
datamodel: cel
context:
  why: {{ init: "''" }}
states:
  try:
    entry:
      - send: {{ event: ok, target: "{ALLOWED}" }}
      - send: {{ event: steal, target: "{FORBIDDEN}" }}
    on:
      error.communication:
        target: caught
        actions: [ {{ assign: {{ location: ctx.why, expr: "ev.data.reason" }} }} ]
  caught:
    type: final
    donedata: {{ params: [ {{ name: why, expr: "ctx.why" }} ] }}
"#
    );
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, &plan);
    assert_eq!(config(&mut host), vec!["done"]);
    assert_eq!(
        ctx(&mut host, "result"),
        Some(val(serde_json::json!({"why": "not-granted"})))
    );
    let targets: Vec<&str> = host.outbox().iter().map(|s| s.target.as_str()).collect();
    assert_eq!(
        targets,
        vec![ALLOWED],
        "the forbidden message is not delivered"
    );
    assert!(
        host.machine(AGENT)
            .unwrap()
            .records()
            .iter()
            .any(|r| matches!(r, Record::Denied { target, .. } if target == FORBIDDEN))
    );
}

// ----- Plan Execution Bounds -------------------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#Plan Execution Bounds#Step bound exceeded]] {level: integration}
#[test]
fn step_bound_exceeded() {
    let looping = "vertep: 1\nname: loop\ndatamodel: cel\nstates:\n  spin:\n    entry:\n      - send: { event: tick }\n    on: { tick: spin }\n";
    let chart = parent(r#"vt:maxMacrosteps="50""#);
    let (mut host, _) = host_with(chart);
    propose(&mut host, looping);
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("bounds".into()))
    );
    let recs = plan_records(&host);
    assert!(recs.iter().any(
        |r| matches!(r, Record::PlanExecution { outcome, .. } if outcome == "bounds:macrosteps")
    ));
}

// @tg: verifies:: [[openspec:plan-charts#Plan Execution Bounds#Duration bound survives deactivation]] {level: integration}
#[test]
fn duration_bound_survives_deactivation() {
    let (mut host, clock) = host_with(parent(r#"vt:maxDuration="10s""#));
    propose(&mut host, WAITING);
    assert_eq!(config(&mut host), vec!["running"]);
    host.deactivate(AGENT);
    advance(&mut host, &clock, Duration::from_secs(11));
    assert!(host.is_active(AGENT), "the timer reactivated the parent");
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("bounds".into()))
    );
    let recs = plan_records(&host);
    assert!(recs.iter().any(
        |r| matches!(r, Record::PlanExecution { outcome, .. } if outcome == "bounds:duration")
    ));
}

// ----- Plan Results Returned to Parent ---------------------------------------------------------

// @tg: verifies:: [[openspec:plan-charts#Plan Results Returned to Parent#Plan completes with result]] {level: integration}
#[test]
fn plan_completes_with_result() {
    let (mut host, _) = host_with(parent(""));
    propose(&mut host, BOOKING);
    assert_eq!(config(&mut host), vec!["done"]);
    assert_eq!(
        ctx(&mut host, "result"),
        Some(val(serde_json::json!({"booked": true})))
    );
    let journaled = host.journal(AGENT).into_iter().any(|e| {
        matches!(&e.input,
        AgentInput::Event { event, .. } if event.name == "done.invoke.trip"
            && event.data == val(serde_json::json!({"booked": true})))
    });
    assert!(
        journaled,
        "done.invoke is journaled in the parent's journal"
    );
}

// @tg: verifies:: [[openspec:plan-charts#Plan Results Returned to Parent#Result violates declared type]] {level: integration}
#[test]
fn result_violates_declared_type() {
    let (mut host, _) = host_with(parent(""));
    propose(
        &mut host,
        &BOOKING.replace(r#"expr: "true""#, r#"expr: "'yes'""#),
    );
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("result".into()))
    );
}

// ----- Plan Lineage ---------------------------------------------------------------------------

fn llm_planned(plan: &str) -> AgentHost {
    let model = ScriptedProvider::new();
    model.push(Ok(text(plan, "planner-model", 300, 120)));
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(model)).build();
    spawn(&mut host, planning_chart());
    host
}

// @tg: verifies:: [[openspec:plan-charts#Plan Lineage#Lineage query]] {level: integration}
#[test]
fn lineage_query() {
    let mut host = llm_planned(BOOKING);
    assert_eq!(config(&mut host), vec!["done"]);
    let g = host.graph(AGENT).unwrap();
    assert!(vertep_agent::ontology::validate(&g).conforms());
    let rows = sparql(
        &g,
        &format!(
            r#"PREFIX a: <{ns}> PREFIX prov: <http://www.w3.org/ns/prov#>
            SELECT ?call ?model ?hash WHERE {{
              ?exec a a:PlanExecution ; a:actor <{AGENT}> ; a:outcome "done" ; prov:used ?plan .
              ?plan prov:wasGeneratedBy ?call .
              ?call a a:LlmCall ; a:model ?model ; a:promptHash ?hash .
            }}"#,
            ns = vocab::AGENT_NS
        ),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "planner-model");
    assert!(rows[0].2.starts_with("sha256:"));
    assert!(rows[0].0.contains("/llm-call/"));
}

// @tg: verifies:: [[openspec:plan-charts#Plan Lineage#Rejected plans recorded]] {level: integration}
#[test]
fn rejected_plans_recorded() {
    let bad = "vertep: 1\nname: p\ndatamodel: ecmascript\nstates:\n  a: { type: final }\n";
    let mut host = llm_planned(bad);
    assert_eq!(config(&mut host), vec!["failed"]);
    let g = host.graph(AGENT).unwrap();
    let rows = sparql(
        &g,
        &format!(
            r#"PREFIX a: <{ns}> PREFIX prov: <http://www.w3.org/ns/prov#>
            SELECT ?hash ?call ?v WHERE {{
              ?plan a a:Plan ; a:accepted false ; a:contentHash ?hash ; prov:wasGeneratedBy ?call ; a:violation ?v .
            }}"#,
            ns = vocab::AGENT_NS
        ),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, vertep_agent::json::sha256(bad.as_bytes()));
    assert!(rows[0].1.contains("/llm-call/"));
    assert!(rows[0].2.contains("ecmascript"));
}

fn sparql(g: &oxrdf::Graph, query: &str) -> Vec<(String, String, String)> {
    use spareval::{QueryEvaluator, QueryResults};
    let mut ds = oxrdf::Dataset::new();
    for t in g.iter() {
        ds.insert(t.in_graph(oxrdf::GraphNameRef::DefaultGraph));
    }
    let q = spargebra::SparqlParser::new().parse_query(query).unwrap();
    let QueryResults::Solutions(sols) = QueryEvaluator::new().prepare(&q).execute(&ds).unwrap()
    else {
        panic!("a SELECT")
    };
    let text = |t: Option<&oxrdf::Term>| match t {
        Some(oxrdf::Term::Literal(l)) => l.value().to_owned(),
        Some(oxrdf::Term::NamedNode(n)) => n.as_str().to_owned(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    sols.map(|s| {
        let s = s.unwrap();
        let v: Vec<&oxrdf::Term> = s.values().iter().flatten().collect();
        (
            text(v.first().copied()),
            text(v.get(1).copied()),
            text(v.get(2).copied()),
        )
    })
    .collect()
}

// ----- Self-modification Through Versioning ---------------------------------------------------

mod selfmod {
    use super::*;
    use vertep_agent::plan::{SelfModError, propose_self_modification};
    use vertep_runtime::{Address, EntryBody, Migration, Node, Shell, VirtualClock};

    const TY: &str = "https://example.org/charts/agent";

    fn node() -> (Node, Address) {
        let clock = VirtualClock::new(T0);
        let shell = Shell {
            spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
            clock: Arc::new(clock.clone()),
            sleep: Arc::new(clock),
        };
        let mut node = Node::builder(Arc::new(vertep_store::MemoryStore::new()), shell)
            .datamodels(Arc::new(vertep_core::null::WithNull(
                vertep_dm_cel::CelRegistry,
            )))
            .build();
        node.register_type(TY, v1()).unwrap();
        let a = Address::virtual_actor(TY, "1").unwrap();
        node.send(&a, ev("go")).unwrap();
        node.run_until_idle();
        (node, a)
    }

    fn v1() -> Chart {
        cel(
            "a",
            "",
            r#"<state id="a"><transition event="go" target="b"/><transition event="vt.self-modification.rejected"/></state>
               <state id="b"><transition event="back" target="a"/><transition event="vt.self-modification.rejected"/></state>"#,
        )
    }

    fn v2() -> Chart {
        cel(
            "a",
            "",
            r#"<state id="a"><transition event="go" target="c"/></state>
               <state id="c"><transition event="back" target="a"/><transition event="faster" target="a"/></state>"#,
        )
    }

    fn version(node: &mut Node, a: &Address) -> String {
        node.state(a).unwrap().version
    }

    // @tg: verifies:: [[openspec:plan-charts#Self-modification Through Versioning#Accepted self-modification]] {level: integration}
    #[test]
    fn accepted_self_modification() {
        let (mut node, a) = node();
        let before = version(&mut node, &a);
        let pos = propose_self_modification(&mut node, &a, v2(), |from, to| {
            Migration::identity("urn:vertep:migration:agent-v2", from, to)
                .map_configuration(["b"], ["c"])
        })
        .unwrap();
        let after = version(&mut node, &a);
        assert_ne!(after, before);
        assert_eq!(after, vertep_rdf::chart_version(&v2()).as_str());
        let j = node.journal(&a).unwrap();
        assert!(matches!(
            j.last().unwrap().body,
            EntryBody::Migration { .. }
        ));
        assert_eq!(j.last().unwrap().position, pos);
        node.send(&a, ev("faster")).unwrap();
        node.run_until_idle();
        assert_eq!(
            node.state(&a).unwrap().configuration,
            vec!["a"],
            "runs the new version"
        );
    }

    // @tg: verifies:: [[openspec:plan-charts#Self-modification Through Versioning#Migration with unmapped configuration rejected]] {level: integration}
    #[test]
    fn migration_with_unmapped_configuration_rejected() {
        let (mut node, a) = node();
        let before = version(&mut node, &a);
        let err = propose_self_modification(&mut node, &a, v2(), |from, to| {
            Migration::identity("urn:vertep:migration:naive", from, to)
        })
        .unwrap_err();
        assert!(matches!(err, SelfModError::Migrate(_)), "{err}");
        assert_eq!(
            version(&mut node, &a),
            before,
            "the actor stays on its version"
        );
        let j = node.journal(&a).unwrap();
        assert!(j.iter().any(|e| matches!(&e.body,
            EntryBody::Input { input, .. } if input.event.name == vocab::SELF_MODIFICATION_REJECTED_EVENT)),
            "the rejection is journaled");
    }

    // @tg: verifies:: [[openspec:plan-charts#Self-modification Through Versioning#No in-place mutation]] {level: integration}
    #[test]
    fn no_in_place_mutation() {
        // A decider (or an LLM result) can only hand the actor an event; a chart inside the
        // payload is data. The agent machine's chart is shared and immutable.
        let rec = Recording::default();
        rec.answer(vertep_agent::DecisionOutcome::Decision(vertep_agent::Decision::new(
            "go",
            val(serde_json::json!({ "chart": "<scxml><state id='pwned'/></scxml>", "states": ["pwned"] })),
        )));
        let (b, _) = builder();
        let mut host = b.decider("d", Arc::new(rec)).build();
        let chart = cel(
            "a",
            "",
            r#"<state id="a" vt:decide="d"><transition event="go" target="b"/></state><state id="b"/>"#,
        );
        let v = host.register(chart.clone()).unwrap();
        host.spawn(AGENT, &v).unwrap();
        host.run_until_idle();
        assert_eq!(config(&mut host), vec!["b"]);
        assert_eq!(host.machine(AGENT).unwrap().chart().version, v);
        let AgentInput::Start { version } = &host.journal(AGENT)[0].input else {
            panic!("entry 1 starts the actor")
        };
        assert_eq!(version, v.as_str());
        // on the runtime, only migration entries change a version
        let (mut node, a) = node();
        let before = version_of(&mut node, &a);
        node.send(
            &a,
            ev_data("back", val(serde_json::json!({ "chart": "<scxml/>" }))),
        )
        .unwrap();
        node.run_until_idle();
        assert_eq!(version_of(&mut node, &a), before);
        assert!(
            node.journal(&a)
                .unwrap()
                .iter()
                .all(|e| !matches!(e.body, EntryBody::Migration { .. }))
        );
    }

    fn version_of(node: &mut Node, a: &Address) -> String {
        version(node, a)
    }
}
