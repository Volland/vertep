//! The deterministic part of a virtual actor: one macrostep from a journaled input to the
//! effects its entry records, used identically by live turns and by replay.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use oxrdf::{Literal, NamedNode, Triple};
use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, Event, EventType, SessionInfo, Value,
};
use vertep_core::session::{
    self, Effect, Input, InvokeRequest, Options, SCXML_EVENT_PROCESSOR, Session, StartInput,
    StepError,
};
use vertep_core::trace::MacrostepTrace;

use crate::address::{self, Address};
use crate::chart::{ChartInfo, UnhandledPolicy, split_type};
use crate::journal::{
    Effects, InputRecord, ReminderSpec, SendRecord, SnapshotData, TimerRecord, digest,
    timer_triples,
};
use crate::json;
use crate::vocab::{self as vt, node};

/// An invocation started by an actor, with the events sent to it since, so that a chart child
/// can be rebuilt from its parent's journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvocationLog {
    /// The invocation.
    pub request: InvokeRequest,
    /// Events sent or forwarded to it, with the timestamp of the sending entry.
    pub events: Vec<(Event, i64)>,
}

/// The runtime's per-actor state besides the session and datamodel: pending timers and
/// reminders, open requests and running invocations. It is part of snapshots and rebuilt by
/// replay.
// @tg: implements:: [[openspec:durable-timers#Timer Cancellation]]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeState {
    /// Pending timers and reminders by IRI.
    pub timers: BTreeMap<String, TimerRecord>,
    /// Open requests: correlation id to the IRI of their timeout timer.
    pub requests: BTreeMap<String, Option<String>>,
    /// Correlation ids already answered.
    pub answered: BTreeSet<String>,
    /// Correlation ids whose timeout fired.
    pub timed_out: BTreeSet<String>,
    /// IRIs of cancelled timers.
    pub cancelled: BTreeSet<String>,
    /// Running invocations by invoke id.
    pub invocations: BTreeMap<String, InvocationLog>,
}

/// Why a macrostep could not be run.
#[derive(Clone, Debug, PartialEq)]
pub enum MachineError {
    /// The session has terminated.
    Terminated,
    /// The interpreter refused the input (budget exceeded, datamodel failure).
    Step(String),
    /// A snapshot could not be restored.
    Snapshot(String),
}

impl core::fmt::Display for MachineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MachineError::Terminated => f.write_str("the session has terminated"),
            MachineError::Step(e) => write!(f, "step failed: {e}"),
            MachineError::Snapshot(e) => write!(f, "snapshot cannot be restored: {e}"),
        }
    }
}

/// What a machine needs from its node.
pub(crate) struct Ctx<'a> {
    pub registry: &'a dyn DatamodelRegistry,
    pub ioprocessors: Value,
    pub send_types: Vec<String>,
    pub invoke_types: Vec<String>,
}

/// The result of applying one input.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    pub effects: Effects,
    pub invokes: Vec<InvokeRequest>,
    pub cancel_invokes: Vec<String>,
    pub migrations: Vec<(String, String)>,
}

/// An actor's session, datamodel and runtime state under one chart version.
pub(crate) struct Machine {
    pub address: Address,
    pub info: Arc<ChartInfo>,
    pub session: Session,
    pub dm: Box<dyn Datamodel>,
    pub rt: RuntimeState,
}

impl Machine {
    fn options(info: &ChartInfo, ctx: &Ctx<'_>) -> Options {
        let mut o = Options::default();
        for t in ctx.send_types.iter().chain(info.vt_send_types.iter()) {
            if !o.send_types.contains(t) {
                o.send_types.push(t.clone());
            }
        }
        for t in &ctx.invoke_types {
            if !o.invoke_types.contains(t) {
                o.invoke_types.push(t.clone());
            }
        }
        o
    }

    fn new_dm(info: &ChartInfo, ctx: &Ctx<'_>) -> Result<Box<dyn Datamodel>, MachineError> {
        vertep_core::null::create_datamodel_for(ctx.registry, info.compiled.chart())
            .map_err(|e| MachineError::Step(format!("{e:?}")))
    }

    fn info(address: &Address, info: &ChartInfo, ctx: &Ctx<'_>) -> SessionInfo {
        SessionInfo {
            sessionid: address.as_str().to_owned(),
            name: info.ir.name.clone(),
            ioprocessors: ctx.ioprocessors.clone(),
        }
    }

