//! Journal shipping (M13 tasks 7.1-7.3, 7.5): a follower pulls a shard's journal from its
//! leader in order with digests, resumes from its durable position after a dropped connection
//! or a restart, and a slow follower bounds what the leader holds for it. The fault-injection
//! run at the end records the follower's decisions as a history and replays it through the Lean
//! lease and fencing model in strict mode (`vertep-lease`, skipped when it is not built, unless
//! the `differential` feature requires it).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::oracle::{lease_oracle, verify};
use common::{actor_in, meta, node};
use vertep_cluster::{
    ClusterMeta, History, HistoryEvent, LeaseConfig, LeaseTable, LocalShipLink, Millis, NodeId,
    PullBatch, PullError, PullRequest, ShardId, ShipFollower, ShipLeader, ShipLink, Verdict,
    ship_journal,
};
use vertep_store::oxrdf::{Literal, NamedNode, Triple};
use vertep_store::testkit::sample_entry;
use vertep_store::{AppendMeta, Entry, JournalStore, MemoryStore, Position};
use vertep_tiramemsu::TiramemsuShard;

const SHARD: ShardId = ShardId::new(3);
const WINDOW: u32 = 8192;

type Leader = Arc<ShipLeader<Arc<MemoryStore>>>;

fn setup() -> (ClusterMeta, NamedNode) {
    let m = meta(8);
    let actor = actor_in(&m, SHARD, "ship");
    (m, actor)
}

/// Appends entries `from..=to` (tagged `e<i>`) to `store`.
fn write(store: &dyn JournalStore, actor: &NamedNode, from: u64, to: u64) {
    for i in from..=to {
        store
            .append(
                actor,
                Position::new(i - 1),
                &[sample_entry(actor, &format!("e{i}"))],
                &AppendMeta::none(),
            )
            .unwrap();
    }
}

fn leader_with(actor: &NamedNode, n: u64, epoch: u64) -> Leader {
    let store = Arc::new(MemoryStore::new());
    write(store.as_ref(), actor, 1, n);
    let l = Arc::new(ShipLeader::new(node("n1"), SHARD, store, 1 << 20));
    l.set_fence(Some(epoch));
    l
}

fn follower_over<J: JournalStore>(m: &ClusterMeta, store: J) -> ShipFollower<J> {
    ShipFollower::new(node("f"), m.clone(), SHARD, store, WINDOW)
}

fn fenced_store() -> Arc<MemoryStore> {
    Arc::new(MemoryStore::new().with_fencing(true))
}

/// The follower's journal is the leader's: same head, same digest at every position.
fn same_journal(a: &dyn JournalStore, b: &dyn JournalStore, actor: &NamedNode) {
    let (ra, rb) = (
        a.read(actor, Position::ZERO).unwrap(),
        b.read(actor, Position::ZERO).unwrap(),
    );
    assert_eq!(ra.len(), rb.len(), "heads differ");
    for (x, y) in ra.iter().zip(&rb) {
        assert_eq!(x.position, y.position);
        assert_eq!(
            x.entry.digest(),
            y.entry.digest(),
            "entry {}",
            x.position.get()
        );
    }
}

type Fault = Box<
    dyn FnMut(&PullRequest, Result<PullBatch, PullError>) -> Result<PullBatch, PullError> + Send,
>;

/// A link that passes every answer through a fault.
struct Faulty<L> {
    inner: L,
    fault: Mutex<Fault>,
}

impl<L: ShipLink> Faulty<L> {
    fn new(inner: L, fault: Fault) -> Self {
        Faulty {
            inner,
            fault: Mutex::new(fault),
        }
    }
}

impl<L: ShipLink> ShipLink for Faulty<L> {
    fn leader(&self) -> String {
        self.inner.leader()
    }
    fn pull(&self, request: &PullRequest) -> Result<PullBatch, PullError> {
        let answer = self.inner.pull(request);
        (self.fault.lock().unwrap())(request, answer)
    }
}

