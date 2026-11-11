//! Event names, invoke types, extension names and the `vt-agent:` terms of the agent layer.

use vertep_core::ir::VT_NS;

/// The input that carries a decider's answer to the actor.
pub const DECISION_EVENT: &str = "vt.decision";
/// Raised in a decide state whose candidate set is empty.
pub const DECIDE_NONE_EVENT: &str = "vt.decide.none";
/// Raised in a decide state when its decider refuses.
pub const DECIDE_REFUSED_EVENT: &str = "vt.decide.refused";
/// Raised for a decision that is not accepted (reason `not-a-candidate`, `payload-schema`,
/// `stale` or `annotation`).
pub const DECIDE_INVALID_EVENT: &str = "error.decide.invalid";
/// Raised when a decision deadline passes without a fallback.
pub const DECIDE_TIMEOUT_EVENT: &str = "error.decide.timeout";
/// Raised when a decide state has reached its `vt:maxDecisions` bound.
pub const BUDGET_EXHAUSTED_EVENT: &str = "vt.agent.budget-exhausted";
/// Raised for a failed or invalid model call.
pub const LLM_ERROR_EVENT: &str = "error.llm";
/// The journal kind of a model result.
pub const LLM_RESULT: &str = "llm.result";
/// A streamed partial model result (never journaled).
pub const LLM_PARTIAL_EVENT: &str = "vt.llm.partial";
/// Raised for a failed tool call.
pub const TOOL_ERROR_EVENT: &str = "error.tool";
/// Raised for a rejected or failed plan.
pub const PLAN_ERROR_EVENT: &str = "error.plan";
/// The event that carries a proposed plan.
pub const PLAN_PROPOSED_EVENT: &str = "vt.plan.proposed";
/// Raised in the sender of an ungranted send.
pub const COMMUNICATION_ERROR_EVENT: &str = "error.communication";
/// Raised for execution failures (unknown provider, template errors).
pub const EXECUTION_ERROR_EVENT: &str = "error.execution";
/// The reply event of a request (tool results delivered for a `<send>` to a tool).
pub const REPLY_EVENT: &str = vertep_runtime::vocab::REPLY_EVENT;
/// The event an actor decider receives.
pub const DECIDE_REQUEST_EVENT: &str = "vt.decide.request";
/// Delivered to a virtual actor whose self-modification proposal was rejected.
pub const SELF_MODIFICATION_REJECTED_EVENT: &str = "vt.self-modification.rejected";
/// Delivered to the parent when an invoked actor's send was refused by its grants.
pub const GRANT_DENIED_EVENT: &str = "vt.grant.denied";

/// The invoke type of an LLM call.
pub const LLM_INVOKE_TYPE: &str = "https://w3id.org/vertep/vt#llm";
/// The invoke type of a tool call.
pub const TOOL_INVOKE_TYPE: &str = "https://w3id.org/vertep/vt#tool";
/// The invoke type of a plan chart.
pub const PLAN_INVOKE_TYPE: &str = "https://w3id.org/vertep/vt#plan";

/// Whether `t` names the invoke type `full` (its IRI or its `vt:` short form).
pub fn is_type(t: &str, full: &str) -> bool {
    t == full
        || full
            .strip_prefix(VT_NS)
            .is_some_and(|l| t == format!("vt:{l}"))
}

/// The reserved data item that exposes cumulative model usage to the chart.
pub const USAGE_VARIABLE: &str = "vt_usage";

/// The `vt-agent:` namespace.
pub const AGENT_NS: &str = "https://w3id.org/vertep/agent#";
/// The PROV-O namespace.
pub const PROV_NS: &str = "http://www.w3.org/ns/prov#";
/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

