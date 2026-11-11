//! Decide states: the `vt:decide` binding read from `vt:` extension nodes, its validation and
//! its RDF projection as `vt:DecideState`.
//!
//! The IR has no decide-state field ([[decisions#Types ride on vt extension nodes]]): a decide
//! state is an ordinary `<state>` carrying extension attributes in the `vt:` namespace.
//!
//! | Attribute | Meaning |
//! |---|---|
//! | `vt:decide="llm/claude"` | the decider binding (required) |
//! | `vt:deadline="30s"` | the decision deadline, a durable timer |
//! | `vt:fallback="guards/escalate"` | the decider consulted when the deadline passes |
//! | `vt:candidates="approve reject"` | event descriptors offered (default: every enabled event) |
//! | `vt:maxDecisions="12"` | accepted decisions allowed in this state over the actor's life |
//!
//! An element `<vt:decide decider=".." deadline=".." fallback=".." candidates=".."
//! maxDecisions=".."/>` is accepted as the same binding, so YAML charts can write it as one
//! extension. Both SCXML and YAML keep `vt:` extensions losslessly, so the binding parses and
//! serializes in every surface without surface-specific code.

use std::collections::BTreeMap;

use oxrdf::{Graph, NamedNode, Triple};
use vertep_core::ir::{Chart, Extension, ExtensionValue, State, StateElement, VT_NS};
use vertep_rdf::ChartVersion;
use vertep_runtime::chart::{element_attributes, parse_duration};

use crate::vocab;

/// The decide binding of one state.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecideSpec {
    /// The decide state's id.
    pub state: String,
    /// The decider binding.
    pub decider: String,
    /// The deadline in milliseconds.
    pub deadline_ms: Option<i64>,
    /// The fallback decider binding.
    pub fallback: Option<String>,
    /// Declared candidate descriptors; `None` offers every enabled event.
    pub candidates: Option<Vec<String>>,
    /// The bound on accepted decisions in this state.
    pub max_decisions: Option<u32>,
}

/// Why a chart's decide bindings are invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecideError {
    /// A decide state names no decider binding.
    MissingDecider {
        /// The state.
        state: String,
    },
    /// A malformed attribute value.
    BadAttribute {
        /// The state.
        state: String,
        /// The attribute's local name.
        attribute: String,
        /// The value.
        value: String,
    },
    /// The binding is on a state that cannot decide (parallel, final or history).
    NotDecidable {
        /// The state.
        state: String,
    },
}

impl core::fmt::Display for DecideError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecideError::MissingDecider { state } => {
                write!(f, "decide state {state} names no decider binding")
            }
            DecideError::BadAttribute {
                state,
                attribute,
                value,
            } => write!(f, "decide state {state}: bad vt:{attribute} {value:?}"),
            DecideError::NotDecidable { state } => write!(
                f,
                "state {state} carries vt:decide but is not an atomic or compound state"
            ),
        }
    }
}

impl std::error::Error for DecideError {}

const KEYS: &[&str] = &[
    "decide",
    "deadline",
    "fallback",
    "candidates",
    "maxDecisions",
];

fn vt_attrs(exts: &[Extension]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in exts.iter().filter(|e| e.ns == VT_NS) {
        match &e.value {
            ExtensionValue::Attribute(v) if KEYS.contains(&e.local.as_str()) => {
                out.insert(e.local.clone(), v.clone());
            }
            ExtensionValue::Element(xml) if e.local == "decide" => {
                let a = element_attributes(xml);
                out.insert(
                    "decide".into(),
                    a.get("decider").cloned().unwrap_or_default(),
                );
                for k in &KEYS[1..] {
                    if let Some(v) = a.get(*k) {
                        out.insert((*k).to_owned(), v.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn spec_of(s: &State, errors: &mut Vec<DecideError>) -> Option<DecideSpec> {
    let a = vt_attrs(&s.extensions);
    if a.is_empty() {
        return None;
    }
    let state = s.id.clone();
    if s.element != StateElement::State {
        errors.push(DecideError::NotDecidable { state });
        return None;
    }
    let bad = |attribute: &str, value: &str| DecideError::BadAttribute {
        state: state.clone(),
        attribute: attribute.to_owned(),
        value: value.to_owned(),
    };
    let decider = a
        .get("decide")
        .map(|d| d.trim().to_owned())
        .unwrap_or_default();
    if decider.is_empty() {
        errors.push(DecideError::MissingDecider {
            state: state.clone(),
        });
    }
    let deadline_ms = match a.get("deadline") {
        Some(d) => match parse_duration(d) {
            Some(ms) => Some(ms),
            None => {
                errors.push(bad("deadline", d));
                None
            }
        },
        None => None,
    };
    let max_decisions = match a.get("maxDecisions") {
        Some(m) => match m.trim().parse::<u32>() {
            Ok(n) => Some(n),
            Err(_) => {
                errors.push(bad("maxDecisions", m));
                None
            }
        },
        None => None,
    };
    let fallback = a
        .get("fallback")
        .map(|f| f.trim().to_owned())
        .filter(|f| !f.is_empty());
    let candidates = a
        .get("candidates")
        .map(|c| c.split_whitespace().map(str::to_owned).collect());
    Some(DecideSpec {
        state,
        decider,
        deadline_ms,
        fallback,
        candidates,
        max_decisions,
    })
}

fn walk(states: &[State], out: &mut BTreeMap<String, DecideSpec>, errors: &mut Vec<DecideError>) {
    for s in states {
        if let Some(spec) = spec_of(s, errors) {
            out.insert(spec.state.clone(), spec);
        }
        walk(&s.children, out, errors);
    }
}

/// The decide bindings of `chart`, by state id.
///
/// # Errors
///
/// Every [`DecideError`]: a decide state without decider binding, a malformed deadline or
/// bound, or a binding on a state that cannot decide. Such a chart is not accepted.
// @tg: implements:: [[openspec:deciders#Decide State Declaration]]
pub fn decide_specs(chart: &Chart) -> Result<BTreeMap<String, DecideSpec>, Vec<DecideError>> {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    walk(&chart.states, &mut out, &mut errors);
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// The node IRI of state `id` in chart version `version`.
pub fn state_node(version: &ChartVersion, id: &str) -> NamedNode {
    NamedNode::new_unchecked(vertep_rdf::iri::node_iri(
        version.as_str(),
        &vertep_rdf::project::state_fragment(id),
    ))
}

/// The statements that type the decide states of `chart` as `vt:DecideState` and link their
/// decider bindings, minted under the chart's version.
pub fn decide_state_triples(chart: &Chart, version: &ChartVersion) -> Vec<Triple> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut specs = BTreeMap::new();
    walk(&chart.states, &mut specs, &mut errors);
    for spec in specs.values() {
        let s = state_node(version, &spec.state);
        out.push(Triple::new(
            s.clone(),
            NamedNode::new_unchecked(vocab::RDF_TYPE),
            NamedNode::new_unchecked(vocab::VT_DECIDE_STATE),
        ));
        if !spec.decider.is_empty() {
            out.push(Triple::new(
                s,
                NamedNode::new_unchecked(vocab::BINDING),
                oxrdf::Literal::new_simple_literal(&spec.decider),
            ));
        }
    }
    out
}

/// The chart's RDF projection with its decide states typed `vt:DecideState`.
// @tg: implements:: [[openspec:deciders#Decide State Declaration]]
pub fn project(chart: &Chart) -> (ChartVersion, Graph) {
    let p = vertep_rdf::project(chart);
    let mut g = p.graph;
    for t in decide_state_triples(chart, &p.version) {
        g.insert(&t);
    }
    (p.version, g)
}
