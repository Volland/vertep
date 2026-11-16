//! The explanation view: for an actor and a journal position, what state the process was in, what
//! was offered, what was chosen and by whom, what became of the answer, and what followed. It is
//! derived by replay from the journal and the registered chart versions and never stored, so it
//! cannot disagree with the journal ([[openspec:decision-records#Explanation View]]).

use serde::Serialize;

/// Why the rationale of a decider is kept apart: it is the decider's own account, not something
/// the system established ([[openspec:decision-records#Rationale Is Labelled As A Claim]]).
pub const CLAIM_LABEL: &str = "unverified claim by the decider";

/// The decider's account of its choice.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Claim {
    /// Who claims it: the decider binding, or `unknown` for an entry that does not say.
    pub claimed_by: String,
    /// The text.
    pub rationale: Option<String>,
    /// The decider's stated confidence, in `[0, 1]`.
    pub confidence: Option<f64>,
    /// Always [`CLAIM_LABEL`].
    pub label: &'static str,
}

/// What the entry says about a decision or a refusal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionRecord {
    /// The offer's correlation id.
    pub correlation: String,
    /// The events that were offered, in offer order; empty when the entry predates offers.
    pub offered: Vec<String>,
    /// False when the entry was written before offers were recorded.
    pub offered_known: bool,
    /// The event the decider chose; `None` for a refusal.
    pub chosen: Option<String>,
    /// What the shell made of it, or `unknown` for an entry written before dispositions.
    pub disposition: String,
    /// The decider binding.
    pub decider: Option<String>,
    /// The decider kind.
    pub decider_kind: Option<String>,
    /// Whether the fallback decider answered.
    pub fallback: bool,
    /// The transitions the chosen event took in its first microstep (source state, index).
    pub taken: Vec<(String, usize)>,
    /// The decider's account.
    pub claim: Option<Claim>,
    /// The derivation the decision rests on, when the decider's reason is a derivation over
    /// recorded statements (a rule decider). Not a claim: it can be re-run.
    pub derivation: Option<serde_json::Value>,
}

fn is_verified(s: &&str) -> bool {
    *s == "verified"
}

/// The explanation of one journal entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Explanation {
    /// The actor.
    pub actor: String,
    /// The journal position explained.
    pub position: u64,
    /// When the input was accepted, in milliseconds since the epoch.
    pub ts: i64,
    /// The kind of input (`event`, `decision`, `refusal`, `timer`, ...).
    pub input: String,
    /// The chart version in force (from the first entry).
    pub chart_version: String,
    /// `verified` when the replay behind this view read every recorded value, `recorded` when
    /// some value was erased: the replay then went on from the recorded inputs and is not a
    /// full verification ([[openspec:journal-redaction#Replay After Erasure]]).
    #[serde(skip_serializing_if = "is_verified")]
    pub replay: &'static str,
    /// The state configuration before the entry.
    pub state_before: Vec<String>,
    /// The state configuration after the entry.
    pub state_after: Vec<String>,
    /// States entered by the entry.
    pub entered: Vec<String>,
    /// States exited by the entry.
    pub exited: Vec<String>,
    /// What the entry asked the host to do, one line each.
    pub effects: Vec<String>,
    /// Present when the entry is a decision or a refusal.
    pub decision: Option<DecisionRecord>,
}

impl Explanation {
    /// The explanation as JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

pub(crate) fn effect_line(e: &crate::machine::AgentEffect) -> String {
    use crate::machine::AgentEffect as E;
    match e {
        E::Offer(r) => format!(
            "offer {} to `{}`: {}",
            r.correlation,
            r.binding,
            r.candidates
                .iter()
                .map(|c| c.event.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        E::CancelOffer { correlation, .. } => format!("cancel offer {correlation}"),
        E::Timer(t) => format!("arm timer {}", t.iri),
        E::CancelTimer(i) => format!("cancel timer {i}"),
        E::StartLlm(s) => format!("start model call {}", s.invokeid),
        E::StartTool { tool, .. } => format!("call tool {tool}"),
        E::CancelInvoke(i) => format!("cancel invoke {i}"),
        E::Send { target, event, .. } => format!("send `{}` to {target}", event.name),
        E::Deliver { event, .. } => format!("deliver `{}` to itself", event.name),
        E::Done(_) => "reached a final state".to_owned(),
    }
}