/// `entry` with one statement added, its digest left as it was.
fn tamper(entry: &Entry) -> Entry {
    let mut triples: Vec<Triple> = entry.triples().iter().map(|t| t.into_owned()).collect();
    triples.push(Triple::new(
        entry.name().clone(),
        NamedNode::new_unchecked("urn:test:tampered"),
        Literal::from("yes"),
    ));
    Entry::new(entry.name().clone(), triples)
}

// @tg: verifies:: [[openspec:journal-shipping#Ordered Shipping#A follower catches up]] {level: unit}
#[test]
fn a_follower_catches_up() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 10, 1);
    let follower = follower_over(&m, fenced_store());
    let link = LocalShipLink::new(node("f"), leader.clone());
    follower
        .catch_up(&link, &actor, 100, &mut |_, _| {})
        .unwrap();
    assert_eq!(follower.position(&actor).unwrap(), Position::new(10));

    // the follower holds 1 to 10; the leader moves on to 25
    write(leader.store().as_ref(), &actor, 11, 25);
    let mut firsts = Vec::new();
    let r = follower
        .catch_up(&link, &actor, 100, &mut |b, a| {
            firsts.extend(b.entries.first().map(|e| e.position.get()));
            assert!(a.refusal().is_none());
        })
        .unwrap();
    assert_eq!(
        firsts.first(),
        Some(&11),
        "shipping starts after the follower's position"
    );
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "batches in order");
    assert_eq!((r.head, r.stored, r.refused), (Position::new(25), 15, 0));
    same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
    // the last pull acknowledged the whole journal
    assert_eq!(leader.acknowledged(&node("f"), &actor), Position::new(25));
}

// @tg: verifies:: [[openspec:journal-shipping#Ordered Shipping#A gap is refused]] {level: unit}
#[test]
fn a_gap_is_refused() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 25, 1);
    let follower = follower_over(&m, fenced_store());
    write(follower.store().as_ref(), &actor, 1, 10);
    let dropped = Arc::new(AtomicBool::new(false));
    let d = dropped.clone();
    // the network loses entry 12, in the middle of a batch, once
    let link = Faulty::new(
        LocalShipLink::new(node("f"), leader.clone()),
        Box::new(move |_, answer| {
            let mut b = answer?;
            if !d.load(Ordering::SeqCst) && b.entries.iter().any(|e| e.position.get() == 13) {
                b.entries.retain(|e| e.position.get() != 12);
                d.store(true, Ordering::SeqCst);
            }
            Ok(b)
        }),
    );
    let mut refusals = Vec::new();
    let mut repulls = Vec::new();
    let r = follower
        .catch_up(&link, &actor, 100, &mut |b, a| {
            repulls.push(b.after.get());
            refusals.extend(a.refusal().cloned());
        })
        .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].position, Position::new(13));
    assert_eq!(
        refusals[0].verdict,
        Verdict::OutOfOrder {
            expected: Position::new(12)
        }
    );
    // it re-pulled from its last accepted position, 11
    assert_eq!(&repulls[..2], &[10, 11]);
    assert_eq!(r.head, Position::new(25));
    same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
}

