//! Stateless worker pools across a cluster.
//!
//! A worker type (an address such as `urn:vertep:worker:summarize`) is served by any number
//! of interchangeable activations, created on demand on the members that declare the type and
//! discarded at will. A [`WorkerPool`] is the dispatcher of one worker type on one node: it
//! knows the activations it created, the requests in progress on each, and its bounded queue,
//! and it decides where the next request runs by the type's [`Policy`]. It is sans-IO: the
//! runtime glue delivers what it assigns and reports completions and failures back.
//!
//! Workers never take a shard lease and never touch a journal. The result of a request made
//! by a virtual actor travels back as an ordinary reply event, which the caller journals as
//! its input, so replaying the caller never calls the worker again.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::meta::NodeId;

/// How a worker type spreads requests over its activations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:worker-pools#Load Balancing Policies]]
pub enum Policy {
    /// Take the activations with free capacity in turn.
    RoundRobin,
    /// Take the activation with the fewest requests in progress.
    LeastOutstanding,
    /// Take an activation on the sender's own node when one has free capacity; otherwise
    /// the activation with the fewest requests in progress.
    PreferLocal,
}

/// The declaration of a worker type: its policy and limits.
#[derive(Clone, Debug, PartialEq, Eq)]
// @tg: implements:: [[openspec:worker-pools#Worker Concurrency Limits]]
pub struct WorkerType {
    /// The worker address every request is sent to.
    pub iri: String,
    /// The balancing policy.
    pub policy: Policy,
    /// The most activations of this type on one node.
    pub max_activations_per_node: usize,
    /// The most requests in progress on one activation.
    pub max_in_flight: usize,
    /// The most requests waiting in the pool queue.
    pub queue_bound: usize,
    /// Whether a request may be run twice: only then is a request whose activation crashed
    /// retried, once, on another activation.
    pub idempotent: bool,
}

impl WorkerType {
    /// A type at `iri` with one activation per node, one request at a time per activation, a
    /// queue of 64, round robin, not idempotent.
    pub fn new(iri: impl Into<String>) -> Self {
        WorkerType {
            iri: iri.into(),
            policy: Policy::RoundRobin,
            max_activations_per_node: 1,
            max_in_flight: 1,
            queue_bound: 64,
            idempotent: false,
        }
    }

    /// The same type with `policy`.
    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// The same type with these limits.
    pub fn with_limits(
        mut self,
        max_activations_per_node: usize,
        max_in_flight: usize,
        queue_bound: usize,
    ) -> Self {
        self.max_activations_per_node = max_activations_per_node.max(1);
        self.max_in_flight = max_in_flight.max(1);
        self.queue_bound = queue_bound;
        self
    }

    /// The same type, declared idempotent.
    pub fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }
}

/// An activation of a worker type, numbered by the dispatcher that created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActivationId(u64);

impl ActivationId {
    /// The number.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ActivationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "activation {}", self.0)
    }
}

/// A request placed on an activation: deliver `payload` to `node`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment<P> {
    /// The request identifier (the message identifier).
    pub request: String,
    /// The activation that serves it.
    pub activation: ActivationId,
    /// The node the activation runs on.
    pub node: NodeId,
    /// Whether the activation was created for this request.
    pub created: bool,
    /// The request itself.
    pub payload: P,
}

/// What became of a submitted request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submit<P> {
    /// It runs on an activation now.
    Assigned(Assignment<P>),
    /// Every activation is at its limit and no new one can be created: it waits in the queue.
    Queued,
    /// The queue is full: the caller gets an error event at once.
    Refused(Refusal),
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The pool queue is at its bound.
    QueueFull {
        /// The bound.
        bound: usize,
    },
    /// No live member declares the worker type.
    NoNode,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::QueueFull { bound } => write!(f, "the worker pool queue is full ({bound})"),
            Refusal::NoNode => f.write_str("no member runs this worker type"),
        }
    }
}

/// Why a request in progress failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The node running its activation failed.
    NodeFailed(NodeId),
    /// Its activation crashed.
    ActivationCrashed(ActivationId),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NodeFailed(n) => write!(f, "worker node {n} failed"),
            Failure::ActivationCrashed(a) => write!(f, "worker {a} crashed"),
        }
    }
}

/// What the dispatcher does about a failed request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<P> {
    /// An idempotent request, retried once on another activation.
    Retried(Assignment<P>),
    /// An idempotent request, retried once, waiting in the queue for capacity.
    RetryQueued(String),
    /// Reported to the caller as an error event.
    Failed {
        /// The request.
        request: String,
        /// The request itself.
        payload: P,
        /// Why.
        failure: Failure,
    },
}

