//! A runtime node: activations, turns, the outbox, timers, supervision and migrations.
//!
//! A [`Node`] owns the in-memory activations of one process. Its scheduler is synchronous and
//! deterministic: [`Node::pump`] drains finished background turns, fires due timers, runs one
//! turn for every actor with a deliverable message and deactivates idle actors;
//! [`Node::run_until_idle`] pumps until nothing is left to do at the current time, and
//! [`Node::run`] is the async loop that sleeps until the next timer or wake-up. Datamodels need
//! not be `Send`, so a node stays on one thread (`tokio::task::LocalSet`); everything outside
//! it talks to it through a [`NodeHandle`], which is `Send + Sync`.
//!
//! One turn of a virtual actor:
//!
//! 1. receive the oldest message not deferred by the active states;
//! 2. drop it if its message id is already journaled, if it is a second reply to a request, or
//!    if it is a timer that was cancelled;
//! 3. stamp the time, run the start macrostep (first entry only) and one macrostep;
//! 4. append the entry with compare-and-set at the expected position, carrying the
//!    current-state delta and the fencing token; on a conflict step down without dispatching;
//! 5. acknowledge the message, then dispatch the recorded sends (the outbox), arm timers, start
//!    and cancel invocations, record dead letters, and write a snapshot every N entries.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use futures::FutureExt;
use oxrdf::vocab::xsd;
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};
use vertep_core::datamodel::{DatamodelRegistry, Environment, Event, EventType, Value};
use vertep_core::ir::Chart;
use vertep_core::session::{InvokeRequest, SCXML_EVENT_PROCESSOR, SCXML_INVOKE_TYPE};
use vertep_rdf::ChartVersion;
use vertep_store::{AppendError, AppendMeta, FencingToken, Position, StateDelta, StoreError};

use crate::actor::{
    Actor, ActorFactory, ActorFailure, ActorFn, ActorInput, ActorStart, ChartActor, FnActor,
    OutTarget, Outgoing, START_EVENT, Turn, TurnOutput, platform_event,
};
use crate::address::{Address, hex};
use crate::chart::{
    ChartInfo, ChartRegistry, Migration, RegisterError, RestartMode, RestartPolicy,
    UnhandledPolicy, migrated_session, static_invoke_sources, unmapped_configurations,
};
use crate::journal::{
    DecodeError, EntryBody, EntryData, InputRecord, SendRecord, SnapshotData, TimerRecord,
};
use crate::machine::{Ctx, Machine, MachineError};
use crate::mailbox::{
    Envelope, Mailbox, MailboxError, MailboxFactory, MemoryMailbox, MemoryMailboxes,
};
use crate::placement::{Placement, Route};
use crate::shell::Shell;
use crate::storage::{Storage, select, text};
use crate::transport::{Directory, Resolver, Transport, Transports};
use crate::vocab::{self as vt, node};

/// Where a node simulates a crash (for tests of the journal protocol): the node stops at that
/// point of the next turn and does nothing afterwards, as if the process died.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    /// After the macrostep ran, before its entry is appended.
    BeforeAppend,
    /// After the entry is appended, before the message is acknowledged and sends dispatched.
    AfterAppend,
}

/// Node settings.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// A virtual actor idle this long, with nothing deliverable, is deactivated.
    pub idle_timeout_ms: i64,
    /// A snapshot is written whenever the journal position is a multiple of this; 0 disables
    /// snapshots.
    pub snapshot_interval: u64,
    /// The fencing token every append carries.
    pub fencing: Option<FencingToken>,
    /// How long an actor whose turn failed waits before it is activated again.
    pub failure_backoff_ms: i64,
    /// Simulated crash point (tests).
    pub crash_at: Option<CrashPoint>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        NodeConfig {
            idle_timeout_ms: 60_000,
            snapshot_interval: 100,
            fencing: None,
            failure_backoff_ms: 1_000,
            crash_at: None,
        }
    }
}

/// Why an actor could not be activated.
#[derive(Clone, Debug, PartialEq)]
pub enum ActivationError {
    /// A replayed macrostep produced a different configuration digest or different effects.
    ReplayDivergence {
        /// The actor.
        actor: Address,
        /// The diverging position.
        position: u64,
    },
    /// A recorded chart version is not in the registry.
    MissingVersion {
        /// The actor.
        actor: Address,
        /// The version IRI.
        version: String,
    },
    /// No chart type is registered for an actor with an empty journal.
    NoChart(Address),
    /// A journal record is malformed.
    Decode(DecodeError),
    /// The store failed.
    Store(StoreError),
    /// The interpreter failed.
    Machine(String),
    /// A turn's append failed for another reason than a conflict or a fencing token.
    Append(AppendError),
}

impl core::fmt::Display for ActivationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ActivationError::ReplayDivergence { actor, position } => {
                write!(f, "replay divergence for {actor} at position {position}")
            }
            ActivationError::MissingVersion { actor, version } => {
                write!(f, "chart version {version} of {actor} is not registered")
            }
            ActivationError::NoChart(a) => write!(f, "no chart type is registered for {a}"),
            ActivationError::Decode(e) => write!(f, "{e}"),
            ActivationError::Store(e) => write!(f, "{e}"),
            ActivationError::Machine(e) => write!(f, "{e}"),
            ActivationError::Append(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ActivationError {}

/// Why a migration or fork was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum MigrateError {
    /// No migration with that IRI is declared.
    UnknownMigration(String),
    /// The actor does not run the migration's source version.
    WrongVersion {
        /// The version the actor runs.
        actual: String,
        /// The migration's source version.
        expected: String,
    },
    /// Reachable configurations the migration does not map.
    Unmapped(Vec<BTreeSet<String>>),
    /// Too many reachable configurations to check.
    TooManyConfigurations(usize),
    /// The datamodel transform failed; nothing was appended.
    TransformFailed(String),
    /// The actor could not be activated.
    Activation(ActivationError),
    /// The actor has no state yet (empty journal).
    NotStarted,
    /// The target actor of a fork already has a journal.
    Exists(Address),
    /// The append failed.
    Append(AppendError),
}

impl core::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MigrateError::UnknownMigration(m) => write!(f, "unknown migration {m}"),
            MigrateError::WrongVersion { actual, expected } => {
                write!(f, "actor runs {actual}, migration starts from {expected}")
            }
            MigrateError::Unmapped(c) => {
                let list: Vec<String> = c
                    .iter()
                    .map(|s| format!("{{{}}}", s.iter().cloned().collect::<Vec<_>>().join(", ")))
                    .collect();
                write!(f, "unmapped configurations: {}", list.join(" "))
            }
            MigrateError::TooManyConfigurations(n) => {
                write!(f, "more than {n} reachable configurations")
            }
            MigrateError::TransformFailed(e) => write!(f, "datamodel transform failed: {e}"),
            MigrateError::Activation(e) => write!(f, "{e}"),
            MigrateError::NotStarted => f.write_str("the actor has no state yet"),
            MigrateError::Exists(a) => write!(f, "{a} already has a journal"),
            MigrateError::Append(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

/// Why a chart version could not be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// Non-final actors still run the version.
    InUse(Vec<Address>),
    /// The store failed while checking.
    Store(String),
}

impl core::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RemoveError::InUse(a) => write!(
                f,
                "version in use by {}",
                a.iter().map(Address::as_str).collect::<Vec<_>>().join(", ")
            ),
            RemoveError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RemoveError {}

/// Why a message could not be delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryError {
    /// The target is not an address or no resolver knows it.
    Unresolvable(String),
    /// The transport refused the message.
    Transport(String),
}

impl core::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DeliveryError::Unresolvable(t) => write!(f, "cannot resolve {t}"),
            DeliveryError::Transport(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DeliveryError {}

/// Why an external ask failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskError {
    /// No reply arrived in time.
    Timeout,
    /// The request could not be delivered.
    Delivery(DeliveryError),
}

impl core::fmt::Display for AskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AskError::Timeout => f.write_str("ask timed out"),
            AskError::Delivery(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AskError {}

/// An actor's state at a journal position.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorState {
    /// The journal position the state is after.
    pub position: u64,
    /// The chart version.
    pub version: String,
    /// Active state ids in document order.
    pub configuration: Vec<String>,
    /// The datamodel snapshot.
    pub datamodel: Value,
    /// Whether the session terminated.
    pub terminated: bool,
}

/// A recorded dead letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLetter {
    /// Its IRI.
    pub iri: String,
    /// The intended receiver.
    pub receiver: String,
    /// The sender, when known.
    pub sender: Option<String>,
    /// The event name.
    pub event: String,
    /// The payload as JSON text.
    pub payload: String,
    /// Why it is a dead letter.
    pub reason: String,
    /// When it was recorded.
    pub recorded_at: i64,
}

/// Counters for observing a node.
#[derive(Clone, Debug, Default)]
pub struct NodeStats {
    /// Activations created, per address.
    pub activations: BTreeMap<Address, u64>,
    /// Deactivations.
    pub deactivations: u64,
    /// Turns that appended an entry.
    pub turns: u64,
    /// Macrosteps re-run by replay.
    pub replayed: u64,
    /// The highest number of concurrently running worker turns seen, per address.
    pub worker_concurrency: BTreeMap<Address, usize>,
    /// Child restarts: (child, time scheduled for).
    pub restarts: Vec<(Address, i64)>,
}

/// A stateless worker pool.
#[derive(Clone)]
struct WorkerSpec {
    pool: usize,
    handler: ActorFn,
}

/// An ask waiting for its reply.
struct AskMailbox {
    tx: Mutex<Option<tokio::sync::oneshot::Sender<Event>>>,
}

impl Mailbox for AskMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        if let Some(tx) = self.tx.lock().ok().and_then(|mut g| g.take()) {
            let _ = tx.send(env.event);
        }
        Ok(())
    }
    fn receive(&self, _: &[String]) -> Option<Envelope> {
        None
    }
    fn ack(&self, _: &str) -> Result<(), MailboxError> {
        Ok(())
    }
    fn release(&self, _: &str) {}
    fn release_all(&self) {}
    fn pending(&self) -> Vec<Envelope> {
        Vec::new()
    }
    fn pending_deliverable(&self, _: &[String]) -> usize {
        0
    }
}

/// Accepts and drops everything (late replies to finished asks).
struct DropMailbox;

impl Mailbox for DropMailbox {
    fn enqueue(&self, _: Envelope) -> Result<(), MailboxError> {
        Ok(())
    }
    fn receive(&self, _: &[String]) -> Option<Envelope> {
        None
    }
    fn ack(&self, _: &str) -> Result<(), MailboxError> {
        Ok(())
    }
    fn release(&self, _: &str) {}
    fn release_all(&self) {}
    fn pending(&self) -> Vec<Envelope> {
        Vec::new()
    }
    fn pending_deliverable(&self, _: &[String]) -> usize {
        0
    }
}

const ASK_PREFIX: &str = "urn:vertep:ask:";

#[derive(Default)]
struct Hosted {
    workers: BTreeMap<Address, WorkerSpec>,
    children: BTreeSet<Address>,
    asks: BTreeMap<Address, Arc<AskMailbox>>,
}

struct Completion {
    address: Address,
    generation: u64,
    message_id: Option<String>,
    origin: Option<String>,
    correlation: Option<String>,
    result: Result<TurnOutput, ActorFailure>,
}

#[derive(Default)]
struct Ready {
    queue: VecDeque<Address>,
    set: HashSet<Address>,
}

/// The part of a node shared with handles and background turns.
struct Shared {
    storage: Arc<dyn Storage>,
    shell: Shell,
    directory: Directory,
    transports: Transports,
    default_mailboxes: Arc<dyn MailboxFactory>,
    scheme_mailboxes: BTreeMap<String, Arc<dyn MailboxFactory>>,
    mailboxes: Mutex<HashMap<Address, Arc<dyn Mailbox>>>,
    charts: RwLock<ChartRegistry>,
    hosted: RwLock<Hosted>,
    ready: Mutex<Ready>,
    completions: Mutex<VecDeque<Completion>>,
    jobs: AtomicUsize,
    wake: tokio::sync::Notify,
    counter: AtomicU64,
    nonce: u64,
    shutdown: AtomicBool,
    /// Where virtual actors and workers live, on a clustered node.
    placement: Option<Arc<dyn Placement>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ask,
    Worker,
    Child,
    Virtual,
}

impl Shared {
    fn charts(&self) -> std::sync::RwLockReadGuard<'_, ChartRegistry> {
        self.charts
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn charts_mut(&self) -> std::sync::RwLockWriteGuard<'_, ChartRegistry> {
        self.charts
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn hosted(&self) -> std::sync::RwLockReadGuard<'_, Hosted> {
        self.hosted
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn hosted_mut(&self) -> std::sync::RwLockWriteGuard<'_, Hosted> {
        self.hosted
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn kind(&self, a: &Address) -> Option<Kind> {
        {
            let h = self.hosted();
            if h.asks.contains_key(a) || a.as_str().starts_with(ASK_PREFIX) {
                return Some(Kind::Ask);
            }
            if h.workers.contains_key(a) {
                return Some(Kind::Worker);
            }
            if h.children.contains(a) {
                return Some(Kind::Child);
            }
        }
        if self.charts().type_of(a.as_str()).is_some() {
            return Some(Kind::Virtual);
        }
        if self
            .storage
            .journal()
            .head(a.node())
            .is_ok_and(|h| h.get() > 0)
        {
            return Some(Kind::Virtual);
        }
        None
    }

