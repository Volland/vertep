//! The cluster inside the actor runtime.
//!
//! A [`ClusterNode`] connects one `vertep-runtime` [`Node`](vertep_runtime::Node) to the
//! cluster:
//!
//! - it is the node's [`Placement`]: a virtual actor is local while this node holds an
//!   unexpired lease on its shard, remote while the routing table names a live owner, and
//!   held in the local mailbox otherwise; requests to a worker type go to the activation the
//!   worker pool picks;
//! - [`ClusterStorage`] is the node's [`Storage`]: one store per shard, every append fenced
//!   by the node's lease on the shard (and replicated to standbys when configured);
//! - [`ClusterEventProcessor`] is registered as a runtime transport and SCXML Event I/O
//!   Processor, so `<send type="cluster">` names it and `_ioprocessors` lists it;
//! - outbound messages become [`Frame`]s sent over a [`Wire`] (QUIC, or [`MemoryWire`] in
//!   process) with retries until acknowledged or the delivery deadline, after which the
//!   sending actor gets `error.communication`;
//! - inbound frames ([`ClusterNode::receive`]) are deduplicated, fenced (recorded in the
//!   receiver's dead-letter graph), delivered to the owner's mailbox once it has room, or
//!   forwarded to the owner with a redirect for the sender.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use vertep_core::datamodel::Value;
use vertep_runtime::actor::platform_event;
use vertep_runtime::mailbox::{Envelope, Mailbox, MailboxError};
use vertep_runtime::transport::{Transport, TransportError};
use vertep_runtime::vocab::REPLY_EVENT;
use vertep_runtime::{Address, Node, NodeHandle, Placement, Route, Storage};
use vertep_store::oxrdf::NamedNode;
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphStore, JournalCapabilities, JournalRecord, JournalStore,
    Position, StoreError,
};

use crate::fencing::{
    DeadLetters, EpochFilter, Fence, FencedMessage, ShardJournal, ShardJournalError,
};
use crate::frame::{Frame, RejectKind, Reply};
use crate::membership::{Drain, Membership};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::{ShardId, assignment};
use crate::replication::{ReplicatedJournal, StandbyLink, may_take};
use crate::routing::{ActivationGate, DedupCache, Inbound, RoutingTable};
use crate::workers::{Assignment, Outcome, Submit, WorkerPool, WorkerType};

/// The type IRI of the cluster Event I/O Processor.
pub const CLUSTER_PROCESSOR: &str = "https://w3id.org/vertep/vt#ClusterEventProcessor";

/// A boxed `Send` future.
pub type WireFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// How frames reach other nodes.
pub trait Wire: Send + Sync {
    /// Sends `frame` from `from` to `to` and returns the receiver's reply.
    fn send(&self, from: &NodeId, to: &NodeId, frame: Frame) -> WireFuture<Result<Reply, String>>;
}

/// The settings of a clustered node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterConfig {
    /// How often a message may be forwarded between nodes before it fails.
    pub hop_limit: u8,
    /// The receiving mailbox bound: an inbound frame waits (and its stream with it) while
    /// the target's mailbox holds this many messages.
    pub mailbox_bound: usize,
    /// How many accepted message identifiers the receiver remembers.
    pub dedup_capacity: usize,
    /// How long an outbound message is retried before the sender gets a delivery error.
    pub delivery_deadline: Duration,
    /// The pause between retries.
    pub retry_backoff: Duration,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        ClusterConfig {
            hop_limit: 4,
            mailbox_bound: 1024,
            dedup_capacity: 65_536,
            delivery_deadline: Duration::from_secs(30),
            retry_backoff: Duration::from_millis(50),
        }
    }
}

#[derive(Default)]
struct Workers {
    pools: BTreeMap<String, WorkerPool<Envelope>>,
    /// The worker types this node runs activations of.
    hosted: BTreeSet<String>,
    /// Requests in progress by correlation id: (worker type, request id).
    by_correlation: BTreeMap<String, (String, String)>,
}

/// What to do once the worker lock is released.
enum Action {
    Dispatch(Assignment<Envelope>),
    Report(Envelope, String),
}

/// One runtime node's membership in the cluster. See the module documentation.
pub struct ClusterNode {
    meta: ClusterMeta,
    membership: Arc<dyn Membership>,
    config: ClusterConfig,
    wire: OnceLock<Arc<dyn Wire>>,
    handle: OnceLock<NodeHandle>,
    routing: Mutex<RoutingTable>,
    epochs: Arc<EpochFilter>,
    dedup: Mutex<DedupCache>,
    /// The epoch under which this node last held each shard.
    fences: Mutex<BTreeMap<ShardId, u64>>,
    /// The virtual actors this node has hosted (their output carries a fence).
    virtuals: Mutex<HashSet<String>>,
    workers: Mutex<Workers>,
    view_version: Mutex<u64>,
    me: Weak<ClusterNode>,
}

