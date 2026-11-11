//! Randomized differential testing of the Rust core against the Lean oracle (task 8.9).
//!
//! A [`GeneratedCase`] is a well-formed chart from `vertep-testkit` and a sequence of inputs
//! drawn from the event names its transitions listen for. The Rust core runs it directly (no
//! host: effects are recorded, not performed) with the datamodel answer tape recorded; a
//! [`TraceOracle`] (the Lean oracle) replays the records and must produce the same traces.
//! A disagreement is shrunk by proptest to a minimal chart and input sequence and stored as a
//! JSON file in the regression corpus (`regressions/`), which [`load_regressions`] replays on
//! every run.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestError, TestRng, TestRunner};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Environment, Event, EventType, SessionInfo, Value};
use vertep_core::ir::{Chart, State};
use vertep_core::null::create_datamodel;
use vertep_core::session::{self, Input, InputKind, Options, StartError, StartInput, StepError};
use vertep_core::tape::{OracleInput, OracleRecord, TapeDatamodel};
use vertep_testkit::{ChartConfig, arb_chart_with};

use crate::lean::{LeanTools, compare_run, replay};

/// One input of a generated case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenInput {
    /// An external event with this name.
    Event(String),
    /// A cancel request.
    Cancel,
}

/// A generated chart with its inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedCase {
    /// The chart.
    pub chart: Chart,
    /// The inputs after the start macrostep.
    pub inputs: Vec<GenInput>,
}

/// The microstep budget of generated runs: small, so eventless loops end quickly.
pub const GENERATED_MICROSTEP_BUDGET: u32 = 200;

/// The chart limits used for differential runs: invokes and extensions included, small sizes
/// so shrinking stays fast.
pub fn chart_config() -> ChartConfig {
    ChartConfig {
        max_depth: 3,
        max_top_states: 3,
        max_children: 3,
        max_transitions: 2,
        max_targets: 1,
        max_block: 2,
        exec_depth: 1,
        ..ChartConfig::default()
    }
}