    /// Starts the session (the start macrostep of entry 1), returning its effects.
    pub fn start(
        address: &Address,
        info: Arc<ChartInfo>,
        ctx: &Ctx<'_>,
        env: Environment,
    ) -> Result<(Machine, Vec<Effect>, MacrostepTrace), MachineError> {
        let mut dm = Self::new_dm(&info, ctx)?;
        let start = StartInput {
            info: Self::info(address, &info, ctx),
            env,
            options: Self::options(&info, ctx),
            data: Vec::new(),
            sessions: None,
        };
        let o = session::start(&info.compiled, start, dm.as_mut())
            .map_err(|e| MachineError::Step(format!("{e:?}")))?;
        Ok((
            Machine {
                address: address.clone(),
                info,
                session: o.session,
                dm,
                rt: RuntimeState::default(),
            },
            o.effects,
            o.trace,
        ))
    }

    /// Restores a machine from a snapshot taken under `info`.
    pub fn from_snapshot(
        address: &Address,
        info: Arc<ChartInfo>,
        ctx: &Ctx<'_>,
        s: &SnapshotData,
    ) -> Result<Machine, MachineError> {
        let mut dm = Self::new_dm(&info, ctx)?;
        dm.init_session(&Self::info(address, &info, ctx))
            .map_err(|e| MachineError::Snapshot(e.message))?;
        let value =
            json::from_text(&s.datamodel).map_err(|e| MachineError::Snapshot(e.to_string()))?;
        dm.restore(&value)
            .map_err(|e| MachineError::Snapshot(e.message))?;
        let session: Session =
            serde_json::from_str(&s.session).map_err(|e| MachineError::Snapshot(e.to_string()))?;
        let rt: RuntimeState =
            serde_json::from_str(&s.runtime).map_err(|e| MachineError::Snapshot(e.to_string()))?;
        Ok(Machine {
            address: address.clone(),
            info,
            session,
            dm,
            rt,
        })
    }

    /// The active state ids.
    pub fn active(&self) -> Vec<String> {
        self.info.active(&self.session)
    }

    /// The configuration digest.
    pub fn digest(&self) -> String {
        digest(
            &self.active(),
            &self.dm.snapshot(),
            self.session.is_terminated(),
        )
    }

    /// Whether `event` would be accepted now: an active state defers it (`vt:defer`), or it
    /// enables at least one transition of the current configuration. Guards run on a scratch
    /// copy of the datamodel made from `registry`, so the machine is unchanged.
    pub fn can(&self, registry: &dyn DatamodelRegistry, event: &Event) -> bool {
        if self.session.is_terminated() {
            return false;
        }
        if self
            .info
            .deferred(&self.session)
            .iter()
            .any(|d| vertep_core::kernel::descriptor_matches(d, &event.name))
        {
            return true;
        }
        let Ok(mut dm) =
            vertep_core::null::create_datamodel_for(registry, self.info.compiled.chart())
        else {
            return false;
        };
        let info = SessionInfo {
            sessionid: self.address.as_str().to_owned(),
            name: self.info.ir.name.clone(),
            ioprocessors: Value::Undefined,
        };
        if dm.init_session(&info).is_err()
            || dm.restore(&self.dm.snapshot()).is_err()
            || dm.check_event(event).is_err()
        {
            return false;
        }
        dm.set_event(event);
        let chart = &self.info.compiled;
        let q = ActiveIn {
            chart,
            session: &self.session,
        };
        !vertep_core::kernel::select_transitions(
            chart,
            &self.session.configuration,
            &self.session.history,
            Some(&event.name),
            |t| {
                chart
                    .transition(t)
                    .cond
                    .as_deref()
                    .is_none_or(|c| dm.eval_cond(c, &q).unwrap_or(false))
            },
        )
        .is_empty()
    }

    /// A snapshot with IRI `iri` at `position`.
    pub fn snapshot(&self, iri: &NamedNode, position: u64) -> SnapshotData {
        SnapshotData {
            iri: iri.as_str().to_owned(),
            actor: self.address.as_str().to_owned(),
            position,
            version: self.info.version.as_str().to_owned(),
            digest: self.digest(),
            session: serde_json::to_string(&self.session).unwrap_or_default(),
            datamodel: json::to_text(&self.dm.snapshot()),
            runtime: serde_json::to_string(&self.rt).unwrap_or_default(),
        }
    }

