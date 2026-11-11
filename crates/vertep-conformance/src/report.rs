//! Running the suite, judging each variant against the allow-list, and the machine-readable
//! report with its human-readable summary.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::allow::AllowList;
use crate::engine::{CaseRun, EngineUnderTest};
use crate::manifest::{Classification, Datamodel, TestCase};
use crate::verdict::Verdict;

/// What a variant's result means for the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Passed and not allow-listed.
    Passed,
    /// Failed and not allow-listed.
    Failed,
    /// Failed as the allow-list expects.
    AllowListed,
    /// Passed although the allow-list expects a failure: remove the entry.
    UnexpectedPass,
    /// A manual test: listed, not run.
    NotRun,
}

/// One engine's verdict on a variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineVerdict {
    /// `pass` or `fail`.
    pub verdict: &'static str,
    /// The failure reason, if it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl EngineVerdict {
    fn of(v: &Verdict) -> Self {
        match v {
            Verdict::Pass => EngineVerdict {
                verdict: "pass",
                reason: None,
            },
            Verdict::Fail(r) => EngineVerdict {
                verdict: "fail",
                reason: Some(r.to_string()),
            },
        }
    }
}

/// One record per test variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Record {
    /// The IRP test identifier.
    pub test: String,
    /// The variant's datamodel.
    pub datamodel: Datamodel,
    /// The IRP classification.
    pub classification: Classification,
    /// The verdict on the Rust core; absent when not run.
    pub rust: Option<EngineVerdict>,
    /// The verdict on the Lean oracle; absent until the oracle runs (task 8.6).
    pub lean: Option<EngineVerdict>,
    /// What the result means for the run.
    pub status: Status,
    /// The failure reason (the Rust core's, or why an unexpected pass is a failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The allow-list reason, when the variant is allow-listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_reason: Option<String>,
    /// Whether this record fails the build.
    pub breaks_build: bool,
}

/// Totals for one classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Variants of this classification.
    pub total: usize,
    /// Passed.
    pub passed: usize,
    /// Failed without an allow-list entry.
    pub failed: usize,
    /// Failed as allow-listed.
    pub allow_listed: usize,
    /// Allow-listed but passed.
    pub unexpected_pass: usize,
    /// Not run (manual).
    pub not_run: usize,
}

impl Totals {
    fn add(&mut self, status: Status) {
        self.total += 1;
        match status {
            Status::Passed => self.passed += 1,
            Status::Failed => self.failed += 1,
            Status::AllowListed => self.allow_listed += 1,
            Status::UnexpectedPass => self.unexpected_pass += 1,
            Status::NotRun => self.not_run += 1,
        }
    }
}

/// The conformance report ([[openspec:scxml-conformance#Conformance Report]]).
// @tg: implements:: [[openspec:scxml-conformance#Conformance Report]]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The engines that ran.
    pub engines: Vec<String>,
    /// One record per variant, in manifest and datamodel order.
    pub records: Vec<Record>,
    /// Totals per classification.
    pub totals: BTreeMap<Classification, Totals>,
    /// Totals per classification and datamodel.
    pub totals_by_datamodel: BTreeMap<String, Totals>,
}

/// Classify a run against the allow-list and the gate
/// ([[openspec:scxml-conformance#Rust Core Conformance]],
/// [[openspec:scxml-conformance#Optional Test Reporting]]).
///
/// A non-allow-listed failure breaks the build only for a mandatory test; an unexpected pass
/// breaks it for any classification, so stale entries are removed.
// @tg: implements:: [[openspec:scxml-conformance#Rust Core Conformance]]
// @tg: implements:: [[openspec:scxml-conformance#Optional Test Reporting]]
pub fn record(case: &TestCase, run: Option<&CaseRun>, allow: &AllowList) -> Record {
    let entry = allow.get(&case.id, case.datamodel);
    let allow_reason = entry.map(|e| e.reason.clone());
    let Some(run) = run else {
        return Record {
            test: case.id.clone(),
            datamodel: case.datamodel,
            classification: case.classification,
            rust: None,
            lean: None,
            status: Status::NotRun,
            reason: None,
            allow_reason,
            breaks_build: false,
        };
    };
    let (status, reason) = match (&run.verdict, entry) {
        (Verdict::Pass, None) => (Status::Passed, None),
        (Verdict::Pass, Some(_)) => (
            Status::UnexpectedPass,
            Some(format!(
                "allow-listed test {}/{} passed: remove its allow-list entry",
                case.id, case.datamodel
            )),
        ),
        (Verdict::Fail(r), None) => (Status::Failed, Some(r.to_string())),
        (Verdict::Fail(r), Some(_)) => (Status::AllowListed, Some(r.to_string())),
    };
    let breaks_build = match status {
        Status::UnexpectedPass => true,
        Status::Failed => case.classification == Classification::Mandatory,
        _ => false,
    };
    Record {
        test: case.id.clone(),
        datamodel: case.datamodel,
        classification: case.classification,
        rust: Some(EngineVerdict::of(&run.verdict)),
        lean: None,
        status,
        reason,
        allow_reason,
        breaks_build,
    }
}

