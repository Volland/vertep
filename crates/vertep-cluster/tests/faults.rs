//! Jepsen-style fault injection, in process: four nodes lease shards and process client
//! messages into fenced journals, sending a side effect to one of two receivers for each,
//! while the run partitions nodes from the lease authority (or, for gossip, from each other),
//! pauses them in the middle of an append, crashes and restarts them, runs one clock at half
//! speed (skew beyond the safety margin), delays and drops frames one way, and loses
//! acknowledgements so that clients retry. The recorded history is then checked: journals
//! linear, one writer per epoch, every acknowledged message processed once, no fenced side
//! effect accepted. Every history is also replayed through the Lean lease and fencing model
//! (`vertep-lease`, strict mode: every append under a recorded grant), which must take the
//! same accept, fence and admit decisions; the test skips that when the Lean executable is not
//! built, unless the `differential` feature requires it. Set `VERTEP_HISTORY_DIR` to keep each
//! run's history as JSON lines.
//!
//! The harness is generic over the membership (the external stand-in, static, gossip, and,
//! with features `etcd` and `kubernetes` and `VERTEP_CONTAINER_TESTS=1`, real etcd and
//! Kubernetes in containers, on real time) and over the journals: one shared fenced store
//! (moves by lease transfer), or local journals per node with replication factor 1 (only the
//! shard's home node may take it) or 3 (the owner replicates to two standbys and an append is
//! acknowledged once both stored it; only the shard's replica set may take it).

mod common;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use common::oracle::{lease_oracle, verify};
use common::{actor_in, meta, node, timing};
use vertep_cluster::{
    AppendDecision, Authority, AuthorityLink, AuthorityMembership, ClusterMeta, EpochFilter, Fence,
    FrameDecision, History, HistoryEvent, LocalStandby, ManualClock, Membership, MemoryAuthority,
    NodeId, ReplicatedJournal, ShardId, ShardJournal, StandbyLink, StandbyReplica, StaticConfig,
};
use vertep_store::oxrdf::NamedNode;
use vertep_store::testkit::sample_entry;
use vertep_store::{AppendError, AppendMeta, JournalStore, MemoryStore, Position};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

// ---------------------------------------------------------------------------------------------
// Membership backends

/// A cluster of nodes on one membership implementation, with its faults.
trait Backend {
    fn label(&self) -> String;
    fn size(&self) -> usize;
    fn id(&self, i: usize) -> NodeId;
    fn member(&self, i: usize) -> Arc<dyn Membership>;
    /// The harness time in milliseconds.
    fn now(&self) -> u64;
    /// One step of time for the authority or network and the clocks of the nodes `running`
    /// accepts (a paused or crashed node's clock stands still; the slow one runs at half
    /// speed).
    fn advance(&mut self, step_ms: u64, running: &dyn Fn(usize) -> bool);
    /// The node's periodic membership duty.
    fn tick(&mut self, i: usize);
    /// Cuts node `i` off (from the lease authority; for gossip, from the other nodes) or
    /// reconnects it.
    fn cut(&mut self, i: usize, cut: bool);
    fn pause(&mut self, i: usize);
    /// Resumes a paused node: its clock jumps to the present.
    fn resume(&mut self, i: usize);
    fn crash(&mut self, i: usize);
    /// Restarts a crashed node as a fresh process.
    fn restart(&mut self, i: usize);
    /// Whether a cut also severs the node's replication links.
    fn cut_severs_data(&self) -> bool {
        false
    }
}

type Linked<A> = AuthorityMembership<AuthorityLink<Arc<A>>>;

struct AuthorityNode<A: Authority> {
    id: NodeId,
    clock: ManualClock,
    link: AuthorityLink<Arc<A>>,
    m: Arc<Linked<A>>,
    slow: bool,
}

/// Nodes on a lease authority: in memory on a manual clock, or a real service on real time
/// (each node then has its own client of the service).
struct AuthorityBackend<A: Authority + 'static> {
    label: String,
    meta: ClusterMeta,
    client: Box<dyn Fn() -> Arc<A>>,
    /// The authority's clock when it runs in memory; `None` for a real service.
    authority_clock: Option<ManualClock>,
    start: std::time::Instant,
    last: u64,
    nodes: Vec<AuthorityNode<A>>,
    lease: vertep_cluster::LeaseConfig,
}

