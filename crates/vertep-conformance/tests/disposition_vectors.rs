//! Conformance vectors for decision dispositions (m12-decision-records 2.3).
//!
//! `vectors/dispositions.json` holds charts and scripted runs: events, a recording decider's
//! answers and refusals, and clock advances, with the decision and refusal entries each run
//! must journal (their disposition and offer) and the configuration it must end in. Every
//! disposition the shell produces has a vector; `fallback-on-refusal` is listed as not
//! produced, with the run that shows it.
//!
//! The test runs each vector on the agent host and checks the journal against the vector. Then
//! it replays each journal into a fresh machine, entry by entry, and writes one observation line
//! per decision or refusal entry: the open offer before it (its candidates and whether it was
//! made to the fallback decider), the answer, what the machine's validation made of it, and the
//! disposition and offer the shell recorded on the entry. The lines must equal
//! `lean/vertep-lean/VertepTests/data/dispositions.jsonl` (rewrite it with `VT_BLESS=1`), which
//! the Lean test `VertepTests.Disposition` checks at build time against the model; and, when
//! the Lean build is present (always under the `differential` feature), `vertep-decide
//! --strict` must accept them and reject a copy with one disposition altered or removed.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value as Json, json};
use vertep_agent::decider::{
    Annotations, Decider, DeciderKind, Decision, DecisionOutcome, DecisionRequest, GuardDecider,
};
use vertep_agent::host::AgentHost;
use vertep_agent::journal::{AgentInput, JournalEntry};
use vertep_agent::machine::{AgentChart, AgentMachine};
use vertep_agent::{Record, json as agent_json};
use vertep_conformance::LeanTools;
use vertep_core::datamodel::{DatamodelRegistry, Event, EventType};
use vertep_core::ir::Chart;
use vertep_runtime::VirtualClock;

const REQUIRED: bool = cfg!(feature = "differential");

/// 2026-11-11T00:00:00Z.
const T0: i64 = 1_794_355_200_000;

const ACTOR: &str = "urn:vertep:test:dispositions";

const VECTORS: &str = include_str!("../vectors/dispositions.json");

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lean/vertep-lean/VertepTests/data/dispositions.jsonl")
}

/// A decider that records every request and never answers by itself.
#[derive(Clone, Default)]
struct Recording {
    requests: Arc<Mutex<Vec<DecisionRequest>>>,
}

impl Decider for Recording {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }

    fn decide(
        &self,
        request: DecisionRequest,
    ) -> Pin<Box<dyn Future<Output = DecisionOutcome> + Send>> {
        self.requests.lock().expect("requests").push(request);
        Box::pin(std::future::pending())
    }
}

fn registry() -> Arc<dyn DatamodelRegistry + Send + Sync> {
    Arc::new(vertep_conformance::HostRegistry)
}

fn read_chart(spec: &Json, extra: &str) -> Chart {
    let s = |k: &str| spec[k].as_str().unwrap_or_default();
    let xml = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="{}">{}{}</scxml>"#,
        s("initial"),
        s("head"),
        s("body").replace("{extra}", extra)
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts)
        .unwrap_or_else(|e| panic!("the vector chart reads: {e:?}"))
        .chart
}

/// The decision and refusal entries of a journal.
fn answers(entries: &[JournalEntry]) -> Vec<&JournalEntry> {
    entries
        .iter()
        .filter(|e| {
            matches!(
                e.input,
                AgentInput::Decision { .. } | AgentInput::Refusal { .. }
            )
        })
        .collect()
}

