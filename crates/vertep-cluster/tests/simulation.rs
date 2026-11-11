//! Deterministic fault injection: nodes acquire shards and append to a shared fenced journal
//! while the simulation partitions them from the lease authority, pauses them, crashes and
//! restarts them, and runs some of their clocks slow. A history checker then verifies that the
//! journal stayed linear and that no two writers ever shared an epoch.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use common::{actor_in, meta, node, timing};
use vertep_cluster::{
    AuthorityLink, AuthorityMembership, ClusterMeta, ManualClock, Membership, MemoryAuthority,
    NodeId, ShardId, ShardJournal,
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

/// One accepted append.
#[derive(Debug, Clone)]
struct Accepted {
    shard: ShardId,
    epoch: u64,
    node: NodeId,
}

/// Every append the store accepted, and the fenced ones.
#[derive(Debug, Default)]
struct History {
    accepted: Vec<Accepted>,
    fenced: usize,
    conflicts: usize,
}

impl History {
    /// The checks of the single-activation guarantee.
    fn check(&self, store: &MemoryStore, actors: &[NamedNode]) {
        let mut writers: BTreeMap<(ShardId, u64), BTreeSet<NodeId>> = BTreeMap::new();
        let mut last: BTreeMap<ShardId, u64> = BTreeMap::new();
        for a in &self.accepted {
            writers
                .entry((a.shard, a.epoch))
                .or_default()
                .insert(a.node.clone());
            let prev = last.insert(a.shard, a.epoch).unwrap_or(0);
            assert!(
                a.epoch >= prev,
                "{}: epoch {} accepted after {prev}",
                a.shard,
                a.epoch
            );
        }
        for ((s, e), w) in &writers {
            assert_eq!(w.len(), 1, "{s} epoch {e} had writers {w:?}");
        }
        let mut entries = 0;
        for actor in actors {
            let records = store.read(actor, Position::ZERO).unwrap();
            for (i, r) in records.iter().enumerate() {
                assert_eq!(
                    r.position,
                    Position::new(i as u64 + 1),
                    "{actor} is not linear"
                );
            }
            entries += records.len();
        }
        assert_eq!(
            entries,
            self.accepted.len(),
            "every accepted append is in a journal once"
        );
    }
}

type Node = AuthorityMembership<AuthorityLink<Arc<MemoryAuthority>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Up,
    Partitioned(u32),
    Paused(u32),
    Crashed(u32),
}

struct SimNode {
    id: NodeId,
    clock: ManualClock,
    link: AuthorityLink<Arc<MemoryAuthority>>,
    m: Arc<Node>,
    state: State,
    slow: bool,
}

fn new_member(
    authority: &Arc<MemoryAuthority>,
    m: &ClusterMeta,
    id: &NodeId,
    now: u64,
) -> (ManualClock, AuthorityLink<Arc<MemoryAuthority>>, Arc<Node>) {
    let clock = ManualClock::at(now);
    let link = AuthorityLink::new(authority.clone());
    let member = Arc::new(AuthorityMembership::new(
        link.clone(),
        m.clone(),
        id.clone(),
        id.as_str(),
        Arc::new(clock.clone()),
        timing().lease,
    ));
    (clock, link, member)
}

fn try_append(
    n: &SimNode,
    store: &Arc<MemoryStore>,
    actor: &NamedNode,
    shard: ShardId,
    tag: &str,
    history: &mut History,
) {
    let Some(lease) = n.m.held(shard) else {
        return;
    };
    let journal = ShardJournal::new(store.clone(), shard, n.m.clone()).unwrap();
    let head = store.head(actor).unwrap();
    match journal.append(
        actor,
        head,
        &[sample_entry(actor, tag)],
        &AppendMeta::none(),
    ) {
        Ok(_) => history.accepted.push(Accepted {
            shard,
            epoch: lease.epoch,
            node: n.id.clone(),
        }),
        Err(AppendError::Fenced { .. }) => history.fenced += 1,
        Err(AppendError::Conflict { .. }) => history.conflicts += 1,
        Err(e) => panic!("unexpected append error: {e}"),
    }
}

