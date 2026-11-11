//! The `deciders` capability: decide states, the decider contract and kinds, journaled and
//! validated decisions, re-decision, deadlines, typed schemas, annotations and safety.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use oxrdf::{Term, TermRef};
use vertep_agent::decide::{DecideError, project};
use vertep_agent::decider::{
    Annotations, Decider, Decision, DecisionOutcome, GuardDecider, HumanDecider, Inbox, LlmDecider,
};
use vertep_agent::host::{AgentHost, InstantiateError};
use vertep_agent::journal::{AgentInput, MemoryJournal};
use vertep_agent::llm::ScriptedProvider;
use vertep_agent::machine::{AgentChart, ChartError};
use vertep_agent::vocab;
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;

const HEAD: &str = r#"<datamodel><data id="level" expr="1"/><data id="ok" expr="true"/>
  <data id="reason" expr="''"/><data id="hits" expr="0"/><data id="paths" expr="[]"/></datamodel>"#;

/// A review chart: `start` enters the decide state `review`, whose candidates are `approve`
/// (guarded by `ok`), `reject` and `escalate` (guarded by `level > 3`).
fn review_body(extra: &str) -> String {
    format!(
        r#"<state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject escalate" {extra}>
    <transition event="approve" cond="ctx.ok" target="approved"/>
    <transition event="reject" target="rejected"/>
    <transition event="escalate" cond="ctx.level > 3" target="escalated"/>
    <transition event="flag"><assign location="ok" expr="false"/></transition>
    <transition event="ping"><assign location="hits" expr="ctx.hits + 1"/></transition>
    <transition event="leave" target="idle"/>
    <transition event="error.decide.invalid"><assign location="reason" expr="ev.data.reason"/></transition>
    <transition event="vt.decide.refused"><assign location="reason" expr="ev.data.reason"/></transition>
    <transition event="error.decide.timeout" target="timedout"/>
  </state>
  <state id="approved"/><state id="rejected"/><state id="escalated"/><state id="timedout"/>"#
    )
}

fn review(extra: &str) -> Chart {
    cel("idle", HEAD, &review_body(extra))
}

/// A host with the recording decider bound to `d`, running the review chart in `review`.
fn reviewing(extra: &str) -> (AgentHost, vertep_runtime::VirtualClock, Recording) {
    let rec = Recording::default();
    let (b, clock) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, review(extra));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    (host, clock, rec)
}

fn decide(host: &mut AgentHost, correlation: &str, event: &str, payload: Value) {
    host.deliver_outcome(
        AGENT,
        correlation,
        DecisionOutcome::Decision(Decision::new(event, payload)),
    );
    host.run_until_idle();
}

fn decisions(host: &AgentHost) -> Vec<AgentInput> {
    host.journal(AGENT)
        .into_iter()
        .map(|e| e.input)
        .filter(|i| matches!(i, AgentInput::Decision { .. }))
        .collect()
}

