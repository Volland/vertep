//! The in-memory test host: many sessions, the SCXML Event I/O Processor between them, invoked
//! `scxml` sessions, autoforward, `done.invoke`, delayed sends on a virtual clock and the
//! external queues, without real time, threads or network.
//!
//! # Scheduling
//!
//! The host keeps one global FIFO of deliveries (an input for one session). Every effect a
//! macrostep emits is performed at once and in order, so deliveries it produces are queued in
//! the order the chart produced them; a session's external queue is the subsequence of the
//! FIFO addressed to it. The host takes the oldest delivery, runs one macrostep for it, and
//! repeats. Only when nothing is ready does virtual time advance, straight to the earliest
//! pending delayed send (ties in the order the sends were made). Processing is therefore
//! infinitely fast compared to any delay, which is what the IRP tests' timeouts assume.
//!
//! # Event I/O Processor
//!
//! Sends of type `http://www.w3.org/TR/scxml/#SCXMLEventProcessor` (or `scxml`) are routed by
//! target: none to the sender's own external queue, `#_internal` (delayed only; immediate ones
//! never leave the core) to the sender as an internal event, `#_parent` to the invoking session
//! with `invokeid` set, `#_scxml_<sessionid>` to that session and `#_<invokeid>` to a child the
//! sender invoked. Every input lists the running sessions, so the core itself raises
//! `error.communication` on the internal queue for an immediate send to `#_scxml_<id>` of no
//! running session. Any other target that names no running session (a delayed send, `#_parent`
//! of the root, `#_<invokeid>` of no running child) is undeliverable when the host routes it:
//! the host queues `error.communication` with the send's `sendid` for the sender. Each session's
//! `_ioprocessors` lists the processor with its `#_scxml_<sessionid>` location.
//!
//! # Invocations
//!
//! An `Invoke` effect of type SCXML starts a child session at once from the inline content or
//! from `src` through the [`ChartLoader`]; its `namelist` and `<param>` values initialize the
//! child's top-level data items of the same names. When a child ends on its own, its parent
//! receives `done.invoke.<invokeid>` with the child's done data after every event the child
//! sent before. `CancelInvoke` cancels a child (and through its own effects its children)
//! without a `done.invoke`; from then on nothing the child sends is delivered, while events it
//! sent before are still queued for the parent. `Forward` delivers a copy of the event to the child, with `invokeid`
//! blank because it names the parent's invocation, not one of the child's. Delayed sends
//! of an ended session are dropped.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, Event, EventType, SessionInfo, Value,
};
use vertep_core::null::create_datamodel;
use vertep_core::session::{
    self, Effect, Input, InputKind, Options, Outcome, SCXML_EVENT_PROCESSOR, SendRequest, Session,
    StartError, StartInput, StepError,
};
use vertep_core::tape::{OracleInput, OracleRecord, TapeDatamodel, TapeEntry};
use vertep_scxml::{ReadOptions, Resolution, ResolveError, SourceRequest};

/// Index of a session in the host.
pub type SessionIdx = usize;

/// The datamodels the host's documents may name.
pub const HOST_DATAMODELS: [&str; 2] = ["null", "ecmascript"];

/// Limits of one host run ([[openspec:scxml-conformance#Pass Fail Protocol]]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// The most inputs (macrosteps after start) the host runs across all sessions.
    pub max_steps: u64,
    /// The latest virtual time, in milliseconds after the run started, the clock may reach.
    pub max_virtual_ms: u64,
    /// The microstep budget of every macrostep.
    pub microstep_budget: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_steps: 10_000,
            max_virtual_ms: 60_000,
            microstep_budget: 10_000,
        }
    }
}

/// Loads charts for the host: the documents of a test and the children they invoke.
pub trait ChartLoader {
    /// Read and compile an SCXML document given as text (inline `<content>`).
    fn load_text(&self, xml: &str) -> Result<CompiledChart, String>;
    /// Read and compile the document behind a `src` (`file:test216sub1.scxml`).
    fn load_src(&self, src: &str) -> Result<CompiledChart, String>;
}

/// Loads documents from one directory: `src` values are file names relative to it, with or
/// without a `file:` prefix. Other references are refused.
#[derive(Clone, Debug)]
pub struct DirLoader {
    dir: PathBuf,
}

