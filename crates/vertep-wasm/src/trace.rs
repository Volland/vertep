//! Step traces: one chart stepped over a sequence of journaled inputs with the pure core,
//! recording the configuration, the datamodel and the effects after every macrostep. The same
//! function runs natively and in WebAssembly, so the two traces can be compared byte for byte.

use serde::{Deserialize, Serialize};
use serde_json::Value as J;
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{DatamodelRegistry, Environment, Event, EventType, SessionInfo};
use vertep_core::session::{self, Input, Options, StartInput};

use crate::charts::{ChartFormat, read_chart};
use crate::functions::json_to_value;

/// One journaled input: an event with the time and seed the journal recorded for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceInput {
    /// The event name.
    pub name: String,
    /// The payload.
    #[serde(default)]
    pub data: J,
    /// The journaled timestamp.
    #[serde(default)]
    pub now_ms: i64,
    /// The journaled seed.
    #[serde(default)]
    pub seed: u64,
}

/// A chart and its inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCase {
    /// A name for reports.
    pub name: String,
    /// The chart source.
    pub chart: String,
    /// Its format (`scxml`, `yaml`, `rdf`); sniffed when absent.
    #[serde(default)]
    pub format: Option<String>,
    /// The journaled time of the start macrostep.
    #[serde(default)]
    pub start_ms: i64,
    /// The inputs, in journal order.
    pub inputs: Vec<TraceInput>,
}

/// The state after one macrostep.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    /// Active state ids in document order.
    pub configuration: Vec<String>,
    /// The datamodel snapshot, in the runtime's lossless JSON encoding.
    pub datamodel: J,
    /// The effects of the macrostep, in order (the core's serde encoding).
    pub effects: J,
    /// Whether the session terminated.
    pub terminated: bool,
}

/// The trace of one case: the start macrostep, then one step per input; or the error that
/// stopped it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// The case name.
    pub name: String,
    /// The chart version IRI.
    pub version: String,
    /// The macrosteps.
    pub steps: Vec<TraceStep>,
    /// Why stepping stopped early, if it did.
    pub error: Option<String>,
}

/// Steps `case` with the datamodels of `registry`.
// @tg: implements:: [[openspec:wasm-bindings#Core Builds For WebAssembly Unchanged]]
pub fn trace(case: &TraceCase, registry: &dyn DatamodelRegistry) -> Trace {
    let mut out = Trace {
        name: case.name.clone(),
        version: String::new(),
        steps: Vec::new(),
        error: None,
    };
    let format = match &case.format {
        Some(f) => match ChartFormat::parse(f) {
            Ok(f) => f,
            Err(e) => {
                out.error = Some(e.to_string());
                return out;
            }
        },
        None => ChartFormat::sniff(&case.chart),
    };
    let chart = match read_chart(&case.chart, format) {
        Ok(c) => c,
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
    };
    out.version = crate::charts::version_of(&chart);
    let compiled = match CompiledChart::new(chart) {
        Ok(c) => c,
        Err(v) => {
            out.error = Some(format!("{v:?}"));
            return out;
        }
    };
    let mut dm = match vertep_core::null::create_datamodel(registry, compiled.datamodel()) {
        Ok(d) => d,
        Err(e) => {
            out.error = Some(format!("{e:?}"));
            return out;
        }
    };
    let start = StartInput {
        info: SessionInfo {
            sessionid: format!("urn:vertep:trace:{}", case.name),
            name: None,
            ioprocessors: vertep_core::datamodel::Value::Undefined,
        },
        env: Environment {
            now_ms: case.start_ms,
            seed: 0,
        },
        options: Options::default(),
        data: Vec::new(),
        sessions: None,
    };
    let mut s = match session::start(&compiled, start, dm.as_mut()) {
        Ok(o) => {
            out.steps.push(step_of(&compiled, &o, dm.as_ref()));
            o.session
        }
        Err(e) => {
            out.error = Some(format!("{e:?}"));
            return out;
        }
    };
    for i in &case.inputs {
        let ev = Event {
            data: json_to_value(&i.data),
            ..Event::named(i.name.clone(), EventType::External)
        };
        let env = Environment {
            now_ms: i.now_ms,
            seed: i.seed,
        };
        match session::step(&compiled, s.clone(), Input::event(ev, env), dm.as_mut()) {
            Ok(o) => {
                out.steps.push(step_of(&compiled, &o, dm.as_ref()));
                s = o.session;
            }
            Err(e) => {
                out.error = Some(format!("{e:?}"));
                break;
            }
        }
    }
    out
}

fn step_of(
    chart: &CompiledChart,
    o: &session::Outcome,
    dm: &dyn vertep_core::datamodel::Datamodel,
) -> TraceStep {
    TraceStep {
        configuration: o.session.active(chart),
        datamodel: serde_json::from_str(&vertep_runtime::json::to_text(&dm.snapshot()))
            .unwrap_or(J::Null),
        effects: serde_json::to_value(&o.effects).unwrap_or(J::Null),
        terminated: o.session.is_terminated(),
    }
}