// ----- Decide State Declaration ------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decide State Declaration#Decide state parsed from SCXML]] {level: integration}
#[test]
fn decide_state_parsed_from_scxml() {
    let chart = review(r#"vt:deadline="30s" vt:fallback="g" vt:maxDecisions="4""#);
    let ac = AgentChart::new(chart.clone()).unwrap();
    let spec = &ac.decides["review"];
    assert_eq!(spec.decider, "d");
    assert_eq!(spec.deadline_ms, Some(30_000));
    assert_eq!(spec.fallback.as_deref(), Some("g"));
    assert_eq!(spec.max_decisions, Some(4));
    assert_eq!(
        spec.candidates.as_deref(),
        Some(&["approve".to_owned(), "reject".into(), "escalate".into()][..])
    );
    let (version, g) = project(&chart);
    let node = vertep_agent::decide::state_node(&version, "review");
    let decide_state = oxrdf::NamedNode::new_unchecked(vocab::VT_DECIDE_STATE);
    assert!(g.contains(oxrdf::TripleRef::new(
        &node,
        oxrdf::vocab::rdf::TYPE,
        &decide_state
    )));
    // the element form is the same binding, so YAML charts can write it as one extension
    let element = cel(
        "review",
        "",
        r#"<state id="review"><vt:decide decider="d" deadline="2s"/><transition event="go" target="end"/></state><final id="end"/>"#,
    );
    let spec = &AgentChart::new(element).unwrap().decides["review"];
    assert_eq!(
        (spec.decider.as_str(), spec.deadline_ms),
        ("d", Some(2_000))
    );
    // the binding survives the SCXML and YAML writers and readers
    let want = AgentChart::new(chart.clone()).unwrap().decides;
    let xml = vertep_scxml::write(&chart).unwrap();
    assert!(xml.contains(r#"vt:decide="d""#), "{xml}");
    assert_eq!(AgentChart::new(common::chart(&xml)).unwrap().decides, want);
    let yaml = vertep_yaml::write(&chart).unwrap();
    let back = vertep_yaml::read_str(&yaml).unwrap();
    assert_eq!(AgentChart::new(back).unwrap().decides, want);
    let from_yaml = vertep_yaml::read_str(
        "vertep: 1\nname: y\nstates:\n  review:\n    extensions:\n      - { local: decide, attribute: llm/claude }\n      - { local: maxDecisions, attribute: \"3\" }\n    on: { approve: done }\n  done: { type: final }\n",
    )
    .unwrap();
    let spec = &AgentChart::new(from_yaml).unwrap().decides["review"];
    assert_eq!(
        (spec.decider.as_str(), spec.max_decisions),
        ("llm/claude", Some(3))
    );
}

// @tg: verifies:: [[openspec:deciders#Decide State Declaration#Decide state without decider rejected]] {level: integration}
#[test]
fn decide_state_without_decider_rejected() {
    let chart = cel(
        "s",
        "",
        r#"<state id="s" vt:decide=" "><transition event="go" target="t"/></state><state id="t"/>"#,
    );
    let err = AgentChart::new(chart.clone()).unwrap_err();
    assert_eq!(
        err,
        ChartError::Decide(vec![DecideError::MissingDecider { state: "s".into() }])
    );
    assert!(err.to_string().contains("s"));
    let (b, _) = builder();
    let mut host = b.build();
    assert!(host.register(chart).is_err());
}

// @tg: verifies:: [[openspec:deciders#Decide State Declaration#Unknown decider binding rejected at instantiation]] {level: integration}
#[test]
fn unknown_decider_binding_rejected_at_instantiation() {
    let (b, _) = builder();
    let mut host = b.build();
    let v = host.register(review("")).unwrap();
    let err = host.spawn(AGENT, &v).unwrap_err();
    assert_eq!(
        err,
        InstantiateError::UnknownDecider {
            state: "review".into(),
            binding: "d".into()
        }
    );
    assert!(err.to_string().contains("d"));
    assert!(host.journal(AGENT).is_empty());
    assert!(!host.is_active(AGENT));
}

// ----- Enabled Event Offer -----------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Enabled Event Offer#Only enabled transitions are offered]] {level: integration}
#[test]
fn only_enabled_transitions_are_offered() {
    let (_host, _, rec) = reviewing("");
    assert_eq!(rec.requests().len(), 1);
    assert_eq!(rec.offered(0), vec!["approve", "reject"]);
}

// @tg: verifies:: [[openspec:deciders#Enabled Event Offer#Candidates include enabled transitions of ancestors]] {level: integration}
#[test]
fn candidates_include_enabled_transitions_of_ancestors() {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(
        &mut host,
        cel(
            "case",
            "",
            r#"<state id="case" initial="review">
                 <transition event="cancel" target="cancelled"/>
                 <state id="review" vt:decide="d"><transition event="approve" target="done"/></state>
                 <state id="done"/>
               </state>
               <state id="cancelled"/>"#,
        ),
    );
    assert_eq!(rec.offered(0), vec!["approve", "cancel"]);
    let c = &rec.requests()[0].candidates[1];
    assert_eq!((c.source.as_str(), c.index), ("case", 0));
    // choosing the ancestor's event takes the ancestor's transition
    decide(
        &mut host,
        &rec.requests()[0].correlation,
        "cancel",
        Value::Undefined,
    );
    assert_eq!(config(&mut host), vec!["cancelled"]);
}

// @tg: verifies:: [[openspec:deciders#Enabled Event Offer#Empty candidate set]] {level: integration}
#[test]
fn empty_candidate_set() {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(
        &mut host,
        cel(
            "review",
            r#"<datamodel><data id="ok" expr="false"/></datamodel>"#,
            r#"<state id="review" vt:decide="d">
                 <transition event="approve" cond="ctx.ok" target="done"/>
                 <transition event="vt.decide.none" cond="ev.data.state == 'review'" target="stuck"/>
               </state>
               <state id="done"/><state id="stuck"/>"#,
        ),
    );
    assert!(rec.requests().is_empty(), "the decider is not consulted");
    assert_eq!(config(&mut host), vec!["stuck"]);
}

// ----- Decider Contract --------------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decider Contract#Decider returns a decision]] {level: integration}
#[test]
fn decider_returns_a_decision() {
    let rec = Recording::default();
    rec.answer(DecisionOutcome::Decision(Decision::new(
        "approve",
        val(serde_json::json!({"note": "fine"})),
    )));
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, review(""));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    let req = &rec.requests()[0];
    assert_eq!(req.actor, AGENT);
    assert_eq!(req.state, "review");
    let d = decisions(&host);
    assert_eq!(d.len(), 1);
    let AgentInput::Decision {
        correlation,
        event,
        payload,
        ..
    } = &d[0]
    else {
        unreachable!()
    };
    assert_eq!(correlation, &req.correlation);
    assert_eq!(event, "approve");
    assert_eq!(payload, &val(serde_json::json!({"note": "fine"})));
    assert_eq!(config(&mut host), vec!["approved"]);
}

// @tg: verifies:: [[openspec:deciders#Decider Contract#Decider refuses]] {level: integration}
#[test]
fn decider_refuses() {
    let rec = Recording::default();
    rec.answer(DecisionOutcome::refuse("not my call"));
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, review(""));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("not my call".into()))
    );
    assert!(decisions(&host).is_empty());
}

