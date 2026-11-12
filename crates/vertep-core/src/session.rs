//! Sessions and the pure `start` and `step` functions (W3C SCXML Appendix D, `interpret` and
//! `mainEventLoop`).
//!
//! A [`Session`] is a value: the configuration as a bitset over states in document order, the
//! history table, the internal queue, active invocations, the send and invoke counters, the
//! late-binding status of data items and the options. [`start`] and [`step`] take a session (or
//! create one) and an [`Input`], and return the next session, the [`Effect`]s to perform and a
//! [`MacrostepTrace`]. They perform no I/O, read no clock and use no randomness; delays are
//! durations inside effects and the time and seed of each input reach the datamodel only through
//! [`Datamodel::set_environment`].
//!
//! The datamodel is the one mutable component kept outside the value (decision D1); it is passed
//! as `&mut dyn Datamodel`. Executable content and condition evaluation live in this module and
//! call it; the transition selection and entry/exit computations are delegated to the
//! trait-object-free [`crate::kernel`].

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::bitset::StateSet;
use crate::compile::{CompiledChart, ROOT, StateIdx, TransIdx};
use crate::datamodel::{
    Datamodel, Environment, EvalError, Event, EventType, SessionInfo, StateQuery, Value,
};
use crate::ir::{Binding, Content, Data, Exec, ExecKind, Invoke, Param, Send, StateKind};
use crate::kernel::{self, History};
use crate::trace::{MacrostepTrace, MicrostepTrace, TraceInput, TransitionRef};

/// The default number of microsteps one macrostep may take before it is cut off.
pub const DEFAULT_MICROSTEP_BUDGET: u32 = 10_000;
/// The type URI of the SCXML Event I/O Processor.
pub const SCXML_EVENT_PROCESSOR: &str = "http://www.w3.org/TR/scxml/#SCXMLEventProcessor";
/// The type URI of SCXML invocations.
pub const SCXML_INVOKE_TYPE: &str = "http://www.w3.org/TR/scxml/";

/// Interpreter options, fixed at session start.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Options {
    /// The maximum number of microsteps per macrostep: selections that fire transitions and
    /// internal events consumed without enabling any (every [`MicrostepTrace`] record).
    pub microstep_budget: u32,
    /// `send` types the host can deliver; any other type raises `error.execution`.
    pub send_types: Vec<String>,
    /// `invoke` types the host can start; any other type raises `error.execution`.
    pub invoke_types: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            microstep_budget: DEFAULT_MICROSTEP_BUDGET,
            send_types: alloc::vec![SCXML_EVENT_PROCESSOR.to_string(), "scxml".to_string()],
            invoke_types: alloc::vec![
                SCXML_INVOKE_TYPE.to_string(),
                "http://www.w3.org/TR/scxml".to_string(),
                "scxml".to_string(),
            ],
        }
    }
}

/// Whether the session still accepts events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Status {
    /// Waiting for the next external event.
    Running,
    /// Ended by a top-level final state or a cancel request.
    Terminated,
}

/// A started invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Invocation {
    /// The invoking state.
    pub state: StateIdx,
    /// Index of the `<invoke>` in the state's invokes.
    pub index: usize,
    /// The invoke identifier.
    pub invokeid: String,
    /// Whether external events are forwarded to it.
    pub autoforward: bool,
    /// Whether `done.invoke.<id>` was received; a completed invocation is not cancelled.
    pub completed: bool,
}

/// The state of one session, as a value.
///
/// The internal queue is part of the value: `raise`, `send` to `#_internal` and platform events
/// (`error.*`, `done.state.*`) feed it, while external events arrive only as [`Input`]s, so
/// internal events are always processed before the next external one.
// @tg: implements:: [[openspec:scxml-interpretation#Internal And External Queues]]
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Session {
    /// `_sessionid`.
    pub sessionid: String,
    /// Running or terminated.
    pub status: Status,
    /// Active states (never the root).
    pub configuration: StateSet,
    /// Recorded history values.
    pub history: History,
    /// The internal event queue.
    pub internal_queue: VecDeque<Event>,
    /// Started, not yet cancelled invocations.
    pub invocations: Vec<Invocation>,
    /// States whose data items have been initialized (late binding); the root is set at start.
    pub data_initialized: StateSet,
    /// Counter for generated send identifiers.
    pub send_counter: u64,
    /// Counter for generated invoke identifiers.
    pub invoke_counter: u64,
    /// Number of macrosteps completed.
    pub macrosteps: u64,
    /// Options fixed at start.
    pub options: Options,
}

impl Session {
    /// Whether the session has ended.
    pub fn is_terminated(&self) -> bool {
        self.status == Status::Terminated
    }

    /// Active state identifiers in document order.
    pub fn active(&self, chart: &CompiledChart) -> Vec<String> {
        chart.ids(&self.configuration)
    }

    /// Whether the state with this identifier is active.
    pub fn is_active(&self, chart: &CompiledChart, id: &str) -> bool {
        chart
            .index_of(id)
            .is_some_and(|i| self.configuration.contains(i))
    }
}

/// The input that starts a session.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StartInput {
    /// System variables.
    pub info: SessionInfo,
    /// Time and seed for the start macrostep.
    pub env: Environment,
    /// Interpreter options.
    pub options: Options,
    /// Values for top-level data items supplied by the invoking session (the evaluated
    /// `namelist` and `<param>` of `<invoke>`, W3C SCXML 6.4.4): a top-level data item whose
    /// identifier is listed here is initialized to the value instead of its `expr`, content or
    /// `src`. Names that match no top-level data item are ignored.
    #[cfg_attr(feature = "serde", serde(default))]
    pub data: Vec<(String, Value)>,
    /// The session identifiers the SCXML Event I/O Processor can deliver to while this input
    /// is processed, when the host knows them. With a list, an immediate `<send>` to
    /// `#_scxml_<id>` naming no listed session queues `error.communication` (with the send id)
    /// on the internal queue at once, as W3C SCXML 6.2.4 requires (IRP test 496); with `None`
    /// every such target goes to the host, which reports delivery failures back as an
    /// `error.communication` input.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sessions: Option<Vec<String>>,
}

