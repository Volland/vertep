//! Gossip membership: SWIM failure detection with indirect probes and piggybacked
//! dissemination, and shard leases granted by a deterministic leader.
//!
//! [`GossipNode`] is sans-IO: it consumes messages and clock ticks and produces messages in an
//! outbox, so the same code runs over the in-process [`SimNetwork`] (deterministic, with
//! partitions, pauses, crashes and clock skew) and, later, over a real datagram transport.
//!
//! - **Probing.** Every protocol period a node pings one member, round robin. Without an ack
//!   within the ack timeout it asks `indirect_probes` other members to ping the target for it
//!   (`PingReq`); without any ack by the end of the period the target is suspect. A suspect
//!   that does not refute within the suspicion timeout is failed. A node refutes a suspicion
//!   of itself by raising its incarnation.
//! - **Dissemination.** Membership and lease updates ride on probe messages, each sent a
//!   bounded number of times.
//! - **Leases.** The lowest active node identifier is the leader and grants leases from its
//!   [`LeaseTable`] with [`EpochPolicy::Tagged`] epochs, so leaders on two sides of a partition
//!   never grant the same epoch. A node that becomes leader first waits one lease duration,
//!   so every lease its predecessor granted has expired or been renewed with it. Gossip lease
//!   granting is weaker than consensus under partitions; journal fencing keeps it safe.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::clock::{Clock, ManualClock, Millis, duration_ms};
use crate::lease::{EpochPolicy, HeldLeases, Lease, LeaseConfig, LeaseError, LeaseTable};
use crate::membership::{Drain, JoinError, JoinRequest, Membership, check_compatible};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;
use crate::view::{Member, MemberStatus, ShardOwner, Subscription, View, ViewTracker};

/// The timing and fan-out of the gossip protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GossipConfig {
    /// How often a node probes one member.
    pub protocol_period: Duration,
    /// How long a direct ping waits for its ack before indirect probes are sent.
    pub ack_timeout: Duration,
    /// How many members are asked to probe a target indirectly.
    pub indirect_probes: usize,
    /// How long a suspect has to refute before it is failed.
    pub suspicion_timeout: Duration,
    /// How many times each update is piggybacked.
    pub retransmit: u32,
    /// The most updates piggybacked on one message.
    pub max_piggyback: usize,
    /// How long to wait before repeating an unanswered join or lease request.
    pub request_timeout: Duration,
    /// Lease timing.
    pub lease: LeaseConfig,
}

impl Default for GossipConfig {
    fn default() -> Self {
        GossipConfig {
            protocol_period: Duration::from_millis(200),
            ack_timeout: Duration::from_millis(80),
            indirect_probes: 2,
            suspicion_timeout: Duration::from_millis(1000),
            retransmit: 8,
            max_piggyback: 64,
            request_timeout: Duration::from_millis(400),
            lease: LeaseConfig {
                duration: Duration::from_millis(2000),
                safety_margin: Duration::from_millis(400),
            },
        }
    }
}

/// A membership or lease fact spread by gossip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// The node is alive at this incarnation.
    Alive {
        /// The node.
        node: NodeId,
        /// Its endpoint.
        endpoint: String,
        /// Its incarnation.
        incarnation: u64,
    },
    /// The node is suspected at this incarnation.
    Suspect {
        /// The node.
        node: NodeId,
        /// The incarnation suspected.
        incarnation: u64,
    },
    /// The node is failed.
    Dead {
        /// The node.
        node: NodeId,
        /// The incarnation declared failed.
        incarnation: u64,
    },
    /// The node is draining before it leaves.
    Draining {
        /// The node.
        node: NodeId,
        /// Its incarnation.
        incarnation: u64,
    },
    /// The node left gracefully.
    Left {
        /// The node.
        node: NodeId,
    },
    /// The leader granted a shard.
    Owner {
        /// The shard.
        shard: ShardId,
        /// The owner.
        node: NodeId,
        /// The grant's epoch.
        epoch: u64,
    },
}

/// A lease operation sent to the leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseOp {
    /// Acquire the shard.
    Acquire(ShardId),
    /// Renew the lease.
    Renew(Lease),
    /// Release the lease.
    Release(Lease),
    /// Revoke the shard's lease.
    Revoke(ShardId),
    /// Hand the lease over to a node.
    Handover(Lease, NodeId),
}

impl LeaseOp {
    fn shard(&self) -> ShardId {
        match self {
            LeaseOp::Acquire(s) | LeaseOp::Revoke(s) => *s,
            LeaseOp::Renew(l) | LeaseOp::Release(l) | LeaseOp::Handover(l, _) => l.shard,
        }
    }
}

/// A gossip protocol message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Ask a seed to admit the sender.
    Join(JoinRequest, u64),
    /// The seed refused the join.
    JoinRefused(JoinError),
    /// The seed admitted the sender: every member and owner it knows.
    Welcome(Vec<Update>),
    /// A probe.
    Ping {
        /// The probe sequence number.
        seq: u64,
        /// Piggybacked updates.
        updates: Vec<Update>,
    },
    /// The answer to a probe.
    Ack {
        /// The probe sequence number.
        seq: u64,
        /// Piggybacked updates.
        updates: Vec<Update>,
    },
    /// Ask the receiver to probe `target` on the sender's behalf.
    PingReq {
        /// The sender's probe sequence number.
        seq: u64,
        /// The member to probe.
        target: NodeId,
        /// Piggybacked updates.
        updates: Vec<Update>,
    },
    /// A lease operation for the leader.
    LeaseRequest {
        /// The request number.
        req: u64,
        /// The operation.
        op: LeaseOp,
    },
    /// The leader's answer.
    LeaseReply {
        /// The request number.
        req: u64,
        /// The resulting lease (`None` for release and revoke).
        result: Result<Option<Lease>, LeaseError>,
    },
}

