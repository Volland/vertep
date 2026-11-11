//! The decider contract and its four kinds: guard, LLM, human and actor deciders.
//!
//! A [`Decider`] receives a [`DecisionRequest`] — the decide state, the candidate set with
//! payload schemas, an immutable [`ContextView`], the actor IRI and a correlation id — and
//! eventually yields a [`DecisionOutcome`]: one [`Decision`] or a [`Refusal`]. It can do nothing
//! else: whatever it returns is validated by the agent machine against the offer, so decider
//! output is untrusted ([[decisions#Offer-time decider safety]]).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use vertep_core::datamodel::{Datamodel, Event, EventType, SessionInfo, StateQuery, Value};
use vertep_runtime::{Address, NodeHandle};

use crate::enabled::Candidate;
use crate::json::{field, from_json, obj, s, to_json};
use crate::llm::{GenerationParams, Provider, ProviderRequest, RenderedPrompt};

/// A decision correlation id.
pub type CorrelationId = String;

/// Annotations a decision may carry. They are recorded in the graph and never influence
/// transition selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    /// Why the decider chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Confidence in `[0, 1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// The decider kind (`guard`, `llm`, `human`, `actor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decider_kind: Option<String>,
    /// The decider binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decider: Option<String>,
    /// The model identifier (LLM deciders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The prompt hash (LLM deciders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
    /// Whether a fallback decider answered after the deadline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    /// The token usage of the model call behind the decision (LLM deciders); it is added to
    /// the actor's cumulative usage and recorded as a `vt-agent:LlmCall`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::llm::Usage>,
}

impl Annotations {
    /// Checks the annotation values (confidence in `[0, 1]`).
    ///
    /// # Errors
    ///
    /// A description of the bad value.
    // @tg: implements:: [[openspec:deciders#Decision Annotations]]
    pub fn validate(&self) -> Result<(), String> {
        match self.confidence {
            Some(c) if !(0.0..=1.0).contains(&c) || c.is_nan() => {
                Err(format!("confidence {c} is outside [0, 1]"))
            }
            _ => Ok(()),
        }
    }
}

/// A read-only view of an actor's datamodel. Cloning shares it; nothing can write through it,
/// so a decider's changes to its own copy never reach the chart.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextView {
    snapshot: Arc<Value>,
    data: Arc<Value>,
}

/// The plain data items of a datamodel snapshot: a CEL snapshot's `ctx` decoded to plain
/// values, any other snapshot as it is.
fn plain_data(snapshot: &Value) -> Value {
    let cel = field(snapshot, "format")
        == Some(&Value::String(vertep_dm_cel::SNAPSHOT_FORMAT.to_owned()));
    if cel {
        let mut dm = vertep_dm_cel::CelDatamodel::default();
        if dm.restore(snapshot).is_ok()
            && let Ok(v) = dm.eval_value("ctx", &NoStates)
        {
            return v;
        }
    }
    snapshot.clone()
}

impl ContextView {
    /// A view of a datamodel snapshot.
    pub fn new(snapshot: Value) -> Self {
        ContextView {
            data: Arc::new(plain_data(&snapshot)),
            snapshot: Arc::new(snapshot),
        }
    }

    /// The whole snapshot, in the datamodel's own encoding.
    pub fn snapshot(&self) -> &Value {
        &self.snapshot
    }

    /// The data items as plain values (`ctx` of a CEL datamodel).
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// The data item at a dotted path.
    pub fn get(&self, path: &str) -> Option<&Value> {
        crate::json::at_path(self.data(), path)
    }
}

/// What a decider is asked.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRequest {
    /// The decide state id.
    pub state: String,
    /// The candidate set with payload schemas.
    pub candidates: Vec<Candidate>,
    /// The context at offer time.
    pub context: ContextView,
    /// The actor IRI.
    pub actor: String,
    /// The correlation id the answer must carry.
    pub correlation: CorrelationId,
    /// The decider binding consulted.
    pub binding: String,
    /// Whether this is a fallback consultation after a deadline.
    pub fallback: bool,
}

impl DecisionRequest {
    /// The request as JSON (for actor deciders, inboxes and prompts).
    pub fn to_json(&self) -> Json {
        json!({
            "state": self.state,
            "actor": self.actor,
            "correlation": self.correlation,
            "candidates": self.candidates.iter().map(|c| json!({
                "event": c.event,
                "schema": c.schema,
            })).collect::<Vec<_>>(),
            "context": to_json(self.context.data()),
        })
    }
}

/// One chosen event with its payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// The chosen event name.
    pub event: String,
    /// The payload.
    pub payload: Value,
    /// Annotations.
    pub annotations: Annotations,
}