/// What an input asks the session to do.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InputKind {
    /// Process an external event.
    Event(Event),
    /// Cancel the session (the parent's or host's cancel request).
    Cancel,
}

/// One input to [`step`]: an external event or a cancel request, plus its environment.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Input {
    /// Time and seed while processing this input.
    pub env: Environment,
    /// The request.
    pub kind: InputKind,
    /// The session identifiers the SCXML Event I/O Processor can deliver to while this input
    /// is processed, when the host knows them. With a list, an immediate `<send>` to
    /// `#_scxml_<id>` naming no listed session queues `error.communication` (with the send id)
    /// on the internal queue at once, as W3C SCXML 6.2.4 requires (IRP test 496); with `None`
    /// every such target goes to the host, which reports delivery failures back as an
    /// `error.communication` input.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sessions: Option<Vec<String>>,
}

impl Input {
    /// An external event.
    pub fn event(event: Event, env: Environment) -> Self {
        Input {
            env,
            kind: InputKind::Event(event),
            sessions: None,
        }
    }

    /// A cancel request.
    pub fn cancel(env: Environment) -> Self {
        Input {
            env,
            kind: InputKind::Cancel,
            sessions: None,
        }
    }
}

/// A request to deliver an event, produced by `<send>`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SendRequest {
    /// The send identifier (declared, or generated from the session id and a counter).
    pub sendid: String,
    /// The event to deliver, with `sendid` set when the chart declared or stored the identifier,
    /// and `origin`/`origintype` set for the SCXML processor.
    pub event: Event,
    /// The evaluated target; `None` means the session's own external queue.
    pub target: Option<String>,
    /// The evaluated Event I/O Processor type.
    pub type_: String,
    /// Delay before delivery, in milliseconds.
    pub delay_ms: u64,
}

/// A request to start an invocation, produced at the end of a macrostep.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InvokeRequest {
    /// The invoke identifier.
    pub invokeid: String,
    /// The invoking state identifier.
    pub state: String,
    /// The evaluated type.
    pub type_: String,
    /// The evaluated source, if any.
    pub src: Option<String>,
    /// Inline content: the evaluated `expr`, or the raw inline body as a string.
    pub content: Option<Value>,
    /// Evaluated `namelist` and `<param>` values as an object, or undefined when none.
    pub params: Value,
    /// Whether external events are forwarded to it.
    pub autoforward: bool,
}

/// An interaction with the outside world, returned by the step instead of performed.
// @tg: implements:: [[openspec:scxml-interpretation#Send And Cancel Effects]]
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Effect {
    /// Deliver an event, possibly after a delay.
    Send(SendRequest),
    /// Cancel a pending delayed send.
    Cancel {
        /// The send identifier.
        sendid: String,
    },
    /// Start an invocation.
    Invoke(InvokeRequest),
    /// Cancel an invocation whose state was exited. From then on the host must not deliver
    /// events from that invocation; events it already queued are still delivered.
    CancelInvoke {
        /// The invoke identifier.
        invokeid: String,
    },
    /// Forward an external event to an `autoforward` invocation.
    Forward {
        /// The invoke identifier.
        invokeid: String,
        /// The event.
        event: Event,
    },
    /// `<log>`.
    Log {
        /// The label.
        label: Option<String>,
        /// The evaluated expression.
        value: Option<Value>,
    },
    /// The session ended.
    SessionDone {
        /// The evaluated `donedata` of the top-level final state, or undefined.
        data: Value,
        /// True when ended by a cancel request rather than a top-level final state.
        cancelled: bool,
    },
}

/// The result of a macrostep.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The session after the macrostep.
    pub session: Session,
    /// Effects in the order they were produced.
    pub effects: Vec<Effect>,
    /// The macrostep record.
    pub trace: MacrostepTrace,
}

/// Why a session could not start.
#[derive(Clone, Debug, PartialEq)]
pub enum StartError {
    /// The chart names a datamodel the host did not register.
    UnsupportedDatamodel {
        /// The datamodel name.
        name: String,
    },
    /// The datamodel passed in is not the one the chart names.
    DatamodelMismatch {
        /// The chart's datamodel.
        chart: String,
        /// The datamodel given.
        given: String,
    },
    /// The datamodel refused to initialize the session.
    Datamodel(EvalError),
    /// The start macrostep did not stabilize within the microstep budget.
    BudgetExceeded {
        /// The budget.
        budget: u32,
        /// The session and effects as left when the budget ran out.
        partial: Box<Outcome>,
    },
}

/// Why a step was rejected or cut short.
#[derive(Clone, Debug, PartialEq)]
pub enum StepError {
    /// The session has terminated and accepts no further input.
    Terminated,
    /// The macrostep did not stabilize within the microstep budget.
    BudgetExceeded {
        /// The budget.
        budget: u32,
        /// The session and effects as left when the budget ran out.
        partial: Box<Outcome>,
    },
}