/// Where a message goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A known member.
    Node(NodeId),
    /// An endpoint, for a join to a seed known only by address.
    Endpoint(String),
}

/// A message to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// The recipient.
    pub to: Destination,
    /// The message.
    pub msg: Message,
}

#[derive(Clone, Debug)]
struct Peer {
    endpoint: String,
    incarnation: u64,
    status: MemberStatus,
    suspect_since: Millis,
}

#[derive(Clone, Debug)]
struct Probe {
    target: NodeId,
    seq: u64,
    sent_at: Millis,
    indirect: bool,
    acked: bool,
}

#[derive(Clone, Debug)]
struct Pending {
    op: LeaseOp,
    sent_at: Millis,
}

#[derive(Debug)]
struct Leader {
    table: LeaseTable,
}

/// Where a join stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinState {
    /// The node founded the cluster or was welcomed.
    Joined,
    /// The join request is in flight.
    Joining,
    /// The seed refused.
    Refused(JoinError),
}

/// One node of a gossip cluster, sans-IO.
// @tg: implements:: [[openspec:cluster-membership#Gossip Membership]]
// @tg: implements:: [[openspec:cluster-membership#Failure Detection]]
#[derive(Debug)]
pub struct GossipNode {
    meta: ClusterMeta,
    id: NodeId,
    endpoint: String,
    config: GossipConfig,
    clock: Arc<dyn Clock>,
    rng: u64,
    incarnation: u64,
    peers: BTreeMap<NodeId, Peer>,
    owners: BTreeMap<ShardId, ShardOwner>,
    version: u64,
    tracker: ViewTracker,
    queue: VecDeque<(Update, u32)>,
    outbox: Vec<Envelope>,
    probe_order: Vec<NodeId>,
    probe: Option<Probe>,
    next_probe_at: Millis,
    seq: u64,
    relays: BTreeMap<u64, (NodeId, u64, Millis)>,
    join: JoinState,
    seed: Option<(String, Millis)>,
    leader: Option<Leader>,
    held: HeldLeases,
    pending: BTreeMap<u64, Pending>,
    results: BTreeMap<ShardId, Result<Lease, LeaseError>>,
    leaving: Option<BTreeSet<ShardId>>,
    /// The member each shard was shipped to when the leave began: it gets the handover.
    successors: BTreeMap<ShardId, NodeId>,
    left: bool,
    /// The low bits of every epoch this node grants as leader: from its identifier and its
    /// instance, so leaders differ, and so do two starts of one node.
    tag: u16,
    /// The highest epoch this node has granted or learned per shard, kept across its terms
    /// as leader so a later term never grants an epoch again.
    highest: BTreeMap<ShardId, u64>,
}

/// The epoch tag of node `id` in its start number `instance` (0 for the first start keeps the
/// tag of the identifier alone).
fn tag_of(id: &NodeId, instance: u64) -> u16 {
    let mut bytes = id.as_str().as_bytes().to_vec();
    if instance != 0 {
        bytes.push(0);
        bytes.extend_from_slice(&instance.to_be_bytes());
    }
    let h = crate::meta::HashVersion::Fnv1a64.hash64(&bytes);
    ((h & 0x7fff) as u16) | 1
}

impl GossipNode {
    /// Node `id` at `endpoint`, not yet part of any cluster; call [`GossipNode::found`] or
    /// [`GossipNode::join`].
    pub fn new(
        meta: ClusterMeta,
        id: NodeId,
        endpoint: impl Into<String>,
        config: GossipConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let endpoint = endpoint.into();
        let seed = crate::meta::HashVersion::Fnv1a64.hash64(id.as_str().as_bytes()) | 1;
        let now = clock.now();
        let mut peers = BTreeMap::new();
        peers.insert(
            id.clone(),
            Peer {
                endpoint: endpoint.clone(),
                incarnation: 0,
                status: MemberStatus::Active,
                suspect_since: now,
            },
        );
        let tracker = ViewTracker::new(View::empty(meta.cluster.clone()));
        let mut node = GossipNode {
            meta,
            id,
            endpoint,
            config,
            clock,
            rng: seed,
            incarnation: 0,
            peers,
            owners: BTreeMap::new(),
            version: 0,
            tracker,
            queue: VecDeque::new(),
            outbox: Vec::new(),
            probe_order: Vec::new(),
            probe: None,
            next_probe_at: now,
            seq: 0,
            relays: BTreeMap::new(),
            join: JoinState::Joining,
            seed: None,
            leader: None,
            held: HeldLeases::new(config.lease.safety_margin),
            pending: BTreeMap::new(),
            results: BTreeMap::new(),
            leaving: None,
            successors: BTreeMap::new(),
            left: false,
            tag: 0,
            highest: BTreeMap::new(),
        };
        node.tag = tag_of(&node.id, 0);
        node.changed();
        node
    }

    /// This node as start number `instance` of its identifier: a number that differs between
    /// successive starts of the node (a boot counter, the start time, a random number). A
    /// restarted node has forgotten the epochs it granted as leader; its new tag keeps it from
    /// granting one of them again.
    pub fn with_instance(mut self, instance: u64) -> Self {
        self.tag = tag_of(&self.id, instance);
        self
    }

    /// Starts a new cluster with this node as its only member.
    pub fn found(&mut self) {
        self.join = JoinState::Joined;
        self.update_leadership();
    }

    /// Joins the cluster through the seed at `seed_endpoint`.
    pub fn join(&mut self, seed_endpoint: impl Into<String>) {
        let seed = seed_endpoint.into();
        if seed == self.endpoint {
            self.found();
            return;
        }
        self.join = JoinState::Joining;
        self.send_join(&seed);
        self.seed = Some((seed, self.clock.now()));
    }

