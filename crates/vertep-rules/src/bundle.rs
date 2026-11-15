//! The data of a rule bundle and of a counterfactual report. No engine here, so a build without
//! the `native` feature can still read, write and move bundles.

use serde::{Deserialize, Serialize};

use crate::{Derivation, RulesError};

/// The bundle format name.
pub const BUNDLE_FORMAT: &str = "vertep-rule-bundle/1";

/// A statement term the bundle can carry. Rules read plain data, so only these kinds exist.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Term {
    /// An IRI.
    Iri(String),
    /// A plain string.
    Str(String),
    /// An integer.
    Int(i64),
    /// A boolean.
    Bool(bool),
    /// A double, as its lexical form.
    Double(String),
}

/// One supporting statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stmt {
    /// The statement id in the store that made the decision, as in `Derivation::support`.
    pub eid: String,
    /// Subject.
    pub s: Term,
    /// Predicate.
    pub p: Term,
    /// Object.
    pub o: Term,
}

/// One program of the closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramSource {
    /// The program node.
    pub iri: String,
    /// Its source text.
    pub text: String,
}

/// A rule decision with everything needed to re-run it with no shard, no model and no network:
/// the derivation, the program closure and the supporting statements
/// ([[openspec:rule-decider#Bundle Includes Rule Closure]]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleBundle {
    /// [`BUNDLE_FORMAT`].
    pub format: String,
    /// The record.
    pub derivation: Derivation,
    /// The program closure, sorted by IRI.
    pub programs: Vec<ProgramSource>,
    /// The supporting statements.
    pub statements: Vec<Stmt>,
}

impl RuleBundle {
    /// The bundle as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a bundle serializes")
    }

    /// A bundle from JSON.
    pub fn from_json(text: &str) -> Result<RuleBundle, RulesError> {
        let b: RuleBundle =
            serde_json::from_str(text).map_err(|e| RulesError::Store(e.to_string()))?;
        if b.format != BUNDLE_FORMAT {
            return Err(RulesError::Store(format!(
                "unknown bundle format {}",
                b.format
            )));
        }
        Ok(b)
    }
}

/// A decision that would differ under another rule version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    /// The decision node.
    pub node: String,
    /// What was decided.
    pub recorded_event: String,
    /// What the other version decides, or `None` when it concludes nothing for an offered event.
    pub counterfactual_event: Option<String>,
    /// What the other version's conclusion rests on, as statement ids of the original store.
    pub counterfactual_support: Vec<String>,
    /// Why there is no counterfactual event, when there is none.
    pub note: Option<String>,
}

/// The result of replaying recorded decisions under another program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualReport {
    /// Decisions the other version reaches again.
    pub unchanged: usize,
    /// Decisions that would differ.
    pub differs: Vec<Divergence>,
}