/// A fresh session value for `chart`, before the start macrostep.
fn new_session(chart: &CompiledChart, sessionid: String, options: Options) -> Session {
    Session {
        sessionid,
        status: Status::Running,
        configuration: chart.empty_set(),
        history: History::new(chart.len()),
        internal_queue: VecDeque::new(),
        invocations: Vec::new(),
        data_initialized: chart.empty_set(),
        send_counter: 0,
        invoke_counter: 0,
        macrosteps: 0,
        options,
    }
}

/// Start a session: initialize the datamodel (all data under early binding, top-level data under
/// late binding), run the top-level script, enter the initial configuration and run macrosteps
/// until stable. `dm` must be a fresh instance of the datamodel the chart names.
// @tg: implements:: [[openspec:scxml-interpretation#Session Initialization]]
// @tg: implements:: [[openspec:datamodel#Data Binding Modes]]
// @tg: implements:: [[openspec:datamodel#System Variables]] {confidence: partial, note: "the core passes system variables and _event to the datamodel"}
pub fn start(
    chart: &CompiledChart,
    input: StartInput,
    dm: &mut dyn Datamodel,
) -> Result<Outcome, StartError> {
    if dm.name() != chart.datamodel() {
        return Err(StartError::DatamodelMismatch {
            chart: chart.datamodel().to_string(),
            given: dm.name().to_string(),
        });
    }
    dm.init_session(&input.info)
        .map_err(StartError::Datamodel)?;
    dm.set_environment(&input.env);
    let session = new_session(chart, input.info.sessionid.clone(), input.options);
    let invoker_data = input.data;
    let mut d = Driver::new(
        chart,
        session,
        dm,
        TraceInput::Start {
            sessionid: input.info.sessionid,
        },
    );
    d.sessions = input.sessions;
    let budget = d.s.options.microstep_budget;
    match d.run_start(&invoker_data) {
        Ok(()) => Ok(d.finish()),
        Err(Budget) => Err(StartError::BudgetExceeded {
            budget,
            partial: Box::new(d.finish()),
        }),
    }
}

/// Process one input: run one macrostep for an external event, or terminate on a cancel request.
/// Equal sessions, inputs and datamodel states give equal results.
// @tg: implements:: [[openspec:scxml-interpretation#Pure Step Function]]
// @tg: implements:: [[openspec:scxml-interpretation#Macrostep Execution]]
pub fn step(
    chart: &CompiledChart,
    session: Session,
    input: Input,
    dm: &mut dyn Datamodel,
) -> Result<Outcome, StepError> {
    if session.status == Status::Terminated {
        return Err(StepError::Terminated);
    }
    dm.set_environment(&input.env);
    let trace_input = match &input.kind {
        InputKind::Event(e) => TraceInput::Event(e.clone()),
        InputKind::Cancel => TraceInput::Cancel,
    };
    let budget = session.options.microstep_budget;
    let mut d = Driver::new(chart, session, dm, trace_input);
    d.sessions = input.sessions;
    let r = match input.kind {
        InputKind::Cancel => {
            d.exit_interpreter(true);
            Ok(())
        }
        InputKind::Event(ev) => d.run_external(ev),
    };
    match r {
        Ok(()) => Ok(d.finish()),
        Err(Budget) => Err(StepError::BudgetExceeded {
            budget,
            partial: Box::new(d.finish()),
        }),
    }
}

/// The microstep budget ran out.
struct Budget;

/// Executable content stopped at a failing element (the error event is already queued).
struct Stop;

/// The configuration as seen by `In()`.
struct Active<'a> {
    chart: &'a CompiledChart,
    config: &'a StateSet,
}

impl StateQuery for Active<'_> {
    fn in_state(&self, id: &str) -> bool {
        self.chart
            .index_of(id)
            .is_some_and(|i| self.config.contains(i))
    }

    fn active_states(&self) -> Option<Vec<String>> {
        Some(
            self.chart
                .states()
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 0 && self.config.contains(*i))
                .map(|(_, s)| s.id.clone())
                .collect(),
        )
    }
}

/// Message of an `error.execution` the core raises itself, not from a datamodel failure.
pub const EXECUTION_ERROR_MESSAGE: &str = "execution error";
/// Message of an `error.communication` for a target the core knows is unreachable.
pub const COMMUNICATION_ERROR_MESSAGE: &str = "target session does not exist";

/// A platform error event whose data is `{kind, message}`, matching the predefined `error.*`
/// payload type. `kind` is the part of the name after `error.`.
fn platform_error(name: &str, message: &str, sendid: Option<String>) -> Event {
    let kind = name.strip_prefix("error.").unwrap_or(name);
    Event {
        name: name.to_string(),
        type_: EventType::Platform,
        sendid,
        data: Value::Object(Vec::from([
            (String::from("kind"), Value::String(kind.to_string())),
            (String::from("message"), Value::String(message.to_string())),
        ])),
        ..Event::default()
    }
}

/// The datamodel as the driver calls it: forwards every operation and remembers the error of
/// the most recent call, so the `error.execution` it causes can carry its message.
struct Capture<'d> {
    inner: &'d mut dyn Datamodel,
    last: Option<EvalError>,
}

impl Capture<'_> {
    fn note<T>(&mut self, r: Result<T, EvalError>) -> Result<T, EvalError> {
        self.last = r.as_ref().err().cloned();
        r
    }
}

