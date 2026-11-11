//! The agent host: the imperative shell that runs agent actors, consults deciders, calls
//! providers and tools, arms durable timers and journals every input before it is processed.
//!
//! The host is to [`AgentMachine`] what [`vertep_runtime::Node`] is to a chart: it owns time
//! (through a [`Clock`], so tests run on a virtual clock), background work (deciders, model and
//! tool calls are futures polled by [`AgentHost::pump`]) and the journal. Its scheduler is
//! synchronous and deterministic; [`AgentHost::settle`] also lets other tasks run.
//!
//! An actor's turn: take the oldest input from its mailbox; drop it when it is a duplicate
//! (message id or decision correlation id already processed); append it to the journal; apply
//! it; perform the effects. Deactivating an actor drops only its in-memory machine: its journal,
//! timers and running calls stay, and the next input or due timer reactivates it by replay,
//! which consults no decider, provider or tool.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::stream::{AbortHandle, abortable};
use futures::stream::{BoxStream, SelectAll, StreamExt};
use oxrdf::Graph;
use vertep_core::datamodel::{DatamodelRegistry, Event, Value};
use vertep_core::ir::Chart;
use vertep_rdf::ChartVersion;
use vertep_runtime::{Address, Clock, NodeHandle};

use crate::decider::{Annotations, Decider, DeciderRegistry, DecisionOutcome, DecisionRequest};
use crate::journal::{AgentInput, AgentJournal, JournalEntry, JournalError, MemoryJournal};
use crate::llm::{
    ErrorClass, Provider, ProviderRegistry, ProviderRequest, StreamChunk, StreamItem,
};
use crate::machine::{
    AgentChart, AgentEffect, AgentMachine, Applied, ChartError, LlmStart, MachineError, UsageTotals,
};
use crate::tool::{Tool, ToolErrorClass, ToolOutcome, ToolRegistry};

/// Why an actor could not be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstantiateError {
    /// A decide state names a decider binding that is not registered.
    UnknownDecider {
        /// The decide state.
        state: String,
        /// The missing binding.
        binding: String,
    },
    /// The chart version is not registered.
    UnknownVersion(String),
    /// The actor already has a journal.
    Exists(String),
    /// The start failed.
    Start(String),
}

impl core::fmt::Display for InstantiateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InstantiateError::UnknownDecider { state, binding } => write!(
                f,
                "decide state {state} names the decider binding {binding}, which is not registered"
            ),
            InstantiateError::UnknownVersion(v) => write!(f, "chart version {v} is not registered"),
            InstantiateError::Exists(a) => write!(f, "{a} already has a journal"),
            InstantiateError::Start(e) => write!(f, "start failed: {e}"),
        }
    }
}

impl std::error::Error for InstantiateError {}

/// Why an actor could not be activated or advanced.
#[derive(Clone, Debug, PartialEq)]
pub enum HostError {
    /// The actor has no journal.
    Unknown(String),
    /// A journaled chart version is not registered.
    MissingVersion(String),
    /// The machine failed.
    Machine(MachineError),
    /// The journal failed.
    Journal(JournalError),
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HostError::Unknown(a) => write!(f, "{a} has no journal"),
            HostError::MissingVersion(v) => write!(f, "chart version {v} is not registered"),
            HostError::Machine(e) => write!(f, "{e}"),
            HostError::Journal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for HostError {}

/// An actor's state at a journal position.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentState {
    /// The journal position the state is after.
    pub position: u64,
    /// Active state ids in document order.
    pub configuration: Vec<String>,
    /// The datamodel snapshot.
    pub datamodel: Value,
    /// Whether the session terminated.
    pub terminated: bool,
    /// The done data.
    pub done: Option<Value>,
}

/// An event the host sent to a target outside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    /// The sender.
    pub from: String,
    /// The target as written.
    pub target: String,
    /// The event.
    pub event: Event,
}

/// Counters for observing a host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostStats {
    /// Decider consultations per binding.
    pub decider_calls: BTreeMap<String, usize>,
    /// Provider calls per binding.
    pub provider_calls: BTreeMap<String, usize>,
    /// Tool executions per IRI.
    pub tool_calls: BTreeMap<String, usize>,
    /// Entries re-applied by replay.
    pub replayed: u64,
    /// Activations by replay.
    pub activations: u64,
    /// Partial events delivered.
    pub partials: u64,
}

