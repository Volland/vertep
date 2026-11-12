//! Differential tests of decide states with adversarial decider outputs (m3-agents 8.5).
//!
//! Generated well-formed charts (the differential generator of `vertep-testkit`) get a
//! `vt:decide` binding on one of their states and, on some of their event names, declared
//! payload types (a `vt:types` block: records, `optional`, lists, unions, a named type, no
//! payload), which makes the agent derive a payload schema for those candidates. The Rust agent
//! machine (`vertep-agent`) then receives a generated mix of events and decisions: decisions for
//! the open offer, stale and forged correlation ids, unknown events, events that were not
//! offered, redeliveries, and payloads that do and do not conform to the chosen event's
//! declared type.
//!
//! Two Lean oracles judge every run:
//!
//! - every decision's outcome is replayed through the Lean decider model by `vertep-decide`,
//!   which must reach the same outcome (taken, rejected with the same reason, or dropped as a
//!   duplicate) and, for a taken decision, a transition Rust took that was a candidate of the
//!   open offer, so enabled at offer time (`Vertep.Agent.decider_safety`). Each line carries the
//!   candidate's schema and the payload, so whether the payload conforms (`payload-schema`
//!   rejections) is decided by the oracle's own checker, not taken from the Rust run;
//! - the core macrosteps of the agent's session, recorded with their datamodel answer tape
//!   (`AgentMachine::start_with`), are replayed through `vertep-oracle`, which must produce
//!   the same macrostep traces.
//!
//! Runs use the small microstep budget of the plain differential runs
//! (`GENERATED_MICROSTEP_BUDGET`), so a generated eventless loop ends quickly.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use vertep_agent::{
    AgentChart, AgentEffect, AgentInput, AgentMachine, Annotations, CoreTape, Record, StartOptions,
};
use vertep_conformance::LeanTools;
use vertep_conformance::differential::{GENERATED_MICROSTEP_BUDGET, chart_config, event_names};
use vertep_conformance::lean::{compare_run, replay};
use vertep_core::datamodel::{DatamodelRegistry, Event, EventType, Value};
use vertep_core::ir::{
    Chart, Extension, ExtensionValue, Span, State, StateElement, Transition, VT_NS,
};
use vertep_core::types::chart::set_declarations;
use vertep_core::types::parse_declarations;
use vertep_testkit::arb_chart_with;

const REQUIRED: bool = cfg!(feature = "differential");

/// 2026-11-11T00:00:00Z.
const T0: i64 = 1_794_355_200_000;

const ACTOR: &str = "urn:vertep:test:agent";

/// One generated input.
#[derive(Clone, Debug)]
enum Move {
    /// An external event, picked among the chart's event names.
    Event(prop::sample::Index),
    /// A decision: correlation pick (0, 1 and 2 the open offer, 3 an earlier one, 4 forged),
    /// event pick (picks 0 and 1: an offered candidate; otherwise among the candidates, the
    /// chart's event names and an unknown name) and payload pick (pick 0 with an event of
    /// declared type: a payload of that type; otherwise among [`payloads`]).
    Decide(
        u8,
        prop::sample::Index,
        prop::sample::Index,
        prop::sample::Index,
    ),
    /// The previous decision again.
    Redo,
}

fn arb_move() -> impl Strategy<Value = Move> {
    prop_oneof![
        3 => any::<prop::sample::Index>().prop_map(Move::Event),
        6 => (
            0..5u8,
            any::<prop::sample::Index>(),
            any::<prop::sample::Index>(),
            any::<prop::sample::Index>()
        )
            .prop_map(|(c, i, e, p)| Move::Decide(c, i, e, p)),
        1 => Just(Move::Redo),
    ]
}