impl Datamodel for Capture<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        let r = self.inner.init_session(info);
        self.note(r)
    }
    fn set_environment(&mut self, env: &Environment) {
        self.inner.set_environment(env)
    }
    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        let r = self.inner.declare(id);
        self.note(r)
    }
    fn set_event(&mut self, event: &Event) {
        self.inner.set_event(event)
    }
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        let r = self.inner.eval_cond(expr, states);
        self.note(r)
    }
    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        let r = self.inner.eval_value(expr, states);
        self.note(r)
    }
    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        let r = self.inner.eval_content(content);
        self.note(r)
    }
    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        let r = self.inner.assign_expr(location, expr, states);
        self.note(r)
    }
    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        let r = self.inner.assign_value(location, value);
        self.note(r)
    }
    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        let r = self.inner.bind_var(name, value);
        self.note(r)
    }
    fn is_location(&mut self, location: &str) -> bool {
        self.last = None;
        self.inner.is_location(location)
    }
    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError> {
        let r = self.inner.exec_script(script, states);
        self.note(r)
    }
    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        let r = self.inner.foreach_items(array, states);
        self.note(r)
    }
    fn snapshot(&self) -> Value {
        self.inner.snapshot()
    }
    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let r = self.inner.restore(snapshot);
        self.note(r)
    }
}

/// Parse a CSS2 time (`5s`, `250ms`, `1.5s`, or a bare number of milliseconds) into
/// milliseconds, without floating point.
pub fn parse_delay(text: &str) -> Option<u64> {
    let t = text.trim();
    let (num, factor) = if let Some(n) = t.strip_suffix("ms") {
        (n.trim(), 1u64)
    } else if let Some(n) = t.strip_suffix('s') {
        (n.trim(), 1000u64)
    } else {
        (t, 1u64)
    };
    if num.is_empty() {
        return None;
    }
    let (int, frac) = match num.split_once('.') {
        Some((i, f)) => (i, f),
        None => (num, ""),
    };
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let mut ms: u64 = 0;
    for b in int.bytes() {
        ms = ms.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
    }
    ms = ms.checked_mul(factor)?;
    // Fractional part, truncated to whole milliseconds.
    let mut scale = factor;
    for b in frac.bytes() {
        scale /= 10;
        if scale == 0 {
            break;
        }
        ms = ms.checked_add(u64::from(b - b'0') * scale)?;
    }
    Some(ms)
}

fn value_to_string(v: Value) -> Result<String, EvalError> {
    match v {
        Value::String(s) => Ok(s),
        Value::Bool(b) => Ok(if b { "true" } else { "false" }.to_string()),
        Value::Number(n) => {
            if n.is_finite() && n == (n as i64) as f64 {
                Ok(format!("{}", n as i64))
            } else {
                Ok(format!("{n}"))
            }
        }
        other => Err(EvalError::new(format!("expected a string, got {other:?}"))),
    }
}

/// Executes one session's macrosteps against a datamodel.
struct Driver<'c, 'd> {
    chart: &'c CompiledChart,
    s: Session,
    dm: Capture<'d>,
    effects: Vec<Effect>,
    to_invoke: StateSet,
    running: bool,
    terminated_now: bool,
    microsteps: Vec<MicrostepTrace>,
    used: u32,
    input: TraceInput,
    /// The reachable session identifiers of the input, if the host listed them.
    sessions: Option<Vec<String>>,
}

impl<'c, 'd> Driver<'c, 'd> {
    fn new(
        chart: &'c CompiledChart,
        s: Session,
        dm: &'d mut dyn Datamodel,
        input: TraceInput,
    ) -> Self {
        Driver {
            chart,
            s,
            dm: Capture {
                inner: dm,
                last: None,
            },
            effects: Vec::new(),
            to_invoke: chart.empty_set(),
            running: true,
            terminated_now: false,
            microsteps: Vec::new(),
            used: 0,
            input,
            sessions: None,
        }
    }

    fn finish(mut self) -> Outcome {
        let trace = MacrostepTrace {
            index: self.s.macrosteps,
            input: self.input,
            microsteps: self.microsteps,
            configuration: self.chart.ids(&self.s.configuration),
            effects: self.effects.clone(),
            terminated: self.terminated_now,
        };
        self.s.macrosteps += 1;
        Outcome {
            session: self.s,
            effects: self.effects,
            trace,
        }
    }

    // ----- datamodel helpers -------------------------------------------------------------

    /// Queue `error.execution`; its message is the datamodel error of the most recent
    /// datamodel call when that call failed, otherwise [`EXECUTION_ERROR_MESSAGE`].
    fn queue_error(&mut self, sendid: Option<String>) {
        let last = self.dm.last.take();
        let message = last
            .as_ref()
            .map_or(EXECUTION_ERROR_MESSAGE, |e| e.message.as_str());
        self.s
            .internal_queue
            .push_back(platform_error("error.execution", message, sendid));
    }

    fn eval_value(&mut self, expr: &str) -> Result<Value, EvalError> {
        let q = Active {
            chart: self.chart,
            config: &self.s.configuration,
        };
        self.dm.eval_value(expr, &q)
    }

    fn eval_string(&mut self, expr: &str) -> Result<String, EvalError> {
        self.eval_value(expr).and_then(value_to_string)
    }

    fn eval_cond(&mut self, expr: &str) -> Result<bool, EvalError> {
        let q = Active {
            chart: self.chart,
            config: &self.s.configuration,
        };
        self.dm.eval_cond(expr, &q)
    }

    fn eval_payload(
        &mut self,
        namelist: &[String],
        params: &[Param],
        content: Option<&Content>,
    ) -> Result<Value, EvalError> {
        if let Some(c) = content {
            return match (&c.expr, &c.body) {
                (Some(x), _) => self.eval_value(x),
                (None, Some(b)) => self.dm.eval_content(b),
                (None, None) => Ok(Value::Undefined),
            };
        }
        if namelist.is_empty() && params.is_empty() {
            return Ok(Value::Undefined);
        }
        let mut pairs = Vec::new();
        for name in namelist {
            let v = self.eval_value(name)?;
            pairs.push((name.clone(), v));
        }
        for p in params {
            let v = match (&p.expr, &p.location) {
                (Some(x), _) => self.eval_value(x)?,
                (None, Some(l)) => self.eval_value(l)?,
                (None, None) => Value::Undefined,
            };
            pairs.push((p.name.clone(), v));
        }
        Ok(Value::Object(pairs))
    }