impl Decision {
    /// A decision for `event` with `payload` and no annotations.
    pub fn new(event: impl Into<String>, payload: Value) -> Self {
        Decision {
            event: event.into(),
            payload,
            annotations: Annotations::default(),
        }
    }

    /// Reads `{event, payload, rationale?, confidence?}`.
    pub fn from_value(v: &Value) -> Option<Decision> {
        let Some(Value::String(event)) = field(v, "event") else {
            return None;
        };
        let payload = field(v, "payload").cloned().unwrap_or(Value::Undefined);
        let annotations = Annotations {
            rationale: match field(v, "rationale") {
                Some(Value::String(r)) => Some(r.clone()),
                _ => None,
            },
            confidence: match field(v, "confidence") {
                Some(Value::Number(c)) => Some(*c),
                _ => None,
            },
            ..Annotations::default()
        };
        Some(Decision {
            event: event.clone(),
            payload,
            annotations,
        })
    }
}

/// A decider declining to choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Why.
    pub reason: String,
}

/// What a decider yields.
#[derive(Clone, Debug, PartialEq)]
pub enum DecisionOutcome {
    /// A chosen event.
    Decision(Decision),
    /// A refusal.
    Refusal(Refusal),
}

impl DecisionOutcome {
    /// A refusal with `reason`.
    pub fn refuse(reason: impl Into<String>) -> Self {
        DecisionOutcome::Refusal(Refusal {
            reason: reason.into(),
        })
    }
}

/// The four decider kinds, and any other implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeciderKind {
    /// Deterministic decision guards.
    Guard,
    /// A language model.
    Llm,
    /// A person through an inbox.
    Human,
    /// Another actor through `vt:request`.
    Actor,
    /// A user-supplied implementation.
    Custom,
}

impl DeciderKind {
    /// The kind's name.
    pub fn as_str(self) -> &'static str {
        match self {
            DeciderKind::Guard => "guard",
            DeciderKind::Llm => "llm",
            DeciderKind::Human => "human",
            DeciderKind::Actor => "actor",
            DeciderKind::Custom => "custom",
        }
    }
}

/// The decider contract. A decider receives an owned request with an immutable context view
/// and can only return a value; it cannot mutate the chart, the context or the store.
// @tg: implements:: [[openspec:deciders#Decider Contract]]
pub trait Decider: Send + Sync {
    /// The decider's kind.
    fn kind(&self) -> DeciderKind;
    /// Decides; the future may take arbitrarily long (the actor keeps running).
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome>;
    /// The request `correlation` was cancelled or superseded; a later answer is stale.
    fn cancel(&self, _correlation: &str) {}
}

/// Decider bindings by name.
#[derive(Clone, Default)]
pub struct DeciderRegistry {
    map: BTreeMap<String, Arc<dyn Decider>>,
}

impl std::fmt::Debug for DeciderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeciderRegistry")
            .field("bindings", &self.map.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl DeciderRegistry {
    /// Registers `decider` under `binding`.
    pub fn insert(&mut self, binding: impl Into<String>, decider: Arc<dyn Decider>) {
        self.map.insert(binding.into(), decider);
    }

    /// The decider bound to `binding`.
    pub fn get(&self, binding: &str) -> Option<&Arc<dyn Decider>> {
        self.map.get(binding)
    }

    /// Whether `binding` is registered.
    pub fn contains(&self, binding: &str) -> bool {
        self.map.contains_key(binding)
    }
}

// ----- guard decider ---------------------------------------------------------------------

/// One rule of a guard decider: an event, an optional CEL guard over the context (`ctx`), and
/// an optional CEL expression for the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardRule {
    /// The event chosen when the guard holds.
    pub event: String,
    /// The guard; `None` always holds.
    pub guard: Option<String>,
    /// The payload expression; `None` gives no payload.
    pub payload: Option<String>,
}

/// Picks the first candidate in document order whose decision guard holds. Guards are CEL
/// expressions over the context view (`ctx.x`), evaluated by a fresh CEL datamodel, so the
/// same candidates and context always give the same decision.
#[derive(Clone, Debug, Default)]
pub struct GuardDecider {
    rules: Vec<GuardRule>,
}

struct NoStates;

impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

impl GuardDecider {
    /// A guard decider that uses each candidate's own `vt:decisionGuard` (from the chart).
    pub fn from_chart() -> Self {
        GuardDecider::default()
    }

    /// A decider with explicit rules: only events with a rule are chosen, and a rule without
    /// its own guard uses the candidate's chart-declared `vt:decisionGuard`, if any.
    pub fn with_rules(rules: Vec<GuardRule>) -> Self {
        GuardDecider { rules }
    }