    /// The mailbox of a local address, created on first use.
    fn local_mailbox(&self, a: &Address) -> Option<(Arc<dyn Mailbox>, Kind)> {
        let kind = self.kind(a)?;
        if kind == Kind::Ask {
            let m: Arc<dyn Mailbox> = match self.hosted().asks.get(a) {
                Some(m) => m.clone(),
                None => Arc::new(DropMailbox),
            };
            return Some((m, kind));
        }
        let mut g = lock(&self.mailboxes);
        if let Some(m) = g.get(a) {
            return Some((m.clone(), kind));
        }
        let m = match kind {
            Kind::Child | Kind::Worker => Arc::new(MemoryMailbox::default()) as Arc<dyn Mailbox>,
            _ => self
                .scheme_mailboxes
                .get(a.scheme())
                .unwrap_or(&self.default_mailboxes)
                .create(a),
        };
        g.insert(a.clone(), m.clone());
        Some((m, kind))
    }

    fn mark_ready(&self, a: &Address) {
        let mut r = lock(&self.ready);
        if r.set.insert(a.clone()) {
            r.queue.push_back(a.clone());
        }
        drop(r);
        self.wake.notify_one();
    }

    /// Delivers `env` with the transport for `type_`.
    fn deliver(&self, env: Envelope, type_: &str) -> Result<(), DeliveryError> {
        let to =
            Address::parse(&env.to).map_err(|_| DeliveryError::Unresolvable(env.to.clone()))?;
        let transport: Arc<dyn Transport> = self
            .transports
            .get(type_)
            .or_else(|| self.transports.get(SCXML_EVENT_PROCESSOR))
            .cloned()
            .ok_or_else(|| DeliveryError::Transport(format!("no transport for {type_}")))?;
        if let Some(m) = self.placed_elsewhere(&to, &env) {
            return transport
                .deliver(m.as_ref(), env)
                .map_err(|e| DeliveryError::Transport(e.0));
        }
        self.deliver_with(env, &to, transport.as_ref())
    }

    /// The mailbox that carries messages for `to` to another node, when the placement says
    /// another node hosts it. Asks and invoked children always live here.
    fn placed_elsewhere(&self, to: &Address, env: &Envelope) -> Option<Arc<dyn Mailbox>> {
        let p = self.placement.as_ref()?;
        let kind = self.kind(to);
        if matches!(kind, Some(Kind::Ask | Kind::Child)) {
            return None;
        }
        match p.route(to, kind == Some(Kind::Virtual), Some(env)) {
            Route::Remote(m) => Some(m),
            Route::Local | Route::Hold => None,
        }
    }

    /// Delivers `env` to `to` on this node (or through the directory), without asking the
    /// placement.
    fn deliver_with(
        &self,
        env: Envelope,
        to: &Address,
        transport: &dyn Transport,
    ) -> Result<(), DeliveryError> {
        let to = to.clone();
        if let Some((m, kind)) = self.local_mailbox(&to) {
            transport
                .deliver(m.as_ref(), env)
                .map_err(|e| DeliveryError::Transport(e.0))?;
            if kind != Kind::Ask {
                self.mark_ready(&to);
            }
            return Ok(());
        }
        let m = self
            .directory
            .resolve(&to)
            .map_err(|_| DeliveryError::Unresolvable(to.as_str().to_owned()))?;
        transport
            .deliver(m.as_ref(), env)
            .map_err(|e| DeliveryError::Transport(e.0))
    }

    fn fresh_id(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!(
            "{prefix}{}-{:016x}-{n}",
            self.shell.clock.now_ms(),
            self.nonce
        )
    }

    fn complete(&self, c: Completion) {
        lock(&self.completions).push_back(c);
        self.jobs.fetch_sub(1, Ordering::SeqCst);
        self.wake.notify_one();
    }
}

/// A handle on a node for callers outside the actor system: send, ask, register resolvers,
/// shut down. It is `Send + Sync` and cheap to clone.
#[derive(Clone)]
pub struct NodeHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for NodeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle").finish_non_exhaustive()
    }
}

impl NodeHandle {
    /// Sends `event` to `to` from outside the actor system; returns the message id.
    ///
    /// # Errors
    ///
    /// [`DeliveryError`] when `to` cannot be resolved.
    pub fn send(&self, to: &Address, event: Event) -> Result<String, DeliveryError> {
        let id = self.shared.fresh_id("urn:vertep:message:");
        self.send_envelope(Envelope::new(id.clone(), to, event))?;
        Ok(id)
    }

    /// Delivers a complete envelope (with its own message id).
    ///
    /// # Errors
    ///
    /// [`DeliveryError`] when its receiver cannot be resolved.
    pub fn send_envelope(&self, env: Envelope) -> Result<(), DeliveryError> {
        self.shared.deliver(env, SCXML_EVENT_PROCESSOR)
    }

    /// Sends `event` to `to` and waits for its `vt:reply`, at most `timeout` on the node's
    /// clock. For callers outside the actor system only (HTTP handlers, tests); charts never
    /// block. A reply arriving after the timeout is dropped.
    ///
    /// # Errors
    ///
    /// [`AskError::Timeout`], or [`AskError::Delivery`] when `to` cannot be resolved.
    // @tg: implements:: [[openspec:messaging#External Blocking Ask]]
    pub async fn ask(
        &self,
        to: &Address,
        event: Event,
        timeout: Duration,
    ) -> Result<Event, AskError> {
        let me = Address::parse(&self.shared.fresh_id(ASK_PREFIX))
            .map_err(|e| AskError::Delivery(DeliveryError::Unresolvable(e.text)))?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shared.hosted_mut().asks.insert(
            me.clone(),
            Arc::new(AskMailbox {
                tx: Mutex::new(Some(tx)),
            }),
        );
        let mut ev = event;
        ev.origin = Some(me.as_str().to_owned());
        ev.origintype = Some(SCXML_EVENT_PROCESSOR.to_owned());
        ev.originverified = false;
        let mut env = Envelope::new(format!("{me}/message"), to, ev);
        env.correlation = Some(format!("{me}/request"));
        if let Err(e) = self.send_envelope(env) {
            self.shared.hosted_mut().asks.remove(&me);
            return Err(AskError::Delivery(e));
        }
        let deadline = self
            .shared
            .shell
            .clock
            .now_ms()
            .saturating_add(i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX));
        let sleep = self.shared.shell.sleep.sleep_until(deadline);
        let r = futures::future::select(rx, sleep).await;
        self.shared.hosted_mut().asks.remove(&me);
        match r {
            futures::future::Either::Left((Ok(ev), _)) => Ok(ev),
            _ => Err(AskError::Timeout),
        }
    }

    /// Registers a resolver for addresses of `scheme`.
    pub fn register_resolver(&self, scheme: &str, resolver: Arc<dyn Resolver>) {
        self.shared.directory.register(scheme, resolver);
    }

    /// Binds `address` to the virtual actor of `chart_type` with `key`: the actor is hosted
    /// here and activated on message like `<chart_type>/<key>`, with `address` as its subject.
    /// Used for actors whose address is a DID.
    pub fn bind_address(&self, address: &Address, chart_type: &str, key: &str) {
        self.shared
            .charts_mut()
            .bind(address.as_str(), chart_type, key);
    }

    /// Tells the node that the mailbox of the local actor `a` received messages from outside
    /// the node (a relay pickup), so the actor is activated and receives them. False when `a`
    /// is not hosted here.
    pub fn notify(&self, a: &Address) -> bool {
        match self.shared.local_mailbox(a) {
            Some((_, kind)) if kind != Kind::Ask => {
                self.shared.mark_ready(a);
                true
            }
            _ => false,
        }
    }

    /// Asks [`Node::run`] to return.
    pub fn shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.wake.notify_one();
    }

    /// The node's clock reading.
    pub fn now_ms(&self) -> i64 {
        self.shared.shell.clock.now_ms()
    }

    /// Completes once the node has new work (a delivered message, a finished background turn,
    /// a shutdown request), for shells that drive the node with [`Node::run_until_idle`]. A
    /// signal sent while nobody waits is kept for the next waiter.
    pub fn wakeup(&self) -> crate::shell::BoxFuture<()> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.wake.notified().await })
    }

    /// Delivers `env` to an actor hosted on this node without asking the placement: what a
    /// cluster does with a message another node sent here for an actor this node owns.
    ///
    /// # Errors
    ///
    /// [`DeliveryError`] when its receiver cannot be resolved.
    pub fn deliver_here(&self, env: Envelope) -> Result<(), DeliveryError> {
        let to =
            Address::parse(&env.to).map_err(|_| DeliveryError::Unresolvable(env.to.clone()))?;
        let transport = self
            .shared
            .transports
            .get(SCXML_EVENT_PROCESSOR)
            .cloned()
            .ok_or_else(|| DeliveryError::Transport("no local transport".into()))?;
        self.shared.deliver_with(env, &to, transport.as_ref())
    }

    /// Records `env` in `receiver`'s dead-letter graph with `reason`, at the node's time.
    pub fn record_dead_letter(&self, receiver: &Address, env: &Envelope, reason: &str) {
        self.shared
            .dead_letter(receiver, env, reason, self.shared.shell.clock.now_ms());
    }

    /// The number of messages waiting in the local mailbox of `a` (none when `a` has no
    /// mailbox here).
    pub fn pending(&self, a: &Address) -> usize {
        lock(&self.shared.mailboxes).get(a).map_or(0, |m| m.len())
    }
}

/// Builds a [`Node`].
pub struct NodeBuilder {
    storage: Arc<dyn Storage>,
    shell: Shell,
    config: NodeConfig,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    mailboxes: Arc<dyn MailboxFactory>,
    scheme_mailboxes: BTreeMap<String, Arc<dyn MailboxFactory>>,
    transports: Transports,
    placement: Option<Arc<dyn Placement>>,
}

impl std::fmt::Debug for NodeBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeBuilder")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl NodeBuilder {
    /// Datamodels the node's charts may name (`null` is always available).
    pub fn datamodels(mut self, r: Arc<dyn DatamodelRegistry + Send + Sync>) -> Self {
        self.registry = r;
        self
    }

    /// The mailbox implementation for every address without a scheme-specific one.
    pub fn mailboxes(mut self, f: Arc<dyn MailboxFactory>) -> Self {
        self.mailboxes = f;
        self
    }

    /// The mailbox implementation for addresses of `scheme`.
    pub fn mailboxes_for_scheme(mut self, scheme: &str, f: Arc<dyn MailboxFactory>) -> Self {
        self.scheme_mailboxes.insert(scheme.to_owned(), f);
        self
    }

    /// Registers a transport besides the local one.
    pub fn transport(mut self, t: Arc<dyn Transport>) -> Self {
        self.transports.register(t);
        self
    }

    /// Node settings.
    pub fn config(mut self, c: NodeConfig) -> Self {
        self.config = c;
        self
    }

    /// The placement of a clustered node: where virtual actors and workers live. Without one
    /// the node hosts every virtual actor it knows the chart type of.
    pub fn placement(mut self, p: Arc<dyn Placement>) -> Self {
        self.placement = Some(p);
        self
    }

    /// Builds the node. On its first [`Node::pump`] (after the chart types are registered) it
    /// recovers what a previous process left: pending timers, undispatched sends and stored
    /// mailbox messages.
    pub fn build(self) -> Node {
        let nonce = {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(self.shell.clock.now_ms() as u64);
            h.finish()
        };
        let mut factories = vec![self.mailboxes.clone()];
        factories.extend(self.scheme_mailboxes.values().cloned());
        let shared = Arc::new(Shared {
            storage: self.storage,
            shell: self.shell,
            directory: Directory::default(),
            transports: self.transports,
            default_mailboxes: self.mailboxes,
            scheme_mailboxes: self.scheme_mailboxes,
            mailboxes: Mutex::new(HashMap::new()),
            charts: RwLock::new(ChartRegistry::default()),
            hosted: RwLock::new(Hosted::default()),
            ready: Mutex::new(Ready::default()),
            completions: Mutex::new(VecDeque::new()),
            jobs: AtomicUsize::new(0),
            wake: tokio::sync::Notify::new(),
            counter: AtomicU64::new(0),
            nonce,
            shutdown: AtomicBool::new(false),
            placement: self.placement,
        });

        Node {
            shared,
            config: self.config,
            registry: self.registry,
            kinds: BTreeMap::new(),
            actors: BTreeMap::new(),
            timers: BTreeMap::new(),
            timer_keys: HashMap::new(),
            volatile: BTreeMap::new(),
            parked: BTreeMap::new(),
            errors: BTreeMap::new(),
            migrations: BTreeMap::new(),
            seq: 0,
            crashed: false,
            recovered: false,
            stats: NodeStats::default(),
            factories,
            outbox: HashMap::new(),
        }
    }
}

struct Armed {
    rec: TimerRecord,
    occurrence: u64,
}