// @tg: verifies:: [[openspec:journal-shipping#Ordered Shipping#A bad digest is refused]] {level: unit}
#[test]
fn a_bad_digest_is_refused() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 25, 1);
    let follower = follower_over(&m, fenced_store());
    write(follower.store().as_ref(), &actor, 1, 10);
    let corrupted = Arc::new(AtomicBool::new(false));
    let c = corrupted.clone();
    let link = Faulty::new(
        LocalShipLink::new(node("f"), leader.clone()),
        Box::new(move |_, answer| {
            let mut b = answer?;
            if !c.load(Ordering::SeqCst)
                && let Some(e) = b.entries.iter_mut().find(|e| e.position.get() == 12)
            {
                e.entry = tamper(&e.entry);
                c.store(true, Ordering::SeqCst);
            }
            Ok(b)
        }),
    );
    // one pull: the tampered entry is refused and nothing from it on is appended
    let (batch, accepted) = follower.pull_once(&link, &actor).unwrap();
    assert!(corrupted.load(Ordering::SeqCst));
    let refusal = accepted.refusal().unwrap();
    assert_eq!(
        (refusal.position, refusal.verdict),
        (Position::new(12), Verdict::BadDigest)
    );
    assert_eq!(accepted.head, Position::new(11));
    assert_eq!(follower.position(&actor).unwrap(), Position::new(11));
    assert!(batch.entries.len() > 2);
    // the next pulls bring the real entry 12
    follower
        .catch_up(&link, &actor, 100, &mut |_, _| {})
        .unwrap();
    same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
}

#[test]
fn a_diverged_journal_and_a_superseded_leader_are_refused() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 5, 2);
    let follower = follower_over(&m, fenced_store());
    // the follower holds a different entry 1
    follower
        .store()
        .append(
            &actor,
            Position::ZERO,
            &[sample_entry(&actor, "other")],
            &AppendMeta::none(),
        )
        .unwrap();
    let link = LocalShipLink::new(node("f"), leader.clone());
    let (_, a) = follower.pull_once(&link, &actor).unwrap();
    assert_eq!(a.refusal().unwrap().verdict, Verdict::Diverged);
    assert_eq!(a.head, Position::new(1));

    // a follower that stored epoch 2 refuses a stale leader at epoch 1
    let follower = follower_over(&m, fenced_store());
    follower
        .catch_up(&link, &actor, 100, &mut |_, _| {})
        .unwrap();
    let stale = leader_with(&actor, 7, 1);
    let (_, a) = follower
        .pull_once(&LocalShipLink::new(node("f"), stale), &actor)
        .unwrap();
    assert_eq!(a.refusal().unwrap().verdict, Verdict::Fenced { highest: 2 });
    assert_eq!(a.head, Position::new(5));

    // a leader without its lease serves nothing
    leader.set_fence(None);
    assert_eq!(
        link.pull(&follower.request(&actor).unwrap()),
        Err(PullError::NotLeader { shard: SHARD })
    );
}

/// A link to `leader` that delivers entries up to `cut_after` and then fails until `up` is
/// set again.
fn cutting_link(
    leader: Leader,
    cut_after: u64,
    up: Arc<AtomicBool>,
) -> Faulty<LocalShipLink<Leader>> {
    let fired = Arc::new(AtomicBool::new(false));
    Faulty::new(
        LocalShipLink::new(node("f"), leader),
        Box::new(move |_, answer| {
            if !up.load(Ordering::SeqCst) {
                return Err(PullError::Unreachable("connection dropped".into()));
            }
            let mut b = answer?;
            if !fired.load(Ordering::SeqCst)
                && b.entries.iter().any(|e| e.position.get() > cut_after)
            {
                // the connection fails after entry `cut_after` reached the follower
                b.entries.retain(|e| e.position.get() <= cut_after);
                fired.store(true, Ordering::SeqCst);
                up.store(false, Ordering::SeqCst);
            }
            Ok(b)
        }),
    )
}