    fn init_data(&mut self, items: &'c [Data]) {
        self.init_data_with(items, &[]);
    }

    /// Initialize data items; an item named in `given` takes that value (invoke parameters).
    fn init_data_with(&mut self, items: &'c [Data], given: &[(String, Value)]) {
        for d in items {
            let r = if let Some((_, v)) = given.iter().find(|(n, _)| *n == d.id) {
                self.dm.assign_value(&d.id, v.clone())
            } else if let Some(e) = &d.expr {
                self.eval_value(e)
                    .and_then(|v| self.dm.assign_value(&d.id, v))
            } else if let Some(c) = &d.content {
                self.dm
                    .eval_content(c)
                    .and_then(|v| self.dm.assign_value(&d.id, v))
            } else if let Some(src) = &d.src {
                Err(EvalError::new(format!("data src {src} was not resolved")))
            } else {
                Ok(())
            };
            if r.is_err() {
                self.queue_error(None);
            }
        }
    }

    // ----- executable content -------------------------------------------------------------

    /// Run a block; an error queues `error.execution` and stops the rest of the block.
    // @tg: implements:: [[openspec:scxml-interpretation#Execution Errors]]
    fn exec_block(&mut self, block: &'c [Exec]) -> Result<(), Stop> {
        for e in block {
            self.exec(e)?;
        }
        Ok(())
    }

    fn fail<T>(&mut self, sendid: Option<String>) -> Result<T, Stop> {
        self.queue_error(sendid);
        Err(Stop)
    }

    fn exec(&mut self, e: &'c Exec) -> Result<(), Stop> {
        match &e.kind {
            ExecKind::Raise { event } => {
                self.s
                    .internal_queue
                    .push_back(Event::named(event.clone(), EventType::Internal));
                Ok(())
            }
            ExecKind::Send(send) => self.exec_send(send),
            ExecKind::Cancel { sendid, sendidexpr } => {
                let id = match (sendid, sendidexpr) {
                    (Some(s), _) => s.clone(),
                    (None, Some(x)) => match self.eval_string(x) {
                        Ok(s) => s,
                        Err(_) => return self.fail(None),
                    },
                    (None, None) => return self.fail(None),
                };
                self.effects.push(Effect::Cancel { sendid: id });
                Ok(())
            }
            ExecKind::Log { label, expr } => {
                let value = match expr {
                    Some(x) => match self.eval_value(x) {
                        Ok(v) => Some(v),
                        Err(_) => return self.fail(None),
                    },
                    None => None,
                };
                self.effects.push(Effect::Log {
                    label: label.clone(),
                    value,
                });
                Ok(())
            }
            ExecKind::Assign {
                location,
                expr,
                content,
            } => {
                let r = match (expr, content) {
                    (Some(x), _) => {
                        let q = Active {
                            chart: self.chart,
                            config: &self.s.configuration,
                        };
                        self.dm.assign_expr(location, x, &q)
                    }
                    (None, Some(c)) => self
                        .dm
                        .eval_content(c)
                        .and_then(|v| self.dm.assign_value(location, v)),
                    (None, None) => self.dm.assign_value(location, Value::Undefined),
                };
                if r.is_err() {
                    return self.fail(None);
                }
                Ok(())
            }
            ExecKind::Script(script) => {
                let r = match &script.body {
                    Some(body) => {
                        let q = Active {
                            chart: self.chart,
                            config: &self.s.configuration,
                        };
                        self.dm.exec_script(body, &q)
                    }
                    None => Err(EvalError::new("script source was not resolved")),
                };
                if r.is_err() {
                    return self.fail(None);
                }
                Ok(())
            }
            ExecKind::If { branches } => {
                for br in branches {
                    let take = match &br.cond {
                        None => true,
                        Some(c) => match self.eval_cond(c) {
                            Ok(b) => b,
                            // The condition counts as false, error.execution is queued, and the
                            // `<if>` element has failed, so the enclosing block stops.
                            Err(_) => return self.fail(None),
                        },
                    };
                    if take {
                        return self.exec_block(&br.body);
                    }
                }
                Ok(())
            }
            ExecKind::Foreach {
                array,
                item,
                index,
                body,
            } => {
                let items = {
                    let q = Active {
                        chart: self.chart,
                        config: &self.s.configuration,
                    };
                    self.dm.foreach_items(array, &q)
                };
                let items = match items {
                    Ok(v) => v,
                    Err(_) => return self.fail(None),
                };
                for (i, v) in items.into_iter().enumerate() {
                    if self.dm.bind_var(item, v).is_err() {
                        return self.fail(None);
                    }
                    if let Some(ix) = index
                        && self.dm.bind_var(ix, Value::Number(i as f64)).is_err()
                    {
                        return self.fail(None);
                    }
                    self.exec_block(body)?;
                }
                Ok(())
            }
            // Executable content from other namespaces is ignored.
            ExecKind::Extension(_) => Ok(()),
        }
    }

