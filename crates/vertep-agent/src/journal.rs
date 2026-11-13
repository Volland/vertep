//! The agent journal: every input an agent actor accepts — external events, decisions,
//! refusals, fired timers, model results and tool results — appended before it is processed.
//!
//! Replaying a journal reproduces the actor exactly and consults no decider, provider or tool:
//! their answers are inputs ([[decisions#Event-sourced actors]]). Entries are numbered from 1;
//! entry 1 starts the actor under a chart version. An append names its expected position
//! (compare-and-set), so two writers cannot interleave.

use std::collections::BTreeMap;
use std::sync::Mutex;

use oxrdf::{Literal, NamedNode, TermRef, Triple};
use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{Event, Value};
use vertep_store::{AppendError, AppendMeta, Entry, JournalRecord, JournalStore, Position};

use crate::decider::Annotations;
use crate::llm::{Cost, ErrorClass, ToolCallRequest, Usage};
use crate::tool::ToolOutcome;

/// One accepted input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AgentInput {
    /// Starts the actor under `version`.
    Start {
        /// The chart version IRI.
        version: String,
    },
    /// An external event.
    Event {
        /// The event.
        event: Event,
        /// Its message id (deduplication).
        message_id: String,
    },
    /// A decider's decision (`vt.decision`).
    Decision {
        /// The offer's correlation id.
        correlation: String,
        /// The chosen event.
        event: String,
        /// The payload.
        payload: Value,
        /// Annotations.
        annotations: Annotations,
    },
    /// A decider's refusal.
    Refusal {
        /// The offer's correlation id.
        correlation: String,
        /// Why.
        reason: String,
    },
    /// A durable timer fired.
    Timer {
        /// The timer IRI.
        iri: String,
    },
    /// A model result (`llm.result`).
    LlmResult {
        /// The invoke id.
        invokeid: String,
        /// The attempt (1-based).
        attempt: u32,
        /// The provider binding.
        provider: String,
        /// The hash of the rendered prompt.
        prompt_hash: String,
        /// The completion content.
        content: String,
        /// Token usage.
        usage: Usage,
        /// The provider's model id.
        model: String,
        /// Measured latency.
        latency_ms: i64,
        /// The cost, when reported.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost: Option<Cost>,
        /// Tool calls the model asked for.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCallRequest>,
    },
    /// A failed model call.
    LlmError {
        /// The invoke id.
        invokeid: String,
        /// The attempt (1-based).
        attempt: u32,
        /// The class.
        class: ErrorClass,
        /// A description.
        message: String,
        /// The provider's retry-after hint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<i64>,
    },
    /// A tool's result.
    ToolResult {
        /// The invoke id, or the correlation of a `<send>` to the tool.
        invokeid: String,
        /// The tool IRI.
        tool: String,
        /// The outcome.
        outcome: ToolOutcome,
    },
}

impl AgentInput {
    /// A short name of the input kind.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentInput::Start { .. } => "start",
            AgentInput::Event { .. } => "event",
            AgentInput::Decision { .. } => "decision",
            AgentInput::Refusal { .. } => "refusal",
            AgentInput::Timer { .. } => "timer",
            AgentInput::LlmResult { .. } => crate::vocab::LLM_RESULT,
            AgentInput::LlmError { .. } => "llm.error",
            AgentInput::ToolResult { .. } => "tool.result",
        }
    }
}

/// One journal entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// The position, from 1.
    pub position: u64,
    /// When the input was accepted, in milliseconds since the epoch.
    pub ts: i64,
    /// The input.
    pub input: AgentInput,
}

impl JournalEntry {
    /// The entry as JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// An entry from JSON text.
    ///
    /// # Errors
    ///
    /// The JSON error.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Why an append failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    /// The journal's head is not the expected position minus one.
    Conflict {
        /// The current head.
        head: u64,
    },
    /// The backend failed.
    Backend(String),
}

impl core::fmt::Display for JournalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JournalError::Conflict { head } => write!(f, "append conflict at head {head}"),
            JournalError::Backend(e) => write!(f, "journal backend: {e}"),
        }
    }
}

impl std::error::Error for JournalError {}

/// The storage of agent journals.
// @tg: implements:: [[openspec:deciders#Decisions Are Journaled Inputs]]
// @tg: implements:: [[openspec:llm-actors#Journaled Model Results]]
pub trait AgentJournal: Send + Sync {
    /// Appends `entry` at `entry.position`, which must be the head plus one.
    ///
    /// # Errors
    ///
    /// [`JournalError::Conflict`] when another entry is at that position.
    fn append(&self, actor: &str, entry: JournalEntry) -> Result<(), JournalError>;
    /// The entries of `actor`, in order; empty when the journal cannot be read (use
    /// [`AgentJournal::try_read`] to see the failure).
    fn read(&self, actor: &str) -> Vec<JournalEntry>;
    /// The entries of `actor`, in order, or why they could not be read. The host replays
    /// through this method, so a failing store never looks like an empty journal.
    ///
    /// # Errors
    ///
    /// [`JournalError::Backend`] when the backend fails or holds an unreadable entry.
    fn try_read(&self, actor: &str) -> Result<Vec<JournalEntry>, JournalError> {
        Ok(self.read(actor))
    }
    /// Every actor with a journal.
    fn actors(&self) -> Vec<String>;
}