// @tg: verifies:: [[openspec:journal-shipping#Resumption#Connection drops mid-ship]] {level: unit}
#[test]
fn connection_drops_mid_ship() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 25, 1);
    let follower = follower_over(&m, fenced_store());
    let up = Arc::new(AtomicBool::new(true));
    let link = cutting_link(leader.clone(), 17, up.clone());
    let mut h = History::new();
    h.record(HistoryEvent::Grant {
        node: node("n1"),
        shard: SHARD,
        epoch: 1,
    });
    let err = follower
        .catch_up(&link, &actor, 100, &mut |b, a| a.record(b, &mut h))
        .unwrap_err();
    assert!(matches!(err, PullError::Unreachable(_)));
    assert_eq!(follower.position(&actor).unwrap(), Position::new(17));

    up.store(true, Ordering::SeqCst);
    let mut first = None;
    follower
        .catch_up(&link, &actor, 100, &mut |b, a| {
            first.get_or_insert(b.after);
            a.record(b, &mut h);
        })
        .unwrap();
    assert_eq!(first, Some(Position::new(17)), "resumes after 17");
    same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
    // 25 entries, each applied once: 25 acknowledged appends, no conflict
    let r = verify("shipping-drop", &h, lease_oracle().as_deref());
    assert_eq!((r.accepted, r.conflicts, r.acked), (25, 0, 25));
}

// @tg: verifies:: [[openspec:journal-shipping#Resumption#Follower restarts]] {level: integration}
#[test]
fn follower_restarts() {
    let (m, actor) = setup();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("follower.db");
    let leader = leader_with(&actor, 25, 1);
    let up = Arc::new(AtomicBool::new(true));
    let link = cutting_link(leader.clone(), 17, up.clone());
    let mut h = History::new();
    h.record(HistoryEvent::Grant {
        node: node("n1"),
        shard: SHARD,
        epoch: 1,
    });
    {
        let follower = follower_over(&m, TiramemsuShard::open(&path).unwrap());
        let _ = follower.catch_up(&link, &actor, 100, &mut |b, a| a.record(b, &mut h));
        assert_eq!(follower.position(&actor).unwrap(), Position::new(17));
        // the follower process stops: its store file is all that is left
    }
    up.store(true, Ordering::SeqCst);
    let follower = follower_over(&m, TiramemsuShard::open(&path).unwrap());
    assert_eq!(follower.request(&actor).unwrap().after, Position::new(17));
    let mut first = None;
    follower
        .catch_up(&link, &actor, 100, &mut |b, a| {
            first.get_or_insert(b.entries.first().map(|e| e.position));
            a.record(b, &mut h);
        })
        .unwrap();
    assert_eq!(first, Some(Some(Position::new(18))), "resumes from 18");
    same_journal(leader.store().as_ref(), follower.store(), &actor);
    let r = verify("shipping-restart", &h, lease_oracle().as_deref());
    assert_eq!((r.accepted, r.conflicts, r.acked), (25, 0, 25));
}

// @tg: verifies:: [[openspec:journal-shipping#Back Pressure#A slow follower]] {level: unit}
#[test]
fn a_slow_follower() {
    let (m, actor) = setup();
    let leader = leader_with(&actor, 0, 1);
    let follower = follower_over(&m, fenced_store());
    let link = LocalShipLink::new(node("f"), leader.clone());
    // the leader writes 20 entries for every pull the follower makes: the follower falls ever
    // further behind
    let mut written = 0;
    for _ in 0..40 {
        write(leader.store().as_ref(), &actor, written + 1, written + 20);
        written += 20;
        let (b, _) = follower.pull_once(&link, &actor).unwrap();
        assert!(b.wire_size() <= WINDOW as usize);
        assert!(leader.buffered(&node("f")) <= WINDOW as usize);
    }
    let behind = written - follower.position(&actor).unwrap().get();
    assert!(behind > 500, "the follower is {behind} entries behind");
    // whatever the lag, the leader never held more than one window for it
    let peak = leader.peak_buffered(&node("f"));
    assert!(peak > 0 && peak <= WINDOW as usize, "peak {peak}");
    // it catches up once it speeds up, and its last pull leaves nothing held
    follower
        .catch_up(&link, &actor, 10_000, &mut |_, _| {})
        .unwrap();
    same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
    assert_eq!(leader.buffered(&node("f")), 0);
    assert!(leader.peak_buffered(&node("f")) <= WINDOW as usize);
    // an entry larger than the window is refused, not split or sent over it
    let tiny = PullRequest {
        window: 16,
        after: Position::ZERO,
        ..follower.request(&actor).unwrap()
    };
    assert!(matches!(link.pull(&tiny), Err(PullError::TooLarge { .. })));
}