/// Run one vector on the host; the journal and the final configuration.
fn run_vector(v: &Json, charts: &Json) -> (Chart, Vec<JournalEntry>, Vec<String>) {
    let name = v["name"].as_str().unwrap_or_default();
    let chart = read_chart(
        &charts[v["chart"].as_str().unwrap_or_default()],
        v["extra"].as_str().unwrap_or_default(),
    );
    let clock = VirtualClock::new(T0);
    let rec = Recording::default();
    let mut b = AgentHost::builder(Arc::new(clock.clone()))
        .datamodels(registry())
        .decider("d", Arc::new(rec.clone()));
    if let Some(fb) = v["fallback"].as_array() {
        let events: Vec<String> = fb
            .iter()
            .filter_map(|e| e.as_str())
            .map(Into::into)
            .collect();
        b = b.decider("g", Arc::new(GuardDecider::first_enabled(events)));
    }
    let mut host = b.build();
    let version = host
        .register(chart.clone())
        .unwrap_or_else(|e| panic!("{name}: the chart is an agent chart: {e:?}"));
    host.spawn(ACTOR, &version)
        .unwrap_or_else(|e| panic!("{name}: the actor starts: {e:?}"));
    host.run_until_idle();
    for step in v["steps"].as_array().into_iter().flatten() {
        let correlation = |a: &Json| -> String {
            match a["correlation"].as_str() {
                Some(c) => c.to_owned(),
                None => {
                    let k = a["offer"].as_u64().unwrap_or(0) as usize;
                    rec.requests.lock().expect("requests")[k]
                        .correlation
                        .clone()
                }
            }
        };
        if let Some(e) = step["send"].as_str() {
            host.send(ACTOR, Event::named(e, EventType::External));
        } else if let Some(a) = step.get("answer") {
            let mut d = Decision::new(
                a["event"].as_str().unwrap_or_default(),
                agent_json::from_json(a.get("payload").unwrap_or(&Json::Null)),
            );
            if a.get("payload").is_none() {
                d.payload = vertep_core::datamodel::Value::Undefined;
            }
            if let Some(ann) = a.get("annotations") {
                d.annotations = serde_json::from_value::<Annotations>(ann.clone())
                    .unwrap_or_else(|e| panic!("{name}: annotations: {e}"));
            }
            host.deliver_outcome(ACTOR, &correlation(a), DecisionOutcome::Decision(d));
        } else if let Some(r) = step.get("refuse") {
            host.deliver_outcome(
                ACTOR,
                &correlation(r),
                DecisionOutcome::refuse(r["reason"].as_str().unwrap_or_default()),
            );
        } else if let Some(ms) = step["advance_ms"].as_u64() {
            clock.advance(Duration::from_millis(ms));
        } else {
            panic!("{name}: unknown step {step}");
        }
        host.run_until_idle();
    }
    let configuration = host
        .state(ACTOR)
        .unwrap_or_else(|e| panic!("{name}: the actor has a state: {e:?}"))
        .configuration;
    (chart, host.journal(ACTOR), configuration)
}

/// Replay a journal into a fresh machine and observe every decision and refusal entry, as
/// `vertep-decide` reads them.
fn observe(name: &str, chart: Chart, entries: &[JournalEntry]) -> Vec<Json> {
    let chart = Arc::new(AgentChart::new(chart).expect("an agent chart"));
    let first = entries.first().expect("a start entry");
    let (mut m, _) = AgentMachine::start(ACTOR, chart.clone(), registry(), first.ts)
        .unwrap_or_else(|e| panic!("{name}: replay starts: {e:?}"));
    let mut lines = Vec::new();
    for e in &entries[1..] {
        let open = m.open_offers().into_iter().next();
        let open_json = open.as_ref().map(|o| {
            json!({
                "corr": o.correlation,
                "cands": o.candidates.iter().map(|c| json!({
                    "event": c.event, "source": c.source, "index": c.index,
                })).collect::<Vec<_>>(),
                "fallback": o.fallback,
            })
        });
        let records = m.records().len();
        let answered = match &e.input {
            AgentInput::Decision { correlation, .. } | AgentInput::Refusal { correlation, .. } => {
                m.answered(correlation)
            }
            _ => false,
        };
        let applied = m
            .apply(&e.input, e.position, e.ts)
            .unwrap_or_else(|err| panic!("{name}: replay of entry {}: {err:?}", e.position));
        let recorded = |line: &mut Json| {
            line["disposition"] = json!(e.disposition.map(|d| d.as_str()));
            line["offered"] = json!(e.offered);
        };
        match &e.input {
            AgentInput::Decision {
                correlation,
                event,
                payload,
                annotations,
            } => {
                let (outcome, reason, taken) = if applied.duplicate {
                    ("duplicate", None, Vec::new())
                } else if let Some(d) = applied.accepted.first() {
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
                let schema = open
                    .as_ref()
                    .and_then(|o| o.candidates.iter().find(|c| c.event == *event))
                    .map_or_else(|| chart.schemas.payload(event), |c| c.schema.clone());
                let mut line = json!({
                    "vector": name,
                    "position": e.position,
                    "open": open_json,
                    "answered": answered,
                    "corr": correlation,
                    "event": event,
                    "outcome": outcome,
                    "reason": reason,
                    "taken": taken.iter().map(|(s, i)| json!([s, i])).collect::<Vec<_>>(),
                    "schema": schema,
                    "payload": agent_json::to_json(payload),
                    "annotations": serde_json::to_value(annotations).expect("annotations"),
                });
                recorded(&mut line);
                lines.push(line);
            }
            AgentInput::Refusal { correlation, .. } => {
                let mut line = json!({
                    "vector": name,
                    "position": e.position,
                    "kind": "refusal",
                    "open": open_json,
                    "corr": correlation,
                });
                recorded(&mut line);
                lines.push(line);
            }
            _ => {}
        }
    }
    lines
}

/// Run every vector, check the Rust shell against it, and return the observation lines.
fn run_vectors() -> Vec<Json> {
    let doc: Json = serde_json::from_str(VECTORS).expect("the vectors are JSON");
    let mut lines = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for v in doc["vectors"].as_array().expect("vectors") {
        let name = v["name"].as_str().expect("a vector name");
        let (chart, journal, configuration) = run_vector(v, &doc["charts"]);
        let got: Vec<Json> = answers(&journal)
            .iter()
            .map(|e| json!({"disposition": e.disposition_or_unknown(), "offered": e.offered}))
            .collect();
        assert_eq!(
            Json::Array(got),
            v["expect"]["entries"],
            "{name}: the journaled dispositions and offers"
        );
        assert_eq!(
            json!(configuration),
            v["expect"]["configuration"],
            "{name}: the configuration"
        );
        for e in answers(&journal) {
            seen.insert(e.disposition_or_unknown());
        }
        lines.extend(observe(name, chart, &journal));
    }
    // every disposition the shell produces has a vector; the one it does not is listed
    let not_produced: Vec<&str> = doc["not_produced"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["disposition"].as_str())
        .collect();
    for d in [
        "accepted",
        "refused-not-offered",
        "refused-guard",
        "refused-invalid",
        "stale",
        "fallback-on-deadline",
        "fallback-on-refusal",
        "refused-by-decider",
    ] {
        assert!(
            seen.contains(d) != not_produced.contains(&d),
            "{d}: a vector produces it, or it is listed as not produced, not both"
        );
    }
    lines
}

fn to_text(lines: &[Json]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// The vectors on the Rust shell: every vector's journal has the dispositions and offers it
/// lists, and its observation lines are the golden ones the Lean test replays.
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#An accepted answer is marked accepted]] {level: integration}
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A refused answer is marked refused]] {level: integration}
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A deadline fallback is marked]] {level: integration}
#[test]
fn disposition_vectors_on_the_rust_shell() {
    let lines = run_vectors();
    let text = to_text(&lines);
    let path = golden_path();
    if std::env::var_os("VT_BLESS").is_some() {
        std::fs::write(&path, &text).expect("write the golden lines");
        return;
    }
    // compared as JSON values, so the key order of the serializer does not matter
    let golden: Vec<Json> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a golden line is JSON"))
        .collect();
    assert!(
        golden == lines,
        "{} differs from the vectors' observations; rerun with VT_BLESS=1 if the change is intended\n{text}",
        path.display()
    );
}