impl fmt::Debug for ClusterNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterNode")
            .field("node", self.membership.local())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ClusterNode {
    /// The cluster side of the node `membership` speaks for.
    pub fn new(membership: Arc<dyn Membership>, config: ClusterConfig) -> Arc<Self> {
        Self::with_epochs(membership, config, Arc::new(EpochFilter::new()))
    }

    /// The same, sharing `epochs` with a transport bound before the node existed.
    pub fn with_epochs(
        membership: Arc<dyn Membership>,
        config: ClusterConfig,
        epochs: Arc<EpochFilter>,
    ) -> Arc<Self> {
        let meta = membership.meta().clone();
        Arc::new_cyclic(|me| ClusterNode {
            routing: Mutex::new(RoutingTable::new(meta.clone())),
            meta,
            membership,
            config,
            wire: OnceLock::new(),
            handle: OnceLock::new(),
            epochs,
            dedup: Mutex::new(DedupCache::new(config.dedup_capacity)),
            fences: Mutex::new(BTreeMap::new()),
            virtuals: Mutex::new(HashSet::new()),
            workers: Mutex::new(Workers::default()),
            view_version: Mutex::new(0),
            me: me.clone(),
        })
    }

    /// This node.
    pub fn local(&self) -> &NodeId {
        self.membership.local()
    }

    /// The membership.
    pub fn membership(&self) -> &Arc<dyn Membership> {
        &self.membership
    }

    /// The highest epoch seen per shard, shared with a QUIC transport's receiving side.
    pub fn epochs(&self) -> Arc<EpochFilter> {
        self.epochs.clone()
    }

    /// Sets the wire outbound frames travel on (once).
    pub fn set_wire(&self, wire: Arc<dyn Wire>) {
        let _ = self.wire.set(wire);
    }

    /// Attaches the runtime node's handle, once the node is built.
    pub fn attach(&self, handle: NodeHandle) {
        let _ = self.handle.set(handle);
    }

    /// This node as the runtime's placement.
    pub fn placement(self: &Arc<Self>) -> Arc<dyn Placement> {
        self.clone()
    }

    /// The cluster Event I/O Processor, to register with `NodeBuilder::transport`.
    pub fn processor(&self) -> Arc<dyn Transport> {
        Arc::new(ClusterEventProcessor)
    }

    /// The dead-letter sink recording fenced frames in the runtime's dead-letter graph.
    pub fn dead_letters(self: &Arc<Self>) -> Arc<dyn DeadLetters> {
        Arc::new(RuntimeDeadLetters(self.clone()))
    }

    /// The routing table's owner of `shard`.
    pub fn known_owner(&self, shard: ShardId) -> Option<(NodeId, u64)> {
        lock(&self.routing)
            .owner(shard)
            .map(|o| (o.node.clone(), o.epoch))
    }

    /// Declares a worker type the cluster balances requests for, with the members that run
    /// activations of it (from configuration). When `nodes` includes this node, its runtime
    /// must have the worker registered at the type's address.
    // @tg: implements:: [[openspec:worker-pools#Pool Placement Across Nodes]]
    pub fn add_worker_type(&self, ty: WorkerType, nodes: impl IntoIterator<Item = NodeId>) {
        let mut w = lock(&self.workers);
        let iri = ty.iri.clone();
        let mut pool = WorkerPool::new(ty, self.local().clone());
        for n in nodes {
            if &n == self.local() {
                w.hosted.insert(iri.clone());
            }
            pool.declare(n);
        }
        w.pools.insert(iri, pool);
    }

    /// The activations of worker type `iri`: (activation, node, requests in progress).
    pub fn worker_activations(&self, iri: &str) -> Vec<(crate::ActivationId, NodeId, usize)> {
        lock(&self.workers)
            .pools
            .get(iri)
            .map(WorkerPool::activations)
            .unwrap_or_default()
    }

    /// Discards every idle worker activation (memory pressure); returns how many.
    pub fn discard_idle_workers(&self) -> usize {
        lock(&self.workers)
            .pools
            .values_mut()
            .map(WorkerPool::discard_idle)
            .sum()
    }

    /// Learns the current membership view: the routing table follows it, the epoch filter
    /// learns its owners' epochs, and worker requests in progress on members that failed are
    /// retried (idempotent types) or reported to their callers.
    pub fn refresh(&self) {
        let view = self.membership.view();
        {
            let mut v = lock(&self.view_version);
            if view.version <= *v {
                return;
            }
            *v = view.version;
        }
        lock(&self.routing).apply_view(&view);
        for (s, o) in &view.owners {
            self.epochs.observe(Fence {
                shard: *s,
                epoch: o.epoch,
            });
        }
        let mut actions = Vec::new();
        {
            let mut w = lock(&self.workers);
            for pool in w.pools.values_mut() {
                let gone: Vec<NodeId> =
                    pool.nodes().filter(|n| !view.is_live(n)).cloned().collect();
                for n in gone {
                    for o in pool.node_failed(&n) {
                        actions.push(match o {
                            Outcome::Retried(a) => Action::Dispatch(a),
                            Outcome::RetryQueued(_) => continue,
                            Outcome::Failed {
                                payload, failure, ..
                            } => Action::Report(payload, failure.to_string()),
                        });
                    }
                }
            }
        }
        self.run(actions);
    }

    /// Acquires the shards the deterministic assignment of the current view gives this node,
    /// skipping any shard that was owned before and whose journal this node does not hold
    /// (`holds_journal`): with a replication factor of one such a shard waits for its owner.
    /// Returns the shards acquired.
    // @tg: implements:: [[openspec:shard-placement#Journal Replication To Standbys]]
    pub fn claim(&self, holds_journal: &dyn Fn(ShardId) -> bool) -> Vec<ShardId> {
        self.refresh();
        let view = self.membership.view();
        let mut out = Vec::new();
        for (s, n) in assignment(&view, self.meta.shard_count) {
            if &n != self.local() || self.membership.held(s).is_some() {
                continue;
            }
            let ever_owned = lock(&self.routing).owner(s).is_some();
            if may_take(ever_owned, holds_journal(s)) && self.membership.acquire(s).is_ok() {
                out.push(s);
            }
        }
        self.refresh();
        out
    }

    fn handle(&self) -> Option<&NodeHandle> {
        self.handle.get()
    }

    fn fence_of(&self, sender: &str) -> Option<Fence> {
        if sender.is_empty() || !lock(&self.virtuals).contains(sender) {
            return None;
        }
        let shard = self.meta.shard_of(sender);
        if let Some(l) = self.membership.held(shard) {
            lock(&self.fences).insert(shard, l.epoch);
            return Some(Fence {
                shard,
                epoch: l.epoch,
            });
        }
        lock(&self.fences)
            .get(&shard)
            .map(|e| Fence { shard, epoch: *e })
    }

    fn frame_of(&self, env: &Envelope, hops: u8) -> Frame {
        let sender = env.event.origin.clone().unwrap_or_default();
        Frame {
            id: env.message_id.clone(),
            fence: self.fence_of(&sender),
            sender,
            target: env.to.clone(),
            hops,
            event: env.event.name.clone(),
            data: serde_json::to_vec(env).unwrap_or_default(),
        }
    }

    /// Delivers a delivery error for `env` to its sender, as `error.communication` carrying the
    /// request's correlation id.
    fn report(&self, env: &Envelope, reason: &str) {
        let (Some(h), Some(origin)) = (self.handle(), env.event.origin.as_deref()) else {
            return;
        };
        let Ok(sender) = Address::parse(origin) else {
            return;
        };
        let mut ev = platform_event(
            "error.communication",
            Value::Object(vec![
                ("target".into(), Value::String(env.to.clone())),
                ("reason".into(), Value::String(reason.to_owned())),
            ]),
        );
        ev.sendid = env.event.sendid.clone();
        let mut err = Envelope::new(format!("{}/delivery-error", env.message_id), &sender, ev);
        err.correlation = env.correlation.clone();
        let _ = h.send_envelope(err);
    }

    fn run(&self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Dispatch(a) => self.dispatch(a),
                Action::Report(env, reason) => self.report(&env, &reason),
            }
        }
    }

    fn dispatch(&self, a: Assignment<Envelope>) {
        if &a.node == self.local() {
            if let Some(h) = self.handle() {
                let _ = h.deliver_here(a.payload);
            }
        } else {
            let _ = self.send_remote(a.node, a.payload);
        }
    }

    /// Sends `env` to `node`, retrying until it is acknowledged or the deadline passes.
    // @tg: implements:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication]]
    // @tg: implements:: [[openspec:cluster-transport#Redirect On Stale Routing]]
    fn send_remote(&self, node: NodeId, env: Envelope) -> Result<(), String> {
        let Some(wire) = self.wire.get().cloned() else {
            return Err("no cluster wire".into());
        };
        let rt = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let me = self.me.upgrade().ok_or("the cluster node is gone")?;
        let frame = self.frame_of(&env, 0);
        rt.spawn(async move {
            let start = tokio::time::Instant::now();
            loop {
                let last = match wire.send(me.local(), &node, frame.clone()).await {
                    Ok(Reply::Ack { .. }) => return,
                    Ok(Reply::Forwarded { redirect, .. }) => {
                        lock(&me.routing).apply_redirect(&redirect);
                        return;
                    }
                    // the receiver recorded it as a dead letter: the sender is stale
                    Ok(Reply::Rejected {
                        kind: RejectKind::Fenced,
                        ..
                    }) => return,
                    Ok(Reply::Rejected {
                        kind: RejectKind::Unavailable,
                        message,
                        ..
                    }) => message,
                    Ok(Reply::Rejected { message, .. }) => {
                        me.report(&env, &message);
                        return;
                    }
                    Err(e) => e,
                };
                if start.elapsed() + me.config.retry_backoff > me.config.delivery_deadline {
                    me.report(
                        &env,
                        &format!("not acknowledged before the deadline: {last}"),
                    );
                    return;
                }
                tokio::time::sleep(me.config.retry_backoff).await;
            }
        });
        Ok(())
    }

    /// Counts a reply or error that answers a worker request, freeing its activation.
    fn observe(&self, env: &Envelope) {
        let Some(c) = env.correlation.as_deref() else {
            return;
        };
        if env.event.name != REPLY_EVENT && !env.event.name.starts_with("error.") {
            return;
        }
        let assigned = {
            let mut w = lock(&self.workers);
            let Some((iri, request)) = w.by_correlation.remove(c) else {
                return;
            };
            w.pools
                .get_mut(&iri)
                .map(|p| p.complete(&request))
                .unwrap_or_default()
        };
        self.run(assigned.into_iter().map(Action::Dispatch).collect());
    }

    fn route_worker(&self, to: &Address, env: Option<&Envelope>) -> Option<Route> {
        let mut actions = Vec::new();
        let route = {
            let mut w = lock(&self.workers);
            let w = &mut *w;
            let pool = w.pools.get_mut(to.as_str())?;
            let Some(env) = env else {
                return Some(Route::Local);
            };
            match pool.submit(env.message_id.clone(), env.clone()) {
                Submit::Assigned(a) => {
                    let route = if &a.node == self.local() {
                        Route::Local
                    } else {
                        Route::Remote(Arc::new(RemoteMailbox {
                            node: self.me.upgrade()?,
                            to: a.node.clone(),
                        }))
                    };
                    match &env.correlation {
                        Some(c) => {
                            w.by_correlation
                                .insert(c.clone(), (to.as_str().to_owned(), a.request.clone()));
                        }
                        None => actions
                            .extend(pool.complete(&a.request).into_iter().map(Action::Dispatch)),
                    }
                    route
                }
                Submit::Queued => Route::Remote(Arc::new(Accepted)),
                Submit::Refused(r) => {
                    actions.push(Action::Report(env.clone(), r.to_string()));
                    Route::Remote(Arc::new(Accepted))
                }
            }
        };
        self.run(actions);
        Some(route)
    }

    /// Handles a frame another node sent, after deduplication and fencing: delivers it to an
    /// actor this node hosts, holds it, or forwards it to the shard's owner.
    // @tg: implements:: [[openspec:cluster-transport#QUIC Cluster Transport]]
    // @tg: implements:: [[openspec:cluster-transport#Flow Control And Backpressure]]
    // @tg: implements:: [[openspec:shard-placement#Message Forwarding To Shard Owner]]
    // @tg: implements:: [[openspec:shard-placement#Activation On Shard Owner Only]]
    pub async fn receive(&self, _from: NodeId, frame: Frame) -> Reply {
        let id = frame.id.clone();
        let Ok(mut env) = serde_json::from_slice::<Envelope>(&frame.data) else {
            return Reply::Rejected {
                id,
                kind: RejectKind::Malformed,
                message: "the frame does not carry a runtime envelope".into(),
            };
        };
        let Some(h) = self.handle().cloned() else {
            return Reply::Rejected {
                id,
                kind: RejectKind::Unavailable,
                message: "the runtime is not attached".into(),
            };
        };
        self.refresh();
        self.observe(&env);
        env.event.origintype = Some(CLUSTER_PROCESSOR.to_owned());
        let worker = lock(&self.workers).hosted.contains(&frame.target);
        let gate = ActivationGate::new(self.membership.clone(), self.config.hop_limit);
        let decision = if worker {
            Inbound::Wait(self.meta.shard_of(&frame.target))
        } else {
            gate.inbound(&frame.target, frame.hops)
        };
        match decision {
            Inbound::Deliver(_) | Inbound::Wait(_) => {
                if let Ok(target) = Address::parse(&env.to) {
                    while h.pending(&target) >= self.config.mailbox_bound {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                }
                match h.deliver_here(env) {
                    Ok(()) => Reply::Ack { id },
                    Err(e) => Reply::Rejected {
                        id,
                        kind: RejectKind::Unavailable,
                        message: e.to_string(),
                    },
                }
            }
            Inbound::Forward { to, hops, redirect } => {
                let Some(wire) = self.wire.get().cloned() else {
                    return Reply::Rejected {
                        id,
                        kind: RejectKind::Unavailable,
                        message: "no cluster wire".into(),
                    };
                };
                let mut fwd = frame;
                fwd.hops = hops;
                match wire.send(self.local(), &to, fwd).await {
                    Ok(Reply::Ack { .. } | Reply::Forwarded { .. }) => {
                        Reply::Forwarded { id, redirect }
                    }
                    Ok(Reply::Rejected { kind, message, .. }) => {
                        Reply::Rejected { id, kind, message }
                    }
                    Err(message) => Reply::Rejected {
                        id,
                        kind: RejectKind::Unavailable,
                        message,
                    },
                }
            }
            Inbound::HopLimit { shard, hops } => Reply::Rejected {
                id,
                kind: RejectKind::HopLimit,
                message: format!("{shard}: forwarded {hops} times without reaching its owner"),
            },
        }
    }

    /// [`ClusterNode::receive`] after the checks a QUIC transport's receiving side makes:
    /// a duplicate is acknowledged without delivery, a fenced frame is recorded as a dead
    /// letter and rejected. The in-memory wire uses this.
    // @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
    pub async fn receive_checked(&self, from: NodeId, frame: Frame) -> Reply {
        if lock(&self.dedup).contains(&frame.target, &frame.id) {
            return Reply::Ack { id: frame.id };
        }
        if let Some(fence) = frame.fence {
            self.refresh();
            let dead = RuntimeDeadLetters(match self.me.upgrade() {
                Some(me) => me,
                None => {
                    return Reply::Rejected {
                        id: frame.id,
                        kind: RejectKind::Unavailable,
                        message: "the cluster node is gone".into(),
                    };
                }
            });
            if !self.epochs.admit_or_dead_letter(
                &frame.id,
                &frame.sender,
                &frame.target,
                &frame.event,
                fence,
                &dead,
            ) {
                return Reply::Rejected {
                    id: frame.id,
                    kind: RejectKind::Fenced,
                    message: format!("epoch {} of {} is superseded", fence.epoch, fence.shard),
                };
            }
        }
        let (target, id) = (frame.target.clone(), frame.id.clone());
        let reply = self.receive(from, frame).await;
        if matches!(reply, Reply::Ack { .. } | Reply::Forwarded { .. }) {
            lock(&self.dedup).first_time(&target, &id);
        }
        reply
    }
}