#[test]
fn messages_round_trip() {
    let (_, actor) = setup();
    let leader = leader_with(&actor, 3, 4);
    let req = PullRequest {
        shard: SHARD,
        actor: actor.clone(),
        after: Position::new(1),
        window: WINDOW,
    };
    assert_eq!(PullRequest::decode(&req.encode()).unwrap(), req);
    let batch = leader.serve_for_test(&req);
    let bytes = vertep_cluster::shipping::encode_reply(&Ok(batch.clone()));
    let back = vertep_cluster::shipping::decode_reply(&bytes)
        .unwrap()
        .unwrap();
    assert_eq!(back.prev, batch.prev);
    assert_eq!(back.entries.len(), 2);
    for (x, y) in back.entries.iter().zip(&batch.entries) {
        assert_eq!((x.position, x.digest), (y.position, y.digest));
        assert_eq!(x.entry.digest(), y.digest, "the entry survives the wire");
    }
    for e in [
        PullError::NotLeader { shard: SHARD },
        PullError::Behind {
            head: Position::new(2),
        },
        PullError::TooLarge {
            position: Position::new(1),
            size: 9,
            window: 4,
        },
        PullError::Store("x".into()),
    ] {
        let bytes = vertep_cluster::shipping::encode_reply(&Err(e.clone()));
        assert_eq!(
            vertep_cluster::shipping::decode_reply(&bytes).unwrap(),
            Err(e)
        );
    }
}

trait ServeForTest {
    fn serve_for_test(&self, req: &PullRequest) -> PullBatch;
}

impl ServeForTest for Leader {
    fn serve_for_test(&self, req: &PullRequest) -> PullBatch {
        use vertep_cluster::PullSource;
        self.serve(&node("f"), req).unwrap()
    }
}

