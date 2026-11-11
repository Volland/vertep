//! Round-trip and shape checks over the conformance corpus: every chart must survive
//! SCXML → IR → SCXML and IR → RDF → IR unchanged, and its projection must conform to the
//! chart shapes.

use std::path::{Path, PathBuf};

use vertep_core::ir::Chart;
use vertep_scxml::{AnyDatamodel, DeferResolver, ReadOptions, check_round_trip};

/// One chart that failed a corpus check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusFailure {
    /// The chart file.
    pub path: PathBuf,
    /// Which check failed and why.
    pub reason: String,
}

/// The result of checking a corpus.
#[derive(Debug, Clone, Default)]
pub struct CorpusReport {
    /// Charts checked.
    pub checked: usize,
    /// Charts that failed a check.
    pub failures: Vec<CorpusFailure>,
}

/// Every `.scxml` file under `dir`, sorted.
pub fn corpus_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "scxml") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Check one chart's RDF round trip and shape conformance.
pub fn check_rdf(chart: &Chart) -> Result<(), String> {
    let projected = vertep_rdf::project(chart);
    let report = vertep_rdf::validate(&projected.graph);
    if !report.conforms() {
        return Err(format!(
            "projection does not conform to the chart shapes: {report:?}"
        ));
    }
    let back =
        vertep_rdf::reconstruct(&projected.graph).map_err(|e| format!("reconstruct: {e:?}"))?;
    if &back.chart != chart {
        return Err("RDF round trip changed the chart".into());
    }
    if vertep_rdf::chart_version(&back.chart) != projected.version {
        return Err("RDF round trip changed the chart version".into());
    }
    Ok(())
}

/// Run the SCXML and RDF round trips and the shape check over every chart under `dir`.
// @tg: implements:: [[openspec:scxml-conformance#Round Trip Corpus Checks]]
pub fn check_corpus(dir: &Path) -> CorpusReport {
    let resolver = DeferResolver;
    let opts = ReadOptions::new()
        .with_datamodels(&AnyDatamodel)
        .with_resolver(&resolver);
    let mut report = CorpusReport::default();
    for path in corpus_files(dir) {
        report.checked += 1;
        let fail = |reason: String| CorpusFailure {
            path: path.clone(),
            reason,
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                report.failures.push(fail(format!("unreadable: {e}")));
                continue;
            }
        };
        match check_round_trip(&text, &opts) {
            Ok(chart) => {
                if let Err(r) = check_rdf(&chart) {
                    report.failures.push(fail(r));
                }
            }
            Err(e) => report.failures.push(fail(format!("SCXML round trip: {e}"))),
        }
    }
    report
}