    fn exec_send(&mut self, send: &'c Send) -> Result<(), Stop> {
        // 1. Identifier, stored at `idlocation` before anything else is evaluated, so error
        //    events raised below can carry it.
        let (sendid, visible) = match (&send.id, &send.idlocation) {
            (Some(id), _) => (id.clone(), true),
            (None, loc) => {
                self.s.send_counter += 1;
                let id = format!("{}.send.{}", self.s.sessionid, self.s.send_counter);
                if let Some(loc) = loc
                    && self
                        .dm
                        .assign_value(loc, Value::String(id.clone()))
                        .is_err()
                {
                    return self.fail(Some(id));
                }
                (id, loc.is_some())
            }
        };
        let err_id = Some(sendid.clone());
        // 2. Event name, target, type and delay.
        let name = match (&send.event, &send.eventexpr) {
            (Some(n), _) => n.clone(),
            (None, Some(x)) => match self.eval_string(x) {
                Ok(n) => n,
                Err(_) => return self.fail(err_id),
            },
            (None, None) => String::new(),
        };
        let target = match (&send.target, &send.targetexpr) {
            (Some(t), _) => Some(t.clone()),
            (None, Some(x)) => match self.eval_string(x) {
                Ok(t) => Some(t),
                Err(_) => return self.fail(err_id),
            },
            (None, None) => None,
        };
        let type_ = match (&send.type_, &send.typeexpr) {
            (Some(t), _) => t.clone(),
            (None, Some(x)) => match self.eval_string(x) {
                Ok(t) => t,
                Err(_) => return self.fail(err_id),
            },
            (None, None) => SCXML_EVENT_PROCESSOR.to_string(),
        };
        let delay_text = match (&send.delay, &send.delayexpr) {
            (Some(d), _) => Some(d.clone()),
            (None, Some(x)) => match self.eval_string(x) {
                Ok(d) => Some(d),
                Err(_) => return self.fail(err_id),
            },
            (None, None) => None,
        };
        let delay_ms = match delay_text {
            None => 0,
            Some(t) => match parse_delay(&t) {
                Some(ms) => ms,
                None => return self.fail(err_id),
            },
        };
        // 3. Data.
        let data = match self.eval_payload(&send.namelist, &send.params, send.content.as_ref()) {
            Ok(v) => v,
            Err(_) => return self.fail(err_id),
        };
        // 4. Routing. An unsupported type or an illegal SCXML target is error.execution
        //    (W3C SCXML 6.2.4); failures to deliver are reported by the host as
        //    error.communication.
        if !self.s.options.send_types.contains(&type_) {
            return self.fail(err_id);
        }
        let scxml = type_ == SCXML_EVENT_PROCESSOR || type_ == "scxml";
        if scxml
            && let Some(t) = &target
            && !(t.starts_with("#_") || t.contains(':'))
        {
            return self.fail(err_id);
        }
        // A session the host says does not exist cannot be dispatched to: error.communication
        // goes on the internal queue at once and the block continues (W3C SCXML 6.2.4).
        if scxml
            && delay_ms == 0
            && let (Some(t), Some(known)) = (&target, &self.sessions)
            && let Some(id) = t.strip_prefix("#_scxml_")
            && !known.iter().any(|k| k == id)
        {
            self.s.internal_queue.push_back(platform_error(
                "error.communication",
                COMMUNICATION_ERROR_MESSAGE,
                err_id,
            ));
            return Ok(());
        }
        let exposed = if visible { Some(sendid.clone()) } else { None };
        if target.as_deref() == Some("#_internal") && delay_ms == 0 {
            self.s.internal_queue.push_back(Event {
                name,
                type_: EventType::Internal,
                sendid: exposed,
                data,
                ..Event::default()
            });
            return Ok(());
        }
        let event = Event {
            name,
            type_: EventType::External,
            sendid: exposed,
            origin: if scxml {
                Some(format!("#_scxml_{}", self.s.sessionid))
            } else {
                None
            },
            origintype: if scxml {
                Some(SCXML_EVENT_PROCESSOR.to_string())
            } else {
                None
            },
            invokeid: None,
            data,
            ..Event::default()
        };
        self.effects.push(Effect::Send(SendRequest {
            sendid,
            event,
            target,
            type_,
            delay_ms,
        }));
        Ok(())
    }

    // ----- selection, microsteps, macrosteps ---------------------------------------------

    fn select(&mut self, event: Option<&str>) -> Vec<TransIdx> {
        let chart = self.chart;
        let config = &self.s.configuration;
        let history = &self.s.history;
        let queue = &mut self.s.internal_queue;
        let dm = &mut self.dm;
        kernel::select_transitions(chart, config, history, event, |t| {
            let Some(cond) = chart.transition(t).cond.as_deref() else {
                return true;
            };
            let q = Active { chart, config };
            match dm.eval_cond(cond, &q) {
                Ok(b) => b,
                Err(e) => {
                    queue.push_back(platform_error("error.execution", &e.message, None));
                    false
                }
            }
        })
    }

    fn record(&mut self, event: Option<String>, ts: &[TransIdx]) {
        let transitions = ts
            .iter()
            .filter_map(|t| {
                let tr = self.chart.transition(*t);
                tr.index.map(|index| TransitionRef {
                    source: self.chart.state(tr.source).id.clone(),
                    index,
                })
            })
            .collect();
        self.microsteps.push(MicrostepTrace { event, transitions });
    }

    /// Charge one microstep record (a selection that fired transitions, or an event consumed
    /// without enabling any) against the budget. Counting the latter too bounds a macrostep in
    /// which every consumed event queues another one, such as a failing eventless guard
    /// re-raising `error.execution` forever.
    fn charge(&mut self) -> Result<(), Budget> {
        if self.used >= self.s.options.microstep_budget {
            return Err(Budget);
        }
        self.used += 1;
        Ok(())
    }

