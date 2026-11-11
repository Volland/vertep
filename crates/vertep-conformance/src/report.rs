//! Running the suite, judging each variant against the allow-list, and the machine-readable
//! report with its human-readable summary.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::allow::AllowList;
use crate::engine::{CaseRun, EngineUnderTest};
use crate::manifest::{Classification, Datamodel, ManifestTest, Suite, SuiteError, TestCase};
use crate::verdict::{FailReason, Verdict};

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
    /// The verdict on the Lean oracle; absent when the oracle did not run.
    pub lean: Option<EngineVerdict>,
    /// How the Lean oracle's verdict or trace differs from the Rust core's, if it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence: Option<String>,
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
    /// Tests deliberately not generated for a datamodel, with the reason.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<Exclusion>,
}

/// A test that has no variant for a datamodel on purpose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Exclusion {
    /// The IRP test identifier.
    pub test: String,
    /// The datamodel without a variant.
    pub datamodel: Datamodel,
    /// The IRP classification of the test.
    pub classification: Classification,
    /// Why there is no variant.
    pub reason: String,
}

/// The tests of `suite` that have no `cel` variant, with their reasons.
pub fn cel_exclusions(suite: &Suite) -> Result<Vec<Exclusion>, SuiteError> {
    Ok(suite
        .cel_exclusions()?
        .into_iter()
        .map(|(test, reason)| Exclusion {
            classification: suite
                .test(&test)
                .map_or(Classification::Manual, ManifestTest::classification),
            test,
            datamodel: Datamodel::Cel,
            reason,
        })
        .collect())
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
            divergence: None,
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
        divergence: None,
        status,
        reason,
        allow_reason,
        breaks_build,
    }
}

/// Run every variant on `engine` (manual tests are listed, not run), in parallel, and build
/// the report.
pub fn run_suite(cases: &[TestCase], allow: &AllowList, engine: &dyn EngineUnderTest) -> Report {
    let runs = run_parallel(cases, |case| engine.run(case, None));
    let records = cases
        .iter()
        .zip(runs)
        .map(|(case, run)| record(case, run.as_ref(), allow))
        .collect();
    Report::from_records(vec![engine.name().to_string()], records)
}

/// Run every variant on the Rust core with the answer tape recorded, replay each run on the
/// second engine (the Lean oracle), and report both verdicts
/// ([[openspec:scxml-conformance#Lean Model Conformance]]). A variant whose second verdict or
/// trace differs from the Rust run breaks the build, whatever its classification; the
/// allow-list applies to the Rust verdict as in [`run_suite`].
// @tg: implements:: [[openspec:scxml-conformance#Lean Model Conformance]]
pub fn run_differential(
    cases: &[TestCase],
    allow: &AllowList,
    rust: &dyn EngineUnderTest,
    lean: &dyn EngineUnderTest,
) -> Report {
    let runs = run_parallel(cases, |case| {
        let r = rust.run(case, None);
        let l = lean.run(case, Some(&r));
        (r, l)
    });
    let records = cases
        .iter()
        .zip(runs)
        .map(|(case, run)| {
            let Some((r, l)) = run else {
                return record(case, None, allow);
            };
            let mut rec = record(case, Some(&r), allow);
            rec.lean = Some(EngineVerdict::of(&l.verdict));
            // Runs Rust could not record (an unreadable document) have nothing to replay.
            let replayable = r.documents.iter().all(|d| !d.records.is_empty());
            let divergence = match (&r.verdict, &l.verdict) {
                _ if !replayable => None,
                (_, Verdict::Fail(FailReason::Divergence(m))) => Some(m.clone()),
                (a, b) if a == b => None,
                (a, b) => Some(format!(
                    "verdicts differ: rust {}, lean {}",
                    EngineVerdict::of(a).verdict,
                    b.reason().map_or("pass".to_string(), ToString::to_string)
                )),
            };
            if divergence.is_some() {
                rec.breaks_build = true;
            }
            rec.divergence = divergence;
            rec
        })
        .collect();
    Report::from_records(
        vec![rust.name().to_string(), lean.name().to_string()],
        records,
    )
}

impl Report {
    /// A report over `records`, with totals.
    pub fn from_records(engines: Vec<String>, records: Vec<Record>) -> Self {
        let mut report = Report {
            engines,
            ..Report::default()
        };
        for r in records {
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

    /// The records where the Lean oracle disagrees with the Rust core.
    pub fn divergences(&self) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|r| r.divergence.is_some())
            .collect()
    }
}

/// Run `work` on the non-manual cases on worker threads; results come back in case order.
pub fn run_parallel<T, F>(cases: &[TestCase], work: F) -> Vec<Option<T>>
where
    T: Send,
    F: Fn(&TestCase) -> T + Sync,
{
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<Option<T>> = (0..cases.len()).map(|_| None).collect();
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
                        let run = work(case);
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
        if !self.excluded.is_empty() {
            let mut by_reason: BTreeMap<(Datamodel, &str), Vec<String>> = BTreeMap::new();
            for e in &self.excluded {
                by_reason
                    .entry((e.datamodel, e.reason.as_str()))
                    .or_default()
                    .push(format!(
                        "{}{}",
                        e.test,
                        match e.classification {
                            Classification::Mandatory => "",
                            Classification::Optional => "(o)",
                            Classification::Manual => "(m)",
                        }
                    ));
            }
            let _ = writeln!(
                s,
                "not generated (o = optional, m = manual; all others mandatory):"
            );
            for ((dm, reason), tests) in by_reason {
                let _ = writeln!(
                    s,
                    "  {dm}: {} tests, {reason}: {}",
                    tests.len(),
                    tests.join(" ")
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