    /// The statements of the actor's current-state graph.
    // @tg: implements:: [[openspec:runtime-ontology#Named Graph Layout]]
    pub fn state_triples(&self, chart_type: Option<(&str, &str)>) -> HashSet<Triple> {
        let a = self.address.node().clone();
        let mut out = HashSet::new();
        let mut add = |p: &str, o: oxrdf::Term| {
            out.insert(Triple::new(a.clone(), node(p), o));
        };
        add(vt::RDF_TYPE, node(vt::ACTOR_CLASS).into());
        add(vt::RDF_TYPE, node(vt::VIRTUAL_ACTOR).into());
        add(vt::CHART_VERSION, self.info.version.named_node().into());
        if let Some((t, k)) = chart_type
            && let Ok(t) = NamedNode::new(t)
        {
            add(vt::CHART_TYPE, t.into());
            add(vt::KEY, Literal::new_simple_literal(k).into());
        }
        add(
            vt::STATUS,
            node(if self.session.is_terminated() {
                vt::TERMINATED
            } else {
                vt::RUNNING
            })
            .into(),
        );
        for id in self.active() {
            add(vt::IN_STATE, state_node(&self.info, &id).into());
        }
        add(
            vt::DATAMODEL_STATE,
            Literal::new_simple_literal(json::to_text(&self.dm.snapshot())).into(),
        );
        for t in self.rt.timers.values() {
            out.extend(timer_triples(t));
        }
        out
    }

    /// Applies one input: the start macrostep (for entry 1) and the macrostep of `input`,
    /// returning everything the entry at `position` records.
    pub fn apply(
        &mut self,
        input: &InputRecord,
        position: u64,
        ts: i64,
        policy: UnhandledPolicy,
        start_effects: Vec<Effect>,
        start_trace: Option<&MacrostepTrace>,
    ) -> Result<Applied, MachineError> {
        let entry = self.address.entry_iri(position);
        let env = Environment {
            now_ms: ts,
            seed: address::seed_of(&entry),
        };
        let mut out = Applied::default();
        let ev = &input.event;
        // runtime bookkeeping of the input
        if let Some(t) = &input.timer
            && self.rt.timers.get(t).is_some_and(|r| r.reminder.is_none())
        {
            self.rt.timers.remove(t);
        }
        if let Some(c) = &input.correlation {
            if ev.name == vt::REPLY_EVENT
                && let Some(timeout) = self.rt.requests.remove(c)
            {
                self.rt.answered.insert(c.clone());
                out.effects.answers = Some(c.clone());
                if let Some(t) = timeout
                    && self.rt.timers.remove(&t).is_some()
                {
                    self.rt.cancelled.insert(t.clone());
                    out.effects.cancels.push(t);
                }
            } else if ev.name == vt::TIMEOUT_EVENT && self.rt.requests.remove(c).is_some() {
                self.rt.timed_out.insert(c.clone());
            }
        }
        if let Some(id) = ev.name.strip_prefix("done.invoke.") {
            self.rt.invocations.remove(id);
        }
        let mut traces: Vec<MacrostepTrace> = start_trace.into_iter().cloned().collect();
        let step = |m: &mut Machine, e: Event| -> Result<session::Outcome, MachineError> {
            session::step(
                &m.info.compiled,
                m.session.clone(),
                Input::event(e, env),
                m.dm.as_mut(),
            )
            .map_err(|e| match e {
                StepError::Terminated => MachineError::Terminated,
                other => MachineError::Step(format!("{other:?}")),
            })
        };
        // A payload that fails its declared type is not delivered: the chart gets
        // `error.execution` instead ([[openspec:type-system#Opt-In Chart Typing]]).
        let admitted = vertep_core::datamodel::admit(self.dm.as_ref(), ev.clone());
        let o = step(self, admitted)?;
        let unhandled = o
            .trace
            .microsteps
            .iter()
            .find(|m| m.event.as_deref() == Some(ev.name.as_str()))
            .is_some_and(|m| m.transitions.is_empty());
        self.session = o.session;
        let mut raw = start_effects;
        raw.extend(o.effects);
        traces.push(o.trace);
        if unhandled {
            out.effects.unhandled = Some(policy);
            if policy == UnhandledPolicy::Error && !self.session.is_terminated() {
                let data = Value::Object(vec![
                    ("name".into(), Value::String(ev.name.clone())),
                    ("data".into(), ev.data.clone()),
                ]);
                let e = Event {
                    name: vt::UNHANDLED_EVENT.into(),
                    type_: EventType::Platform,
                    data,
                    ..Event::default()
                };
                let o2 = step(self, e)?;
                self.session = o2.session;
                raw.extend(o2.effects);
                traces.push(o2.trace);
            }
        }
        for t in &traces {
            for m in &t.microsteps {
                for tr in &m.transitions {
                    let frag = format!(
                        "{}:t{}",
                        vertep_rdf::project::state_fragment(&tr.source),
                        tr.index
                    );
                    let iri = vertep_rdf::iri::node_iri(self.info.version.as_str(), &frag);
                    if !out.effects.transitions.contains(&iri) {
                        out.effects.transitions.push(iri);
                    }
                }
            }
        }
        out.effects.transitions.sort();
        self.collect(raw, &entry, position, ts, input, &mut out);
        out.effects.terminated = self.session.is_terminated();
        if out.effects.terminated {
            let reminders: Vec<String> = self
                .rt
                .timers
                .iter()
                .filter(|(_, t)| t.reminder.is_some())
                .map(|(k, _)| k.clone())
                .collect();
            for r in reminders {
                self.rt.timers.remove(&r);
                self.rt.cancelled.insert(r.clone());
                out.effects.cancels.push(r);
            }
        }
        Ok(out)
    }