/// xorshift, as in the cluster fault-injection runs.
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
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One fault-injection run: a leader writes and ships; partway the lease is handed over to a
/// second leader holding a copy of the journal, while the first keeps serving as a stale
/// leader; the network drops connections, loses and tampers with entries and replays old
/// batches; the follower restarts. Returns the recorded history and what the faults did.
fn faulty_run(seed: u64) -> (History, [usize; 4]) {
    let (m, actor) = setup();
    let mut rng = Rng(seed | 1);
    let mut table = LeaseTable::new(m.shard_count, LeaseConfig::default());
    let mut h = History::new();
    let (n1, n2, f): (NodeId, NodeId, NodeId) = (node("n1"), node("n2"), node("f"));
    let lease1 = table.acquire(SHARD, &n1, Millis(0)).unwrap();
    h.record(HistoryEvent::Grant {
        node: n1.clone(),
        shard: SHARD,
        epoch: lease1.epoch,
    });
    let s1 = Arc::new(MemoryStore::new());
    let l1 = Arc::new(ShipLeader::new(n1.clone(), SHARD, s1.clone(), 1 << 20));
    l1.set_fence(Some(lease1.epoch));
    let mut l2: Option<Arc<ShipLeader<Arc<MemoryStore>>>> = None;
    let fstore = fenced_store();
    let mut follower = Some(follower_over(&m, fstore.clone()));
    let mut written = 0u64;
    let mut last: Option<PullBatch> = None;
    // fenced, conflicts (gap or repeat), bad digests, restarts
    let mut seen = [0usize; 4];
    for round in 0..300 {
        // the current leader writes
        let current = l2.as_ref().map_or(s1.clone(), |l| l.store().clone());
        // faster than the follower's window, so that it lags when the lease moves
        let k = rng.next() % 7;
        if k > 0 {
            write(current.as_ref(), &actor, written + 1, written + k);
            written += k;
        }
        // a handover: n2 takes the shard with a copy of the journal, n1 does not learn it
        if l2.is_none() && round > 60 && rng.chance(3) {
            let lease2 = table.handover(&lease1, &n2, Millis(1)).unwrap();
            h.record(HistoryEvent::Grant {
                node: n2.clone(),
                shard: SHARD,
                epoch: lease2.epoch,
            });
            let s2 = Arc::new(MemoryStore::new());
            ship_journal(s1.as_ref(), s2.as_ref()).unwrap();
            let l = Arc::new(ShipLeader::new(n2.clone(), SHARD, s2, 1 << 20));
            l.set_fence(Some(lease2.epoch));
            l2 = Some(l);
        }
        if rng.chance(4) {
            // the follower restarts over its durable store
            drop(follower.take());
            follower = Some(follower_over(&m, fstore.clone()));
            seen[3] += 1;
        }
        let fl = follower.as_ref().unwrap();
        // the follower pulls from the leader it knows, sometimes from the stale one
        let source = match &l2 {
            Some(l) if !rng.chance(25) => l.clone(),
            _ => l1.clone(),
        };
        let req = fl.request(&actor).unwrap();
        let answer = LocalShipLink::new(f.clone(), source).pull(&req);
        let Ok(clean) = answer else { continue };
        let mut batch = clean.clone();
        match rng.next() % 10 {
            0 => continue, // the connection drops before the answer arrives
            1 => batch.entries.truncate(batch.entries.len() / 2),
            2 if batch.entries.len() > 1 => {
                let i = (rng.next() as usize) % (batch.entries.len() - 1);
                batch.entries.remove(i);
            }
            3 if !batch.entries.is_empty() => {
                let i = (rng.next() as usize) % batch.entries.len();
                batch.entries[i].entry = tamper(&batch.entries[i].entry);
            }
            4 => {
                if let Some(old) = last.clone() {
                    batch = old; // the network replays an old answer
                }
            }
            _ => {}
        }
        last = Some(clean);
        let accepted = fl.accept(&batch).unwrap();
        match accepted.refusal().map(|r| r.verdict) {
            Some(Verdict::Fenced { .. }) => seen[0] += 1,
            Some(Verdict::OutOfOrder { .. } | Verdict::Duplicate) => seen[1] += 1,
            Some(Verdict::BadDigest) => seen[2] += 1,
            _ => {}
        }
        accepted.record(&batch, &mut h);
    }
    // the network heals: the follower catches up with the current leader
    let fl = follower.as_ref().unwrap();
    let current = l2.unwrap_or(l1);
    let link = LocalShipLink::new(f.clone(), current.clone());
    fl.catch_up(&link, &actor, 10_000, &mut |b, a| a.record(b, &mut h))
        .unwrap();
    same_journal(current.store().as_ref(), fstore.as_ref(), &actor);
    (h, seen)
}

// @tg: verifies:: [[openspec:journal-shipping#Ordered Shipping#A gap is refused]] {level: integration}
// @tg: verifies:: [[openspec:journal-shipping#Resumption#Connection drops mid-ship]] {level: integration}
#[test]
fn fault_histories_replay_through_the_lean_lease_model() {
    let lease = lease_oracle();
    let mut total = [0usize; 4];
    for seed in [7, 11, 1913, 2026, 31337, 65_521] {
        let (h, seen) = faulty_run(seed);
        let r = verify(&format!("shipping-{seed}"), &h, lease.as_deref());
        assert!(r.accepted > 0 && r.acked == r.accepted, "{r:?}");
        for (t, s) in total.iter_mut().zip(seen) {
            *t += s;
        }
    }
    // every fault actually happened somewhere in the runs
    assert!(total.iter().all(|&n| n > 0), "faults seen: {total:?}");
}