impl<A: Authority + 'static> AuthorityBackend<A> {
    fn new(
        label: &str,
        meta: ClusterMeta,
        authority_clock: Option<ManualClock>,
        lease: vertep_cluster::LeaseConfig,
        client: Box<dyn Fn() -> Arc<A>>,
    ) -> Self {
        let mut b = AuthorityBackend {
            label: label.to_owned(),
            meta,
            client,
            authority_clock,
            start: std::time::Instant::now(),
            last: 0,
            nodes: Vec::new(),
            lease,
        };
        for i in 0..4 {
            let n = b.fresh(node(&format!("n{i}")), i == 3);
            let _ = n.m.join();
            b.nodes.push(n);
        }
        b
    }

    fn fresh(&self, id: NodeId, slow: bool) -> AuthorityNode<A> {
        let clock = ManualClock::at(self.now());
        let link = AuthorityLink::new((self.client)());
        let m = Arc::new(AuthorityMembership::new(
            link.clone(),
            self.meta.clone(),
            id.clone(),
            id.as_str(),
            Arc::new(clock.clone()),
            self.lease,
        ));
        AuthorityNode {
            id,
            clock,
            link,
            m,
            slow,
        }
    }
}

impl<A: Authority + 'static> Backend for AuthorityBackend<A> {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn size(&self) -> usize {
        self.nodes.len()
    }
    fn id(&self, i: usize) -> NodeId {
        self.nodes[i].id.clone()
    }
    fn member(&self, i: usize) -> Arc<dyn Membership> {
        self.nodes[i].m.clone()
    }
    fn now(&self) -> u64 {
        match &self.authority_clock {
            Some(c) => vertep_cluster::Clock::now(c).0,
            None => self.start.elapsed().as_millis() as u64,
        }
    }
    fn advance(&mut self, step_ms: u64, running: &dyn Fn(usize) -> bool) {
        let elapsed = match &self.authority_clock {
            Some(c) => {
                c.advance_ms(step_ms);
                step_ms
            }
            None => {
                let target = self.last + step_ms;
                let now = self.now();
                if now < target {
                    std::thread::sleep(std::time::Duration::from_millis(target - now));
                }
                let now = self.now();
                let e = now - self.last;
                self.last = now;
                e
            }
        };
        for (i, n) in self.nodes.iter().enumerate() {
            if running(i) {
                n.clock
                    .advance_ms(if n.slow { elapsed / 2 } else { elapsed });
            }
        }
    }
    fn tick(&mut self, i: usize) {
        let _ = self.nodes[i].m.tick();
    }
    fn cut(&mut self, i: usize, cut: bool) {
        self.nodes[i].link.set_connected(!cut);
    }
    fn pause(&mut self, _: usize) {}
    fn resume(&mut self, i: usize) {
        self.nodes[i].clock.set(self.now());
    }
    fn crash(&mut self, _: usize) {}
    fn restart(&mut self, i: usize) {
        let (id, slow) = (self.nodes[i].id.clone(), self.nodes[i].slow);
        self.nodes[i] = self.fresh(id, slow);
    }
}

#[cfg(feature = "gossip")]
/// SWIM gossip on the simulated network, four nodes.
struct GossipBackend {
    meta: ClusterMeta,
    net: vertep_cluster::gossip::SimNetwork,
    nodes: Vec<(NodeId, vertep_cluster::gossip::GossipMembership)>,
}

#[cfg(feature = "gossip")]
impl GossipBackend {
    fn new(meta: ClusterMeta) -> Self {
        use vertep_cluster::gossip::{GossipConfig, SimNetwork};
        let mut net = SimNetwork::new(
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(5),
        );
        let nodes: Vec<_> = (0..4)
            .map(|i| {
                let id = node(&format!("n{i}"));
                let h = net.add_node(
                    &meta,
                    id.clone(),
                    &format!("n{i}:7946"),
                    GossipConfig::default(),
                );
                h.node().join("n0:7946");
                (id, h)
            })
            .collect();
        net.set_rate(&nodes[3].0, 50);
        net.run_for(std::time::Duration::from_secs(3));
        GossipBackend { meta, net, nodes }
    }
}