impl DirLoader {
    /// A loader for `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        DirLoader { dir: dir.into() }
    }

    fn path_of(&self, src: &str) -> Result<PathBuf, String> {
        let rel = src.strip_prefix("file:").unwrap_or(src);
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        if rel.is_empty()
            || rel.contains("..")
            || rel.contains(':')
            || rel.contains('\\')
            || Path::new(rel).is_absolute()
        {
            return Err(format!("`{src}` is not a file next to the test"));
        }
        Ok(self.dir.join(rel))
    }

    fn read_file(&self, src: &str) -> Result<String, String> {
        let path = self.path_of(src)?;
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read and compile the SCXML text of a document.
    pub fn compile(&self, xml: &str) -> Result<CompiledChart, String> {
        let resolver = |r: &SourceRequest<'_>| -> Result<Resolution, ResolveError> {
            self.read_file(r.src)
                .map(Resolution::Inline)
                .map_err(ResolveError::new)
        };
        let options = ReadOptions::new()
            .with_datamodels(&HOST_DATAMODELS)
            .with_resolver(&resolver);
        let parsed = vertep_scxml::read(xml, &options).map_err(|e| e.to_string())?;
        CompiledChart::new(parsed.chart).map_err(|v| {
            let msgs: Vec<String> = v.iter().map(|x| format!("{x:?}")).collect();
            format!("invalid chart: {}", msgs.join("; "))
        })
    }

    /// Read and compile the document at `path`.
    pub fn compile_file(&self, path: &Path) -> Result<CompiledChart, String> {
        let xml = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.compile(&xml)
    }
}

/// Inline `<content>` is kept relative to a context whose default namespace is SCXML (the
/// reader's convention), so a root element without its own default namespace gets the SCXML
/// one before the document is read on its own.
pub fn in_scxml_context(xml: &str) -> String {
    let Some(start) = xml
        .find('<')
        .filter(|_| !xml.trim_start().starts_with("<?"))
    else {
        return xml.to_string();
    };
    let tag_end = xml[start..].find('>').map_or(xml.len(), |e| start + e);
    let tag = &xml[start..tag_end];
    if tag.contains("xmlns=") {
        return xml.to_string();
    }
    let name_end = tag
        .find(|c: char| c.is_whitespace() || c == '/')
        .map_or(tag_end, |e| start + e);
    format!(
        "{} xmlns=\"{}\"{}",
        &xml[..name_end],
        vertep_scxml::SCXML_NS,
        &xml[name_end..]
    )
}

impl ChartLoader for DirLoader {
    fn load_text(&self, xml: &str) -> Result<CompiledChart, String> {
        self.compile(&in_scxml_context(xml))
    }

    fn load_src(&self, src: &str) -> Result<CompiledChart, String> {
        let xml = self.read_file(src)?;
        self.compile(&xml)
    }
}

/// Why a host run stopped before the root session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The root session ended.
    Finished,
    /// The step budget ran out.
    StepBudget(u64),
    /// The next delayed event is due after the virtual-time budget.
    TimeBudget(u64),
    /// A macrostep did not stabilize within the microstep budget.
    MicrostepBudget {
        /// The session.
        session: String,
    },
    /// Nothing is ready and no delayed event is pending, but the root still runs.
    Stalled,
    /// The root session could not start.
    StartFailed(String),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Finished => f.write_str("finished"),
            Stop::StepBudget(n) => write!(f, "step budget of {n} inputs exceeded"),
            Stop::TimeBudget(ms) => write!(f, "virtual-time budget of {ms} ms exceeded"),
            Stop::MicrostepBudget { session } => {
                write!(f, "microstep budget exceeded in session {session}")
            }
            Stop::Stalled => f.write_str("stalled: no event pending and none scheduled"),
            Stop::StartFailed(m) => write!(f, "root session did not start: {m}"),
        }
    }
}

/// Something the host did, for diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub enum HostLog {
    /// A `<log>` effect.
    Log {
        /// The session.
        session: String,
        /// The label.
        label: Option<String>,
        /// The value.
        value: Option<Value>,
    },
    /// An event delivered to a session.
    Delivered {
        /// The session.
        session: String,
        /// Virtual time.
        at_ms: u64,
        /// The event name.
        event: String,
    },
    /// A send that could not be delivered.
    Undeliverable {
        /// The sending session.
        session: String,
        /// The target.
        target: String,
    },
    /// A child session that could not be started.
    InvokeFailed {
        /// The parent session.
        session: String,
        /// The invoke id.
        invokeid: String,
        /// Why.
        reason: String,
    },
}