/// Run `vertep-decide --obs FILE --strict` on `text`; its exit code and standard error.
fn decide_strict(tools: &LeanTools, text: &str, tag: &str) -> (Option<i32>, String) {
    let dir = std::env::temp_dir().join(format!(
        "vertep-dispositions-{}-{tag}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("dispositions.jsonl");
    std::fs::write(&path, text).expect("write observations");
    let out = std::process::Command::new(&tools.decide)
        .arg("--obs")
        .arg(&path)
        .arg("--strict")
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", tools.decide.display()));
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    )
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

fn check_strict(tools: &LeanTools) {
    let lines = run_vectors();
    let (code, err) = decide_strict(tools, &to_text(&lines), "ok");
    assert_eq!(code, Some(0), "vertep-decide --strict: {err}");
    // a recorded disposition the model does not compute fails
    let mut altered = lines.clone();
    let k = altered
        .iter()
        .position(|l| l["disposition"] == "refused-guard")
        .expect("a refused-guard line");
    altered[k]["disposition"] = json!("accepted");
    let (code, err) = decide_strict(tools, &to_text(&altered), "altered");
    assert_eq!(code, Some(1), "an altered disposition is rejected: {err}");
    assert!(err.contains("the model computes refused-guard"), "{err}");
    // so does an entry without one, in strict mode
    let mut missing = lines;
    missing[0]["disposition"] = Json::Null;
    missing[0]["offered"] = Json::Null;
    let (code, err) = decide_strict(tools, &to_text(&missing), "missing");
    assert_eq!(code, Some(1), "a missing disposition is rejected: {err}");
}

/// The vectors' journals replayed through the Lean model in strict mode, with the Lean build
/// (skipped when it is not built).
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#An accepted answer is marked accepted]] {level: integration}
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A refused answer is marked refused]] {level: integration}
// @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A deadline fallback is marked]] {level: integration}
#[test]
fn conformance_disposition_vectors_strict_lean() {
    let Some(tools) = tools() else { return };
    check_strict(&tools);
}

/// The same, required: the nightly differential job fails instead of skipping.
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_disposition_vectors_strict_lean() {
    let Some(tools) = tools() else { return };
    check_strict(&tools);
}
