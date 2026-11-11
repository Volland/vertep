//! Shard placement: the shard count, the IRI hash, one journal per shard, activation on the
//! owner, single activation, fencing, forwarding, rebalancing and resharding.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{AuthorityHarness, RecordingDrain, actor_in, grant, meta, node, timing};
use vertep_cluster::testkit::Harness;
use vertep_cluster::{
    ActivationGate, AuthorityLink, AuthorityMembership, ClusterId, ClusterMeta, DedupCache,
    HashVersion, Inbound, JoinError, JoinRequest, ManualClock, Membership, MemoryAuthority,
    MetaError, ReshardError, ShardCount, ShardId, ShardJournal, ShardStores, ShipError, shard_of,
};
use vertep_store::oxrdf::NamedNode;
use vertep_store::testkit::sample_entry;
use vertep_store::{
    AppendError, AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, MemoryStore,
    Position, StoreError,
};
use vertep_tiramemsu::TiramemsuShard;

// @tg: verifies:: [[openspec:shard-placement#Fixed Shard Count#Shard count recorded at creation]] {level: integration}
#[test]
fn shard_count_recorded_at_creation() {
    let dir = tempfile::tempdir().unwrap();
    let created = ClusterMeta::new(ClusterId::new("prod").unwrap(), 256).unwrap();
    created.create(dir.path()).unwrap();
    let recorded = ClusterMeta::load(dir.path()).unwrap();
    assert_eq!(recorded.shard_count.get(), 256);
    let clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        recorded.clone(),
        timing(),
        Arc::new(clock.clone()),
    ));
    for i in 0..3 {
        let n = AuthorityMembership::new(
            authority.clone(),
            ClusterMeta::load(dir.path()).unwrap(),
            node(&format!("n{i}")),
            "x",
            Arc::new(clock.clone()),
            timing().lease,
        );
        n.join().unwrap();
        assert_eq!(n.meta().shard_count.get(), 256);
    }
    // creating again with another count is refused
    let other = ClusterMeta::new(ClusterId::new("prod").unwrap(), 512).unwrap();
    assert!(matches!(
        other.create(dir.path()),
        Err(MetaError::AlreadyExists { .. })
    ));
}

// @tg: verifies:: [[openspec:shard-placement#Fixed Shard Count#Non power of two is rejected]] {level: unit}
#[test]
fn non_power_of_two_is_rejected() {
    let err = ClusterMeta::new(ClusterId::new("prod").unwrap(), 300).unwrap_err();
    assert_eq!(err, MetaError::NotPowerOfTwo(300));
    assert!(err.to_string().contains("must be a power of two"));
    assert!(ShardCount::new(0).is_err());
    assert!(ShardCount::new(1024).is_ok());
}

// @tg: verifies:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash#Same IRI, same shard everywhere]] {level: unit}
#[test]
fn same_iri_same_shard_everywhere() {
    // golden vectors: FNV-1a 64 of the UTF-8 bytes, independent of OS, pointer width and
    // endianness; checked on every CI platform
    let golden: &[(&str, u64, u32, u32)] = &[
        ("", 0xcbf2_9ce4_8422_2325, 37, 805),
        ("urn:vt:actor:alice", 0x4059_01a2_15b4_a8bd, 189, 189),
        ("urn:vt:actor:bob", 0x2306_eebb_8911_278a, 138, 906),
        ("https://example.org/actors/42", 0x82a2_0329_0b8b_7406, 6, 6),
        ("did:web:example.com#agent", 0x4f2e_d8bf_6aa0_f9ef, 239, 495),
        ("urn:vt:actor:ünïcödé", 0x33e2_7cf6_2596_a494, 148, 148),
    ];
    let n256 = ShardCount::new(256).unwrap();
    let n1024 = ShardCount::new(1024).unwrap();
    for (iri, hash, s256, s1024) in golden {
        assert_eq!(HashVersion::Fnv1a64.hash64(iri.as_bytes()), *hash, "{iri}");
        assert_eq!(shard_of(iri, n256, HashVersion::Fnv1a64).get(), *s256);
        assert_eq!(shard_of(iri, n1024, HashVersion::Fnv1a64).get(), *s1024);
    }
    // the cluster placement is the single-node tiramemsu placement
    for i in 0..5_000 {
        let iri = format!("urn:vt:actor:{i}");
        for n in [1u64, 2, 64, 256, 1024] {
            let c = ShardCount::new(n).unwrap();
            assert_eq!(
                shard_of(&iri, c, HashVersion::CURRENT).index(),
                vertep_tiramemsu::shard_of(&iri, n as usize)
            );
        }
    }
}