/// Event names that some transition of the chart listens for (`a.b.*` gives `a.b`), plus one
/// name nothing listens for.
pub fn event_names(chart: &Chart) -> Vec<String> {
    fn walk(states: &[State], out: &mut Vec<String>) {
        for s in states {
            for t in &s.transitions {
                for d in &t.events {
                    let name = match d.as_str() {
                        "*" => "any".to_string(),
                        d => d.trim_end_matches(".*").trim_end_matches('.').to_string(),
                    };
                    if !name.is_empty() && !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
            walk(&s.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&chart.states, &mut out);
    out.push("unheard".to_string());
    out
}

/// Generated cases: a well-formed chart and up to `max_inputs` inputs, mostly events the
/// chart listens for, sometimes a cancel.
// @tg: implements:: [[openspec:scxml-conformance#Randomized Differential Testing]] {note: "chart and input generators"}
pub fn arb_case(cfg: ChartConfig, max_inputs: usize) -> BoxedStrategy<GeneratedCase> {
    // Inputs are picked as indices into the chart's event names rather than generated from
    // the chart, so that chart and inputs shrink independently.
    let pick = prop_oneof![
        12 => any::<proptest::sample::Index>().prop_map(Some),
        1 => Just(None),
    ];
    (
        arb_chart_with(cfg),
        proptest::collection::vec(pick, 0..=max_inputs),
    )
        .prop_map(|(chart, picks)| {
            let names = event_names(&chart);
            let inputs = picks
                .into_iter()
                .map(|p| match p {
                    Some(i) => GenInput::Event(i.get(&names).clone()),
                    None => GenInput::Cancel,
                })
                .collect();
            GeneratedCase { chart, inputs }
        })
        .boxed()
}

/// A Rust run of a generated case.
#[derive(Clone, Debug)]
pub struct RustRun {
    /// One record per macrostep, with the answer tape.
    pub records: Vec<OracleRecord>,
    /// Whether the last macrostep ran out of its microstep budget (its trace is partial).
    pub budget_exceeded: bool,
}

/// Run a generated case on the Rust core and return its records (with the answer tape), or
/// `Err` with why the case cannot run (the chart is not well-formed).
pub fn rust_records(case: &GeneratedCase) -> Result<RustRun, String> {
    let chart =
        CompiledChart::new(case.chart.clone()).map_err(|v| format!("not well-formed: {v:?}"))?;
    let registry = crate::host::HostRegistry;
    let mut dm = create_datamodel(&registry, chart.datamodel()).map_err(|e| format!("{e:?}"))?;
    let env = |i: u64| Environment {
        now_ms: 1_767_225_600_000 + 1_000 * i64::try_from(i).unwrap_or(0),
        seed: i,
    };
    let start = StartInput {
        info: SessionInfo {
            sessionid: "vt0".to_string(),
            name: chart.chart().name.clone(),
            ioprocessors: Value::Undefined,
        },
        env: env(0),
        options: Options {
            microstep_budget: GENERATED_MICROSTEP_BUDGET,
            ..Options::default()
        },
        data: Vec::new(),
        sessions: Some(vec!["vt0".to_string()]),
    };
    let mut records = Vec::new();
    let mut tape = TapeDatamodel::new(dm.as_mut());
    let mut session = match session::start(&chart, start.clone(), &mut tape) {
        Ok(o) => {
            records.push(OracleRecord {
                input: OracleInput::Start(start),
                tape: tape.take(),
                trace: o.trace,
            });
            o.session
        }
        Err(StartError::BudgetExceeded { partial, .. }) => {
            records.push(OracleRecord {
                input: OracleInput::Start(start),
                tape: tape.take(),
                trace: partial.trace,
            });
            return Ok(RustRun {
                records,
                budget_exceeded: true,
            });
        }
        Err(e) => return Err(format!("start: {e:?}")),
    };
    for (i, gi) in case.inputs.iter().enumerate() {
        if session.is_terminated() {
            break;
        }
        let kind = match gi {
            GenInput::Event(n) => InputKind::Event(Event::named(n.clone(), EventType::External)),
            GenInput::Cancel => InputKind::Cancel,
        };
        let input = Input {
            env: env(i as u64 + 1),
            kind,
            sessions: Some(vec!["vt0".to_string()]),
        };
        match session::step(&chart, session, input.clone(), &mut tape) {
            Ok(o) => {
                records.push(OracleRecord {
                    input: OracleInput::Step(input),
                    tape: tape.take(),
                    trace: o.trace,
                });
                session = o.session;
            }
            Err(StepError::BudgetExceeded { partial, .. }) => {
                records.push(OracleRecord {
                    input: OracleInput::Step(input),
                    tape: tape.take(),
                    trace: partial.trace,
                });
                return Ok(RustRun {
                    records,
                    budget_exceeded: true,
                });
            }
            Err(StepError::Terminated) => break,
        }
    }
    Ok(RustRun {
        records,
        budget_exceeded: false,
    })
}

/// Checks a Rust run of a generated case against a second implementation.
pub trait TraceOracle {
    /// `Ok` when the second implementation reproduces the run; otherwise what differs.
    fn check(&self, case: &GeneratedCase, run: &RustRun) -> Result<(), String>;
}

impl TraceOracle for LeanTools {
    fn check(&self, case: &GeneratedCase, run: &RustRun) -> Result<(), String> {
        // A run that ran out of the microstep budget agrees when the oracle reports the budget
        // error on the same macrostep with the same partial trace.
        let lines = replay(self, &case.chart, &run.records)?;
        compare_run(&run.records, Some(run.budget_exceeded), &lines).map_err(|d| d.to_string())
    }
}

/// How a differential run went.
#[derive(Clone, Debug, Default)]
pub struct DifferentialOutcome {
    /// Cases compared (including shrinking steps).
    pub compared: u64,
    /// Generated charts the Rust validator rejected.
    pub skipped: u64,
    /// The shrunk counterexample, if the two disagreed, with the file it was stored in.
    pub counterexample: Option<Counterexample>,
    /// Why the run was aborted (too many skipped cases), if it was.
    pub aborted: Option<String>,
}

/// A minimal disagreement.
#[derive(Clone, Debug)]
pub struct Counterexample {
    /// The shrunk case.
    pub case: GeneratedCase,
    /// What differed.
    pub reason: String,
    /// The regression file it was stored in.
    pub stored: PathBuf,
}

/// Settings of a differential run.
#[derive(Clone, Debug)]
pub struct DifferentialConfig {
    /// Generated cases to compare.
    pub cases: u32,
    /// Seed of the generator (runs are reproducible).
    pub seed: u64,
    /// Chart limits.
    pub chart: ChartConfig,
    /// Most inputs per case.
    pub max_inputs: usize,
    /// Where counterexamples are stored.
    pub regressions: PathBuf,
}

impl Default for DifferentialConfig {
    fn default() -> Self {
        DifferentialConfig {
            cases: 256,
            seed: 0x7665_7274_6570,
            chart: chart_config(),
            max_inputs: 6,
            regressions: default_regressions_dir(),
        }
    }
}

/// The committed regression corpus.
pub fn default_regressions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions")
}

/// Generate cases, compare the Rust core with `oracle` on each, shrink the first
/// disagreement and store it ([[openspec:scxml-conformance#Randomized Differential Testing]]).
// @tg: implements:: [[openspec:scxml-conformance#Randomized Differential Testing]]
pub fn run_generated(cfg: &DifferentialConfig, oracle: &dyn TraceOracle) -> DifferentialOutcome {
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&cfg.seed.to_le_bytes());
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: cfg.cases,
            max_global_rejects: cfg.cases.saturating_mul(16).max(256),
            failure_persistence: None,
            max_shrink_iters: 2_048,
            // A non-terminating oracle costs its whole timeout per shrinking step.
            max_shrink_time: 120_000,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &seed),
    );
    let compared = Cell::new(0u64);
    let skipped = Cell::new(0u64);
    let strategy = arb_case(cfg.chart.clone(), cfg.max_inputs);
    let result = runner.run(&strategy, |case| {
        let run = match rust_records(&case) {
            Ok(r) => r,
            Err(why) => {
                skipped.set(skipped.get() + 1);
                return Err(TestCaseError::reject(why));
            }
        };
        compared.set(compared.get() + 1);
        oracle.check(&case, &run).map_err(TestCaseError::fail)
    });
    let mut out = DifferentialOutcome {
        compared: compared.get(),
        skipped: skipped.get(),
        ..DifferentialOutcome::default()
    };
    match result {
        Ok(()) => {}
        Err(TestError::Abort(r)) => out.aborted = Some(r.to_string()),
        Err(TestError::Fail(reason, case)) => {
            let reason = reason.to_string();
            let stored = store_regression(&cfg.regressions, &case, &reason)
                .unwrap_or_else(|e| PathBuf::from(format!("<not stored: {e}>")));
            out.counterexample = Some(Counterexample {
                case,
                reason,
                stored,
            });
        }
    }
    out
}

