//! Plain data: the policy and the record of one rule decision. No engine here.

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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The digest of the program closure: node, source text, and each include with the id of its
    /// source statement.
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
