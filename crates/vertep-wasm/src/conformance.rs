//! The W3C SCXML IRP conformance suite in WebAssembly (feature `conformance`, test builds
//! only): the same in-memory test host and verdicts as `vertep-conformance`'s native gate, with
//! the suite's documents passed in as text instead of read from files.
//!
//! - [`input`] (native): the committed suite's non-manual variants, the files of their
//!   directories and the allow-list entries, as the JSON a WebAssembly run takes.
//! - [`native_verdicts`] (native): the verdicts of the native harness (`RustCore` on the
//!   suite's files) for those variants.
//! - [`run`]: the verdicts of the same variants from documents held in memory; in WebAssembly
//!   it is the `conformanceRun` export.
//!
//! A verdict is `{test, datamodel, verdict: "pass" | "fail", reason?}`; the two runs agree when
//! their verdict lists are equal.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vertep_conformance::host::{HOST_DATAMODELS, in_scxml_context};
use vertep_conformance::verdict::judge;
use vertep_conformance::{Budget, ChartLoader, Host, HostRegistry, Verdict};
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::DatamodelRegistry;
use vertep_scxml::{ReadOptions, Resolution, ResolveError, SourceRequest};

/// One variant to run: its documents are `starts`, its directory `dir` (paths relative to the
/// suite root, `/`-separated).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    /// The test id.
    pub test: String,
    /// The variant's datamodel (`ecmascript`, `null`, `cel`).
    pub datamodel: String,
    /// `mandatory` or `optional`.
    pub classification: String,
    /// The variant's directory.
    pub dir: String,
    /// The start documents.
    pub starts: Vec<String>,
}

/// What a run takes: the variants and the text of every file in their directories.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Input {
    /// The variants.
    pub cases: Vec<Case>,
    /// File text by path relative to the suite root.
    pub files: BTreeMap<String, String>,
    /// The allow-listed variants: `[test, datamodel]`.
    pub allow: Vec<(String, String)>,
}

/// The verdict on one variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseVerdict {
    /// The test id.
    pub test: String,
    /// The datamodel.
    pub datamodel: String,
    /// `pass` or `fail`.
    pub verdict: String,
    /// The failure reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl CaseVerdict {
    fn of(case: &Case, v: &Verdict) -> Self {
        CaseVerdict {
            test: case.test.clone(),
            datamodel: case.datamodel.clone(),
            verdict: if v.is_pass() { "pass" } else { "fail" }.to_owned(),
            reason: v.reason().map(ToString::to_string),
        }
    }
}

/// Loads a variant's documents from memory, exactly as `vertep_conformance::DirLoader` loads
/// them from its directory.
struct MemLoader<'a> {
    files: &'a BTreeMap<String, String>,
    dir: &'a str,
}

impl MemLoader<'_> {
    fn read_file(&self, src: &str) -> Result<String, String> {
        let rel = src.strip_prefix("file:").unwrap_or(src);
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        if rel.is_empty()
            || rel.contains("..")
            || rel.contains(':')
            || rel.contains('\\')
            || rel.starts_with('/')
        {
            return Err(format!("`{src}` is not a file next to the test"));
        }
        let path = format!("{}/{rel}", self.dir);
        self.files
            .get(&path)
            .cloned()
            .ok_or_else(|| format!("{path}: no such file"))
    }

    fn compile(&self, xml: &str) -> Result<CompiledChart, String> {
        let resolver = |r: &SourceRequest<'_>| -> Result<Resolution, ResolveError> {
            self.read_file(r.src)
                .map(Resolution::Inline)
                .map_err(ResolveError::new)
        };
        let options = ReadOptions::new()
            .with_datamodels(&HOST_DATAMODELS)
            .with_resolver(&resolver);
        let parsed = vertep_scxml::read(xml, &options).map_err(|e| e.to_string())?;
        CompiledChart::new(parsed.chart).map_err(|v| {
            let msgs: Vec<String> = v.iter().map(|x| format!("{x:?}")).collect();
            format!("invalid chart: {}", msgs.join("; "))
        })
    }
}

impl ChartLoader for MemLoader<'_> {
    fn load_text(&self, xml: &str) -> Result<CompiledChart, String> {
        self.compile(&in_scxml_context(xml))
    }

    fn load_src(&self, src: &str) -> Result<CompiledChart, String> {
        let xml = self.read_file(src)?;
        self.compile(&xml)
    }
}