struct Meddler;

impl Decider for Meddler {
    fn kind(&self) -> vertep_agent::DeciderKind {
        vertep_agent::DeciderKind::Custom
    }

    fn decide(
        &self,
        request: vertep_agent::DecisionRequest,
    ) -> futures::future::BoxFuture<'static, DecisionOutcome> {
        // The decider can only change its own copy of what it was given.
        let mut mine = request.context.snapshot().clone();
        if let Value::Object(fields) = &mut mine {
            fields.clear();
        }
        let mine = Arc::new(mine);
        Box::pin(async move {
            let _ = mine;
            DecisionOutcome::Decision(Decision::new("reject", Value::Undefined))
        })
    }
}

// @tg: verifies:: [[openspec:deciders#Decider Contract#Decider has no write access]] {level: integration}
#[test]
fn decider_has_no_write_access() {
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(Meddler)).build();
    spawn(&mut host, review(""));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["rejected"]);
    assert_eq!(ctx(&mut host, "level"), Some(Value::Number(1.0)));
    assert_eq!(ctx(&mut host, "ok"), Some(Value::Bool(true)));
}

// ----- Decisions Are Journaled Inputs ------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Decision appended before processing]] {level: integration}
#[test]
fn decision_appended_before_processing() {
    let (mut host, _, rec) = reviewing("");
    let corr = rec.requests()[0].correlation.clone();
    let mut d = Decision::new("approve", Value::Undefined);
    d.annotations.rationale = Some("looks right".into());
    host.deliver_outcome(AGENT, &corr, DecisionOutcome::Decision(d));
    host.run_until_idle();
    let j = host.journal(AGENT);
    let (pos, entry) = j
        .iter()
        .find_map(|e| match &e.input {
            AgentInput::Decision {
                correlation,
                annotations,
                ..
            } => Some((e.position, (correlation.clone(), annotations.clone()))),
            _ => None,
        })
        .expect("the decision is journaled");
    assert_eq!(entry.0, corr);
    assert_eq!(entry.1.rationale.as_deref(), Some("looks right"));
    assert_eq!(
        host.state_at(AGENT, pos - 1).unwrap().configuration,
        vec!["review"]
    );
    assert_eq!(
        host.state_at(AGENT, pos).unwrap().configuration,
        vec!["approved"]
    );
}

// @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Replay does not call the decider]] {level: integration}
#[test]
fn replay_does_not_call_the_decider() {
    let journal = Arc::new(MemoryJournal::new());
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b
        .journal(journal.clone())
        .decider("d", Arc::new(rec.clone()))
        .build();
    spawn(&mut host, review(""));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    host.send(AGENT, ev("flag"));
    host.run_until_idle();
    let corr = rec.requests().last().unwrap().correlation.clone();
    decide(&mut host, &corr, "reject", Value::Undefined);
    let before = host.state(AGENT).unwrap();
    assert_eq!(before.configuration, vec!["rejected"]);

    let other = Recording::default();
    let (b, _) = builder();
    let mut replayed = b
        .journal(journal)
        .decider("d", Arc::new(other.clone()))
        .build();
    replayed.register(review("")).unwrap();
    let after = replayed.state(AGENT).unwrap();
    assert_eq!(after.configuration, before.configuration);
    assert_eq!(after.datamodel, before.datamodel);
    replayed.run_until_idle();
    assert!(
        other.requests().is_empty(),
        "the decider is invoked zero times"
    );
    assert!(replayed.stats().decider_calls.is_empty());
}

// @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Duplicate decision deduplicated]] {level: integration}
#[test]
fn duplicate_decision_deduplicated() {
    let (mut host, _, rec) = reviewing(r#"vt:maxDecisions="5""#);
    let corr = rec.requests()[0].correlation.clone();
    let input = AgentInput::Decision {
        correlation: corr,
        event: "reject".into(),
        payload: Value::Undefined,
        annotations: Annotations::default(),
    };
    host.deliver(AGENT, input.clone());
    host.deliver(AGENT, input);
    host.run_until_idle();
    assert_eq!(decisions(&host).len(), 1);
    assert_eq!(host.machine(AGENT).unwrap().decision_count("review"), 1);
    assert_eq!(config(&mut host), vec!["rejected"]);
}

// ----- Decision Validation Against Candidates ------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decision Validation Against Candidates#Decision outside the candidate set rejected]] {level: integration}
#[test]
fn decision_outside_the_candidate_set_rejected() {
    let (mut host, _, rec) = reviewing("");
    decide(
        &mut host,
        &rec.requests()[0].correlation,
        "delete_all",
        Value::Undefined,
    );
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("not-a-candidate".into()))
    );
}