/// Traces every case and returns them as one JSON array text.
pub fn trace_all(cases: &[TraceCase], registry: &dyn DatamodelRegistry) -> String {
    let traces: Vec<Trace> = cases.iter().map(|c| trace(c, registry)).collect();
    serde_json::to_string(&traces).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

// ---- the Lean differential traces ---------------------------------------------------------

/// One input of a generated differential case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffInput {
    /// An external event with this name.
    Event(String),
    /// A cancel request.
    Cancel,
}

/// A generated chart (as IR) with its inputs: the cases the Rust-versus-Lean differential
/// test generates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffCase {
    /// The chart.
    pub chart: vertep_core::ir::Chart,
    /// The inputs after the start macrostep.
    pub inputs: Vec<DiffInput>,
}

/// The microstep budget of generated runs, as in the native differential test.
pub const DIFF_MICROSTEP_BUDGET: u32 = 200;

/// The run of one case: the oracle records (input, answer tape, macrostep trace) the Lean
/// model replays, whether the last macrostep ran out of its budget, or why it cannot run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffRun {
    /// One record per macrostep.
    pub records: Vec<vertep_core::tape::OracleRecord>,
    /// Whether the last macrostep's trace is partial.
    pub budget_exceeded: bool,
    /// Why the case could not run.
    pub error: Option<String>,
}

/// Runs a generated case exactly as the native differential test does before it hands the
/// records to the Lean model: the same session, clock, seeds, sessions list and budget.
// @tg: implements:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly]] {confidence: partial, note: "the differential traces; the W3C suite runner is not ported"}
pub fn diff_records(case: &DiffCase, registry: &dyn DatamodelRegistry) -> DiffRun {
    use vertep_core::session::{InputKind, StartError, StepError};
    use vertep_core::tape::{OracleInput, OracleRecord, TapeDatamodel};
    let fail = |e: String| DiffRun {
        records: Vec::new(),
        budget_exceeded: false,
        error: Some(e),
    };
    let chart = match CompiledChart::new(case.chart.clone()) {
        Ok(c) => c,
        Err(v) => return fail(format!("not well-formed: {v:?}")),
    };
    let mut dm = match vertep_core::null::create_datamodel(registry, chart.datamodel()) {
        Ok(d) => d,
        Err(e) => return fail(format!("{e:?}")),
    };
    let env = |i: u64| Environment {
        now_ms: 1_767_225_600_000 + 1_000 * i64::try_from(i).unwrap_or(0),
        seed: i,
    };
    let start = StartInput {
        info: SessionInfo {
            sessionid: "vt0".to_string(),
            name: chart.chart().name.clone(),
            ioprocessors: vertep_core::datamodel::Value::Undefined,
        },
        env: env(0),
        options: Options {
            microstep_budget: DIFF_MICROSTEP_BUDGET,
            ..Options::default()
        },
        data: Vec::new(),
        sessions: Some(vec!["vt0".to_string()]),
    };
    let mut records = Vec::new();
    let mut tape = TapeDatamodel::new(dm.as_mut());
    let mut s = match session::start(&chart, start.clone(), &mut tape) {
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
            return DiffRun {
                records,
                budget_exceeded: true,
                error: None,
            };
        }
        Err(e) => return fail(format!("start: {e:?}")),
    };
    for (i, gi) in case.inputs.iter().enumerate() {
        if s.is_terminated() {
            break;
        }
        let kind = match gi {
            // As in `vertep_conformance::differential::rust_records`: every third event arrives
            // from a verified sender DID (the records must stay identical).
            DiffInput::Event(n) if i % 3 == 2 => InputKind::Event(
                Event::named(n.clone(), EventType::External).with_verified_origin(
                    "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                ),
            ),
            DiffInput::Event(n) => InputKind::Event(Event::named(n.clone(), EventType::External)),
            DiffInput::Cancel => InputKind::Cancel,
        };
        let input = Input {
            env: env(i as u64 + 1),
            kind,
            sessions: Some(vec!["vt0".to_string()]),
        };
        match session::step(&chart, s, input.clone(), &mut tape) {
            Ok(o) => {
                records.push(OracleRecord {
                    input: OracleInput::Step(input),
                    tape: tape.take(),
                    trace: o.trace,
                });
                s = o.session;
            }
            Err(StepError::BudgetExceeded { partial, .. }) => {
                records.push(OracleRecord {
                    input: OracleInput::Step(input),
                    tape: tape.take(),
                    trace: partial.trace,
                });
                return DiffRun {
                    records,
                    budget_exceeded: true,
                    error: None,
                };
            }
            Err(StepError::Terminated) => break,
        }
    }
    DiffRun {
        records,
        budget_exceeded: false,
        error: None,
    }
}

/// Runs every case and returns the runs as one JSON array text.
pub fn diff_all(cases: &[DiffCase], registry: &dyn DatamodelRegistry) -> String {
    let runs: Vec<DiffRun> = cases.iter().map(|c| diff_records(c, registry)).collect();
    serde_json::to_string(&runs).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
