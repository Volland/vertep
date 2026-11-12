//! One test per scenario of the scxml-conformance requirements the harness realizes now (the
//! Lean, differential and round-trip requirements come with tasks 8.6, 8.9 and 8.10). Test
//! names start with `conformance_` so the CI filter `conformance` runs them with the gate.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vertep_conformance::host::HostLog;
use vertep_conformance::manifest::DATAMODELS;
use vertep_conformance::{
    AllowList, AllowListError, Budget, CaseRun, Classification, Datamodel, Deviation, DirLoader,
    DocumentRun, EngineUnderTest, FailReason, Host, RustCore, Status, Stop, Suite, TestCase,
    Verdict, run_suite,
};
use vertep_core::datamodel::DatamodelRegistry;

/// Run an SCXML document on the in-memory host with `budget`; returns the verdict, the stop
/// reason and the host log.
fn run_text(xml: &str, budget: Budget) -> (Verdict, Stop, Vec<HostLog>, u64) {
    let registry = vertep_conformance::host::HostRegistry;
    let reg: &dyn DatamodelRegistry = &registry;
    let loader = DirLoader::new(std::env::temp_dir());
    let chart = loader.compile(xml).expect("the chart compiles");
    let mut host = Host::new(reg, &loader, budget);
    let stop = match host.start_root(chart.clone()) {
        Ok(_) => host.run(),
        Err(s) => s,
    };
    let records = host.take_root_records();
    let verdict =
        vertep_conformance::verdict::judge(&chart, records.last().map(|r| &r.trace), &stop);
    (verdict, stop, host.log().to_vec(), host.now_ms())
}

fn scxml(body: &str) -> String {
    format!(
        r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="s0">{body}
        <final id="pass"/><final id="fail"/></scxml>"##
    )
}

fn delivered(log: &[HostLog], session: &str) -> Vec<String> {
    log.iter()
        .filter_map(|l| match l {
            HostLog::Delivered {
                session: s, event, ..
            } if s == session => Some(event.clone()),
            _ => None,
        })
        .collect()
}