#[cfg(feature = "gossip")]
impl Backend for GossipBackend {
    fn label(&self) -> String {
        "gossip".into()
    }
    fn size(&self) -> usize {
        self.nodes.len()
    }
    fn id(&self, i: usize) -> NodeId {
        self.nodes[i].0.clone()
    }
    fn member(&self, i: usize) -> Arc<dyn Membership> {
        Arc::new(self.nodes[i].1.clone())
    }
    fn now(&self) -> u64 {
        self.net.now().0
    }
    fn advance(&mut self, step_ms: u64, _: &dyn Fn(usize) -> bool) {
        self.net.run_for(std::time::Duration::from_millis(step_ms));
    }
    fn tick(&mut self, _: usize) {}
    fn cut(&mut self, i: usize, cut: bool) {
        let id = self.nodes[i].0.clone();
        if cut {
            for (other, _) in &self.nodes {
                if other != &id {
                    self.net.cut(&id, other);
                }
            }
        } else {
            self.net.reconnect(&id);
        }
    }
    fn pause(&mut self, i: usize) {
        self.net.pause(&self.nodes[i].0);
    }
    fn resume(&mut self, i: usize) {
        self.net.resume(&self.nodes[i].0);
    }
    fn crash(&mut self, i: usize) {
        self.net.crash(&self.nodes[i].0);
    }
    fn restart(&mut self, i: usize) {
        let id = self.nodes[i].0.clone();
        let seed = if i == 0 { "n1:7946" } else { "n0:7946" };
        if let Some(h) = self.net.restart(
            &self.meta,
            &id,
            vertep_cluster::gossip::GossipConfig::default(),
            seed,
        ) {
            self.nodes[i].1 = h;
        }
    }
    fn cut_severs_data(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Journals and the world

/// Where shard journals live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Journals {
    /// One shared fenced store: shards move by lease transfer.
    Shared,
    /// A local fenced store per node and shard, replicated to the next `rf - 1` nodes.
    Local(usize),
}

impl Journals {
    fn label(self) -> String {
        match self {
            Journals::Shared => "shared journal".into(),
            Journals::Local(rf) => format!("local journals, replication factor {rf}"),
        }
    }
}

type Store = Arc<MemoryStore>;
type Replicated = ReplicatedJournal<Store, dyn Membership>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Up,
    Partitioned(u32),
    /// Paused in the middle of processing a message: it read the journal head before.
    Paused(u32),
    Crashed(u32),
}

/// What a node was doing when it paused.
#[derive(Clone, Debug)]
struct InFlight {
    message: String,
    actor: usize,
    head: Position,
}

/// A frame on the network: delivered after `delay` steps.
struct InTransit {
    delay: u32,
    receiver: usize,
    fence: Fence,
    message: String,
}

struct World {
    meta: ClusterMeta,
    journals: Journals,
    nodes: usize,
    shared: Store,
    /// Per node and shard: the node's own copy of the shard's journals ("its disk").
    local: Vec<Vec<Store>>,
    /// Per node and shard: the node's link to its standby replica of the shard.
    links: Vec<Vec<Arc<LocalStandby<Store>>>>,
    /// The replicated journal of each (owner, shard), kept while the owner runs.
    replicated: BTreeMap<(usize, ShardId), Arc<Replicated>>,
    actors: Vec<(ShardId, NamedNode)>,
    /// Client messages not yet acknowledged, with their target actor.
    pending: VecDeque<(String, usize)>,
    receivers: Vec<(NodeId, EpochFilter)>,
    network: Vec<InTransit>,
    history: History,
    /// Every lease grant recorded so far, as (node, shard, epoch).
    granted: BTreeSet<(NodeId, ShardId, u64)>,
}

