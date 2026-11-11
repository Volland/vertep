//! Differential tests of decide states with adversarial decider outputs (m3-agents 8.5).
//!
//! Generated well-formed charts (the differential generator of `vertep-testkit`) get a
//! `vt:decide` binding on one of their states; the Rust agent machine (`vertep-agent`) then
//! receives a generated mix of events and decisions: decisions for the open offer, stale and
//! forged correlation ids, unknown events, events that were not offered, and redeliveries.
//! Every decision's outcome is replayed through the Lean decider model by `vertep-decide`,
//! which must reach the same outcome (taken, rejected with the same reason, or dropped as a
//! duplicate) and, for a taken decision, a transition Rust took that was a candidate of the
//! open offer, so enabled at offer time (`Vertep.Agent.decider_safety`).

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use vertep_agent::{AgentChart, AgentEffect, AgentInput, AgentMachine, Annotations, Record};
use vertep_conformance::LeanTools;
use vertep_conformance::differential::{chart_config, event_names};
use vertep_core::datamodel::{DatamodelRegistry, Event, EventType, Value};
use vertep_core::ir::{Chart, Extension, ExtensionValue, Span, State, StateElement, VT_NS};
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
    /// A decision: correlation pick (0 and 1 the open offer, 2 an earlier one, 3 forged) and
    /// event pick (pick 0: an offered candidate; otherwise among the candidates, the chart's
    /// event names and an unknown name).
    Decide(u8, prop::sample::Index, prop::sample::Index),
    /// The previous decision again.
    Redo,
}

fn arb_move() -> impl Strategy<Value = Move> {
    prop_oneof![
        3 => any::<prop::sample::Index>().prop_map(Move::Event),
        6 => (0..4u8, any::<prop::sample::Index>(), any::<prop::sample::Index>())
            .prop_map(|(c, i, e)| Move::Decide(c, i, e)),
        1 => Just(Move::Redo),
    ]
}

fn states_of(states: &[State], out: &mut Vec<String>) {
    for s in states {
        if s.element == StateElement::State {
            out.push(s.id.clone());
        }
        states_of(&s.children, out);
    }
}

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
            return;
        }
        bind_decide(&mut s.children, id);
    }
}

/// The chart with a decide binding on the picked state, if it has an ordinary state.
fn with_decide(mut chart: Chart, pick: &prop::sample::Index) -> Option<Chart> {
    let mut ids = Vec::new();
    states_of(&chart.states, &mut ids);
    if ids.is_empty() {
        return None;
    }
    let id = pick.get(&ids).clone();
    bind_decide(&mut chart.states, &id);
    Some(chart)
}

/// Run the moves; one observation line per decision, as `vertep-decide` reads them.
fn run(chart: Chart, moves: &[Move]) -> Option<Vec<serde_json::Value>> {
    let names = event_names(&chart);
    let chart = Arc::new(AgentChart::new(chart).ok()?);
    let registry: Arc<dyn DatamodelRegistry + Send + Sync> =
        Arc::new(vertep_conformance::HostRegistry);
    let (mut m, applied) = AgentMachine::start(ACTOR, chart, registry, T0).ok()?;
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
            Move::Decide(c, i, e) => {
                let correlation = match (c, &open) {
                    (0 | 1, Some(o)) => o.correlation.clone(),
                    (2, _) if !seen.is_empty() => i.get(&seen).clone(),
                    _ => "urn:vertep:test:forged".to_owned(),
                };
                let mut events: Vec<String> = open
                    .as_ref()
                    .map(|o| o.candidates.iter().map(|c| c.event.clone()).collect())
                    .unwrap_or_default();
                // pick 0 is a well-formed decision: an offered candidate when there is one
                if *c != 0 || events.is_empty() {
                    events.extend(names.iter().cloned());
                    events.push("delete_all".to_owned());
                }
                AgentInput::Decision {
                    correlation,
                    event: e.get(&events).clone(),
                    payload: Value::Undefined,
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
                correlation, event, ..
            } => (
                Some((correlation.clone(), event.clone())),
                m.answered(correlation),
            ),
            _ => (None, false),
        };
        let Ok(a) = m.apply(&input, position, T0 + 1_000 * position as i64) else {
            break;
        };
        note(&a.effects, &mut seen);
        let Some((corr, event)) = decision else {
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
        }));
    }
    Some(out)
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

fn check_decide(tools: &LeanTools, cases: u32) {
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
    let decisions = std::cell::Cell::new(0usize);
    let taken = std::cell::Cell::new(0usize);
    let strategy = (
        arb_chart_with(chart_config()),
        any::<prop::sample::Index>(),
        prop::collection::vec(arb_move(), 1..20),
    );
    let result = runner.run(&strategy, |(chart, pick, moves)| {
        let Some(chart) = with_decide(chart, &pick) else {
            return Err(TestCaseError::reject("no ordinary state"));
        };
        let Some(lines) = run(chart, &moves) else {
            return Err(TestCaseError::reject("not an agent chart"));
        };
        decisions.set(decisions.get() + lines.len());
        taken.set(taken.get() + lines.iter().filter(|l| l["outcome"] == "taken").count());
        judge(tools, &lines).map_err(TestCaseError::fail)
    });
    if let Err(e) = result {
        panic!("the agent machine disagrees with the Lean decider model: {e:?}");
    }
    println!(
        "{} decisions compared with Lean, {} taken",
        decisions.get(),
        taken.get()
    );
    assert!(taken.get() > 0, "some decisions were accepted");
}

/// Generated charts with a decide state and adversarial decider outputs: every decision's
/// outcome agrees with the Lean decider model. `VT_DECIDE_CASES` sets the count.
// @tg: verifies:: [[openspec:deciders#Decider Safety#Arbitrary decider output is safe]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_decide_states_agree_with_lean() {
    let Some(tools) = tools() else { return };
    let cases = std::env::var("VT_DECIDE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    check_decide(&tools, cases);
}

/// A small sample of the same, with the gate.
#[test]
fn conformance_decide_smoke_sample() {
    let Some(tools) = tools() else { return };
    check_decide(&tools, 16);
}
