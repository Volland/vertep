//! The W3C SCXML IRP conformance gate on the Rust core.
//!
//! `cargo test -p vertep-conformance --release -- --nocapture conformance` runs every
//! mandatory and optional test variant, writes `target/conformance-report.json`, prints the
//! summary and fails when a mandatory test fails without an allow-list entry or an
//! allow-listed test passes.

use vertep_conformance::allow::default_allow_list_path;
use vertep_conformance::{AllowList, RustCore, Suite, report_path, run_suite};

#[test]
fn conformance() {
    let suite = Suite::committed().expect("the committed suite reads");
    if let Err(problems) = suite.verify_hashes() {
        panic!(
            "the committed IRP suite differs from its pinned import:\n{}",
            problems.join("\n")
        );
    }
    let allow =
        AllowList::load(&default_allow_list_path(), &suite).expect("the allow-list is valid");
    let engine = RustCore::default();
    let cases = match std::env::var("VT_CONFORMANCE_ONLY") {
        Ok(only) => {
            let ids: Vec<&str> = only.split(',').collect();
            suite
                .cases()
                .into_iter()
                .filter(|c| ids.contains(&c.id.as_str()))
                .collect()
        }
        Err(_) => suite.cases(),
    };
    let report = run_suite(&cases, &allow, &engine);
    let path = report_path();
    report.write_json(&path).expect("write the report");
    println!("{}", report.summary());
    println!("report: {}", path.display());
    assert!(
        report.gate_passes(),
        "conformance gate failed:\n{}",
        report.summary()
    );
}