impl World {
    fn new(meta: ClusterMeta, journals: Journals, nodes: usize) -> Self {
        let shards = meta.shard_count.get() as usize;
        let local: Vec<Vec<Store>> = (0..nodes)
            .map(|_| (0..shards).map(|_| Arc::new(MemoryStore::new())).collect())
            .collect();
        let links = (0..nodes)
            .map(|i| {
                (0..shards)
                    .map(|s| {
                        let replica = StandbyReplica::new(
                            meta.clone(),
                            ShardId::new(s as u32),
                            local[i][s].clone(),
                        );
                        Arc::new(LocalStandby::new(node(&format!("n{i}")), Arc::new(replica)))
                    })
                    .collect()
            })
            .collect();
        let actors = meta
            .shard_count
            .shards()
            .flat_map(|s| {
                [
                    (s, actor_in(&meta, s, "f-a")),
                    (s, actor_in(&meta, s, "f-b")),
                ]
            })
            .collect();
        World {
            meta,
            journals,
            nodes,
            shared: Arc::new(MemoryStore::new()),
            local,
            links,
            replicated: BTreeMap::new(),
            actors,
            pending: VecDeque::new(),
            receivers: vec![
                (node("r0"), EpochFilter::new()),
                (node("r1"), EpochFilter::new()),
            ],
            network: Vec::new(),
            history: History::new(),
            granted: BTreeSet::new(),
        }
    }

    /// The nodes that keep a copy of `shard`'s journal: its home node and the next ones.
    fn replica_set(&self, shard: ShardId) -> Vec<usize> {
        match self.journals {
            Journals::Shared => (0..self.nodes).collect(),
            Journals::Local(rf) => (0..rf).map(|k| (shard.index() + k) % self.nodes).collect(),
        }
    }

    /// Whether node `i` may take `shard`: only a node holding its journal.
    fn may_take(&self, i: usize, shard: ShardId) -> bool {
        self.replica_set(shard).contains(&i)
    }

    fn store(&self, i: usize, shard: ShardId) -> &Store {
        match self.journals {
            Journals::Shared => &self.shared,
            Journals::Local(_) => &self.local[i][shard.index()],
        }
    }

    /// Connects the replication links of the nodes that can answer.
    fn set_links(&self, up: &dyn Fn(usize) -> bool) {
        for (i, per_shard) in self.links.iter().enumerate() {
            for l in per_shard {
                l.set_up(up(i));
            }
        }
    }

    /// A restarted node's replicated journals start over (its stores, its disk, stay).
    fn restarted(&mut self, i: usize) {
        self.replicated.retain(|(o, _), _| *o != i);
    }

    /// Records the grant of every lease node `i` holds that the history does not have yet:
    /// leases it acquired itself, and those its membership acquired for it. Every append
    /// runs under a lease the node holds, so every append's epoch is then a recorded grant.
    fn record_grants(&mut self, id: &NodeId, m: &dyn Membership) {
        for s in self.meta.shard_count.shards() {
            if let Some(l) = m.held(s)
                && self.granted.insert((id.clone(), s, l.epoch))
            {
                self.history.record(HistoryEvent::Grant {
                    node: id.clone(),
                    shard: s,
                    epoch: l.epoch,
                });
            }
        }
    }

    fn processed(&self, i: usize, actor: usize, message: &str) -> bool {
        let (s, a) = &self.actors[actor];
        let suffix = format!("/entry/{message}");
        self.store(i, *s)
            .read(a, Position::ZERO)
            .unwrap()
            .iter()
            .any(|r| r.entry.name().as_str().ends_with(&suffix))
    }

    fn head(&self, i: usize, actor: usize) -> Position {
        let (s, a) = &self.actors[actor];
        self.store(i, *s).head(a).unwrap()
    }