// @tg: verifies:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash#Hash function version is recorded]] {level: integration}
#[test]
fn hash_function_version_is_recorded() {
    let m = meta(64);
    assert!(m.to_text().contains("hash_version = 1"));
    assert_eq!(
        ClusterMeta::from_text(&m.to_text()).unwrap().hash_version,
        HashVersion::Fnv1a64
    );
    let authority = MemoryAuthority::new(m.clone(), timing(), Arc::new(ManualClock::new()));
    let mut req = JoinRequest::new(&m, node("future"), "x");
    req.hash_version = 2;
    let err = vertep_cluster::Authority::join(&authority, &req).unwrap_err();
    assert_eq!(
        err,
        JoinError::HashVersionMismatch {
            cluster: 1,
            node: 2
        }
    );
}

fn open_shards(dir: &std::path::Path, m: &ClusterMeta) -> ShardStores<TiramemsuShard> {
    let mut stores = ShardStores::new(m.shard_count, m.hash_version);
    for s in m.shard_count.shards() {
        let file = dir.join(vertep_tiramemsu::shard_file_name(
            s.index(),
            m.shard_count.get() as usize,
        ));
        stores.insert(s, TiramemsuShard::open(file).unwrap());
    }
    stores
}

// @tg: verifies:: [[openspec:shard-placement#One Journal File Per Shard#Actors of one shard share a store]] {level: integration}
#[test]
fn actors_of_one_shard_share_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let m = meta(16);
    let stores = open_shards(dir.path(), &m);
    let seven = ShardId::new(7);
    let a = actor_in(&m, seven, "a");
    let b = actor_in(&m, seven, "b");
    assert_ne!(a, b);
    for actor in [&a, &b] {
        let (s, store) = stores.for_actor(actor.as_str()).unwrap();
        assert_eq!(s, seven);
        store
            .append(
                actor,
                Position::ZERO,
                &[sample_entry(actor, "1")],
                &AppendMeta::none(),
            )
            .unwrap();
    }
    for (s, store) in stores.opened() {
        let actors = store.actors().unwrap();
        if s == seven {
            assert_eq!(actors.len(), 2);
        } else {
            assert!(actors.is_empty(), "{s} got an entry");
        }
    }
}

/// A store whose appends wait until the test opens its gate.
struct Gated<J> {
    inner: J,
    entered: mpsc::Sender<()>,
    gate: Mutex<mpsc::Receiver<()>>,
}

impl<J: JournalStore> JournalStore for Gated<J> {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.entered.send(()).unwrap();
        self.gate.lock().unwrap().recv().unwrap();
        self.inner.append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.inner.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.inner.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.inner.actors()
    }
}

// @tg: verifies:: [[openspec:shard-placement#One Journal File Per Shard#Shards write independently]] {level: integration}
#[test]
fn shards_write_independently() {
    let dir = tempfile::tempdir().unwrap();
    let m = meta(16);
    let (entered_tx, entered) = mpsc::channel();
    let (open, gate) = mpsc::channel();
    let mut stores: ShardStores<Box<dyn JournalStore>> =
        ShardStores::new(m.shard_count, m.hash_version);
    stores.insert(
        ShardId::new(7),
        Box::new(Gated {
            inner: TiramemsuShard::open(dir.path().join("s7.db")).unwrap(),
            entered: entered_tx,
            gate: Mutex::new(gate),
        }),
    );
    stores.insert(
        ShardId::new(8),
        Box::new(TiramemsuShard::open(dir.path().join("s8.db")).unwrap()),
    );
    let a7 = actor_in(&m, ShardId::new(7), "w");
    let a8 = actor_in(&m, ShardId::new(8), "w");
    std::thread::scope(|scope| {
        let stores = &stores;
        let t7 = scope.spawn(|| {
            let (_, s) = stores.for_actor(a7.as_str()).unwrap();
            s.append(
                &a7,
                Position::ZERO,
                &[sample_entry(&a7, "1")],
                &AppendMeta::none(),
            )
        });
        // shard 7's commit is in progress and held open
        entered.recv().unwrap();
        let (_, s8) = stores.for_actor(a8.as_str()).unwrap();
        assert_eq!(
            s8.append(
                &a8,
                Position::ZERO,
                &[sample_entry(&a8, "1")],
                &AppendMeta::none()
            )
            .unwrap(),
            Position::new(1),
            "shard 8 commits while shard 7's writer is busy"
        );
        assert!(!t7.is_finished());
        open.send(()).unwrap();
        assert_eq!(t7.join().unwrap().unwrap(), Position::new(1));
    });
}