/// An in-memory journal; entries are kept as JSON text, as a durable store would.
#[derive(Debug, Default)]
pub struct MemoryJournal {
    inner: Mutex<BTreeMap<String, Vec<String>>>,
}

impl MemoryJournal {
    /// An empty journal.
    pub fn new() -> Self {
        MemoryJournal::default()
    }
}

impl AgentJournal for MemoryJournal {
    fn append(&self, actor: &str, entry: JournalEntry) -> Result<(), JournalError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| JournalError::Backend("poisoned".into()))?;
        let list = g.entry(actor.to_owned()).or_default();
        let head = list.len() as u64;
        if entry.position != head + 1 {
            return Err(JournalError::Conflict { head });
        }
        list.push(entry.to_json());
        Ok(())
    }

    fn read(&self, actor: &str) -> Vec<JournalEntry> {
        self.inner
            .lock()
            .map(|g| {
                g.get(actor)
                    .map(|l| {
                        l.iter()
                            .filter_map(|t| JournalEntry::from_json(t).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    fn actors(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// The predicate of the JSON text of an agent input in a stored journal entry.
pub const AGENT_INPUT: &str = "https://w3id.org/vertep/vt#agentInput";

/// An agent journal in a [`JournalStore`]: each [`JournalEntry`] is one store entry of the
/// agent's journal (its address must be an IRI), named `<actor>/agent-input/<position>` and
/// holding the entry's JSON text as `vt:agentInput`. Appends are compare-and-set at the head,
/// so agents survive restarts on a durable store (tiramemsu) and can be forked from it, and a
/// durable object's custody journal can carry an agent as well.
#[derive(Clone)]
pub struct StoreAgentJournal {
    store: std::sync::Arc<dyn JournalStore>,
}

impl core::fmt::Debug for StoreAgentJournal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoreAgentJournal").finish_non_exhaustive()
    }
}

impl StoreAgentJournal {
    /// An agent journal in `store`.
    pub fn new(store: std::sync::Arc<dyn JournalStore>) -> Self {
        StoreAgentJournal { store }
    }

    fn address(actor: &str) -> Result<NamedNode, JournalError> {
        NamedNode::new(actor)
            .map_err(|e| JournalError::Backend(format!("agent address {actor} is not an IRI: {e}")))
    }

    fn decode(record: &JournalRecord) -> Result<JournalEntry, JournalError> {
        let p = NamedNode::new_unchecked(AGENT_INPUT);
        let text = record
            .entry
            .triples()
            .objects_for_subject_predicate(record.entry.name(), &p)
            .find_map(|o| match o {
                TermRef::Literal(l) => Some(l.value().to_owned()),
                _ => None,
            })
            .ok_or_else(|| {
                JournalError::Backend(format!(
                    "entry {} at {} has no vt:agentInput",
                    record.entry.name(),
                    record.position.get()
                ))
            })?;
        let entry = JournalEntry::from_json(&text)
            .map_err(|e| JournalError::Backend(format!("entry {}: {e}", record.entry.name())))?;
        if entry.position != record.position.get() {
            return Err(JournalError::Backend(format!(
                "entry {} states position {}, stored at {}",
                record.entry.name(),
                entry.position,
                record.position.get()
            )));
        }
        Ok(entry)
    }
}

// @tg: implements:: [[openspec:durable-objects#Durable Agent Journal]]
impl AgentJournal for StoreAgentJournal {
    fn append(&self, actor: &str, entry: JournalEntry) -> Result<(), JournalError> {
        let address = Self::address(actor)?;
        let name = NamedNode::new(format!("{actor}/agent-input/{}", entry.position))
            .map_err(|e| JournalError::Backend(e.to_string()))?;
        let stored = Entry::new(
            name.clone(),
            [Triple::new(
                name,
                NamedNode::new_unchecked(AGENT_INPUT),
                Literal::new_simple_literal(entry.to_json()),
            )],
        );
        let expected = Position::new(entry.position.saturating_sub(1));
        match self
            .store
            .append(&address, expected, &[stored], &AppendMeta::none())
        {
            Ok(_) => Ok(()),
            Err(AppendError::Conflict { actual, .. }) => {
                Err(JournalError::Conflict { head: actual.get() })
            }
            Err(e) => Err(JournalError::Backend(e.to_string())),
        }
    }

    fn read(&self, actor: &str) -> Vec<JournalEntry> {
        self.try_read(actor).unwrap_or_default()
    }

    fn try_read(&self, actor: &str) -> Result<Vec<JournalEntry>, JournalError> {
        let address = Self::address(actor)?;
        self.store
            .read(&address, Position::ZERO)
            .map_err(|e| JournalError::Backend(e.to_string()))?
            .iter()
            .map(Self::decode)
            .collect()
    }

    fn actors(&self) -> Vec<String> {
        self.store
            .actors()
            .map(|a| a.into_iter().map(|n| n.into_string()).collect())
            .unwrap_or_default()
    }
}
