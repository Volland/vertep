//! The agent machine: one agent actor's session, datamodel and agent state, advanced
//! deterministically by journaled inputs.
//!
//! The machine wraps the pure core (`vertep_core::session::step`) and adds, without changing
//! SCXML semantics, the agent features that lower onto existing constructs:
//!
//! - **Decide states.** After every macrostep it recomputes the candidate set of each active
//!   decide state ([`crate::enabled::enabled_events`]) and offers, re-offers (when the
//!   fingerprint changed or the state was re-entered) or cancels requests. Correlation ids are
//!   derived from the journal position, so replay reproduces them. A `vt.decision` input is
//!   accepted only when its correlation id names the open offer, its event is a candidate, its
//!   payload conforms to the candidate's schema, its annotations are valid and the transition
//!   it would take is the one enabled at offer time; the chosen event is then raised, otherwise
//!   `error.decide.invalid` with a reason.
//! - **LLM, tool and plan invocations** (`vt:llm`, `vt:tool`, `vt:plan` invoke types):
//!   templates are rendered here (pure), calls are effects for the host, and their results come
//!   back as journaled inputs.
//! - **Durable timers** for deadlines, retry backoff, plan durations and delayed sends: part of
//!   the machine state, rebuilt by replay.
//!
//! Every effect is a description; the [`crate::host::AgentHost`] performs them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use serde_json::Value as Json;
use vertep_core::compile::{CompiledChart, Domain, StateIdx};
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, EvalError, Event, EventType, SessionInfo,
    StateQuery, Value,
};
use vertep_core::ir::{Chart, Invoke, State};
use vertep_core::kernel::{descriptor_matches, transition_domain};
use vertep_core::session::{
    self, Effect, Input, InvokeRequest, SendRequest, Session, StartInput, StepError,
};
use vertep_core::tape::{OracleInput, OracleRecord, TapeDatamodel};
use vertep_core::trace::MacrostepTrace;
use vertep_rdf::ChartVersion;

use crate::decide::{DecideError, DecideSpec, decide_specs};
use crate::decider::{Annotations, ContextView, DecisionRequest};
use crate::enabled::{Candidate, enabled_events, fingerprint, selected_transition};
use crate::grant::{self, Grant};
use crate::journal::AgentInput;
use crate::json::{field, obj, s, sha256, to_json};
use crate::llm::{Cost, ErrorClass, GenerationParams, LlmSpec, RenderedPrompt, Usage, render};
use crate::plan::{AcceptedPlan, PlanBounds, PlanError, PlanErrorClass, PlanFormat, submit};
use crate::schema::{EventSchemas, Violation, check_type, validate};
use crate::tool::ToolOutcome;
use crate::vocab;

/// The maximum number of macrosteps one input may cause (follow-up events included).
pub const MAX_STEPS_PER_INPUT: usize = 256;

// ----- the chart -------------------------------------------------------------------------

/// A chart prepared for agent execution: the IR as written, its version, the compiled chart,
/// its decide bindings and its event payload schemas.
pub struct AgentChart {
    /// The chart as written.
    pub ir: Chart,
    /// Its content-addressed version.
    pub version: ChartVersion,
    /// The compiled chart.
    pub compiled: CompiledChart,
    /// The decide bindings, by state id.
    pub decides: BTreeMap<String, DecideSpec>,
    /// The payload schemas of its events.
    pub schemas: EventSchemas,
}

impl std::fmt::Debug for AgentChart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentChart")
            .field("version", &self.version.as_str())
            .field("decides", &self.decides.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Why a chart cannot run as an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChartError {
    /// Invalid decide bindings.
    Decide(Vec<DecideError>),
    /// Core well-formedness violations.
    Invalid(Vec<String>),
    /// A transition on `vt.llm.partial` has a target; partials are not journaled and cannot
    /// drive replayable state changes.
    PartialWithTarget {
        /// The source state.
        state: String,
    },
}

impl core::fmt::Display for ChartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ChartError::Decide(v) => {
                let v: Vec<String> = v.iter().map(ToString::to_string).collect();
                write!(f, "{}", v.join("; "))
            }
            ChartError::Invalid(v) => write!(f, "chart is invalid: {}", v.join("; ")),
            ChartError::PartialWithTarget { state } => write!(
                f,
                "state {state} has a targeted transition on vt.llm.partial; only targetless transitions may handle partials"
            ),
        }
    }
}

impl std::error::Error for ChartError {}

fn check_partials(states: &[State]) -> Result<(), ChartError> {
    for s in states {
        for t in &s.transitions {
            let on_partial = t.events.iter().any(|d| {
                let d = d.trim();
                d != "*" && descriptor_matches(d, vocab::LLM_PARTIAL_EVENT)
            });
            if on_partial && !t.targets.is_empty() {
                return Err(ChartError::PartialWithTarget {
                    state: s.id.clone(),
                });
            }
        }
        check_partials(&s.children)?;
    }
    Ok(())
}

impl AgentChart {
    /// Validates and prepares `chart`.
    ///
    /// # Errors
    ///
    /// [`ChartError`] for invalid decide bindings, targeted partial transitions or core
    /// violations.
    // @tg: implements:: [[openspec:deciders#Decide State Declaration]]
    // @tg: implements:: [[openspec:llm-actors#Streaming Partials]]
    pub fn new(chart: Chart) -> Result<AgentChart, ChartError> {
        let decides = decide_specs(&chart).map_err(ChartError::Decide)?;
        check_partials(&chart.states)?;
        let compiled = CompiledChart::new(chart.clone())
            .map_err(|v| ChartError::Invalid(v.iter().map(|x| format!("{x}")).collect()))?;
        Ok(AgentChart {
            version: vertep_rdf::chart_version(&chart),
            schemas: EventSchemas::of(&chart),
            ir: chart,
            compiled,
            decides,
        })
    }

    fn invoke(&self, state: StateIdx, index: usize) -> Option<&Invoke> {
        self.compiled.state(state).invokes.get(index)
    }

    fn state_ir(&self, id: &str) -> Option<&State> {
        fn find<'a>(states: &'a [State], id: &str) -> Option<&'a State> {
            for s in states {
                if s.id == id {
                    return Some(s);
                }
                if let Some(f) = find(&s.children, id) {
                    return Some(f);
                }
            }
            None
        }
        find(&self.ir.states, id)
    }
}

// ----- effects and records ---------------------------------------------------------------

/// A durable timer of an agent actor.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentTimer {
    /// The timer IRI.
    pub iri: String,
    /// When it is due, in milliseconds since the epoch.
    pub due_ms: i64,
    /// What it does.
    pub kind: TimerKind,
}

/// What a timer does when it fires.
#[derive(Clone, Debug, PartialEq)]
pub enum TimerKind {
    /// A decision deadline.
    Deadline {
        /// The decide state.
        state: String,
        /// The offer.
        correlation: String,
    },
    /// The backoff before the next attempt of an LLM call.
    Retry {
        /// The invoke id.
        invokeid: String,
    },
    /// A plan's duration bound.
    PlanDuration {
        /// The plan's invoke id.
        invokeid: String,
    },
    /// A delayed `<send>` to the actor itself.
    DelayedSend {
        /// The send id.
        sendid: String,
        /// The event.
        event: Event,
    },
}

/// A model call for the host to make.
#[derive(Clone, Debug, PartialEq)]
pub struct LlmStart {
    /// The invoke id.
    pub invokeid: String,
    /// The attempt (1-based).
    pub attempt: u32,
    /// The provider binding.
    pub provider: String,
    /// The model.
    pub model: String,
    /// The rendered prompt.
    pub prompt: RenderedPrompt,
    /// Generation parameters.
    pub params: GenerationParams,
    /// The structured output schema.
    pub output_schema: Option<Json>,
    /// The granted tool IRIs offered to the model.
    pub tools: Vec<String>,
    /// Whether to stream partials.
    pub stream: bool,
}

/// What a machine asks its host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEffect {
    /// Ask a decider.
    Offer(DecisionRequest),
    /// An offer was cancelled or superseded.
    CancelOffer {
        /// The offer.
        correlation: String,
        /// The decider binding.
        binding: String,
    },
    /// Arm a durable timer.
    Timer(AgentTimer),
    /// Disarm a timer.
    CancelTimer(String),
    /// Make a model call.
    StartLlm(LlmStart),
    /// Call a tool.
    StartTool {
        /// The invoke id or send correlation.
        invokeid: String,
        /// The tool IRI.
        tool: String,
        /// The arguments.
        args: Value,
    },
    /// Cancel a running LLM or tool call.
    CancelInvoke(String),
    /// Send an event to another target (actor, tool, parent).
    Send {
        /// The target as written.
        target: String,
        /// The event.
        event: Event,
        /// A correlation id for replies.
        correlation: String,
    },
    /// Deliver an event to the actor itself as a new journaled input.
    Deliver {
        /// The deterministic message id.
        message_id: String,
        /// The event.
        event: Event,
    },
    /// The actor reached a top-level final state.
    Done(Value),
}

/// An accepted decision, with the transitions enabled at offer time and those taken.
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptedDecision {
    /// The correlation id.
    pub correlation: String,
    /// The chosen event.
    pub event: String,
    /// The transitions enabled at offer time (source id, index).
    pub offered: Vec<(String, usize)>,
    /// The transitions the chosen event took in its first microstep.
    pub taken: Vec<(String, usize)>,
}