/// An engine with fixed verdicts, to exercise the gate and the report without the suite.
struct Fixed(Vec<(&'static str, bool)>);

impl EngineUnderTest for Fixed {
    fn name(&self) -> &'static str {
        "fixed"
    }

    fn run(&self, case: &TestCase, _reference: Option<&CaseRun>) -> CaseRun {
        let pass = self
            .0
            .iter()
            .find(|(id, _)| *id == case.id)
            .is_some_and(|(_, p)| *p);
        let verdict = if pass {
            Verdict::Pass
        } else {
            Verdict::Fail(FailReason::FailState)
        };
        CaseRun::from_documents(vec![DocumentRun {
            path: PathBuf::new(),
            verdict,
            records: Vec::new(),
            log: Vec::new(),
        }])
    }
}

fn case(id: &str, classification: Classification) -> TestCase {
    TestCase {
        id: id.to_string(),
        datamodel: Datamodel::Ecmascript,
        classification,
        dir: PathBuf::new(),
        starts: vec![PathBuf::from(format!("{id}/test{id}.scxml"))],
    }
}

fn deviation(id: &str) -> Deviation {
    Deviation {
        test: id.to_string(),
        datamodel: Datamodel::Ecmascript,
        reason: "expected".to_string(),
    }
}

// ----- IRP Suite Import ------------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#IRP Suite Import#Pinned revision]] {level: integration}
#[test]
fn conformance_suite_matches_its_pinned_hashes() {
    let suite = Suite::committed().unwrap();
    let files = suite
        .verify_hashes()
        .expect("the committed suite is the pinned import");
    assert!(files > 230, "every suite file is hashed, got {files}");
    // A copy of the suite with one byte changed is refused.
    let dir = std::env::temp_dir().join(format!("vt-irp-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("ecmascript/144")).unwrap();
    for f in ["manifest.xml", "SHA256SUMS"] {
        std::fs::copy(suite.root.join(f), dir.join(f)).unwrap();
    }
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    let only: String = sums
        .lines()
        .filter(|l| l.ends_with("ecmascript/144/test144.scxml"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(dir.join("SHA256SUMS"), only).unwrap();
    let original =
        std::fs::read_to_string(suite.root.join("ecmascript/144/test144.scxml")).unwrap();
    let path = dir.join("ecmascript/144/test144.scxml");
    std::fs::write(&path, &original).unwrap();
    let copy = Suite::load(&dir).unwrap();
    assert_eq!(copy.verify_hashes(), Ok(1), "the unchanged copy verifies");
    std::fs::write(&path, original.replace("foo", "fop")).unwrap();
    let problems = copy.verify_hashes().unwrap_err();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("test144.scxml: content hash differs")),
        "{problems:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// @tg: verifies:: [[openspec:scxml-conformance#IRP Suite Import#Datamodel variants]] {level: integration}
#[test]
fn conformance_templated_test_has_a_runnable_ecmascript_variant() {
    let suite = Suite::committed().unwrap();
    let t = suite.test("144").expect("test 144 is in the manifest");
    assert_eq!(t.classification(), Classification::Mandatory);
    let case = suite
        .case(t, Datamodel::Ecmascript)
        .expect("144 has an ecmascript variant");
    let text = std::fs::read_to_string(&case.starts[0]).unwrap();
    assert!(text.contains(r##"datamodel="ecmascript""##));
    assert!(
        !text.contains("conf:"),
        "the conf: template markup is transformed away"
    );
    assert_eq!(RustCore::default().run(&case, None).verdict, Verdict::Pass);
    // Every manifest test is offered for at least one datamodel, and the null chart only as
    // null.
    for t in &suite.tests {
        assert!(
            DATAMODELS.iter().any(|d| suite.case(t, *d).is_some()),
            "{} has no variant",
            t.id
        );
    }
    assert!(suite.has_variant("436", Datamodel::Null));
    assert!(!suite.has_variant("436", Datamodel::Ecmascript));
}

// ----- Pass Fail Protocol ----------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Pass Fail Protocol#Pass state reached]] {level: integration}
#[test]
fn conformance_entering_pass_is_a_pass() {
    let (v, stop, _, _) = run_text(
        &scxml(r##"<state id="s0"><transition target="pass"/></state>"##),
        Budget::default(),
    );
    assert_eq!(stop, Stop::Finished);
    assert_eq!(v, Verdict::Pass);
    let (v, _, _, _) = run_text(
        &scxml(r##"<state id="s0"><transition target="fail"/></state>"##),
        Budget::default(),
    );
    assert_eq!(v, Verdict::Fail(FailReason::FailState));
}

// @tg: verifies:: [[openspec:scxml-conformance#Pass Fail Protocol#Budget exceeded]] {level: integration}
#[test]
fn conformance_budget_exceeded_is_a_failure_with_its_reason() {
    // Sends itself an event forever: the step budget runs out.
    let ping = scxml(
        r##"<state id="s0"><onentry><send event="e"/></onentry>
           <transition event="e"><send event="e"/></transition></state>"##,
    );
    let budget = Budget {
        max_steps: 50,
        ..Budget::default()
    };
    let (v, stop, _, _) = run_text(&ping, budget);
    assert_eq!(stop, Stop::StepBudget(50));
    assert!(
        matches!(v, Verdict::Fail(FailReason::BudgetExceeded(ref m)) if m.contains("step budget"))
    );
    // Waits for a timeout beyond the virtual-time budget.
    let slow = scxml(
        r##"<state id="s0"><onentry><send event="t" delay="10s"/></onentry>
           <transition event="t" target="pass"/></state>"##,
    );
    let budget = Budget {
        max_virtual_ms: 5_000,
        ..Budget::default()
    };
    let (v, _, _, _) = run_text(&slow, budget);
    assert!(
        matches!(v, Verdict::Fail(FailReason::BudgetExceeded(ref m)) if m.contains("virtual-time"))
    );
}

// ----- Test Host -------------------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Test Host#Invoked child session]] {level: integration}
#[test]
fn conformance_child_event_arrives_before_done_invoke() {
    let parent = scxml(
        r##"<state id="s0">
             <invoke id="child" type="http://www.w3.org/TR/scxml/">
               <content>
                 <scxml initial="c" version="1.0" datamodel="ecmascript">
                   <state id="c"><onentry><send target="#_parent" event="hello"/></onentry>
                     <transition target="end"/></state>
                   <final id="end"/>
                 </scxml>
               </content>
             </invoke>
             <transition event="hello" target="s1"/>
             <transition event="*" target="fail"/>
           </state>
           <state id="s1">
             <transition event="done.invoke.child" target="pass"/>
             <transition event="*" target="fail"/>
           </state>"##,
    );
    let (v, _, log, _) = run_text(&parent, Budget::default());
    assert_eq!(v, Verdict::Pass);
    assert_eq!(delivered(&log, "vt0"), vec!["hello", "done.invoke.child"]);
}

// @tg: verifies:: [[openspec:scxml-conformance#Test Host#Delayed send with virtual time]] {level: integration}
#[test]
fn conformance_delayed_send_uses_virtual_time() {
    let doc = scxml(
        r##"<state id="s0">
             <onentry><send event="later" delay="1s"/><send event="never" delay="2s" id="n"/>
               <cancel sendid="n"/></onentry>
             <transition event="later" target="s1"/>
             <transition event="*" target="fail"/>
           </state>
           <state id="s1"><onentry><send event="tick" delay="500ms"/></onentry>
             <transition event="tick" target="pass"/><transition event="*" target="fail"/></state>"##,
    );
    let started = Instant::now();
    let (v, _, log, now) = run_text(&doc, Budget::default());
    assert_eq!(v, Verdict::Pass);
    assert_eq!(now, 1_500, "the clock advanced to each due time");
    assert_eq!(delivered(&log, "vt0"), vec!["later", "tick"]);
    assert!(
        started.elapsed() < Duration::from_millis(1_000),
        "no real waiting"
    );
}

// ----- Rust Core Conformance -------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Rust Core Conformance#Mandatory test failure breaks the build]] {level: integration}
#[test]
fn conformance_unlisted_mandatory_failure_breaks_the_build() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("2", Classification::Mandatory),
    ];
    let report = run_suite(&cases, &AllowList::default(), &Fixed(vec![("1", true)]));
    assert!(!report.gate_passes());
    let failing: Vec<&str> = report
        .build_failures()
        .iter()
        .map(|r| r.test.as_str())
        .collect();
    assert_eq!(failing, vec!["2"]);
    assert!(report.summary().contains("2/ecmascript [mandatory]"));
}

// @tg: verifies:: [[openspec:scxml-conformance#Rust Core Conformance#All mandatory tests pass]] {level: integration}
#[test]
fn conformance_passing_or_allow_listed_mandatory_tests_pass_the_gate() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("2", Classification::Mandatory),
    ];
    let allow = AllowList {
        entries: vec![deviation("2")],
    };
    let report = run_suite(&cases, &allow, &Fixed(vec![("1", true)]));
    assert!(report.gate_passes(), "{}", report.summary());
    assert_eq!(report.records[1].status, Status::AllowListed);
}

// ----- Expected Deviation Allow List -----------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Expected Deviation Allow List#Unexpected pass fails]] {level: integration}
#[test]
fn conformance_allow_listed_pass_fails_the_run() {
    let cases = vec![case("7", Classification::Optional)];
    let allow = AllowList {
        entries: vec![deviation("7")],
    };
    let report = run_suite(&cases, &allow, &Fixed(vec![("7", true)]));
    assert_eq!(report.records[0].status, Status::UnexpectedPass);
    assert!(!report.gate_passes());
    assert!(
        report.records[0]
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("remove its allow-list entry"))
    );
}

// @tg: verifies:: [[openspec:scxml-conformance#Expected Deviation Allow List#Entry without a reason]] {level: integration}
#[test]
fn conformance_allow_list_entry_without_reason_is_rejected() {
    let suite = Suite::committed().unwrap();
    let no_reason = "[[deviation]]\ntest = \"201\"\ndatamodel = \"ecmascript\"\n";
    assert!(matches!(
        AllowList::parse(no_reason, &suite),
        Err(AllowListError::MissingReason { .. })
    ));
    let blank = "[[deviation]]\ntest = \"201\"\ndatamodel = \"ecmascript\"\nreason = \"  \"\n";
    assert!(matches!(
        AllowList::parse(blank, &suite),
        Err(AllowListError::MissingReason { .. })
    ));
    // Entries naming tests the suite lacks, and duplicates, are rejected too.
    let unknown = "[[deviation]]\ntest = \"9999\"\ndatamodel = \"ecmascript\"\nreason = \"x\"\n";
    assert!(matches!(
        AllowList::parse(unknown, &suite),
        Err(AllowListError::UnknownTest { .. })
    ));
    let wrong_variant =
        "[[deviation]]\ntest = \"436\"\ndatamodel = \"ecmascript\"\nreason = \"x\"\n";
    assert!(matches!(
        AllowList::parse(wrong_variant, &suite),
        Err(AllowListError::UnknownTest { .. })
    ));
    let twice = format!("{0}reason = \"x\"\n{0}reason = \"y\"\n", no_reason);
    assert!(matches!(
        AllowList::parse(&twice, &suite),
        Err(AllowListError::Duplicate { .. })
    ));
}

// ----- Optional Test Reporting -----------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Optional Test Reporting#Optional failure does not break the build]] {level: integration}
#[test]
fn conformance_optional_failure_is_reported_but_not_gating() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("5", Classification::Optional),
    ];
    let report = run_suite(&cases, &AllowList::default(), &Fixed(vec![("1", true)]));
    assert!(report.gate_passes());
    assert_eq!(report.records[1].status, Status::Failed);
    assert!(!report.records[1].breaks_build);
    assert_eq!(report.totals[&Classification::Optional].failed, 1);
    assert!(
        report
            .summary()
            .contains("optional failures (not gating):\n  5/ecmascript")
    );
}

// @tg: verifies:: [[openspec:scxml-conformance#Optional Test Reporting#Manual tests listed]] {level: integration}
#[test]
fn conformance_manual_tests_are_listed_as_not_run() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("301", Classification::Manual),
    ];
    let report = run_suite(&cases, &AllowList::default(), &Fixed(vec![("1", true)]));
    assert_eq!(report.records[1].status, Status::NotRun);
    assert!(report.records[1].rust.is_none());
    assert_eq!(report.totals[&Classification::Manual].not_run, 1);
    assert!(
        report
            .summary()
            .contains("manual tests not run: 301/ecmascript")
    );
    // The committed suite lists all nine manual tests, 513 (a text description) included; a
    // test with variants for several datamodels counts once.
    let suite = Suite::committed().unwrap();
    let manual: std::collections::BTreeSet<String> = suite
        .cases()
        .into_iter()
        .filter(|c| c.classification == Classification::Manual)
        .map(|c| c.id)
        .collect();
    assert_eq!(manual.len(), 9, "{manual:?}");
    assert!(manual.contains("513"));
}

