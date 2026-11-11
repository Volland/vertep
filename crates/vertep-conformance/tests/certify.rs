//! The Rust validator and the Lean `WellFormed` check (`vt-certify`) agree on generated charts:
//! well-formed ones from `vertep-testkit`, and the same charts broken by one targeted mutation
//! per validation rule.
//!
//! The full sample is `#[ignore]` (`--features differential -- --ignored differential`;
//! `VT_CERTIFY_CHARTS` sets its size); a small sample runs with the gate. Without the Lean
//! build both skip with a message, except under the `differential` feature.

use vertep_conformance::LeanTools;
use vertep_conformance::differential::{
    Certification, MUTATIONS, certify, sample_validation_charts,
};
use vertep_core::ir::Chart;
use vertep_core::validate::validate;

const REQUIRED: bool = cfg!(feature = "differential");

/// Compare the verdicts on `n` sampled charts; returns the disagreements and the rejections
/// `vt-certify` gave as "not a chart projection" (exit 2). A dangling reference (an unknown
/// target) cannot be carried by the projection, so Lean rejects such charts at decoding
/// rather than in `WellFormed`; that still counts as the same verdict when Rust rejects too.
fn disagreements(tools: &LeanTools, n: usize) -> (Vec<String>, Vec<String>) {
    let charts = sample_validation_charts(n, 0x6365_7274);
    let workers = std::thread::available_parallelism()
        .map(|w| w.get())
        .unwrap_or(4)
        .min(16);
    let chunks: Vec<&[(Chart, Option<usize>)]> = charts
        .chunks(charts.len().div_ceil(workers).max(1))
        .collect();
    let results: Vec<(Vec<String>, Vec<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut undecodable = Vec::new();
                    for (chart, mutation) in chunk.iter() {
                        let rust = validate(chart);
                        let what = mutation.map_or("well-formed sample", |m| MUTATIONS[m]);
                        let lean = match certify(tools, chart) {
                            Ok(c) => c,
                            Err(e) => {
                                out.push(format!("{what}: {e}"));
                                continue;
                            }
                        };
                        let agree = match (&rust, &lean) {
                            (Ok(()), Certification::Certified) => true,
                            (Err(_), Certification::NotWellFormed(_)) => true,
                            (Err(_), Certification::NotAProjection(m)) => {
                                undecodable.push(format!("{what}: {m}"));
                                true
                            }
                            _ => false,
                        };
                        if !agree {
                            out.push(format!(
                                "{what}: Rust {:?}, Lean {lean:?}\n{}",
                                rust.err(),
                                vertep_scxml::write(chart).unwrap_or_default()
                            ));
                        }
                    }
                    (out, undecodable)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("certify worker"))
            .collect()
    });
    let mut bad = Vec::new();
    let mut undecodable = Vec::new();
    for (b, u) in results {
        bad.extend(b);
        undecodable.extend(u);
    }
    (bad, undecodable)
}

fn check(n: usize) {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let (bad, undecodable) = disagreements(&tools, n);
    println!(
        "validation: {n} charts ({} mutated), {} disagreements, {} rejected by both with Lean at decoding",
        n / 2,
        bad.len(),
        undecodable.len()
    );
    for u in &undecodable {
        println!("  {}", u.lines().next().unwrap_or(""));
    }
    assert!(
        bad.is_empty(),
        "Rust validate and vt-certify disagree:\n{}",
        bad.join("\n---\n")
    );
}

// @tg: verifies:: [[openspec:formal-model#IR Model In Lean#Same verdict as Rust validation]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_validation_matches_vt_certify() {
    let n = std::env::var("VT_CERTIFY_CHARTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(192);
    check(n);
}

#[test]
fn conformance_validation_smoke_sample_matches_vt_certify() {
    // Every mutation once, plus as many well-formed charts.
    check(2 * MUTATIONS.len());
}