/// What applying one input produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// The effects, in order.
    pub effects: Vec<AgentEffect>,
    /// True when the input was a duplicate and changed nothing.
    pub duplicate: bool,
    /// True when the input belonged to a cancelled or finished invocation and was dropped.
    pub dropped: bool,
    /// Decisions accepted by this input.
    pub accepted: Vec<AcceptedDecision>,
}

/// How [`AgentMachine::start_with`] starts a machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartOptions {
    /// Record the core macrosteps ([`CoreTape`]).
    pub record_core: bool,
    /// The microstep budget of each macrostep; `None` keeps the core default.
    pub microstep_budget: Option<u32>,
}

/// The core macrosteps of the actor's own session, recorded for the Lean oracle by a machine
/// started with [`StartOptions::record_core`]: one [`OracleRecord`] (input, datamodel
/// answer tape, trace) per `session::start` or `session::step`, in order, in the JSON Lines
/// format `vertep-oracle --records` reads.
///
/// Only the core is recorded. What the agent layer does between macrosteps (offers, decision
/// validation on a probe datamodel, binding `vt_usage`) does not go through the session, and
/// the sessions of plan charts are not the actor's; the events the agent layer raises appear
/// as the inputs of the macrosteps they cause.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoreTape {
    /// One record per macrostep.
    pub records: Vec<OracleRecord>,
    /// True when the last recorded macrostep ran out of the microstep budget (its trace is
    /// partial and the machine returned [`MachineError::Step`]).
    pub budget_exceeded: bool,
}

/// A provenance record, projected into the `vt-agent:` graph.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// A decision (accepted or rejected).
    Decision {
        /// Its IRI.
        iri: String,
        /// The actor.
        actor: String,
        /// The decide state.
        state: String,
        /// The correlation id.
        correlation: String,
        /// The chosen event.
        event: String,
        /// The decider binding.
        binding: String,
        /// Annotations.
        annotations: Annotations,
        /// Whether it was accepted.
        accepted: bool,
        /// The rejection reason.
        reason: Option<String>,
    },
    /// A model call.
    LlmCall {
        /// Its IRI.
        iri: String,
        /// The actor.
        actor: String,
        /// The invoke id.
        invokeid: String,
        /// The provider binding.
        provider: String,
        /// The model.
        model: String,
        /// The prompt hash.
        prompt_hash: String,
        /// Usage.
        usage: Usage,
        /// Latency.
        latency_ms: i64,
        /// Cost.
        cost: Option<Cost>,
        /// The attempt.
        attempt: u32,
        /// `ok`, `schema`, or an error class.
        outcome: String,
    },
    /// A tool call.
    ToolCall {
        /// Its IRI.
        iri: String,
        /// The actor.
        actor: String,
        /// The tool.
        tool: String,
        /// `ok` or the error class.
        outcome: String,
    },
    /// A submitted plan.
    Plan {
        /// The plan IRI (its version IRI when accepted, `urn:vertep:plan:<hash>` otherwise).
        iri: String,
        /// The parent actor.
        actor: String,
        /// The invoking state.
        state: String,
        /// The invoke id.
        invokeid: String,
        /// The SHA-256 of the submitted text.
        content_hash: String,
        /// The proposing LLM call or decision IRI.
        generated_by: Option<String>,
        /// Whether validation accepted it.
        accepted: bool,
        /// The violations of a rejected plan.
        violations: Vec<String>,
    },
    /// One execution of an accepted plan.
    PlanExecution {
        /// Its IRI.
        iri: String,
        /// The plan IRI.
        plan: String,
        /// The parent actor.
        actor: String,
        /// `done`, `bounds:<bound>`, `result`, `cancelled`.
        outcome: String,
    },
    /// A send refused by a grant.
    Denied {
        /// Its IRI.
        iri: String,
        /// The sender.
        sender: String,
        /// The target.
        target: String,
        /// The event.
        event: String,
    },
}

/// Why a machine could not apply an input.
#[derive(Clone, Debug, PartialEq)]
pub enum MachineError {
    /// The session has terminated.
    Terminated,
    /// The interpreter failed.
    Step(String),
    /// The datamodel could not be created.
    Datamodel(String),
    /// One input caused more than [`MAX_STEPS_PER_INPUT`] macrosteps.
    Livelock,
}

impl core::fmt::Display for MachineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MachineError::Terminated => f.write_str("the session has terminated"),
            MachineError::Step(e) => write!(f, "step failed: {e}"),
            MachineError::Datamodel(e) => write!(f, "datamodel: {e}"),
            MachineError::Livelock => f.write_str("one input caused too many macrosteps"),
        }
    }
}

impl std::error::Error for MachineError {}

// ----- state -----------------------------------------------------------------------------

/// An outstanding or answered offer of one decide state.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The correlation id.
    pub correlation: String,
    /// The decider binding consulted.
    pub binding: String,
    /// The candidates offered.
    pub candidates: Vec<Candidate>,
    /// Whether the fallback decider was consulted.
    pub fallback: bool,
    /// The deadline timer IRI.
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Slot {
    offer: Option<Offer>,
    open: bool,
    fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Running {
    Llm {
        start: Box<LlmStart>,
        spec: LlmSpec,
        output: Option<(vertep_core::types::Type, Json)>,
    },
    Tool {
        tool: String,
    },
}

/// Cumulative model usage of an actor and its invoked descendants.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsageTotals {
    /// Prompt tokens.
    pub prompt_tokens: u64,
    /// Completion tokens.
    pub completion_tokens: u64,
    /// Model calls.
    pub calls: u64,
    /// Summed cost (in the calls' currencies; one currency is assumed).
    pub cost: f64,
}

impl UsageTotals {
    /// The value bound to `vt_usage`.
    pub fn to_value(self) -> Value {
        obj([
            ("prompt_tokens", Value::Number(self.prompt_tokens as f64)),
            (
                "completion_tokens",
                Value::Number(self.completion_tokens as f64),
            ),
            (
                "total_tokens",
                Value::Number((self.prompt_tokens + self.completion_tokens) as f64),
            ),
            ("calls", Value::Number(self.calls as f64)),
            ("cost", Value::Number(self.cost)),
        ])
    }
}

struct PlanRun {
    state: String,
    plan_iri: String,
    plan: AcceptedPlan,
    compiled: CompiledChart,
    session: Session,
    dm: Box<dyn Datamodel>,
    grant: Grant,
    bounds: PlanBounds,
    macrosteps: u64,
    sends: u64,
    timer: Option<String>,
    seq: u64,
}

/// The datamodel wrapper that makes `vt_usage` read-only.
struct AgentDm {
    inner: Box<dyn Datamodel>,
}

fn is_usage(location: &str) -> bool {
    let l = location.trim();
    let l = l.strip_prefix("ctx.").unwrap_or(l);
    l == vocab::USAGE_VARIABLE || l.starts_with("vt_usage.") || l.starts_with("vt_usage[")
}

fn read_only() -> EvalError {
    EvalError::new("vt_usage is a read-only system variable")
}

impl Datamodel for AgentDm {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.inner.init_session(info)
    }
    fn set_environment(&mut self, env: &Environment) {
        self.inner.set_environment(env);
    }
    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        if is_usage(id) {
            return Err(read_only());
        }
        self.inner.declare(id)
    }
    fn set_event(&mut self, event: &Event) {
        self.inner.set_event(event);
    }
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        self.inner.eval_cond(expr, states)
    }
    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        self.inner.eval_value(expr, states)
    }
    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        self.inner.eval_content(content)
    }
    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        if is_usage(location) {
            return Err(read_only());
        }
        self.inner.assign_expr(location, expr, states)
    }
    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        if is_usage(location) {
            return Err(read_only());
        }
        self.inner.assign_value(location, value)
    }
    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        if is_usage(name) {
            return Err(read_only());
        }
        self.inner.bind_var(name, value)
    }
    fn is_location(&mut self, location: &str) -> bool {
        self.inner.is_location(location)
    }
    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError> {
        self.inner.exec_script(script, states)
    }
    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        self.inner.foreach_items(array, states)
    }
    fn snapshot(&self) -> Value {
        self.inner.snapshot()
    }
    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        self.inner.restore(snapshot)
    }
}

/// One agent actor's deterministic state.
pub struct AgentMachine {
    actor: String,
    chart: Arc<AgentChart>,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    session: Session,
    dm: AgentDm,
    slots: BTreeMap<String, Slot>,
    stale: BTreeSet<String>,
    answered: BTreeSet<String>,
    decisions: BTreeMap<String, u32>,
    timers: BTreeMap<String, AgentTimer>,
    running: BTreeMap<String, Running>,
    plans: BTreeMap<String, PlanRun>,
    seen: BTreeSet<String>,
    usage: UsageTotals,
    records: Vec<Record>,
    done: Option<Value>,
    delivered: u64,
    core: Option<CoreTape>,
}

impl std::fmt::Debug for AgentMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMachine")
            .field("actor", &self.actor)
            .field("configuration", &self.configuration())
            .finish_non_exhaustive()
    }
}

/// An accepted decision waiting for its macrostep: correlation, event, transitions offered.
type PendingDecision = (String, String, Vec<(String, usize)>);

/// The per-input context.
struct Cx {
    position: u64,
    ts: i64,
    k: u64,
    env: Environment,
    effects: Vec<AgentEffect>,
    queue: VecDeque<Event>,
    raised: Vec<Event>,
    current: Option<Event>,
    accepted: Vec<AcceptedDecision>,
    pending_decision: Option<PendingDecision>,
    dropped: bool,
}

fn seed(actor: &str, position: u64) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(format!("{actor}#{position}").as_bytes());
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    u64::from_be_bytes(b)
}

