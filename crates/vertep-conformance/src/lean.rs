//! The Lean oracle as an engine under test (decision D5).
//!
//! The Lean package builds three executables into `lean/vertep-lean/.lake/build/bin/`
//! (`cd lean/vertep-lean && lake build`): `vertep-oracle` replays a Rust run's records (input,
//! datamodel answer tape, trace per macrostep) against the chart's RDF projection and prints
//! its own trace per macrostep, `vt-certify` checks a chart projection for well-formedness, and
//! `vertep-shapes` checks the SHACL shapes. [`LeanTools::find`] locates them (or the directory
//! in `VT_LEAN_BIN`).
//!
//! Charts cross as the N-Quads of their `vertep-rdf` projection; records as JSON Lines of
//! [`OracleRecord`]. The oracle prints numbers as decimals and keys in its own order, so traces
//! are compared as JSON values with numbers compared numerically ([`json_eq`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value as Json;
use vertep_core::ir::Chart;
use vertep_core::tape::OracleRecord;
use vertep_core::trace::MacrostepTrace;

use crate::engine::{CaseRun, DocumentRun, EngineUnderTest};
use crate::host::{DirLoader, Stop};
use crate::manifest::TestCase;
use crate::verdict::{FailReason, Verdict, judge};

/// How long one oracle replay may take before it counts as not terminating.
pub const ORACLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The Lean executables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeanTools {
    /// `vertep-oracle`.
    pub oracle: PathBuf,
    /// `vt-certify`.
    pub certify: PathBuf,
    /// `vertep-shapes`.
    pub shapes: PathBuf,
}

/// The directory the Lean executables are expected in: `$VT_LEAN_BIN`, or
/// `lean/vertep-lean/.lake/build/bin` in the workspace.
pub fn lean_bin_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VT_LEAN_BIN") {
        return PathBuf::from(dir);
    }
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(crate_dir)
        .join("lean/vertep-lean/.lake/build/bin")
}

impl LeanTools {
    /// Locate the executables, or explain how to build them.
    pub fn find() -> Result<Self, String> {
        let dir = lean_bin_dir();
        let tools = LeanTools {
            oracle: dir.join("vertep-oracle"),
            certify: dir.join("vt-certify"),
            shapes: dir.join("vertep-shapes"),
        };
        let missing: Vec<String> = [&tools.oracle, &tools.certify]
            .iter()
            .filter(|p| !p.is_file())
            .map(|p| p.display().to_string())
            .collect();
        if missing.is_empty() {
            Ok(tools)
        } else {
            Err(format!(
                "the Lean oracle is not built (missing {}); run `cd lean/vertep-lean && lake build` or set VT_LEAN_BIN",
                missing.join(", ")
            ))
        }
    }

    /// Locate the executables for a test: `None` (and a message on stderr) when they are not
    /// built, unless `required`, in which case a missing build panics. Tests pass
    /// `cfg!(feature = "differential")` so the CI job fails instead of skipping.
    pub fn for_test(required: bool) -> Option<Self> {
        match LeanTools::find() {
            Ok(t) => Some(t),
            Err(msg) if required => panic!("{msg}"),
            Err(msg) => {
                eprintln!("skipped: {msg}");
                None
            }
        }
    }
}

/// The chart's projection as N-Quads, the form the Lean executables read.
pub fn chart_nquads(chart: &Chart) -> String {
    vertep_rdf::write_chart(&vertep_rdf::project(chart), vertep_rdf::RdfFormat::NQuads)
}

/// A scratch directory removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vertep-lean-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One output line of the oracle.
#[derive(Clone, Debug, PartialEq)]
pub enum LeanLine {
    /// The Lean trace of the macrostep.
    Trace(Json),
    /// The oracle failed on the macrostep (for example a request differing from the tape).
    Error(String),
}

/// Replay `records` on the oracle for `chart`. Fails when the oracle rejects its input.
pub fn replay(
    tools: &LeanTools,
    chart: &Chart,
    records: &[OracleRecord],
) -> Result<Vec<LeanLine>, String> {
    let scratch = Scratch::new().map_err(|e| format!("scratch directory: {e}"))?;
    let chart_path = scratch.0.join("chart.nq");
    let records_path = scratch.0.join("records.jsonl");
    let out_path = scratch.0.join("traces.jsonl");
    std::fs::write(&chart_path, chart_nquads(chart)).map_err(|e| e.to_string())?;
    let mut lines = String::new();
    for r in records {
        lines.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        lines.push('\n');
    }
    std::fs::write(&records_path, lines).map_err(|e| e.to_string())?;
    let mut child = Command::new(&tools.oracle)
        .arg("--chart")
        .arg(&chart_path)
        .arg("--records")
        .arg(&records_path)
        .arg("--out")
        .arg(&out_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", tools.oracle.display()))?;
    let deadline = std::time::Instant::now() + ORACLE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "vertep-oracle did not finish within {} s (a macrostep that never ends?)",
                    ORACLE_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(e) => return Err(format!("vertep-oracle: {e}")),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("vertep-oracle: {e}"))?;
    if out.status.code() == Some(3) {
        return Err(format!(
            "vertep-oracle rejected its input: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = std::fs::read_to_string(&out_path).map_err(|e| {
        format!(
            "vertep-oracle wrote no traces ({e}); stderr: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Json = serde_json::from_str(l).map_err(|e| format!("oracle output: {e}"))?;
            Ok(match v.get("error") {
                Some(err) if v.as_object().is_some_and(|o| o.len() == 1) => {
                    LeanLine::Error(match err {
                        Json::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                }
                _ => LeanLine::Trace(v),
            })
        })
        .collect()
}

/// JSON equality with numbers compared by value (`1` equals `1.0`) and object keys in any
/// order.
pub fn json_eq(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_eq(p, q))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_eq(v, w)))
        }
        _ => a == b,
    }
}

