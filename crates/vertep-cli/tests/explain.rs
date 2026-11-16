//! `vt explain` and `vt verify-bundle`.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use vertep_agent::decider::GuardDecider;
use vertep_agent::host::AgentHost;
use vertep_runtime::VirtualClock;

const VT: &str = env!("CARGO_BIN_EXE_vt");
const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
  version="1.0" datamodel="cel" initial="idle">
  <datamodel><data id="level" expr="1"/></datamodel>
  <state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.level > 3" target="approved"/>
    <transition event="reject" target="rejected"/>
  </state>
  <state id="approved"/><state id="rejected"/>
</scxml>"#;

fn vt(args: &[&str]) -> Output {
    Command::new(VT).args(args).output().expect("vt runs")
}

fn write_bundle(dir: &Path) -> std::path::PathBuf {
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(1_794_355_200_000)))
        .decider("d", Arc::new(GuardDecider::first_enabled(["reject"])))
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let chart = vertep_scxml::read(CHART, &opts).unwrap().chart;
    let version = host.register(chart).unwrap();
    host.spawn("https://example.org/agents/a1", &version)
        .unwrap();
    host.send(
        "https://example.org/agents/a1",
        vertep_core::datamodel::Event::named("start", vertep_core::datamodel::EventType::External),
    );
    host.run_until_idle();
    let n = host.journal("https://example.org/agents/a1").len() as u64;
    let path = dir.join("bundle.json");
    std::fs::write(
        &path,
        host.export_bundle("https://example.org/agents/a1", 1, n)
            .unwrap()
            .to_json(),
    )
    .unwrap();
    path
}

// @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A bundle verifies offline]] {level: e2e}
// @tg: verifies:: [[openspec:decision-records#Explanation Bundle#An altered entry is detected]] {level: e2e}
#[test]
fn verify_bundle_and_explain_work_from_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = write_bundle(dir.path());
    let b = bundle.to_str().unwrap();

    let ok = vt(&["verify-bundle", b]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let text = String::from_utf8_lossy(&ok.stdout);
    assert!(
        text.starts_with("verified: https://example.org/agents/a1 positions 1 to 3"),
        "{text}"
    );
    assert!(text.contains("0 decider calls"));

    let x = vt(&["explain", b, "--position", "3"]);
    let t = String::from_utf8_lossy(&x.stdout);
    assert!(x.status.success(), "{}", String::from_utf8_lossy(&x.stderr));
    assert!(
        t.contains("entry 3 (decision)") && t.contains("accepted: offered [reject], chose reject"),
        "{t}"
    );
    assert!(t.contains("[review] -> [rejected]"), "{t}");
    let j = vt(&["explain", b, "--json"]);
    assert!(String::from_utf8_lossy(&j.stdout).contains("\"disposition\": \"accepted\""));

    // alter one entry on disk: verification fails and names it
    let altered = std::fs::read_to_string(&bundle).unwrap().replacen(
        "\"event\": \"reject\"",
        "\"event\": \"approve\"",
        1,
    );
    assert_ne!(altered, std::fs::read_to_string(&bundle).unwrap());
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, altered).unwrap();
    let r = vt(&["verify-bundle", bad.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("entry 3 was altered"),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
}