/// A platform event. An `error.*` event also carries the `kind` (the name after `error.`) and
/// `message` fields of the predefined error payload type, so typed charts that do not declare
/// the richer agent payload can still read it.
fn platform(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        type_: EventType::Platform,
        data: crate::grant::with_error_kind(name, data),
        ..Event::default()
    }
}

fn internal(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        type_: EventType::Internal,
        data,
        ..Event::default()
    }
}

fn paths(v: &[Violation]) -> Value {
    Value::Array(v.iter().map(|x| s(x.path.clone())).collect())
}

fn strings(v: &[String]) -> Value {
    Value::Array(v.iter().map(|x| s(x.clone())).collect())
}

impl AgentMachine {
    /// Starts `actor` under `chart`: the start macrostep and the first offers.
    ///
    /// # Errors
    ///
    /// [`MachineError`] when the datamodel is unavailable or the start fails.
    pub fn start(
        actor: &str,
        chart: Arc<AgentChart>,
        registry: Arc<dyn DatamodelRegistry + Send + Sync>,
        ts: i64,
    ) -> Result<(AgentMachine, Applied), MachineError> {
        AgentMachine::start_with(actor, chart, registry, ts, StartOptions::default())
    }

    /// Starts `actor` like [`AgentMachine::start`] with `options`: a microstep budget other than
    /// the core default, and recording the core macrosteps of its session with their datamodel
    /// answer tape ([`AgentMachine::core_tape`]) so that a test can replay them through the Lean
    /// oracle. Neither changes anything else.
    ///
    /// # Errors
    ///
    /// As [`AgentMachine::start`].
    pub fn start_with(
        actor: &str,
        chart: Arc<AgentChart>,
        registry: Arc<dyn DatamodelRegistry + Send + Sync>,
        ts: i64,
        options: StartOptions,
    ) -> Result<(AgentMachine, Applied), MachineError> {
        let record = options.record_core;
        let options_budget = options.microstep_budget;
        let inner =
            vertep_core::null::create_datamodel(registry.as_ref(), chart.compiled.datamodel())
                .map_err(|e| MachineError::Datamodel(format!("{e:?}")))?;
        let mut dm = AgentDm { inner };
        let env = Environment {
            now_ms: ts,
            seed: seed(actor, 1),
        };
        let mut options = session::Options::default();
        if let Some(b) = options_budget {
            options.microstep_budget = b;
        }
        for t in [
            vocab::LLM_INVOKE_TYPE,
            vocab::TOOL_INVOKE_TYPE,
            vocab::PLAN_INVOKE_TYPE,
        ] {
            options.invoke_types.push(t.to_owned());
            if let Some(l) = t.strip_prefix(vertep_core::ir::VT_NS) {
                options.invoke_types.push(format!("vt:{l}"));
            }
        }
        options
            .send_types
            .push(vertep_runtime::vocab::REQUEST_SEND_TYPE.to_owned());
        let start = StartInput {
            info: SessionInfo {
                sessionid: actor.to_owned(),
                name: chart.ir.name.clone(),
                ioprocessors: Value::Undefined,
            },
            env,
            options,
            data: Vec::new(),
            sessions: None,
        };
        let mut core = None;
        let o = if record {
            let mut tape = TapeDatamodel::new(&mut dm);
            let o = session::start(&chart.compiled, start.clone(), &mut tape)
                .map_err(|e| MachineError::Step(format!("{e:?}")))?;
            core = Some(CoreTape {
                records: vec![OracleRecord {
                    input: OracleInput::Start(start),
                    tape: tape.take(),
                    trace: o.trace.clone(),
                }],
                budget_exceeded: false,
            });
            o
        } else {
            session::start(&chart.compiled, start, &mut dm)
                .map_err(|e| MachineError::Step(format!("{e:?}")))?
        };
        let mut m = AgentMachine {
            actor: actor.to_owned(),
            chart,
            registry,
            session: o.session,
            dm,
            slots: BTreeMap::new(),
            stale: BTreeSet::new(),
            answered: BTreeSet::new(),
            decisions: BTreeMap::new(),
            timers: BTreeMap::new(),
            running: BTreeMap::new(),
            plans: BTreeMap::new(),
            seen: BTreeSet::new(),
            usage: UsageTotals::default(),
            records: Vec::new(),
            done: None,
            delivered: 0,
            core,
        };
        let mut cx = Cx {
            position: 1,
            ts,
            k: 0,
            env,
            effects: Vec::new(),
            queue: VecDeque::new(),
            raised: Vec::new(),
            current: None,
            accepted: Vec::new(),
            pending_decision: None,
            dropped: false,
        };
        m.after_macrostep(o.effects, &o.trace, &mut cx);
        m.flush_raised(&mut cx);
        m.drain(&mut cx)?;
        Ok((m, cx.into_applied()))
    }

    /// The actor IRI.
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// The chart.
    pub fn chart(&self) -> &Arc<AgentChart> {
        &self.chart
    }

    /// The active state ids in document order.
    pub fn configuration(&self) -> Vec<String> {
        self.session.active(&self.chart.compiled)
    }

    /// Whether `id` is active.
    pub fn is_active(&self, id: &str) -> bool {
        self.session.is_active(&self.chart.compiled, id)
    }

    /// The datamodel snapshot.
    pub fn datamodel(&self) -> Value {
        self.dm.snapshot()
    }

    /// A read-only view of the datamodel.
    pub fn context(&self) -> ContextView {
        ContextView::new(self.dm.snapshot())
    }

    /// Whether the session terminated.
    pub fn is_terminated(&self) -> bool {
        self.session.is_terminated()
    }

    /// The done data, once terminated by a final state.
    pub fn done_data(&self) -> Option<&Value> {
        self.done.as_ref()
    }

    /// The open offers.
    pub fn open_offers(&self) -> Vec<DecisionRequest> {
        self.slots
            .iter()
            .filter(|(_, s)| s.open)
            .filter_map(|(state, s)| {
                s.offer.as_ref().map(|o| DecisionRequest {
                    state: state.clone(),
                    candidates: o.candidates.clone(),
                    context: self.context(),
                    actor: self.actor.clone(),
                    correlation: o.correlation.clone(),
                    binding: o.binding.clone(),
                    fallback: o.fallback,
                })
            })
            .collect()
    }

    /// Whether a decision with this correlation id was already processed.
    pub fn answered(&self, correlation: &str) -> bool {
        self.answered.contains(correlation)
    }

    /// Whether message `id` was already processed.
    pub fn seen(&self, id: &str) -> bool {
        self.seen.contains(id)
    }

    /// The pending durable timers.
    pub fn timers(&self) -> Vec<AgentTimer> {
        self.timers.values().cloned().collect()
    }

    /// The running LLM calls, for the host to restart after replay.
    pub fn running_llm(&self) -> Vec<LlmStart> {
        self.running
            .values()
            .filter_map(|r| match r {
                Running::Llm { start, .. } => Some((**start).clone()),
                Running::Tool { .. } => None,
            })
            .collect()
    }

    /// Whether invocation `id` is a running LLM or tool call.
    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    /// The cumulative usage.
    pub fn usage(&self) -> UsageTotals {
        self.usage
    }

    /// Accepted decisions per decide state.
    pub fn decision_count(&self, state: &str) -> u32 {
        self.decisions.get(state).copied().unwrap_or(0)
    }

    /// The provenance records so far.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// The core macrosteps recorded so far, for a machine started with
    /// [`StartOptions::record_core`]; `None` otherwise.
    pub fn core_tape(&self) -> Option<&CoreTape> {
        self.core.as_ref()
    }

    /// Applies one journaled input at `position`, accepted at `ts`.
    ///
    /// # Errors
    ///
    /// [`MachineError`].
    // @tg: implements:: [[openspec:deciders#Decisions Are Journaled Inputs]]
    pub fn apply(
        &mut self,
        input: &AgentInput,
        position: u64,
        ts: i64,
    ) -> Result<Applied, MachineError> {
        if self.session.is_terminated() {
            return Err(MachineError::Terminated);
        }
        let mut cx = Cx {
            position,
            ts,
            k: 0,
            env: Environment {
                now_ms: ts,
                seed: seed(&self.actor, position),
            },
            effects: Vec::new(),
            queue: VecDeque::new(),
            raised: Vec::new(),
            current: None,
            accepted: Vec::new(),
            pending_decision: None,
            dropped: false,
        };
        match input {
            AgentInput::Start { .. } => {}
            AgentInput::Event { event, message_id } => {
                if !self.seen.insert(message_id.clone()) {
                    return Ok(Applied {
                        duplicate: true,
                        ..Applied::default()
                    });
                }
                cx.queue.push_back(event.clone());
            }
            AgentInput::Decision {
                correlation,
                event,
                payload,
                annotations,
            } => {
                if self.answered.contains(correlation) {
                    return Ok(Applied {
                        duplicate: true,
                        ..Applied::default()
                    });
                }
                self.on_decision(correlation, event, payload, annotations, &mut cx);
            }
            AgentInput::Refusal {
                correlation,
                reason,
            } => {
                if self.answered.contains(correlation) {
                    return Ok(Applied {
                        duplicate: true,
                        ..Applied::default()
                    });
                }
                self.on_refusal(correlation, reason, &mut cx);
            }
            AgentInput::Timer { iri } => self.on_timer(iri, &mut cx),
            AgentInput::LlmResult {
                invokeid,
                attempt,
                provider,
                prompt_hash,
                content,
                usage,
                model,
                latency_ms,
                cost,
                tool_calls,
            } => self.on_llm_result(
                LlmResultRef {
                    invokeid,
                    attempt: *attempt,
                    provider,
                    prompt_hash,
                    content,
                    usage: *usage,
                    model,
                    latency_ms: *latency_ms,
                    cost: cost.as_ref(),
                    tool_calls,
                },
                &mut cx,
            ),
            AgentInput::LlmError {
                invokeid,
                attempt,
                class,
                message,
                retry_after_ms,
            } => self.on_llm_error(
                invokeid,
                *attempt,
                *class,
                message,
                *retry_after_ms,
                &mut cx,
            ),
            AgentInput::ToolResult {
                invokeid,
                tool,
                outcome,
            } => self.on_tool_result(invokeid, tool, outcome, &mut cx),
        }
        self.flush_raised(&mut cx);
        self.drain(&mut cx)?;
        Ok(cx.into_applied())
    }