    fn journal_append(
        &mut self,
        i: usize,
        m: &Arc<dyn Membership>,
        shard: ShardId,
        actor: &NamedNode,
        f: &InFlight,
    ) -> Result<Position, AppendError> {
        let entry = [sample_entry(actor, &f.message)];
        let meta = AppendMeta::none();
        match self.journals {
            Journals::Shared | Journals::Local(1) => {
                ShardJournal::new(self.store(i, shard).clone(), shard, m.clone())
                    .unwrap()
                    .append(actor, f.head, &entry, &meta)
            }
            Journals::Local(rf) => {
                let standbys: Vec<Arc<dyn StandbyLink>> = self
                    .replica_set(shard)
                    .into_iter()
                    .filter(|&j| j != i)
                    .map(|j| self.links[j][shard.index()].clone() as Arc<dyn StandbyLink>)
                    .collect();
                let store = self.store(i, shard).clone();
                let j = self
                    .replicated
                    .entry((i, shard))
                    .or_insert_with(|| {
                        Arc::new(ReplicatedJournal::new(
                            ShardJournal::new(store, shard, m.clone()).unwrap(),
                            standbys,
                            rf - 1,
                        ))
                    })
                    .clone();
                j.append(actor, f.head, &entry, &meta)
            }
        }
    }

    /// Appends the processing of `f` to its actor's journal at `f.head` on behalf of node
    /// `i`; on acceptance sends the side effect and maybe acknowledges.
    fn append(
        &mut self,
        i: usize,
        id: &NodeId,
        m: &Arc<dyn Membership>,
        f: &InFlight,
        rng: &mut Rng,
    ) {
        let (shard, actor) = self.actors[f.actor].clone();
        let Some(lease) = m.held(shard) else {
            return;
        };
        let decision = match self.journal_append(i, m, shard, &actor, f) {
            Ok(_) => AppendDecision::Accept,
            Err(AppendError::Fenced { .. }) => AppendDecision::Fenced,
            // a stale head, or the same message already appended (the entry IRI is taken)
            Err(AppendError::Conflict { .. } | AppendError::Invalid(_)) => AppendDecision::Conflict,
            // the lease lapsed by the node's clock, or the replication quorum is unreachable:
            // nothing was written
            Err(AppendError::Storage(_)) => return,
            Err(e) => panic!("unexpected append error: {e}"),
        };
        self.history.record(HistoryEvent::Append {
            node: id.clone(),
            shard,
            epoch: lease.epoch,
            actor: actor.as_str().to_owned(),
            version: f.head.get() + 1,
            message: f.message.clone(),
            decision,
        });
        if decision != AppendDecision::Accept {
            return;
        }
        // the side effect: a frame to one receiver, maybe delayed, maybe lost one way
        if !rng.chance(10) {
            self.network.push(InTransit {
                delay: rng.below(40) as u32,
                receiver: rng.below(self.receivers.len() as u64) as usize,
                fence: Fence {
                    shard,
                    epoch: lease.epoch,
                },
                message: f.message.clone(),
            });
        }
        // the acknowledgement to the client is lost now and then: the client retries
        if !rng.chance(20) {
            self.ack(&f.message);
        }
    }

    fn ack(&mut self, message: &str) {
        self.pending.retain(|(m, _)| m != message);
        self.history.record(HistoryEvent::Ack {
            message: message.to_owned(),
        });
    }

    fn deliver_frames(&mut self) {
        let mut keep = Vec::new();
        for mut t in std::mem::take(&mut self.network) {
            if t.delay > 0 {
                t.delay -= 1;
                keep.push(t);
                continue;
            }
            let (id, filter) = &self.receivers[t.receiver];
            let decision = match filter.admit(t.fence) {
                Ok(()) => FrameDecision::Admit,
                Err(_) => FrameDecision::Fenced,
            };
            self.history.record(HistoryEvent::Frame {
                receiver: id.clone(),
                shard: t.fence.shard,
                epoch: t.fence.epoch,
                message: t.message,
                decision,
            });
        }
        self.network = keep;
    }
}

/// How long a run is and how fast it goes.
#[derive(Clone, Copy, Debug)]
struct Pace {
    steps: u32,
    step_ms: u64,
}