// @tg: verifies:: [[openspec:shard-placement#Activation On Shard Owner Only#Owner activates on first message]] {level: integration}
#[test]
fn owner_activates_on_first_message() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let m = meta(64);
    let s = ShardId::new(12);
    let actor = actor_in(&m, s, "first");
    let store = Arc::new(MemoryStore::new());
    // the actor was active before: two committed macrosteps
    for (i, tag) in ["a", "b"].iter().enumerate() {
        store
            .append(
                &actor,
                Position::new(i as u64),
                &[sample_entry(&actor, tag)],
                &AppendMeta::none(),
            )
            .unwrap();
    }
    grant(&h.nodes[0], s);
    let gate = ActivationGate::new(h.nodes[0].clone(), 3);
    let journal = ShardJournal::new(store.clone(), s, h.nodes[0].clone()).unwrap();
    let Inbound::Deliver(lease) = gate.inbound(actor.as_str(), 0) else {
        panic!("the owner activates");
    };
    // activation replays the journal, then the message's macrostep commits under the lease
    let replay = journal.read(&actor, Position::ZERO).unwrap();
    assert_eq!(replay.len(), 2);
    let head = journal
        .append(
            &actor,
            Position::new(2),
            &[sample_entry(&actor, "c")],
            &AppendMeta::none(),
        )
        .unwrap();
    assert_eq!(head, Position::new(3));
    assert_eq!(journal.fence().unwrap().epoch, lease.epoch);
}