    /// Processes an event that is not journaled (a streamed partial): one macrostep.
    ///
    /// # Errors
    ///
    /// [`MachineError`].
    pub fn apply_transient(&mut self, event: Event, ts: i64) -> Result<Applied, MachineError> {
        if self.session.is_terminated() {
            return Err(MachineError::Terminated);
        }
        let mut cx = Cx {
            position: 0,
            ts,
            k: 0,
            env: Environment {
                now_ms: ts,
                seed: seed(&self.actor, 0),
            },
            effects: Vec::new(),
            queue: VecDeque::from([event]),
            raised: Vec::new(),
            current: None,
            accepted: Vec::new(),
            pending_decision: None,
            dropped: false,
        };
        self.drain(&mut cx)?;
        Ok(cx.into_applied())
    }

    // ----- stepping ----------------------------------------------------------------------

    fn bind_usage(&mut self) {
        if self.usage.calls > 0 {
            let _ = self
                .dm
                .inner
                .bind_var(vocab::USAGE_VARIABLE, self.usage.to_value());
        }
    }

    fn flush_raised(&mut self, cx: &mut Cx) {
        for e in cx.raised.drain(..).rev() {
            cx.queue.push_front(e);
        }
    }

    fn drain(&mut self, cx: &mut Cx) -> Result<(), MachineError> {
        let mut n = 0;
        while let Some(ev) = cx.queue.pop_front() {
            if self.session.is_terminated() {
                cx.queue.clear();
                break;
            }
            n += 1;
            if n > MAX_STEPS_PER_INPUT {
                return Err(MachineError::Livelock);
            }
            self.bind_usage();
            let input = Input::event(ev.clone(), cx.env);
            let r = match self.core.as_mut() {
                None => session::step(
                    &self.chart.compiled,
                    self.session.clone(),
                    input,
                    &mut self.dm,
                ),
                Some(core) => {
                    let mut tape = TapeDatamodel::new(&mut self.dm);
                    let r = session::step(
                        &self.chart.compiled,
                        self.session.clone(),
                        input.clone(),
                        &mut tape,
                    );
                    let trace = match &r {
                        Ok(o) => Some(o.trace.clone()),
                        Err(StepError::BudgetExceeded { partial, .. }) => {
                            core.budget_exceeded = true;
                            Some(partial.trace.clone())
                        }
                        Err(StepError::Terminated) => None,
                    };
                    if let Some(trace) = trace {
                        core.records.push(OracleRecord {
                            input: OracleInput::Step(input),
                            tape: tape.take(),
                            trace,
                        });
                    }
                    r
                }
            };
            let o = r.map_err(|e| MachineError::Step(format!("{e:?}")))?;
            if let Some((corr, event, offered)) = cx.pending_decision.take() {
                let taken = o
                    .trace
                    .microsteps
                    .iter()
                    .find(|m| m.event.as_deref() == Some(event.as_str()))
                    .map(|m| {
                        m.transitions
                            .iter()
                            .map(|t| (t.source.clone(), t.index))
                            .collect()
                    })
                    .unwrap_or_default();
                cx.accepted.push(AcceptedDecision {
                    correlation: corr,
                    event,
                    offered,
                    taken,
                });
            }
            self.session = o.session;
            cx.current = Some(ev);
            self.after_macrostep(o.effects, &o.trace, cx);
            self.flush_raised(cx);
        }
        Ok(())
    }

    fn after_macrostep(&mut self, effects: Vec<Effect>, trace: &MacrostepTrace, cx: &mut Cx) {
        for e in effects {
            match e {
                Effect::Send(req) => self.on_send(req, cx),
                Effect::Cancel { sendid } => {
                    let found: Vec<String> = self
                        .timers
                        .iter()
                        .filter(|(_, t)| {
                            matches!(&t.kind, TimerKind::DelayedSend { sendid: s, .. } if *s == sendid)
                        })
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in found {
                        self.timers.remove(&k);
                        cx.effects.push(AgentEffect::CancelTimer(k));
                    }
                }
                Effect::Invoke(req) => self.on_invoke(req, cx),
                Effect::CancelInvoke { invokeid } => self.on_cancel_invoke(&invokeid, cx),
                Effect::Forward { invokeid, event } => {
                    if self.plans.contains_key(&invokeid) {
                        self.run_plan(&invokeid, vec![event], cx);
                    }
                }
                Effect::Log { .. } => {}
                Effect::SessionDone { data, cancelled } => {
                    if !cancelled {
                        self.done = Some(data.clone());
                        cx.effects.push(AgentEffect::Done(data));
                    }
                }
            }
        }
        if !self.session.is_terminated() {
            self.decide_pass(trace, cx);
        } else {
            let open: Vec<String> = self.slots.keys().cloned().collect();
            for state in open {
                if let Some(slot) = self.slots.remove(&state) {
                    self.close(slot, cx);
                }
            }
        }
    }

    // ----- decide states -----------------------------------------------------------------

    fn probe(&self) -> Option<Box<dyn Datamodel>> {
        let mut p = vertep_core::null::create_datamodel(
            self.registry.as_ref(),
            self.chart.compiled.datamodel(),
        )
        .ok()?;
        p.init_session(&SessionInfo {
            sessionid: self.actor.clone(),
            name: self.chart.ir.name.clone(),
            ioprocessors: Value::Undefined,
        })
        .ok()?;
        p.restore(&self.dm.snapshot()).ok()?;
        Some(p)
    }

    fn candidates(&self, spec: &DecideSpec) -> Vec<Candidate> {
        let Some(mut probe) = self.probe() else {
            return Vec::new();
        };
        enabled_events(
            &self.chart.compiled,
            &self.session,
            probe.as_mut(),
            &spec.state,
            spec.candidates.as_deref(),
            &self.chart.schemas,
        )
    }

    fn exited(&self, trace: &MacrostepTrace, state: StateIdx) -> bool {
        let c = &self.chart.compiled;
        for m in &trace.microsteps {
            for tr in &m.transitions {
                let Some(src) = c.index_of(&tr.source) else {
                    continue;
                };
                let Some(&t) = c
                    .state(src)
                    .transitions
                    .iter()
                    .find(|&&t| c.transition(t).index == Some(tr.index))
                else {
                    continue;
                };
                if c.transition(t).domain == Domain::Targetless {
                    continue;
                }
                if let Some(d) = transition_domain(c, &self.session.history, t)
                    && c.is_descendant(state, d)
                {
                    return true;
                }
            }
        }
        false
    }

    fn close(&mut self, slot: Slot, cx: &mut Cx) {
        if let (true, Some(o)) = (slot.open, slot.offer) {
            self.stale.insert(o.correlation.clone());
            if let Some(t) = o.deadline
                && self.timers.remove(&t).is_some()
            {
                cx.effects.push(AgentEffect::CancelTimer(t));
            }
            cx.effects.push(AgentEffect::CancelOffer {
                correlation: o.correlation,
                binding: o.binding,
            });
        }
    }

    fn offer(
        &mut self,
        spec: &DecideSpec,
        binding: &str,
        fallback: bool,
        candidates: Vec<Candidate>,
        fp: String,
        cx: &mut Cx,
    ) {
        let correlation = format!("{}/decision/{}.{}", self.actor, cx.position, cx.k);
        cx.k += 1;
        let deadline = spec.deadline_ms.map(|d| {
            let t = AgentTimer {
                iri: format!("{correlation}/deadline"),
                due_ms: cx.ts.saturating_add(d),
                kind: TimerKind::Deadline {
                    state: spec.state.clone(),
                    correlation: correlation.clone(),
                },
            };
            self.timers.insert(t.iri.clone(), t.clone());
            cx.effects.push(AgentEffect::Timer(t.clone()));
            t.iri
        });
        cx.effects.push(AgentEffect::Offer(DecisionRequest {
            state: spec.state.clone(),
            candidates: candidates.clone(),
            context: self.context(),
            actor: self.actor.clone(),
            correlation: correlation.clone(),
            binding: binding.to_owned(),
            fallback,
        }));
        self.slots.insert(
            spec.state.clone(),
            Slot {
                offer: Some(Offer {
                    correlation,
                    binding: binding.to_owned(),
                    candidates,
                    fallback,
                    deadline,
                }),
                open: true,
                fingerprint: fp,
            },
        );
    }