const TYPED_HEAD: &str = r#"<vt:types><![CDATA[event approve: { amount: decimal, reason: string };
event reject: { reason: string };
event error.decide.invalid: { reason: string, paths: list<string> };]]></vt:types>
<datamodel><data id="paths" vt:type="list<string>" expr="[]"/><data id="reason" vt:type="string" expr="''"/></datamodel>"#;

const TYPED_BODY: &str = r#"<state id="review" vt:decide="d">
    <transition event="approve" target="approved"/>
    <transition event="reject" target="rejected"/>
    <transition event="error.decide.invalid"><assign location="reason" expr="ev.data.reason"/><assign location="paths" expr="ev.data.paths"/></transition>
  </state><state id="approved"/><state id="rejected"/>"#;

// @tg: verifies:: [[openspec:deciders#Decision Validation Against Candidates#Payload that violates the schema rejected]] {level: integration}
#[test]
fn payload_that_violates_the_schema_rejected() {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, cel("review", TYPED_HEAD, TYPED_BODY));
    let corr = rec.requests()[0].correlation.clone();
    decide(
        &mut host,
        &corr,
        "approve",
        val(serde_json::json!({"amount": 12.5})),
    );
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("payload-schema".into()))
    );
    assert_eq!(
        ctx(&mut host, "paths"),
        Some(Value::Array(vec![Value::String("/reason".into())]))
    );
}

// @tg: verifies:: [[openspec:deciders#Decision Validation Against Candidates#Stale decision rejected]] {level: integration}
#[test]
fn stale_decision_rejected() {
    let (mut host, _, rec) = reviewing("");
    let first = rec.requests()[0].correlation.clone();
    host.send(AGENT, ev("leave"));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    assert_eq!(rec.requests().len(), 2);
    assert_ne!(rec.requests()[1].correlation, first);
    decide(&mut host, &first, "approve", Value::Undefined);
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("stale".into()))
    );
}

// ----- Re-decision After Context Change ---------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Re-decision After Context Change#Candidate set shrinks while waiting]] {level: integration}
#[test]
fn candidate_set_shrinks_while_waiting() {
    let (mut host, _, rec) = reviewing("");
    let first = rec.requests()[0].correlation.clone();
    host.send(AGENT, ev("flag"));
    host.run_until_idle();
    assert_eq!(rec.cancelled(), vec![first.clone()]);
    assert_eq!(rec.requests().len(), 2);
    assert_eq!(rec.offered(1), vec!["reject"]);
    assert_ne!(rec.requests()[1].correlation, first);
    decide(&mut host, &first, "approve", Value::Undefined);
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("stale".into()))
    );
    assert_eq!(
        host.offers(AGENT)[0].correlation,
        rec.requests()[1].correlation
    );
}

// @tg: verifies:: [[openspec:deciders#Re-decision After Context Change#Unchanged candidate set keeps the request]] {level: integration}
#[test]
fn unchanged_candidate_set_keeps_the_request() {
    let (mut host, _, rec) = reviewing("");
    let first = rec.requests()[0].correlation.clone();
    host.send(AGENT, ev("ping"));
    host.run_until_idle();
    assert_eq!(ctx(&mut host, "hits"), Some(Value::Number(1.0)));
    assert_eq!(rec.requests().len(), 1);
    assert!(rec.cancelled().is_empty());
    assert_eq!(host.offers(AGENT)[0].correlation, first);
    decide(&mut host, &first, "approve", Value::Undefined);
    assert_eq!(config(&mut host), vec!["approved"]);
}

