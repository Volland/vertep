//! Decision records for an auditor (feature `agent`): verify an explanation bundle exported by an
//! agent host and return the explanations it recomputed, or replay a verified bundle under another
//! chart version. Both run offline, with no host, decider, network or runtime: everything comes
//! from the bundle ([[openspec:decision-records#Explanation Bundle]]).
//!
//! Inputs and outputs are JSON, as on the command bridge. The explanations and the counterfactual
//! report keep the canonical JSON of `vertep-agent` (the text the bundle's digests cover, and what
//! `vt explain --json` prints), so the same record reads the same in Rust, JavaScript and Python.

use std::sync::Arc;

use serde_json::{Value as J, json};
use vertep_agent::bundle::{Bundle, BundleError};
use vertep_agent::counterfactual::DecisionSource;
use vertep_agent::host::AgentHost;

use crate::charts::{ChartFormat, read_chart};
use crate::error::{ApiError, Code, Res};

fn invalid(e: &BundleError) -> ApiError {
    ApiError::new(Code::BundleInvalid, format!("verification failed: {e}"))
}

fn parse(text: &str) -> Res<Bundle> {
    serde_json::from_str::<J>(text)
        .map_err(|e| ApiError::arg(format!("the bundle is not JSON: {e}")))?;
    Bundle::from_json(text).map_err(|e| invalid(&e))
}

/// Verifies the bundle `text` offline and returns
/// `{actor, from, to, deciderCalls, explanations}`, the explanations being the ones recomputed
/// for positions `from` to `to` (each as `Explanation::to_json` writes it).
///
/// # Errors
///
/// `InvalidArgument` for text that is not JSON; `BundleInvalid` for a bundle that does not
/// verify, with a message naming the altered entry, the missing chart or the wrong digest.
// @tg: implements:: [[openspec:decision-records#Explanation Bundle]] {note: "the bindings' offline verifier"}
// @tg: implements:: [[openspec:decision-records#Explanation View]] {confidence: partial, note: "explanations recomputed from a bundle; no live host in the bindings"}
pub fn verify_bundle(text: &str) -> Res<J> {
    let b = parse(text)?;
    let v = b.verify().map_err(|e| invalid(&e))?;
    let explanations = v
        .explanations
        .iter()
        .map(|x| serde_json::to_value(x).map_err(|e| ApiError::arg(e.to_string())))
        .collect::<Res<Vec<J>>>()?;
    Ok(json!({
        "actor": b.actor,
        "from": b.from,
        "to": b.to,
        "deciderCalls": v.decider_calls,
        "explanations": explanations,
    }))
}