    // @tg: implements:: [[openspec:llm-actors#LLM as Main Driver]] {note: "vt:maxDecisions and vt.agent.budget-exhausted; the loop itself is an ordinary chart"}
    fn fresh(&mut self, spec: &DecideSpec, cx: &mut Cx) {
        if let Some(max) = spec.max_decisions
            && self.decision_count(&spec.state) >= max
        {
            cx.raised.push(internal(
                vocab::BUDGET_EXHAUSTED_EVENT,
                obj([
                    ("state", s(&spec.state)),
                    ("decisions", Value::Number(f64::from(max))),
                ]),
            ));
            self.slots.insert(
                spec.state.clone(),
                Slot {
                    offer: None,
                    open: false,
                    fingerprint: String::new(),
                },
            );
            return;
        }
        let cands = self.candidates(spec);
        let fp = fingerprint(&cands);
        if cands.is_empty() {
            self.none(spec, fp, cx);
        } else {
            self.offer(spec, &spec.decider.clone(), false, cands, fp, cx);
        }
    }

    fn none(&mut self, spec: &DecideSpec, fp: String, cx: &mut Cx) {
        cx.raised.push(internal(
            vocab::DECIDE_NONE_EVENT,
            obj([("state", s(&spec.state))]),
        ));
        self.slots.insert(
            spec.state.clone(),
            Slot {
                offer: None,
                open: false,
                fingerprint: fp,
            },
        );
    }

    // @tg: implements:: [[openspec:deciders#Enabled Event Offer]]
    // @tg: implements:: [[openspec:deciders#Re-decision After Context Change]]
    // @tg: implements:: [[openspec:deciders#Non-blocking Decision Wait]]
    fn decide_pass(&mut self, trace: &MacrostepTrace, cx: &mut Cx) {
        let specs: Vec<DecideSpec> = self.chart.decides.values().cloned().collect();
        for spec in specs {
            let Some(idx) = self.chart.compiled.index_of(&spec.state) else {
                continue;
            };
            let active = self.session.configuration.contains(idx);
            if !active {
                if let Some(slot) = self.slots.remove(&spec.state) {
                    self.close(slot, cx);
                }
                continue;
            }
            if self.slots.contains_key(&spec.state)
                && self.exited(trace, idx)
                && let Some(slot) = self.slots.remove(&spec.state)
            {
                self.close(slot, cx);
            }
            let Some(slot) = self.slots.get(&spec.state).cloned() else {
                self.fresh(&spec, cx);
                continue;
            };
            if slot.offer.is_none() && slot.fingerprint.is_empty() && !slot.open {
                // budget exhausted: nothing more is offered until the state is re-entered.
                if spec
                    .max_decisions
                    .is_some_and(|m| self.decision_count(&spec.state) >= m)
                {
                    continue;
                }
            }
            let cands = self.candidates(&spec);
            let fp = fingerprint(&cands);
            if fp == slot.fingerprint {
                continue;
            }
            if slot.open {
                self.slots.remove(&spec.state);
                let binding = slot
                    .offer
                    .as_ref()
                    .map_or(spec.decider.clone(), |o| o.binding.clone());
                let fallback = slot.offer.as_ref().is_some_and(|o| o.fallback);
                self.close(slot, cx);
                if cands.is_empty() {
                    self.none(&spec, fp, cx);
                } else {
                    self.offer(&spec, &binding, fallback, cands, fp, cx);
                }
            } else if cands.is_empty() {
                if let Some(sl) = self.slots.get_mut(&spec.state) {
                    sl.fingerprint = fp;
                }
            } else {
                self.offer(&spec, &spec.decider.clone(), false, cands, fp, cx);
            }
        }
    }

    fn open_slot(&self, correlation: &str) -> Option<(String, Offer)> {
        self.slots.iter().find_map(|(state, sl)| match &sl.offer {
            Some(o) if sl.open && o.correlation == correlation => Some((state.clone(), o.clone())),
            _ => None,
        })
    }

    fn reject(
        &mut self,
        correlation: &str,
        event: &str,
        state: Option<&str>,
        reason: &str,
        detail: Value,
        cx: &mut Cx,
    ) {
        let mut data = vec![
            ("reason".to_owned(), s(reason)),
            ("correlation".to_owned(), s(correlation)),
            ("event".to_owned(), s(event)),
        ];
        if let Some(st) = state {
            data.push(("state".into(), s(st)));
        }
        match detail {
            Value::Array(_) => data.push(("paths".into(), detail)),
            Value::Undefined => data.push(("paths".into(), Value::Array(Vec::new()))),
            other => {
                data.push(("paths".into(), Value::Array(Vec::new())));
                data.push(("detail".into(), other));
            }
        }
        cx.raised
            .push(platform(vocab::DECIDE_INVALID_EVENT, Value::Object(data)));
    }

    // @tg: implements:: [[openspec:deciders#Decision Validation Against Candidates]]
    // @tg: implements:: [[openspec:deciders#Decider Safety]] {confidence: partial, note: "enforced in Rust; the Lean theorem is task 8.2"}
    fn on_decision(
        &mut self,
        correlation: &str,
        event: &str,
        payload: &Value,
        annotations: &Annotations,
        cx: &mut Cx,
    ) {
        let iri = format!("{}/decision/{}", self.actor, cx.position);
        if let Some(u) = annotations.usage {
            // the model call behind an LLM decision counts whether or not it is accepted
            self.usage.prompt_tokens += u.prompt_tokens;
            self.usage.completion_tokens += u.completion_tokens;
            self.usage.calls += 1;
            self.records.push(Record::LlmCall {
                iri: format!("{iri}/llm-call"),
                actor: self.actor.clone(),
                invokeid: correlation.to_owned(),
                provider: annotations.decider.clone().unwrap_or_default(),
                model: annotations.model.clone().unwrap_or_default(),
                prompt_hash: annotations.prompt_hash.clone().unwrap_or_default(),
                usage: u,
                latency_ms: 0,
                cost: None,
                attempt: 1,
                outcome: "decision".into(),
            });
        }
        let Some((state, offer)) = self.open_slot(correlation) else {
            self.answered.insert(correlation.to_owned());
            self.records.push(Record::Decision {
                iri,
                actor: self.actor.clone(),
                state: String::new(),
                correlation: correlation.to_owned(),
                event: event.to_owned(),
                binding: annotations.decider.clone().unwrap_or_default(),
                annotations: annotations.clone(),
                accepted: false,
                reason: Some("stale".into()),
            });
            self.reject(correlation, event, None, "stale", Value::Undefined, cx);
            return;
        };
        let record = |m: &mut AgentMachine, accepted: bool, reason: Option<&str>| {
            m.records.push(Record::Decision {
                iri: iri.clone(),
                actor: m.actor.clone(),
                state: state.clone(),
                correlation: correlation.to_owned(),
                event: event.to_owned(),
                binding: offer.binding.clone(),
                annotations: annotations.clone(),
                accepted,
                reason: reason.map(str::to_owned),
            });
        };
        let finish = |m: &mut AgentMachine, cx: &mut Cx| {
            m.answered.insert(correlation.to_owned());
            if let Some(sl) = m.slots.get_mut(&state) {
                sl.open = false;
            }
            if let Some(t) = &offer.deadline
                && m.timers.remove(t).is_some()
            {
                cx.effects.push(AgentEffect::CancelTimer(t.clone()));
            }
        };
        if let Err(e) = annotations.validate() {
            finish(self, cx);
            record(self, false, Some("annotation"));
            self.reject(correlation, event, Some(&state), "annotation", s(e), cx);
            return;
        }
        let Some(cand) = offer.candidates.iter().find(|c| c.event == event) else {
            finish(self, cx);
            record(self, false, Some("not-a-candidate"));
            self.reject(
                correlation,
                event,
                Some(&state),
                "not-a-candidate",
                Value::Undefined,
                cx,
            );
            return;
        };
        if let Err(v) = validate(&cand.schema, payload) {
            finish(self, cx);
            record(self, false, Some("payload-schema"));
            self.reject(
                correlation,
                event,
                Some(&state),
                "payload-schema",
                paths(&v),
                cx,
            );
            return;
        }
        let chosen = internal(event, payload.clone());
        let selected = self.probe().and_then(|mut p| {
            selected_transition(
                &self.chart.compiled,
                &self.session,
                p.as_mut(),
                &state,
                &chosen,
            )
        });
        if selected != Some((cand.source.clone(), cand.index)) {
            finish(self, cx);
            record(self, false, Some("not-enabled"));
            self.reject(
                correlation,
                event,
                Some(&state),
                "not-enabled",
                Value::Undefined,
                cx,
            );
            return;
        }
        finish(self, cx);
        record(self, true, None);
        *self.decisions.entry(state.clone()).or_insert(0) += 1;
        cx.pending_decision = Some((
            correlation.to_owned(),
            event.to_owned(),
            offer
                .candidates
                .iter()
                .map(|c| (c.source.clone(), c.index))
                .collect(),
        ));
        cx.queue.push_back(chosen);
    }

    // @tg: implements:: [[openspec:deciders#Decider Contract]]
    fn on_refusal(&mut self, correlation: &str, reason: &str, cx: &mut Cx) {
        self.answered.insert(correlation.to_owned());
        let Some((state, offer)) = self.open_slot(correlation) else {
            return;
        };
        if let Some(sl) = self.slots.get_mut(&state) {
            sl.open = false;
        }
        if let Some(t) = &offer.deadline
            && self.timers.remove(t).is_some()
        {
            cx.effects.push(AgentEffect::CancelTimer(t.clone()));
        }
        cx.raised.push(internal(
            vocab::DECIDE_REFUSED_EVENT,
            obj([
                ("state", s(&state)),
                ("reason", s(reason)),
                ("correlation", s(correlation)),
            ]),
        ));
    }

