//! Agents on Vertep charts: `vt:decide` states and pluggable deciders, LLM and tool actors,
//! capability grants and sandboxed plan charts written by an LLM.
//!
//! Nothing here adds execution semantics to SCXML. Every agent feature lowers onto constructs
//! the core already has: states, `<invoke>`, `<send>`, internal events, durable timers and
//! journal entries.
//!
//! - [`decide`] reads the `vt:decide` binding of a state; [`enabled`] computes its candidate
//!   set with the pure `enabled_events` function; [`decider`] defines the [`Decider`] contract
//!   and its guard, LLM, human and actor kinds.
//! - [`llm`] defines the [`Provider`] contract, prompt templates and the `vt:llm` invoke;
//!   [`tool`] registers tools with typed arguments and results; [`grant`] computes and
//!   enforces capability grants.
//! - [`plan`] parses, validates and content-addresses LLM-written plan charts.
//! - [`machine`] advances one agent actor deterministically from journaled inputs;
//!   [`host`] is its imperative shell: it consults deciders, calls providers and tools, arms
//!   durable timers and journals every input before it is processed ([`journal`]).
//! - [`ontology`] is the `vt-agent:` module (shipped with `vertep-rdf`), its SHACL shapes and
//!   the provenance graph of an agent run.
//!
//! See `tg.md/views/agents.md` for the architecture.
//!
//! @tg: implements:: [[decisions#Deciders at choice points]]
//! @tg: implements:: [[decisions#Offer-time decider safety]] {confidence: partial, note: "checked at runtime; the Lean theorem is owned by vertep-lean"}
//! @tg: implements:: [[decisions#LLM calls as invoked actors first]]
//! @tg: implements:: [[decisions#Plan charts may use the null datamodel]]
//! @tg: implements:: [[decisions#Agent machines record their core macrosteps on request]]

#[cfg(feature = "anthropic")]
pub mod anthropic;
pub mod decide;
pub mod decider;
pub mod enabled;
pub mod grant;
pub mod host;
pub mod journal;
pub mod json;
pub mod llm;
pub mod machine;
pub mod ontology;
pub mod plan;
pub mod schema;
pub mod tool;
pub mod vocab;

pub use decide::{DecideError, DecideSpec};
pub use decider::{
    ActorDecider, Annotations, ContextView, Decider, DeciderKind, DeciderRegistry, Decision,
    DecisionOutcome, DecisionRequest, GuardDecider, HumanDecider, Inbox, LlmDecider, Refusal,
};
pub use enabled::{Candidate, enabled_events};
pub use grant::Grant;
pub use host::{AgentHost, AgentHostBuilder, AgentState, HostError, InstantiateError};
pub use journal::{AgentInput, AgentJournal, JournalEntry, MemoryJournal};
pub use llm::{
    ErrorClass, Provider, ProviderError, ProviderRegistry, ProviderRequest, ProviderResult,
    ScriptedProvider, Usage,
};
pub use machine::{
    AgentChart, AgentEffect, AgentMachine, ChartError, CoreTape, Record, StartOptions, UsageTotals,
};
pub use plan::{AcceptedPlan, PlanBounds, PlanError, PlanErrorClass, PlanFormat};
pub use tool::{Tool, ToolRegistry};