    fn send_join(&mut self, seed: &str) {
        let req = JoinRequest::new(&self.meta, self.id.clone(), self.endpoint.clone());
        self.outbox.push(Envelope {
            to: Destination::Endpoint(seed.to_owned()),
            msg: Message::Join(req, self.incarnation),
        });
    }

    /// Where the join stands.
    pub fn join_state(&self) -> &JoinState {
        &self.join
    }

    /// This node.
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// The node's endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// True after a graceful leave completed.
    pub fn has_left(&self) -> bool {
        self.left
    }

    /// The messages produced since the last call.
    pub fn take_outbox(&mut self) -> Vec<Envelope> {
        std::mem::take(&mut self.outbox)
    }

    /// The current leader: the lowest active member identifier.
    pub fn leader(&self) -> Option<NodeId> {
        self.peers
            .iter()
            .find(|(_, p)| p.status == MemberStatus::Active)
            .map(|(id, _)| id.clone())
    }

    fn rand(&mut self) -> u64 {
        // xorshift64: deterministic per node
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn enqueue(&mut self, u: Update) {
        let n = self.peers.len().max(2) as f64;
        let times = self.config.retransmit.max((3.0 * n.log2().ceil()) as u32);
        self.queue.retain(|(q, _)| !supersedes(&u, q));
        self.queue.push_back((u, times));
    }

    fn piggyback(&mut self) -> Vec<Update> {
        let mut out = Vec::new();
        let max = self.config.max_piggyback;
        for (u, left) in self.queue.iter_mut().rev() {
            if out.len() == max {
                break;
            }
            out.push(u.clone());
            *left = left.saturating_sub(1);
        }
        self.queue.retain(|(_, left)| *left > 0);
        out
    }

    fn send(&mut self, to: &NodeId, msg: Message) {
        self.outbox.push(Envelope {
            to: Destination::Node(to.clone()),
            msg,
        });
    }

    fn view_now(&self) -> View {
        View {
            cluster: self.meta.cluster.clone(),
            version: self.version,
            members: self
                .peers
                .iter()
                .map(|(id, p)| {
                    (
                        id.clone(),
                        Member {
                            id: id.clone(),
                            endpoint: p.endpoint.clone(),
                            status: p.status,
                        },
                    )
                })
                .collect(),
            owners: self.owners.clone(),
        }
    }

    fn changed(&mut self) {
        self.version += 1;
        let view = self.view_now();
        self.tracker.offer(view.clone());
        self.held.retain_current(&view);
        self.update_leadership();
    }

    fn update_leadership(&mut self) {
        let me_leader = self.join == JoinState::Joined
            && !self.left
            && self.leader().as_ref() == Some(&self.id);
        match (me_leader, self.leader.is_some()) {
            (true, false) => {
                let now = self.clock.now();
                let lease = self.config.lease;
                let mut table = LeaseTable::new(self.meta.shard_count, lease)
                    .with_policy(EpochPolicy::Tagged(self.tag));
                let until = now.after(lease.duration);
                for (s, o) in &self.owners {
                    table.seed(*s, &o.node, o.epoch, until);
                }
                for (s, e) in &self.highest {
                    table.note_epoch(*s, *e);
                }
                // a predecessor may have granted leases this node never heard of
                table.block_new_grants_until(until);
                self.leader = Some(Leader { table });
            }
            (false, true) => {
                // keep what this term granted for the next one
                if let Some(l) = self.leader.take() {
                    for s in self.meta.shard_count.shards() {
                        let e = l.table.highest_epoch(s);
                        if e > 0 {
                            let h = self.highest.entry(s).or_default();
                            *h = (*h).max(e);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Applies one update; returns whether it changed anything.
    fn apply(&mut self, u: &Update) -> bool {
        let now = self.clock.now();
        match u {
            Update::Alive {
                node,
                endpoint,
                incarnation,
            } => {
                if node == &self.id {
                    return false;
                }
                match self.peers.get_mut(node) {
                    None => {
                        self.peers.insert(
                            node.clone(),
                            Peer {
                                endpoint: endpoint.clone(),
                                incarnation: *incarnation,
                                status: MemberStatus::Active,
                                suspect_since: now,
                            },
                        );
                        true
                    }
                    Some(p) if *incarnation > p.incarnation => {
                        p.incarnation = *incarnation;
                        p.endpoint = endpoint.clone();
                        p.status = MemberStatus::Active;
                        true
                    }
                    Some(_) => false,
                }
            }
            Update::Suspect { node, incarnation } => {
                if node == &self.id {
                    if *incarnation >= self.incarnation && !self.left {
                        self.refute(*incarnation);
                    }
                    return false;
                }
                match self.peers.get_mut(node) {
                    Some(p)
                        if *incarnation >= p.incarnation && p.status == MemberStatus::Active =>
                    {
                        p.incarnation = *incarnation;
                        p.status = MemberStatus::Suspect;
                        p.suspect_since = now;
                        true
                    }
                    _ => false,
                }
            }
            Update::Dead { node, incarnation } => {
                if node == &self.id {
                    if *incarnation >= self.incarnation && !self.left {
                        self.refute(*incarnation);
                    }
                    return false;
                }
                match self.peers.get_mut(node) {
                    Some(p)
                        if *incarnation >= p.incarnation && p.status != MemberStatus::Failed =>
                    {
                        p.incarnation = *incarnation;
                        p.status = MemberStatus::Failed;
                        true
                    }
                    _ => false,
                }
            }
            Update::Draining { node, incarnation } => match self.peers.get_mut(node) {
                Some(p) if *incarnation >= p.incarnation && p.status != MemberStatus::Draining => {
                    p.incarnation = *incarnation;
                    p.status = MemberStatus::Draining;
                    true
                }
                _ => false,
            },
            Update::Left { node } => node != &self.id && self.peers.remove(node).is_some(),
            Update::Owner { shard, node, epoch } => {
                let newer = self.owners.get(shard).is_none_or(|o| o.epoch < *epoch);
                if newer {
                    self.owners.insert(
                        *shard,
                        ShardOwner {
                            node: node.clone(),
                            epoch: *epoch,
                        },
                    );
                    if let Some(l) = &mut self.leader {
                        l.table
                            .seed(*shard, node, *epoch, now.after(self.config.lease.duration));
                    }
                }
                newer
            }
        }
    }

    fn refute(&mut self, suspected: u64) {
        self.incarnation = self.incarnation.max(suspected) + 1;
        if let Some(me) = self.peers.get_mut(&self.id) {
            me.incarnation = self.incarnation;
        }
        let alive = Update::Alive {
            node: self.id.clone(),
            endpoint: self.endpoint.clone(),
            incarnation: self.incarnation,
        };
        self.enqueue(alive);
    }

    fn apply_all(&mut self, updates: &[Update]) {
        let mut any = false;
        for u in updates {
            if self.apply(u) {
                any = true;
                self.enqueue(u.clone());
            }
        }
        if any {
            self.changed();
        }
    }

    fn snapshot_updates(&self) -> Vec<Update> {
        let mut out = Vec::new();
        for (id, p) in &self.peers {
            out.push(Update::Alive {
                node: id.clone(),
                endpoint: p.endpoint.clone(),
                incarnation: p.incarnation,
            });
            match p.status {
                MemberStatus::Suspect => out.push(Update::Suspect {
                    node: id.clone(),
                    incarnation: p.incarnation,
                }),
                MemberStatus::Failed => out.push(Update::Dead {
                    node: id.clone(),
                    incarnation: p.incarnation,
                }),
                MemberStatus::Draining => out.push(Update::Draining {
                    node: id.clone(),
                    incarnation: p.incarnation,
                }),
                MemberStatus::Active => {}
            }
        }
        for (s, o) in &self.owners {
            out.push(Update::Owner {
                shard: *s,
                node: o.node.clone(),
                epoch: o.epoch,
            });
        }
        out
    }

    /// Handles a message from `from`.
    pub fn handle(&mut self, from: &NodeId, msg: Message) {
        if self.left {
            return;
        }
        match msg {
            Message::Join(req, incarnation) => {
                let duplicate = self.peers.get(&req.node).is_some_and(|p| {
                    p.status != MemberStatus::Failed && p.endpoint != req.endpoint
                });
                let refusal = check_compatible(&self.meta, &req)
                    .err()
                    .or_else(|| duplicate.then(|| JoinError::DuplicateNode(req.node.clone())));
                if let Some(e) = refusal {
                    self.outbox.push(Envelope {
                        to: Destination::Endpoint(req.endpoint),
                        msg: Message::JoinRefused(e),
                    });
                    return;
                }
                let inc =
                    incarnation.max(self.peers.get(&req.node).map_or(0, |p| p.incarnation + 1));
                let alive = Update::Alive {
                    node: req.node.clone(),
                    endpoint: req.endpoint.clone(),
                    incarnation: inc,
                };
                self.apply_all(std::slice::from_ref(&alive));
                let welcome = self.snapshot_updates();
                self.send(&req.node, Message::Welcome(welcome));
            }
            Message::JoinRefused(e) => {
                if self.join == JoinState::Joining {
                    self.join = JoinState::Refused(e);
                    self.seed = None;
                }
            }
            Message::Welcome(updates) => {
                if self.join == JoinState::Joining {
                    self.join = JoinState::Joined;
                    self.seed = None;
                    // adopt the incarnation the cluster knows us by
                    for u in &updates {
                        if let Update::Alive {
                            node, incarnation, ..
                        } = u
                            && node == &self.id
                            && *incarnation > self.incarnation
                        {
                            self.incarnation = *incarnation;
                        }
                    }
                    self.apply_all(&updates);
                    self.refute(self.incarnation.saturating_sub(1));
                    self.changed();
                }
            }
            Message::Ping { seq, updates } => {
                self.apply_all(&updates);
                let updates = self.piggyback();
                self.send(from, Message::Ack { seq, updates });
            }
            Message::Ack { seq, updates } => {
                self.apply_all(&updates);
                if let Some(p) = &mut self.probe
                    && p.seq == seq
                {
                    p.acked = true;
                }
                if let Some((requester, their_seq, _)) = self.relays.remove(&seq) {
                    let updates = self.piggyback();
                    self.send(
                        &requester,
                        Message::Ack {
                            seq: their_seq,
                            updates,
                        },
                    );
                }
            }
            Message::PingReq {
                seq,
                target,
                updates,
            } => {
                self.apply_all(&updates);
                self.seq += 1;
                let mine = self.seq;
                self.relays
                    .insert(mine, (from.clone(), seq, self.clock.now()));
                let updates = self.piggyback();
                self.send(&target, Message::Ping { seq: mine, updates });
            }
            Message::LeaseRequest { req, op } => {
                let result = self.lead(from, &op);
                self.send(from, Message::LeaseReply { req, result });
            }
            Message::LeaseReply { req, result } => self.on_lease_reply(req, result),
        }
    }

    /// Applies a lease operation as the leader.
    fn lead(&mut self, from: &NodeId, op: &LeaseOp) -> Result<Option<Lease>, LeaseError> {
        let now = self.clock.now();
        let status = self.peers.get(from).map(|p| p.status);
        let Some(leader) = &mut self.leader else {
            return Err(LeaseError::Unavailable(format!(
                "{} is not the lease leader",
                self.id
            )));
        };
        let out = match op {
            LeaseOp::Acquire(s) => {
                if status != Some(MemberStatus::Active) {
                    return Err(LeaseError::NotEligible {
                        node: from.clone(),
                        status,
                    });
                }
                leader.table.acquire(*s, from, now).map(Some)
            }
            LeaseOp::Renew(l) => {
                if &l.owner != from || status.is_none_or(|s| s == MemberStatus::Failed) {
                    return Err(LeaseError::NotEligible {
                        node: from.clone(),
                        status,
                    });
                }
                leader.table.renew(l, now).map(Some)
            }
            LeaseOp::Release(l) => leader.table.release(l).map(|()| None),
            LeaseOp::Revoke(s) => leader.table.revoke(*s, now).map(|()| None),
            LeaseOp::Handover(l, to) => {
                if self.peers.get(to).map(|p| p.status) != Some(MemberStatus::Active) {
                    return Err(LeaseError::NotEligible {
                        node: to.clone(),
                        status: self.peers.get(to).map(|p| p.status),
                    });
                }
                leader.table.handover(l, to, now).map(Some)
            }
        };
        // publish the grant table's owners
        let owners = leader.table.owners();
        let mut updates = Vec::new();
        for (s, o) in owners {
            if self.owners.get(&s) != Some(&o) {
                updates.push(Update::Owner {
                    shard: s,
                    node: o.node,
                    epoch: o.epoch,
                });
            }
        }
        self.apply_all(&updates);
        out
    }

    fn on_lease_reply(&mut self, req: u64, result: Result<Option<Lease>, LeaseError>) {
        let Some(p) = self.pending.remove(&req) else {
            return;
        };
        let shard = p.op.shard();
        match (&p.op, result) {
            (LeaseOp::Acquire(_) | LeaseOp::Renew(_), Ok(Some(lease))) => {
                self.held.record(lease.clone(), p.sent_at);
                self.results.insert(shard, Ok(lease));
            }
            (LeaseOp::Acquire(_), Err(e)) => {
                self.results.insert(shard, Err(e));
            }
            (LeaseOp::Renew(_), Err(LeaseError::Unavailable(_))) => {}
            (LeaseOp::Renew(_), Err(_)) => {
                self.held.forget(shard);
            }
            (LeaseOp::Handover(..), Ok(_)) => {
                self.held.forget(shard);
                if let Some(w) = &mut self.leaving {
                    w.remove(&shard);
                }
            }
            _ => {}
        }
    }

    fn request(&mut self, op: LeaseOp) -> Result<(), LeaseError> {
        let Some(leader) = self.leader() else {
            return Err(LeaseError::Unavailable("no lease leader".into()));
        };
        self.seq += 1;
        let req = self.seq;
        self.pending.insert(
            req,
            Pending {
                op: op.clone(),
                sent_at: self.clock.now(),
            },
        );
        self.send(&leader, Message::LeaseRequest { req, op });
        Ok(())
    }

    fn in_flight(&self, shard: ShardId) -> bool {
        self.pending.values().any(|p| p.op.shard() == shard)
    }

    /// Runs the protocol's timers: probes, suspicion timeouts, join and lease retries, lease
    /// renewals and the end of a leave.
    pub fn tick(&mut self) {
        if self.left {
            return;
        }
        let now = self.clock.now();
        if let Some((seed, at)) = self.seed.clone()
            && now >= at.after(self.config.request_timeout)
        {
            self.send_join(&seed);
            self.seed = Some((seed, now));
        }
        if self.join != JoinState::Joined {
            return;
        }
        self.expire_requests(now);
        self.check_probe(now);
        self.check_suspects(now);
        self.renew_due(now);
        self.continue_leave();
        if now >= self.next_probe_at {
            self.next_probe_at = now.after(self.config.protocol_period);
            self.start_probe(now);
        }
    }

    fn expire_requests(&mut self, now: Millis) {
        let timeout = self.config.request_timeout;
        self.pending.retain(|_, p| now < p.sent_at.after(timeout));
        let horizon = self.config.protocol_period * 2;
        self.relays.retain(|_, (_, _, at)| now < at.after(horizon));
    }

    fn check_probe(&mut self, now: Millis) {
        let Some(p) = self.probe.clone() else {
            return;
        };
        if p.acked {
            self.probe = None;
            return;
        }
        if !p.indirect && now >= p.sent_at.after(self.config.ack_timeout) {
            let mut helpers: Vec<NodeId> = self
                .peers
                .iter()
                .filter(|(id, s)| {
                    **id != self.id && **id != p.target && s.status == MemberStatus::Active
                })
                .map(|(id, _)| id.clone())
                .collect();
            let k = self.config.indirect_probes.min(helpers.len());
            for _ in 0..k {
                let i = (self.rand() as usize) % helpers.len();
                let h = helpers.swap_remove(i);
                let updates = self.piggyback();
                self.send(
                    &h,
                    Message::PingReq {
                        seq: p.seq,
                        target: p.target.clone(),
                        updates,
                    },
                );
            }
            if let Some(probe) = &mut self.probe {
                probe.indirect = true;
            }
        }
        if now >= p.sent_at.after(self.config.protocol_period) {
            self.probe = None;
            let inc = self.peers.get(&p.target).map(|x| x.incarnation);
            if let Some(incarnation) = inc {
                let u = Update::Suspect {
                    node: p.target,
                    incarnation,
                };
                self.apply_all(&[u]);
            }
        }
    }

    fn check_suspects(&mut self, now: Millis) {
        let timeout = self.config.suspicion_timeout;
        let dead: Vec<Update> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                p.status == MemberStatus::Suspect && now >= p.suspect_since.after(timeout)
            })
            .map(|(id, p)| Update::Dead {
                node: id.clone(),
                incarnation: p.incarnation,
            })
            .collect();
        if !dead.is_empty() {
            self.apply_all(&dead);
        }
    }

    fn renew_due(&mut self, now: Millis) {
        for lease in self.held.due(now) {
            if self.in_flight(lease.shard) {
                continue;
            }
            if self.leader.is_some() {
                let r = self.lead_local(LeaseOp::Renew(lease.clone()));
                if r.is_err() {
                    self.held.forget(lease.shard);
                }
            } else {
                let _ = self.request(LeaseOp::Renew(lease));
            }
        }
    }

    fn lead_local(&mut self, op: LeaseOp) -> Result<Option<Lease>, LeaseError> {
        let sent = self.clock.now();
        let me = self.id.clone();
        let out = self.lead(&me, &op)?;
        if let Some(lease) = &out
            && lease.owner == self.id
        {
            self.held.record(lease.clone(), sent);
        }
        Ok(out)
    }

    fn start_probe(&mut self, now: Millis) {
        if self.probe_order.is_empty() {
            let mut order: Vec<NodeId> = self
                .peers
                .iter()
                .filter(|(id, p)| **id != self.id && p.status != MemberStatus::Failed)
                .map(|(id, _)| id.clone())
                .collect();
            // Fisher-Yates with the node's own generator
            for i in (1..order.len()).rev() {
                let j = (self.rand() as usize) % (i + 1);
                order.swap(i, j);
            }
            self.probe_order = order;
        }
        let Some(target) = self.probe_order.pop() else {
            return;
        };
        if self
            .peers
            .get(&target)
            .is_none_or(|p| p.status == MemberStatus::Failed)
        {
            return;
        }
        self.seq += 1;
        let seq = self.seq;
        self.probe = Some(Probe {
            target: target.clone(),
            seq,
            sent_at: now,
            indirect: false,
            acked: false,
        });
        let updates = self.piggyback();
        self.send(&target, Message::Ping { seq, updates });
    }

    fn continue_leave(&mut self) {
        let Some(waiting) = self.leaving.clone() else {
            return;
        };
        if waiting.is_empty() {
            let left = Update::Left {
                node: self.id.clone(),
            };
            let all: Vec<NodeId> = self
                .peers
                .keys()
                .filter(|id| **id != self.id)
                .cloned()
                .collect();
            for p in all {
                self.send(
                    &p,
                    Message::Ping {
                        seq: 0,
                        updates: vec![left.clone()],
                    },
                );
            }
            self.left = true;
            self.leaving = None;
            self.leader = None;
            return;
        }
        for shard in waiting {
            if self.in_flight(shard) {
                continue;
            }
            let Some(lease) = self.held.all().find(|l| l.shard == shard).cloned() else {
                if let Some(w) = &mut self.leaving {
                    w.remove(&shard);
                }
                continue;
            };
            let others: Vec<NodeId> = self
                .peers
                .iter()
                .filter(|(id, p)| **id != self.id && p.status == MemberStatus::Active)
                .map(|(id, _)| id.clone())
                .collect();
            if others.is_empty() {
                continue;
            }
            // the member the shard was shipped to, while it is active
            let to = match self.successors.get(&shard) {
                Some(n) if others.contains(n) => n.clone(),
                _ => others[shard.index() % others.len()].clone(),
            };
            if self.leader.is_some() {
                if self.lead_local(LeaseOp::Handover(lease, to)).is_ok() {
                    self.held.forget(shard);
                    if let Some(w) = &mut self.leaving {
                        w.remove(&shard);
                    }
                }
            } else {
                let _ = self.request(LeaseOp::Handover(lease, to));
            }
        }
    }

    /// Starts a graceful leave: marks this node draining, drains every held shard through
    /// `drain` and ships it to another active member ([`Drain::ship_shard`]), and hands each
    /// to that member; once all are handed over (on later ticks) the node announces that it
    /// left.
    ///
    /// # Errors
    ///
    /// The drain's reason for the first shard that could not be drained.
    pub fn leave(&mut self, drain: &mut dyn Drain) -> Result<(), String> {
        let draining = Update::Draining {
            node: self.id.clone(),
            incarnation: self.incarnation,
        };
        if let Some(me) = self.peers.get_mut(&self.id) {
            me.status = MemberStatus::Draining;
        }
        self.enqueue(draining);
        self.changed();
        let shards: BTreeSet<ShardId> = self.held.all().map(|l| l.shard).collect();
        let others: Vec<NodeId> = self
            .peers
            .iter()
            .filter(|(id, p)| **id != self.id && p.status == MemberStatus::Active)
            .map(|(id, _)| id.clone())
            .collect();
        for s in &shards {
            drain.drain_shard(*s)?;
            if !others.is_empty() {
                let to = others[s.index() % others.len()].clone();
                drain.ship_shard(*s, &to)?;
                self.successors.insert(*s, to);
            }
        }
        self.leaving = Some(shards);
        self.continue_leave();
        Ok(())
    }

    // --- the Membership operations, on the node itself

    fn acquire_op(&mut self, shard: ShardId) -> Result<Lease, LeaseError> {
        let now = self.clock.now();
        if !self.meta.shard_count.contains(shard) {
            return Err(LeaseError::OutOfRange {
                shard,
                count: self.meta.shard_count,
            });
        }
        if let Some(l) = self.held.valid(shard, now) {
            return Ok(l.clone());
        }
        if let Some(r) = self.results.remove(&shard) {
            match r {
                Ok(l) if self.held.valid(shard, now).is_some() => return Ok(l),
                Ok(_) => {}
                Err(e) => return Err(e),
            }
        }
        let status = self.peers.get(&self.id).map(|p| p.status);
        if self.leaving.is_some() || status != Some(MemberStatus::Active) {
            return Err(LeaseError::NotEligible {
                node: self.id.clone(),
                status,
            });
        }
        if self.leader.is_some() {
            return self
                .lead_local(LeaseOp::Acquire(shard))?
                .ok_or(LeaseError::Pending { shard });
        }
        if !self.in_flight(shard) {
            self.request(LeaseOp::Acquire(shard))?;
        }
        Err(LeaseError::Pending { shard })
    }

    fn renew_op(&mut self, lease: &Lease) -> Result<Lease, LeaseError> {
        if self.leader.is_some() {
            return self
                .lead_local(LeaseOp::Renew(lease.clone()))?
                .ok_or(LeaseError::Pending { shard: lease.shard });
        }
        if !self.in_flight(lease.shard) {
            self.request(LeaseOp::Renew(lease.clone()))?;
        }
        Err(LeaseError::Pending { shard: lease.shard })
    }

    fn release_op(&mut self, lease: &Lease) -> Result<(), LeaseError> {
        self.held.forget(lease.shard);
        if self.leader.is_some() {
            self.lead_local(LeaseOp::Release(lease.clone())).map(|_| ())
        } else {
            self.request(LeaseOp::Release(lease.clone()))
        }
    }

    fn handover_op(&mut self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        let shard = lease.shard;
        if self.leader.is_some() {
            let next = self
                .lead_local(LeaseOp::Handover(lease.clone(), to.clone()))?
                .ok_or(LeaseError::Pending { shard })?;
            self.held.forget(shard);
            return Ok(next);
        }
        if !self.in_flight(shard) {
            self.request(LeaseOp::Handover(lease.clone(), to.clone()))?;
        }
        Err(LeaseError::Pending { shard })
    }

    fn revoke_op(&mut self, shard: ShardId) -> Result<(), LeaseError> {
        if self.leader.is_some() {
            self.lead_local(LeaseOp::Revoke(shard)).map(|_| ())
        } else {
            self.request(LeaseOp::Revoke(shard))
        }
    }
}

fn supersedes(new: &Update, old: &Update) -> bool {
    let node = |u: &Update| match u {
        Update::Alive { node, .. }
        | Update::Suspect { node, .. }
        | Update::Dead { node, .. }
        | Update::Draining { node, .. }
        | Update::Left { node } => Some(node.clone()),
        Update::Owner { .. } => None,
    };
    match (new, old) {
        (Update::Owner { shard: a, .. }, Update::Owner { shard: b, .. }) => a == b,
        _ => node(new).is_some() && node(new) == node(old),
    }
}

/// A shared handle on a [`GossipNode`]: the [`Membership`] implementation of the gossip
/// cluster. The node's driver (a [`SimNetwork`] or a real transport) holds the same handle.
#[derive(Clone, Debug)]
// @tg: implements:: [[openspec:cluster-membership#Gossip Membership]]
pub struct GossipMembership {
    node: Arc<Mutex<GossipNode>>,
    id: NodeId,
    meta: ClusterMeta,
}

impl GossipMembership {
    /// A handle on `node`.
    pub fn new(node: GossipNode) -> Self {
        let id = node.id.clone();
        let meta = node.meta.clone();
        GossipMembership {
            node: Arc::new(Mutex::new(node)),
            id,
            meta,
        }
    }

    /// Locks the node.
    pub fn node(&self) -> MutexGuard<'_, GossipNode> {
        self.node
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Membership for GossipMembership {
    fn local(&self) -> &NodeId {
        &self.id
    }

    fn meta(&self) -> &ClusterMeta {
        &self.meta
    }

    fn view(&self) -> View {
        self.node().tracker.current().clone()
    }

    fn subscribe(&self) -> Subscription {
        self.node().tracker.subscribe()
    }

    fn acquire(&self, shard: ShardId) -> Result<Lease, LeaseError> {
        self.node().acquire_op(shard)
    }

    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        self.node().renew_op(lease)
    }

    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.node().release_op(lease)
    }

    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.node().revoke_op(shard)
    }

    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        self.node().handover_op(lease, to)
    }

    fn held(&self, shard: ShardId) -> Option<Lease> {
        let n = self.node();
        let now = n.clock.now();
        n.held.valid(shard, now).cloned()
    }

    fn forget(&self, shard: ShardId) {
        self.node().held.forget(shard);
    }
}

#[derive(Debug)]
struct SimNode {
    handle: GossipMembership,
    clock: ManualClock,
    /// How fast the node's clock runs, in percent of the network's time.
    rate: u64,
}

#[derive(Debug)]
struct InFlight {
    at: Millis,
    seq: u64,
    from: NodeId,
    to: NodeId,
    msg: Message,
}

/// An in-process network of gossip nodes on simulated time: deterministic delivery with a
/// fixed latency, cut links, partitions, paused nodes (their clocks stop and their messages
/// wait), crashed nodes and clock skew.
#[derive(Debug)]
pub struct SimNetwork {
    now: Millis,
    step: Duration,
    latency: Duration,
    nodes: BTreeMap<NodeId, SimNode>,
    endpoints: BTreeMap<String, NodeId>,
    in_flight: Vec<InFlight>,
    seq: u64,
    cut: BTreeSet<(NodeId, NodeId)>,
    paused: BTreeSet<NodeId>,
    crashed: BTreeSet<NodeId>,
    delivered: u64,
}

impl SimNetwork {
    /// An empty network with message latency `latency`, simulated in steps of `step`.
    pub fn new(step: Duration, latency: Duration) -> Self {
        SimNetwork {
            now: Millis(0),
            step,
            latency,
            nodes: BTreeMap::new(),
            endpoints: BTreeMap::new(),
            in_flight: Vec::new(),
            seq: 0,
            cut: BTreeSet::new(),
            paused: BTreeSet::new(),
            crashed: BTreeSet::new(),
            delivered: 0,
        }
    }

    /// The simulated time.
    pub fn now(&self) -> Millis {
        self.now
    }

    /// Adds node `id` at `endpoint` with its own clock (starting at the network's time) and
    /// returns its membership handle; the caller then founds or joins.
    pub fn add_node(
        &mut self,
        meta: &ClusterMeta,
        id: NodeId,
        endpoint: &str,
        config: GossipConfig,
    ) -> GossipMembership {
        self.add_instance(meta, id, endpoint, config, 0)
    }

    fn add_instance(
        &mut self,
        meta: &ClusterMeta,
        id: NodeId,
        endpoint: &str,
        config: GossipConfig,
        instance: u64,
    ) -> GossipMembership {
        let clock = ManualClock::at(self.now.0);
        let node = GossipNode::new(
            meta.clone(),
            id.clone(),
            endpoint,
            config,
            Arc::new(clock.clone()),
        )
        .with_instance(instance);
        let handle = GossipMembership::new(node);
        self.endpoints.insert(endpoint.to_owned(), id.clone());
        self.nodes.insert(
            id,
            SimNode {
                handle: handle.clone(),
                clock,
                rate: 100,
            },
        );
        handle
    }

    /// The handle of node `id`.
    pub fn handle(&self, id: &NodeId) -> Option<&GossipMembership> {
        self.nodes.get(id).map(|n| &n.handle)
    }

    /// Cuts the link between `a` and `b`, both directions.
    pub fn cut(&mut self, a: &NodeId, b: &NodeId) {
        self.cut.insert((a.clone(), b.clone()));
        self.cut.insert((b.clone(), a.clone()));
    }

    /// Partitions the network: nodes in different groups cannot reach each other.
    pub fn partition(&mut self, groups: &[Vec<NodeId>]) {
        for (i, g) in groups.iter().enumerate() {
            for h in &groups[i + 1..] {
                for a in g {
                    for b in h {
                        self.cut(a, b);
                    }
                }
            }
        }
    }

    /// Restores every link of `id`, leaving other cuts in place.
    pub fn reconnect(&mut self, id: &NodeId) {
        self.cut.retain(|(a, b)| a != id && b != id);
    }

    /// Restarts crashed node `id` as a fresh process with no state: a new node at the same
    /// endpoint and a new instance number, which must join again (through `seed`).
    pub fn restart(
        &mut self,
        meta: &ClusterMeta,
        id: &NodeId,
        config: GossipConfig,
        seed: &str,
    ) -> Option<GossipMembership> {
        let endpoint = self.nodes.get(id)?.handle.node().endpoint().to_owned();
        let rate = self.nodes.get(id).map_or(100, |n| n.rate);
        self.crashed.remove(id);
        self.paused.remove(id);
        self.in_flight.retain(|m| &m.to != id && &m.from != id);
        // a new start of the node: its instance is the time it starts
        let h = self.add_instance(meta, id.clone(), &endpoint, config, self.now.0.max(1));
        if let Some(n) = self.nodes.get_mut(id) {
            n.rate = rate;
        }
        h.node().join(seed);
        Some(h)
    }

    /// Runs `id`'s clock at `percent` of the network's speed (clock skew that grows).
    pub fn set_rate(&mut self, id: &NodeId, percent: u64) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.rate = percent;
        }
    }