    // @tg: implements:: [[openspec:deciders#Decision Deadline and Fallback]]
    fn on_timer(&mut self, iri: &str, cx: &mut Cx) {
        let Some(t) = self.timers.remove(iri) else {
            cx.dropped = true;
            return;
        };
        match t.kind {
            TimerKind::Deadline { state, correlation } => {
                let Some((_, offer)) = self.open_slot(&correlation) else {
                    return;
                };
                let Some(spec) = self.chart.decides.get(&state).cloned() else {
                    return;
                };
                let fp = self
                    .slots
                    .get(&state)
                    .map(|s| s.fingerprint.clone())
                    .unwrap_or_default();
                let mut slot = self.slots.remove(&state);
                if let Some(sl) = slot.as_mut()
                    && let Some(o) = sl.offer.as_mut()
                {
                    o.deadline = None;
                }
                if let Some(sl) = slot {
                    self.close(sl, cx);
                }
                match (&spec.fallback, offer.fallback) {
                    (Some(fb), false) => {
                        let fb = fb.clone();
                        self.offer(&spec, &fb, true, offer.candidates, fp, cx);
                    }
                    _ => {
                        self.slots.insert(
                            state.clone(),
                            Slot {
                                offer: None,
                                open: false,
                                fingerprint: fp,
                            },
                        );
                        cx.raised.push(platform(
                            vocab::DECIDE_TIMEOUT_EVENT,
                            obj([("state", s(&state)), ("correlation", s(&correlation))]),
                        ));
                    }
                }
            }
            TimerKind::Retry { invokeid } => {
                if let Some(Running::Llm { start, .. }) = self.running.get(&invokeid) {
                    cx.effects.push(AgentEffect::StartLlm((**start).clone()));
                }
            }
            TimerKind::PlanDuration { invokeid } => {
                if self.plans.contains_key(&invokeid) {
                    self.stop_plan(&invokeid, "bounds:duration", cx);
                    self.deliver(
                        platform(
                            vocab::PLAN_ERROR_EVENT,
                            obj([
                                ("class", s(PlanErrorClass::Bounds.as_str())),
                                ("bound", s("duration")),
                                ("invokeid", s(&invokeid)),
                            ]),
                        ),
                        Some(&invokeid),
                        cx,
                    );
                }
            }
            TimerKind::DelayedSend { event, .. } => cx.queue.push_back(event),
        }
    }

    // ----- sends and invocations ---------------------------------------------------------

    fn deliver(&mut self, mut event: Event, invokeid: Option<&str>, cx: &mut Cx) {
        if let Some(id) = invokeid {
            event.invokeid = Some(id.to_owned());
        }
        self.delivered += 1;
        let message_id = format!(
            "{}/deliver/{}.{}.{}",
            self.actor, cx.position, cx.k, self.delivered
        );
        cx.k += 1;
        cx.effects.push(AgentEffect::Deliver { message_id, event });
    }

    fn on_send(&mut self, req: SendRequest, cx: &mut Cx) {
        let target = req.target.clone().unwrap_or_default();
        if target.is_empty() || target == "#_internal" {
            if req.delay_ms > 0 {
                let t = AgentTimer {
                    iri: format!("{}/send/{}.{}", self.actor, cx.position, req.sendid),
                    due_ms: cx
                        .ts
                        .saturating_add(i64::try_from(req.delay_ms).unwrap_or(i64::MAX)),
                    kind: TimerKind::DelayedSend {
                        sendid: req.sendid.clone(),
                        event: req.event,
                    },
                };
                self.timers.insert(t.iri.clone(), t.clone());
                cx.effects.push(AgentEffect::Timer(t));
            } else {
                self.deliver(req.event, None, cx);
            }
            return;
        }
        if let Some(id) = target.strip_prefix("#_")
            && self.plans.contains_key(id)
        {
            let id = id.to_owned();
            self.run_plan(&id, vec![req.event], cx);
            return;
        }
        let correlation = format!("{}/send/{}.{}", self.actor, cx.position, cx.k);
        cx.k += 1;
        cx.effects.push(AgentEffect::Send {
            target,
            event: req.event,
            correlation,
        });
    }

    fn invoke_ir(&self, invokeid: &str) -> Option<(String, Invoke)> {
        let inv = self
            .session
            .invocations
            .iter()
            .find(|i| i.invokeid == invokeid)?;
        let ir = self.chart.invoke(inv.state, inv.index)?.clone();
        Some((self.chart.compiled.state(inv.state).id.clone(), ir))
    }

    fn declared_grant(&self, state: &str, inv: &Invoke) -> Option<Grant> {
        grant::declared(&inv.extensions).or_else(|| {
            self.chart
                .state_ir(state)
                .and_then(|s| grant::declared(&s.extensions))
        })
    }

    fn on_invoke(&mut self, req: InvokeRequest, cx: &mut Cx) {
        let Some((state, inv)) = self.invoke_ir(&req.invokeid) else {
            return;
        };
        let t = req.type_.as_str();
        if vocab::is_type(t, vocab::LLM_INVOKE_TYPE) {
            self.start_llm(&req, &state, &inv, cx);
        } else if vocab::is_type(t, vocab::TOOL_INVOKE_TYPE) {
            self.start_tool(&req, &state, &inv, cx);
        } else if vocab::is_type(t, vocab::PLAN_INVOKE_TYPE) {
            self.start_plan(&req, &state, &inv, cx);
        }
    }

    // @tg: implements:: [[openspec:llm-actors#LLM Invocation]]
    // @tg: implements:: [[openspec:llm-actors#Prompt Templates]]
    fn start_llm(&mut self, req: &InvokeRequest, state: &str, inv: &Invoke, cx: &mut Cx) {
        let spec = LlmSpec::of(inv, req.content.as_ref());
        let fail = |reason: &str, extra: Vec<(&str, Value)>| {
            let mut data = vec![
                ("reason".to_owned(), s(reason)),
                ("invokeid".to_owned(), s(&req.invokeid)),
            ];
            data.extend(extra.into_iter().map(|(k, v)| (k.to_owned(), v)));
            platform(vocab::EXECUTION_ERROR_EVENT, Value::Object(data))
        };
        let ctx = self.context();
        let prompt = match render(&spec.template, ctx.data(), cx.current.as_ref()) {
            Ok(p) => p,
            Err(e) => {
                cx.raised.push(fail("template", vec![("path", s(e.path))]));
                return;
            }
        };
        let output = match &spec.output {
            Some(t) => match self.chart.schemas.of_type_text(t) {
                Ok(o) => Some(o),
                Err(e) => {
                    cx.raised.push(fail("output-type", vec![("message", s(e))]));
                    return;
                }
            },
            None => None,
        };
        let tools = self
            .declared_grant(state, inv)
            .map(|g| g.tools.into_iter().collect())
            .unwrap_or_default();
        let start = LlmStart {
            invokeid: req.invokeid.clone(),
            attempt: 1,
            provider: spec.provider.clone(),
            model: spec.model.clone(),
            prompt,
            params: spec.params.clone(),
            output_schema: output.as_ref().map(|(_, s)| s.clone()),
            tools,
            stream: spec.stream,
        };
        cx.effects.push(AgentEffect::StartLlm(start.clone()));
        self.running.insert(
            req.invokeid.clone(),
            Running::Llm {
                start: Box::new(start),
                spec,
                output,
            },
        );
    }

    fn call_iri(&self, position: u64) -> String {
        format!("{}/llm-call/{}", self.actor, position)
    }