// @tg: verifies:: [[openspec:shard-placement#Activation On Shard Owner Only#Non-owner refuses activation]] {level: integration}
#[test]
fn non_owner_refuses_activation() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let m = meta(64);
    let s = ShardId::new(12);
    let actor = actor_in(&m, s, "nonowner");
    let lease = grant(&h.nodes[0], s);
    h.tick_all();
    let gate = ActivationGate::new(h.nodes[1].clone(), 3);
    let refusal = gate.may_activate(actor.as_str()).unwrap_err();
    assert_eq!(refusal.owner.unwrap().node, node("n0"));
    match gate.inbound(actor.as_str(), 0) {
        Inbound::Forward { to, redirect, .. } => {
            assert_eq!(to, node("n0"));
            assert_eq!(redirect.epoch, lease.epoch);
        }
        other => panic!("{other:?}"),
    }
    // and its journal refuses appends for the shard
    let journal = ShardJournal::new(MemoryStore::new(), s, h.nodes[1].clone()).unwrap();
    assert!(matches!(
        journal.append(
            &actor,
            Position::ZERO,
            &[sample_entry(&actor, "x")],
            &AppendMeta::none()
        ),
        Err(AppendError::Storage(StoreError::Unavailable(_)))
    ));
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Concurrent activation attempts]] {level: integration}
#[test]
fn concurrent_activation_attempts() {
    for round in 0..20 {
        let h = AuthorityHarness::external_cluster(2, 64);
        let m = meta(64);
        let actor = actor_in(&m, ShardId::new(round % 64), "race");
        let shard = m.shard_of(actor.as_str());
        // both nodes got a message for the actor and try to activate it at once
        let outcomes: Vec<Inbound> = std::thread::scope(|scope| {
            let hs: Vec<_> = h
                .nodes
                .iter()
                .map(|n| {
                    let n = n.clone();
                    let actor = actor.clone();
                    scope.spawn(move || {
                        let _ = n.acquire(shard);
                        let _ = n.poll();
                        ActivationGate::new(n, 3).inbound(actor.as_str(), 0)
                    })
                })
                .collect();
            hs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        let delivering = outcomes
            .iter()
            .filter(|o| matches!(o, Inbound::Deliver(_)))
            .count();
        assert_eq!(delivering, 1, "round {round}: {outcomes:?}");
        let winner = h.nodes.iter().find(|n| n.held(shard).is_some()).unwrap();
        for o in &outcomes {
            if let Inbound::Forward { to, .. } = o {
                assert_eq!(to, winner.local());
            }
        }
    }
}

fn split_brain(store: Arc<dyn JournalStore>) {
    let m = meta(64);
    let nine = ShardId::new(9);
    let actor = actor_in(&m, nine, "split");
    let authority_clock = ManualClock::new();
    let a_clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        m.clone(),
        timing(),
        Arc::new(authority_clock.clone()),
    ));
    let a_link = AuthorityLink::new(authority.clone());
    let a = Arc::new(AuthorityMembership::new(
        a_link.clone(),
        m.clone(),
        node("a"),
        "a",
        Arc::new(a_clock.clone()),
        timing().lease,
    ));
    let b = Arc::new(AuthorityMembership::new(
        authority.clone(),
        m.clone(),
        node("b"),
        "b",
        Arc::new(authority_clock.clone()),
        timing().lease,
    ));
    a.join().unwrap();
    b.join().unwrap();
    let a_lease = a.acquire(nine).unwrap();
    let a_journal = ShardJournal::new(store.clone(), nine, a.clone()).unwrap();
    let b_journal = ShardJournal::new(store.clone(), nine, b.clone()).unwrap();
    a_journal
        .append(
            &actor,
            Position::ZERO,
            &[sample_entry(&actor, "v1")],
            &AppendMeta::none(),
        )
        .unwrap();
    // partition, and A pauses (its clock stands still)
    a_link.set_connected(false);
    for _ in 0..12 {
        authority_clock.advance(Duration::from_secs(1));
        b.tick().unwrap();
    }
    let b_lease = b.acquire(nine).unwrap();
    assert!(b_lease.epoch > a_lease.epoch);
    b_journal
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "v2")],
            &AppendMeta::none(),
        )
        .unwrap();
    // A wakes up believing in its lease and appends at the current head: the version check
    // passes, the epoch check does not
    assert!(a.held(nine).is_some());
    let err = a_journal
        .append(
            &actor,
            Position::new(2),
            &[sample_entry(&actor, "v3-stale")],
            &AppendMeta::none(),
        )
        .unwrap_err();
    assert!(err.is_fenced(), "{err:?}");
    assert!(a.held(nine).is_none(), "A deactivates the shard");
    let journal: Vec<_> = store
        .read(&actor, Position::ZERO)
        .unwrap()
        .into_iter()
        .map(|r| {
            r.entry
                .name()
                .as_str()
                .rsplit('/')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(journal, ["v1", "v2"], "B's entries are kept");
}

// @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Stale owner append fails after split brain]] {level: integration}
#[test]
fn stale_owner_append_fails_after_split_brain() {
    split_brain(Arc::new(MemoryStore::new()));
    let dir = tempfile::tempdir().unwrap();
    split_brain(Arc::new(
        TiramemsuShard::open(dir.path().join("shard-9.db")).unwrap(),
    ));
}

#[test]
fn a_store_without_fencing_cannot_back_a_shard() {
    let h = AuthorityHarness::external_cluster(1, 64);
    assert!(
        ShardJournal::new(
            MemoryStore::new().with_fencing(false),
            ShardId::new(1),
            h.nodes[0].clone()
        )
        .is_err()
    );
}

// @tg: verifies:: [[openspec:shard-placement#Message Forwarding To Shard Owner#Message sent to the wrong node]] {level: integration}
#[test]
fn message_sent_to_the_wrong_node() {
    let h = AuthorityHarness::external_cluster(3, 64);
    let m = meta(64);
    let actor = actor_in(&m, ShardId::new(3), "wrong");
    grant(&h.nodes[2], ShardId::new(3));
    h.tick_all();
    let gates: Vec<_> = h
        .nodes
        .iter()
        .map(|n| ActivationGate::new(n.clone(), 3))
        .collect();
    let mut processed = 0;
    let mut dedup = DedupCache::new(8);
    let (mut at, mut hops) = (0usize, 0u8);
    loop {
        match gates[at].inbound(actor.as_str(), hops) {
            Inbound::Forward { to, hops: h2, .. } => {
                at = h.nodes.iter().position(|n| n.local() == &to).unwrap();
                hops = h2;
            }
            Inbound::Deliver(_) => {
                if dedup.first_time(actor.as_str(), "m") {
                    processed += 1;
                }
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!((at, hops, processed), (2, 1, 1));
}

// @tg: verifies:: [[openspec:shard-placement#Message Forwarding To Shard Owner#Hop limit reached]] {level: integration}
#[test]
fn hop_limit_reached() {
    let h = AuthorityHarness::external_cluster(3, 64);
    let m = meta(64);
    let actor = actor_in(&m, ShardId::new(3), "hops");
    grant(&h.nodes[2], ShardId::new(3));
    h.tick_all();
    let gate = ActivationGate::new(h.nodes[0].clone(), 2);
    assert!(matches!(
        gate.inbound(actor.as_str(), 1),
        Inbound::Forward { hops: 2, .. }
    ));
    assert_eq!(
        gate.inbound(actor.as_str(), 2),
        Inbound::HopLimit {
            shard: ShardId::new(3),
            hops: 2
        }
    );
}

fn fill(
    store: &dyn JournalStore,
    m: &ClusterMeta,
    shard: ShardId,
    actors: usize,
    entries: u64,
) -> u64 {
    let mut n = 0;
    for i in 0..actors {
        let a = actor_in(m, shard, &format!("ship{i}"));
        for e in 0..entries {
            store
                .append(
                    &a,
                    Position::new(e),
                    &[sample_entry(&a, &e.to_string())],
                    &AppendMeta::none(),
                )
                .unwrap();
            n += 1;
        }
    }
    n
}

fn same_journals(a: &dyn JournalStore, b: &dyn JournalStore) {
    let actors = a.actors().unwrap();
    assert_eq!(actors, b.actors().unwrap());
    for actor in actors {
        let (x, y) = (
            a.read(&actor, Position::ZERO).unwrap(),
            b.read(&actor, Position::ZERO).unwrap(),
        );
        assert_eq!(x.len(), y.len());
        for (r, s) in x.iter().zip(&y) {
            assert_eq!(r.position, s.position);
            assert!(r.entry.is_isomorphic(&s.entry));
        }
    }
}

// @tg: verifies:: [[openspec:shard-placement#Rebalancing By Journal File Shipping#Shard moved to a new node]] {level: integration}
#[test]
fn shard_moved_to_a_new_node() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = AuthorityHarness::external_cluster(1, 64);
    let m = meta(64);
    let five = ShardId::new(5);
    let source = Arc::new(TiramemsuShard::open(dir.path().join("a-shard-5.db")).unwrap());
    let target = Arc::new(TiramemsuShard::open(dir.path().join("b-shard-5.db")).unwrap());
    let old = grant(&h.nodes[0], five);
    let old_journal = ShardJournal::new(source.clone(), five, h.nodes[0].clone()).unwrap();
    let total = fill(&old_journal, &m, five, 3, 4);
    // a new node joins and the rebalancer moves shard 5 to it
    let b = h.add_node("n1");
    b.join().unwrap();
    let mut drain = RecordingDrain::default();
    let (next, report) = vertep_cluster::move_by_shipping(
        h.nodes[0].as_ref(),
        &old,
        &node("n1"),
        &mut drain,
        source.as_ref(),
        target.as_ref(),
    )
    .unwrap();
    assert_eq!(report.entries, total);
    same_journals(source.as_ref(), target.as_ref());
    assert!(next.epoch > old.epoch);
    h.tick_all();
    // every actor resumes from its last committed state on the new node
    let new_journal = ShardJournal::new(target.clone(), five, b.clone()).unwrap();
    for actor in target.actors().unwrap() {
        let head = new_journal.head(&actor).unwrap();
        assert_eq!(head, Position::new(4));
        new_journal
            .append(
                &actor,
                head,
                &[sample_entry(&actor, "after-move")],
                &AppendMeta::none(),
            )
            .unwrap();
    }
    assert!(h.nodes[0].held(five).is_none());
}

/// A store that fails every append after the first `budget`.
struct Flaky<J> {
    inner: J,
    budget: AtomicUsize,
}

impl<J: JournalStore> JournalStore for Flaky<J> {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        if self
            .budget
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| b.checked_sub(1))
            .is_err()
        {
            return Err(AppendError::Storage(StoreError::Unavailable(
                "connection broke".into(),
            )));
        }
        self.inner.append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.inner.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.inner.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.inner.actors()
    }
}

// @tg: verifies:: [[openspec:shard-placement#Rebalancing By Journal File Shipping#Shipping interrupted]] {level: integration}
#[test]
fn shipping_interrupted() {
    let mut h = AuthorityHarness::external_cluster(2, 64);
    let m = meta(64);
    let five = ShardId::new(5);
    let source = Arc::new(MemoryStore::new());
    let target = Arc::new(MemoryStore::new());
    let old = grant(&h.nodes[0], five);
    let total = fill(source.as_ref(), &m, five, 3, 4);
    let flaky = Flaky {
        inner: target.clone(),
        budget: AtomicUsize::new(5),
    };
    let err = vertep_cluster::move_by_shipping(
        h.nodes[0].as_ref(),
        &old,
        &node("n1"),
        &mut RecordingDrain::default(),
        source.as_ref(),
        &flaky,
    )
    .unwrap_err();
    assert!(matches!(err, ShipError::Append { .. }));
    // the old owner keeps the lease and resumes processing
    h.settle();
    assert_eq!(h.nodes[0].held(five).map(|l| l.epoch), Some(old.epoch));
    let journal = ShardJournal::new(source.clone(), five, h.nodes[0].clone()).unwrap();
    let a0 = actor_in(&m, five, "ship0");
    journal
        .append(
            &a0,
            Position::new(4),
            &[sample_entry(&a0, "resumed")],
            &AppendMeta::none(),
        )
        .unwrap();
    // shipping restarts from the last verified position: only the rest is copied
    let lease = h.nodes[0].held(five).unwrap();
    let (next, report) = vertep_cluster::move_by_shipping(
        h.nodes[0].as_ref(),
        &lease,
        &node("n1"),
        &mut RecordingDrain::default(),
        source.as_ref(),
        target.as_ref(),
    )
    .unwrap();
    assert_eq!(report.entries, total + 1 - 5);
    same_journals(source.as_ref(), target.as_ref());
    assert!(next.epoch > old.epoch);
}

/// A store that counts reads and appends.
struct Counting<J> {
    inner: J,
    ops: AtomicUsize,
}

impl<J: JournalStore> JournalStore for Counting<J> {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        self.inner.append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        self.inner.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.inner.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.inner.actors()
    }
}