#[cfg(feature = "quic")]
mod quic {
    //! The same protocol over the QUIC cluster transport.

    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use vertep_cluster::shipping::quic::{QuicShipLink, ShipService};
    use vertep_cluster::transport::{
        ClusterCa, ClusterTransport, NodeIdentity, PeerSet, Receiving, TransportConfig,
    };
    use vertep_cluster::{ClusterId, EpochFilter, MemoryDeadLetters, PullSource};

    fn bind(id: NodeIdentity, peers: &[&str], service: Arc<ShipService>) -> ClusterTransport {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        ClusterTransport::bind(
            addr,
            id,
            Arc::new(PeerSet::new(peers.iter().map(|p| node(p)))),
            Receiving {
                handler: service,
                epochs: Arc::new(EpochFilter::new()),
                dead_letters: Arc::new(MemoryDeadLetters::default()),
            },
            TransportConfig::default(),
        )
        .unwrap()
    }

    struct Net {
        ca: ClusterCa,
        leader_t: ClusterTransport,
        follower_t: Arc<ClusterTransport>,
    }

    fn net(leader: &Leader) -> Net {
        let ca = ClusterCa::generate(&ClusterId::new("test").unwrap()).unwrap();
        let service = Arc::new(ShipService::new(None));
        service.serve(leader.clone() as Arc<dyn PullSource>);
        let leader_t = bind(ca.issue(&node("n1")).unwrap(), &["f"], service);
        let follower_t = Arc::new(bind(
            ca.issue(&node("f")).unwrap(),
            &["n1"],
            Arc::new(ShipService::new(None)),
        ));
        Net {
            ca,
            leader_t,
            follower_t,
        }
    }

    async fn within<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(30), f)
            .await
            .expect("timed out")
    }

    // @tg: verifies:: [[openspec:journal-shipping#Ordered Shipping#A follower catches up]] {level: integration}
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_follower_catches_up_over_quic() {
        let (m, actor) = setup();
        let leader = leader_with(&actor, 25, 1);
        let follower = follower_over(&m, fenced_store());
        write(follower.store().as_ref(), &actor, 1, 10);
        let n = net(&leader);
        let link = QuicShipLink::new(
            n.follower_t.clone(),
            node("n1"),
            n.leader_t.local_addr().unwrap(),
        );
        let r = within(link.catch_up(&follower, &actor, 100, &mut |b, a| {
            assert!(a.refusal().is_none());
            assert!(b.wire_size() <= WINDOW as usize);
        }))
        .await
        .unwrap();
        assert_eq!((r.head, r.stored), (Position::new(25), 15));
        assert!(r.rounds > 2, "several windows: {}", r.rounds);
        same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
        assert_eq!(leader.acknowledged(&node("f"), &actor), Position::new(25));
    }

    // @tg: verifies:: [[openspec:journal-shipping#Resumption#Connection drops mid-ship]] {level: integration}
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_leader_goes_away_mid_ship_over_quic() {
        let (m, actor) = setup();
        let leader = leader_with(&actor, 25, 1);
        let follower = follower_over(&m, fenced_store());
        let n = net(&leader);
        let link = QuicShipLink::new(
            n.follower_t.clone(),
            node("n1"),
            n.leader_t.local_addr().unwrap(),
        );
        // one pull, then the leader's endpoint closes
        let req = follower.request(&actor).unwrap();
        let b = within(link.pull(&req)).await.unwrap();
        let mut h = History::new();
        h.record(HistoryEvent::Grant {
            node: node("n1"),
            shard: SHARD,
            epoch: 1,
        });
        follower.accept(&b).unwrap().record(&b, &mut h);
        let mid = follower.position(&actor).unwrap();
        assert!(mid > Position::ZERO && mid < Position::new(25));
        n.leader_t.close();
        drop(n.leader_t);
        let err = within(link.pull(&follower.request(&actor).unwrap())).await;
        assert!(matches!(err, Err(PullError::Unreachable(_))), "{err:?}");
        // the leader comes back on a new endpoint; the follower resumes from its position
        let service = Arc::new(ShipService::new(None));
        service.serve(leader.clone() as Arc<dyn PullSource>);
        let back = bind(n.ca.issue(&node("n1")).unwrap(), &["f"], service);
        link.set_addr(back.local_addr().unwrap());
        let mut first = None;
        within(link.catch_up(&follower, &actor, 100, &mut |b, a| {
            first.get_or_insert(b.after);
            a.record(b, &mut h);
        }))
        .await
        .unwrap();
        assert_eq!(first, Some(mid));
        same_journal(leader.store().as_ref(), follower.store().as_ref(), &actor);
        // every entry applied once across the drop
        let r = h.check().unwrap();
        assert_eq!((r.accepted, r.conflicts, r.acked), (25, 0, 25));
    }

    // @tg: verifies:: [[openspec:journal-shipping#Back Pressure#A slow follower]] {level: integration}
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_follower_over_quic() {
        let (m, actor) = setup();
        let leader = leader_with(&actor, 0, 1);
        let follower = follower_over(&m, fenced_store());
        let n = net(&leader);
        let link = QuicShipLink::new(
            n.follower_t.clone(),
            node("n1"),
            n.leader_t.local_addr().unwrap(),
        );
        let mut written = 0;
        for _ in 0..30 {
            write(leader.store().as_ref(), &actor, written + 1, written + 20);
            written += 20;
            let b = within(link.pull(&follower.request(&actor).unwrap()))
                .await
                .unwrap();
            follower.accept(&b).unwrap();
            // the follower is slow to come back for more
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(written - follower.position(&actor).unwrap().get() > 300);
        let peak = leader.peak_buffered(&node("f"));
        assert!(peak > 0 && peak <= WINDOW as usize, "peak {peak}");
    }
}