/// Runs the fault schedule of `seed` against `b` with `journals`.
fn run(b: &mut dyn Backend, journals: Journals, seed: u64, pace: Pace) -> History {
    let m = b.member(0).meta().clone();
    let n = b.size();
    let mut w = World::new(m.clone(), journals, n);
    let mut rng = Rng(seed | 1);
    let mut states = vec![State::Up; n];
    let mut paused_in: Vec<Option<InFlight>> = vec![None; n];
    let mut next_message = 0u64;
    for _ in 0..pace.steps {
        let running = states.clone();
        b.advance(pace.step_ms, &|i| {
            matches!(running[i], State::Up | State::Partitioned(_))
        });
        // clients send
        if rng.chance(60) {
            let actor = rng.below(w.actors.len() as u64) as usize;
            w.pending.push_back((format!("m{next_message}"), actor));
            next_message += 1;
        }
        for i in 0..n {
            states[i] = match states[i] {
                State::Up if rng.chance(2) => {
                    b.cut(i, true);
                    State::Partitioned(rng.below(80) as u32 + 10)
                }
                State::Up if rng.chance(1) => {
                    // paused mid-operation: it read a message (one of a shard it holds, if
                    // any) and the head, then stalls
                    let m = b.member(i);
                    let busy = w
                        .pending
                        .iter()
                        .find(|(_, a)| m.held(w.actors[*a].0).is_some())
                        .or(w.pending.front())
                        .cloned();
                    if let Some((msg, actor)) = busy {
                        paused_in[i] = Some(InFlight {
                            message: msg,
                            head: w.head(i, actor),
                            actor,
                        });
                    }
                    b.pause(i);
                    State::Paused(rng.below(80) as u32 + 10)
                }
                State::Up if rng.chance(1) => {
                    b.crash(i);
                    State::Crashed(rng.below(80) as u32 + 10)
                }
                State::Partitioned(0) => {
                    b.cut(i, false);
                    State::Up
                }
                State::Partitioned(k) => State::Partitioned(k - 1),
                State::Paused(0) => State::Up,
                State::Paused(k) => State::Paused(k - 1),
                State::Crashed(0) => {
                    b.restart(i);
                    w.restarted(i);
                    State::Up
                }
                State::Crashed(k) => State::Crashed(k - 1),
                State::Up => State::Up,
            };
            let (id, member) = (b.id(i), b.member(i));
            if states[i] == State::Up
                && let Some(f) = paused_in[i].take()
            {
                // the resumed process finishes its append before it looks at the clock
                w.append(i, &id, &member, &f, &mut rng);
                b.resume(i);
            }
            if matches!(states[i], State::Paused(_) | State::Crashed(_)) {
                continue;
            }
            b.tick(i);
            w.record_grants(&id, member.as_ref());
            // work: process a pending message of a shard this node holds, or take a shard
            let candidate = w
                .pending
                .iter()
                .find(|(_, a)| member.held(w.actors[*a].0).is_some())
                .cloned();
            match candidate {
                Some((msg, actor)) => {
                    if w.processed(i, actor, &msg) {
                        // a retried message already in the journal: acknowledge, don't redo
                        w.ack(&msg);
                    } else {
                        let f = InFlight {
                            message: msg,
                            head: w.head(i, actor),
                            actor,
                        };
                        w.append(i, &id, &member, &f, &mut rng);
                    }
                }
                None => {
                    if let Some((_, actor)) = w.pending.front().cloned()
                        && rng.chance(40)
                    {
                        let s = w.actors[actor].0;
                        if w.may_take(i, s) && member.acquire(s).is_ok() {
                            w.record_grants(&id, member.as_ref());
                        }
                    }
                }
            }
            if rng.chance(2) {
                let s = ShardId::new(rng.below(u64::from(w.meta.shard_count.get())) as u32);
                if let Some(l) = member.held(s) {
                    let _ = member.release(&l);
                }
            }
        }
        let severs = b.cut_severs_data();
        w.set_links(&|j| match states[j] {
            State::Up => true,
            State::Partitioned(_) => !severs,
            State::Paused(_) | State::Crashed(_) => false,
        });
        w.deliver_frames();
    }
    w.history
}

fn add(total: &mut vertep_cluster::CheckReport, r: &vertep_cluster::CheckReport) {
    total.accepted += r.accepted;
    total.fenced += r.fenced;
    total.conflicts += r.conflicts;
    total.admitted += r.admitted;
    total.dropped += r.dropped;
    total.acked += r.acked;
}