struct Slot {
    sessionid: String,
    chart: Rc<CompiledChart>,
    /// `None` while the session is being stepped, or when it could not start.
    session: Option<Session>,
    dm: Box<dyn Datamodel>,
    parent: Option<(SessionIdx, String)>,
    children: BTreeMap<String, SessionIdx>,
    ended: bool,
    /// Cancelled by its parent: nothing it sends from now on is delivered.
    cancelled: bool,
}

struct Timer {
    due: u64,
    seq: u64,
    from: SessionIdx,
    request: SendRequest,
}

struct Delivery {
    to: SessionIdx,
    kind: InputKind,
}

/// The session done record of the root.
#[derive(Clone, Debug, PartialEq)]
pub struct RootEnd {
    /// The done data.
    pub data: Value,
    /// Whether it was cancelled.
    pub cancelled: bool,
}

/// The in-memory multi-session host ([[openspec:scxml-conformance#Test Host]]).
// @tg: implements:: [[openspec:scxml-conformance#Test Host]]
pub struct Host<'a> {
    registry: &'a dyn DatamodelRegistry,
    loader: &'a dyn ChartLoader,
    budget: Budget,
    record_tape: bool,
    sessions: Vec<Slot>,
    ready: VecDeque<Delivery>,
    timers: Vec<Timer>,
    seq: u64,
    now_ms: u64,
    steps: u64,
    microstep_overrun: Option<String>,
    root_records: Vec<OracleRecord>,
    root_end: Option<RootEnd>,
    log: Vec<HostLog>,
}

/// The fixed epoch the virtual clock starts at (2026-01-01T00:00:00Z), in Unix milliseconds.
pub const EPOCH_MS: i64 = 1_767_225_600_000;

impl<'a> Host<'a> {
    /// A host with no sessions.
    pub fn new(
        registry: &'a dyn DatamodelRegistry,
        loader: &'a dyn ChartLoader,
        budget: Budget,
    ) -> Self {
        Host {
            registry,
            loader,
            budget,
            record_tape: false,
            sessions: Vec::new(),
            ready: VecDeque::new(),
            timers: Vec::new(),
            seq: 0,
            now_ms: 0,
            steps: 0,
            microstep_overrun: None,
            root_records: Vec::new(),
            root_end: None,
            log: Vec::new(),
        }
    }

    /// Record the root session's datamodel answer tape alongside its trace (for the Lean
    /// oracle, decision D5).
    pub fn record_tape(mut self, on: bool) -> Self {
        self.record_tape = on;
        self
    }

    /// The current virtual time in milliseconds since the run started.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Inputs processed so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The root session's macrostep records (input, tape when recorded, trace) in order.
    pub fn root_records(&self) -> &[OracleRecord] {
        &self.root_records
    }

    /// Take the root session's records.
    pub fn take_root_records(&mut self) -> Vec<OracleRecord> {
        std::mem::take(&mut self.root_records)
    }

    /// How the root session ended, once it has.
    pub fn root_end(&self) -> Option<&RootEnd> {
        self.root_end.as_ref()
    }

    /// The root's compiled chart, once started.
    pub fn root_chart(&self) -> Option<&CompiledChart> {
        self.sessions.first().map(|s| s.chart.as_ref())
    }

    /// What the host did, in order.
    pub fn log(&self) -> &[HostLog] {
        &self.log
    }

    /// The `_sessionid` of a session.
    pub fn sessionid(&self, idx: SessionIdx) -> Option<&str> {
        self.sessions.get(idx).map(|s| s.sessionid.as_str())
    }

    /// The number of sessions started so far, the root included.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Whether a session has ended.
    pub fn has_ended(&self, idx: SessionIdx) -> bool {
        self.sessions.get(idx).is_none_or(|s| s.ended)
    }

    /// The session value of a running session.
    pub fn session(&self, idx: SessionIdx) -> Option<&Session> {
        self.sessions.get(idx).and_then(|s| s.session.as_ref())
    }