// ----- Decision Deadline and Fallback -----------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decision Deadline and Fallback#Deadline falls back to a deterministic decider]] {level: integration}
#[test]
fn deadline_falls_back_to_a_deterministic_decider() {
    let slow = Recording::default();
    let (b, clock) = builder();
    let mut host = b
        .decider("d", Arc::new(slow.clone()))
        .decider(
            "guards/escalate",
            Arc::new(GuardDecider::first_enabled(["reject"])),
        )
        .build();
    spawn(
        &mut host,
        review(r#"vt:deadline="30s" vt:fallback="guards/escalate""#),
    );
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    let first = slow.requests()[0].correlation.clone();
    advance(&mut host, &clock, Duration::from_secs(29));
    assert_eq!(config(&mut host), vec!["review"]);
    advance(&mut host, &clock, Duration::from_secs(1));
    assert_eq!(config(&mut host), vec!["rejected"]);
    assert_eq!(slow.cancelled(), vec![first.clone()]);
    let d = decisions(&host);
    let AgentInput::Decision {
        correlation,
        annotations,
        ..
    } = &d[0]
    else {
        unreachable!()
    };
    assert_ne!(correlation, &first);
    assert!(annotations.fallback);
    assert_eq!(annotations.decider.as_deref(), Some("guards/escalate"));
    assert_eq!(annotations.decider_kind.as_deref(), Some("guard"));
}

// @tg: verifies:: [[openspec:deciders#Decision Deadline and Fallback#Deadline without fallback]] {level: integration}
#[test]
fn deadline_without_fallback() {
    let rec = Recording::default();
    let (b, clock) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    // the timeout transition is targetless here, so the late decision meets the same state
    let body = review_body(r#"vt:deadline="30s""#).replace(
        r#"<transition event="error.decide.timeout" target="timedout"/>"#,
        r#"<transition event="error.decide.timeout"><assign location="reason" expr="'timeout'"/></transition>"#,
    );
    spawn(&mut host, cel("idle", HEAD, &body));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    let corr = rec.requests()[0].correlation.clone();
    advance(&mut host, &clock, Duration::from_secs(30));
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("timeout".into()))
    );
    decide(&mut host, &corr, "approve", Value::Undefined);
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("stale".into()))
    );
}

// @tg: verifies:: [[openspec:deciders#Decision Deadline and Fallback#Deadline survives deactivation]] {level: integration}
#[test]
fn deadline_survives_deactivation() {
    let rec = Recording::default();
    let (b, clock) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, review(r#"vt:deadline="30s""#));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    host.deactivate(AGENT);
    assert!(!host.is_active(AGENT));
    let activations = host.stats().activations;
    advance(&mut host, &clock, Duration::from_secs(31));
    assert!(host.is_active(AGENT), "the timer reactivated the actor");
    assert_eq!(host.stats().activations, activations + 1);
    assert_eq!(config(&mut host), vec!["timedout"]);
}

// ----- Decider Kinds -------------------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decider Kinds#Guard decider is deterministic]] {level: unit}
#[test]
fn guard_decider_is_deterministic() {
    let (_host, _, rec) = reviewing("");
    let req = rec.requests()[0].clone();
    let g = GuardDecider::with_rules(vec![
        vertep_agent::decider::GuardRule {
            event: "approve".into(),
            guard: Some("ctx.level > 5".into()),
            payload: None,
        },
        vertep_agent::decider::GuardRule {
            event: "reject".into(),
            guard: Some("ctx.ok".into()),
            payload: Some("{'why': 'level ' + string(ctx.level)}".into()),
        },
    ]);
    let a = g.choose(&req);
    let b = g.choose(&req);
    assert_eq!(a, b);
    let DecisionOutcome::Decision(d) = a else {
        panic!("{a:?}")
    };
    assert_eq!(d.event, "reject");
    assert_eq!(d.payload, val(serde_json::json!({"why": "level 1"})));
}

// @tg: verifies:: [[openspec:deciders#Decider Kinds#Human in the loop]] {level: integration}
#[test]
fn human_in_the_loop() {
    let inbox = Inbox::default();
    let (b, _) = builder();
    let mut host = b
        .decider("human/support", Arc::new(HumanDecider::new(inbox.clone())))
        .build();
    spawn(
        &mut host,
        cel(
            "review",
            TYPED_HEAD,
            &TYPED_BODY.replace("vt:decide=\"d\"", "vt:decide=\"human/support\""),
        ),
    );
    let pending = inbox.pending();
    assert_eq!(pending.len(), 1);
    let names: Vec<&str> = pending[0]
        .candidates
        .iter()
        .map(|c| c.event.as_str())
        .collect();
    assert_eq!(names, vec!["approve", "reject"]);
    assert_eq!(
        pending[0].candidates[1].schema["required"],
        serde_json::json!(["reason"])
    );
    assert!(inbox.answer(
        &pending[0].correlation,
        DecisionOutcome::Decision(Decision::new(
            "reject",
            val(serde_json::json!({"reason": "duplicate"}))
        ))
    ));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["rejected"]);
    let d = decisions(&host);
    let AgentInput::Decision { annotations, .. } = &d[0] else {
        unreachable!()
    };
    assert_eq!(annotations.decider_kind.as_deref(), Some("human"));
    assert!(inbox.pending().is_empty());
}