#[derive(Clone, Debug)]
struct Activation {
    node: NodeId,
    in_flight: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct Pending<P> {
    activation: ActivationId,
    payload: P,
    attempts: u8,
}

/// The dispatcher of one worker type on one node.
// @tg: implements:: [[openspec:worker-pools#Stateless Worker Activations]]
// @tg: implements:: [[openspec:worker-pools#Pool Placement Across Nodes]]
// @tg: implements:: [[openspec:worker-pools#Load Balancing Policies]]
// @tg: implements:: [[openspec:worker-pools#Worker Concurrency Limits]]
// @tg: implements:: [[openspec:worker-pools#No Journal For Workers]]
// @tg: implements:: [[openspec:worker-pools#Worker Failure Reporting]]
#[derive(Clone, Debug)]
pub struct WorkerPool<P> {
    ty: WorkerType,
    local: NodeId,
    nodes: BTreeSet<NodeId>,
    activations: BTreeMap<ActivationId, Activation>,
    requests: BTreeMap<String, Pending<P>>,
    queue: VecDeque<(String, P, u8)>,
    next: u64,
    turn: u64,
}

impl<P: Clone> WorkerPool<P> {
    /// The dispatcher of `ty` on node `local`, with no member declaring it yet.
    pub fn new(ty: WorkerType, local: NodeId) -> Self {
        WorkerPool {
            ty,
            local,
            nodes: BTreeSet::new(),
            activations: BTreeMap::new(),
            requests: BTreeMap::new(),
            queue: VecDeque::new(),
            next: 0,
            turn: 0,
        }
    }

    /// The worker type.
    pub fn worker_type(&self) -> &WorkerType {
        &self.ty
    }

    /// Records that `node` declares the worker type and is live: activations may be placed
    /// on it.
    pub fn declare(&mut self, node: NodeId) {
        self.nodes.insert(node);
    }

    /// The members that declare the type.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeId> {
        self.nodes.iter()
    }

    /// The activations, with their node and requests in progress.
    pub fn activations(&self) -> Vec<(ActivationId, NodeId, usize)> {
        self.activations
            .iter()
            .map(|(id, a)| (*id, a.node.clone(), a.in_flight.len()))
            .collect()
    }

    /// The requests in progress on `activation`.
    pub fn in_flight(&self, activation: ActivationId) -> usize {
        self.activations
            .get(&activation)
            .map_or(0, |a| a.in_flight.len())
    }

    /// The requests waiting in the queue.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// The assignment of a request in progress.
    pub fn assignment_of(&self, request: &str) -> Option<(ActivationId, NodeId)> {
        let p = self.requests.get(request)?;
        let a = self.activations.get(&p.activation)?;
        Some((p.activation, a.node.clone()))
    }

    fn count_on(&self, node: &NodeId) -> usize {
        self.activations
            .values()
            .filter(|a| &a.node == node)
            .count()
    }

    /// The activation with free capacity the policy picks, if any.
    fn pick(&mut self, exclude: Option<ActivationId>) -> Option<ActivationId> {
        let max = self.ty.max_in_flight;
        let free: Vec<(ActivationId, &Activation)> = self
            .activations
            .iter()
            .filter(|(id, a)| a.in_flight.len() < max && Some(**id) != exclude)
            .map(|(id, a)| (*id, a))
            .collect();
        if free.is_empty() {
            return None;
        }
        let least = |c: &[(ActivationId, &Activation)]| {
            c.iter()
                .min_by_key(|(id, a)| (a.in_flight.len(), *id))
                .map(|(id, _)| *id)
        };
        match self.ty.policy {
            Policy::RoundRobin => {
                let i = (self.turn as usize) % free.len();
                self.turn = self.turn.wrapping_add(1);
                Some(free[i].0)
            }
            Policy::LeastOutstanding => least(&free),
            Policy::PreferLocal => {
                let local: Vec<_> = free
                    .iter()
                    .filter(|(_, a)| a.node == self.local)
                    .copied()
                    .collect();
                least(&local).or_else(|| least(&free))
            }
        }
    }

    /// The node a new activation goes to: a declaring member under the per-node limit with
    /// the fewest activations (the sender's own node first under [`Policy::PreferLocal`]).
    fn place(&self) -> Option<NodeId> {
        let max = self.ty.max_activations_per_node;
        if self.ty.policy == Policy::PreferLocal
            && self.nodes.contains(&self.local)
            && self.count_on(&self.local) < max
        {
            return Some(self.local.clone());
        }
        let room: Vec<&NodeId> = self
            .nodes
            .iter()
            .filter(|n| self.count_on(n) < max)
            .collect();
        let fewest = room.iter().map(|n| self.count_on(n)).min()?;
        let ties: Vec<&&NodeId> = room.iter().filter(|n| self.count_on(n) == fewest).collect();
        Some((*ties[(self.next as usize) % ties.len()]).clone())
    }

    fn create(&mut self, node: NodeId) -> ActivationId {
        let id = ActivationId(self.next);
        self.next += 1;
        self.activations.insert(
            id,
            Activation {
                node,
                in_flight: BTreeSet::new(),
            },
        );
        id
    }