impl Placement for ClusterNode {
    // @tg: implements:: [[openspec:cluster-transport#Routing Via The Shard Table]]
    // @tg: implements:: [[openspec:shard-placement#Activation On Shard Owner Only]]
    // @tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]
    // @tg: implements:: [[openspec:worker-pools#Stateless Worker Activations]]
    // @tg: implements:: [[openspec:worker-pools#Results Journaled By The Caller]]
    fn route(&self, to: &Address, virtual_actor: bool, env: Option<&Envelope>) -> Route {
        if let Some(env) = env {
            self.observe(env);
        }
        if let Some(r) = self.route_worker(to, env) {
            return r;
        }
        if !virtual_actor {
            return Route::Local;
        }
        let shard = self.meta.shard_of(to.as_str());
        if let Some(lease) = self.membership.held(shard) {
            lock(&self.fences).insert(shard, lease.epoch);
            lock(&self.virtuals).insert(to.as_str().to_owned());
            return Route::Local;
        }
        self.refresh();
        let view = self.membership.view();
        let owner = lock(&self.routing).owner(shard).cloned();
        match (owner, self.me.upgrade()) {
            (Some(o), Some(me)) if &o.node != self.local() && view.is_live(&o.node) => {
                Route::Remote(Arc::new(RemoteMailbox {
                    node: me,
                    to: o.node,
                }))
            }
            _ => Route::Hold,
        }
    }
}