fn run_authority_simulation(seed: u64, steps: u32) -> History {
    let m = meta(8);
    let authority_clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        m.clone(),
        timing(),
        Arc::new(authority_clock.clone()),
    ));
    let store = Arc::new(MemoryStore::new());
    let actors: Vec<(ShardId, NamedNode)> = m
        .shard_count
        .shards()
        .flat_map(|s| [(s, actor_in(&m, s, "sim-a")), (s, actor_in(&m, s, "sim-b"))])
        .collect();
    let mut rng = Rng(seed | 1);
    let mut nodes: Vec<SimNode> = (0..4)
        .map(|i| {
            let id = node(&format!("n{i}"));
            let (clock, link, member) = new_member(&authority, &m, &id, 0);
            member.join().unwrap();
            SimNode {
                id,
                clock,
                link,
                m: member,
                state: State::Up,
                slow: i == 3,
            }
        })
        .collect();
    let mut history = History::default();
    let step_ms = 250;
    for step in 0..steps {
        authority_clock.advance_ms(step_ms);
        let now = authority_clock.now_ms();
        for n in &mut nodes {
            // faults
            n.state = match n.state {
                State::Up if rng.chance(2) => {
                    n.link.set_connected(false);
                    State::Partitioned(rng.below(80) as u32 + 10)
                }
                State::Up if rng.chance(1) => State::Paused(rng.below(80) as u32 + 10),
                State::Up if rng.chance(1) => State::Crashed(rng.below(80) as u32 + 10),
                State::Partitioned(0) => {
                    n.link.set_connected(true);
                    State::Up
                }
                State::Partitioned(k) => State::Partitioned(k - 1),
                State::Paused(0) => {
                    // the process resumes mid-operation and acts before it reads the clock
                    let (s, a) = &actors[rng.below(actors.len() as u64) as usize];
                    try_append(
                        n,
                        &store,
                        a,
                        *s,
                        &format!("{step}-{}-resume", n.id),
                        &mut history,
                    );
                    n.clock.set(now);
                    State::Up
                }
                State::Paused(k) => State::Paused(k - 1),
                State::Crashed(0) => {
                    // restart with an empty memory of leases
                    let (clock, link, member) = new_member(&authority, &m, &n.id, now);
                    n.clock = clock;
                    n.link = link;
                    n.m = member;
                    State::Up
                }
                State::Crashed(k) => State::Crashed(k - 1),
                s => s,
            };
            if matches!(n.state, State::Paused(_) | State::Crashed(_)) {
                continue;
            }
            // a slow clock runs at half speed: it believes in its leases too long
            n.clock
                .advance_ms(if n.slow { step_ms / 2 } else { step_ms });
            let _ = n.m.tick();
            // work: take a random shard or write to one held
            let (s, a) = &actors[rng.below(actors.len() as u64) as usize];
            if n.m.held(*s).is_some() {
                try_append(n, &store, a, *s, &format!("{step}-{}", n.id), &mut history);
            } else if rng.chance(30) {
                let _ = n.m.acquire(*s);
            }
            // a held shard is sometimes released
            if rng.chance(3)
                && let Some(l) = n.m.held(*s)
            {
                let _ = n.m.release(&l);
            }
        }
    }
    let all: Vec<NamedNode> = actors.into_iter().map(|(_, a)| a).collect();
    history.check(&store, &all);
    history
}

trait NowMs {
    fn now_ms(&self) -> u64;
}

impl NowMs for ManualClock {
    fn now_ms(&self) -> u64 {
        vertep_cluster::Clock::now(self).0
    }
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: integration}
#[test]
fn fault_injection_never_yields_two_writers() {
    let mut fenced = 0;
    let mut accepted = 0;
    for seed in [1, 7, 42, 1234, 99_991] {
        let h = run_authority_simulation(seed, 2_000);
        fenced += h.fenced;
        accepted += h.accepted.len();
    }
    assert!(
        accepted > 1_000,
        "the simulation made progress ({accepted} appends)"
    );
    assert!(fenced > 0, "the fencing backstop was exercised");
}

