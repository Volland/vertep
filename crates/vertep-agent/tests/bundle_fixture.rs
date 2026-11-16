//! A bundle exported on one machine must verify on every other. The fixture is exported once
//! (`VERTEP_BLESS=1 cargo test -p vertep-agent --test bundle_fixture`) and CI verifies it on
//! Linux and macOS.
//!
//! Two exports of the same run are not byte-identical, and need not be: the message id of an
//! external event embeds the process id (`urn:vertep:agent-message:<ms>-<pid>-<n>`), and the
//! entry digests cover it. What must hold is that a bundle verifies wherever it is read.

mod common;

use std::sync::Arc;

use common::*;
use vertep_agent::bundle::Bundle;
use vertep_agent::decider::{Decision, DecisionOutcome};
use vertep_core::datamodel::Value;

const FIXTURE: &str = "tests/fixtures/review-bundle.json";

fn exported() -> Bundle {
    let rec = Recording::default();
    let (b, _) = builder();
    let mut host = b.decider("d", Arc::new(rec.clone())).build();
    let chart = cel(
        "idle",
        r#"<datamodel><data id="ok" expr="true"/></datamodel>"#,
        r#"<state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.ok" target="approved"/>
    <transition event="reject" target="rejected"/>
  </state>
  <state id="approved"/><state id="rejected"/>"#,
    );
    spawn(&mut host, chart);
    host.send(AGENT, ev("start"));
    host.run_until_idle();
    host.deliver_outcome(
        AGENT,
        &rec.requests()[0].correlation,
        DecisionOutcome::Decision(Decision::new("reject", Value::Undefined)),
    );
    host.run_until_idle();
    let n = host.journal(AGENT).len() as u64;
    host.export_bundle(AGENT, 1, n).unwrap()
}

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)
}

#[test]
fn the_recorded_bundle_verifies_on_this_machine() {
    if std::env::var_os("VERTEP_BLESS").is_some() {
        std::fs::write(path(), exported().to_json()).unwrap();
    }
    let text = std::fs::read_to_string(path()).expect("tests/fixtures/review-bundle.json");
    let v = Bundle::from_json(&text).unwrap().verify().unwrap();
    assert_eq!(v.decider_calls, 0);
    assert_eq!(v.explanations.len(), 3);
}