type ToInput = Arc<
    dyn Fn(Result<crate::llm::ProviderResult, crate::llm::ProviderError>) -> AgentInput
        + Send
        + Sync,
>;

enum Completion {
    Input { actor: String, input: AgentInput },
    Partial { actor: String, event: Event },
}

/// Builds an [`AgentHost`].
pub struct AgentHostBuilder {
    clock: Arc<dyn Clock>,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    journal: Arc<dyn AgentJournal>,
    deciders: DeciderRegistry,
    providers: ProviderRegistry,
    tools: ToolRegistry,
    node: Option<NodeHandle>,
}

impl std::fmt::Debug for AgentHostBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHostBuilder").finish_non_exhaustive()
    }
}

impl AgentHostBuilder {
    /// The datamodels charts may name (default: `cel` and `null`).
    pub fn datamodels(mut self, r: Arc<dyn DatamodelRegistry + Send + Sync>) -> Self {
        self.registry = r;
        self
    }

    /// The journal (default: a fresh [`MemoryJournal`]).
    pub fn journal(mut self, j: Arc<dyn AgentJournal>) -> Self {
        self.journal = j;
        self
    }

    /// Binds `decider` to `binding`.
    pub fn decider(mut self, binding: &str, decider: Arc<dyn Decider>) -> Self {
        self.deciders.insert(binding, decider);
        self
    }

    /// Binds `provider` to `binding`.
    pub fn provider(mut self, binding: &str, provider: Arc<dyn Provider>) -> Self {
        self.providers.insert(binding, provider);
        self
    }

    /// Registers `tool`.
    pub fn tool(mut self, tool: Tool) -> Self {
        self.tools.insert(tool);
        self
    }

    /// Sends to targets outside the host through this node.
    pub fn node(mut self, handle: NodeHandle) -> Self {
        self.node = Some(handle);
        self
    }

    /// The host.
    pub fn build(self) -> AgentHost {
        AgentHost {
            clock: self.clock,
            registry: self.registry,
            journal: self.journal,
            deciders: self.deciders,
            providers: self.providers,
            tools: self.tools,
            node: self.node,
            charts: BTreeMap::new(),
            actors: BTreeMap::new(),
            heads: BTreeMap::new(),
            mailboxes: BTreeMap::new(),
            timers: BTreeMap::new(),
            jobs: SelectAll::new(),
            aborts: BTreeMap::new(),
            inflight: BTreeSet::new(),
            outbox: Vec::new(),
            errors: BTreeMap::new(),
            stats: HostStats::default(),
        }
    }
}

/// The agent host. See the module documentation.
pub struct AgentHost {
    clock: Arc<dyn Clock>,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    journal: Arc<dyn AgentJournal>,
    deciders: DeciderRegistry,
    providers: ProviderRegistry,
    tools: ToolRegistry,
    node: Option<NodeHandle>,
    charts: BTreeMap<String, Arc<AgentChart>>,
    actors: BTreeMap<String, AgentMachine>,
    heads: BTreeMap<String, u64>,
    mailboxes: BTreeMap<String, VecDeque<AgentInput>>,
    timers: BTreeMap<(i64, String), String>,
    jobs: SelectAll<BoxStream<'static, Completion>>,
    aborts: BTreeMap<(String, String), AbortHandle>,
    inflight: BTreeSet<(String, String)>,
    outbox: Vec<Sent>,
    errors: BTreeMap<String, HostError>,
    stats: HostStats,
}