/// Payload types declared on generated event names (no `decimal`, `datetime` or `iri`: their
/// schemas use `pattern`), each with a payload of that type.
/// A payload type (`None`: no payload) and a value of that type.
type Declared = (Option<&'static str>, fn() -> Value);

const TYPES: [Declared; 8] = [
    (None, || Value::Undefined),
    (Some("{ n: int }"), || obj(&[("n", Value::Number(1.0))])),
    (Some("{ s: string, b: optional<bool> }"), || {
        obj(&[("s", Value::String("a".into()))])
    }),
    (Some("list<int>"), || {
        Value::Array(vec![Value::Number(1.0), Value::Number(2.0)])
    }),
    (Some("string"), || Value::String("x".into())),
    (Some("ok string | none"), || {
        obj(&[("tag", Value::String("none".into()))])
    }),
    (Some("Pair"), || {
        obj(&[("n", Value::Number(1.0)), ("s", Value::String("x".into()))])
    }),
    (Some("bool"), || Value::Bool(true)),
];

fn obj(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

/// Decision payloads, well and ill typed for the declared types.
fn payloads() -> Vec<Value> {
    let n = |x: f64| Value::Number(x);
    let s = |x: &str| Value::String(x.into());
    vec![
        Value::Undefined,
        Value::Null,
        n(1.0),
        n(1.5),
        s("x"),
        Value::Bool(true),
        obj(&[("n", n(1.0))]),
        obj(&[("n", s("1"))]),
        obj(&[("n", n(1.5))]),
        obj(&[("n", n(1.0)), ("x", Value::Bool(true))]),
        obj(&[("s", s("a"))]),
        obj(&[("s", s("a")), ("b", Value::Bool(true))]),
        obj(&[("s", s("a")), ("b", n(2.0))]),
        Value::Array(vec![n(1.0), n(2.0)]),
        Value::Array(vec![n(1.0), s("x")]),
        obj(&[("tag", s("ok")), ("value", s("v"))]),
        obj(&[("tag", s("ok"))]),
        obj(&[("tag", s("bad"))]),
        obj(&[("n", n(1.0)), ("s", s("x"))]),
    ]
}

/// The chart with payload types declared for the picked event names (a `vt:types` block); the
/// declared type of each name, by name.
fn with_types(
    mut chart: Chart,
    picks: &[(prop::sample::Index, prop::sample::Index)],
) -> (Chart, Vec<(String, usize)>) {
    let names = event_names(&chart);
    let mut declared: Vec<(String, usize)> = Vec::new();
    for (k, (e, t)) in picks.iter().enumerate() {
        let name = if k == 0 {
            DECIDE_EVENTS[0].to_owned()
        } else {
            e.get(&names).clone()
        };
        if !declared.iter().any(|(n, _)| *n == name) {
            declared.push((name, t.index(TYPES.len())));
        }
    }
    if declared.is_empty() {
        return (chart, declared);
    }
    let mut text = String::from("type Pair = { n: int, s: string };\n");
    for (name, t) in &declared {
        let quoted = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
        match TYPES[*t].0 {
            Some(ty) => text.push_str(&format!("event {quoted}: {ty};\n")),
            None => text.push_str(&format!("event {quoted};\n")),
        }
    }
    let decls = parse_declarations(&text).expect("generated declarations parse");
    set_declarations(&mut chart, Some(&decls));
    (chart, declared)
}

fn states_of(states: &[State], out: &mut Vec<String>) {
    for s in states {
        if s.element == StateElement::State {
            out.push(s.id.clone());
        }
        states_of(&s.children, out);
    }
}

/// The events of the transitions added to the decide state; the first has a declared payload
/// type in every typed case.
const DECIDE_EVENTS: [&str; 2] = ["vt_d0", "vt_d1"];

fn bind_decide(states: &mut [State], id: &str) {
    for s in states {
        if s.id == id {
            s.extensions.push(Extension {
                ns: VT_NS.to_owned(),
                local: "decide".to_owned(),
                value: ExtensionValue::Attribute("guards/d".to_owned()),
                position: 0,
                span: Span::NONE,
            });
            // two choices of the decider's own, after the generated transitions: each
            // re-enters the decide state, so a taken one is offered again
            for e in DECIDE_EVENTS {
                s.transitions.push(Transition {
                    events: vec![e.to_owned()],
                    targets: vec![id.to_owned()],
                    ..Transition::default()
                });
            }
            return;
        }
        bind_decide(&mut s.children, id);
    }
}

/// The ordinary states the start macrostep enters by default (before eventless transitions):
/// the default initial states, recursively, all regions of a parallel state.
fn initial_path(states: &[State], initial: &[String], out: &mut Vec<String>) {
    let entered: Vec<&State> = if initial.is_empty() {
        states
            .iter()
            .filter(|s| !matches!(s.element, StateElement::History(_)))
            .take(1)
            .collect()
    } else {
        states.iter().filter(|s| initial.contains(&s.id)).collect()
    };
    for s in entered {
        if s.element == StateElement::State {
            out.push(s.id.clone());
        }
        if s.element == StateElement::Parallel {
            for c in &s.children {
                initial_path(std::slice::from_ref(c), &[], out);
            }
        } else {
            let init = match &s.initial_element {
                Some(i) => i.transition.targets.clone(),
                None => s.initial.clone(),
            };
            initial_path(&s.children, &init, out);
        }
    }
}

/// The chart with a decide binding on the picked state, if it has an ordinary state: mostly
/// one the start enters (so offers are open), sometimes any.
fn with_decide(mut chart: Chart, pick: &prop::sample::Index) -> Option<Chart> {
    let mut ids = Vec::new();
    initial_path(&chart.states, &chart.initial, &mut ids);
    if ids.is_empty() || pick.index(4) == 0 {
        ids.clear();
        states_of(&chart.states, &mut ids);
    }
    if ids.is_empty() {
        return None;
    }
    let id = pick.get(&ids).clone();
    bind_decide(&mut chart.states, &id);
    Some(chart)
}

/// A Rust run of a generated case.
struct Run {
    /// One observation line per decision, as `vertep-decide` reads them.
    lines: Vec<serde_json::Value>,
    /// The chart as the agent ran it.
    chart: Chart,
    /// Whether the chart is typed (declared payload types apply).
    typed: bool,
    /// The core macrosteps with their answer tape.
    core: CoreTape,
}

/// Run the moves on a machine that records its core macrosteps.
fn run(chart: Chart, declared: &[(String, usize)], moves: &[Move]) -> Option<Run> {
    let names = event_names(&chart);
    let pool = payloads();
    let chart = Arc::new(AgentChart::new(chart).ok()?);
    let typed = chart.schemas.typed();
    let registry: Arc<dyn DatamodelRegistry + Send + Sync> =
        Arc::new(vertep_conformance::HostRegistry);
    let (mut m, applied) = AgentMachine::start_with(
        ACTOR,
        chart.clone(),
        registry,
        T0,
        StartOptions {
            record_core: true,
            microstep_budget: Some(GENERATED_MICROSTEP_BUDGET),
        },
    )
    .ok()?;
    let mut seen: Vec<String> = Vec::new();
    let note = |effects: &[AgentEffect], seen: &mut Vec<String>| {
        for e in effects {
            if let AgentEffect::Offer(r) = e {
                seen.push(r.correlation.clone());
            }
        }
    };
    note(&applied.effects, &mut seen);
    let mut out = Vec::new();
    let mut last: Option<AgentInput> = None;
    for (k, mv) in moves.iter().enumerate() {
        let position = k as u64 + 2;
        let open = m.open_offers().into_iter().next();
        let input = match mv {
            Move::Event(i) => AgentInput::Event {
                event: Event::named(i.get(&names).clone(), EventType::External),
                message_id: format!("urn:vertep:test:m{k}"),
            },
            Move::Decide(c, i, e, p) => {
                let correlation = match (c, &open) {
                    (0..=2, Some(o)) => o.correlation.clone(),
                    (3, _) if !seen.is_empty() => i.get(&seen).clone(),
                    // a forged id, sometimes one forged before
                    _ if i.index(3) == 0 => "urn:vertep:test:forged".to_owned(),
                    _ => format!("urn:vertep:test:forged/{k}"),
                };
                let mut events: Vec<String> = open
                    .as_ref()
                    .map(|o| o.candidates.iter().map(|c| c.event.clone()).collect())
                    .unwrap_or_default();
                // pick 0 is a well-formed decision: an offered candidate when there is one,
                // with a payload of its declared type; pick 1 an offered candidate with any
                // payload
                if *c > 1 || events.is_empty() {
                    events.extend(names.iter().cloned());
                    events.push("delete_all".to_owned());
                }
                let event = e.get(&events).clone();
                let payload = match declared.iter().find(|(n, _)| *n == event) {
                    Some((_, t)) if *c == 0 => (TYPES[*t].1)(),
                    _ => p.get(&pool).clone(),
                };
                AgentInput::Decision {
                    correlation,
                    event,
                    payload,
                    annotations: Annotations::default(),
                }
            }
            Move::Redo => match &last {
                Some(d) => d.clone(),
                None => continue,
            },
        };
        let records = m.records().len();
        let (decision, answered) = match &input {
            AgentInput::Decision {
                correlation,
                event,
                payload,
                ..
            } => (
                Some((correlation.clone(), event.clone(), payload.clone())),
                m.answered(correlation),
            ),
            _ => (None, false),
        };
        let Ok(a) = m.apply(&input, position, T0 + 1_000 * position as i64) else {
            break;
        };
        note(&a.effects, &mut seen);
        let Some((corr, event, payload)) = decision else {
            continue;
        };
        last = Some(input);
        let (outcome, reason, taken) = if a.duplicate {
            ("duplicate", None, Vec::new())
        } else if let Some(d) = a.accepted.first() {
            ("taken", None, d.taken.clone())
        } else {
            let reason = m.records()[records..].iter().find_map(|r| match r {
                Record::Decision {
                    accepted: false,
                    reason,
                    ..
                } => reason.clone(),
                _ => None,
            });
            ("rejected", reason, Vec::new())
        };
        // the schema a candidate for this event carries
        let schema = open
            .as_ref()
            .and_then(|o| o.candidates.iter().find(|c| c.event == event))
            .map_or_else(|| chart.schemas.payload(&event), |c| c.schema.clone());
        out.push(serde_json::json!({
            "open": open.map(|o| serde_json::json!({
                "corr": o.correlation,
                "cands": o.candidates.iter().map(|c| serde_json::json!({
                    "event": c.event, "source": c.source, "index": c.index,
                })).collect::<Vec<_>>(),
            })),
            "answered": answered,
            "corr": corr,
            "event": event,
            "outcome": outcome,
            "reason": reason,
            "taken": taken.iter().map(|(s, i)| serde_json::json!([s, i])).collect::<Vec<_>>(),
            "schema": schema,
            "payload": vertep_agent::json::to_json(&payload),
        }));
    }
    let core = m.core_tape().cloned().unwrap_or_default();
    Some(Run {
        lines: out,
        chart: chart.ir.clone(),
        typed,
        core,
    })
}

/// Ask `vertep-decide`; `Err` with its reason on a disagreement.
fn judge(tools: &LeanTools, lines: &[serde_json::Value]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "vertep-decide-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("decisions.jsonl");
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&path, &text).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(&tools.decide)
        .arg("--obs")
        .arg(&path)
        .output()
        .map_err(|e| format!("{}: {e}", tools.decide.display()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    match out.status.code() {
        Some(0) => Ok(()),
        _ => Err(format!(
            "{}\n{text}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Replay the agent's core macrosteps through `vertep-oracle`; `Err` on a difference.
fn judge_core(tools: &LeanTools, run: &Run) -> Result<(), String> {
    let lines = replay(tools, &run.chart, &run.core.records)?;
    compare_run(&run.core.records, Some(run.core.budget_exceeded), &lines)
        .map_err(|d| format!("vertep-oracle: {d}"))
}

fn tools() -> Option<LeanTools> {
    let t = LeanTools::for_test(REQUIRED)?;
    if t.decide.is_file() {
        Some(t)
    } else if REQUIRED {
        panic!(
            "{} is not built: run `cd lean/vertep-lean && lake build vertep-decide`",
            t.decide.display()
        )
    } else {
        eprintln!("skipped: {} is not built", t.decide.display());
        None
    }
}

#[derive(Default)]
struct Counts {
    decisions: usize,
    taken: usize,
    payload_rejected: usize,
    typed: usize,
    macrosteps: usize,
}

fn check_decide(tools: &LeanTools, cases: u32) -> Counts {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases,
            failure_persistence: None,
            max_global_rejects: cases.saturating_mul(16).max(256),
            max_shrink_iters: 512,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[11u8; 32]),
    );
    let counts = std::cell::RefCell::new(Counts::default());
    let strategy = (
        arb_chart_with(chart_config()),
        any::<prop::sample::Index>(),
        prop::collection::vec(
            (any::<prop::sample::Index>(), any::<prop::sample::Index>()),
            0..4,
        ),
        prop::collection::vec(arb_move(), 1..20),
    );
    let result = runner.run(&strategy, |(chart, pick, types, moves)| {
        let Some(chart) = with_decide(chart, &pick) else {
            return Err(TestCaseError::reject("no ordinary state"));
        };
        let (chart, declared) = with_types(chart, &types);
        let Some(run) = run(chart, &declared, &moves) else {
            return Err(TestCaseError::reject("not an agent chart"));
        };
        {
            let mut c = counts.borrow_mut();
            c.decisions += run.lines.len();
            c.taken += run.lines.iter().filter(|l| l["outcome"] == "taken").count();
            c.payload_rejected += run
                .lines
                .iter()
                .filter(|l| l["reason"] == "payload-schema")
                .count();
            c.typed += usize::from(run.typed);
            c.macrosteps += run.core.records.len();
        }
        judge(tools, &run.lines).map_err(TestCaseError::fail)?;
        judge_core(tools, &run).map_err(TestCaseError::fail)
    });
    if let Err(e) = result {
        panic!("the agent machine disagrees with the Lean models: {e:?}");
    }
    let c = counts.into_inner();
    println!(
        "{} decisions compared with Lean ({} taken, {} rejected for their payload), {} typed charts, {} core macrosteps replayed",
        c.decisions, c.taken, c.payload_rejected, c.typed, c.macrosteps
    );
    assert!(c.taken > 0, "some decisions were accepted");
    assert!(c.macrosteps > 0, "some core macrosteps were replayed");
    c
}

/// Generated charts with a decide state, declared payload types and adversarial decider
/// outputs: every decision's outcome agrees with the Lean decider model, and the agent's core
/// macrosteps with the Lean oracle. `VT_DECIDE_CASES` sets the count.
// @tg: verifies:: [[openspec:deciders#Decider Safety#Arbitrary decider output is safe]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_decide_states_agree_with_lean() {
    let Some(tools) = tools() else { return };
    let cases = std::env::var("VT_DECIDE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    let c = check_decide(&tools, cases);
    assert!(c.typed > 0, "some generated charts were typed");
    assert!(
        c.payload_rejected > 0,
        "some decisions were rejected for their payload"
    );
}

/// A small sample of the same, with the gate.
#[test]
fn conformance_decide_smoke_sample() {
    let Some(tools) = tools() else { return };
    check_decide(&tools, 32);
}