    /// Restores every link.
    pub fn heal(&mut self) {
        self.cut.clear();
    }

    /// Pauses `id`: its clock stops, it does not tick, and messages to it wait.
    pub fn pause(&mut self, id: &NodeId) {
        self.paused.insert(id.clone());
    }

    /// Resumes a paused node; its clock jumps to the network's time.
    pub fn resume(&mut self, id: &NodeId) {
        self.paused.remove(id);
        if let Some(n) = self.nodes.get(id) {
            n.clock.set(self.now.0);
        }
    }

    /// Crashes `id`: it stops and every message to it is lost.
    pub fn crash(&mut self, id: &NodeId) {
        self.crashed.insert(id.clone());
    }

    /// Moves `id`'s clock ahead of the others by `d`.
    pub fn skew(&mut self, id: &NodeId, d: Duration) {
        if let Some(n) = self.nodes.get(id) {
            n.clock.advance(d);
        }
    }

    /// The number of messages delivered so far.
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    fn collect(&mut self, from: &NodeId) {
        let out = match self.nodes.get(from) {
            Some(n) => n.handle.node().take_outbox(),
            None => return,
        };
        for env in out {
            let to = match env.to {
                Destination::Node(n) => Some(n),
                Destination::Endpoint(e) => self.endpoints.get(&e).cloned(),
            };
            if let Some(to) = to {
                self.seq += 1;
                self.in_flight.push(InFlight {
                    at: self.now.after(self.latency),
                    seq: self.seq,
                    from: from.clone(),
                    to,
                    msg: env.msg,
                });
            }
        }
    }

