//! Plain data: the policy and the record of one rule decision. No engine here.

use serde::{Deserialize, Serialize};
use vertep_core::datamodel::Value;

/// The engine pinned by this build: tiramemsu's ELPI integration over lean-elpi. A decision
/// records it ([[openspec:rule-programs#Rule Version Is Pinned By The Entry]]); replay with
/// another value reports the difference. Updated by hand when `tiramemsu` or `leanelpi` moves.
pub const ENGINE: &str = "tiramemsu-0.12/leanelpi-0.4";

/// How a rule decider runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulePolicy {
    /// The step budget of one evaluation, never wall-clock time
    /// ([[openspec:rule-programs#Evaluation Is Bounded]]).
    pub fuel: u64,
    /// The engine identifier recorded on every derivation.
    pub engine: String,
}

impl Default for RulePolicy {
    fn default() -> Self {
        RulePolicy {
            fuel: 1_000_000,
            engine: ENGINE.to_owned(),
        }
    }
}

/// The record of one rule decision: what the conclusion rests on and under which rules.
///
/// This is the rationale of a rule decision, labelled a derivation and distinct from a
/// decider's claim ([[openspec:rule-decider#Derivation Is The Rationale]]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    /// The decision node, `urn:vt:decision:<correlation>`.
    pub node: String,
    /// The actor that was asking.
    pub actor: String,
    /// The decide state.
    pub state: String,
    /// The program node.
    pub program: String,
    /// The event concluded.
    pub event: String,
    /// The supporting statements, in id order, as displayed by the store (`e12`).
    pub support: Vec<String>,
    /// The digest of the program closure: the IRI and source text of the program and of each
    /// include, sorted by IRI. It depends on the text only, so it holds in any store.
    pub closure_digest: String,
    /// The engine that ran.
    pub engine: String,
    /// The step budget.
    pub fuel_limit: u64,
    /// The steps used.
    pub fuel_used: u64,
    /// The transaction in which the offer and context were written; replay reads the store as
    /// of it.
    pub tx: u64,
    /// True when every program of the closure was inside the rules profile; otherwise the
    /// decision is `unchecked` ([[openspec:rule-verification#Rules Profile]]).
    #[serde(default)]
    pub profile_checked: bool,
}

impl Derivation {
    /// A one-line rendering for an annotation.
    pub fn summary(&self) -> String {
        format!(
            "derivation: {} from {} statements by {} (closure {})",
            self.event,
            self.support.len(),
            self.program,
            &self.closure_digest[..self.closure_digest.len().min(12)]
        )
    }
}

/// The suffix of the graph that holds the derived facts about an actor: `<actor>/derived`. It is
/// a graph of its own, not the actor's state graph, because the runtime compares the state graph
/// with the machine's state after every turn and would read a derived fact there as a state change.
pub const DERIVED_GRAPH_SUFFIX: &str = "/derived";

/// The answer of a totality check.
#[derive(Clone, Debug, PartialEq)]
pub enum Totality {
    /// Every checked offer got a decision or an explicit refusal.
    Total {
        /// How many offers were run.
        checked: usize,
    },
    /// An offer no clause covers.
    Gap {
        /// The offered events.
        offer: Vec<String>,
        /// The sample context under which nothing applied.
        context: Vec<(String, Value)>,
    },
    /// The check did not decide, with the reason (never a guess).
    Unknown(String),
}