// @tg: verifies:: [[openspec:shard-placement#Rebalancing By Lease Transfer#Shard moved with a shared journal]] {level: integration}
#[test]
fn shard_moved_with_a_shared_journal() {
    let h = AuthorityHarness::external_cluster(2, 64);
    let m = meta(64);
    let five = ShardId::new(5);
    let actor = actor_in(&m, five, "shared");
    let shared = Arc::new(Counting {
        inner: MemoryStore::new(),
        ops: AtomicUsize::new(0),
    });
    let old = grant(&h.nodes[0], five);
    let a = ShardJournal::new(shared.clone(), five, h.nodes[0].clone()).unwrap();
    a.append(
        &actor,
        Position::ZERO,
        &[sample_entry(&actor, "1")],
        &AppendMeta::none(),
    )
    .unwrap();
    let before = shared.ops.load(Ordering::SeqCst);
    let next = vertep_cluster::move_by_lease_transfer(
        h.nodes[0].as_ref(),
        &old,
        &node("n1"),
        &mut RecordingDrain::default(),
    )
    .unwrap();
    assert_eq!(
        shared.ops.load(Ordering::SeqCst),
        before,
        "no journal data copied"
    );
    h.tick_all();
    let b = ShardJournal::new(shared.clone(), five, h.nodes[1].clone()).unwrap();
    assert_eq!(b.fence().unwrap().epoch, next.epoch);
    b.append(
        &actor,
        Position::new(1),
        &[sample_entry(&actor, "2")],
        &AppendMeta::none(),
    )
    .unwrap();
    assert_eq!(shared.head(&actor).unwrap(), Position::new(2));
}