impl std::fmt::Debug for AgentHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHost")
            .field("active", &self.actors.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl AgentHost {
    /// A builder for a host reading time from `clock`.
    pub fn builder(clock: Arc<dyn Clock>) -> AgentHostBuilder {
        AgentHostBuilder {
            clock,
            registry: Arc::new(vertep_core::null::WithNull(vertep_dm_cel::CelRegistry)),
            journal: Arc::new(MemoryJournal::new()),
            deciders: DeciderRegistry::default(),
            providers: ProviderRegistry::default(),
            tools: ToolRegistry::default(),
            node: None,
        }
    }

    /// The clock reading.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Counters.
    pub fn stats(&self) -> &HostStats {
        &self.stats
    }

    /// The journal.
    pub fn journal_store(&self) -> &Arc<dyn AgentJournal> {
        &self.journal
    }

    /// Registers `chart`, returning its version.
    ///
    /// # Errors
    ///
    /// [`ChartError`] when the chart cannot run as an agent.
    pub fn register(&mut self, chart: Chart) -> Result<ChartVersion, ChartError> {
        let c = AgentChart::new(chart)?;
        let v = c.version.clone();
        self.charts
            .entry(v.as_str().to_owned())
            .or_insert_with(|| Arc::new(c));
        Ok(v)
    }

    /// Starts actor `actor` under chart version `version`: checks that every decider binding
    /// the chart names is registered, journals the start and runs the start macrostep.
    ///
    /// # Errors
    ///
    /// [`InstantiateError`]; nothing is journaled and no actor is activated.
    // @tg: implements:: [[openspec:deciders#Decide State Declaration]]
    pub fn spawn(&mut self, actor: &str, version: &ChartVersion) -> Result<(), InstantiateError> {
        let chart = self
            .charts
            .get(version.as_str())
            .cloned()
            .ok_or_else(|| InstantiateError::UnknownVersion(version.as_str().to_owned()))?;
        for spec in chart.decides.values() {
            for b in std::iter::once(&spec.decider).chain(spec.fallback.iter()) {
                if !self.deciders.contains(b) {
                    return Err(InstantiateError::UnknownDecider {
                        state: spec.state.clone(),
                        binding: b.clone(),
                    });
                }
            }
        }
        if !self.journal.read(actor).is_empty() {
            return Err(InstantiateError::Exists(actor.to_owned()));
        }
        let ts = self.now_ms();
        let (m, applied) = AgentMachine::start(actor, chart, self.registry.clone(), ts)
            .map_err(|e| InstantiateError::Start(e.to_string()))?;
        self.journal
            .append(
                actor,
                JournalEntry {
                    position: 1,
                    ts,
                    input: AgentInput::Start {
                        version: version.as_str().to_owned(),
                    },
                },
            )
            .map_err(|e| InstantiateError::Start(e.to_string()))?;
        self.heads.insert(actor.to_owned(), 1);
        self.actors.insert(actor.to_owned(), m);
        self.dispatch(actor, applied.effects);
        Ok(())
    }

    /// Sends `event` to `actor`; returns the message id.
    pub fn send(&mut self, actor: &str, event: Event) -> String {
        // Unique across hosts in this process (a host replaying another host's journal must not
        // reuse its message ids, or deduplication would drop the new messages).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!(
            "urn:vertep:agent-message:{}-{}-{n}",
            self.now_ms(),
            std::process::id()
        );
        self.enqueue(
            actor,
            AgentInput::Event {
                event,
                message_id: id.clone(),
            },
        );
        id
    }

    /// Delivers an input as is (a decision or refusal from outside, a replayed message).
    pub fn deliver(&mut self, actor: &str, input: AgentInput) {
        self.enqueue(actor, input);
    }

    /// Delivers `outcome` for the offer `correlation` of `actor`, as a decider would.
    pub fn deliver_outcome(&mut self, actor: &str, correlation: &str, outcome: DecisionOutcome) {
        let input = outcome_input(correlation, outcome, Annotations::default());
        self.enqueue(actor, input);
    }

    fn enqueue(&mut self, actor: &str, input: AgentInput) {
        self.mailboxes
            .entry(actor.to_owned())
            .or_default()
            .push_back(input);
    }

    /// Whether `actor` has an in-memory activation.
    pub fn is_active(&self, actor: &str) -> bool {
        self.actors.contains_key(actor)
    }

    /// The last error of `actor`.
    pub fn last_error(&self, actor: &str) -> Option<&HostError> {
        self.errors.get(actor)
    }

    /// Drops the in-memory activation of `actor`; its journal, timers and running calls stay.
    pub fn deactivate(&mut self, actor: &str) {
        self.actors.remove(actor);
    }

    /// The machine of an active actor.
    pub fn machine(&self, actor: &str) -> Option<&AgentMachine> {
        self.actors.get(actor)
    }

    /// Activates `actor` by replaying its journal (a no-op when active).
    ///
    /// # Errors
    ///
    /// [`HostError`].
    pub fn activate(&mut self, actor: &str) -> Result<(), HostError> {
        if self.actors.contains_key(actor) {
            return Ok(());
        }
        let entries = self.journal.read(actor);
        let (m, effects) = self.replay(actor, &entries, None)?;
        self.stats.activations += 1;
        self.stats.replayed += entries.len() as u64;
        self.heads.insert(actor.to_owned(), entries.len() as u64);
        // Re-arm timers, re-ask outstanding offers and restart running calls.
        for t in m.timers() {
            self.timers
                .insert((t.due_ms, t.iri.clone()), actor.to_owned());
        }
        let mut redo = Vec::new();
        for req in m.open_offers() {
            if !self
                .inflight
                .contains(&(actor.to_owned(), req.correlation.clone()))
            {
                redo.push(AgentEffect::Offer(req));
            }
        }
        for s in m.running_llm() {
            if !self
                .inflight
                .contains(&(actor.to_owned(), s.invokeid.clone()))
            {
                redo.push(AgentEffect::StartLlm(s));
            }
        }
        for e in effects {
            if let AgentEffect::Deliver { message_id, .. } = &e
                && !m.seen(message_id)
                && !self.queued(actor, message_id)
            {
                redo.push(e);
            }
        }
        self.actors.insert(actor.to_owned(), m);
        self.dispatch(actor, redo);
        Ok(())
    }

    fn queued(&self, actor: &str, id: &str) -> bool {
        self.mailboxes.get(actor).is_some_and(|q| {
            q.iter()
                .any(|i| matches!(i, AgentInput::Event { message_id, .. } if message_id == id))
        })
    }

    /// Replays `entries` (up to `upto`, inclusive) into a fresh machine, returning it and the
    /// effects the replay produced (never performed).
    fn replay(
        &self,
        actor: &str,
        entries: &[JournalEntry],
        upto: Option<u64>,
    ) -> Result<(AgentMachine, Vec<AgentEffect>), HostError> {
        let Some(first) = entries.first() else {
            return Err(HostError::Unknown(actor.to_owned()));
        };
        let AgentInput::Start { version } = &first.input else {
            return Err(HostError::Unknown(actor.to_owned()));
        };
        let chart = self
            .charts
            .get(version)
            .cloned()
            .ok_or_else(|| HostError::MissingVersion(version.clone()))?;
        let (mut m, applied) = AgentMachine::start(actor, chart, self.registry.clone(), first.ts)
            .map_err(HostError::Machine)?;
        let mut effects = applied.effects;
        for e in &entries[1..] {
            if upto.is_some_and(|u| e.position > u) {
                break;
            }
            match m.apply(&e.input, e.position, e.ts) {
                Ok(a) => effects.extend(a.effects),
                Err(MachineError::Terminated) => {}
                Err(err) => return Err(HostError::Machine(err)),
            }
        }
        Ok((m, effects))
    }

    /// The journal of `actor`.
    pub fn journal(&self, actor: &str) -> Vec<JournalEntry> {
        self.journal.read(actor)
    }

    /// The current state of `actor` (activating it).
    ///
    /// # Errors
    ///
    /// [`HostError`].
    pub fn state(&mut self, actor: &str) -> Result<AgentState, HostError> {
        self.activate(actor)?;
        let position = self.heads.get(actor).copied().unwrap_or(0);
        let m = self
            .actors
            .get(actor)
            .ok_or_else(|| HostError::Unknown(actor.to_owned()))?;
        Ok(snapshot(m, position))
    }

    /// The state of `actor` after journal position `position`, by replay.
    ///
    /// # Errors
    ///
    /// [`HostError`].
    pub fn state_at(&self, actor: &str, position: u64) -> Result<AgentState, HostError> {
        let entries = self.journal.read(actor);
        let (m, _) = self.replay(actor, &entries, Some(position))?;
        Ok(snapshot(&m, position.min(entries.len() as u64)))
    }

    /// The `vt-agent:` provenance graph of `actor`: decisions with their annotations, model
    /// calls, tool calls, plans and refused sends.
    ///
    /// # Errors
    ///
    /// [`HostError`].
    pub fn graph(&mut self, actor: &str) -> Result<Graph, HostError> {
        self.activate(actor)?;
        let m = self
            .actors
            .get(actor)
            .ok_or_else(|| HostError::Unknown(actor.to_owned()))?;
        Ok(crate::ontology::records_graph(m.records()))
    }

    /// The cumulative model usage of `actor`.
    pub fn usage(&self, actor: &str) -> Option<UsageTotals> {
        self.actors.get(actor).map(AgentMachine::usage)
    }

    /// The open offers of `actor`.
    pub fn offers(&self, actor: &str) -> Vec<DecisionRequest> {
        self.actors
            .get(actor)
            .map(AgentMachine::open_offers)
            .unwrap_or_default()
    }

    /// Events sent to targets outside the host.
    pub fn outbox(&self) -> &[Sent] {
        &self.outbox
    }

    /// The pending durable timers, as (due time, IRI, actor).
    pub fn pending_timers(&self) -> Vec<(i64, String, String)> {
        self.timers
            .iter()
            .map(|((t, i), a)| (*t, i.clone(), a.clone()))
            .collect()
    }

    /// Forks `actor` at journal position `position` into `new_actor`: the new journal holds
    /// entries 1 to `position`, then `inject` (a different decision, an event) is delivered.
    /// History up to the fork is replayed, never re-executed.
    ///
    /// # Errors
    ///
    /// [`HostError`]; [`HostError::Unknown`] when `new_actor` already has a journal.
    pub fn fork(
        &mut self,
        actor: &str,
        position: u64,
        new_actor: &str,
        inject: Vec<AgentInput>,
    ) -> Result<(), HostError> {
        if !self.journal.read(new_actor).is_empty() {
            return Err(HostError::Unknown(format!("{new_actor} exists")));
        }
        for e in self.journal.read(actor) {
            if e.position > position {
                break;
            }
            let input = match e.input {
                AgentInput::Decision {
                    correlation,
                    event,
                    payload,
                    annotations,
                } => AgentInput::Decision {
                    correlation: rename(&correlation, actor, new_actor),
                    event,
                    payload,
                    annotations,
                },
                AgentInput::Refusal {
                    correlation,
                    reason,
                } => AgentInput::Refusal {
                    correlation: rename(&correlation, actor, new_actor),
                    reason,
                },
                AgentInput::Timer { iri } => AgentInput::Timer {
                    iri: rename(&iri, actor, new_actor),
                },
                AgentInput::ToolResult {
                    invokeid,
                    tool,
                    outcome,
                } => AgentInput::ToolResult {
                    invokeid: rename(&invokeid, actor, new_actor),
                    tool,
                    outcome,
                },
                other => other,
            };
            self.journal
                .append(new_actor, JournalEntry { input, ..e })
                .map_err(HostError::Journal)?;
        }
        self.activate(new_actor)?;
        for i in inject {
            self.enqueue(new_actor, i);
        }
        Ok(())
    }

    // ----- scheduling ------------------------------------------------------------------

    /// One scheduling round; true when anything happened.
    pub fn pump(&mut self) -> bool {
        let mut progress = self.poll_jobs();
        progress |= self.fire_timers();
        progress |= self.process_mailboxes();
        progress
    }

    /// Pumps until nothing is left to do at the current time; returns the rounds that did
    /// something.
    pub fn run_until_idle(&mut self) -> usize {
        let mut n = 0;
        while n < 100_000 && self.pump() {
            n += 1;
        }
        n
    }

    /// Runs until idle, letting other tasks (a node, a channel) make progress in between.
    pub async fn settle(&mut self) {
        let mut quiet = 0;
        while quiet < 4 {
            let did = self.run_until_idle();
            tokio::task::yield_now().await;
            quiet = if did > 0 { 0 } else { quiet + 1 };
        }
    }

    fn poll_jobs(&mut self) -> bool {
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        let mut got = false;
        while let Poll::Ready(Some(c)) = self.jobs.poll_next_unpin(&mut cx) {
            {
                {
                    got = true;
                    match c {
                        Completion::Input { actor, input } => {
                            if let AgentInput::Decision { correlation, .. }
                            | AgentInput::Refusal { correlation, .. } = &input
                            {
                                self.inflight.remove(&(actor.clone(), correlation.clone()));
                            }
                            if let AgentInput::LlmResult { invokeid, .. }
                            | AgentInput::LlmError { invokeid, .. }
                            | AgentInput::ToolResult { invokeid, .. } = &input
                            {
                                self.inflight.remove(&(actor.clone(), invokeid.clone()));
                                self.aborts.remove(&(actor.clone(), invokeid.clone()));
                            }
                            self.enqueue(&actor, input);
                        }
                        Completion::Partial { actor, event } => {
                            self.stats.partials += 1;
                            let now = self.now_ms();
                            if self.activate(&actor).is_ok()
                                && let Some(m) = self.actors.get_mut(&actor)
                                && let Ok(a) = m.apply_transient(event, now)
                            {
                                let effects = a.effects;
                                self.dispatch(&actor, effects);
                            }
                        }
                    }
                }
            }
        }
        got
    }

    fn fire_timers(&mut self) -> bool {
        let now = self.now_ms();
        let due: Vec<(i64, String)> = self
            .timers
            .range(..(now.saturating_add(1), String::new()))
            .map(|(k, _)| k.clone())
            .collect();
        let mut any = false;
        for k in due {
            if let Some(actor) = self.timers.remove(&k) {
                any = true;
                self.enqueue(&actor, AgentInput::Timer { iri: k.1 });
            }
        }
        any
    }

    fn process_mailboxes(&mut self) -> bool {
        let ready: Vec<String> = self
            .mailboxes
            .iter()
            .filter(|(_, q)| !q.is_empty())
            .map(|(a, _)| a.clone())
            .collect();
        let mut any = false;
        for actor in ready {
            let Some(input) = self.mailboxes.get_mut(&actor).and_then(VecDeque::pop_front) else {
                continue;
            };
            any = true;
            if let Err(e) = self.turn(&actor, input) {
                self.errors.insert(actor, e);
            }
        }
        any
    }

    fn turn(&mut self, actor: &str, input: AgentInput) -> Result<(), HostError> {
        self.activate(actor)?;
        let Some(m) = self.actors.get(actor) else {
            return Err(HostError::Unknown(actor.to_owned()));
        };
        let duplicate = match &input {
            AgentInput::Event { message_id, .. } => m.seen(message_id),
            AgentInput::Decision { correlation, .. } | AgentInput::Refusal { correlation, .. } => {
                m.answered(correlation)
            }
            AgentInput::Timer { iri } => !m.timers().iter().any(|t| t.iri == *iri),
            AgentInput::LlmResult { invokeid, .. } | AgentInput::LlmError { invokeid, .. } => {
                !m.is_running(invokeid)
            }
            AgentInput::ToolResult { invokeid, .. } => {
                !m.is_running(invokeid) && !invokeid.starts_with(&format!("{actor}/send/"))
            }
            AgentInput::Start { .. } => true,
        };
        if duplicate || m.is_terminated() {
            return Ok(());
        }
        let position = self.heads.get(actor).copied().unwrap_or(0) + 1;
        let ts = self.now_ms();
        self.journal
            .append(
                actor,
                JournalEntry {
                    position,
                    ts,
                    input: input.clone(),
                },
            )
            .map_err(HostError::Journal)?;
        self.heads.insert(actor.to_owned(), position);
        let Some(m) = self.actors.get_mut(actor) else {
            return Err(HostError::Unknown(actor.to_owned()));
        };
        let applied: Applied = match m.apply(&input, position, ts) {
            Ok(a) => a,
            Err(MachineError::Terminated) => return Ok(()),
            Err(e) => return Err(HostError::Machine(e)),
        };
        self.dispatch(actor, applied.effects);
        Ok(())
    }

    // ----- effects -------------------------------------------------------------------

    fn spawn_job(&mut self, actor: &str, key: &str, job: BoxStream<'static, Completion>) {
        let (job, handle) = abortable(job);
        self.inflight.insert((actor.to_owned(), key.to_owned()));
        self.aborts
            .insert((actor.to_owned(), key.to_owned()), handle);
        self.jobs.push(Box::pin(job));
    }

    fn dispatch(&mut self, actor: &str, effects: Vec<AgentEffect>) {
        for e in effects {
            match e {
                AgentEffect::Offer(req) => self.offer(actor, req),
                AgentEffect::CancelOffer {
                    correlation,
                    binding,
                } => {
                    if let Some(h) = self.aborts.remove(&(actor.to_owned(), correlation.clone())) {
                        h.abort();
                    }
                    self.inflight
                        .remove(&(actor.to_owned(), correlation.clone()));
                    if let Some(d) = self.deciders.get(&binding) {
                        d.cancel(&correlation);
                    }
                }
                AgentEffect::Timer(t) => {
                    self.timers.insert((t.due_ms, t.iri), actor.to_owned());
                }
                AgentEffect::CancelTimer(iri) => {
                    self.timers.retain(|(_, i), _| *i != iri);
                }
                AgentEffect::StartLlm(s) => self.start_llm(actor, s),
                AgentEffect::StartTool {
                    invokeid,
                    tool,
                    args,
                } => self.start_tool(actor, invokeid, tool, args),
                AgentEffect::CancelInvoke(id) => {
                    if let Some(h) = self.aborts.remove(&(actor.to_owned(), id.clone())) {
                        h.abort();
                    }
                    self.inflight.remove(&(actor.to_owned(), id));
                }
                AgentEffect::Send {
                    target,
                    event,
                    correlation,
                } => self.send_out(actor, target, event, correlation),
                AgentEffect::Deliver { message_id, event } => {
                    if !self.queued(actor, &message_id) {
                        self.enqueue(actor, AgentInput::Event { event, message_id });
                    }
                }
                AgentEffect::Done(_) => {}
            }
        }
    }

    // @tg: implements:: [[openspec:deciders#Non-blocking Decision Wait]]
    fn offer(&mut self, actor: &str, req: DecisionRequest) {
        let Some(decider) = self.deciders.get(&req.binding).cloned() else {
            let input = outcome_input(
                &req.correlation,
                DecisionOutcome::refuse(format!("no decider is bound to {}", req.binding)),
                Annotations::default(),
            );
            self.enqueue(actor, input);
            return;
        };
        *self
            .stats
            .decider_calls
            .entry(req.binding.clone())
            .or_insert(0) += 1;
        let base = Annotations {
            decider_kind: Some(decider.kind().as_str().to_owned()),
            decider: Some(req.binding.clone()),
            fallback: req.fallback,
            ..Annotations::default()
        };
        let corr = req.correlation.clone();
        let actor_s = actor.to_owned();
        let fut = decider.decide(req);
        let c2 = corr.clone();
        let job = futures::stream::once(async move {
            let outcome = fut.await;
            Completion::Input {
                actor: actor_s,
                input: outcome_input(&c2, outcome, base),
            }
        })
        .boxed();
        self.spawn_job(actor, &corr, job);
    }

    // @tg: implements:: [[openspec:llm-actors#LLM Invocation]]
    // @tg: implements:: [[openspec:llm-actors#Streaming Partials]]
    fn start_llm(&mut self, actor: &str, s: LlmStart) {
        let Some(provider) = self.providers.get(&s.provider).cloned() else {
            self.enqueue(
                actor,
                AgentInput::LlmError {
                    invokeid: s.invokeid.clone(),
                    attempt: s.attempt,
                    class: ErrorClass::UnknownProvider,
                    message: format!("no provider is bound to {}", s.provider),
                    retry_after_ms: None,
                },
            );
            return;
        };
        *self
            .stats
            .provider_calls
            .entry(s.provider.clone())
            .or_insert(0) += 1;
        let req = ProviderRequest {
            prompt: s.prompt.clone(),
            model: s.model.clone(),
            params: s.params.clone(),
            output_schema: s.output_schema.clone(),
            tools: self.tools.schemas(&s.tools),
        };
        let clock = self.clock.clone();
        let t0 = clock.now_ms();
        let id = s.invokeid.clone();
        let streamed = s.stream;
        let start = s;
        let to_input: ToInput = Arc::new(move |r| match r {
            Ok(r) => AgentInput::LlmResult {
                invokeid: start.invokeid.clone(),
                attempt: start.attempt,
                provider: start.provider.clone(),
                prompt_hash: start.prompt.hash.clone(),
                content: r.content,
                usage: r.usage,
                model: r.model,
                latency_ms: clock.now_ms().saturating_sub(t0),
                cost: r.cost,
                tool_calls: r.tool_calls,
            },
            Err(e) => AgentInput::LlmError {
                invokeid: start.invokeid.clone(),
                attempt: start.attempt,
                class: e.class,
                message: e.message,
                retry_after_ms: e.retry_after_ms,
            },
        });
        let items = if streamed {
            provider.stream(req.clone())
        } else {
            None
        };
        let actor_s = actor.to_owned();
        let job: BoxStream<'static, Completion> = match items {
            Some(items) => {
                let idc = id.clone();
                items
                    .map(move |item| match item {
                        Ok(StreamItem::Chunk(StreamChunk::Delta(d))) => Completion::Partial {
                            actor: actor_s.clone(),
                            event: partial_event(&idc, &d),
                        },
                        Ok(StreamItem::Done(r)) => Completion::Input {
                            actor: actor_s.clone(),
                            input: to_input(Ok(r)),
                        },
                        Err(e) => Completion::Input {
                            actor: actor_s.clone(),
                            input: to_input(Err(e)),
                        },
                    })
                    .boxed()
            }
            None => {
                let fut = provider.complete(req);
                futures::stream::once(async move {
                    let r = fut.await;
                    Completion::Input {
                        actor: actor_s,
                        input: to_input(r),
                    }
                })
                .boxed()
            }
        };
        self.spawn_job(actor, &id, job);
    }

    // @tg: implements:: [[openspec:llm-actors#Tool Actors]]
    fn start_tool(&mut self, actor: &str, invokeid: String, tool: String, args: Value) {
        let Some(t) = self.tools.get(&tool).cloned() else {
            self.enqueue(
                actor,
                AgentInput::ToolResult {
                    invokeid,
                    tool,
                    outcome: ToolOutcome::error(
                        ToolErrorClass::Unknown,
                        "no tool at this IRI",
                        &[],
                    ),
                },
            );
            return;
        };
        if let Err(v) = t.check_args(&args) {
            self.enqueue(
                actor,
                AgentInput::ToolResult {
                    invokeid,
                    tool,
                    outcome: ToolOutcome::error(ToolErrorClass::Arguments, "invalid arguments", &v),
                },
            );
            return;
        }
        *self.stats.tool_calls.entry(tool.clone()).or_insert(0) += 1;
        let fut = t.call(args);
        let actor_s = actor.to_owned();
        let id = invokeid.clone();
        let job = futures::stream::once(async move {
            let outcome = fut.await;
            Completion::Input {
                actor: actor_s,
                input: AgentInput::ToolResult {
                    invokeid: id,
                    tool,
                    outcome,
                },
            }
        })
        .boxed();
        self.spawn_job(actor, &invokeid, job);
    }

    fn send_out(&mut self, actor: &str, target: String, event: Event, correlation: String) {
        if self.tools.get(&target).is_some() {
            let args = event.data.clone();
            self.start_tool(actor, correlation, target, args);
            return;
        }
        if self.heads.contains_key(&target) || !self.journal.read(&target).is_empty() {
            let mut ev = event;
            ev.origin = Some(actor.to_owned());
            self.enqueue(
                &target,
                AgentInput::Event {
                    event: ev,
                    message_id: correlation,
                },
            );
            return;
        }
        if let Some(h) = &self.node
            && let Ok(a) = Address::parse(&target)
        {
            let _ = h.send(&a, event.clone());
        }
        self.outbox.push(Sent {
            from: actor.to_owned(),
            target,
            event,
        });
    }
}