/// Where and how a Lean replay differs from the Rust run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The macrostep index.
    pub macrostep: u64,
    /// What differs: the oracle's error (which names the request it made and the one the tape
    /// recorded) or both traces.
    pub detail: String,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "macrostep {}: {}", self.macrostep, self.detail)
    }
}

/// Compare the oracle's lines with the Rust records, macrostep by macrostep.
// @tg: implements:: [[openspec:scxml-conformance#Lean Model Conformance]]
pub fn compare(records: &[OracleRecord], lean: &[LeanLine]) -> Result<(), Divergence> {
    for (i, r) in records.iter().enumerate() {
        let macrostep = r.trace.index;
        let rust = serde_json::to_value(&r.trace).unwrap_or(Json::Null);
        match lean.get(i) {
            None => {
                return Err(Divergence {
                    macrostep,
                    detail: format!(
                        "the oracle returned {} traces for {} records",
                        lean.len(),
                        records.len()
                    ),
                });
            }
            Some(LeanLine::Error(e)) => {
                return Err(Divergence {
                    macrostep,
                    detail: format!(
                        "the Lean oracle failed: {e}; Rust request sequence: {}",
                        tape_summary(r)
                    ),
                });
            }
            Some(LeanLine::Trace(t)) if !json_eq(&rust, t) => {
                return Err(Divergence {
                    macrostep,
                    detail: format!("traces differ\n  rust: {rust}\n  lean: {t}"),
                });
            }
            Some(LeanLine::Trace(_)) => {}
        }
    }
    if lean.len() > records.len() {
        return Err(Divergence {
            macrostep: records.len() as u64,
            detail: format!(
                "the oracle returned {} traces for {} records",
                lean.len(),
                records.len()
            ),
        });
    }
    Ok(())
}

fn tape_summary(r: &OracleRecord) -> String {
    let reqs: Vec<String> = r
        .tape
        .iter()
        .map(|e| serde_json::to_string(&e.request).unwrap_or_default())
        .collect();
    format!("[{}]", reqs.join(", "))
}

/// The Lean oracle as an engine: it replays the Rust run of each document given as
/// `reference` (recorded with the answer tape) and judges its own last trace.
#[derive(Clone, Debug)]
pub struct LeanOracle {
    /// The executables.
    pub tools: LeanTools,
}

impl LeanOracle {
    /// Replay one Rust document run; the verdict is the Lean trace's, or the divergence.
    pub fn run_document(&self, dir: &Path, rust: &DocumentRun) -> DocumentRun {
        let fail = |m: String| DocumentRun {
            path: rust.path.clone(),
            verdict: Verdict::Fail(FailReason::InterpreterError(m)),
            records: Vec::new(),
            log: Vec::new(),
        };
        if rust.records.is_empty() {
            return fail("the Rust run recorded no macrosteps to replay".into());
        }
        let chart = match DirLoader::new(dir).compile_file(&rust.path) {
            Ok(c) => c,
            Err(e) => return fail(e),
        };
        let diverged = |m: String| DocumentRun {
            path: rust.path.clone(),
            verdict: Verdict::Fail(FailReason::Divergence(m)),
            records: Vec::new(),
            log: Vec::new(),
        };
        let lines = match replay(&self.tools, chart.chart(), &rust.records) {
            Ok(l) => l,
            Err(e) => return diverged(e),
        };
        if let Err(d) = compare(&rust.records, &lines) {
            return diverged(d.to_string());
        }
        let last: Option<MacrostepTrace> = match lines.last() {
            Some(LeanLine::Trace(t)) => serde_json::from_value(normalize_numbers(t.clone())).ok(),
            _ => None,
        };
        // A host-level stop (budget, stall) is not the oracle's to judge: Lean ran the same
        // macrosteps, so it inherits the reason; a terminated session is judged from Lean's
        // own trace.
        let verdict = match &rust.verdict {
            Verdict::Fail(
                FailReason::BudgetExceeded(_)
                | FailReason::Stalled
                | FailReason::InterpreterError(_),
            ) => rust.verdict.clone(),
            _ => judge(&chart, last.as_ref(), &Stop::Finished),
        };
        DocumentRun {
            path: rust.path.clone(),
            verdict,
            records: Vec::new(),
            log: Vec::new(),
        }
    }
}

/// Integral decimals (`3.0`) as integers, so serde can read them into integer fields.
fn normalize_numbers(v: Json) -> Json {
    match v {
        Json::Number(n) => match n.as_f64() {
            Some(f) if n.as_u64().is_none() && f.fract() == 0.0 && (0.0..9.0e15).contains(&f) => {
                Json::from(f as u64)
            }
            _ => Json::Number(n),
        },
        Json::Array(a) => Json::Array(a.into_iter().map(normalize_numbers).collect()),
        Json::Object(o) => Json::Object(
            o.into_iter()
                .map(|(k, v)| (k, normalize_numbers(v)))
                .collect(),
        ),
        other => other,
    }
}

impl EngineUnderTest for LeanOracle {
    fn name(&self) -> &'static str {
        "lean"
    }

    fn run(&self, case: &TestCase, reference: Option<&CaseRun>) -> CaseRun {
        let Some(reference) = reference else {
            return CaseRun::from_documents(vec![DocumentRun {
                path: case.dir.clone(),
                verdict: Verdict::Fail(FailReason::InterpreterError(
                    "the Lean oracle needs the Rust run with its answer tape".into(),
                )),
                records: Vec::new(),
                log: Vec::new(),
            }]);
        };
        CaseRun::from_documents(
            reference
                .documents
                .iter()
                .map(|d| self.run_document(&case.dir, d))
                .collect(),
        )
    }
}