#[cfg(feature = "gossip")]
mod gossip {
    use super::*;
    use std::time::Duration;
    use vertep_cluster::gossip::{GossipConfig, GossipMembership, SimNetwork};

    fn run_gossip_simulation(seed: u64, rounds: u32) -> History {
        let m = meta(8);
        let mut net = SimNetwork::new(Duration::from_millis(10), Duration::from_millis(5));
        let nodes: Vec<(NodeId, GossipMembership)> = (0..5)
            .map(|i| {
                let id = node(&format!("n{i}"));
                let h = net.add_node(
                    &m,
                    id.clone(),
                    &format!("n{i}:7946"),
                    GossipConfig::default(),
                );
                h.node().join("n0:7946");
                (id, h)
            })
            .collect();
        net.run_for(Duration::from_secs(3));
        let store = Arc::new(MemoryStore::new());
        let actors: Vec<(ShardId, NamedNode)> = m
            .shard_count
            .shards()
            .map(|s| (s, actor_in(&m, s, "gsim")))
            .collect();
        let mut rng = Rng(seed | 1);
        let mut history = History::default();
        let mut paused: BTreeSet<NodeId> = BTreeSet::new();
        for round in 0..rounds {
            match rng.below(100) {
                0..=3 => {
                    // split the cluster in two
                    let cut = rng.below(4) as usize + 1;
                    let ids: Vec<NodeId> = nodes.iter().map(|(i, _)| i.clone()).collect();
                    net.partition(&[ids[..cut].to_vec(), ids[cut..].to_vec()]);
                }
                4..=7 => net.heal(),
                8..=9 => {
                    let id = nodes[rng.below(5) as usize].0.clone();
                    net.pause(&id);
                    paused.insert(id);
                }
                10..=13 => {
                    if let Some(id) = paused.pop_first() {
                        // resumed: one action on the stale clock first
                        let h = &nodes.iter().find(|(i, _)| *i == id).unwrap().1;
                        let (s, a) = &actors[rng.below(8) as usize];
                        append(h, &id, &store, a, *s, &format!("{round}-r"), &mut history);
                        net.resume(&id);
                    }
                }
                _ => {}
            }
            net.run_for(Duration::from_millis(100));
            for (id, h) in &nodes {
                if paused.contains(id) {
                    continue;
                }
                let (s, a) = &actors[rng.below(8) as usize];
                if h.held(*s).is_some() {
                    append(h, id, &store, a, *s, &format!("{round}-{id}"), &mut history);
                } else if rng.chance(20) {
                    let _ = h.acquire(*s);
                }
            }
        }
        let all: Vec<NamedNode> = actors.into_iter().map(|(_, a)| a).collect();
        history.check(&store, &all);
        history
    }

    fn append(
        h: &GossipMembership,
        id: &NodeId,
        store: &Arc<MemoryStore>,
        actor: &NamedNode,
        shard: ShardId,
        tag: &str,
        history: &mut History,
    ) {
        let Some(lease) = h.held(shard) else {
            return;
        };
        let journal = ShardJournal::new(store.clone(), shard, Arc::new(h.clone())).unwrap();
        let head = store.head(actor).unwrap();
        match journal.append(
            actor,
            head,
            &[sample_entry(actor, tag)],
            &AppendMeta::none(),
        ) {
            Ok(_) => history.accepted.push(Accepted {
                shard,
                epoch: lease.epoch,
                node: id.clone(),
            }),
            Err(AppendError::Fenced { .. }) => history.fenced += 1,
            Err(AppendError::Conflict { .. }) => history.conflicts += 1,
            Err(e) => panic!("unexpected append error: {e}"),
        }
    }

    #[test]
    fn gossip_partitions_never_yield_two_writers_per_epoch() {
        let (mut accepted, mut fenced) = (0, 0);
        for seed in [3, 11, 2024] {
            let h = run_gossip_simulation(seed, 600);
            accepted += h.accepted.len();
            fenced += h.fenced;
        }
        assert!(
            accepted > 300,
            "the simulation made progress ({accepted} appends)"
        );
        assert!(
            fenced > 0,
            "leaders on both sides of a partition were fenced apart"
        );
    }
}