/// Verifies the bundle `text`, then replays its journal under `chart`
/// (`{source, format?}`, the format sniffed when absent) with the decisions the journal recorded,
/// and returns the counterfactual report (`CounterfactualReport::to_json`). The bundle is only
/// read; no decider is called.
///
/// # Errors
///
/// As [`verify_bundle`]; `ChartInvalid` for a chart that does not read; `InvalidArgument` for a
/// chart that cannot run as an agent chart.
// @tg: implements:: [[openspec:decision-records#Counterfactual Replay]] {confidence: partial, note: "from a bundle, recorded decisions only; no live host in the bindings"}
pub fn counterfactual_bundle(text: &str, chart: &J) -> Res<J> {
    let b = parse(text)?;
    b.verify().map_err(|e| invalid(&e))?;
    let source = chart
        .get("source")
        .and_then(J::as_str)
        .ok_or_else(|| ApiError::arg("the chart is {source, format?}"))?;
    let format = match chart.get("format").and_then(J::as_str) {
        Some(f) => ChartFormat::parse(f)?,
        None => ChartFormat::sniff(source),
    };
    let tried = read_chart(source, format)?;
    // a scratch host holding the bundle's charts and journal, as verification builds it
    let mut host = AgentHost::builder(Arc::new(vertep_runtime::VirtualClock::new(0))).build();
    for (name, nquads) in &b.charts {
        let c = vertep_rdf::read_chart(nquads.as_bytes(), vertep_rdf::RdfFormat::NQuads)
            .map_err(|e| ApiError::arg(format!("chart {name}: {e:?}")))?
            .chart;
        host.register(c)
            .map_err(|e| ApiError::arg(format!("chart {name}: {e}")))?;
    }
    for entry in &b.entries {
        host.journal_store()
            .append(&b.actor, entry.clone())
            .map_err(|e| ApiError::arg(e.to_string()))?;
    }
    let report = host
        .counterfactual(&b.actor, tried, DecisionSource::Recorded)
        .map_err(|e| ApiError::arg(e.to_string()))?;
    serde_json::to_value(&report).map_err(|e| ApiError::arg(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../vertep-agent/tests/fixtures/review-bundle.json");

    fn tampered(f: impl FnOnce(&mut J)) -> String {
        let mut j: J = serde_json::from_str(FIXTURE).unwrap();
        f(&mut j);
        j.to_string()
    }

    // @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A bundle verifies offline]] {level: unit}
    #[test]
    fn the_fixture_verifies_without_a_decider() {
        let v = verify_bundle(FIXTURE).unwrap();
        assert_eq!(v["deciderCalls"], 0);
        assert_eq!(v["explanations"].as_array().unwrap().len(), 3);
        let last = &v["explanations"][2];
        assert_eq!(last["input"], "decision");
        assert_eq!(last["decision"]["chosen"], "reject");
        assert_eq!(last["state_after"], json!(["rejected"]));
    }

    // @tg: verifies:: [[openspec:decision-records#Explanation Bundle#An altered entry is detected]] {level: unit}
    #[test]
    fn an_altered_entry_is_named() {
        let text = tampered(|j| j["entries"][1]["ts"] = json!(1));
        let e = verify_bundle(&text).unwrap_err();
        assert_eq!(e.code, Code::BundleInvalid);
        assert!(e.message.contains("entry 2"), "{}", e.message);
    }

    // @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A missing chart is detected]] {level: unit}
    #[test]
    fn a_missing_chart_is_named() {
        let mut version = String::new();
        let text = tampered(|j| {
            let charts = j["charts"].as_object_mut().unwrap();
            version = charts.keys().next().unwrap().clone();
            charts.clear();
        });
        let e = verify_bundle(&text).unwrap_err();
        assert_eq!(e.code, Code::BundleInvalid);
        assert!(e.message.contains(&version), "{}", e.message);
    }

    #[test]
    fn text_that_is_not_json_is_an_argument_error() {
        assert_eq!(verify_bundle("{").unwrap_err().code, Code::InvalidArgument);
    }

    #[test]
    fn the_recorded_chart_replays_without_divergence() {
        let b = Bundle::from_json(FIXTURE).unwrap();
        let nquads = b.charts.values().next().unwrap();
        let r =
            counterfactual_bundle(FIXTURE, &json!({"source": nquads, "format": "nquads"})).unwrap();
        assert_eq!(r["first_divergence"], J::Null);
        assert_eq!(r["decisions"], "recorded");
        assert_eq!(r["steps"].as_array().unwrap().len(), 3);
    }

    /// The fixture's chart with a rejection that escalates instead.
    pub(crate) const ESCALATING: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="idle"><datamodel><data id="ok" expr="true"/></datamodel><state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.ok" target="approved"/>
    <transition event="reject" target="escalated"/>
  </state>
  <state id="approved"/><state id="rejected"/><state id="escalated"/></scxml>"#;

    #[test]
    fn another_chart_diverges_at_the_decision() {
        let r = counterfactual_bundle(FIXTURE, &json!({ "source": ESCALATING })).unwrap();
        assert_eq!(r["first_divergence"], 3, "{r:#}");
        assert_eq!(r["steps"][2]["original_state"], json!(["rejected"]));
        assert_eq!(r["steps"][2]["counterfactual_state"], json!(["escalated"]));
    }
}