/// A mailbox that carries messages to another node.
struct RemoteMailbox {
    node: Arc<ClusterNode>,
    to: NodeId,
}

impl Mailbox for RemoteMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        self.node
            .send_remote(self.to.clone(), env)
            .map_err(MailboxError)
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

/// A mailbox that accepts a message another component keeps (a worker pool's queue).
struct Accepted;

impl Mailbox for Accepted {
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

/// The cluster Event I/O Processor (`https://w3id.org/vertep/vt#ClusterEventProcessor`,
/// alias `cluster`): delivers into the mailbox the placement chose, which for an actor on
/// another node carries the message there. Events it delivers name it as their origin type.
// @tg: implements:: [[openspec:cluster-transport#QUIC Cluster Transport]]
#[derive(Clone, Copy, Debug, Default)]
pub struct ClusterEventProcessor;

impl Transport for ClusterEventProcessor {
    fn type_iri(&self) -> &str {
        CLUSTER_PROCESSOR
    }

    fn aliases(&self) -> Vec<String> {
        vec!["cluster".to_owned()]
    }

    fn deliver(&self, mailbox: &dyn Mailbox, mut env: Envelope) -> Result<(), TransportError> {
        env.event.origintype = Some(CLUSTER_PROCESSOR.to_owned());
        mailbox
            .enqueue(env)
            .map_err(|e| TransportError(e.to_string()))
    }
}

/// Records fenced frames in the runtime's dead-letter graph, reason `fenced`.
struct RuntimeDeadLetters(Arc<ClusterNode>);

impl DeadLetters for RuntimeDeadLetters {
    fn fenced(&self, m: FencedMessage) {
        let (Some(h), Ok(target)) = (self.0.handle(), Address::parse(&m.target)) else {
            return;
        };
        let mut ev = vertep_core::datamodel::Event::named(
            m.event.clone(),
            vertep_core::datamodel::EventType::External,
        );
        ev.origin = (!m.sender.is_empty()).then(|| m.sender.clone());
        let env = Envelope::new(m.id.clone(), &target, ev);
        h.record_dead_letter(&target, &env, "fenced");
    }
}

/// Drains the actors of one shard on a runtime node, for a leave or a planned handover:
/// every macrostep in progress finishes and its outbox is dispatched, then the shard's actors
/// are deactivated; messages arriving afterwards follow the placement to the next owner.
// @tg: implements:: [[openspec:cluster-membership#Graceful Leave With Drain]]
pub struct NodeDrain<'a> {
    node: &'a mut Node,
    meta: ClusterMeta,
}