/// The in-memory stand-in for an external authority, on a manual clock.
fn stand_in() -> AuthorityBackend<MemoryAuthority> {
    let m = meta(8);
    let clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        m.clone(),
        timing(),
        Arc::new(clock.clone()),
    ));
    AuthorityBackend::new(
        "stand-in",
        m,
        Some(clock),
        timing().lease,
        Box::new(move || authority.clone()),
    )
}

/// The static membership's hosted authority, on a manual clock.
fn static_members() -> AuthorityBackend<MemoryAuthority> {
    let config = StaticConfig {
        meta: meta(8),
        members: (0..4)
            .map(|i| (node(&format!("n{i}")), format!("n{i}")))
            .collect(),
        timing: timing(),
    };
    let clock = ManualClock::new();
    let authority = Arc::new(config.authority(Arc::new(clock.clone())));
    AuthorityBackend::new(
        "static",
        config.meta.clone(),
        Some(clock),
        timing().lease,
        Box::new(move || authority.clone()),
    )
}

const PACE: Pace = Pace {
    steps: 2_000,
    step_ms: 250,
};

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[test]
fn fault_injection_history_is_linear_once_and_unfenced() {
    let mut total = vertep_cluster::CheckReport::default();
    let lease = lease_oracle();
    for seed in [5, 17, 4242, 31_337, 77_777] {
        let h = run(&mut stand_in(), Journals::Shared, seed, PACE);
        add(&mut total, &verify(&seed.to_string(), &h, lease.as_deref()));
    }
    assert!(total.accepted > 1_000, "progress: {total:?}");
    assert!(total.acked > 1_000, "messages acknowledged: {total:?}");
    assert!(total.fenced > 0, "stale appends were fenced: {total:?}");
    assert!(
        total.conflicts > 0,
        "paused writers lost compare-and-set: {total:?}"
    );
    assert!(
        total.dropped > 0,
        "late frames were fenced at receivers: {total:?}"
    );
}