    /// Queue an external event for a session, as if an outside sender delivered it.
    pub fn enqueue(&mut self, to: SessionIdx, event: Event) {
        self.ready.push_back(Delivery {
            to,
            kind: InputKind::Event(event),
        });
    }

    fn env(&self, idx: SessionIdx) -> Environment {
        let now = i64::try_from(self.now_ms).unwrap_or(i64::MAX - EPOCH_MS);
        Environment {
            now_ms: EPOCH_MS + now,
            seed: ((idx as u64) << 32) ^ self.steps,
        }
    }

    fn ioprocessors(sessionid: &str) -> Value {
        let entry = Value::Object(vec![(
            "location".to_string(),
            Value::String(format!("#_scxml_{sessionid}")),
        )]);
        Value::Object(vec![
            (SCXML_EVENT_PROCESSOR.to_string(), entry.clone()),
            ("scxml".to_string(), entry),
        ])
    }

    fn options(&self) -> Options {
        Options {
            microstep_budget: self.budget.microstep_budget,
            ..Options::default()
        }
    }

    /// Start the root session of `chart`. Returns its index (always 0).
    pub fn start_root(&mut self, chart: CompiledChart) -> Result<SessionIdx, Stop> {
        self.spawn(Rc::new(chart), None, Value::Undefined)
            .map_err(Stop::StartFailed)
    }

    /// Create and start a session; its start effects are performed before returning.
    fn spawn(
        &mut self,
        chart: Rc<CompiledChart>,
        parent: Option<(SessionIdx, String)>,
        params: Value,
    ) -> Result<SessionIdx, String> {
        let idx = self.sessions.len();
        let sessionid = format!("vt{idx}");
        let dm =
            create_datamodel(self.registry, chart.datamodel()).map_err(|e| format!("{e:?}"))?;
        let input = StartInput {
            info: SessionInfo {
                sessionid: sessionid.clone(),
                name: chart.chart().name.clone(),
                ioprocessors: Host::ioprocessors(&sessionid),
            },
            env: self.env(idx),
            options: self.options(),
            data: match params {
                Value::Object(pairs) => pairs,
                _ => Vec::new(),
            },
            sessions: None,
        };
        self.sessions.push(Slot {
            sessionid,
            chart: Rc::clone(&chart),
            session: None,
            dm,
            parent,
            children: BTreeMap::new(),
            ended: false,
            cancelled: false,
        });
        let record = idx == 0;
        let mut input = input;
        input.sessions = Some(self.live_sessions());
        let slot = &mut self.sessions[idx];
        let (result, tape) = if record && self.record_tape {
            let mut tape = TapeDatamodel::new(slot.dm.as_mut());
            let r = session::start(&chart, input.clone(), &mut tape);
            (r, tape.take())
        } else {
            (
                session::start(&chart, input.clone(), slot.dm.as_mut()),
                Vec::new(),
            )
        };
        let outcome = match result {
            Ok(o) => o,
            Err(StartError::BudgetExceeded { partial, .. }) => {
                self.microstep_overrun = Some(self.sessions[idx].sessionid.clone());
                self.sessions[idx].ended = true;
                if record {
                    self.root_records.push(OracleRecord {
                        input: OracleInput::Start(input),
                        tape,
                        trace: partial.trace,
                    });
                }
                return Ok(idx);
            }
            Err(e) => {
                self.sessions[idx].ended = true;
                return Err(format!("{e:?}"));
            }
        };
        self.after_step(idx, OracleInput::Start(input), tape, outcome);
        Ok(idx)
    }

    fn after_step(
        &mut self,
        idx: SessionIdx,
        input: OracleInput,
        tape: Vec<TapeEntry>,
        outcome: Outcome,
    ) {
        let Outcome {
            session,
            effects,
            trace,
        } = outcome;
        self.sessions[idx].session = Some(session);
        if idx == 0 {
            self.root_records.push(OracleRecord { input, tape, trace });
        }
        for e in effects {
            self.perform(idx, e);
        }
    }