enum Volatile {
    Deliver(Box<Envelope>),
    /// A recorded send of the entry at the position that its transport refused: retried until
    /// delivered, and owed by the outbox until then.
    Resend(Address, u64, Box<SendRecord>),
    Restart(Address, u64),
}

struct VirtualActivation {
    machine: Option<Machine>,
    first: Arc<ChartInfo>,
    chart_type: Option<(String, String)>,
    head: u64,
    seen: HashSet<String>,
    state: HashSet<Triple>,
    last_active: i64,
}

impl VirtualActivation {
    fn deferred(&self) -> Vec<String> {
        self.machine
            .as_ref()
            .map(|m| m.info.deferred(&m.session))
            .unwrap_or_default()
    }

    fn info(&self) -> &Arc<ChartInfo> {
        self.machine.as_ref().map_or(&self.first, |m| &m.info)
    }
}

struct ChildActivation {
    parent: Address,
    invokeid: String,
    make: Arc<dyn Fn() -> Result<Box<dyn Actor>, ActorFailure> + Send + Sync>,
    actor: Box<dyn Actor>,
    busy: bool,
    generation: u64,
    policy: RestartPolicy,
    failures: Vec<i64>,
    out_seq: u64,
    suppress: bool,
}

struct WorkerPool {
    spec: WorkerSpec,
    busy: usize,
}

enum Activation {
    Virtual(Box<VirtualActivation>),
    Child(Box<ChildActivation>),
    Worker(WorkerPool),
}

/// A runtime node. See the module documentation.
pub struct Node {
    shared: Arc<Shared>,
    config: NodeConfig,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    kinds: BTreeMap<String, ActorFactory>,
    actors: BTreeMap<Address, Activation>,
    timers: BTreeMap<(i64, String), Armed>,
    timer_keys: HashMap<String, (i64, String)>,
    volatile: BTreeMap<(i64, u64), Volatile>,
    parked: BTreeMap<Address, i64>,
    errors: BTreeMap<Address, ActivationError>,
    migrations: BTreeMap<Address, VecDeque<(String, String)>>,
    seq: u64,
    crashed: bool,
    recovered: bool,
    stats: NodeStats,
    factories: Vec<Arc<dyn MailboxFactory>>,
    outbox: HashMap<Address, Outbox>,
}

/// What one actor's outbox still owes in this process. The durable mark
/// (`vt:dispatchedThrough`) moves only over entries whose sends were all delivered, so a send a
/// transport refused is dispatched again after a crash.
#[derive(Default)]
struct Outbox {
    /// The highest journal position whose sends were dispatched.
    tip: u64,
    /// Positions with sends a transport refused and that are not delivered yet: how many each.
    owed: BTreeMap<u64, usize>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("active", &self.actors.keys().collect::<Vec<_>>())
            .field("timers", &self.timers.len())
            .finish_non_exhaustive()
    }
}

/// The outcome of loading an actor's journal.
struct Loaded {
    machine: Option<Machine>,
    head: u64,
    seen: HashSet<String>,
    entries: Vec<EntryData>,
}

fn is_scxml_invoke(t: &str) -> bool {
    t == SCXML_INVOKE_TYPE || t == "http://www.w3.org/TR/scxml" || t == "scxml"
}

impl Node {
    /// A builder for a node on `storage` with the shell services `shell`.
    pub fn builder(storage: Arc<dyn Storage>, shell: Shell) -> NodeBuilder {
        NodeBuilder {
            storage,
            shell,
            config: NodeConfig::default(),
            registry: Arc::new(vertep_core::null::NullRegistry),
            mailboxes: Arc::new(MemoryMailboxes),
            scheme_mailboxes: BTreeMap::new(),
            transports: Transports::local(),
            placement: None,
        }
    }

    /// A handle for callers outside the actor system.
    pub fn handle(&self) -> NodeHandle {
        NodeHandle {
            shared: self.shared.clone(),
        }
    }

    /// Counters.
    pub fn stats(&self) -> &NodeStats {
        &self.stats
    }

