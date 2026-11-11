//! The IRP suite on the Lean oracle (task 8.6): every runnable variant runs on the Rust core
//! with its datamodel answer tape recorded, then the oracle replays the tape against the chart's
//! RDF projection and must produce the same trace and verdict.
//!
//! The full run is `#[ignore]` and runs in CI as
//! `cargo test -p vertep-conformance --release --features differential -- --ignored differential`;
//! a small smoke sample runs with the gate. Without the Lean build both skip with a message,
//! except under the `differential` feature, where a missing build fails.

use vertep_conformance::allow::default_allow_list_path;
use vertep_conformance::engine::DocumentRun;
use vertep_conformance::lean::{LeanLine, compare, replay};
use vertep_conformance::{
    AllowList, EngineUnderTest, FailReason, LeanOracle, LeanTools, RustCore, Suite, TestCase,
    Verdict, run_differential,
};

const REQUIRED: bool = cfg!(feature = "differential");

fn rust() -> RustCore {
    RustCore {
        record_tape: true,
        ..RustCore::default()
    }
}

fn case(suite: &Suite, id: &str) -> TestCase {
    let t = suite.test(id).expect("test in the manifest");
    suite
        .cases()
        .into_iter()
        .find(|c| c.id == t.id)
        .expect("a variant")
}

#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_irp_suite_on_lean() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let suite = Suite::committed().unwrap();
    let allow = AllowList::load(&default_allow_list_path(), &suite).unwrap();
    let report = run_differential(&suite.cases(), &allow, &rust(), &LeanOracle { tools });
    let path = vertep_conformance::report_path().with_file_name("conformance-lean-report.json");
    report.write_json(&path).expect("write the report");
    println!("{}", report.summary());
    let replayed = report
        .records
        .iter()
        .filter(|r| r.lean.is_some() && r.divergence.is_none())
        .count();
    println!(
        "lean: {replayed} variants agree with Rust, {} differ; report: {}",
        report.divergences().len(),
        path.display()
    );
    for r in report.divergences() {
        println!(
            "  {}/{}: {}",
            r.test,
            r.datamodel,
            r.divergence.as_deref().unwrap_or("")
        );
    }
    assert!(
        report.divergences().is_empty(),
        "the Lean oracle differs from the Rust core"
    );
    assert!(report.gate_passes(), "{}", report.summary());
}

// @tg: verifies:: [[openspec:scxml-conformance#Lean Model Conformance#Same verdict on both]] {level: integration}
#[test]
fn conformance_lean_smoke_sample_agrees_with_rust() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let suite = Suite::committed().unwrap();
    let lean = LeanOracle { tools };
    // A raise-order test, a null-datamodel In() test, an invoke with done.invoke, a delayed
    // send, and a test with three start documents.
    for id in ["144", "436", "215", "175", "403"] {
        let case = case(&suite, id);
        let r = rust().run(&case, None);
        assert_eq!(r.verdict, Verdict::Pass, "{id} on Rust");
        let l = lean.run(&case, Some(&r));
        assert_eq!(l.verdict, r.verdict, "{id}: Lean and Rust verdicts");
        for d in &r.documents {
            let chart = vertep_conformance::DirLoader::new(&case.dir)
                .compile_file(&d.path)
                .unwrap();
            let lines = replay(&lean.tools, chart.chart(), &d.records).unwrap();
            assert_eq!(lines.len(), d.records.len());
            compare(&d.records, &lines).expect("identical traces");
        }
    }
}

// @tg: verifies:: [[openspec:scxml-conformance#Lean Model Conformance#Divergence reported]] {level: integration}
#[test]
fn conformance_lean_divergence_names_test_macrostep_and_requests() {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let suite = Suite::committed().unwrap();
    let lean = LeanOracle { tools };
    // Test 147 evaluates conditions; change one recorded request so the oracle's own request
    // (from the chart) differs from the tape.
    let case = case(&suite, "147");
    let mut r = rust().run(&case, None);
    let doc: &mut DocumentRun = &mut r.documents[0];
    let (step, entry) = doc
        .records
        .iter()
        .enumerate()
        .find_map(|(i, rec)| {
            rec.tape
                .iter()
                .position(|e| matches!(e.request, vertep_core::tape::Request::EvalCond { .. }))
                .map(|j| (i, j))
        })
        .expect("147 evaluates a condition");
    let macrostep = doc.records[step].trace.index;
    doc.records[step].tape[entry].request = vertep_core::tape::Request::EvalCond {
        expr: "tampered".into(),
    };
    let chart = vertep_conformance::DirLoader::new(&case.dir)
        .compile_file(&doc.path)
        .unwrap();
    let lines = replay(&lean.tools, chart.chart(), &doc.records).unwrap();
    assert!(
        lines.iter().any(|l| matches!(l, LeanLine::Error(_))),
        "the oracle reports the mismatch: {lines:?}"
    );
    let d = compare(&doc.records, &lines).expect_err("a divergence");
    assert_eq!(d.macrostep, macrostep);
    assert!(
        d.detail.contains("tampered"),
        "both requests are named: {d}"
    );
    let l = lean.run(&case, Some(&r));
    let Verdict::Fail(FailReason::Divergence(m)) = &l.verdict else {
        panic!("the Lean run fails with a divergence: {:?}", l.verdict)
    };
    assert!(m.contains(&format!("macrostep {macrostep}")), "{m}");
    // The differential report names the test.
    let allow = AllowList::default();
    let tampered = TamperedRust(r.clone());
    let report = run_differential(std::slice::from_ref(&case), &allow, &tampered, &lean);
    assert!(!report.gate_passes());
    assert_eq!(report.divergences()[0].test, "147");
}

/// A "Rust engine" that returns a prepared (tampered) run.
struct TamperedRust(vertep_conformance::CaseRun);

impl EngineUnderTest for TamperedRust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn run(
        &self,
        _case: &TestCase,
        _reference: Option<&vertep_conformance::CaseRun>,
    ) -> vertep_conformance::CaseRun {
        self.0.clone()
    }
}