// @tg: verifies:: [[openspec:deciders#Decider Kinds#Actor decider uses the request protocol]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn actor_decider_uses_the_request_protocol() {
    use vertep_runtime::{Address, Node, Shell, VirtualClock};
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    let mut node = Node::builder(Arc::new(vertep_store::MemoryStore::new()), shell)
        .datamodels(Arc::new(vertep_dm_ecma::EcmaRegistry))
        .build();
    let ty = "https://example.org/charts/approver";
    node.register_type(
        ty,
        chart(
            r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="s">
              <state id="s"><transition event="vt.decide.request">
                <send event="vt:reply" targetexpr="_event.origin"><content expr="({event: 'approve', payload: {}, rationale: 'policy 7'})"/></send>
              </transition></state></scxml>"##,
        ),
    )
    .unwrap();
    let approver = Address::virtual_actor(ty, "1").unwrap();
    let decider =
        vertep_agent::ActorDecider::new(node.handle(), approver.clone(), Duration::from_secs(5));
    let (b, _) = builder();
    let mut host = b.decider("actor/approver", Arc::new(decider)).build();
    spawn(&mut host, review("").replace_decider("actor/approver"));
    host.send(AGENT, ev("start"));
    for _ in 0..20 {
        host.settle().await;
        node.settle().await;
    }
    assert_eq!(config(&mut host), vec!["approved"]);
    let corr = match &decisions(&host)[0] {
        AgentInput::Decision {
            correlation,
            annotations,
            ..
        } => {
            assert_eq!(annotations.rationale.as_deref(), Some("policy 7"));
            assert_eq!(annotations.decider_kind.as_deref(), Some("actor"));
            correlation.clone()
        }
        _ => unreachable!(),
    };
    // the approver received the request with the decision's correlation id
    let j = node.journal(&approver).unwrap();
    let got = j
        .iter()
        .find_map(|e| match &e.body {
            vertep_runtime::EntryBody::Input { input, .. }
                if input.event.name == vocab::DECIDE_REQUEST_EVENT =>
            {
                vertep_agent::json::field(&input.event.data, "correlation").cloned()
            }
            _ => None,
        })
        .expect("the request reached the approver");
    assert_eq!(got, Value::String(corr));
}

trait ReplaceDecider {
    fn replace_decider(self, binding: &str) -> Chart;
}

impl ReplaceDecider for Chart {
    fn replace_decider(mut self, binding: &str) -> Chart {
        fn walk(states: &mut [vertep_core::ir::State], binding: &str) {
            for s in states {
                for e in &mut s.extensions {
                    if e.local == "decide"
                        && let vertep_core::ir::ExtensionValue::Attribute(v) = &mut e.value
                    {
                        *v = binding.to_owned();
                    }
                }
                walk(&mut s.children, binding);
            }
        }
        walk(&mut self.states, binding);
        self
    }
}

// ----- Typed Decision Schemas ------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Typed Decision Schemas#JSON Schema from typed events]] {level: integration}
#[test]
fn json_schema_from_typed_events() {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(&mut host, cel("review", TYPED_HEAD, TYPED_BODY));
    let approve = &rec.requests()[0].candidates[0];
    assert_eq!(approve.event, "approve");
    let s = &approve.schema;
    assert_eq!(s["type"], "object");
    // a decimal is a JSON number (or its exact decimal text)
    let amount = &s["properties"]["amount"];
    assert!(
        amount["type"] == "number"
            || amount["anyOf"]
                .as_array()
                .is_some_and(|v| v.contains(&serde_json::json!({"type": "number"}))),
        "{amount}"
    );
    assert_eq!(s["properties"]["reason"]["type"], "string");
    let mut required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    required.sort_unstable();
    assert_eq!(required, vec!["amount", "reason"]);
    decide(
        &mut host,
        &rec.requests()[0].correlation,
        "approve",
        val(serde_json::json!({"amount": 12.5, "reason": "ok"})),
    );
    assert_eq!(config(&mut host), vec!["approved"]);
}

// @tg: verifies:: [[openspec:deciders#Typed Decision Schemas#Untyped chart still decidable]] {level: integration}
#[test]
fn untyped_chart_still_decidable() {
    let (mut host, _, rec) = reviewing("");
    let dynamic = vertep_agent::schema::dynamic_value_schema();
    for c in &rec.requests()[0].candidates {
        assert_eq!(c.schema, dynamic);
    }
    decide(
        &mut host,
        &rec.requests()[0].correlation,
        "approve",
        val(serde_json::json!([1, "anything", {"x": null}])),
    );
    assert_eq!(config(&mut host), vec!["approved"]);
}