    fn step_session(&mut self, idx: SessionIdx, kind: InputKind) {
        let Some(slot) = self.sessions.get(idx) else {
            return;
        };
        if slot.ended || slot.session.is_none() {
            return;
        }
        if let InputKind::Event(e) = &kind {
            self.log.push(HostLog::Delivered {
                session: slot.sessionid.clone(),
                at_ms: self.now_ms,
                event: e.name.clone(),
            });
        }
        let env = self.env(idx);
        self.steps += 1;
        let input = Input {
            env,
            kind,
            sessions: Some(self.live_sessions()),
        };
        let record = idx == 0;
        let slot = &mut self.sessions[idx];
        let chart = Rc::clone(&slot.chart);
        let Some(session) = slot.session.take() else {
            return;
        };
        let (result, tape) = if record && self.record_tape {
            let mut tape = TapeDatamodel::new(slot.dm.as_mut());
            let r = session::step(&chart, session, input.clone(), &mut tape);
            (r, tape.take())
        } else {
            (
                session::step(&chart, session, input.clone(), slot.dm.as_mut()),
                Vec::new(),
            )
        };
        match result {
            Ok(outcome) => self.after_step(idx, OracleInput::Step(input), tape, outcome),
            Err(StepError::BudgetExceeded { partial, .. }) => {
                self.microstep_overrun = Some(self.sessions[idx].sessionid.clone());
                self.sessions[idx].ended = true;
                if record {
                    self.root_records.push(OracleRecord {
                        input: OracleInput::Step(input),
                        tape,
                        trace: partial.trace,
                    });
                }
            }
            Err(StepError::Terminated) => {
                self.sessions[idx].ended = true;
            }
        }
    }