// ----- Conformance Report ----------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-conformance#Conformance Report#Report contents]] {level: integration}
#[test]
fn conformance_report_has_a_record_per_variant_and_totals() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("2", Classification::Mandatory),
        case("3", Classification::Optional),
        case("4", Classification::Manual),
    ];
    let allow = AllowList {
        entries: vec![deviation("2")],
    };
    let report = run_suite(&cases, &allow, &Fixed(vec![("1", true), ("3", true)]));
    assert_eq!(report.records.len(), cases.len());
    let m = report.totals[&Classification::Mandatory];
    assert_eq!((m.total, m.passed, m.failed, m.allow_listed), (2, 1, 0, 1));
    assert_eq!(report.totals[&Classification::Optional].passed, 1);
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    let r = &json["records"][1];
    assert_eq!(r["test"], "2");
    assert_eq!(r["datamodel"], "ecmascript");
    assert_eq!(r["classification"], "mandatory");
    assert_eq!(r["rust"]["verdict"], "fail");
    assert_eq!(r["rust"]["reason"], "entered final state `fail`");
    assert!(r["lean"].is_null(), "no Lean verdict until the oracle runs");
    assert_eq!(r["status"], "allow-listed");
    assert_eq!(r["allow_reason"], "expected");
    assert_eq!(json["totals"]["mandatory"]["allow_listed"], 1);
}

// @tg: verifies:: [[openspec:scxml-conformance#Conformance Report#Summary printed]] {level: integration}
#[test]
fn conformance_summary_has_totals_and_failing_tests() {
    let cases = vec![
        case("1", Classification::Mandatory),
        case("2", Classification::Mandatory),
    ];
    let report = run_suite(&cases, &AllowList::default(), &Fixed(vec![("1", true)]));
    let s = report.summary();
    assert!(s.contains("mandatory/ecmascript"), "{s}");
    assert!(s.contains("2 variants:   1 passed,   1 failed"), "{s}");
    assert!(s.contains("gate: FAILED by 1 variant(s):"), "{s}");
    assert!(
        s.contains("2/ecmascript [mandatory] entered final state `fail`"),
        "{s}"
    );
}