impl fmt::Debug for NodeDrain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeDrain").finish_non_exhaustive()
    }
}

impl<'a> NodeDrain<'a> {
    /// The drain of `node`, whose actors are placed by `meta`.
    pub fn new(node: &'a mut Node, meta: ClusterMeta) -> Self {
        NodeDrain { node, meta }
    }
}

impl Drain for NodeDrain<'_> {
    fn drain_shard(&mut self, shard: ShardId) -> Result<(), String> {
        let meta = self.meta.clone();
        self.node
            .drain(&|a: &Address| meta.shard_of(a.as_str()) == shard);
        Ok(())
    }
}

/// An in-process wire between [`ClusterNode`]s, with fault injection: nodes can be cut off
/// in either direction, and frames can be dropped one way.
#[derive(Default)]
pub struct MemoryWire {
    nodes: Mutex<BTreeMap<NodeId, Weak<ClusterNode>>>,
    cut: Mutex<BTreeSet<(NodeId, NodeId)>>,
    down: Mutex<BTreeSet<NodeId>>,
    replies_lost: Mutex<BTreeSet<(NodeId, NodeId)>>,
}

impl fmt::Debug for MemoryWire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryWire").finish_non_exhaustive()
    }
}

impl MemoryWire {
    /// An empty wire.
    pub fn new() -> Arc<Self> {
        Arc::new(MemoryWire::default())
    }