/// Run every variant on `engine` (manual tests are listed, not run), in parallel, and build
/// the report.
pub fn run_suite(cases: &[TestCase], allow: &AllowList, engine: &dyn EngineUnderTest) -> Report {
    let runs = run_parallel(cases, engine);
    let mut report = Report {
        engines: vec![engine.name().to_string()],
        ..Report::default()
    };
    for (case, run) in cases.iter().zip(runs) {
        let r = record(case, run.as_ref(), allow);
        report
            .totals
            .entry(r.classification)
            .or_default()
            .add(r.status);
        report
            .totals_by_datamodel
            .entry(format!("{}/{}", r.classification, r.datamodel))
            .or_default()
            .add(r.status);
        report.records.push(r);
    }
    report
}

/// Run the non-manual cases on worker threads; results come back in case order.
pub fn run_parallel(cases: &[TestCase], engine: &dyn EngineUnderTest) -> Vec<Option<CaseRun>> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<Option<CaseRun>> = vec![None; cases.len()];
    let collected = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn_scoped(scope, || {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(case) = cases.get(i) else {
                            break;
                        };
                        if case.classification == Classification::Manual {
                            continue;
                        }
                        let run = engine.run(case, None);
                        if let Ok(mut c) = collected.lock() {
                            c.push((i, run));
                        }
                    }
                })
                .expect("spawn a conformance worker");
        }
    });
    for (i, run) in collected.into_inner().unwrap_or_default() {
        results[i] = Some(run);
    }
    results
}

impl Report {
    /// The records that fail the build.
    pub fn build_failures(&self) -> Vec<&Record> {
        self.records.iter().filter(|r| r.breaks_build).collect()
    }

    /// Whether the run passes the gate.
    pub fn gate_passes(&self) -> bool {
        self.build_failures().is_empty()
    }

    /// The report as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
    }

    /// Write the JSON report to `path`, creating its directory.
    pub fn write_json(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())
    }

    /// The human-readable summary: totals per classification and datamodel, the failures that
    /// break the build, optional failures, and the manual tests not run.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "SCXML IRP conformance ({})", self.engines.join(", "));
        for (k, t) in &self.totals_by_datamodel {
            let _ = writeln!(
                s,
                "  {k:<22} {:>3} variants: {:>3} passed, {:>3} failed, {:>3} allow-listed, {:>2} unexpected passes, {:>2} not run",
                t.total, t.passed, t.failed, t.allow_listed, t.unexpected_pass, t.not_run
            );
        }
        let failures = self.build_failures();
        if failures.is_empty() {
            let _ = writeln!(
                s,
                "gate: OK (every mandatory test passes or is allow-listed)"
            );
        } else {
            let _ = writeln!(s, "gate: FAILED by {} variant(s):", failures.len());
            for r in failures {
                let _ = writeln!(
                    s,
                    "  {}/{} [{}] {}",
                    r.test,
                    r.datamodel,
                    r.classification,
                    r.reason.as_deref().unwrap_or("")
                );
            }
        }
        let optional: Vec<&Record> = self
            .records
            .iter()
            .filter(|r| r.classification == Classification::Optional && r.status == Status::Failed)
            .collect();
        if !optional.is_empty() {
            let _ = writeln!(s, "optional failures (not gating):");
            for r in optional {
                let _ = writeln!(
                    s,
                    "  {}/{} {}",
                    r.test,
                    r.datamodel,
                    r.reason.as_deref().unwrap_or("")
                );
            }
        }
        let manual: Vec<String> = self
            .records
            .iter()
            .filter(|r| r.status == Status::NotRun)
            .map(|r| format!("{}/{}", r.test, r.datamodel))
            .collect();
        if !manual.is_empty() {
            let _ = writeln!(s, "manual tests not run: {}", manual.join(", "));
        }
        s
    }
}
