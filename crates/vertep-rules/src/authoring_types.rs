//! The data of the authoring flow: no engine here, so a build without the `native` feature can
//! still name stages, errors and the gate's context.

use crate::{Derivation, RulePolicy, clauses};

/// The predicates of the authoring records.
pub mod vocab {
    /// The program a proposal is for.
    pub const FOR_PROGRAM: &str = "urn:vt:rules:forProgram";
    /// Who proposed.
    pub const PROPOSED_BY: &str = "urn:vt:rules:proposedBy";
    /// The grant the proposer used.
    pub const GRANT: &str = "urn:vt:rules:grant";
    /// Where the proposal came from.
    pub const SOURCE: &str = "urn:vt:rules:source";
    /// A governed actor address.
    pub const SCOPE: &str = "urn:vt:rules:scope";
    /// The stage of a proposal.
    pub const STAGE: &str = "urn:vt:rules:stage";
    /// A decision node where the shadow run differed.
    pub const SHADOW_DIFFERS: &str = "urn:vt:rules:shadowDiffers";
    /// A journaled promotion act of a proposal.
    pub const ACT: &str = "urn:vt:rules:act";
    /// Who made an act.
    pub const ACT_BY: &str = "urn:vt:rules:actBy";
    /// The stage an act moved to.
    pub const ACT_STAGE: &str = "urn:vt:rules:actStage";
    /// The scope an act named.
    pub const ACT_SCOPE: &str = "urn:vt:rules:actScope";
    /// The digest of the program an act promoted.
    pub const ACT_DIGEST: &str = "urn:vt:rules:actDigest";
}

/// What an identity asks to do, for the capability check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Put a proposal forward.
    Propose,
    /// Move a proposal along its stages.
    Promote,
}

/// The stages of a proposal, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Stored, inert.
    Proposed,
    /// Run beside the live program.
    Shadow,
    /// Live for a named scope.
    Scoped,
    /// The live program.
    Live,
}

impl Stage {
    /// The stage's name.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Proposed => "proposed",
            Stage::Shadow => "shadow",
            Stage::Scoped => "scoped",
            Stage::Live => "live",
        }
    }

    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub(crate) fn parse(s: &str) -> Option<Stage> {
        [Stage::Proposed, Stage::Shadow, Stage::Scoped, Stage::Live]
            .into_iter()
            .find(|x| x.name() == s)
    }

    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub(crate) fn next(self) -> Option<Stage> {
        match self {
            Stage::Proposed => Some(Stage::Shadow),
            Stage::Shadow => Some(Stage::Scoped),
            Stage::Scoped => Some(Stage::Live),
            Stage::Live => None,
        }
    }
}

/// A gate refusal: a stable code, the offending clause when there is one, and the reason, so an
/// agent can repair and resubmit ([[openspec:rule-authoring#Gate Errors Are Structured]]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateError {
    /// A stable code: `self-governance`, `capability`, `type-error`, `profile`, `no-cases`,
    /// `trial-failed`, `existing-data`, `stage`, `skipped-step` or `store`.
    pub code: &'static str,
    /// The clause the error is about.
    pub clause: Option<String>,
    /// Why.
    pub reason: String,
}

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.reason)?;
        if let Some(c) = &self.clause {
            write!(f, " [{c}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for GateError {}

#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) fn err(
    code: &'static str,
    clause: Option<String>,
    reason: impl Into<String>,
) -> GateError {
    GateError {
        code,
        clause,
        reason: reason.into(),
    }
}

/// Who is acting and under which grant.
#[derive(Clone, Debug)]
pub struct Actor<'a> {
    /// The acting identity (an actor address or a DID).
    pub identity: &'a str,
    /// The grant presented.
    pub grant: &'a str,
}

/// What the gate checks a proposal against.
pub struct GateContext<'a> {
    /// The event names the chart declares; a program may conclude only these (and `refuse`).
    pub events: &'a [String],
    /// Recorded decisions for the trial run.
    pub cases: &'a [Derivation],
    /// A promote-grant holder's explicit waiver of the trial run when there are no cases.
    pub allow_no_cases: bool,
    /// The capability check.
    pub authorize: &'a dyn Fn(&Actor<'_>, Action) -> bool,
    /// How the rules run.
    pub policy: &'a RulePolicy,
}

/// A proposal's authorship ([[openspec:rule-authoring#Authorship Is Recorded]]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorship {
    /// The program the proposal is for.
    pub program: String,
    /// Who proposed.
    pub proposer: String,
    /// The grant used.
    pub grant: String,
    /// Where the proposal came from.
    pub source: String,
    /// The actors the program would govern.
    pub scope: Vec<String>,
}

/// A journaled promotion act, as read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Act {
    /// Who made it.
    pub by: String,
    /// The stage it moved to.
    pub stage: Stage,
    /// The scope it named, if any.
    pub scope: Option<String>,
    /// The digest of the program it promoted.
    pub digest: String,
}

/// A readable comparison of two program versions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClauseDiff {
    /// Clauses only in the new version.
    pub added: Vec<String>,
    /// Clauses only in the old version.
    pub removed: Vec<String>,
}

/// The clauses that differ between two versions of a program
/// ([[openspec:rule-authoring#Rule Diff]]); `counterfactual` gives the decisions that would differ.
// @tg: implements:: [[openspec:rule-authoring#Rule Diff]]
pub fn diff(old: &str, new: &str) -> ClauseDiff {
    let (a, b) = (clauses(old), clauses(new));
    ClauseDiff {
        added: b.iter().filter(|c| !a.contains(c)).cloned().collect(),
        removed: a.iter().filter(|c| !b.contains(c)).cloned().collect(),
    }
}