    /// Chooses the first candidate (in document order) among `events`, unconditionally.
    pub fn first_enabled<I: IntoIterator<Item = S>, S: Into<String>>(events: I) -> Self {
        GuardDecider {
            rules: events
                .into_iter()
                .map(|e| GuardRule {
                    event: e.into(),
                    guard: None,
                    payload: None,
                })
                .collect(),
        }
    }

    fn datamodel(context: &ContextView) -> vertep_dm_cel::CelDatamodel {
        let mut dm = vertep_dm_cel::CelDatamodel::default();
        let _ = dm.init_session(&SessionInfo::default());
        let snap = context.snapshot();
        if dm.restore(snap).is_err()
            && let Value::Object(fields) = context.data()
        {
            for (k, v) in fields {
                let _ = dm.bind_var(k, v.clone());
            }
        }
        dm.set_event(&Event::named("vt.decide", EventType::Internal));
        dm
    }

    /// The decision for `request`, synchronously.
    pub fn choose(&self, request: &DecisionRequest) -> DecisionOutcome {
        let mut dm = Self::datamodel(&request.context);
        for c in &request.candidates {
            let rule = self.rules.iter().find(|r| r.event == c.event);
            let guard = match (rule, &c.decision_guard) {
                (Some(r), _) => r.guard.clone().or_else(|| c.decision_guard.clone()),
                (None, Some(g)) if self.rules.is_empty() => Some(g.clone()),
                _ => continue,
            };
            let holds = match &guard {
                None => true,
                Some(g) => dm.eval_cond(g, &NoStates).unwrap_or(false),
            };
            if !holds {
                continue;
            }
            let payload = match rule.and_then(|r| r.payload.as_deref()) {
                Some(p) => dm.eval_value(p, &NoStates).unwrap_or(Value::Undefined),
                None => Value::Undefined,
            };
            let mut d = Decision::new(c.event.clone(), payload);
            d.annotations.rationale = Some(match &guard {
                Some(g) => format!("decision guard `{g}` holds"),
                None => "first enabled candidate".to_owned(),
            });
            return DecisionOutcome::Decision(d);
        }
        DecisionOutcome::refuse("no decision guard holds")
    }
}

impl Decider for GuardDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Guard
    }

    // @tg: implements:: [[openspec:deciders#Decider Kinds]]
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let out = self.choose(&request);
        Box::pin(async move { out })
    }
}

// ----- human decider ----------------------------------------------------------------------

type Pending = (
    DecisionRequest,
    tokio::sync::oneshot::Sender<DecisionOutcome>,
);

/// A human inbox: requests wait here until a person answers. Cloning shares the inbox.
#[derive(Clone, Default)]
pub struct Inbox {
    inner: Arc<Mutex<BTreeMap<String, Pending>>>,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inbox")
            .field("pending", &self.pending().len())
            .finish()
    }
}

impl Inbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Pending>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The requests waiting for an answer, by correlation id order.
    pub fn pending(&self) -> Vec<DecisionRequest> {
        self.lock().values().map(|(r, _)| r.clone()).collect()
    }

    /// Answers the request `correlation`; false when it is not pending (answered, cancelled
    /// or unknown).
    pub fn answer(&self, correlation: &str, outcome: DecisionOutcome) -> bool {
        match self.lock().remove(correlation) {
            Some((_, tx)) => tx.send(outcome).is_ok(),
            None => false,
        }
    }

    fn cancel(&self, correlation: &str) {
        self.lock().remove(correlation);
    }
}

/// Publishes each request to an [`Inbox`] and waits for a person's answer.
#[derive(Clone, Debug)]
pub struct HumanDecider {
    inbox: Inbox,
}

impl HumanDecider {
    /// A human decider over `inbox`.
    pub fn new(inbox: Inbox) -> Self {
        HumanDecider { inbox }
    }
}

impl Decider for HumanDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Human
    }

    // @tg: implements:: [[openspec:deciders#Decider Kinds]]
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inbox
            .lock()
            .insert(request.correlation.clone(), (request, tx));
        Box::pin(async move {
            match rx.await {
                Ok(o) => o,
                Err(_) => DecisionOutcome::refuse("the request was cancelled"),
            }
        })
    }

    fn cancel(&self, correlation: &str) {
        self.inbox.cancel(correlation);
    }
}

// ----- actor decider ----------------------------------------------------------------------

/// Sends each request to another actor as a non-blocking request (`vt.decide.request`, with
/// the request — including its correlation id — as data) and reads the reply's data as
/// `{event, payload, rationale?, confidence?}`.
#[derive(Clone)]
pub struct ActorDecider {
    handle: NodeHandle,
    target: Address,
    timeout: Duration,
}

