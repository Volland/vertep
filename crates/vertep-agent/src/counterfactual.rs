//! Counterfactual replay: the inputs of a journal run under another chart version into a scratch
//! machine, with a report of every position where the configuration or the disposition of an
//! answer differs. The original journal is only read
//! ([[openspec:decision-records#Counterfactual Replay]]).

use serde::Serialize;

use crate::decider::{DecisionOutcome, DecisionRequest};

/// Where the answers to the offers come from when the chart under test is replayed.
pub enum DecisionSource<'a> {
    /// The decisions and refusals the journal recorded, re-validated against the new chart.
    Recorded,
    /// A function consulted at each decision input, given the offer the new chart makes at that
    /// point; the recorded answer is replaced by its answer.
    Supplied(&'a dyn Fn(&DecisionRequest) -> DecisionOutcome),
}

/// One journal position under both charts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CounterfactualStep {
    /// The journal position.
    pub position: u64,
    /// The kind of input.
    pub input: String,
    /// The configuration after the entry under the recorded chart.
    pub original_state: Vec<String>,
    /// The configuration after the entry under the chart being tried.
    pub counterfactual_state: Vec<String>,
    /// The disposition recorded on the entry, for a decision or refusal.
    pub recorded_disposition: Option<String>,
    /// What the shell would have made of the same answer under the new chart.
    pub counterfactual_disposition: Option<String>,
    /// True when the configurations differ.
    pub state_diverged: bool,
    /// True when both dispositions are known and differ.
    pub disposition_changed: bool,
}

/// The report of a counterfactual replay.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CounterfactualReport {
    /// The actor.
    pub actor: String,
    /// The chart version the journal ran under.
    pub original_version: String,
    /// The chart version tried.
    pub counterfactual_version: String,
    /// Where the answers came from: `recorded` or `supplied`.
    pub decisions: &'static str,
    /// Every position, in order.
    pub steps: Vec<CounterfactualStep>,
    /// The first position whose configuration differs.
    pub first_divergence: Option<u64>,
    /// The positions where an answer would have been handled differently.
    pub changed_dispositions: Vec<u64>,
}

impl CounterfactualReport {
    /// The report as JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