    fn microstep(&mut self, event: Option<String>, ts: &[TransIdx]) -> Result<(), Budget> {
        self.charge()?;
        self.record(event, ts);
        let chart = self.chart;
        let domains = kernel::transition_domains(chart, &self.s.history, ts);
        self.exit_states(ts);
        for &t in ts {
            let _ = self.exec_block(&chart.transition(t).body);
        }
        self.enter_states(ts, &domains);
        Ok(())
    }

    // @tg: implements:: [[openspec:scxml-interpretation#Exit And Entry Order]]
    fn exit_states(&mut self, ts: &[TransIdx]) {
        let chart = self.chart;
        let exit = kernel::compute_exit_set(chart, &self.s.configuration, &self.s.history, ts);
        self.to_invoke.difference_with(&exit);
        kernel::record_history(chart, &self.s.configuration, &exit, &mut self.s.history);
        for s in exit.to_vec_rev() {
            for b in &chart.state(s).onexit {
                let _ = self.exec_block(b);
            }
            self.cancel_invocations(s);
            self.s.configuration.remove(s);
        }
    }

    fn cancel_invocations(&mut self, s: StateIdx) {
        let mut keep = Vec::new();
        for inv in core::mem::take(&mut self.s.invocations) {
            if inv.state == s {
                if !inv.completed {
                    self.effects.push(Effect::CancelInvoke {
                        invokeid: inv.invokeid,
                    });
                }
            } else {
                keep.push(inv);
            }
        }
        self.s.invocations = keep;
    }

    // @tg: implements:: [[openspec:scxml-interpretation#Final States And Done Events]]
    fn enter_states(&mut self, ts: &[TransIdx], domains: &[Option<StateIdx>]) {
        let chart = self.chart;
        let plan = kernel::compute_entry_set(chart, &self.s.history, ts, domains);
        let late = chart.binding() == Binding::Late;
        for s in plan.states.to_vec() {
            self.s.configuration.insert(s);
            self.to_invoke.insert(s);
            let st = chart.state(s);
            if late && !self.s.data_initialized.contains(s) {
                self.s.data_initialized.insert(s);
                self.init_data(&st.data);
            }
            for b in &st.onentry {
                let _ = self.exec_block(b);
            }
            if plan.default_entry.contains(s)
                && let Some(it) = st.initial
            {
                let _ = self.exec_block(&chart.transition(it).body);
            }
            if let Some(&(_, t)) = plan.default_history_content.iter().find(|(p, _)| *p == s) {
                let _ = self.exec_block(&chart.transition(t).body);
            }
            if st.kind == StateKind::Final {
                let parent = st.parent.unwrap_or(ROOT);
                if parent == ROOT {
                    self.running = false;
                } else {
                    let data = self.done_data(s);
                    let pid = &chart.state(parent).id;
                    self.s.internal_queue.push_back(Event {
                        name: format!("done.state.{pid}"),
                        type_: EventType::Platform,
                        data,
                        ..Event::default()
                    });
                    if let Some(gp) = chart.state(parent).parent
                        && gp != ROOT
                        && chart.state(gp).kind == StateKind::Parallel
                        && chart
                            .state(gp)
                            .children
                            .iter()
                            .all(|c| kernel::is_in_final_state(chart, &self.s.configuration, *c))
                    {
                        let gid = &chart.state(gp).id;
                        self.s.internal_queue.push_back(Event::named(
                            format!("done.state.{gid}"),
                            EventType::Platform,
                        ));
                    }
                }
            }
        }
    }

    fn done_data(&mut self, s: StateIdx) -> Value {
        let chart = self.chart;
        let Some(dd) = &chart.state(s).donedata else {
            return Value::Undefined;
        };
        match self.eval_payload(&[], &dd.params, dd.content.as_ref()) {
            Ok(v) => v,
            Err(_) => {
                self.queue_error(None);
                Value::Undefined
            }
        }
    }

    /// The inner loop of `mainEventLoop` plus the invokes at the end of the macrostep, repeated
    /// while invoking raised internal events.
    fn run_macrostep(&mut self) -> Result<(), Budget> {
        loop {
            while self.running {
                let mut enabled = self.select(None);
                let mut name = None;
                if enabled.is_empty() {
                    match self.s.internal_queue.pop_front() {
                        None => break,
                        Some(ev) => {
                            self.dm.set_event(&ev);
                            enabled = self.select(Some(&ev.name));
                            if enabled.is_empty() {
                                self.charge()?;
                                self.record(Some(ev.name), &[]);
                                continue;
                            }
                            name = Some(ev.name);
                        }
                    }
                }
                self.microstep(name, &enabled)?;
            }
            if !self.running {
                self.exit_interpreter(false);
                return Ok(());
            }
            self.start_invokes();
            if self.s.internal_queue.is_empty() {
                return Ok(());
            }
        }
    }

    fn run_start(&mut self, invoker_data: &[(String, Value)]) -> Result<(), Budget> {
        let chart = self.chart;
        // Create every data item, then initialize them per the binding mode.
        for s in 0..chart.len() {
            for d in &chart.state(s).data {
                if self.dm.declare(&d.id).is_err() {
                    self.queue_error(None);
                }
            }
        }
        match chart.binding() {
            Binding::Early => {
                for s in 0..chart.len() {
                    let given = if s == ROOT { invoker_data } else { &[] };
                    self.init_data_with(&chart.state(s).data, given);
                    self.s.data_initialized.insert(s);
                }
            }
            Binding::Late => {
                self.init_data_with(&chart.state(ROOT).data, invoker_data);
                self.s.data_initialized.insert(ROOT);
            }
        }
        if let Some(script) = &chart.chart().script {
            let r = match &script.body {
                Some(body) => {
                    let q = Active {
                        chart,
                        config: &self.s.configuration,
                    };
                    self.dm.exec_script(body, &q)
                }
                None => Err(EvalError::new("script source was not resolved")),
            };
            if r.is_err() {
                self.queue_error(None);
            }
        }
        if let Some(it) = chart.state(ROOT).initial {
            let domains = kernel::transition_domains(chart, &self.s.history, &[it]);
            self.enter_states(&[it], &domains);
        }
        self.run_macrostep()
    }