    /// Connects `node` to the wire and sets the wire as its outbound path.
    pub fn connect(self: &Arc<Self>, node: &Arc<ClusterNode>) {
        lock(&self.nodes).insert(node.local().clone(), Arc::downgrade(node));
        node.set_wire(self.clone());
    }

    /// Crashes or restores `node`: no frame reaches it or leaves it while it is down.
    pub fn set_down(&self, node: &NodeId, down: bool) {
        let mut d = lock(&self.down);
        if down {
            d.insert(node.clone());
        } else {
            d.remove(node);
        }
    }

    /// Cuts or restores the direction `from` to `to`.
    pub fn set_cut(&self, from: &NodeId, to: &NodeId, cut: bool) {
        let mut c = lock(&self.cut);
        if cut {
            c.insert((from.clone(), to.clone()));
        } else {
            c.remove(&(from.clone(), to.clone()));
        }
    }

    /// Loses or restores the replies from `to` back to `from`: frames arrive, their
    /// acknowledgements do not.
    pub fn set_replies_lost(&self, from: &NodeId, to: &NodeId, lost: bool) {
        let mut c = lock(&self.replies_lost);
        if lost {
            c.insert((from.clone(), to.clone()));
        } else {
            c.remove(&(from.clone(), to.clone()));
        }
    }
}