// ----- Decision Annotations ----------------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Decision Annotations#Annotated LLM decision in the graph]] {level: integration}
#[test]
fn annotated_llm_decision_in_the_graph() {
    let model = ScriptedProvider::new();
    model.push_text(
        r#"{"event":"approve","payload":{},"rationale":"invoice matches PO","confidence":0.82}"#,
        900,
        40,
    );
    let decider = LlmDecider::new("claude", Arc::new(model.clone()), "claude-opus-5-5");
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(decider)).build();
    spawn(&mut host, review(""));
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["approved"]);
    // the model was asked for a oneOf over the candidates
    let schema = model.requests()[0].output_schema.clone().unwrap();
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    let g = host.graph(AGENT).unwrap();
    let chose = oxrdf::NamedNode::new_unchecked(vocab::CHOSE);
    let stmt = g
        .triples_for_predicate(&chose)
        .next()
        .expect("decision chose an event")
        .into_owned();
    assert!(stmt.object.to_string().ends_with("event/approve>"));
    let reifier = g
        .iter()
        .find(|t| {
            t.predicate == oxrdf::vocab::rdf::REIFIES
                && matches!(t.object, TermRef::Triple(inner) if *inner == stmt)
        })
        .expect("the statement is annotated")
        .subject
        .into_owned();
    let value = |p: &str| -> Term {
        g.object_for_subject_predicate(&reifier, &oxrdf::NamedNode::new_unchecked(p))
            .unwrap_or_else(|| panic!("annotation {p}"))
            .into_owned()
    };
    assert_eq!(
        value(vocab::RATIONALE).to_string(),
        "\"invoice matches PO\""
    );
    assert!(value(vocab::CONFIDENCE).to_string().starts_with("\"0.82\""));
    assert_eq!(value(vocab::MODEL).to_string(), "\"scripted-model\"");
    assert!(
        value(vocab::PROMPT_HASH)
            .to_string()
            .starts_with("\"sha256:")
    );
    assert!(vertep_agent::ontology::validate(&g).conforms());
}

// @tg: verifies:: [[openspec:deciders#Decision Annotations#Annotations do not change behaviour]] {level: integration}
#[test]
fn annotations_do_not_change_behaviour() {
    let run = |rationale: &str, confidence: f64| {
        let (mut host, _, rec) = reviewing("");
        let mut d = Decision::new("approve", val(serde_json::json!({"n": 1})));
        d.annotations.rationale = Some(rationale.into());
        d.annotations.confidence = Some(confidence);
        host.deliver_outcome(
            AGENT,
            &rec.requests()[0].correlation,
            DecisionOutcome::Decision(d),
        );
        host.run_until_idle();
        let s = host.state(AGENT).unwrap();
        (s.configuration, s.datamodel)
    };
    assert_eq!(run("because", 0.1), run("a different reason", 0.99));
}

// @tg: verifies:: [[openspec:deciders#Decision Annotations#Confidence out of range rejected]] {level: integration}
#[test]
fn confidence_out_of_range_rejected() {
    let (mut host, _, rec) = reviewing("");
    let mut d = Decision::new("approve", Value::Undefined);
    d.annotations.confidence = Some(1.4);
    host.deliver_outcome(
        AGENT,
        &rec.requests()[0].correlation,
        DecisionOutcome::Decision(d),
    );
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["review"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("annotation".into()))
    );
}

// ----- Non-blocking Decision Wait ----------------------------------------------------------

// @tg: verifies:: [[openspec:deciders#Non-blocking Decision Wait#Other events processed while waiting]] {level: integration}
#[test]
fn other_events_processed_while_waiting() {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    spawn(
        &mut host,
        cel(
            "p",
            r#"<datamodel><data id="pings" expr="0"/></datamodel>"#,
            r#"<parallel id="p">
                 <state id="a"><state id="review" vt:decide="d"><transition event="approve" target="ok"/></state><state id="ok"/></state>
                 <state id="b"><state id="idle"><transition event="ping" target="pinged"><assign location="pings" expr="ctx.pings + 1"/></transition></state><state id="pinged"/></state>
               </parallel>"#,
        ),
    );
    let corr = rec.requests()[0].correlation.clone();
    assert_eq!(
        rec.offered(0),
        vec!["approve"],
        "ping is not a candidate of review"
    );
    host.send(AGENT, ev("ping"));
    host.run_until_idle();
    assert_eq!(ctx(&mut host, "pings"), Some(Value::Number(1.0)));
    assert!(config(&mut host).contains(&"pinged".to_owned()));
    assert_eq!(host.offers(AGENT)[0].correlation, corr);
    assert!(rec.cancelled().is_empty());
    decide(&mut host, &corr, "approve", Value::Undefined);
    assert!(config(&mut host).contains(&"ok".to_owned()));
}