    /// The storage.
    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.shared.storage
    }

    /// The node's clock reading.
    pub fn now_ms(&self) -> i64 {
        self.shared.shell.clock.now_ms()
    }

    /// Whether `a` has an activation on this node.
    pub fn is_active(&self, a: &Address) -> bool {
        self.actors.contains_key(a)
    }

    /// Activates virtual actor `a` now, without waiting for a message (a no-op when active).
    ///
    /// # Errors
    ///
    /// [`ActivationError`] when its journal cannot be replayed.
    pub fn activate_actor(&mut self, a: &Address) -> Result<(), ActivationError> {
        if self.actors.contains_key(a) {
            return Ok(());
        }
        let now = self.now_ms();
        self.activate(a, now)
    }

    /// The last activation error of `a`.
    pub fn last_error(&self, a: &Address) -> Option<&ActivationError> {
        self.errors.get(a)
    }

    /// The registered chart version `iri`.
    pub fn chart(&self, iri: &str) -> Option<Arc<ChartInfo>> {
        self.shared.charts().get(iri)
    }

    /// The latest version of chart type `ty`.
    pub fn latest(&self, ty: &str) -> Option<Arc<ChartInfo>> {
        self.shared.charts().latest(ty)
    }

    /// The messages waiting for `a`, in order.
    pub fn mailbox(&self, a: &Address) -> Vec<Envelope> {
        self.shared
            .local_mailbox(a)
            .map(|(m, _)| m.pending())
            .unwrap_or_default()
    }

    // ----- registration ---------------------------------------------------------------------

    /// Registers `chart` as the latest version of chart type `chart_type`; virtual actors
    /// addressed `<chart_type>/<key>` run it. Existing actors keep their version.
    ///
    /// # Errors
    ///
    /// [`RegisterError`] when the type is not an IRI, the chart is invalid, or an `<invoke>`
    /// targets a virtual actor.
    // @tg: implements:: [[openspec:chart-versioning#Pin Until Final]]
    // @tg: implements:: [[openspec:actor-model#Virtual Actors Are Not Invokable]]
    pub fn register_type(
        &mut self,
        chart_type: &str,
        chart: Chart,
    ) -> Result<ChartVersion, RegisterError> {
        Address::parse(chart_type).map_err(|_| RegisterError::BadType(chart_type.to_owned()))?;
        let mut srcs = Vec::new();
        static_invoke_sources(&chart.states, &mut srcs);
        {
            let charts = self.shared.charts();
            for (state, src) in srcs {
                let own = src
                    .strip_prefix(chart_type)
                    .is_some_and(|r| r.starts_with('/'));
                if own || charts.type_of(&src).is_some() {
                    return Err(RegisterError::InvokesVirtualActor { state, src });
                }
            }
        }
        let info = ChartInfo::new(chart, Some(chart_type.to_owned()))?;
        Ok(self.shared.charts_mut().insert(info).version.clone())
    }

    /// Registers a chart version without a type (for invoked charts and migrations).
    ///
    /// # Errors
    ///
    /// [`RegisterError`] when the chart is invalid.
    pub fn register_chart(&mut self, chart: Chart) -> Result<ChartVersion, RegisterError> {
        let info = ChartInfo::new(chart, None)?;
        Ok(self.shared.charts_mut().insert(info).version.clone())
    }

    /// Declares a migration.
    pub fn register_migration(&mut self, m: Migration) {
        self.shared.charts_mut().add_migration(m);
    }

    /// Registers an actor kind that `<invoke type="...">` can start.
    pub fn register_actor_kind(&mut self, invoke_type: &str, factory: ActorFactory) {
        self.kinds.insert(invoke_type.to_owned(), factory);
    }

    /// Registers a pool of `pool` stateless workers at `address`, each running `handler`.
    // @tg: implements:: [[openspec:actor-model#Stateless Worker Lifecycle]]
    pub fn register_worker(&mut self, address: Address, pool: usize, handler: ActorFn) {
        self.shared.hosted_mut().workers.insert(
            address,
            WorkerSpec {
                pool: pool.max(1),
                handler,
            },
        );
    }

    /// Sends `event` to `to` from outside the actor system.
    ///
    /// # Errors
    ///
    /// [`DeliveryError`] when `to` cannot be resolved.
    pub fn send(&self, to: &Address, event: Event) -> Result<String, DeliveryError> {
        self.handle().send(to, event)
    }

    /// Delivers a complete envelope.
    ///
    /// # Errors
    ///
    /// [`DeliveryError`] when its receiver cannot be resolved.
    pub fn send_envelope(&self, env: Envelope) -> Result<(), DeliveryError> {
        self.handle().send_envelope(env)
    }

    /// Removes chart version `iri`, unless a non-final actor still uses it.
    ///
    /// # Errors
    ///
    /// [`RemoveError::InUse`] listing those actors.
    pub fn remove_version(&mut self, iri: &str) -> Result<(), RemoveError> {
        let j = self.shared.storage.journal();
        let mut users = BTreeSet::new();
        for (a, act) in &self.actors {
            if let Activation::Virtual(v) = act
                && let Some(m) = &v.machine
                && m.info.version.as_str() == iri
                && !m.session.is_terminated()
            {
                users.insert(a.clone());
            }
        }
        for a in j.actors().map_err(|e| RemoveError::Store(e.to_string()))? {
            let head = j.head(&a).map_err(|e| RemoveError::Store(e.to_string()))?;
            let recs = j
                .read(&a, head)
                .map_err(|e| RemoveError::Store(e.to_string()))?;
            let Ok(addr) = Address::parse(a.as_str()) else {
                continue;
            };
            if let Some(r) = recs.last()
                && let Ok(e) = EntryData::from_record(&addr, r)
                && e.version == iri
                && !e.effects().is_some_and(|x| x.terminated)
            {
                users.insert(addr);
            }
        }
        if !users.is_empty() {
            return Err(RemoveError::InUse(users.into_iter().collect()));
        }
        self.shared.charts_mut().remove(iri);
        Ok(())
    }

    // ----- scheduling -----------------------------------------------------------------------

    /// One scheduling round; true when anything happened.
    pub fn pump(&mut self) -> bool {
        if self.crashed {
            return false;
        }
        if !self.recovered {
            self.recovered = true;
            self.recover();
        }
        let now = self.now_ms();
        let unparked: Vec<Address> = self
            .parked
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(a, _)| a.clone())
            .collect();
        for a in unparked {
            self.parked.remove(&a);
            self.shared.mark_ready(&a);
        }
        let mut progress = self.drain_completions(now);
        progress |= self.fire_timers(now);
        progress |= self.fire_volatile(now);
        progress |= self.process_ready(now);
        progress |= self.deactivate_idle(now);
        progress
    }

    /// Pumps until nothing is left to do at the current time; returns the rounds that did
    /// something.
    pub fn run_until_idle(&mut self) -> usize {
        let mut n = 0;
        while n < 1_000_000 && self.pump() {
            n += 1;
        }
        n
    }

    /// Runs until idle and no background turn is running.
    pub async fn settle(&mut self) {
        loop {
            self.run_until_idle();
            if self.crashed
                || (self.shared.jobs.load(Ordering::SeqCst) == 0
                    && lock(&self.shared.completions).is_empty())
            {
                return;
            }
            self.shared.wake.notified().await;
        }
    }

    /// The node's event loop: runs until idle, then sleeps until the next timer, idle
    /// deadline or wake-up, until [`NodeHandle::shutdown`].
    pub async fn run(&mut self) {
        loop {
            self.run_until_idle();
            if self.crashed || self.shared.shutdown.load(Ordering::SeqCst) {
                return;
            }
            let notified = self.shared.wake.notified();
            match self.next_deadline() {
                Some(t) => {
                    let sleep = self.shared.shell.sleep.sleep_until(t);
                    futures::future::select(Box::pin(notified), sleep).await;
                }
                None => notified.await,
            }
        }
    }

    /// The earliest time something is due (a timer, a failed actor's back-off, an idle
    /// deadline), for shells that drive the node with [`Node::run_until_idle`] themselves.
    pub fn next_deadline(&self) -> Option<i64> {
        let mut d: Vec<i64> = Vec::new();
        d.extend(self.timers.keys().next().map(|k| k.0));
        d.extend(self.volatile.keys().next().map(|k| k.0));
        d.extend(self.parked.values().min().copied());
        for a in self.actors.values() {
            if let Activation::Virtual(v) = a {
                d.push(v.last_active.saturating_add(self.config.idle_timeout_ms));
            }
        }
        d.into_iter().min()
    }

    fn ctx(&self, a: &Address) -> (Vec<String>, Vec<String>, Value) {
        let mut invoke_types: Vec<String> = self.kinds.keys().cloned().collect();
        invoke_types.push(SCXML_INVOKE_TYPE.to_owned());
        (
            self.shared.transports.send_types(),
            invoke_types,
            self.shared.transports.ioprocessors(a),
        )
    }

    fn park(&mut self, a: &Address, now: i64) {
        self.parked.insert(
            a.clone(),
            now.saturating_add(self.config.failure_backoff_ms),
        );
    }

    fn crash(&mut self) {
        self.crashed = true;
    }

    fn process_ready(&mut self, now: i64) -> bool {
        let batch: Vec<Address> = {
            let mut r = lock(&self.shared.ready);
            r.set.clear();
            r.queue.drain(..).collect()
        };
        let mut progress = false;
        for a in batch {
            if self.crashed {
                break;
            }
            if self.parked.contains_key(&a) {
                continue;
            }
            let Some(kind) = self.shared.kind(&a) else {
                continue;
            };
            let did = match kind {
                Kind::Ask => false,
                Kind::Worker => self.worker_turns(&a, now),
                Kind::Child => self.child_next(&a, now),
                Kind::Virtual => self.virtual_turn(&a, now),
            };
            progress |= did;
            if did && self.has_work(&a, kind) {
                self.shared.mark_ready(&a);
            }
        }
        progress
    }

    fn has_work(&self, a: &Address, kind: Kind) -> bool {
        if self.migrations.get(a).is_some_and(|q| !q.is_empty()) {
            return true;
        }
        let Some((m, _)) = self.shared.local_mailbox(a) else {
            return false;
        };
        let deferred = match self.actors.get(a) {
            Some(Activation::Virtual(v)) => v.deferred(),
            Some(Activation::Child(c)) if c.busy => return false,
            Some(Activation::Worker(w)) if w.busy >= w.spec.pool => return false,
            _ => Vec::new(),
        };
        let _ = kind;
        m.has_deliverable(&deferred)
    }

    // ----- virtual actors -------------------------------------------------------------------

    /// Restores `a` from its latest valid snapshot and the entries after it, up to `upto`.
    // @tg: implements:: [[openspec:event-journal#Deterministic Replay]]
    // @tg: implements:: [[openspec:chart-versioning#Replay With Recorded Version]]
    fn load(&mut self, a: &Address, upto: Option<u64>) -> Result<Loaded, ActivationError> {
        let j = self.shared.storage.journal();
        let recs = j
            .read(a.node(), Position::ZERO)
            .map_err(ActivationError::Store)?;
        let mut entries = Vec::with_capacity(recs.len());
        for r in &recs {
            if upto.is_some_and(|u| r.position.get() > u) {
                break;
            }
            entries.push(EntryData::from_record(a, r).map_err(ActivationError::Decode)?);
        }
        let head = entries.len() as u64;
        let seen: HashSet<String> = entries.iter().map(|e| e.message_id.clone()).collect();
        let (sends, invokes, ioproc) = self.ctx(a);
        let registry = self.registry.clone();
        let ctx = Ctx {
            registry: registry.as_ref(),
            ioprocessors: ioproc,
            send_types: sends,
            invoke_types: invokes,
        };
        let shared = self.shared.clone();
        let charts = |v: &str| shared.charts().get(v);
        let mut replayed = 0u64;
        let missing = |v: &str| ActivationError::MissingVersion {
            actor: a.clone(),
            version: v.to_owned(),
        };
        // the latest valid snapshot at or before the head
        let mut machine: Option<Machine> = None;
        let mut from = 1u64;
        for s in self.snapshots(a) {
            if s.position > head || s.position == 0 {
                continue;
            }
            let e = &entries[(s.position - 1) as usize];
            if e.version != s.version || e.digest != s.digest {
                continue;
            }
            let Some(info) = charts(&s.version) else {
                continue;
            };
            if let Ok(m) = Machine::from_snapshot(a, info, &ctx, &s) {
                machine = Some(m);
                from = s.position + 1;
                break;
            }
        }
        for e in entries.iter().skip((from - 1) as usize) {
            match &e.body {
                EntryBody::Fork { snapshot, .. } => {
                    let info =
                        charts(&snapshot.version).ok_or_else(|| missing(&snapshot.version))?;
                    let mut m = Machine::from_snapshot(a, info, &ctx, snapshot)
                        .map_err(|x| ActivationError::Machine(x.to_string()))?;
                    m.session.sessionid = a.as_str().to_owned();
                    machine = Some(m);
                }
                EntryBody::Migration { after, .. } => {
                    let info = charts(&after.version).ok_or_else(|| missing(&after.version))?;
                    machine = Some(
                        Machine::from_snapshot(a, info, &ctx, after)
                            .map_err(|x| ActivationError::Machine(x.to_string()))?,
                    );
                }
                EntryBody::Input { input, effects } => {
                    let info = charts(&e.version).ok_or_else(|| missing(&e.version))?;
                    let env = Environment {
                        now_ms: e.timestamp_ms,
                        seed: crate::address::seed_of(&e.iri()),
                    };
                    let (mut m, start, trace) = match machine.take() {
                        Some(m) => {
                            if m.info.version.as_str() != e.version {
                                return Err(ActivationError::ReplayDivergence {
                                    actor: a.clone(),
                                    position: e.position,
                                });
                            }
                            (m, Vec::new(), None)
                        }
                        None => {
                            let (m, eff, tr) = Machine::start(a, info.clone(), &ctx, env)
                                .map_err(|x| ActivationError::Machine(x.to_string()))?;
                            (m, eff, Some(tr))
                        }
                    };
                    let policy = info.unhandled.unwrap_or(UnhandledPolicy::DeadLetter);
                    let applied = m
                        .apply(
                            input,
                            e.position,
                            e.timestamp_ms,
                            policy,
                            start,
                            trace.as_ref(),
                        )
                        .map_err(|x| ActivationError::Machine(x.to_string()))?;
                    replayed += 1;
                    let same_sends = applied.effects.sends.len() == effects.sends.len()
                        && applied
                            .effects
                            .sends
                            .iter()
                            .zip(&effects.sends)
                            .all(|(x, y)| x.send_id == y.send_id && x.event.name == y.event.name);
                    if m.digest() != e.digest || !same_sends {
                        return Err(ActivationError::ReplayDivergence {
                            actor: a.clone(),
                            position: e.position,
                        });
                    }
                    machine = Some(m);
                }
            }
        }
        self.stats.replayed += replayed;
        Ok(Loaded {
            machine,
            head,
            seen,
            entries,
        })
    }

    /// Every snapshot record of `a`, newest first.
    fn snapshots(&self, a: &Address) -> Vec<SnapshotData> {
        let q = format!(
            "SELECT ?s ?n ?v ?d ?ss ?dm ?rt WHERE {{ GRAPH ?s {{ ?s a <{snap}> ; <{actor}> {a} ; \
             <{pos}> ?n ; <{ver}> ?v ; <{dig}> ?d ; <{ses}> ?ss ; <{dms}> ?dm ; <{rts}> ?rt }} }}",
            snap = vt::SNAPSHOT,
            actor = vt::SNAPSHOT_OF,
            a = a.node(),
            pos = vt::POSITION,
            ver = vt::CHART_VERSION,
            dig = vt::CONFIGURATION_DIGEST,
            ses = vt::SESSION_STATE,
            dms = vt::DATAMODEL_STATE,
            rts = vt::RUNTIME_STATE,
        );
        let Ok(sol) = select(self.shared.storage.graph(a.node()), &q) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..sol.len() {
            let g = |v: &str| sol.get(i, v).and_then(text);
            if let (Some(s), Some(n), Some(v), Some(d), Some(ss), Some(dm), Some(rt)) = (
                g("s"),
                g("n").and_then(|n| n.parse::<u64>().ok()),
                g("v"),
                g("d"),
                g("ss"),
                g("dm"),
                g("rt"),
            ) {
                out.push(SnapshotData {
                    iri: s,
                    actor: a.as_str().to_owned(),
                    position: n,
                    version: v,
                    digest: d,
                    session: ss,
                    datamodel: dm,
                    runtime: rt,
                });
            }
        }
        out.sort_by_key(|x| std::cmp::Reverse(x.position));
        out
    }

    fn outbox_marks(&self, a: &Address) -> (u64, HashSet<String>) {
        let q = format!(
            "SELECT ?s ?p ?o WHERE {{ GRAPH {g} {{ ?s ?p ?o }} }}",
            g = a.outbox_graph()
        );
        let mut through = 0;
        let mut fired = HashSet::new();
        if let Ok(sol) = select(self.shared.storage.graph(a.node()), &q) {
            for i in 0..sol.len() {
                let p = sol.get(i, "p").and_then(text).unwrap_or_default();
                if p == vt::DISPATCHED_THROUGH {
                    through = through.max(
                        sol.get(i, "o")
                            .and_then(text)
                            .and_then(|n| n.parse().ok())
                            .unwrap_or(0),
                    );
                } else if p == vt::DISPATCHED
                    && let Some(s) = sol.get(i, "s").and_then(text)
                {
                    fired.insert(s);
                }
            }
        }
        (through, fired)
    }

    /// Activates `a`: replays its journal, re-arms its timers, dispatches sends left
    /// undispatched and rebuilds running invocations.
    // @tg: implements:: [[openspec:actor-model#Virtual Actor Activation]]
    fn activate(&mut self, a: &Address, now: i64) -> Result<(), ActivationError> {
        let loaded = self.load(a, None)?;
        let chart_type = self.shared.charts().type_of(a.as_str());
        let first = match &loaded.machine {
            Some(m) => m.info.clone(),
            None => chart_type
                .as_ref()
                .and_then(|(t, _)| self.shared.charts().latest(t))
                .ok_or_else(|| ActivationError::NoChart(a.clone()))?,
        };
        let (through, fired) = self.outbox_marks(a);
        let mut machine = loaded.machine;
        if let Some(m) = &mut machine {
            m.rt.timers.retain(|k, _| !fired.contains(k));
        }
        let ct = chart_type.as_ref().map(|(t, k)| (t.as_str(), k.as_str()));
        let state = machine
            .as_ref()
            .map(|m| m.state_triples(ct))
            .unwrap_or_default();
        // re-arm timers
        if let Some(m) = &machine {
            for t in m.rt.timers.values() {
                if !self.timer_keys.contains_key(&t.iri) {
                    self.arm(t.clone(), now, true);
                }
            }
        }
        let invocations: Vec<_> = machine
            .as_ref()
            .map(|m| m.rt.invocations.values().cloned().collect())
            .unwrap_or_default();
        *self.stats.activations.entry(a.clone()).or_default() += 1;
        self.errors.remove(a);
        self.actors.insert(
            a.clone(),
            Activation::Virtual(Box::new(VirtualActivation {
                machine,
                first,
                chart_type,
                head: loaded.head,
                seen: loaded.seen,
                state,
                last_active: now,
            })),
        );
        // the outbox: sends of entries after the last dispatched position
        for e in loaded.entries.iter().filter(|e| e.position > through) {
            if let Some(eff) = e.effects() {
                self.dispatch_sends(a, e.position, &eff.sends, now);
            }
        }
        if loaded.head > through {
            self.advance_outbox(a, loaded.head);
        }
        // invoked children live inside their parent: rebuild them
        for log in invocations {
            self.start_child(a, &log.request, &log.events, now);
        }
        Ok(())
    }

    /// Moves the durable outbox mark of `a` forward to `n` (never back: another node sharing
    /// the store may have moved it further).
    fn mark_through(&self, a: &Address, n: u64) {
        let g = self.shared.storage.graph(a.node());
        let graph = GraphName::NamedNode(a.outbox_graph());
        let p = node(vt::DISPATCHED_THROUGH);
        let old: Vec<Quad> = crate::storage::graph_quads(g, &a.outbox_graph())
            .unwrap_or_default()
            .into_iter()
            .filter(|q| q.predicate == p)
            .collect();
        let current = old
            .iter()
            .filter_map(|q| match &q.object {
                Term::Literal(l) => l.value().parse::<u64>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        if current >= n {
            return;
        }
        let _ = g.remove_quads(&old);
        let _ = g.insert_quads(&[Quad::new(
            a.node().clone(),
            p,
            Literal::new_typed_literal(n.to_string(), xsd::INTEGER),
            graph,
        )]);
    }

    fn mark_fired(&self, scheduler: &str, timer: &str) {
        let Ok(a) = Address::parse(scheduler) else {
            return;
        };
        let _ = self
            .shared
            .storage
            .graph(a.node())
            .insert_quads(&[Quad::new(
                NamedNode::new_unchecked(timer),
                node(vt::DISPATCHED),
                Literal::new_typed_literal("true", xsd::BOOLEAN),
                a.outbox_graph(),
            )]);
    }

    /// One turn of virtual actor `a`.
    // @tg: implements:: [[openspec:actor-model#Turn Is One Macrostep]]
    // @tg: implements:: [[openspec:actor-model#Actor Failure Isolation]]
    // @tg: implements:: [[openspec:event-journal#Compare-And-Set Append]]
    // @tg: implements:: [[openspec:event-journal#Message Deduplication]]
    // @tg: implements:: [[openspec:event-journal#Inputs Only From The Journal]]
    // @tg: implements:: [[openspec:messaging#Duplicate Reply Suppression]]
    fn virtual_turn(&mut self, a: &Address, now: i64) -> bool {
        if let Some(progress) = self.placement_turn(a, now) {
            return progress;
        }
        if !self.actors.contains_key(a)
            && let Err(e) = self.activate(a, now)
        {
            self.errors.insert(a.clone(), e);
            self.park(a, now);
            return false;
        }
        if let Some(q) = self.migrations.get_mut(a)
            && let Some((to, m)) = q.pop_front()
        {
            self.self_migrate(a, &to, &m, now);
            return true;
        }
        let Some((mailbox, _)) = self.shared.local_mailbox(a) else {
            return false;
        };
        let Some(Activation::Virtual(mut va)) = self.actors.remove(a) else {
            return false;
        };
        let deferred = va.deferred();
        let Some(env) = mailbox.receive(&deferred) else {
            self.actors.insert(a.clone(), Activation::Virtual(va));
            return false;
        };
        va.last_active = now;
        // dedup and drops: acknowledged without a macrostep
        let drop_reason = self.drop_reason(&va, &env);
        if let Some(reason) = drop_reason {
            if let Some(dl) = reason {
                self.dead_letter(a, &env, dl, now);
            }
            let _ = mailbox.ack(&env.message_id);
            self.actors.insert(a.clone(), Activation::Virtual(va));
            return true;
        }
        let position = va.head + 1;
        let entry_iri = a.entry_iri(position);
        let input = InputRecord {
            event: Event {
                type_: EventType::External,
                ..env.event.clone()
            },
            message_id: env.message_id.clone(),
            correlation: env.correlation.clone(),
            timer: env.timer.clone(),
        };
        let (sends, invokes, ioproc) = self.ctx(a);
        let registry = self.registry.clone();
        let info = va.info().clone();
        let policy = info.unhandled.unwrap_or(UnhandledPolicy::DeadLetter);
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let ctx = Ctx {
                registry: registry.as_ref(),
                ioprocessors: ioproc,
                send_types: sends,
                invoke_types: invokes,
            };
            let (mut m, start, trace) = match va.machine.take() {
                Some(m) => (m, Vec::new(), None),
                None => {
                    let env = Environment {
                        now_ms: now,
                        seed: crate::address::seed_of(&entry_iri),
                    };
                    let (m, eff, tr) = Machine::start(a, info.clone(), &ctx, env)?;
                    (m, eff, Some(tr))
                }
            };
            let applied = m.apply(&input, position, now, policy, start, trace.as_ref());
            Ok::<_, MachineError>((m, applied))
        }));
        let (machine, applied) = match result {
            Ok(Ok((m, Ok(applied)))) => (m, applied),
            Ok(Ok((_, Err(MachineError::Terminated)))) => {
                self.dead_letter(a, &env, "terminated", now);
                let _ = mailbox.ack(&env.message_id);
                // the activation is stale (its machine was consumed): reload on next message
                self.stats.deactivations += 1;
                return true;
            }
            _ => {
                // a failed or panicking turn: nothing appended, message redelivered later
                mailbox.release(&env.message_id);
                self.stats.deactivations += 1;
                self.park(a, now);
                return true;
            }
        };
        let ct = va
            .chart_type
            .as_ref()
            .map(|(t, k)| (t.as_str(), k.as_str()));
        let new_state = machine.state_triples(ct);
        let delta = state_delta(&va.state, &new_state);
        let data = EntryData {
            actor: a.clone(),
            position,
            version: machine.info.version.as_str().to_owned(),
            message_id: env.message_id.clone(),
            timestamp_ms: now,
            digest: machine.digest(),
            body: EntryBody::Input {
                input,
                effects: applied.effects.clone(),
            },
        };
        if self.config.crash_at == Some(CrashPoint::BeforeAppend) {
            self.crash();
            return true;
        }
        if let Err(e) = self.append(a, va.head, &data, delta.clone()) {
            // compare-and-set lost, fenced or failed: step down, dispatch nothing
            mailbox.release(&env.message_id);
            self.stats.deactivations += 1;
            if !(e.is_conflict() || e.is_fenced()) {
                self.errors.insert(a.clone(), ActivationError::Append(e));
                self.park(a, now);
            }
            return true;
        }
        self.stats.turns += 1;
        if self.config.crash_at == Some(CrashPoint::AfterAppend) {
            self.crash();
            return true;
        }
        let _ = mailbox.ack(&env.message_id);
        va.head = position;
        va.seen.insert(env.message_id.clone());
        va.state = new_state;
        let snapshot = (self.config.snapshot_interval > 0
            && position % self.config.snapshot_interval == 0)
            .then(|| machine.snapshot(&a.snapshot_iri(position), position));
        va.machine = Some(machine);
        self.actors.insert(a.clone(), Activation::Virtual(va));
        if let Some(s) = snapshot {
            self.write_snapshot(a, &s);
        }
        self.after_append(
            a,
            &data,
            applied.invokes,
            applied.cancel_invokes,
            applied.migrations,
            &env,
            now,
        );
        true
    }

    /// `Some(reason)` when the message is acknowledged without a turn; the inner reason, when
    /// set, also records a dead letter.
    fn drop_reason(&self, va: &VirtualActivation, env: &Envelope) -> Option<Option<&'static str>> {
        if va.seen.contains(&env.message_id) {
            return Some(None);
        }
        let rt = va.machine.as_ref().map(|m| &m.rt);
        if let Some(t) = &env.timer
            && rt.is_some_and(|rt| rt.cancelled.contains(t))
        {
            return Some(None);
        }
        if let Some(c) = &env.correlation {
            let answered = rt.is_some_and(|rt| rt.answered.contains(c));
            let open = rt.is_some_and(|rt| rt.requests.contains_key(c));
            if env.event.name == vt::REPLY_EVENT && !open {
                return Some(if answered { None } else { Some("late-reply") });
            }
            if env.event.name == vt::TIMEOUT_EVENT && !open {
                return Some(None);
            }
        }
        if va
            .machine
            .as_ref()
            .is_some_and(|m| m.session.is_terminated())
        {
            return Some(Some("terminated"));
        }
        None
    }

    fn append(
        &self,
        a: &Address,
        expected: u64,
        data: &EntryData,
        delta: StateDelta,
    ) -> Result<(), AppendError> {
        let s = &self.shared.storage;
        let caps = s.journal().capabilities();
        let entry = data.to_entry();
        let mut meta = AppendMeta::none();
        if let Some(f) = &self.config.fencing {
            meta = meta.with_fencing(f.clone());
        }
        if caps.state_in_append && !delta.is_empty() {
            meta = meta.with_state(delta.clone());
        }
        s.journal()
            .append(a.node(), Position::new(expected), &[entry], &meta)?;
        let notes = data.annotations();
        if !notes.is_empty() {
            let _ = vertep_store::reification::write_annotations(s.graph(a.node()), &notes);
        }
        if !caps.state_in_append && !delta.is_empty() {
            let g = s.graph(a.node());
            let gn = GraphName::NamedNode(a.node().clone());
            let r: Vec<Quad> = delta
                .retract
                .iter()
                .map(|t| t.clone().in_graph(gn.clone()))
                .collect();
            let i: Vec<Quad> = delta
                .assert
                .iter()
                .map(|t| t.clone().in_graph(gn.clone()))
                .collect();
            let _ = g.remove_quads(&r);
            let _ = g.insert_quads(&i);
        }
        Ok(())
    }

    fn write_snapshot(&self, a: &Address, s: &SnapshotData) {
        let g = NamedNode::new_unchecked(&s.iri);
        let quads: Vec<Quad> = crate::journal::snapshot_triples(s)
            .into_iter()
            .map(|t| t.in_graph(g.clone()))
            .collect();
        let _ = self.shared.storage.graph(a.node()).insert_quads(&quads);
    }

    /// The transactional outbox and the other effects of an appended entry.
    // @tg: implements:: [[openspec:event-journal#Transactional Outbox]]
    #[allow(clippy::too_many_arguments)]
    fn after_append(
        &mut self,
        a: &Address,
        data: &EntryData,
        invokes: Vec<InvokeRequest>,
        cancel_invokes: Vec<String>,
        migrations: Vec<(String, String)>,
        env: &Envelope,
        now: i64,
    ) {
        let Some(eff) = data.effects() else { return };
        self.dispatch_sends(a, data.position, &eff.sends, now);
        if !eff.sends.is_empty() {
            self.advance_outbox(a, data.position);
        }
        for c in &eff.cancels {
            self.disarm(c);
        }
        for t in &eff.timers {
            self.arm(t.clone(), now, false);
        }
        if eff.unhandled == Some(UnhandledPolicy::DeadLetter) {
            self.dead_letter(a, env, "unhandled", now);
        }
        for id in cancel_invokes {
            self.cancel_child(&Address::invoked(a, &id));
        }
        for req in invokes {
            self.start_child(a, &req, &[], now);
        }
        if !migrations.is_empty() {
            self.migrations
                .entry(a.clone())
                .or_default()
                .extend(migrations);
            self.shared.mark_ready(a);
        }
    }

    fn communication_error(&self, to: &Address, about: &str, send_id: &str) {
        let mut ev = platform_event("error.communication", Value::Undefined);
        ev.sendid = Some(send_id.to_owned());
        let env = Envelope::new(format!("{about}/error"), to, ev);
        let _ = self.shared.deliver(env, SCXML_EVENT_PROCESSOR);
    }

    /// Dispatches the recorded sends of the entry of `from` at `position`; the sends a transport
    /// refused stay owed until a retry delivers them.
    fn dispatch_sends(&mut self, from: &Address, position: u64, sends: &[SendRecord], now: i64) {
        let refused = sends
            .iter()
            .filter(|s| !self.dispatch_send(from, position, s, now))
            .count();
        let o = self.outbox.entry(from.clone()).or_default();
        o.tip = o.tip.max(position);
        if refused > 0 {
            *o.owed.entry(position).or_default() += refused;
        }
    }

    /// Moves the durable outbox mark of `a` to `upto`, or to just before its first entry with
    /// an owed send ([[openspec:event-journal#Transactional Outbox]]: the mark passes only
    /// delivered sends, as in the Lean journal model's `outbox_safe`).
    fn advance_outbox(&mut self, a: &Address, upto: u64) {
        let o = self.outbox.entry(a.clone()).or_default();
        o.tip = o.tip.max(upto);
        let through = match o.owed.keys().next() {
            Some(&p) => p.saturating_sub(1).min(o.tip),
            None => o.tip,
        };
        if through > 0 {
            self.mark_through(a, through);
        }
    }

    /// A retried send of the entry of `from` at `position` was delivered (or dead-lettered).
    fn settle_send(&mut self, from: &Address, position: u64) {
        let Some(o) = self.outbox.get_mut(from) else {
            return;
        };
        if let Some(n) = o.owed.get_mut(&position) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                o.owed.remove(&position);
            }
        }
        let tip = o.tip;
        self.advance_outbox(from, tip);
    }

    /// Dispatches one recorded send of the entry of `from` at `position`: resolves its
    /// recipient and delivers it with its send IRI as message id; an unresolvable recipient
    /// becomes `error.communication` for the sender and a dead letter. A send its transport
    /// refuses is retried after the failure backoff; `false` then, as it is not delivered yet.
    fn dispatch_send(&mut self, from: &Address, position: u64, s: &SendRecord, now: i64) -> bool {
        let env = Envelope {
            message_id: s.iri.clone(),
            to: s.recipient.clone().unwrap_or_default(),
            event: s.event.clone(),
            correlation: s.correlation.clone(),
            timer: None,
        };
        let type_ = if s.type_.starts_with(vt::REQUEST_SEND_TYPE) {
            SCXML_EVENT_PROCESSOR
        } else {
            s.type_.as_str()
        };
        match self.shared.deliver(env.clone(), type_) {
            Ok(()) => {}
            Err(DeliveryError::Unresolvable(t)) => {
                self.communication_error(from, &s.iri, &s.send_id);
                let receiver = Address::parse(&t).unwrap_or_else(|_| from.clone());
                self.dead_letter(&receiver, &env, "unresolvable", now);
            }
            Err(DeliveryError::Transport(_)) => {
                // retried after the failure backoff, and owed by the outbox until delivered
                self.seq += 1;
                self.volatile.insert(
                    (now.saturating_add(self.config.failure_backoff_ms), self.seq),
                    Volatile::Resend(from.clone(), position, Box::new(s.clone())),
                );
                return false;
            }
        }
        true
    }

    /// Records a dead letter in the receiver's dead-letter graph.
    // @tg: implements:: [[openspec:messaging#Queryable Dead Letters]]
    fn dead_letter(&self, receiver: &Address, env: &Envelope, reason: &str, now: i64) {
        self.shared.dead_letter(receiver, env, reason, now);
    }
}