macro_rules! agent_terms {
    ($($(#[$m:meta])* $name:ident = $local:literal;)*) => {
        $($(#[$m])* pub const $name: &str = concat!("https://w3id.org/vertep/agent#", $local);)*
        /// Every `vt-agent:` term this module names, for checks against the ontology file.
        pub const ALL: &[&str] = &[$($name),*];
    };
}

agent_terms! {
    /// `vt-agent:Decider`.
    DECIDER = "Decider";
    /// `vt-agent:Decision`.
    DECISION = "Decision";
    /// `vt-agent:LlmCall`.
    LLM_CALL = "LlmCall";
    /// `vt-agent:Tool`.
    TOOL = "Tool";
    /// `vt-agent:ToolCall`.
    TOOL_CALL = "ToolCall";
    /// `vt-agent:Grant`.
    GRANT = "Grant";
    /// `vt-agent:Plan`.
    PLAN = "Plan";
    /// `vt-agent:PlanExecution`.
    PLAN_EXECUTION = "PlanExecution";
    /// `vt-agent:chose`.
    CHOSE = "chose";
    /// `vt-agent:rationale`.
    RATIONALE = "rationale";
    /// `vt-agent:confidence`.
    CONFIDENCE = "confidence";
    /// `vt-agent:deciderKind`.
    DECIDER_KIND = "deciderKind";
    /// `vt-agent:model`.
    MODEL = "model";
    /// `vt-agent:promptHash`.
    PROMPT_HASH = "promptHash";
    /// `vt-agent:fallback`.
    FALLBACK = "fallback";
    /// `vt-agent:correlationId`.
    CORRELATION_ID = "correlationId";
    /// `vt-agent:decideState`.
    DECIDE_STATE = "decideState";
    /// `vt-agent:actor`.
    ACTOR = "actor";
    /// `vt-agent:binding`.
    BINDING = "binding";
    /// `vt-agent:invokeId`.
    INVOKE_ID = "invokeId";
    /// `vt-agent:provider`.
    PROVIDER = "provider";
    /// `vt-agent:promptTokens`.
    PROMPT_TOKENS = "promptTokens";
    /// `vt-agent:completionTokens`.
    COMPLETION_TOKENS = "completionTokens";
    /// `vt-agent:latencyMs`.
    LATENCY_MS = "latencyMs";
    /// `vt-agent:cost`.
    COST = "cost";
    /// `vt-agent:currency`.
    CURRENCY = "currency";
    /// `vt-agent:tool`.
    TOOL_P = "tool";
    /// `vt-agent:outcome`.
    OUTCOME = "outcome";
    /// `vt-agent:contentHash`.
    CONTENT_HASH = "contentHash";
    /// `vt-agent:violation`.
    VIOLATION = "violation";
    /// `vt-agent:invokingState`.
    INVOKING_STATE = "invokingState";
    /// `vt-agent:allows`.
    ALLOWS = "allows";
    /// `vt-agent:decider`.
    DECIDER_P = "decider";
    /// `vt-agent:executes`.
    EXECUTES = "executes";
    /// `vt-agent:DeniedSend`.
    DENIED_SEND = "DeniedSend";
    /// `vt-agent:accepted`.
    ACCEPTED = "accepted";
    /// `vt-agent:reason`.
    REASON = "reason";
    /// `vt-agent:attempt`.
    ATTEMPT = "attempt";
    /// `vt-agent:generatedBy`.
    GENERATED_BY = "generatedBy";
    /// `vt-agent:sender`.
    SENDER = "sender";
    /// `vt-agent:target`.
    TARGET = "target";
    /// `vt-agent:eventName`.
    EVENT_NAME = "eventName";
}

/// `prov:Agent`.
pub const PROV_AGENT: &str = "http://www.w3.org/ns/prov#Agent";
/// `prov:Activity`.
pub const PROV_ACTIVITY: &str = "http://www.w3.org/ns/prov#Activity";
/// `prov:Entity`.
pub const PROV_ENTITY: &str = "http://www.w3.org/ns/prov#Entity";
/// `prov:wasAssociatedWith`.
pub const PROV_WAS_ASSOCIATED_WITH: &str = "http://www.w3.org/ns/prov#wasAssociatedWith";
/// `prov:wasGeneratedBy`.
pub const PROV_WAS_GENERATED_BY: &str = "http://www.w3.org/ns/prov#wasGeneratedBy";
/// `prov:used`.
pub const PROV_USED: &str = "http://www.w3.org/ns/prov#used";
/// `vt:DecideState`, the chart-module class of a decide state.
pub const VT_DECIDE_STATE: &str = "https://w3id.org/vertep/vt#DecideState";