fn stores(n: usize) -> Vec<MemoryStore> {
    (0..n).map(|_| MemoryStore::new()).collect()
}

// @tg: verifies:: [[openspec:shard-placement#Resharding Procedure#Doubling the shard count]] {level: integration}
#[test]
fn doubling_the_shard_count() {
    let m = meta(64);
    let old = stores(64);
    let mut written = Vec::new();
    for i in 0..300 {
        let a = NamedNode::new(format!("urn:vt:actor:reshard-{i}")).unwrap();
        let s = m.shard_of(a.as_str());
        for e in 0..(i % 3 + 1) as u64 {
            old[s.index()]
                .append(
                    &a,
                    Position::new(e),
                    &[sample_entry(&a, &e.to_string())],
                    &AppendMeta::none(),
                )
                .unwrap();
        }
        written.push(a);
    }
    let new = stores(128);
    let old_refs: Vec<&dyn JournalStore> = old.iter().map(|s| s as &dyn JournalStore).collect();
    let new_refs: Vec<&dyn JournalStore> = new.iter().map(|s| s as &dyn JournalStore).collect();
    let (next, report) = vertep_cluster::reshard_offline(&m, &[], &old_refs, &new_refs).unwrap();
    assert_eq!(next.shard_count.get(), 128);
    assert_eq!(report.actors.len(), 300);
    let dir = tempfile::tempdir().unwrap();
    next.create(dir.path()).unwrap();
    assert_eq!(
        ClusterMeta::load(dir.path()).unwrap().shard_count.get(),
        128
    );
    for a in written {
        let s_new = next.shard_of(a.as_str());
        let s_old = m.shard_of(a.as_str());
        let (lo, hi) = s_old.split(m.shard_count);
        assert!(s_new == lo || s_new == hi);
        let before = old[s_old.index()].read(&a, Position::ZERO).unwrap();
        let after = new[s_new.index()].read(&a, Position::ZERO).unwrap();
        assert_eq!(before.len(), after.len());
        assert!(
            before
                .iter()
                .zip(&after)
                .all(|(x, y)| x.position == y.position && x.entry.is_isomorphic(&y.entry))
        );
        for (i, other) in new.iter().enumerate() {
            if i != s_new.index() {
                assert_eq!(other.head(&a).unwrap(), Position::ZERO);
            }
        }
    }
}

// @tg: verifies:: [[openspec:shard-placement#Resharding Procedure#Resharding refuses a running cluster]] {level: integration}
#[test]
fn resharding_refuses_a_running_cluster() {
    let h = AuthorityHarness::external_cluster(3, 4);
    grant(&h.nodes[2], ShardId::new(0));
    grant(&h.nodes[1], ShardId::new(1));
    let old = stores(4);
    let new = stores(8);
    let old_refs: Vec<&dyn JournalStore> = old.iter().map(|s| s as &dyn JournalStore).collect();
    let new_refs: Vec<&dyn JournalStore> = new.iter().map(|s| s as &dyn JournalStore).collect();
    let err =
        vertep_cluster::reshard_offline(&meta(4), &h.authority.live_leases(), &old_refs, &new_refs)
            .unwrap_err();
    assert_eq!(err, ReshardError::LeasesHeld(vec![node("n1"), node("n2")]));
    let msg = err.to_string();
    assert!(msg.contains("n1") && msg.contains("n2"), "{msg}");
}