impl Wire for MemoryWire {
    fn send(&self, from: &NodeId, to: &NodeId, frame: Frame) -> WireFuture<Result<Reply, String>> {
        let blocked = {
            let d = lock(&self.down);
            d.contains(from)
                || d.contains(to)
                || lock(&self.cut).contains(&(from.clone(), to.clone()))
        };
        let target = lock(&self.nodes).get(to).and_then(Weak::upgrade);
        let lost = lock(&self.replies_lost).contains(&(from.clone(), to.clone()));
        let from = from.clone();
        let to = to.clone();
        Box::pin(async move {
            if blocked {
                return Err(format!("{to} is unreachable"));
            }
            let Some(target) = target else {
                return Err(format!("{to} is not connected"));
            };
            let reply = target.receive_checked(from, frame).await;
            if lost {
                return Err("the reply was lost".into());
            }
            Ok(reply)
        })
    }
}

/// The QUIC wire: frames go over a [`crate::transport::ClusterTransport`] to the endpoint
/// each member advertises in the membership view.
#[cfg(feature = "quic")]
pub struct QuicWire {
    transport: Arc<crate::transport::ClusterTransport>,
    membership: Arc<dyn Membership>,
}

#[cfg(feature = "quic")]
impl fmt::Debug for QuicWire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicWire").finish_non_exhaustive()
    }
}

#[cfg(feature = "quic")]
impl QuicWire {
    /// Frames over `transport`, to the endpoints in `membership`'s view.
    pub fn new(
        transport: Arc<crate::transport::ClusterTransport>,
        membership: Arc<dyn Membership>,
    ) -> Arc<Self> {
        Arc::new(QuicWire {
            transport,
            membership,
        })
    }
}

#[cfg(feature = "quic")]
impl Wire for QuicWire {
    fn send(&self, _from: &NodeId, to: &NodeId, frame: Frame) -> WireFuture<Result<Reply, String>> {
        let addr = self
            .membership
            .view()
            .member(to)
            .and_then(|m| m.endpoint.parse::<std::net::SocketAddr>().ok());
        let transport = self.transport.clone();
        let to = to.clone();
        Box::pin(async move {
            let addr = addr.ok_or_else(|| format!("no endpoint for {to}"))?;
            transport
                .send(&to, addr, &frame)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// The receiving side of the QUIC transport for a [`ClusterNode`], attached once the node
/// exists (the transport is bound first, so its address can go into the membership): it
/// delivers frames the transport has already deduplicated and fenced, and records fenced
/// frames in the runtime's dead-letter graph.
#[cfg(feature = "quic")]
#[derive(Default)]
pub struct NodeFrameHandler(OnceLock<Arc<ClusterNode>>);

#[cfg(feature = "quic")]
impl fmt::Debug for NodeFrameHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeFrameHandler").finish_non_exhaustive()
    }
}

#[cfg(feature = "quic")]
impl NodeFrameHandler {
    /// A handler with no node yet.
    pub fn new() -> Arc<Self> {
        Arc::new(NodeFrameHandler::default())
    }

    /// Attaches the node (once).
    pub fn attach(&self, node: Arc<ClusterNode>) {
        let _ = self.0.set(node);
    }
}

#[cfg(feature = "quic")]
impl crate::transport::FrameHandler for NodeFrameHandler {
    fn handle(&self, from: NodeId, frame: Frame) -> crate::transport::BoxFuture<'static, Reply> {
        let node = self.0.get().cloned();
        Box::pin(async move {
            match node {
                Some(node) => node.receive(from, frame).await,
                None => Reply::Rejected {
                    id: frame.id,
                    kind: RejectKind::Unavailable,
                    message: "the node is starting".into(),
                },
            }
        })
    }
}

#[cfg(feature = "quic")]
impl DeadLetters for NodeFrameHandler {
    fn fenced(&self, message: FencedMessage) {
        if let Some(node) = self.0.get() {
            RuntimeDeadLetters(node.clone()).fenced(message);
        }
    }
}

enum Backend<S> {
    Plain(ShardJournal<S, dyn Membership>),
    Replicated(ReplicatedJournal<S, dyn Membership>),
}

impl<S: JournalStore> Backend<S> {
    fn journal(&self) -> &dyn JournalStore {
        match self {
            Backend::Plain(j) => j,
            Backend::Replicated(j) => j,
        }
    }