/// A stored counterexample.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Regression {
    /// What differed when it was found.
    pub reason: String,
    /// The chart as SCXML, for reading (the `chart` field is authoritative).
    pub scxml: Option<String>,
    /// The case.
    pub case: GeneratedCase,
}

/// Store a counterexample as `<dir>/<hash>.json`; returns the path.
pub fn store_regression(
    dir: &Path,
    case: &GeneratedCase,
    reason: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec(case).map_err(std::io::Error::other)?;
    let hash: String = Sha256::digest(&body)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let path = dir.join(format!("{hash}.json"));
    let r = Regression {
        reason: reason.to_string(),
        scxml: vertep_scxml::write(&case.chart).ok(),
        case: case.clone(),
    };
    let text = serde_json::to_string_pretty(&r).map_err(std::io::Error::other)?;
    std::fs::write(&path, text + "\n")?;
    Ok(path)
}

/// Every stored counterexample under `dir`, sorted by file name.
pub fn load_regressions(dir: &Path) -> Result<Vec<(PathBuf, Regression)>, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let r: Regression =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            Ok((p, r))
        })
        .collect()
}

/// Replay every stored counterexample; returns the ones that still disagree.
pub fn replay_regressions(dir: &Path, oracle: &dyn TraceOracle) -> Result<Vec<String>, String> {
    let mut failing = Vec::new();
    for (path, r) in load_regressions(dir)? {
        let run = rust_records(&r.case)?;
        if let Err(e) = oracle.check(&r.case, &run) {
            failing.push(format!("{}: {e}", path.display()));
        }
    }
    Ok(failing)
}

// ----- Rust validation against vt-certify ---------------------------------------------------

/// What `vt-certify` said about a chart projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Certification {
    /// Exit 0: well-formed, certificate issued.
    Certified,
    /// Exit 1: not well-formed; the violations it printed.
    NotWellFormed(String),
    /// Exit 2: the input is not a chart projection.
    NotAProjection(String),
}