    // @tg: implements:: [[openspec:llm-actors#Structured Output]]
    // @tg: implements:: [[openspec:llm-actors#Journaled Model Results]]
    // @tg: implements:: [[openspec:llm-actors#Usage and Cost Accounting]]
    fn on_llm_result(&mut self, r: LlmResultRef<'_>, cx: &mut Cx) {
        let Some(Running::Llm { start, output, .. }) = self.running.get(r.invokeid).cloned() else {
            cx.dropped = true;
            return;
        };
        if start.attempt != r.attempt {
            cx.dropped = true;
            return;
        }
        let call = self.call_iri(cx.position);
        self.usage.prompt_tokens += r.usage.prompt_tokens;
        self.usage.completion_tokens += r.usage.completion_tokens;
        self.usage.calls += 1;
        self.usage.cost += r.cost.map_or(0.0, |c| c.amount);
        let mut rec = Record::LlmCall {
            iri: call.clone(),
            actor: self.actor.clone(),
            invokeid: r.invokeid.to_owned(),
            provider: r.provider.to_owned(),
            model: r.model.to_owned(),
            prompt_hash: r.prompt_hash.to_owned(),
            usage: r.usage,
            latency_ms: r.latency_ms,
            cost: r.cost.cloned(),
            attempt: r.attempt,
            outcome: "ok".into(),
        };
        let data = match &output {
            Some((ty, schema)) => {
                let parsed = crate::llm::parse_output(r.content);
                let checked = parsed
                    .map_err(|e| {
                        vec![Violation {
                            path: String::new(),
                            message: e,
                        }]
                    })
                    .and_then(|v| validate(schema, &v).map(|()| v))
                    .and_then(|v| {
                        check_type(self.chart.schemas.declarations(), ty, &v).map(|()| v)
                    });
                match checked {
                    Ok(v) => v,
                    Err(v) => {
                        if let Record::LlmCall { outcome, .. } = &mut rec {
                            *outcome = "schema".into();
                        }
                        self.records.push(rec);
                        self.llm_failed(
                            r.invokeid,
                            r.attempt,
                            ErrorClass::Schema,
                            None,
                            Some(paths(&v)),
                            cx,
                        );
                        return;
                    }
                }
            }
            None => obj([
                ("text", s(r.content)),
                ("model", s(r.model)),
                ("call", s(&call)),
                ("promptHash", s(r.prompt_hash)),
                (
                    "usage",
                    obj([
                        ("prompt_tokens", Value::Number(r.usage.prompt_tokens as f64)),
                        (
                            "completion_tokens",
                            Value::Number(r.usage.completion_tokens as f64),
                        ),
                    ]),
                ),
                (
                    "toolCalls",
                    Value::Array(
                        r.tool_calls
                            .iter()
                            .map(|t| {
                                obj([
                                    ("tool", s(&t.tool)),
                                    ("args", crate::json::from_json(&t.args)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        };
        self.records.push(rec);
        self.running.remove(r.invokeid);
        let mut ev = Event::named(format!("done.invoke.{}", r.invokeid), EventType::External);
        ev.data = data;
        ev.invokeid = Some(r.invokeid.to_owned());
        cx.queue.push_back(ev);
    }

    // @tg: implements:: [[openspec:llm-actors#Retries Expressed in the Chart]]
    fn llm_failed(
        &mut self,
        invokeid: &str,
        attempt: u32,
        class: ErrorClass,
        retry_after_ms: Option<i64>,
        violated: Option<Value>,
        cx: &mut Cx,
    ) {
        let Some(Running::Llm { start, spec, .. }) = self.running.get(invokeid).cloned() else {
            return;
        };
        if let Some(r) = spec.retry
            && class.retryable()
            && attempt < r.attempts
        {
            let next = attempt + 1;
            let wait = r.backoff_before(next).max(retry_after_ms.unwrap_or(0));
            let t = AgentTimer {
                iri: format!("{}/retry/{}/{}", self.actor, invokeid, next),
                due_ms: cx.ts.saturating_add(wait),
                kind: TimerKind::Retry {
                    invokeid: invokeid.to_owned(),
                },
            };
            self.timers.insert(t.iri.clone(), t.clone());
            cx.effects.push(AgentEffect::Timer(t));
            if let Some(Running::Llm { start: st, .. }) = self.running.get_mut(invokeid) {
                st.attempt = next;
            }
            let _ = start;
            return;
        }
        self.running.remove(invokeid);
        let mut data = vec![
            ("class".to_owned(), s(class.as_str())),
            ("invokeid".to_owned(), s(invokeid)),
            ("attempts".to_owned(), Value::Number(f64::from(attempt))),
        ];
        if let Some(ms) = retry_after_ms {
            data.push(("retryAfterMs".into(), Value::Number(ms as f64)));
        }
        if let Some(p) = violated {
            data.push(("paths".into(), p));
        }
        let mut ev = platform(vocab::LLM_ERROR_EVENT, Value::Object(data));
        ev.invokeid = Some(invokeid.to_owned());
        cx.queue.push_back(ev);
    }

    // @tg: implements:: [[openspec:llm-actors#Provider Contract]]
    fn on_llm_error(
        &mut self,
        invokeid: &str,
        attempt: u32,
        class: ErrorClass,
        message: &str,
        retry_after_ms: Option<i64>,
        cx: &mut Cx,
    ) {
        let Some(Running::Llm { start, .. }) = self.running.get(invokeid).cloned() else {
            cx.dropped = true;
            return;
        };
        if start.attempt != attempt {
            cx.dropped = true;
            return;
        }
        self.records.push(Record::LlmCall {
            iri: self.call_iri(cx.position),
            actor: self.actor.clone(),
            invokeid: invokeid.to_owned(),
            provider: start.provider.clone(),
            model: start.model.clone(),
            prompt_hash: start.prompt.hash.clone(),
            usage: Usage::default(),
            latency_ms: 0,
            cost: None,
            attempt,
            outcome: class.as_str().into(),
        });
        if class == ErrorClass::UnknownProvider {
            self.running.remove(invokeid);
            let mut ev = platform(
                vocab::EXECUTION_ERROR_EVENT,
                obj([
                    ("reason", s("unknown-provider")),
                    ("provider", s(&start.provider)),
                    ("invokeid", s(invokeid)),
                    ("message", s(message)),
                ]),
            );
            ev.invokeid = Some(invokeid.to_owned());
            cx.queue.push_back(ev);
            return;
        }
        self.llm_failed(invokeid, attempt, class, retry_after_ms, None, cx);
    }

    // @tg: implements:: [[openspec:llm-actors#Tool Actors]]
    // @tg: implements:: [[openspec:llm-actors#Capability Grants]]
    fn start_tool(&mut self, req: &InvokeRequest, state: &str, inv: &Invoke, cx: &mut Cx) {
        let tool = req.src.clone().unwrap_or_default();
        let args = match (&req.params, &req.content) {
            (Value::Undefined, Some(c)) => c.clone(),
            (p, _) => p.clone(),
        };
        if let Some(g) = self.declared_grant(state, inv)
            && !g.allows_tool(&tool)
        {
            self.records.push(Record::Denied {
                iri: format!("{}/denied/{}.{}", self.actor, cx.position, cx.k),
                sender: self.actor.clone(),
                target: tool.clone(),
                event: format!("invoke {}", req.invokeid),
            });
            let mut ev = crate::grant::not_granted_event(&tool, &req.invokeid);
            ev.invokeid = Some(req.invokeid.clone());
            cx.raised.push(ev);
            return;
        }
        self.running
            .insert(req.invokeid.clone(), Running::Tool { tool: tool.clone() });
        cx.effects.push(AgentEffect::StartTool {
            invokeid: req.invokeid.clone(),
            tool,
            args,
        });
    }

    fn on_tool_result(&mut self, invokeid: &str, tool: &str, outcome: &ToolOutcome, cx: &mut Cx) {
        let via_invoke = matches!(self.running.get(invokeid), Some(Running::Tool { .. }));
        if !via_invoke && !invokeid.starts_with(&format!("{}/send/", self.actor)) {
            cx.dropped = true;
            return;
        }
        self.running.remove(invokeid);
        self.records.push(Record::ToolCall {
            iri: format!("{}/tool-call/{}", self.actor, cx.position),
            actor: self.actor.clone(),
            tool: tool.to_owned(),
            outcome: match outcome {
                ToolOutcome::Ok(_) => "ok".into(),
                ToolOutcome::Err { class, .. } => class.as_str().into(),
            },
        });
        let mut ev = match outcome {
            ToolOutcome::Ok(v) if via_invoke => {
                let mut e = Event::named(format!("done.invoke.{invokeid}"), EventType::External);
                e.data = v.clone();
                e
            }
            ToolOutcome::Ok(v) => {
                let mut e = Event::named(vocab::REPLY_EVENT, EventType::External);
                e.data = obj([("tool", s(tool)), ("result", v.clone())]);
                e
            }
            ToolOutcome::Err {
                class,
                message,
                paths,
            } => platform(
                vocab::TOOL_ERROR_EVENT,
                obj([
                    ("class", s(class.as_str())),
                    ("tool", s(tool)),
                    ("message", s(message)),
                    ("paths", strings(paths)),
                ]),
            ),
        };
        if via_invoke {
            ev.invokeid = Some(invokeid.to_owned());
        }
        cx.queue.push_back(ev);
    }

    fn on_cancel_invoke(&mut self, invokeid: &str, cx: &mut Cx) {
        if self.running.remove(invokeid).is_some() {
            let retry: Vec<String> = self
                .timers
                .iter()
                .filter(
                    |(_, t)| matches!(&t.kind, TimerKind::Retry { invokeid: i } if i == invokeid),
                )
                .map(|(k, _)| k.clone())
                .collect();
            for k in retry {
                self.timers.remove(&k);
                cx.effects.push(AgentEffect::CancelTimer(k));
            }
            cx.effects
                .push(AgentEffect::CancelInvoke(invokeid.to_owned()));
        }
        if self.plans.contains_key(invokeid) {
            self.stop_plan(invokeid, "cancelled", cx);
        }
    }

    // ----- plans -------------------------------------------------------------------------

    // @tg: implements:: [[openspec:plan-charts#Plan Submission]]
    // @tg: implements:: [[openspec:plan-charts#Plan Lineage]]
    fn start_plan(&mut self, req: &InvokeRequest, state: &str, inv: &Invoke, cx: &mut Cx) {
        let pick = |k: &str| {
            field(&req.params, k).or_else(|| req.content.as_ref().and_then(|c| field(c, k)))
        };
        let text = match (pick("plan"), &req.content) {
            (Some(Value::String(t)), _) => t.clone(),
            (_, Some(Value::String(t))) => t.clone(),
            _ => String::new(),
        };
        let format = PlanFormat::parse(match pick("format") {
            Some(Value::String(f)) => Some(f.as_str()),
            _ => None,
        });
        let generated_by = match pick("generatedBy") {
            Some(Value::String(g)) => Some(g.clone()),
            _ => None,
        };
        let declared = self.declared_grant(state, inv);
        let functions = declared
            .as_ref()
            .map(|g| g.functions.clone())
            .unwrap_or_default();
        let bounds = PlanBounds::of(&inv.extensions);
        match submit(&text, format, &functions) {
            Err(e) => {
                self.records.push(Record::Plan {
                    iri: format!(
                        "urn:vertep:plan:{}",
                        e.content_hash.trim_start_matches("sha256:")
                    ),
                    actor: self.actor.clone(),
                    state: state.to_owned(),
                    invokeid: req.invokeid.clone(),
                    content_hash: e.content_hash.clone(),
                    generated_by,
                    accepted: false,
                    violations: e.violations.clone(),
                });
                self.deliver(plan_error(&e, &req.invokeid), Some(&req.invokeid), cx);
            }
            Ok(plan) => {
                let plan_iri = plan.version.as_str().to_owned();
                self.records.push(Record::Plan {
                    iri: plan_iri.clone(),
                    actor: self.actor.clone(),
                    state: state.to_owned(),
                    invokeid: req.invokeid.clone(),
                    content_hash: plan.content_hash.clone(),
                    generated_by,
                    accepted: true,
                    violations: Vec::new(),
                });
                self.launch_plan(req, state, plan, plan_iri, declared, bounds, cx);
            }
        }
    }

    // @tg: implements:: [[openspec:plan-charts#Sandboxed Plan Execution]]
    #[allow(clippy::too_many_arguments)]
    fn launch_plan(
        &mut self,
        req: &InvokeRequest,
        state: &str,
        plan: AcceptedPlan,
        plan_iri: String,
        declared: Option<Grant>,
        bounds: PlanBounds,
        cx: &mut Cx,
    ) {
        let id = req.invokeid.clone();
        let Ok(compiled) = CompiledChart::new(plan.chart.clone()) else {
            return;
        };
        let grant = grant::effective(declared.as_ref(), plan.needs.as_ref())
            .unwrap_or_else(Grant::parent_only);
        let Ok(mut dm) =
            vertep_core::null::create_datamodel(self.registry.as_ref(), compiled.datamodel())
        else {
            self.deliver(
                platform(
                    vocab::PLAN_ERROR_EVENT,
                    obj([("class", s("invalid")), ("reason", s("datamodel"))]),
                ),
                Some(&id),
                cx,
            );
            return;
        };
        let start = StartInput {
            info: SessionInfo {
                sessionid: format!("{}#_{id}", self.actor),
                name: plan.chart.name.clone(),
                ioprocessors: Value::Undefined,
            },
            env: cx.env,
            options: session::Options::default(),
            data: Vec::new(),
            sessions: None,
        };
        let Ok(o) = session::start(&compiled, start, dm.as_mut()) else {
            self.deliver(
                platform(
                    vocab::PLAN_ERROR_EVENT,
                    obj([("class", s("invalid")), ("reason", s("start"))]),
                ),
                Some(&id),
                cx,
            );
            return;
        };
        let timer = bounds.max_duration_ms.map(|d| {
            let t = AgentTimer {
                iri: format!("{}/plan/{}/{}/duration", self.actor, id, cx.position),
                due_ms: cx.ts.saturating_add(d),
                kind: TimerKind::PlanDuration {
                    invokeid: id.clone(),
                },
            };
            self.timers.insert(t.iri.clone(), t.clone());
            cx.effects.push(AgentEffect::Timer(t.clone()));
            t.iri
        });
        self.plans.insert(
            id.clone(),
            PlanRun {
                state: state.to_owned(),
                plan_iri,
                plan,
                compiled,
                session: o.session,
                dm,
                grant,
                bounds,
                macrosteps: 1,
                sends: 0,
                timer,
                seq: 0,
            },
        );
        let own = self.plan_effects(&id, o.effects, cx);
        if !own.is_empty() {
            self.run_plan(&id, own, cx);
        }
    }

    fn stop_plan(&mut self, id: &str, outcome: &str, cx: &mut Cx) {
        if let Some(p) = self.plans.remove(id) {
            if let Some(t) = p.timer
                && self.timers.remove(&t).is_some()
            {
                cx.effects.push(AgentEffect::CancelTimer(t));
            }
            self.records.push(Record::PlanExecution {
                iri: format!("{}/plan-execution/{}/{}", self.actor, id, cx.position),
                plan: p.plan_iri,
                actor: self.actor.clone(),
                outcome: outcome.to_owned(),
            });
            let _ = p.state;
        }
    }

    /// Feeds `events` to plan `id` and runs it until it waits, finishes or hits a bound.
    fn run_plan(&mut self, id: &str, events: Vec<Event>, cx: &mut Cx) {
        let mut queue: VecDeque<Event> = events.into();
        while let Some(ev) = queue.pop_front() {
            let Some(p) = self.plans.get_mut(id) else {
                return;
            };
            if p.macrosteps >= p.bounds.max_macrosteps {
                self.plan_bound(id, "macrosteps", cx);
                return;
            }
            p.macrosteps += 1;
            let r = session::step(
                &p.compiled,
                p.session.clone(),
                Input::event(ev, cx.env),
                p.dm.as_mut(),
            );
            match r {
                Ok(o) => {
                    p.session = o.session;
                    let more = self.plan_effects(id, o.effects, cx);
                    queue.extend(more);
                }
                Err(_) => {
                    self.stop_plan(id, "failed", cx);
                    return;
                }
            }
        }
    }

    fn plan_bound(&mut self, id: &str, bound: &str, cx: &mut Cx) {
        self.stop_plan(id, &format!("bounds:{bound}"), cx);
        self.deliver(
            platform(
                vocab::PLAN_ERROR_EVENT,
                obj([
                    ("class", s(PlanErrorClass::Bounds.as_str())),
                    ("bound", s(bound)),
                    ("invokeid", s(id)),
                ]),
            ),
            Some(id),
            cx,
        );
    }

    /// Handles a plan step's effects; returns events the plan sent itself.
    // @tg: implements:: [[openspec:plan-charts#Plan Execution Bounds]]
    // @tg: implements:: [[openspec:plan-charts#Plan Results Returned to Parent]]
    fn plan_effects(&mut self, id: &str, effects: Vec<Effect>, cx: &mut Cx) -> Vec<Event> {
        let mut own = Vec::new();
        let mut first_run = Vec::new();
        for e in effects {
            let Some(p) = self.plans.get_mut(id) else {
                break;
            };
            match e {
                Effect::Send(req) => {
                    let target = req.target.clone().unwrap_or_default();
                    if target.is_empty() || target == "#_internal" {
                        own.push(req.event);
                        continue;
                    }
                    let name = if target == "#_parent" {
                        grant::PARENT.to_owned()
                    } else {
                        target.clone()
                    };
                    if !p.grant.allows(&name) {
                        let sender = format!("{}#_{id}", self.actor);
                        self.records.push(Record::Denied {
                            iri: format!("{}/denied/{}.{}", self.actor, cx.position, cx.k),
                            sender,
                            target: name.clone(),
                            event: req.event.name.clone(),
                        });
                        own.push(crate::grant::not_granted_event(&name, &req.event.name));
                        continue;
                    }
                    p.sends += 1;
                    if p.sends > p.bounds.max_sends {
                        self.plan_bound(id, "sends", cx);
                        return Vec::new();
                    }
                    if name == grant::PARENT {
                        let mut ev = req.event;
                        ev.origin = Some(format!("{}#_{id}", self.actor));
                        p.seq += 1;
                        first_run.push(ev);
                    } else {
                        let correlation = format!("{}/plan/{id}/{}", self.actor, p.seq);
                        p.seq += 1;
                        cx.effects.push(AgentEffect::Send {
                            target,
                            event: req.event,
                            correlation,
                        });
                    }
                }
                Effect::SessionDone { data, cancelled } if !cancelled => {
                    let result = p.plan.result.clone();
                    let ok = result
                        .as_ref()
                        .is_none_or(|(decls, ty)| check_type(decls, ty, &data).is_ok());
                    for ev in first_run.drain(..) {
                        self.deliver(ev, Some(id), cx);
                    }
                    if ok {
                        self.stop_plan(id, "done", cx);
                        let mut ev = Event::named(format!("done.invoke.{id}"), EventType::External);
                        ev.data = data;
                        self.deliver(ev, Some(id), cx);
                    } else {
                        self.stop_plan(id, "result", cx);
                        self.deliver(
                            platform(
                                vocab::PLAN_ERROR_EVENT,
                                obj([
                                    ("class", s(PlanErrorClass::Result.as_str())),
                                    ("invokeid", s(id)),
                                    ("data", data),
                                ]),
                            ),
                            Some(id),
                            cx,
                        );
                    }
                    return Vec::new();
                }
                _ => {}
            }
        }
        for ev in first_run {
            self.deliver(ev, Some(id), cx);
        }
        own
    }
}

fn plan_error(e: &PlanError, invokeid: &str) -> Event {
    platform(
        vocab::PLAN_ERROR_EVENT,
        obj([
            ("class", s(e.class.as_str())),
            ("reason", s(e.reasons.first().cloned().unwrap_or_default())),
            ("reasons", strings(&e.reasons)),
            ("violations", strings(&e.violations)),
            ("contentHash", s(&e.content_hash)),
            ("invokeid", s(invokeid)),
        ]),
    )
}

struct LlmResultRef<'a> {
    invokeid: &'a str,
    attempt: u32,
    provider: &'a str,
    prompt_hash: &'a str,
    content: &'a str,
    usage: Usage,
    model: &'a str,
    latency_ms: i64,
    cost: Option<&'a Cost>,
    tool_calls: &'a [crate::llm::ToolCallRequest],
}

impl Cx {
    fn into_applied(self) -> Applied {
        Applied {
            effects: self.effects,
            duplicate: false,
            dropped: self.dropped,
            accepted: self.accepted,
        }
    }
}

/// The JSON of a value (re-exported for hosts).
pub fn value_json(v: &Value) -> Json {
    to_json(v)
}

/// The SHA-256 of a text (re-exported for hosts).
pub fn hash(text: &str) -> String {
    sha256(text.as_bytes())
}
