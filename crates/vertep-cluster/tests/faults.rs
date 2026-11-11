//! Jepsen-style fault injection, in process: four nodes lease shards from an authority and
//! process client messages into a shared fenced journal, sending a side effect to one of two
//! receivers for each, while the run partitions nodes from the authority, pauses them in the
//! middle of an append, crashes and restarts them, runs one clock at half speed (skew beyond
//! the safety margin), delays and drops frames one way, and loses acknowledgements so that
//! clients retry. The recorded history is then checked: journals linear, one writer per
//! epoch, every acknowledged message processed once, no fenced side effect accepted. Every
//! history is also replayed through the Lean lease and fencing model (`vertep-lease`, strict
//! mode: every append under a recorded grant), which must take the same accept, fence and
//! admit decisions; the test skips that when the Lean executable is not built, unless the
//! `differential` feature requires it. Set `VERTEP_HISTORY_DIR` to keep each run's history as
//! JSON lines.

mod common;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use common::{actor_in, meta, node, timing};
use vertep_cluster::{
    AppendDecision, AuthorityLink, AuthorityMembership, ClusterMeta, EpochFilter, Fence,
    FrameDecision, History, HistoryEvent, ManualClock, Membership, MemoryAuthority, NodeId,
    ShardId, ShardJournal,
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

type Member = AuthorityMembership<AuthorityLink<Arc<MemoryAuthority>>>;

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

struct SimNode {
    id: NodeId,
    clock: ManualClock,
    link: AuthorityLink<Arc<MemoryAuthority>>,
    m: Arc<Member>,
    state: State,
    slow: bool,
    paused_in: Option<InFlight>,
}

fn new_member(
    authority: &Arc<MemoryAuthority>,
    m: &ClusterMeta,
    id: &NodeId,
    now: u64,
) -> (
    ManualClock,
    AuthorityLink<Arc<MemoryAuthority>>,
    Arc<Member>,
) {
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

/// A frame on the network: delivered after `delay` steps.
struct InTransit {
    delay: u32,
    receiver: usize,
    fence: Fence,
    message: String,
}

struct World {
    meta: ClusterMeta,
    store: Arc<MemoryStore>,
    actors: Vec<(ShardId, NamedNode)>,
    /// Client messages not yet acknowledged, with their target actor.
    pending: VecDeque<(String, usize)>,
    receivers: Vec<(NodeId, EpochFilter)>,
    network: Vec<InTransit>,
    history: History,
    /// Every lease grant recorded so far, as (node, shard, epoch).
    granted: std::collections::BTreeSet<(NodeId, ShardId, u64)>,
}

impl World {
    /// Records the grant of every lease `n` holds that the history does not have yet: leases
    /// it acquired itself, and those `Membership::tick` acquired for it (shards the view hands
    /// to it, leases it learns again after a rejoin). Every append runs under a lease the node
    /// holds, so every append's epoch is then a recorded grant.
    fn record_grants(&mut self, n: &SimNode) {
        for s in self.meta.shard_count.shards() {
            if let Some(l) = n.m.held(s)
                && self.granted.insert((n.id.clone(), s, l.epoch))
            {
                self.history.record(HistoryEvent::Grant {
                    node: n.id.clone(),
                    shard: s,
                    epoch: l.epoch,
                });
            }
        }
    }

    fn processed(&self, actor: usize, message: &str) -> bool {
        let a = &self.actors[actor].1;
        let suffix = format!("/entry/{message}");
        self.store
            .read(a, Position::ZERO)
            .unwrap()
            .iter()
            .any(|r| r.entry.name().as_str().ends_with(&suffix))
    }

    /// Appends the processing of `message` to its actor's journal at `head` on behalf of `n`;
    /// on acceptance sends the side effect and maybe acknowledges.
    fn append(&mut self, n: &SimNode, f: &InFlight, rng: &mut Rng) {
        let (shard, actor) = self.actors[f.actor].clone();
        let Some(lease) = n.m.held(shard) else {
            return;
        };
        let journal = ShardJournal::new(self.store.clone(), shard, n.m.clone()).unwrap();
        let decision = match journal.append(
            &actor,
            f.head,
            &[sample_entry(&actor, &f.message)],
            &AppendMeta::none(),
        ) {
            Ok(_) => AppendDecision::Accept,
            Err(AppendError::Fenced { .. }) => AppendDecision::Fenced,
            // a stale head, or the same message already appended (the entry IRI is taken)
            Err(AppendError::Conflict { .. } | AppendError::Invalid(_)) => AppendDecision::Conflict,
            Err(AppendError::Storage(_)) => return, // the lease lapsed by the node's clock
            Err(e) => panic!("unexpected append error: {e}"),
        };
        self.history.record(HistoryEvent::Append {
            node: n.id.clone(),
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

#[allow(clippy::needless_range_loop)] // nodes are borrowed one at a time, by index
fn run(seed: u64, steps: u32) -> History {
    let m = meta(8);
    let authority_clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        m.clone(),
        timing(),
        Arc::new(authority_clock.clone()),
    ));
    let actors: Vec<(ShardId, NamedNode)> = m
        .shard_count
        .shards()
        .flat_map(|s| [(s, actor_in(&m, s, "f-a")), (s, actor_in(&m, s, "f-b"))])
        .collect();
    let mut w = World {
        meta: m.clone(),
        store: Arc::new(MemoryStore::new()),
        actors,
        pending: VecDeque::new(),
        receivers: vec![
            (node("r0"), EpochFilter::new()),
            (node("r1"), EpochFilter::new()),
        ],
        network: Vec::new(),
        history: History::new(),
        granted: std::collections::BTreeSet::new(),
    };
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
                paused_in: None,
            }
        })
        .collect();
    let step_ms = 250;
    let mut next_message = 0u64;
    for _ in 0..steps {
        authority_clock.advance_ms(step_ms);
        let now = vertep_cluster::Clock::now(&authority_clock).0;
        // clients send
        if rng.chance(60) {
            let actor = rng.below(w.actors.len() as u64) as usize;
            w.pending.push_back((format!("m{next_message}"), actor));
            next_message += 1;
        }
        for i in 0..nodes.len() {
            let n = &mut nodes[i];
            n.state = match n.state {
                State::Up if rng.chance(2) => {
                    n.link.set_connected(false);
                    State::Partitioned(rng.below(80) as u32 + 10)
                }
                State::Up if rng.chance(1) => {
                    // paused mid-operation: it read a message and the head, then stalls
                    if let Some((msg, actor)) = w.pending.front().cloned() {
                        let head = w.store.head(&w.actors[actor].1).unwrap();
                        n.paused_in = Some(InFlight {
                            message: msg,
                            actor,
                            head,
                        });
                    }
                    State::Paused(rng.below(80) as u32 + 10)
                }
                State::Up if rng.chance(1) => State::Crashed(rng.below(80) as u32 + 10),
                State::Partitioned(0) => {
                    n.link.set_connected(true);
                    State::Up
                }
                State::Partitioned(k) => State::Partitioned(k - 1),
                State::Paused(0) => State::Up,
                State::Paused(k) => State::Paused(k - 1),
                State::Crashed(0) => {
                    let (clock, link, member) = new_member(&authority, &m, &n.id, now);
                    n.clock = clock;
                    n.link = link;
                    n.m = member;
                    State::Up
                }
                State::Crashed(k) => State::Crashed(k - 1),
                s => s,
            };
            if let (State::Up, Some(f)) = (n.state, n.paused_in.clone()) {
                // the resumed process finishes its append before it looks at the clock
                n.paused_in = None;
                let n = &nodes[i];
                w.append(n, &f, &mut rng);
                nodes[i].clock.set(now);
            }
            let n = &mut nodes[i];
            if matches!(n.state, State::Paused(_) | State::Crashed(_)) {
                continue;
            }
            n.clock
                .advance_ms(if n.slow { step_ms / 2 } else { step_ms });
            let _ = n.m.tick();
            let n = &nodes[i];
            w.record_grants(n);
            // work: process a pending message of a shard this node holds, or take a shard
            let candidate = w
                .pending
                .iter()
                .find(|(_, a)| n.m.held(w.actors[*a].0).is_some())
                .cloned();
            match candidate {
                Some((msg, actor)) => {
                    if w.processed(actor, &msg) {
                        // a retried message already in the journal: acknowledge, don't redo
                        w.ack(&msg);
                    } else {
                        let head = w.store.head(&w.actors[actor].1).unwrap();
                        let f = InFlight {
                            message: msg,
                            actor,
                            head,
                        };
                        w.append(n, &f, &mut rng);
                    }
                }
                None => {
                    if let Some((_, actor)) = w.pending.front().cloned()
                        && rng.chance(40)
                    {
                        let s = w.actors[actor].0;
                        if n.m.acquire(s).is_ok() {
                            w.record_grants(n);
                        }
                    }
                }
            }
            if rng.chance(2) {
                let s = ShardId::new(rng.below(u64::from(w.meta.shard_count.get())) as u32);
                if let Some(l) = n.m.held(s) {
                    let _ = n.m.release(&l);
                }
            }
        }
        w.deliver_frames();
    }
    w.history
}

/// The Lean lease checker `vertep-lease`, or `None` (with a message) when it is not built,
/// unless the `differential` feature requires it.
fn lease_oracle() -> Option<std::path::PathBuf> {
    let required = cfg!(feature = "differential");
    let tools = vertep_conformance::LeanTools::for_test(required)?;
    if tools.lease.is_file() {
        Some(tools.lease)
    } else if required {
        panic!(
            "{} is not built: run `cd lean/vertep-lean && lake build vertep-lease`",
            tools.lease.display()
        )
    } else {
        eprintln!("skipped: {} is not built", tools.lease.display());
        None
    }
}

/// Replays `h` through the Lean lease model in strict mode (every append under a recorded
/// grant): `Ok` with its summary when every decision agrees, `Err` with its reason otherwise.
fn lean_lease_verdict(lease: &std::path::Path, h: &History) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("history.jsonl");
    std::fs::write(&path, h.to_jsonl()).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(lease)
        .arg("--history")
        .arg(&path)
        .output()
        .map_err(|e| format!("{}: {e}", lease.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[test]
fn fault_injection_history_is_linear_once_and_unfenced() {
    let mut total = vertep_cluster::CheckReport::default();
    let lease = lease_oracle();
    for seed in [5, 17, 4242, 31_337, 77_777] {
        let h = run(seed, 2_000);
        let r = h.check().unwrap_or_else(|v| panic!("seed {seed}: {v}"));
        if let Ok(dir) = std::env::var("VERTEP_HISTORY_DIR") {
            let path = std::path::Path::new(&dir).join(format!("cluster-history-{seed}.jsonl"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(path, h.to_jsonl()).unwrap();
        }
        // the Lean lease and fencing model takes the same decisions (m4-cluster 6.5)
        if let Some(lease) = &lease {
            lean_lease_verdict(lease, &h)
                .unwrap_or_else(|e| panic!("seed {seed}: the Lean lease model disagrees: {e}"));
        }
        assert_eq!(h.to_jsonl().lines().count(), h.events.len());
        total.accepted += r.accepted;
        total.fenced += r.fenced;
        total.conflicts += r.conflicts;
        total.admitted += r.admitted;
        total.dropped += r.dropped;
        total.acked += r.acked;
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

#[test]
fn histories_export_for_the_oracle() {
    let h = run(9, 400);
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