impl Shared {
    /// Records a dead letter in the receiver's dead-letter graph.
    fn dead_letter(&self, receiver: &Address, env: &Envelope, reason: &str, now: i64) {
        let g = receiver.dead_letter_graph();
        let d = receiver.dead_letter_iri(&env.message_id);
        let q = |p: &str, o: Term| Quad::new(d.clone(), node(p), o, g.clone());
        let mut quads = vec![
            q(vt::RDF_TYPE, node(vt::DEAD_LETTER).into()),
            q(vt::RECEIVER, receiver.node().clone().into()),
            q(
                vt::EVENT_NAME,
                Literal::new_simple_literal(&env.event.name).into(),
            ),
            q(
                vt::PAYLOAD,
                Literal::new_simple_literal(crate::json::to_text(&env.event.data)).into(),
            ),
            q(vt::REASON, Literal::new_simple_literal(reason).into()),
            q(
                vt::RECORDED_AT,
                Literal::new_typed_literal(now.to_string(), xsd::INTEGER).into(),
            ),
            q(
                vt::MESSAGE_ID,
                Literal::new_simple_literal(&env.message_id).into(),
            ),
        ];
        if let Some(o) = &env.event.origin {
            quads.push(q(vt::SENDER, Literal::new_simple_literal(o).into()));
        }
        let _ = self.storage.graph(receiver.node()).insert_quads(&quads);
    }
}