#[test]
fn a_follower_that_takes_a_refused_entry_is_caught() {
    // the same history with one refusal turned into an acceptance, as a follower that skipped
    // its position check would have recorded it: both the checker and the Lean model object
    let (mut h, _) = faulty_run(2026);
    let at = h
        .events
        .iter()
        .position(|e| {
            matches!(
                e,
                HistoryEvent::Append {
                    decision: vertep_cluster::AppendDecision::Conflict,
                    ..
                }
            )
        })
        .unwrap();
    if let HistoryEvent::Append { decision, .. } = &mut h.events[at] {
        *decision = vertep_cluster::AppendDecision::Accept;
    }
    assert!(h.check().is_err());
    if let Some(lease) = lease_oracle() {
        let verdict = common::oracle::lean_lease_verdict(&lease, &h);
        assert!(verdict.is_err(), "the Lean model accepted {verdict:?}");
    }
}

#[test]
fn bounded_reads_agree_with_full_reads() {
    // the leader reads in bounded ranges; the backends that override `read_range` must return
    // exactly the prefix `read` would
    let (_, actor) = setup();
    let dir = tempfile::tempdir().unwrap();
    let file = TiramemsuShard::open(dir.path().join("s.db")).unwrap();
    let memory = MemoryStore::new();
    for store in [&file as &dyn JournalStore, &memory] {
        write(store, &actor, 1, 9);
        for from in [0, 1, 4, 9, 10] {
            for limit in [0, 1, 3, 20] {
                let full = store.read(&actor, Position::new(from)).unwrap();
                let part = store
                    .read_range(&actor, Position::new(from), limit)
                    .unwrap();
                assert_eq!(
                    part.len(),
                    full.len().min(limit),
                    "from {from} limit {limit}"
                );
                for (a, b) in part.iter().zip(&full) {
                    assert_eq!(a.position, b.position);
                    assert!(a.entry.is_isomorphic(&b.entry));
                }
            }
        }
    }
}
