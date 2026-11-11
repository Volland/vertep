//! Randomized differential testing of the Rust core against the Lean oracle (task 8.9) and of
//! the Rust validator against `vt-certify`.
//!
//! The full runs are `#[ignore]` and run in CI as
//! `cargo test -p vertep-conformance --release --features differential -- --ignored differential`
//! (`VT_DIFF_CASES` overrides the number of generated cases). Small smoke samples run with the
//! gate. Without the Lean build every Lean test skips with a message, except under the
//! `differential` feature, where a missing build fails.

use vertep_conformance::LeanTools;
use vertep_conformance::differential::{
    DifferentialConfig, GENERATED_MICROSTEP_BUDGET, GeneratedCase, RustRun, TraceOracle,
    default_regressions_dir, load_regressions, replay_regressions, run_generated,
};
use vertep_conformance::lean::LeanLine;

const REQUIRED: bool = cfg!(feature = "differential");

fn cases_from_env(default: u32) -> u32 {
    std::env::var("VT_DIFF_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn assert_agreement(tools: &LeanTools, cases: u32) {
    let cfg = DifferentialConfig {
        cases,
        ..DifferentialConfig::default()
    };
    let out = run_generated(&cfg, tools);
    println!(
        "differential: {} generated cases compared with Lean, {} skipped",
        out.compared, out.skipped
    );
    assert!(out.aborted.is_none(), "aborted: {:?}", out.aborted);
    if let Some(c) = out.counterexample {
        panic!(
            "Rust and Lean disagree; shrunk counterexample stored in {}\n{}\ninputs: {:?}\n{}",
            c.stored.display(),
            vertep_scxml::write(&c.case.chart).unwrap_or_default(),
            c.case.inputs,
            c.reason
        );
    }
    let failing = replay_regressions(&default_regressions_dir(), tools).expect("corpus reads");
    assert!(
        failing.is_empty(),
        "stored regressions disagree:\n{}",
        failing.join("\n")
    );
}

// @tg: verifies:: [[openspec:scxml-conformance#Randomized Differential Testing#Agreement on generated charts]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_generated_charts_agree_with_lean() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    assert_agreement(&tools, cases_from_env(512));
}

#[test]
fn conformance_differential_smoke_sample() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    assert_agreement(&tools, 24);
}

/// The idle-event loop of regression `5cbfa3338d26b03f` (a parallel state's failing eventless
/// guard queues two `error.execution` per round and enables nothing): Rust runs out of the
/// microstep budget, and the oracle terminates with the budget error and the same partial trace.
// @tg: verifies:: [[openspec:formal-model#Differential Oracle#Budget exhaustion agrees with Rust]] {level: integration}
#[test]
fn differential_budget_exhaustion_reported_with_partial_trace() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let (_, r) = load_regressions(&default_regressions_dir())
        .unwrap()
        .into_iter()
        .find(|(p, _)| p.ends_with("5cbfa3338d26b03f.json"))
        .expect("the regression is committed");
    let run = vertep_conformance::differential::rust_records(&r.case).unwrap();
    assert!(run.budget_exceeded, "Rust runs out of the budget");
    let lines = vertep_conformance::lean::replay(&tools, &r.case.chart, &run.records).unwrap();
    assert_eq!(lines.len(), run.records.len());
    match lines.last() {
        Some(LeanLine::BudgetExceeded { budget, trace }) => {
            assert_eq!(*budget, u64::from(GENERATED_MICROSTEP_BUDGET));
            let micro = trace["microsteps"].as_array().unwrap();
            assert_eq!(micro.len(), GENERATED_MICROSTEP_BUDGET as usize);
        }
        other => panic!("expected the budget error with the partial trace, got {other:?}"),
    }
    tools.check(&r.case, &run).expect("agreement");
}

/// A stand-in second implementation that "disagrees" whenever the chart has two or more
/// states or the run has more than one macrostep, so the shrunk counterexample is known.
struct DisagreesOnSize;

impl TraceOracle for DisagreesOnSize {
    fn check(&self, case: &GeneratedCase, run: &RustRun) -> Result<(), String> {
        let states = vertep_testkit::state_ids(&case.chart).len();
        if states >= 2 && run.records.len() >= 2 {
            Err(format!("{states} states, {} macrosteps", run.records.len()))
        } else {
            Ok(())
        }
    }
}

// @tg: verifies:: [[openspec:scxml-conformance#Randomized Differential Testing#Counterexample shrunk and kept]] {level: integration}
#[test]
fn conformance_counterexample_is_shrunk_and_kept() {
    let dir = std::env::temp_dir().join(format!("vt-regressions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = DifferentialConfig {
        cases: 256,
        regressions: dir.clone(),
        ..DifferentialConfig::default()
    };
    let out = run_generated(&cfg, &DisagreesOnSize);
    let c = out
        .counterexample
        .expect("the stand-in disagrees on some case");
    // Shrunk to the smallest disagreeing case: two states, one input.
    assert_eq!(
        vertep_testkit::state_ids(&c.case.chart).len(),
        2,
        "{:?}",
        c.case
    );
    assert_eq!(c.case.inputs.len(), 1, "{:?}", c.case.inputs);
    assert!(c.reason.contains("2 states, 2 macrosteps"), "{}", c.reason);
    // Kept: the corpus holds it and replays it.
    assert!(c.stored.starts_with(&dir));
    let stored = load_regressions(&dir).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].1.case, c.case);
    assert!(
        stored[0]
            .1
            .scxml
            .as_deref()
            .is_some_and(|x| x.contains("<scxml"))
    );
    let failing = replay_regressions(&dir, &DisagreesOnSize).unwrap();
    assert_eq!(failing.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The committed corpus parses and every case still runs on the Rust core.
#[test]
fn conformance_regression_corpus_reads() {
    for (path, r) in load_regressions(&default_regressions_dir()).unwrap() {
        vertep_conformance::differential::rust_records(&r.case)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}