    fn assign(
        &mut self,
        request: String,
        payload: P,
        attempts: u8,
        exclude: Option<ActivationId>,
    ) -> Option<Assignment<P>> {
        let (activation, created) = match self.pick(exclude) {
            Some(a) => (a, false),
            None => (self.create(self.place()?), true),
        };
        let a = self.activations.get_mut(&activation)?;
        a.in_flight.insert(request.clone());
        let node = a.node.clone();
        self.requests.insert(
            request.clone(),
            Pending {
                activation,
                payload: payload.clone(),
                attempts,
            },
        );
        Some(Assignment {
            request,
            activation,
            node,
            created,
            payload,
        })
    }

    /// Places request `request`: on an activation with free capacity, on a new activation,
    /// in the queue, or nowhere.
    pub fn submit(&mut self, request: impl Into<String>, payload: P) -> Submit<P> {
        let request = request.into();
        if self.nodes.is_empty() {
            return Submit::Refused(Refusal::NoNode);
        }
        if self.queue.is_empty()
            && let Some(a) = self.assign(request.clone(), payload.clone(), 0, None)
        {
            return Submit::Assigned(a);
        }
        if self.queue.len() >= self.ty.queue_bound {
            return Submit::Refused(Refusal::QueueFull {
                bound: self.ty.queue_bound,
            });
        }
        self.queue.push_back((request, payload, 0));
        Submit::Queued
    }

    /// Places queued requests while there is capacity.
    fn drain_queue(&mut self) -> Vec<Assignment<P>> {
        let mut out = Vec::new();
        while let Some((request, payload, attempts)) = self.queue.pop_front() {
            match self.assign(request.clone(), payload.clone(), attempts, None) {
                Some(a) => out.push(a),
                None => {
                    self.queue.push_front((request, payload, attempts));
                    break;
                }
            }
        }
        out
    }

    fn finish(&mut self, request: &str) -> Option<Pending<P>> {
        let p = self.requests.remove(request)?;
        if let Some(a) = self.activations.get_mut(&p.activation) {
            a.in_flight.remove(request);
        }
        Some(p)
    }

    /// Records that `request` finished (its reply reached the caller, or its deadline passed);
    /// returns the queued requests that now have capacity.
    pub fn complete(&mut self, request: &str) -> Vec<Assignment<P>> {
        self.finish(request);
        self.drain_queue()
    }

    /// Discards `activation` if it is idle (for example under memory pressure). Workers keep
    /// no state, so nothing is lost; the next request uses or creates another activation.
    pub fn discard(&mut self, activation: ActivationId) -> bool {
        if self
            .activations
            .get(&activation)
            .is_some_and(|a| a.in_flight.is_empty())
        {
            self.activations.remove(&activation);
            true
        } else {
            false
        }
    }

    /// Discards every idle activation; returns how many.
    pub fn discard_idle(&mut self) -> usize {
        let idle: Vec<ActivationId> = self
            .activations
            .iter()
            .filter(|(_, a)| a.in_flight.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for id in &idle {
            self.activations.remove(id);
        }
        idle.len()
    }

    fn failed(&mut self, request: String, p: Pending<P>, failure: Failure) -> Outcome<P> {
        if self.ty.idempotent && p.attempts == 0 {
            match self.assign(request.clone(), p.payload.clone(), 1, Some(p.activation)) {
                Some(a) => return Outcome::Retried(a),
                None if self.queue.len() < self.ty.queue_bound => {
                    self.queue.push_front((request.clone(), p.payload, 1));
                    return Outcome::RetryQueued(request);
                }
                None => {}
            }
        }
        Outcome::Failed {
            request,
            payload: p.payload,
            failure,
        }
    }

    /// Records that the activation serving `request` crashed while handling it. The
    /// activation is dropped; an idempotent request is retried once on another activation,
    /// any other is reported as failed, and so is every other request it was serving.
    pub fn activation_crashed(&mut self, request: &str) -> Vec<Outcome<P>> {
        let Some(p) = self.requests.remove(request) else {
            return Vec::new();
        };
        let crashed = p.activation;
        let others: Vec<String> = self
            .activations
            .remove(&crashed)
            .map(|a| a.in_flight.into_iter().filter(|r| r != request).collect())
            .unwrap_or_default();
        let mut out = vec![self.failed(request.to_owned(), p, Failure::ActivationCrashed(crashed))];
        for r in others {
            if let Some(p) = self.requests.remove(&r) {
                out.push(self.failed(r, p, Failure::ActivationCrashed(crashed)));
            }
        }
        out
    }

    /// Records that `node` failed or no longer declares the type: its activations are gone,
    /// and each request in progress on them is retried once (idempotent types) or reported.
    pub fn node_failed(&mut self, node: &NodeId) -> Vec<Outcome<P>> {
        self.nodes.remove(node);
        let gone: Vec<ActivationId> = self
            .activations
            .iter()
            .filter(|(_, a)| &a.node == node)
            .map(|(id, _)| *id)
            .collect();
        let mut lost = Vec::new();
        for id in gone {
            if let Some(a) = self.activations.remove(&id) {
                lost.extend(a.in_flight);
            }
        }
        let mut out = Vec::new();
        for r in lost {
            if let Some(p) = self.requests.remove(&r) {
                out.push(self.failed(r, p, Failure::NodeFailed(node.clone())));
            }
        }
        out
    }
}