    // @tg: implements:: [[openspec:scxml-interpretation#Invoke Lifecycle]]
    fn run_external(&mut self, ev: Event) -> Result<(), Budget> {
        // Every event the host delivers is processed. Ignoring events from a cancelled
        // invocation is the host's job when it receives them (W3C SCXML 6.4: once cancelled,
        // the processor MUST NOT insert the invoked session's events into the external queue);
        // events queued before the cancellation, such as the `done.invoke.<id>` of a child that
        // finished first, are still processed (IRP test 236). Only active invocations run
        // `finalize` or get the event forwarded.
        if let Some(rest) = ev.name.strip_prefix("done.invoke.") {
            for inv in self.s.invocations.iter_mut() {
                if inv.invokeid == rest || ev.invokeid.as_deref() == Some(inv.invokeid.as_str()) {
                    inv.completed = true;
                }
            }
        }
        self.dm.set_event(&ev);
        let mut invs = self.s.invocations.clone();
        invs.sort_by_key(|i| (i.state, i.index));
        let chart = self.chart;
        for inv in invs {
            if ev.invokeid.as_deref() == Some(inv.invokeid.as_str()) {
                let finalize = &chart.state(inv.state).invokes[inv.index].finalize;
                let _ = self.exec_block(finalize);
            }
            if inv.autoforward && !inv.completed {
                self.effects.push(Effect::Forward {
                    invokeid: inv.invokeid.clone(),
                    event: ev.clone(),
                });
            }
        }
        let enabled = self.select(Some(&ev.name));
        if enabled.is_empty() {
            self.record(Some(ev.name), &[]);
        } else {
            self.microstep(Some(ev.name), &enabled)?;
        }
        self.run_macrostep()
    }

    fn start_invokes(&mut self) {
        let chart = self.chart;
        for s in self.to_invoke.to_vec() {
            for (i, inv) in chart.state(s).invokes.iter().enumerate() {
                self.invoke(s, i, inv);
            }
        }
        self.to_invoke.clear();
    }

    fn invoke(&mut self, s: StateIdx, index: usize, inv: &'c Invoke) {
        let chart = self.chart;
        let state_id = &chart.state(s).id;
        let invokeid = match &inv.id {
            Some(id) => id.clone(),
            None => {
                self.s.invoke_counter += 1;
                let id = format!("{}.{}", state_id, self.s.invoke_counter);
                if let Some(loc) = &inv.idlocation
                    && self
                        .dm
                        .assign_value(loc, Value::String(id.clone()))
                        .is_err()
                {
                    self.queue_error(None);
                    return;
                }
                id
            }
        };
        let type_ = match (&inv.type_, &inv.typeexpr) {
            (Some(t), _) => Ok(t.clone()),
            (None, Some(x)) => self.eval_string(x),
            (None, None) => Ok(SCXML_INVOKE_TYPE.to_string()),
        };
        let src = match (&inv.src, &inv.srcexpr) {
            (Some(t), _) => Ok(Some(t.clone())),
            (None, Some(x)) => self.eval_string(x).map(Some),
            (None, None) => Ok(None),
        };
        let content = match &inv.content {
            Some(Content { expr: Some(x), .. }) => self.eval_value(x).map(Some),
            Some(Content {
                expr: None,
                body: Some(b),
                ..
            }) => Ok(Some(Value::String(b.clone()))),
            _ => Ok(None),
        };
        let params = self.eval_payload(&inv.namelist, &inv.params, None);
        let (Ok(type_), Ok(src), Ok(content), Ok(params)) = (type_, src, content, params) else {
            self.queue_error(None);
            return;
        };
        if !self.s.options.invoke_types.contains(&type_) {
            self.queue_error(None);
            return;
        }
        self.s.invocations.push(Invocation {
            state: s,
            index,
            invokeid: invokeid.clone(),
            autoforward: inv.autoforward,
            completed: false,
        });
        self.effects.push(Effect::Invoke(InvokeRequest {
            invokeid,
            state: state_id.clone(),
            type_,
            src,
            content,
            params,
            autoforward: inv.autoforward,
        }));
    }

    /// `exitInterpreter`: exit every active state in exit order, cancel invocations and emit
    /// one session-done effect.
    // @tg: implements:: [[openspec:scxml-interpretation#Session Termination]]
    fn exit_interpreter(&mut self, cancelled: bool) {
        let chart = self.chart;
        let mut data = Value::Undefined;
        for s in self.s.configuration.to_vec_rev() {
            for b in &chart.state(s).onexit {
                let _ = self.exec_block(b);
            }
            self.cancel_invocations(s);
            self.s.configuration.remove(s);
            if chart.state(s).kind == StateKind::Final && chart.state(s).parent == Some(ROOT) {
                data = self.done_data(s);
            }
        }
        self.effects.push(Effect::SessionDone { data, cancelled });
        self.s.internal_queue.clear();
        self.to_invoke.clear();
        self.s.status = Status::Terminated;
        self.running = false;
        self.terminated_now = true;
    }
}