fn snapshot(m: &AgentMachine, position: u64) -> AgentState {
    AgentState {
        position,
        configuration: m.configuration(),
        datamodel: m.datamodel(),
        terminated: m.is_terminated(),
        done: m.done_data().cloned(),
    }
}

fn rename(text: &str, from: &str, to: &str) -> String {
    match text.strip_prefix(from) {
        Some(rest) => format!("{to}{rest}"),
        None => text.to_owned(),
    }
}

fn partial_event(invokeid: &str, delta: &str) -> Event {
    let mut e = Event::named(
        crate::vocab::LLM_PARTIAL_EVENT,
        vertep_core::datamodel::EventType::External,
    );
    e.data = crate::json::obj([
        ("delta", crate::json::s(delta)),
        ("invokeid", crate::json::s(invokeid)),
    ]);
    e.invokeid = Some(invokeid.to_owned());
    e
}

/// The journal input for a decider's outcome; `base` supplies decider kind, binding and the
/// fallback flag when the decider did not set them.
pub fn outcome_input(correlation: &str, outcome: DecisionOutcome, base: Annotations) -> AgentInput {
    match outcome {
        DecisionOutcome::Decision(d) => {
            let mut a = d.annotations;
            a.decider_kind = a.decider_kind.or(base.decider_kind);
            a.decider = a.decider.or(base.decider);
            a.fallback = a.fallback || base.fallback;
            AgentInput::Decision {
                correlation: correlation.to_owned(),
                event: d.event,
                payload: d.payload,
                annotations: a,
            }
        }
        DecisionOutcome::Refusal(r) => AgentInput::Refusal {
            correlation: correlation.to_owned(),
            reason: r.reason,
        },
    }
}