    /// Runs the network for `d` of simulated time.
    pub fn run_for(&mut self, d: Duration) {
        let end = self.now.after(d);
        let ids: Vec<NodeId> = self.nodes.keys().cloned().collect();
        for id in &ids {
            self.collect(id);
        }
        while self.now < end {
            self.now = self.now.after(self.step);
            let step_ms = duration_ms(self.step);
            for (id, n) in &self.nodes {
                if !self.paused.contains(id) && !self.crashed.contains(id) {
                    n.clock.advance_ms(step_ms * n.rate / 100);
                }
            }
            // deliver what is due, in send order
            self.in_flight.sort_by_key(|m| (m.at, m.seq));
            let mut keep = Vec::new();
            let due: Vec<InFlight> = std::mem::take(&mut self.in_flight);
            for m in due {
                if m.at > self.now || self.paused.contains(&m.to) {
                    keep.push(m);
                    continue;
                }
                if self.crashed.contains(&m.to)
                    || self.crashed.contains(&m.from)
                    || self.cut.contains(&(m.from.clone(), m.to.clone()))
                {
                    continue;
                }
                if let Some(n) = self.nodes.get(&m.to) {
                    n.handle.node().handle(&m.from, m.msg);
                    self.delivered += 1;
                }
                self.collect(&m.to);
            }
            self.in_flight.extend(keep);
            for id in &ids {
                if self.paused.contains(id) || self.crashed.contains(id) {
                    continue;
                }
                if let Some(n) = self.nodes.get(id) {
                    n.handle.node().tick();
                }
                self.collect(id);
            }
        }
    }
}