/// The same schedule on every in-process membership and journal mode.
fn matrix(make: &dyn Fn() -> Box<dyn Backend>, seeds: &[u64], pace: Pace) {
    let lease = lease_oracle();
    for journals in [Journals::Shared, Journals::Local(1), Journals::Local(3)] {
        let mut total = vertep_cluster::CheckReport::default();
        let mut label = String::new();
        for &seed in seeds {
            let mut b = make();
            label = b.label();
            let h = run(b.as_mut(), journals, seed, pace);
            let name = format!(
                "{label}-{}-{seed}",
                match journals {
                    Journals::Shared => "shared".to_owned(),
                    Journals::Local(rf) => format!("rf{rf}"),
                }
            );
            add(&mut total, &verify(&name, &h, lease.as_deref()));
        }
        eprintln!("{label}, {}: {total:?}", journals.label());
        assert!(
            total.accepted > 200,
            "{label}, {}: progress {total:?}",
            journals.label()
        );
        assert!(
            total.acked > 200,
            "{label}, {}: acknowledged {total:?}",
            journals.label()
        );
    }
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[test]
fn fault_injection_on_static_membership_with_replication_factors_1_and_3() {
    matrix(&|| Box::new(static_members()), &[5, 4242], PACE);
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[test]
fn fault_injection_on_the_external_stand_in_with_replication_factors_1_and_3() {
    matrix(&|| Box::new(stand_in()), &[17, 31_337], PACE);
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[cfg(feature = "gossip")]
#[test]
fn fault_injection_on_gossip_membership_with_replication_factors_1_and_3() {
    matrix(&|| Box::new(GossipBackend::new(meta(8))), &[3, 2024], PACE);
}

#[test]
fn histories_export_for_the_oracle() {
    let h = run(
        &mut stand_in(),
        Journals::Shared,
        9,
        Pace {
            steps: 400,
            step_ms: 250,
        },
    );
    let text = h.to_jsonl();
    let mut ops: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines() {
        assert!(
            line.starts_with("{\"op\":\"") && line.ends_with('}'),
            "{line}"
        );
        let op = line[7..].split('"').next().unwrap().to_owned();
        *ops.entry(op).or_default() += 1;
    }
    for op in ["grant", "append", "frame", "ack"] {
        assert!(
            ops.get(op).copied().unwrap_or(0) > 0,
            "{op} recorded: {ops:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Real services, on real time (feature `etcd` / `kubernetes`, `VERTEP_CONTAINER_TESTS=1`)

/// Lease 4 s with a 1 s margin, suspect after 2 s, failed after 4 s: whole seconds for etcd.
#[cfg(any(feature = "etcd", feature = "kubernetes"))]
fn live_timing() -> vertep_cluster::AuthorityConfig {
    use std::time::Duration;
    vertep_cluster::AuthorityConfig {
        lease: vertep_cluster::LeaseConfig {
            duration: Duration::from_secs(4),
            safety_margin: Duration::from_secs(1),
        },
        detector: vertep_cluster::DetectorConfig {
            suspicion_time: Duration::from_secs(2),
            failure_time: Duration::from_secs(4),
        },
    }
}

/// 450 steps of 100 ms of real time: the same fault rates, each fault lasting 1 to 9 s.
#[cfg(any(feature = "etcd", feature = "kubernetes"))]
const LIVE: Pace = Pace {
    steps: 450,
    step_ms: 100,
};

#[cfg(any(feature = "etcd", feature = "kubernetes"))]
fn live_matrix<A: Authority + 'static>(
    label: &str,
    client: impl Fn(&ClusterMeta) -> Arc<A> + Clone + 'static,
) {
    let lease = lease_oracle();
    let mut fenced = 0;
    for (journals, seed) in [
        (Journals::Shared, 5),
        (Journals::Local(1), 17),
        (Journals::Local(3), 4242),
    ] {
        let cluster = vertep_cluster::ClusterId::new(common::containers::unique("f")).unwrap();
        let m = ClusterMeta::new(cluster, 8).unwrap();
        let c = client.clone();
        let mm = m.clone();
        let mut b = AuthorityBackend::new(
            label,
            m,
            None,
            live_timing().lease,
            Box::new(move || c(&mm)),
        );
        let h = run(&mut b, journals, seed, LIVE);
        let name = format!("{label}-{seed}");
        let r = verify(&name, &h, lease.as_deref());
        eprintln!("{label}, {}: {r:?}", journals.label());
        // replication factor 3 needs its whole replica set up for every append: less progress
        assert!(
            r.accepted > 15,
            "{label}, {}: progress {r:?}",
            journals.label()
        );
        assert!(
            r.acked > 15,
            "{label}, {}: acknowledged {r:?}",
            journals.label()
        );
        fenced += r.fenced + r.dropped;
    }
    assert!(fenced > 0, "{label}: the fencing backstop was exercised");
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[cfg(feature = "etcd")]
#[test]
fn fault_injection_on_etcd_with_replication_factors_1_and_3() {
    use vertep_cluster::external::etcd::{EtcdAuthority, EtcdConfig};
    if !common::containers::enabled("fault injection on etcd") {
        return;
    }
    let etcd = Arc::new(common::containers::Etcd::start());
    live_matrix("etcd", move |m: &ClusterMeta| {
        let mut config = EtcdConfig::new(&etcd.endpoint);
        config.timeout = std::time::Duration::from_millis(500);
        Arc::new(
            EtcdAuthority::new(
                m.clone(),
                live_timing(),
                &config,
                Arc::new(vertep_cluster::MonotonicClock::new()),
            )
            .unwrap(),
        )
    });
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[cfg(feature = "kubernetes")]
#[test]
fn fault_injection_on_kubernetes_with_replication_factors_1_and_3() {
    use vertep_cluster::external::kubernetes::{KubeAuthority, KubeConfig};
    if !common::containers::enabled("fault injection on Kubernetes") {
        return;
    }
    let k3s = common::containers::K3s::shared();
    live_matrix("kubernetes", move |m: &ClusterMeta| {
        let mut config = KubeConfig::from_kubeconfig(&k3s.kubeconfig).unwrap();
        config.timeout = std::time::Duration::from_millis(500);
        Arc::new(
            KubeAuthority::new(
                m.clone(),
                live_timing(),
                &config,
                Arc::new(vertep_cluster::MonotonicClock::new()),
            )
            .unwrap(),
        )
    });
}