    /// The `_sessionid`s of the sessions that have not ended: what `#_scxml_<id>` can reach.
    fn live_sessions(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|s| !s.ended)
            .map(|s| s.sessionid.clone())
            .collect()
    }

    fn alive(&self, idx: SessionIdx) -> bool {
        self.sessions.get(idx).is_some_and(|s| !s.ended)
    }

    fn undeliverable(&mut self, from: SessionIdx, request: &SendRequest) {
        self.log.push(HostLog::Undeliverable {
            session: self.sessions[from].sessionid.clone(),
            target: request.target.clone().unwrap_or_default(),
        });
        self.ready.push_back(Delivery {
            to: from,
            kind: InputKind::Event(Event {
                name: "error.communication".to_string(),
                type_: EventType::Platform,
                sendid: Some(request.sendid.clone()),
                ..Event::default()
            }),
        });
    }

    /// Route a send whose delay has passed through the SCXML Event I/O Processor.
    fn route(&mut self, from: SessionIdx, request: SendRequest) {
        if !self.alive(from) || self.sessions[from].cancelled {
            return;
        }
        let scxml = request.type_ == SCXML_EVENT_PROCESSOR || request.type_ == "scxml";
        if !scxml {
            self.undeliverable(from, &request);
            return;
        }
        let mut event = request.event.clone();
        let to = match request.target.as_deref() {
            None => Some(from),
            Some("#_internal") => {
                event.type_ = EventType::Internal;
                Some(from)
            }
            Some("#_parent") => match self.sessions[from].parent.clone() {
                Some((p, invokeid)) if self.alive(p) => {
                    event.invokeid = Some(invokeid);
                    Some(p)
                }
                _ => None,
            },
            Some(t) => {
                if let Some(sid) = t.strip_prefix("#_scxml_") {
                    self.sessions
                        .iter()
                        .position(|s| s.sessionid == sid && !s.ended)
                } else if let Some(inv) = t.strip_prefix("#_") {
                    self.sessions[from]
                        .children
                        .get(inv)
                        .copied()
                        .filter(|c| self.alive(*c))
                } else {
                    None
                }
            }
        };
        match to {
            Some(to) => self.ready.push_back(Delivery {
                to,
                kind: InputKind::Event(event),
            }),
            None => self.undeliverable(from, &request),
        }
    }

    fn perform(&mut self, idx: SessionIdx, effect: Effect) {
        match effect {
            Effect::Send(request) => {
                if request.delay_ms > 0 {
                    self.seq += 1;
                    self.timers.push(Timer {
                        due: self.now_ms.saturating_add(request.delay_ms),
                        seq: self.seq,
                        from: idx,
                        request,
                    });
                } else {
                    self.route(idx, request);
                }
            }
            Effect::Cancel { sendid } => {
                self.timers
                    .retain(|t| !(t.from == idx && t.request.sendid == sendid));
            }
            Effect::Invoke(request) => {
                let chart = match (&request.content, &request.src) {
                    (Some(Value::String(xml)), _) => self.loader.load_text(xml),
                    (Some(other), _) => Err(format!("invoke content is not SCXML: {other:?}")),
                    (None, Some(src)) => self.loader.load_src(src),
                    (None, None) => Err("invoke has neither content nor src".to_string()),
                };
                let started = chart.and_then(|c| {
                    self.spawn(
                        Rc::new(c),
                        Some((idx, request.invokeid.clone())),
                        request.params.clone(),
                    )
                });
                match started {
                    Ok(child) => {
                        self.sessions[idx]
                            .children
                            .insert(request.invokeid.clone(), child);
                    }
                    Err(reason) => {
                        self.log.push(HostLog::InvokeFailed {
                            session: self.sessions[idx].sessionid.clone(),
                            invokeid: request.invokeid.clone(),
                            reason,
                        });
                        self.ready.push_back(Delivery {
                            to: idx,
                            kind: InputKind::Event(Event {
                                name: "error.execution".to_string(),
                                type_: EventType::Platform,
                                ..Event::default()
                            }),
                        });
                    }
                }
            }
            Effect::CancelInvoke { invokeid } => {
                if let Some(child) = self.sessions[idx].children.get(&invokeid).copied()
                    && self.alive(child)
                {
                    // Once cancelled, nothing the child sends reaches anyone (W3C SCXML 6.4),
                    // including what its exit handlers send during the cancellation.
                    self.sessions[child].cancelled = true;
                    self.timers.retain(|t| t.from != child);
                    self.step_session(child, InputKind::Cancel);
                }
            }
            Effect::Forward {
                invokeid,
                mut event,
            } => {
                if let Some(child) = self.sessions[idx].children.get(&invokeid).copied()
                    && self.alive(child)
                {
                    // `invokeid` names an invocation of the parent; in the child the event does
                    // not come from one of its own invocations, so the field is blank there
                    // (W3C SCXML 5.10.1).
                    event.invokeid = None;
                    self.ready.push_back(Delivery {
                        to: child,
                        kind: InputKind::Event(event),
                    });
                }
            }
            Effect::Log { label, value } => {
                self.log.push(HostLog::Log {
                    session: self.sessions[idx].sessionid.clone(),
                    label,
                    value,
                });
            }
            Effect::SessionDone { data, cancelled } => {
                self.sessions[idx].ended = true;
                self.timers.retain(|t| t.from != idx);
                if idx == 0 {
                    self.root_end = Some(RootEnd {
                        data: data.clone(),
                        cancelled,
                    });
                }
                if !cancelled
                    && let Some((p, invokeid)) = self.sessions[idx].parent.clone()
                    && self.alive(p)
                {
                    self.ready.push_back(Delivery {
                        to: p,
                        kind: InputKind::Event(Event {
                            name: format!("done.invoke.{invokeid}"),
                            type_: EventType::External,
                            invokeid: Some(invokeid),
                            data,
                            ..Event::default()
                        }),
                    });
                }
            }
        }
    }

    /// Run until the root session ends or a budget runs out.
    pub fn run(&mut self) -> Stop {
        loop {
            if self.root_end.is_some() {
                return Stop::Finished;
            }
            if let Some(session) = self.microstep_overrun.take() {
                return Stop::MicrostepBudget { session };
            }
            if self.sessions.first().is_none_or(|s| s.ended) {
                return Stop::Stalled;
            }
            if let Some(d) = self.ready.pop_front() {
                if !self.alive(d.to) {
                    continue;
                }
                if self.steps >= self.budget.max_steps {
                    return Stop::StepBudget(self.budget.max_steps);
                }
                self.step_session(d.to, d.kind);
                continue;
            }
            let Some(next) = self
                .timers
                .iter()
                .enumerate()
                .min_by_key(|(_, t)| (t.due, t.seq))
                .map(|(i, _)| i)
            else {
                return Stop::Stalled;
            };
            if self.timers[next].due > self.budget.max_virtual_ms {
                return Stop::TimeBudget(self.budget.max_virtual_ms);
            }
            let timer = self.timers.swap_remove(next);
            self.now_ms = self.now_ms.max(timer.due);
            self.route(timer.from, timer.request);
        }
    }
}