impl Node {
    /// The dead letters recorded for `receiver`, by SPARQL over its dead-letter graph.
    ///
    /// # Errors
    ///
    /// The store's query errors.
    pub fn dead_letters(&self, receiver: &Address) -> Result<Vec<DeadLetter>, StoreError> {
        let q = format!(
            "SELECT ?d ?sender ?name ?payload ?reason ?at WHERE {{ GRAPH {g} {{ ?d a <{dl}> ; \
             <{rcv}> {r} ; <{en}> ?name ; <{pl}> ?payload ; <{rs}> ?reason ; <{at}> ?at . \
             OPTIONAL {{ ?d <{snd}> ?sender }} }} }} ORDER BY ?at ?d",
            g = receiver.dead_letter_graph(),
            dl = vt::DEAD_LETTER,
            rcv = vt::RECEIVER,
            r = receiver.node(),
            en = vt::EVENT_NAME,
            pl = vt::PAYLOAD,
            rs = vt::REASON,
            at = vt::RECORDED_AT,
            snd = vt::SENDER,
        );
        let sol = select(self.shared.storage.graph(receiver.node()), &q)?;
        let mut out = Vec::new();
        for i in 0..sol.len() {
            let g = |v: &str| sol.get(i, v).and_then(text);
            out.push(DeadLetter {
                iri: g("d").unwrap_or_default(),
                receiver: receiver.as_str().to_owned(),
                sender: g("sender"),
                event: g("name").unwrap_or_default(),
                payload: g("payload").unwrap_or_default(),
                reason: g("reason").unwrap_or_default(),
                recorded_at: g("at").and_then(|x| x.parse().ok()).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    fn deactivate_idle(&mut self, now: i64) -> bool {
        let idle: Vec<Address> = self
            .actors
            .iter()
            .filter_map(|(a, act)| match act {
                Activation::Virtual(v)
                    if now.saturating_sub(v.last_active) >= self.config.idle_timeout_ms =>
                {
                    Some((a, v))
                }
                _ => None,
            })
            .filter(|(a, v)| {
                let busy = self
                    .shared
                    .local_mailbox(a)
                    .is_some_and(|(m, _)| m.has_deliverable(&v.deferred()));
                let children = v
                    .machine
                    .as_ref()
                    .is_some_and(|m| !m.rt.invocations.is_empty());
                !busy && !children && self.migrations.get(*a).is_none_or(|q| q.is_empty())
            })
            .map(|(a, _)| a.clone())
            .collect();
        for a in &idle {
            self.deactivate(a);
        }
        !idle.is_empty()
    }

    /// Deactivates `a`, keeping its mailbox: its state is recoverable from the journal.
    // @tg: implements:: [[openspec:actor-model#Virtual Actor Deactivation]]
    pub fn deactivate(&mut self, a: &Address) {
        if self.actors.remove(a).is_some() {
            self.stats.deactivations += 1;
            if let Some((m, _)) = self.shared.local_mailbox(a) {
                m.release_all();
            }
        }
    }

    /// The virtual actors active on this node.
    pub fn active_actors(&self) -> Vec<Address> {
        self.actors
            .iter()
            .filter(|(_, act)| matches!(act, Activation::Virtual(_)))
            .map(|(a, _)| a.clone())
            .collect()
    }

    /// Drains the virtual actors `which` selects, for a shard handover: runs until idle (every
    /// macrostep in progress finishes and its outbox is dispatched), then deactivates them.
    /// Returns the actors deactivated. Their messages that arrive afterwards follow the
    /// placement.
    pub fn drain(&mut self, which: &dyn Fn(&Address) -> bool) -> Vec<Address> {
        self.run_until_idle();
        let drained: Vec<Address> = self
            .active_actors()
            .into_iter()
            .filter(|a| which(a))
            .collect();
        for a in &drained {
            self.deactivate(a);
        }
        drained
    }

    /// Applies the placement before a turn of virtual actor `a`. `None` when `a` lives here
    /// (or the node has no placement); otherwise the actor is deactivated here, and its
    /// waiting messages either move to the node that hosts it or stay held.
    fn placement_turn(&mut self, a: &Address, now: i64) -> Option<bool> {
        let p = self.shared.placement.clone()?;
        match p.route(a, true, None) {
            Route::Local => None,
            Route::Hold => {
                self.deactivate(a);
                self.park(a, now);
                Some(false)
            }
            Route::Remote(remote) => {
                self.deactivate(a);
                let Some((m, _)) = self.shared.local_mailbox(a) else {
                    return Some(false);
                };
                let mut moved = false;
                for env in m.pending() {
                    let id = env.message_id.clone();
                    if remote.enqueue(env).is_ok() {
                        let _ = m.ack(&id);
                        moved = true;
                    } else {
                        self.park(a, now);
                        break;
                    }
                }
                Some(moved)
            }
        }
    }

    // ----- timers ---------------------------------------------------------------------------

    /// Arms a timer; with `catch_up`, a reminder whose occurrences are overdue fires once for
    /// all of them.
    fn arm(&mut self, rec: TimerRecord, now: i64, catch_up: bool) {
        self.disarm(&rec.iri);
        let mut occurrence = 0;
        if catch_up
            && let Some(r) = &rec.reminder
            && rec.due_ms <= now
            && r.period_ms > 0
        {
            occurrence = u64::try_from((now - rec.due_ms) / r.period_ms).unwrap_or(0);
        }
        let key = (rec.due_of(occurrence), rec.iri.clone());
        self.timer_keys.insert(rec.iri.clone(), key.clone());
        self.timers.insert(key, Armed { rec, occurrence });
    }

    fn disarm(&mut self, iri: &str) {
        if let Some(k) = self.timer_keys.remove(iri) {
            self.timers.remove(&k);
        }
    }

    /// Fires every due timer in due-time order, as messages with ids derived from the timer.
    // @tg: implements:: [[openspec:durable-timers#Timer Fires As Input]]
    // @tg: implements:: [[openspec:durable-timers#Timers Reactivate Virtual Actors]]
    // @tg: implements:: [[openspec:durable-timers#Missed Timers After Downtime]]
    fn fire_timers(&mut self, now: i64) -> bool {
        let mut progress = false;
        while let Some(((due, _), _)) = self.timers.first_key_value() {
            if *due > now {
                break;
            }
            let Some((_, armed)) = self.timers.pop_first() else {
                break;
            };
            progress = true;
            let rec = armed.rec;
            self.timer_keys.remove(&rec.iri);
            let env = Envelope {
                message_id: rec.message_id(armed.occurrence),
                to: rec.recipient.clone(),
                event: rec.event.clone(),
                correlation: rec.correlation.clone(),
                timer: Some(rec.iri.clone()),
            };
            match self.shared.deliver(env.clone(), SCXML_EVENT_PROCESSOR) {
                Ok(()) => {}
                Err(DeliveryError::Unresolvable(t)) => {
                    if let Ok(s) = Address::parse(&rec.actor) {
                        self.communication_error(&s, &rec.iri, &rec.send_id);
                        let receiver = Address::parse(&t).unwrap_or_else(|_| s.clone());
                        self.dead_letter(&receiver, &env, "unresolvable", now);
                    }
                }
                Err(DeliveryError::Transport(_)) => {
                    // try again after the failure backoff
                    let key = (
                        now.saturating_add(self.config.failure_backoff_ms),
                        rec.iri.clone(),
                    );
                    self.timer_keys.insert(rec.iri.clone(), key.clone());
                    self.timers.insert(
                        key,
                        Armed {
                            rec,
                            occurrence: armed.occurrence,
                        },
                    );
                    continue;
                }
            }
            if let Some(r) = &rec.reminder {
                // the next occurrence after now: missed occurrences collapse into this firing
                let mut next = armed.occurrence + 1;
                if r.period_ms > 0 && rec.due_of(next) <= now {
                    next = u64::try_from((now - rec.due_ms) / r.period_ms).unwrap_or(0) + 1;
                }
                let key = (rec.due_of(next), rec.iri.clone());
                self.timer_keys.insert(rec.iri.clone(), key.clone());
                self.timers.insert(
                    key,
                    Armed {
                        rec,
                        occurrence: next,
                    },
                );
            } else {
                self.mark_fired(&rec.actor, &rec.iri);
                if let Ok(s) = Address::parse(&rec.actor)
                    && s.as_str() != rec.recipient
                    && let Some(Activation::Virtual(v)) = self.actors.get_mut(&s)
                    && let Some(m) = &mut v.machine
                {
                    m.rt.timers.remove(&rec.iri);
                }
            }
        }
        progress
    }

    fn fire_volatile(&mut self, now: i64) -> bool {
        let mut progress = false;
        while let Some(((due, _), _)) = self.volatile.first_key_value() {
            if *due > now {
                break;
            }
            let Some((_, v)) = self.volatile.pop_first() else {
                break;
            };
            progress = true;
            match v {
                Volatile::Deliver(env) => {
                    if self
                        .shared
                        .deliver((*env).clone(), SCXML_EVENT_PROCESSOR)
                        .is_err()
                    {
                        self.seq += 1;
                        self.volatile.insert(
                            (now.saturating_add(self.config.failure_backoff_ms), self.seq),
                            Volatile::Deliver(env),
                        );
                    }
                }
                Volatile::Resend(from, position, s) => {
                    if self.dispatch_send(&from, position, &s, now) {
                        self.settle_send(&from, position);
                    }
                }
                Volatile::Restart(a, generation) => self.restart_child(&a, generation, now),
            }
        }
        progress
    }

    // ----- recovery -------------------------------------------------------------------------

    /// After a restart: arm pending timers, dispatch undispatched sends and deliver stored
    /// mailbox messages.
    fn recover(&mut self) {
        let now = self.now_ms();
        let storage = self.shared.storage.clone();
        let q_timers = format!(
            "SELECT ?env WHERE {{ GRAPH ?a {{ ?t <{actor}> ?a ; <{env}> ?env }} }}",
            actor = vt::ACTOR,
            env = vt::ENVELOPE
        );
        let q_fired = format!(
            "SELECT ?t WHERE {{ GRAPH ?g {{ ?t <{d}> ?x }} }}",
            d = vt::DISPATCHED
        );
        let mut fired = HashSet::new();
        let mut pending = Vec::new();
        for g in storage.graphs() {
            if let Ok(sol) = select(g, &q_fired) {
                fired.extend(sol.values("t").iter().filter_map(text));
            }
            if let Ok(sol) = select(g, &q_timers) {
                for t in sol.values("env") {
                    if let Some(rec) =
                        text(&t).and_then(|j| serde_json::from_str::<TimerRecord>(&j).ok())
                    {
                        pending.push(rec);
                    }
                }
            }
        }
        for rec in pending {
            if !fired.contains(&rec.iri) {
                self.arm(rec, now, true);
            }
        }
        // undispatched sends
        if let Ok(actors) = storage.journal().actors() {
            for n in actors {
                let Ok(a) = Address::parse(n.as_str()) else {
                    continue;
                };
                let head = storage.journal().head(&n).map(|h| h.get()).unwrap_or(0);
                let (through, _) = self.outbox_marks(&a);
                if head <= through {
                    continue;
                }
                let Ok(recs) = storage.journal().read(&n, Position::new(through + 1)) else {
                    continue;
                };
                for r in recs {
                    if let Ok(e) = EntryData::from_record(&a, &r)
                        && let Some(eff) = e.effects()
                    {
                        self.dispatch_sends(&a, e.position, &eff.sends, now);
                    }
                }
                self.advance_outbox(&a, head);
            }
        }
        for f in self.factories.clone() {
            for a in f.recover() {
                self.shared.mark_ready(&a);
            }
        }
    }

    // ----- invoked actors -------------------------------------------------------------------

    fn is_virtual(&self, src: &str) -> bool {
        self.shared.charts().type_of(src).is_some()
    }

    /// Starts the child of `parent` for invocation `req`, replaying `events` (recorded in the
    /// parent's journal) without delivering its outputs.
    // @tg: implements:: [[openspec:actor-model#Invoked Actor Lifecycle]]
    fn start_child(
        &mut self,
        parent: &Address,
        req: &InvokeRequest,
        events: &[(Event, i64)],
        now: i64,
    ) {
        let child = Address::invoked(parent, &req.invokeid);
        let fail = |node: &mut Node, cause: String| {
            let ev = platform_event(
                "error.execution",
                Value::Object(vec![
                    ("invokeid".into(), Value::String(req.invokeid.clone())),
                    ("cause".into(), Value::String(cause)),
                ]),
            );
            let env = Envelope::new(format!("{child}/start-error"), parent, ev);
            let _ = node.shared.deliver(env, SCXML_EVENT_PROCESSOR);
        };
        if let Some(src) = &req.src
            && self.is_virtual(src)
        {
            fail(
                self,
                format!("{src} is a virtual actor and cannot be invoked"),
            );
            return;
        }
        let start = ActorStart {
            address: child.clone(),
            parent: parent.clone(),
            invokeid: req.invokeid.clone(),
            src: req.src.clone(),
            content: req.content.clone(),
            params: req.params.clone(),
        };
        let make: Arc<dyn Fn() -> Result<Box<dyn Actor>, ActorFailure> + Send + Sync> =
            if is_scxml_invoke(&req.type_) {
                let info = match self.child_chart(req) {
                    Ok(i) => i,
                    Err(e) => {
                        fail(self, e);
                        return;
                    }
                };
                let registry = self.registry.clone();
                let io = self.shared.transports.ioprocessors(&child);
                let data: Vec<(String, Value)> = match &req.params {
                    Value::Object(kv) => kv.clone(),
                    _ => Vec::new(),
                };
                let addr = child.clone();
                Arc::new(move || {
                    Ok(Box::new(ChartActor::new(
                        addr.clone(),
                        info.clone(),
                        registry.clone(),
                        data.clone(),
                        io.clone(),
                    )) as Box<dyn Actor>)
                })
            } else if let Some(f) = self.kinds.get(&req.type_).cloned() {
                let start = start.clone();
                Arc::new(move || f(&start))
            } else {
                fail(self, format!("no actor kind handles {}", req.type_));
                return;
            };
        let actor = match make() {
            Ok(a) => a,
            Err(e) => {
                fail(self, e.0);
                return;
            }
        };
        let policy = self
            .actors
            .get(parent)
            .and_then(|p| match p {
                Activation::Virtual(v) => {
                    Some(v.info().restart_policy(&req.state, index_of(v, req)))
                }
                _ => None,
            })
            .unwrap_or_default();
        self.shared.hosted_mut().children.insert(child.clone());
        self.seq += 1;
        let generation = self.seq;
        self.actors.insert(
            child.clone(),
            Activation::Child(Box::new(ChildActivation {
                parent: parent.clone(),
                invokeid: req.invokeid.clone(),
                make,
                actor,
                busy: false,
                generation,
                policy,
                failures: Vec::new(),
                out_seq: 0,
                suppress: !events.is_empty(),
            })),
        );
        let mut start_ev = Event::named(START_EVENT, EventType::External);
        start_ev.data = req.params.clone();
        self.child_input(&child, start_ev, None, None, now);
        for (e, ts) in events {
            self.child_input(&child, e.clone(), None, None, *ts);
        }
        if let Some(Activation::Child(c)) = self.actors.get_mut(&child) {
            c.suppress = false;
        }
    }

    fn child_chart(&mut self, req: &InvokeRequest) -> Result<Arc<ChartInfo>, String> {
        if let Some(src) = &req.src {
            let charts = self.shared.charts();
            return charts
                .latest(src)
                .or_else(|| charts.get(src))
                .ok_or_else(|| format!("no chart registered for {src}"));
        }
        let Some(Value::String(xml)) = &req.content else {
            return Err("an scxml invoke needs src or inline content".into());
        };
        let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
        let parsed = vertep_scxml::read(xml, &opts).map_err(|e| format!("{e:?}"))?;
        let info = ChartInfo::new(parsed.chart, None).map_err(|e| e.to_string())?;
        Ok(self.shared.charts_mut().insert(info))
    }

    fn cancel_child(&mut self, child: &Address) {
        if self.actors.remove(child).is_some() {
            self.shared.hosted_mut().children.remove(child);
            lock(&self.shared.mailboxes).remove(child);
        }
    }

    /// Runs one input through a child, catching panics.
    fn child_input(
        &mut self,
        child: &Address,
        event: Event,
        message_id: Option<String>,
        origin_correlation: Option<(Option<String>, Option<String>)>,
        now: i64,
    ) {
        let Some(Activation::Child(c)) = self.actors.get_mut(child) else {
            return;
        };
        let input = ActorInput {
            event: event.clone(),
            message_id: message_id.clone().unwrap_or_default(),
            received_at_ms: now,
        };
        let generation = c.generation;
        let turn = std::panic::catch_unwind(AssertUnwindSafe(|| c.actor.turn(input)))
            .unwrap_or_else(|_| Turn::Ready(Err(ActorFailure("panic".into()))));
        let (origin, correlation) = origin_correlation.unwrap_or((event.origin.clone(), None));
        match turn {
            Turn::Ready(result) => {
                self.child_done(
                    Completion {
                        address: child.clone(),
                        generation,
                        message_id,
                        origin,
                        correlation,
                        result,
                    },
                    now,
                );
            }
            Turn::Pending(fut) => {
                c.busy = true;
                let shared = self.shared.clone();
                let address = child.clone();
                shared.jobs.fetch_add(1, Ordering::SeqCst);
                let job = async move {
                    let result = AssertUnwindSafe(fut)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| Err(ActorFailure("panic".into())));
                    shared.complete(Completion {
                        address,
                        generation,
                        message_id,
                        origin,
                        correlation,
                        result,
                    });
                };
                self.shared.shell.spawn.spawn(Box::pin(job));
            }
        }
    }

    fn child_next(&mut self, child: &Address, now: i64) -> bool {
        let Some(Activation::Child(c)) = self.actors.get(child) else {
            return false;
        };
        if c.busy {
            return false;
        }
        let Some((m, _)) = self.shared.local_mailbox(child) else {
            return false;
        };
        let Some(env) = m.receive(&[]) else {
            return false;
        };
        let origin = env.event.origin.clone();
        self.child_input(
            child,
            env.event,
            Some(env.message_id),
            Some((origin, env.correlation)),
            now,
        );
        true
    }

    fn drain_completions(&mut self, now: i64) -> bool {
        let done: Vec<Completion> = lock(&self.shared.completions).drain(..).collect();
        let progress = !done.is_empty();
        for c in done {
            match self.actors.get(&c.address) {
                Some(Activation::Child(_)) => self.child_done(c, now),
                Some(Activation::Worker(_)) => self.worker_done(c, now),
                _ => {}
            }
        }
        progress
    }

    fn child_done(&mut self, c: Completion, now: i64) {
        let Some(Activation::Child(ca)) = self.actors.get_mut(&c.address) else {
            return;
        };
        if ca.generation != c.generation {
            return;
        }
        ca.busy = false;
        if let Some(id) = &c.message_id
            && let Some((m, _)) = self.shared.local_mailbox(&c.address)
        {
            let _ = m.ack(id);
        }
        let parent = ca.parent.clone();
        let invokeid = ca.invokeid.clone();
        let suppress = ca.suppress;
        match c.result {
            Ok(out) => {
                if !suppress {
                    for o in out.sends {
                        self.child_send(
                            &c.address,
                            &parent,
                            &invokeid,
                            o,
                            c.origin.as_deref(),
                            c.correlation.as_deref(),
                            now,
                        );
                    }
                }
                if let Some(data) = out.done {
                    let restart = matches!(
                        self.actors.get(&c.address),
                        Some(Activation::Child(x)) if x.policy.mode == RestartMode::Always
                    );
                    if restart {
                        self.child_failed(&c.address, "completed".into(), now);
                    } else {
                        if !suppress {
                            let mut ev = Event::named(
                                format!("done.invoke.{invokeid}"),
                                EventType::External,
                            );
                            ev.data = data;
                            ev.invokeid = Some(invokeid.clone());
                            ev.origin = Some(c.address.as_str().to_owned());
                            ev.origintype = Some(SCXML_EVENT_PROCESSOR.to_owned());
                            ev.originverified = false;
                            let env = Envelope::new(
                                format!("{}/{}/done", c.address, c.generation),
                                &parent,
                                ev,
                            );
                            let _ = self.shared.deliver(env, SCXML_EVENT_PROCESSOR);
                        }
                        self.cancel_child(&c.address);
                    }
                } else {
                    self.shared.mark_ready(&c.address);
                }
            }
            Err(f) => self.child_failed(&c.address, f.0, now),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn child_send(
        &mut self,
        child: &Address,
        parent: &Address,
        invokeid: &str,
        o: Outgoing,
        origin: Option<&str>,
        correlation: Option<&str>,
        now: i64,
    ) {
        let mut ev = o.event;
        ev.origin = Some(child.as_str().to_owned());
        ev.origintype = Some(SCXML_EVENT_PROCESSOR.to_owned());
        ev.originverified = false;
        let to = match o.target {
            OutTarget::Parent => {
                ev.invokeid = Some(invokeid.to_owned());
                Some(parent.clone())
            }
            OutTarget::Origin => origin.and_then(|x| Address::parse(x).ok()),
            OutTarget::Itself => Some(child.clone()),
            OutTarget::Address(a) => Some(a),
            OutTarget::Invalid(_) => None,
        };
        let generation = match self.actors.get_mut(child) {
            Some(Activation::Child(c)) => {
                c.out_seq += 1;
                format!("{}/{}", c.generation, c.out_seq)
            }
            _ => {
                self.seq += 1;
                format!("w/{}", self.seq)
            }
        };
        let Some(to) = to else {
            self.communication_error(
                child,
                &format!("{child}/{generation}"),
                ev.sendid.as_deref().unwrap_or(""),
            );
            return;
        };
        let mut env = Envelope::new(
            format!("{child}/{}-{generation}", self.shared.nonce),
            &to,
            ev,
        );
        if env.event.name == vt::REPLY_EVENT {
            env.correlation = correlation.map(str::to_owned);
        }
        if o.delay_ms > 0 {
            self.seq += 1;
            self.volatile.insert(
                (
                    now.saturating_add(i64::try_from(o.delay_ms).unwrap_or(i64::MAX)),
                    self.seq,
                ),
                Volatile::Deliver(Box::new(env)),
            );
        } else if let Err(DeliveryError::Unresolvable(_)) =
            self.shared.deliver(env.clone(), SCXML_EVENT_PROCESSOR)
        {
            self.dead_letter(&to, &env, "unresolvable", now);
        }
    }

    /// Applies the child's `vt:restart` policy after a failure.
    fn child_failed(&mut self, child: &Address, cause: String, now: i64) {
        let Some(Activation::Child(ca)) = self.actors.get_mut(child) else {
            return;
        };
        let p = ca.policy;
        let give_up = if p.mode == RestartMode::Never {
            true
        } else {
            ca.failures.retain(|t| *t > now.saturating_sub(p.window_ms));
            ca.failures.push(now);
            ca.failures.len() as u32 > p.limit
        };
        let parent = ca.parent.clone();
        let invokeid = ca.invokeid.clone();
        let generation = ca.generation;
        if give_up {
            let ev = platform_event(
                "error.execution",
                Value::Object(vec![
                    ("invokeid".into(), Value::String(invokeid.clone())),
                    ("cause".into(), Value::String(cause)),
                ]),
            );
            let mut ev = ev;
            ev.invokeid = Some(invokeid);
            let env = Envelope::new(format!("{child}/{generation}/failed"), &parent, ev);
            let _ = self.shared.deliver(env, SCXML_EVENT_PROCESSOR);
            self.cancel_child(child);
        } else {
            let n = ca.failures.len() as u32;
            ca.busy = true;
            let at = now.saturating_add(p.backoff(n));
            self.seq += 1;
            self.volatile
                .insert((at, self.seq), Volatile::Restart(child.clone(), generation));
            self.stats.restarts.push((child.clone(), at));
        }
    }

    fn restart_child(&mut self, child: &Address, generation: u64, now: i64) {
        let Some(Activation::Child(ca)) = self.actors.get_mut(child) else {
            return;
        };
        if ca.generation != generation {
            return;
        }
        match (ca.make)() {
            Ok(actor) => {
                self.seq += 1;
                ca.actor = actor;
                ca.generation = self.seq;
                ca.busy = false;
                let ev = Event::named(START_EVENT, EventType::External);
                self.child_input(child, ev, None, None, now);
            }
            Err(e) => self.child_failed(child, e.0, now),
        }
    }

    // ----- stateless workers ----------------------------------------------------------------

    fn worker_turns(&mut self, a: &Address, now: i64) -> bool {
        if !self.actors.contains_key(a) {
            let Some(spec) = self.shared.hosted().workers.get(a).cloned() else {
                return false;
            };
            *self.stats.activations.entry(a.clone()).or_default() += 1;
            self.actors
                .insert(a.clone(), Activation::Worker(WorkerPool { spec, busy: 0 }));
        }
        let Some((m, _)) = self.shared.local_mailbox(a) else {
            return false;
        };
        let mut progress = false;
        loop {
            let Some(Activation::Worker(pool)) = self.actors.get_mut(a) else {
                return progress;
            };
            if pool.busy >= pool.spec.pool {
                return progress;
            }
            let Some(env) = m.receive(&[]) else {
                return progress;
            };
            pool.busy += 1;
            let busy = pool.busy;
            let c = self.stats.worker_concurrency.entry(a.clone()).or_default();
            *c = (*c).max(busy);
            let mut actor = FnActor::new(a.clone(), pool.spec.handler.clone());
            let input = ActorInput {
                event: env.event.clone(),
                message_id: env.message_id.clone(),
                received_at_ms: now,
            };
            let shared = self.shared.clone();
            let completion = |result| Completion {
                address: a.clone(),
                generation: 0,
                message_id: Some(env.message_id.clone()),
                origin: env.event.origin.clone(),
                correlation: env.correlation.clone(),
                result,
            };
            match actor.turn(input) {
                Turn::Ready(r) => {
                    shared.jobs.fetch_add(1, Ordering::SeqCst);
                    shared.complete(completion(r));
                }
                Turn::Pending(fut) => {
                    shared.jobs.fetch_add(1, Ordering::SeqCst);
                    let mut c = completion(Ok(TurnOutput::default()));
                    let s2 = shared.clone();
                    self.shared.shell.spawn.spawn(Box::pin(async move {
                        c.result = AssertUnwindSafe(fut)
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|_| Err(ActorFailure("panic".into())));
                        s2.complete(c);
                    }));
                }
            }
            progress = true;
        }
    }

    fn worker_done(&mut self, c: Completion, now: i64) {
        if let Some(Activation::Worker(pool)) = self.actors.get_mut(&c.address) {
            pool.busy = pool.busy.saturating_sub(1);
        }
        if let (Some(id), Some((m, _))) = (&c.message_id, self.shared.local_mailbox(&c.address)) {
            let _ = m.ack(id);
        }
        if let Ok(out) = c.result {
            for o in out.sends {
                let mut ev = o.event;
                ev.origin = Some(c.address.as_str().to_owned());
                ev.origintype = Some(SCXML_EVENT_PROCESSOR.to_owned());
                ev.originverified = false;
                let to = match o.target {
                    OutTarget::Origin => c.origin.as_deref().and_then(|x| Address::parse(x).ok()),
                    OutTarget::Address(a) => Some(a),
                    _ => None,
                };
                let Some(to) = to else { continue };
                self.seq += 1;
                let mut env = Envelope::new(
                    format!("{}/{}-{}", c.address, self.shared.nonce, self.seq),
                    &to,
                    ev,
                );
                if env.event.name == vt::REPLY_EVENT {
                    env.correlation = c.correlation.clone();
                }
                if let Err(DeliveryError::Unresolvable(_)) =
                    self.shared.deliver(env.clone(), SCXML_EVENT_PROCESSOR)
                {
                    self.dead_letter(&to, &env, "unresolvable", now);
                }
            }
        }
        self.shared.mark_ready(&c.address);
    }

    // ----- migrations, forks and time travel ------------------------------------------------

    fn self_migrate(&mut self, a: &Address, to: &str, migration: &str, now: i64) {
        let current = match self.actors.get(a) {
            Some(Activation::Virtual(v)) => v.info().version.as_str().to_owned(),
            _ => return,
        };
        if current == to {
            return;
        }
        if let Err(e) = self.migrate_at(a, migration, now) {
            let ev = platform_event(
                "error.execution",
                Value::Object(vec![
                    ("migration".into(), Value::String(migration.to_owned())),
                    ("cause".into(), Value::String(e.to_string())),
                ]),
            );
            let env = Envelope::new(format!("{a}/migration-refused/{}", self.seq), a, ev);
            self.seq += 1;
            let _ = self.shared.deliver(env, SCXML_EVENT_PROCESSOR);
        }
    }

    /// Migrates actor `a` with the declared migration `migration`: checks that every
    /// reachable configuration of the old version is mapped, transforms the datamodel and
    /// appends one migration entry holding the snapshot before, the migration IRI and the
    /// snapshot after. Nothing is appended when any step fails.
    ///
    /// # Errors
    ///
    /// [`MigrateError`].
    // @tg: implements:: [[openspec:chart-versioning#Explicit Migration]]
    // @tg: implements:: [[openspec:chart-versioning#Migration Compatibility Check]]
    pub fn migrate(&mut self, a: &Address, migration: &str) -> Result<u64, MigrateError> {
        let now = self.now_ms();
        self.migrate_at(a, migration, now)
    }

    fn migrate_at(&mut self, a: &Address, migration: &str, now: i64) -> Result<u64, MigrateError> {
        if !self.actors.contains_key(a) {
            self.activate(a, now).map_err(MigrateError::Activation)?;
        }
        let m = self
            .shared
            .charts()
            .migration(migration)
            .ok_or_else(|| MigrateError::UnknownMigration(migration.to_owned()))?;
        let to = self.shared.charts().get(m.to.as_str()).ok_or_else(|| {
            MigrateError::Activation(ActivationError::MissingVersion {
                actor: a.clone(),
                version: m.to.as_str().to_owned(),
            })
        })?;
        let Some(Activation::Virtual(va)) = self.actors.get(a) else {
            return Err(MigrateError::NotStarted);
        };
        let Some(old) = &va.machine else {
            return Err(MigrateError::NotStarted);
        };
        if old.info.version != m.from {
            return Err(MigrateError::WrongVersion {
                actual: old.info.version.as_str().to_owned(),
                expected: m.from.as_str().to_owned(),
            });
        }
        let unmapped = unmapped_configurations(&m, &old.info, &to)
            .map_err(MigrateError::TooManyConfigurations)?;
        if !unmapped.is_empty() {
            return Err(MigrateError::Unmapped(unmapped));
        }
        let cfg: BTreeSet<String> = old.active().into_iter().collect();
        let new_cfg = m
            .map(&cfg, &to)
            .ok_or_else(|| MigrateError::Unmapped(vec![cfg.clone()]))?;
        let value = (m.transform)(old.dm.snapshot()).map_err(MigrateError::TransformFailed)?;
        let position = va.head + 1;
        let e = a.entry_iri(position);
        let before = old.snapshot(
            &NamedNode::new_unchecked(format!("{}/before", e.as_str())),
            va.head,
        );
        let (sends, invokes, ioproc) = self.ctx(a);
        let ctx = Ctx {
            registry: self.registry.as_ref(),
            ioprocessors: ioproc,
            send_types: sends,
            invoke_types: invokes,
        };
        let session = migrated_session(&old.session, &new_cfg, &to);
        let mut probe = SnapshotData {
            iri: format!("{}/after", e.as_str()),
            actor: a.as_str().to_owned(),
            position,
            version: to.version.as_str().to_owned(),
            digest: String::new(),
            session: serde_json::to_string(&session).unwrap_or_default(),
            datamodel: crate::json::to_text(&value),
            runtime: serde_json::to_string(&old.rt).unwrap_or_default(),
        };
        let new = Machine::from_snapshot(a, to.clone(), &ctx, &probe)
            .map_err(|x| MigrateError::TransformFailed(x.to_string()))?;
        probe.digest = new.digest();
        let ct = va
            .chart_type
            .as_ref()
            .map(|(t, k)| (t.as_str(), k.as_str()));
        let new_state = new.state_triples(ct);
        let delta = state_delta(&va.state, &new_state);
        let data = EntryData {
            actor: a.clone(),
            position,
            version: to.version.as_str().to_owned(),
            message_id: format!("{a}/migration/{position}"),
            timestamp_ms: now,
            digest: probe.digest.clone(),
            body: EntryBody::Migration {
                migration: m.iri.clone(),
                from: m.from.as_str().to_owned(),
                before,
                after: probe,
            },
        };
        self.append(a, va.head, &data, delta)
            .map_err(MigrateError::Append)?;
        if let Some(Activation::Virtual(va)) = self.actors.get_mut(a) {
            va.machine = Some(new);
            va.head = position;
            va.seen.insert(data.message_id.clone());
            va.state = new_state;
        }
        Ok(position)
    }

    /// Forks `source` at `position` under `new`: the new actor's journal starts with a fork
    /// entry referencing the source and holding its state there; the source is unchanged.
    ///
    /// # Errors
    ///
    /// [`MigrateError::Exists`] when `new` has a journal, or replay errors.
    // @tg: implements:: [[openspec:event-journal#Journal Read Access]]
    // @tg: implements:: [[openspec:chart-versioning#Fork Before Migration]]
    pub fn fork(
        &mut self,
        source: &Address,
        position: u64,
        new: &Address,
    ) -> Result<(), MigrateError> {
        let j = self.shared.storage.journal();
        if j.head(new.node()).map(|h| h.get()).unwrap_or(0) > 0 {
            return Err(MigrateError::Exists(new.clone()));
        }
        let loaded = self
            .load(source, Some(position))
            .map_err(MigrateError::Activation)?;
        let Some(mut m) = loaded.machine else {
            return Err(MigrateError::NotStarted);
        };
        m.address = new.clone();
        m.session.sessionid = new.as_str().to_owned();
        let e = new.entry_iri(1);
        let snap = m.snapshot(
            &NamedNode::new_unchecked(format!("{}/state", e.as_str())),
            position,
        );
        let state = m.state_triples(None);
        let delta = state_delta(&HashSet::new(), &state);
        let data = EntryData {
            actor: new.clone(),
            position: 1,
            version: m.info.version.as_str().to_owned(),
            message_id: format!("{new}/fork"),
            timestamp_ms: self.now_ms(),
            digest: snap.digest.clone(),
            body: EntryBody::Fork {
                source: source.as_str().to_owned(),
                position,
                snapshot: snap,
            },
        };
        self.append(new, 0, &data, delta)
            .map_err(MigrateError::Append)
    }

    /// The state of `a` after journal position `position`, rebuilt by replay; the live actor
    /// and the journal are not touched.
    ///
    /// # Errors
    ///
    /// [`ActivationError`] when the journal cannot be replayed.
    pub fn state_at(&mut self, a: &Address, position: u64) -> Result<ActorState, ActivationError> {
        let loaded = self.load(a, Some(position))?;
        Ok(actor_state(loaded.machine.as_ref(), loaded.head))
    }

    /// The current state of `a`: the live activation's, or rebuilt by replay.
    ///
    /// # Errors
    ///
    /// [`ActivationError`] when the journal cannot be replayed.
    pub fn state(&mut self, a: &Address) -> Result<ActorState, ActivationError> {
        if let Some(Activation::Virtual(v)) = self.actors.get(a) {
            return Ok(actor_state(v.machine.as_ref(), v.head));
        }
        let loaded = self.load(a, None)?;
        Ok(actor_state(loaded.machine.as_ref(), loaded.head))
    }

    /// Whether `a` would accept `event` in its current configuration: the event enables at
    /// least one transition or an active state defers it. Guards are evaluated on a copy of the
    /// datamodel; nothing changes and nothing is journaled. An actor without a journal has no
    /// configuration yet and accepts nothing.
    ///
    /// # Errors
    ///
    /// [`ActivationError`] when the journal cannot be replayed.
    pub fn can(&mut self, a: &Address, event: &Event) -> Result<bool, ActivationError> {
        if let Some(Activation::Virtual(v)) = self.actors.get(a)
            && let Some(m) = &v.machine
        {
            return Ok(m.can(self.registry.as_ref(), event));
        }
        let loaded = self.load(a, None)?;
        Ok(loaded
            .machine
            .as_ref()
            .is_some_and(|m| m.can(self.registry.as_ref(), event)))
    }

    /// The decoded journal of `a`.
    ///
    /// # Errors
    ///
    /// Store or decoding errors.
    pub fn journal(&self, a: &Address) -> Result<Vec<EntryData>, ActivationError> {
        let recs = self
            .shared
            .storage
            .journal()
            .read(a.node(), Position::ZERO)
            .map_err(ActivationError::Store)?;
        recs.iter()
            .map(|r| EntryData::from_record(a, r).map_err(ActivationError::Decode))
            .collect()
    }

    /// The pending durable timers armed on this node, in due order.
    pub fn pending_timers(&self) -> Vec<(i64, TimerRecord)> {
        self.timers
            .iter()
            .map(|((due, _), a)| (*due, a.rec.clone()))
            .collect()
    }

    /// The datamodel registry.
    pub fn datamodels(&self) -> &Arc<dyn DatamodelRegistry + Send + Sync> {
        &self.registry
    }

    /// A fresh, unique id with `prefix`.
    pub fn fresh_id(&self, prefix: &str) -> String {
        self.shared.fresh_id(prefix)
    }

    /// The digest of `text` as short hex (exposed for tests and tools).
    pub fn short_hex(text: &str) -> String {
        hex(text.as_bytes())
    }
}

fn index_of(v: &VirtualActivation, req: &InvokeRequest) -> usize {
    v.machine
        .as_ref()
        .and_then(|m| {
            m.session
                .invocations
                .iter()
                .find(|i| i.invokeid == req.invokeid)
                .map(|i| i.index)
        })
        .unwrap_or(0)
}

fn actor_state(m: Option<&Machine>, head: u64) -> ActorState {
    match m {
        Some(m) => ActorState {
            position: head,
            version: m.info.version.as_str().to_owned(),
            configuration: m.active(),
            datamodel: m.dm.snapshot(),
            terminated: m.session.is_terminated(),
        },
        None => ActorState {
            position: head,
            version: String::new(),
            configuration: Vec::new(),
            datamodel: Value::Undefined,
            terminated: false,
        },
    }
}

/// The delta from `old` to `new`, in a stable order.
fn state_delta(old: &HashSet<Triple>, new: &HashSet<Triple>) -> StateDelta {
    let mut retract: Vec<Triple> = old.difference(new).cloned().collect();
    let mut assert: Vec<Triple> = new.difference(old).cloned().collect();
    retract.sort_by_cached_key(ToString::to_string);
    assert.sort_by_cached_key(ToString::to_string);
    StateDelta { retract, assert }
}
