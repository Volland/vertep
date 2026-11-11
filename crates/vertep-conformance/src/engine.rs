//! Engines under test. The Rust core runs here; the Lean oracle (tasks 8.6 and 8.9) plugs in as
//! another [`EngineUnderTest`] that receives the Rust run's records (input, datamodel answer
//! tape, trace per macrostep) through [`EngineUnderTest::run`]'s `reference` and must
//! reproduce the trace.

use std::path::{Path, PathBuf};

use vertep_core::datamodel::DatamodelRegistry;
use vertep_core::tape::OracleRecord;

use crate::host::{Budget, DirLoader, Host, HostLog, HostRegistry};
use crate::manifest::TestCase;
use crate::verdict::{FailReason, Verdict, judge};

/// The run of one start document.
#[derive(Clone, Debug)]
pub struct DocumentRun {
    /// The document.
    pub path: PathBuf,
    /// Its verdict.
    pub verdict: Verdict,
    /// The root session's macrostep records (with the answer tape when it was recorded).
    pub records: Vec<OracleRecord>,
    /// What the host did, for diagnosing failures.
    pub log: Vec<HostLog>,
}

/// The run of one test variant: every start document, in manifest order.
#[derive(Clone, Debug)]
pub struct CaseRun {
    /// The verdict: pass when every document passes, otherwise the first failure.
    pub verdict: Verdict,
    /// One run per start document.
    pub documents: Vec<DocumentRun>,
}

impl CaseRun {
    /// Combine document runs.
    pub fn from_documents(documents: Vec<DocumentRun>) -> Self {
        let verdict = documents
            .iter()
            .map(|d| &d.verdict)
            .find(|v| !v.is_pass())
            .cloned()
            .unwrap_or(Verdict::Pass);
        CaseRun { verdict, documents }
    }
}

/// Something that runs IRP test documents and reports a verdict and a trace.
pub trait EngineUnderTest: Sync {
    /// A short name for reports (`rust`, `lean`).
    fn name(&self) -> &'static str;

    /// Run a test variant. `reference` is another engine's run of the same case, for engines
    /// that replay a recorded answer tape; the Rust core ignores it.
    fn run(&self, case: &TestCase, reference: Option<&CaseRun>) -> CaseRun;
}

/// The Rust core (`vertep-core` with the `null` and `ecmascript` datamodels) on the in-memory
/// test host.
#[derive(Clone, Copy, Debug, Default)]
pub struct RustCore {
    /// Limits of each run.
    pub budget: Budget,
    /// Record the root session's datamodel answer tape.
    pub record_tape: bool,
}

impl RustCore {
    /// Run one document with its directory as the source of `src` references.
    pub fn run_document(&self, dir: &Path, path: &Path) -> DocumentRun {
        let registry = HostRegistry;
        let loader = DirLoader::new(dir);
        let chart = match loader.compile_file(path) {
            Ok(c) => c,
            Err(e) => {
                return DocumentRun {
                    path: path.to_path_buf(),
                    verdict: Verdict::Fail(FailReason::InterpreterError(e)),
                    records: Vec::new(),
                    log: Vec::new(),
                };
            }
        };
        let reg: &dyn DatamodelRegistry = &registry;
        let mut host = Host::new(reg, &loader, self.budget).record_tape(self.record_tape);
        let stop = match host.start_root(chart.clone()) {
            Ok(_) => host.run(),
            Err(stop) => stop,
        };
        let records = host.take_root_records();
        let verdict = judge(&chart, records.last().map(|r| &r.trace), &stop);
        DocumentRun {
            path: path.to_path_buf(),
            verdict,
            records,
            log: host.log().to_vec(),
        }
    }
}

impl EngineUnderTest for RustCore {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn run(&self, case: &TestCase, _reference: Option<&CaseRun>) -> CaseRun {
        let documents = case
            .starts
            .iter()
            .map(|p| {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.run_document(&case.dir, p)
                }));
                r.unwrap_or_else(|panic| {
                    let msg = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                        .unwrap_or_else(|| "unknown panic".to_string());
                    DocumentRun {
                        path: p.clone(),
                        verdict: Verdict::Fail(FailReason::InterpreterError(format!(
                            "panic: {msg}"
                        ))),
                        records: Vec::new(),
                        log: Vec::new(),
                    }
                })
            })
            .collect();
        CaseRun::from_documents(documents)
    }
}