    fn store(&self) -> &S {
        match self {
            Backend::Plain(j) => j.store(),
            Backend::Replicated(j) => j.inner().store(),
        }
    }
}

/// A node's storage in a cluster: one store per shard, each actor's journal and graphs in the
/// store of its shard, every append fenced by this node's lease on the shard and, for the
/// shards given standbys, acknowledged only once enough standbys stored it.
// @tg: implements:: [[openspec:shard-placement#One Journal File Per Shard]]
// @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
pub struct ClusterStorage<S> {
    meta: ClusterMeta,
    shards: Vec<Backend<S>>,
}

impl<S> fmt::Debug for ClusterStorage<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterStorage")
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

impl<S: JournalStore + GraphStore> ClusterStorage<S> {
    /// The storage over `stores`, one per shard in shard order, fenced by `membership`.
    ///
    /// # Errors
    ///
    /// [`ShardJournalError::NoFencing`] when a store does not enforce fencing.
    ///
    /// # Panics
    ///
    /// When the number of stores is not the shard count.
    pub fn new(membership: Arc<dyn Membership>, stores: Vec<S>) -> Result<Self, ShardJournalError> {
        Self::with_standbys(membership, stores, |_| Vec::new(), 0)
    }

    /// The same, with `standbys(shard)` receiving each shard's appends and an append
    /// acknowledged once `required` of them stored it (shards with no standby are not
    /// replicated).
    ///
    /// # Errors
    ///
    /// [`ShardJournalError::NoFencing`] when a store does not enforce fencing.
    ///
    /// # Panics
    ///
    /// When the number of stores is not the shard count.
    pub fn with_standbys(
        membership: Arc<dyn Membership>,
        stores: Vec<S>,
        standbys: impl Fn(ShardId) -> Vec<Arc<dyn StandbyLink>>,
        required: usize,
    ) -> Result<Self, ShardJournalError> {
        let meta = membership.meta().clone();
        assert_eq!(
            stores.len(),
            meta.shard_count.get() as usize,
            "one store per shard"
        );
        let mut shards = Vec::with_capacity(stores.len());
        for (i, store) in stores.into_iter().enumerate() {
            let shard = ShardId::new(i as u32);
            let j = ShardJournal::new(store, shard, membership.clone())?;
            let links = standbys(shard);
            shards.push(if links.is_empty() {
                Backend::Plain(j)
            } else {
                Backend::Replicated(ReplicatedJournal::new(j, links, required))
            });
        }
        Ok(ClusterStorage { meta, shards })
    }

    /// The store of `shard`.
    pub fn store(&self, shard: ShardId) -> &S {
        self.shards[shard.index()].store()
    }

    /// Whether this node's store of `shard` holds any journal.
    pub fn holds_journal(&self, shard: ShardId) -> bool {
        self.store(shard).actors().is_ok_and(|a| !a.is_empty())
    }

    fn backend(&self, actor: &str) -> &Backend<S> {
        &self.shards[self.meta.shard_of(actor).index()]
    }
}

impl<S: JournalStore + GraphStore> JournalStore for ClusterStorage<S> {
    fn capabilities(&self) -> JournalCapabilities {
        self.shards[0].journal().capabilities()
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.backend(actor.as_str())
            .journal()
            .append(actor, expected, entries, meta)
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.backend(actor.as_str()).journal().read(actor, from)
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.backend(actor.as_str()).journal().head(actor)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        let mut out = Vec::new();
        for s in &self.shards {
            out.extend(s.journal().actors()?);
        }
        Ok(out)
    }
}

impl<S: JournalStore + GraphStore + 'static> Storage for ClusterStorage<S> {
    fn journal(&self) -> &dyn JournalStore {
        self
    }

    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore {
        self.backend(actor.as_str()).store()
    }

    fn graphs(&self) -> Vec<&dyn GraphStore> {
        self.shards
            .iter()
            .map(|s| s.store() as &dyn GraphStore)
            .collect()
    }
}