/// Run `vt-certify` on the chart's projection.
pub fn certify(tools: &LeanTools, chart: &Chart) -> Result<Certification, String> {
    let dir = std::env::temp_dir().join(format!(
        "vertep-certify-{}-{:x}",
        std::process::id(),
        Sha256::digest(crate::lean::chart_nquads(chart).as_bytes())[..8]
            .iter()
            .fold(0u64, |a, b| (a << 8) | u64::from(*b))
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("chart.nq");
    std::fs::write(&path, crate::lean::chart_nquads(chart)).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(&tools.certify)
        .arg("--chart")
        .arg(&path)
        .arg("--out")
        .arg(dir.join("cert.nt"))
        .arg("--checker-hash")
        .arg("00")
        .output()
        .map_err(|e| format!("{}: {e}", tools.certify.display()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    match out.status.code() {
        Some(0) => Ok(Certification::Certified),
        Some(1) => Ok(Certification::NotWellFormed(stderr)),
        Some(2) => Ok(Certification::NotAProjection(stderr)),
        other => Err(format!("vt-certify exited with {other:?}: {stderr}")),
    }
}

/// The ways [`mutate`] breaks a well-formed chart, each aimed at one validation rule.
pub const MUTATIONS: [&str; 9] = [
    "unknown transition target",
    "history state at the root",
    "final state with a transition",
    "initial target outside the state",
    "history default cycle",
    "target set of two siblings",
    "history target with its sibling",
    "duplicate data id",
    "send with event and eventexpr",
];

fn atomic(id: &str) -> State {
    State::new(id, vertep_core::ir::StateElement::State)
}

fn compound(id: &str, children: Vec<State>) -> State {
    let mut s = atomic(id);
    s.children = children;
    s
}

fn to(targets: &[&str]) -> vertep_core::ir::Transition {
    vertep_core::ir::Transition {
        events: vec!["go".to_string()],
        targets: targets.iter().map(|t| (*t).to_string()).collect(),
        ..vertep_core::ir::Transition::default()
    }
}

fn history(id: &str, default: Option<&str>) -> State {
    let mut h = State::new(
        id,
        vertep_core::ir::StateElement::History(vertep_core::ir::HistoryType::Shallow),
    );
    if let Some(d) = default {
        let mut t = to(&[d]);
        t.events.clear();
        h.transitions.push(t);
    }
    h
}

/// Apply mutation `which` (an index into [`MUTATIONS`]) to a chart.
pub fn mutate(mut chart: Chart, which: usize) -> Chart {
    let first = chart
        .states
        .first()
        .map(|s| s.id.clone())
        .unwrap_or_default();
    match which % MUTATIONS.len() {
        0 => chart.states.push({
            let mut s = atomic("m_src");
            s.transitions.push(to(&["m_nowhere"]));
            s
        }),
        1 => chart.states.push(history("m_hist", Some(&first))),
        2 => chart.states.push({
            let mut f = State::new("m_final", vertep_core::ir::StateElement::Final);
            f.transitions.push(to(&[&first]));
            f
        }),
        3 => chart.states.push({
            let mut c = compound("m_comp", vec![atomic("m_comp1")]);
            c.initial = vec![first];
            c
        }),
        4 => chart.states.push({
            let mut c = compound("m_cyc", vec![atomic("m_cyc1"), history("m_cyc_h", None)]);
            c.initial = vec!["m_cyc_h".to_string()];
            c
        }),
        5 => chart.states.push({
            let mut c = compound("m_pair", vec![atomic("m_pair_a"), atomic("m_pair_b")]);
            c.transitions.push(to(&["m_pair_a", "m_pair_b"]));
            c
        }),
        6 => chart.states.push({
            let mut c = compound(
                "m_hs",
                vec![
                    atomic("m_hs1"),
                    atomic("m_hs2"),
                    history("m_hs_h", Some("m_hs1")),
                ],
            );
            c.transitions.push(to(&["m_hs_h", "m_hs2"]));
            c
        }),
        7 => {
            for _ in 0..2 {
                chart.data.push(vertep_core::ir::Data {
                    id: "m_dup".to_string(),
                    src: None,
                    expr: Some("1".to_string()),
                    content: None,
                    extensions: Vec::new(),
                    span: vertep_core::ir::Span::NONE,
                });
            }
        }
        _ => chart.states.push({
            let mut s = atomic("m_send");
            s.onentry.push(vec![vertep_core::ir::Exec::new(
                vertep_core::ir::ExecKind::Send(Box::new(vertep_core::ir::Send {
                    event: Some("e".to_string()),
                    eventexpr: Some("'e'".to_string()),
                    ..vertep_core::ir::Send::default()
                })),
            )]);
            s
        }),
    }
    chart
}

/// `n` generated charts for the validation comparison, deterministically from `seed`: even
/// samples as generated (well-formed), odd samples broken by the mutations of [`MUTATIONS`] in
/// turn (the index is returned with them).
pub fn sample_validation_charts(n: usize, seed: u64) -> Vec<(Chart, Option<usize>)> {
    use proptest::strategy::ValueTree;
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &bytes),
    );
    let strategy = arb_chart_with(chart_config());
    (0..n)
        .filter_map(|i| {
            let chart = strategy.new_tree(&mut runner).ok()?.current();
            Some(if i % 2 == 0 {
                (chart, None)
            } else {
                let m = (i / 2) % MUTATIONS.len();
                (mutate(chart, m), Some(m))
            })
        })
        .collect()
}