fn run_document(files: &BTreeMap<String, String>, dir: &str, path: &str) -> Verdict {
    use vertep_conformance::FailReason;
    let loader = MemLoader { files, dir };
    let xml = match files.get(path) {
        Some(x) => x,
        None => {
            return Verdict::Fail(FailReason::InterpreterError(format!(
                "{path}: no such file"
            )));
        }
    };
    let chart = match loader.compile(xml) {
        Ok(c) => c,
        Err(e) => return Verdict::Fail(FailReason::InterpreterError(e)),
    };
    let registry = HostRegistry;
    let reg: &dyn DatamodelRegistry = &registry;
    let mut host = Host::new(reg, &loader, Budget::default());
    let stop = match host.start_root(chart.clone()) {
        Ok(_) => host.run(),
        Err(stop) => stop,
    };
    let records = host.take_root_records();
    judge(&chart, records.last().map(|r| &r.trace), &stop)
}

/// Runs every variant of `input` from its in-memory documents.
// @tg: implements:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly]] {confidence: partial, note: "the IRP suite from in-memory documents"}
pub fn run(input: &Input) -> Vec<CaseVerdict> {
    input
        .cases
        .iter()
        .map(|case| {
            let verdict = case
                .starts
                .iter()
                .map(|p| run_document(&input.files, &case.dir, p))
                .find(|v| !v.is_pass())
                .unwrap_or(Verdict::Pass);
            CaseVerdict::of(case, &verdict)
        })
        .collect()
}

#[cfg(not(target_family = "wasm"))]
mod native {
    use std::path::Path;

    use vertep_conformance::allow::default_allow_list_path;
    use vertep_conformance::{AllowList, Classification, RustCore, Suite, TestCase};

    use super::{Case, CaseVerdict, Input};

    fn rel(root: &Path, p: &Path) -> String {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn cases(suite: &Suite) -> Vec<TestCase> {
        suite
            .cases()
            .into_iter()
            .filter(|c| c.classification != Classification::Manual)
            .collect()
    }

    /// The committed suite's non-manual variants, their directories' files and the
    /// allow-list, as a run's input.
    ///
    /// # Errors
    ///
    /// The suite or allow-list read error.
    pub fn input() -> Result<Input, String> {
        let suite = Suite::committed().map_err(|e| e.to_string())?;
        let allow =
            AllowList::load(&default_allow_list_path(), &suite).map_err(|e| e.to_string())?;
        let mut input = Input::default();
        for c in cases(&suite) {
            for entry in std::fs::read_dir(&c.dir).map_err(|e| e.to_string())? {
                let p = entry.map_err(|e| e.to_string())?.path();
                if p.is_file() {
                    let key = rel(&suite.root, &p);
                    if !input.files.contains_key(&key)
                        && let Ok(text) = std::fs::read_to_string(&p)
                    {
                        input.files.insert(key, text);
                    }
                }
            }
            if allow.get(&c.id, c.datamodel).is_some() {
                input
                    .allow
                    .push((c.id.clone(), c.datamodel.as_str().to_owned()));
            }
            input.cases.push(Case {
                test: c.id.clone(),
                datamodel: c.datamodel.as_str().to_owned(),
                classification: c.classification.to_string(),
                dir: rel(&suite.root, &c.dir),
                starts: c.starts.iter().map(|s| rel(&suite.root, s)).collect(),
            });
        }
        Ok(input)
    }

    /// The native harness's verdicts on the committed suite's non-manual variants.
    ///
    /// # Errors
    ///
    /// The suite read error.
    pub fn native_verdicts() -> Result<Vec<CaseVerdict>, String> {
        use vertep_conformance::EngineUnderTest;
        let suite = Suite::committed().map_err(|e| e.to_string())?;
        let engine = RustCore::default();
        Ok(cases(&suite)
            .iter()
            .map(|c| {
                let run = engine.run(c, None);
                let case = Case {
                    test: c.id.clone(),
                    datamodel: c.datamodel.as_str().to_owned(),
                    classification: c.classification.to_string(),
                    dir: String::new(),
                    starts: Vec::new(),
                };
                CaseVerdict::of(&case, &run.verdict)
            })
            .collect())
    }
}

#[cfg(not(target_family = "wasm"))]
pub use native::{input, native_verdicts};