impl std::fmt::Debug for ActorDecider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorDecider")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl ActorDecider {
    /// An actor decider asking `target` through `handle`, waiting at most `timeout` (on the
    /// node's clock) for the reply.
    pub fn new(handle: NodeHandle, target: Address, timeout: Duration) -> Self {
        ActorDecider {
            handle,
            target,
            timeout,
        }
    }
}

impl Decider for ActorDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Actor
    }

    // @tg: implements:: [[openspec:deciders#Decider Kinds]]
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let handle = self.handle.clone();
        let target = self.target.clone();
        let timeout = self.timeout;
        let event = Event {
            name: crate::vocab::DECIDE_REQUEST_EVENT.into(),
            data: from_json(&request.to_json()),
            ..Event::default()
        };
        Box::pin(async move {
            match handle.ask(&target, event, timeout).await {
                Ok(reply) => match Decision::from_value(&reply.data) {
                    Some(d) => DecisionOutcome::Decision(d),
                    None => DecisionOutcome::refuse("the actor's reply is not a decision"),
                },
                Err(e) => DecisionOutcome::refuse(format!("actor decider: {e}")),
            }
        })
    }
}

// ----- LLM decider ------------------------------------------------------------------------

/// The structured-output schema of an LLM decision: `oneOf` over the candidates, each branch
/// `{event: const, payload: <candidate schema>}` plus optional `rationale` and `confidence`.
pub fn decision_schema(candidates: &[Candidate]) -> Json {
    let branches: Vec<Json> = candidates
        .iter()
        .map(|c| {
            json!({
                "type": "object",
                "properties": {
                    "event": { "const": c.event },
                    "payload": c.schema,
                    "rationale": { "type": "string" },
                    "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                },
                "required": ["event"],
                "additionalProperties": false,
            })
        })
        .collect();
    json!({ "oneOf": branches })
}

/// Asks a model for a structured decision restricted to the candidate set.
#[derive(Clone)]
pub struct LlmDecider {
    provider: Arc<dyn Provider>,
    provider_name: String,
    model: String,
    params: GenerationParams,
    instructions: String,
}

impl std::fmt::Debug for LlmDecider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmDecider")
            .field("provider", &self.provider_name)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl LlmDecider {
    /// An LLM decider calling `model` on `provider` (registered as `provider_name`).
    pub fn new(
        provider_name: impl Into<String>,
        provider: Arc<dyn Provider>,
        model: impl Into<String>,
    ) -> Self {
        LlmDecider {
            provider,
            provider_name: provider_name.into(),
            model: model.into(),
            params: GenerationParams::default(),
            instructions: "Choose exactly one of the candidate events for the decide state.".into(),
        }
    }

    /// Task instructions placed before the request in the prompt.
    pub fn with_instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = text.into();
        self
    }

    /// Generation parameters.
    pub fn with_params(mut self, params: GenerationParams) -> Self {
        self.params = params;
        self
    }

    /// The prompt for `request`: the instructions and the request as canonical JSON.
    pub fn prompt(&self, request: &DecisionRequest) -> RenderedPrompt {
        RenderedPrompt::new(format!(
            "{}\nAnswer with JSON {{\"event\", \"payload\", \"rationale\", \"confidence\"}}.\nRequest: {}",
            self.instructions,
            crate::json::canonical(&request.to_json())
        ))
    }
}

impl Decider for LlmDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Llm
    }

    // @tg: implements:: [[openspec:deciders#Decider Kinds]]
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let prompt = self.prompt(&request);
        let req = ProviderRequest {
            prompt: prompt.clone(),
            model: self.model.clone(),
            params: self.params.clone(),
            output_schema: Some(decision_schema(&request.candidates)),
            tools: Vec::new(),
        };
        let provider = self.provider.clone();
        Box::pin(async move {
            match provider.complete(req).await {
                Ok(r) => {
                    let parsed: Option<Json> = serde_json::from_str(&r.content).ok();
                    match parsed
                        .as_ref()
                        .map(from_json)
                        .as_ref()
                        .and_then(Decision::from_value)
                    {
                        Some(mut d) => {
                            d.annotations.model = Some(r.model);
                            d.annotations.prompt_hash = Some(prompt.hash);
                            d.annotations.usage = Some(r.usage);
                            DecisionOutcome::Decision(d)
                        }
                        None => DecisionOutcome::refuse("the model's answer is not a decision"),
                    }
                }
                Err(e) => DecisionOutcome::refuse(format!("model call failed: {}", e.class)),
            }
        })
    }
}

/// The decision payload `{event, payload}` as a value (for actor deciders' replies).
pub fn decision_value(event: &str, payload: Value) -> Value {
    obj([("event", s(event)), ("payload", payload)])
}