    fn recipient(&self, target: Option<&str>) -> Option<String> {
        match target {
            None | Some("#_internal") => Some(self.address.as_str().to_owned()),
            Some("#_parent") => None,
            Some(t) => {
                if let Some(sid) = t.strip_prefix("#_scxml_") {
                    Address::parse(sid).ok().map(|a| a.as_str().to_owned())
                } else if let Some(id) = t.strip_prefix("#_") {
                    Some(Address::invoked(&self.address, id).as_str().to_owned())
                } else {
                    Address::parse(t).ok().map(|a| a.as_str().to_owned())
                }
            }
        }
    }

    fn collect(
        &mut self,
        raw: Vec<Effect>,
        entry: &NamedNode,
        position: u64,
        ts: i64,
        input: &InputRecord,
        out: &mut Applied,
    ) {
        let mut requests = 0usize;
        for e in raw {
            match e {
                Effect::Send(req) => {
                    let (base, params) = split_type(&req.type_);
                    if base == vt::REMINDER_SEND_TYPE {
                        let name = params.get("name").cloned().unwrap_or_default();
                        let existing: Vec<String> = self
                            .rt
                            .timers
                            .iter()
                            .filter(|(_, t)| t.reminder.as_ref().is_some_and(|r| r.name == name))
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in existing {
                            self.rt.timers.remove(&k);
                            self.rt.cancelled.insert(k.clone());
                            out.effects.cancels.push(k);
                        }
                        if params.contains_key("cancel") {
                            continue;
                        }
                        let period: i64 = params
                            .get("period")
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(1)
                            .max(1);
                        let t = TimerRecord {
                            iri: address::timer_iri(entry, &format!("reminder:{name}"))
                                .as_str()
                                .to_owned(),
                            actor: self.address.as_str().to_owned(),
                            send_id: name.clone(),
                            due_ms: ts.saturating_add(period),
                            event: self.origin_event(req.event),
                            recipient: self.address.as_str().to_owned(),
                            reminder: Some(ReminderSpec {
                                name,
                                period_ms: period,
                            }),
                            correlation: None,
                        };
                        self.rt.timers.insert(t.iri.clone(), t.clone());
                        out.effects.timers.push(t);
                        continue;
                    }
                    if base == vt::MIGRATE_SEND_TYPE {
                        out.migrations.push((
                            params.get("to").cloned().unwrap_or_default(),
                            params.get("migration").cloned().unwrap_or_default(),
                        ));
                        continue;
                    }
                    let recipient = self.recipient(req.target.as_deref());
                    if let Some(id) = req.target.as_deref().and_then(|t| t.strip_prefix("#_"))
                        && let Some(log) = self.rt.invocations.get_mut(id)
                    {
                        log.events.push((req.event.clone(), ts));
                    }
                    let correlation = if base == vt::REQUEST_SEND_TYPE {
                        let c = self.address.correlation_id(position, requests);
                        requests += 1;
                        out.effects.requests.push(c.clone());
                        let timeout = params.get("timeout").and_then(|t| t.parse::<i64>().ok());
                        let timer = timeout.map(|t| {
                            let rec = TimerRecord {
                                iri: address::timer_iri(entry, &format!("timeout:{c}"))
                                    .as_str()
                                    .to_owned(),
                                actor: self.address.as_str().to_owned(),
                                send_id: format!("timeout:{c}"),
                                due_ms: ts.saturating_add(t),
                                event: self.origin_event(Event {
                                    name: vt::TIMEOUT_EVENT.into(),
                                    type_: EventType::Platform,
                                    data: Value::Object(vec![(
                                        "correlation".into(),
                                        Value::String(c.clone()),
                                    )]),
                                    ..Event::default()
                                }),
                                recipient: self.address.as_str().to_owned(),
                                reminder: None,
                                correlation: Some(c.clone()),
                            };
                            self.rt.timers.insert(rec.iri.clone(), rec.clone());
                            out.effects.timers.push(rec.clone());
                            rec.iri
                        });
                        self.rt.requests.insert(c.clone(), timer);
                        Some(c)
                    } else if req.event.name == vt::REPLY_EVENT {
                        input.correlation.clone()
                    } else {
                        None
                    };
                    let event = self.origin_event(req.event);
                    if req.delay_ms > 0 {
                        let t = TimerRecord {
                            iri: address::timer_iri(entry, &req.sendid).as_str().to_owned(),
                            actor: self.address.as_str().to_owned(),
                            send_id: req.sendid.clone(),
                            due_ms: ts
                                .saturating_add(i64::try_from(req.delay_ms).unwrap_or(i64::MAX)),
                            event,
                            recipient: recipient.unwrap_or_else(|| req.target.unwrap_or_default()),
                            reminder: None,
                            correlation,
                        };
                        self.rt.timers.insert(t.iri.clone(), t.clone());
                        out.effects.timers.push(t);
                    } else {
                        let k = out.effects.sends.len();
                        out.effects.sends.push(SendRecord {
                            iri: address::send_iri(entry, k).as_str().to_owned(),
                            send_id: req.sendid,
                            event,
                            target: req.target,
                            recipient,
                            type_: req.type_,
                            correlation,
                        });
                    }
                }
                Effect::Cancel { sendid } => {
                    let found: Vec<String> = self
                        .rt
                        .timers
                        .iter()
                        .filter(|(_, t)| t.send_id == sendid && t.reminder.is_none())
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in found {
                        self.rt.timers.remove(&k);
                        self.rt.cancelled.insert(k.clone());
                        out.effects.cancels.push(k);
                    }
                }
                Effect::Invoke(req) => {
                    out.effects
                        .invokes
                        .push(serde_json::to_string(&req).unwrap_or_default());
                    self.rt.invocations.insert(
                        req.invokeid.clone(),
                        InvocationLog {
                            request: req.clone(),
                            events: Vec::new(),
                        },
                    );
                    out.invokes.push(req);
                }
                Effect::CancelInvoke { invokeid } => {
                    self.rt.invocations.remove(&invokeid);
                    out.cancel_invokes.push(invokeid);
                }
                Effect::Forward { invokeid, event } => {
                    let k = out.effects.sends.len();
                    if let Some(log) = self.rt.invocations.get_mut(&invokeid) {
                        log.events.push((event.clone(), ts));
                    }
                    out.effects.sends.push(SendRecord {
                        iri: address::send_iri(entry, k).as_str().to_owned(),
                        send_id: format!("forward:{k}"),
                        event: self.origin_event(event),
                        target: Some(format!("#_{invokeid}")),
                        recipient: Some(
                            Address::invoked(&self.address, &invokeid)
                                .as_str()
                                .to_owned(),
                        ),
                        type_: SCXML_EVENT_PROCESSOR.to_owned(),
                        correlation: None,
                    });
                }
                Effect::Log { .. } | Effect::SessionDone { .. } => {}
            }
        }
    }

    // @tg: implements:: [[openspec:messaging#Event Origin]]
    fn origin_event(&self, mut e: Event) -> Event {
        e.origin = Some(self.address.as_str().to_owned());
        e.origintype = Some(SCXML_EVENT_PROCESSOR.to_owned());
        // A runtime-assigned origin inside the cluster is not a verified sender DID.
        e.originverified = false;
        e
    }
}

/// `In()` against a session's configuration, for guards evaluated by [`Machine::can`].
struct ActiveIn<'a> {
    chart: &'a vertep_core::compile::CompiledChart,
    session: &'a Session,
}

impl vertep_core::datamodel::StateQuery for ActiveIn<'_> {
    fn in_state(&self, id: &str) -> bool {
        self.session.is_active(self.chart, id)
    }

    fn active_states(&self) -> Option<Vec<String>> {
        Some(self.session.active(self.chart))
    }
}

/// The node IRI of state `id` in chart version `info`.
pub fn state_node(info: &ChartInfo, id: &str) -> NamedNode {
    NamedNode::new_unchecked(vertep_rdf::iri::node_iri(
        info.version.as_str(),
        &vertep_rdf::project::state_fragment(id),
    ))
}