// @tg: verifies:: [[openspec:deciders#Non-blocking Decision Wait#Leaving the decide state cancels the request]] {level: integration}
#[test]
fn leaving_the_decide_state_cancels_the_request() {
    let (mut host, _, rec) = reviewing("");
    let corr = rec.requests()[0].correlation.clone();
    host.send(AGENT, ev("leave"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["idle"]);
    assert_eq!(rec.cancelled(), vec![corr.clone()]);
    assert!(host.offers(AGENT).is_empty());
    decide(&mut host, &corr, "approve", Value::Undefined);
    assert_eq!(config(&mut host), vec!["idle"]);
    let records = host.machine(AGENT).unwrap().records().to_vec();
    assert!(records.iter().any(|r| matches!(
        r,
        vertep_agent::Record::Decision { reason: Some(x), accepted: false, .. } if x == "stale"
    )));
}

// ----- Decider Safety (the Rust half; the Lean oracle comparison is task 8.5) -----------------

// @tg: verifies:: [[openspec:deciders#Decider Safety#Arbitrary decider output is safe]] {level: integration}
#[test]
fn arbitrary_decider_output_is_safe() {
    use vertep_agent::machine::{AgentEffect, AgentMachine};
    // A deterministic pseudo-random walk of adversarial decider outputs (unknown events, bad
    // payloads, stale and forged correlation ids, out-of-range confidences) interleaved with
    // context changes, applied to the pure machine. Every accepted decision must take exactly
    // transitions that were enabled at offer time; every rejected one must change nothing.
    let mut seed: u64 = 0x5eed_1234_abcd_0042;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let head = TYPED_HEAD.replace(
        "<datamodel>",
        r#"<datamodel><data id="ok" vt:type="bool" expr="true"/>"#,
    );
    let chart = Arc::new(
        AgentChart::new(cel(
            "idle",
            &head,
            r#"<state id="idle"><transition event="start" target="case"/></state>
            <state id="case" initial="review">
              <transition event="cancel" target="idle"/>
              <state id="review" vt:decide="d">
                <transition event="approve" cond="ctx.ok" target="approved"/>
                <transition event="reject" target="rejected"/>
                <transition event="flag"><assign location="ok" expr="!ctx.ok"/></transition>
              </state>
              <state id="approved"><transition event="back" target="review"/></state>
              <state id="rejected"><transition event="back" target="review"/></state>
            </state>"#,
        ))
        .unwrap(),
    );
    let registry: Arc<dyn vertep_core::datamodel::DatamodelRegistry + Send + Sync> =
        Arc::new(vertep_core::null::WithNull(vertep_dm_cel::CelRegistry));
    let mut accepted_total = 0;
    let mut rejected_total = 0;
    for run in 0..60 {
        let (mut m, _) = AgentMachine::start(AGENT, chart.clone(), registry.clone(), T0).unwrap();
        let mut position = 1;
        let mut seen: Vec<String> = Vec::new();
        for step in 0..25 {
            position += 1;
            let input = if next(3) == 0 {
                let e = ["start", "flag", "cancel", "back"][next(4) as usize];
                AgentInput::Event {
                    event: ev(e),
                    message_id: format!("m{run}.{step}"),
                }
            } else {
                let open = m.open_offers();
                let correlation = match (open.first(), next(4)) {
                    (Some(o), 0 | 1) => o.correlation.clone(),
                    _ if !seen.is_empty() => seen[next(seen.len() as u64) as usize].clone(),
                    _ => "urn:forged".to_owned(),
                };
                let event = [
                    "approve",
                    "reject",
                    "delete_all",
                    "flag",
                    "cancel",
                    "vt.decide.none",
                ][next(6) as usize];
                let payload = match next(3) {
                    0 => val(serde_json::json!({"reason": "r", "amount": 1})),
                    1 => val(serde_json::json!({"amount": "NaN"})),
                    _ => Value::Undefined,
                };
                let mut annotations = Annotations::default();
                if next(5) == 0 {
                    annotations.confidence = Some(1.5);
                }
                AgentInput::Decision {
                    correlation,
                    event: event.into(),
                    payload,
                    annotations,
                }
            };
            let before = m.configuration();
            let a = m.apply(&input, position, T0).unwrap();
            for e in &a.effects {
                if let AgentEffect::Offer(r) = e {
                    seen.push(r.correlation.clone());
                }
            }
            if matches!(input, AgentInput::Decision { .. }) && !a.duplicate {
                match a.accepted.as_slice() {
                    [d] => {
                        accepted_total += 1;
                        assert!(
                            !d.taken.is_empty(),
                            "an accepted decision takes a transition"
                        );
                        for t in &d.taken {
                            assert!(d.offered.contains(t), "{t:?} was not enabled at offer time");
                        }
                    }
                    [] => {
                        rejected_total += 1;
                        assert_eq!(
                            before,
                            m.configuration(),
                            "a rejected output changes nothing"
                        );
                    }
                    more => panic!("one input accepted {} decisions", more.len()),
                }
            }
        }
    }
    assert!(
        accepted_total > 20 && rejected_total > 20,
        "{accepted_total} / {rejected_total}"
    );
}
