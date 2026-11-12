//! Decentralised custody: every scenario of `object-custody` and the custody-level scenarios
//! of `durable-objects`, on custodians in one process with switchable links.

use std::sync::{Arc, Mutex};
use std::thread;

use oxrdf::Term;
use vertep_federation::{DidDocument, KeyManager};
use vertep_objects::chain::EntrySigner;
use vertep_objects::custody::{
    AppendBatch, ConfigError, ContentKey, Custodian, CustodyConfig, CustodyError, CustodyHost,
    CustodyRequest, CustodyResponse, CustodyTransport, Evidence, EvidenceSink, LinkError,
    LocalTransport, ReconfigurationRequest, RecordingPublisher, WireEntry,
};
use vertep_objects::testkit::{CustodyFixture, TestHost, turn};
use vertep_objects::{ChainLink, content_hash, seal};
use vertep_store::{AppendError, GraphStore, MemoryStore, Position, StoreError};

const A: &str = "did:web:a.example.org";
const B: &str = "did:web:b.example.org";
const C: &str = "did:web:c.example.org";
const K1: &str = "did:web:k1.example.net";
const K2: &str = "did:web:k2.example.net";
const K3: &str = "did:web:k3.example.net";

/// Three hosts and two keepers, quorum three.
fn five() -> CustodyFixture {
    CustodyFixture::new(&[A, B, C], &[K1, K2], None)
}

fn write(h: &CustodyHost, f: &CustodyFixture, from: u64, n: u64, note: &str) {
    let obj = f.object.iri();
    for i in from..from + n {
        h.append_turn(
            Position::new(i - 1),
            &[turn(&obj, i, &format!("{note} {i}"))],
        )
        .unwrap_or_else(|e| panic!("append {i}: {e}"));
    }
}

fn notes(h: &CustodyHost) -> Vec<String> {
    h.read_committed(Position::ZERO)
        .unwrap()
        .iter()
        .flat_map(|r| {
            r.entry
                .triples()
                .iter()
                .filter(|t| t.predicate.as_str().ends_with("#note"))
                .map(|t| match t.object {
                    oxrdf::TermRef::Literal(l) => l.value().to_owned(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- Custodian Set In The DID Document ----

// @tg: verifies:: [[openspec:object-custody#Custodian Set In The DID Document#Custodian set resolved]] {level: unit}
#[test]
fn custodian_set_resolved() {
    let f = five();
    let mut doc = DidDocument::new(f.object.as_str());
    doc.service.push(f.config.to_service(f.object.as_str()));
    let back = DidDocument::from_json(&doc.to_json()).unwrap();
    let cfg = CustodyConfig::from_document(&back).unwrap();
    assert_eq!(cfg.custodians.len(), 5);
    assert_eq!(cfg.hosts().count(), 3);
    assert_eq!(cfg.quorum, 3);
    assert!(cfg.is_host(A) && !cfg.is_host(K1));
}

// @tg: verifies:: [[openspec:object-custody#Custodian Set In The DID Document#Quorum below a majority refused]] {level: unit}
#[test]
fn quorum_below_a_majority_refused() {
    let f = five();
    let weak = CustodyConfig {
        version: 1,
        custodians: f.config.custodians.clone(),
        quorum: 2,
    };
    let mut doc = DidDocument::new(f.object.as_str());
    doc.service.push(weak.to_service(f.object.as_str()));
    let err = CustodyConfig::from_document(&doc).unwrap_err();
    assert_eq!(
        err,
        ConfigError::QuorumBelowMajority {
            quorum: 2,
            custodians: 5
        }
    );
    assert!(err.to_string().contains("invalid custody declaration"));
}

// ---- Quorum-Promised Custody Epochs ----

// @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs#Epoch won]] {level: integration}
#[test]
fn epoch_won() {
    let f = five();
    f.set_up(K1, false);
    f.set_up(K2, false);
    let a = f.custody_host(A);
    let e = a.elect(7).unwrap();
    assert_eq!(e.epoch, 7);
    assert_eq!(a.writer_epoch(), Some(7));
    let rec = &a.epochs()[0];
    assert_eq!(rec.epoch, 7);
    assert_eq!(rec.promises.len(), 3);
    write(&a, &f, 1, 1, "turn");
    let promised_by = a
        .lineage()
        .iter()
        .filter(|t| t.predicate.as_str().ends_with("#promisedBy"))
        .count();
    assert_eq!(
        promised_by, 3,
        "the lineage records epoch 7 with three promises"
    );
}

// @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs#Epoch lost to a higher one]] {level: integration}
#[test]
fn epoch_lost_to_a_higher_one() {
    let f = CustodyFixture::new(&[A, B, C], &[], None);
    for d in [B, C] {
        let r = f.party(d).replica.handle(&CustodyRequest::Prepare {
            object: f.object.as_str().into(),
            epoch: 8,
            candidate: "did:web:other.example.org".into(),
        });
        assert!(matches!(r, CustodyResponse::Promise(_)));
    }
    let a = f.custody_host(A);
    match a.elect(7).unwrap_err() {
        CustodyError::NoQuorum { highest, .. } => assert_eq!(highest, 8),
        e => panic!("{e}"),
    }
    let refused = f.party(B).replica.handle(&CustodyRequest::Prepare {
        object: f.object.as_str().into(),
        epoch: 7,
        candidate: A.into(),
    });
    assert!(matches!(
        refused,
        CustodyResponse::Refused { highest: 8, .. }
    ));
    // Retrying above 8 wins.
    assert!(a.activate().unwrap() > 8);
}

// @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs#Clock skew does not matter]] {level: integration}
#[test]
fn clock_skew_does_not_matter() {
    // Custody decisions take no time input at all: no message carries a clock reading, and a
    // contention-free election never waits.
    let run = || {
        let f = five();
        let a = f.custody_host(A);
        a.elect(1).unwrap();
        write(&a, &f, 1, 3, "turn");
        let carried: Vec<String> = f
            .parties
            .iter()
            .flat_map(|p| p.transport.carried())
            .collect();
        (notes(&a), a.writer_epoch(), carried)
    };
    let (n1, e1, carried) = run();
    let (n2, e2, _) = run();
    assert_eq!((n1, e1), (n2, e2));
    for m in carried {
        for clocky in ["\"ts\"", "time", "lease", "expires", "deadline"] {
            assert!(
                !m.contains(clocky),
                "a custody message carries {clocky}: {m}"
            );
        }
    }
}

// ---- Journal Replication Over TSP (transport-neutral part) ----

// @tg: verifies:: [[openspec:object-custody#Journal Replication Over TSP#Lagging custodian catches up]] {level: integration}
#[test]
fn lagging_custodian_catches_up() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    f.set_up(C, false);
    write(&a, &f, 1, 40, "turn");
    assert_eq!(f.party(C).replica.head(), Position::ZERO);
    f.set_up(C, true);
    write(&a, &f, 41, 1, "turn");
    assert_eq!(f.party(C).replica.head(), Position::new(41));
    assert_eq!(f.party(K2).replica.head(), Position::new(41));
}

// ---- Quorum Before Effects ----

// @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Committed entry survives failover]] {level: integration}
#[test]
fn committed_entry_survives_failover() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    f.set_up(B, false);
    f.set_up(K2, false);
    write(&a, &f, 1, 1, "acked by three");
    f.set_up(A, false);
    f.set_up(B, true);
    f.set_up(K2, true);
    let b = f.custody_host(B);
    b.activate().unwrap();
    assert_eq!(notes(&b), vec!["acked by three 1"]);
}

// @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Uncommitted entry discarded]] {level: integration}
#[test]
fn uncommitted_entry_discarded() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 1, "committed");
    for d in [B, C, K1, K2] {
        f.set_up(d, false);
    }
    let err = a
        .append_turn(Position::new(1), &[turn(&f.object.iri(), 2, "lonely")])
        .unwrap_err();
    assert!(matches!(
        err,
        AppendError::Storage(StoreError::Unavailable(_))
    ));
    assert_eq!(a.read_committed(Position::ZERO).map(|r| r.len()).ok(), None);
    f.set_up(A, false);
    for d in [B, C, K1, K2] {
        f.set_up(d, true);
    }
    let b = f.custody_host(B);
    b.activate().unwrap();
    assert_eq!(notes(&b), vec!["committed 1"]);
}

// @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Inherited entries commit with the new epoch]] {level: integration}
#[test]
fn inherited_entries_commit_with_the_new_epoch() {
    let f = CustodyFixture::new(&[A, B, C], &[K1], Some(3));
    let a = f.custody_host(A);
    a.elect(5).unwrap();
    write(&a, &f, 1, 39, "turn");
    // Entries 40 to 42 reach only A and C.
    f.set_up(B, false);
    f.set_up(K1, false);
    let obj = f.object.iri();
    let tail = [
        turn(&obj, 40, "x"),
        turn(&obj, 41, "x"),
        turn(&obj, 42, "x"),
    ];
    assert!(
        a.append_turn(Position::new(39), &tail).is_err(),
        "no quorum in epoch 5"
    );
    assert_eq!(f.party(C).replica.head(), Position::new(42));
    f.set_up(A, false);
    f.set_up(B, true);
    f.set_up(K1, true);
    // Entries 40..42 written by A in epoch 5, held by C: B adopts them, but K1 drops appends.
    let b = f.custody_host(B);
    f.party(K1).transport.set_appends(false);
    let e = b.elect(6).unwrap();
    assert!(!e.committed, "only B and C accepted in epoch 6");
    assert!(
        b.read_committed(Position::ZERO).is_err(),
        "not readable before the quorum"
    );
    f.party(K1).transport.set_appends(true);
    assert!(b.catch_up().unwrap());
    let head = b.read_committed(Position::ZERO).unwrap().len();
    assert_eq!(
        head, 42,
        "40..42 commit once a quorum accepted them in epoch 6"
    );
}

// @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Stale tail replaced]] {level: integration}
#[test]
fn stale_tail_replaced() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(5).unwrap();
    write(&a, &f, 1, 40, "turn");
    for d in [B, K1, K2] {
        f.set_up(d, false);
    }
    let _ = a.append_turn(Position::new(40), &[turn(&f.object.iri(), 41, "stale")]);
    assert_eq!(f.party(C).replica.head(), Position::new(41));
    f.set_up(A, false);
    f.set_up(C, false);
    for d in [B, K1, K2] {
        f.set_up(d, true);
    }
    let b = f.custody_host(B);
    b.elect(6).unwrap();
    write(&b, &f, 41, 1, "fresh");
    f.set_up(C, true);
    write(&b, &f, 42, 1, "after");
    let c_entries = f.party(C).replica.entries();
    let b_entries = f.party(B).replica.entries();
    assert_eq!(
        c_entries[40].facts().unwrap().hash,
        b_entries[40].facts().unwrap().hash,
        "C's entry 41 was replaced"
    );
    assert!(f.party(C).replica.evidence().is_empty(), "not equivocation");
}

// ---- Encrypted Replicas At Keepers ----

// @tg: verifies:: [[openspec:object-custody#Encrypted Replicas At Keepers#Keeper cannot read state]] {level: integration}
#[test]
fn keeper_cannot_read_state() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 3, "card ending 4242");
    let keeper_store = format!("{:?}", f.party(K1).store.dataset());
    assert!(
        !keeper_store.contains("card ending"),
        "payload in clear at a keeper"
    );
    assert!(
        !keeper_store.contains("#note"),
        "predicate in clear at a keeper"
    );
    let host_store = format!("{:?}", f.party(B).store.dataset());
    assert!(
        host_store.contains("card ending"),
        "hosts hold clear entries"
    );
    assert_eq!(f.party(K1).replica.head(), Position::new(3));
}

// @tg: verifies:: [[openspec:object-custody#Encrypted Replicas At Keepers#New host decrypts keeper replicas]] {level: integration}
#[test]
fn new_host_decrypts_keeper_replicas() {
    let mut f = CustodyFixture::new(&[A], &[K1, K2], Some(2));
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 3, "turn");
    // The controller adds host D.
    let d = f.make_party("did:web:d.example.org");
    a.add_transport(d.transport.clone());
    f.parties.push(d);
    let mut cfg = f.config.clone();
    cfg.version = 2;
    cfg.custodians
        .push(Custodian::host("did:web:d.example.org"));
    cfg.quorum = 3;
    let req = ReconfigurationRequest {
        object: f.object.as_str().into(),
        config: cfg,
        did_log_entry: None,
        signature: String::new(),
    }
    .sign(&f.controller.signer)
    .unwrap();
    a.reconfigure(&req).unwrap();
    // D loses its disk, A disappears: D activates from the keepers' replicas only.
    f.restart("did:web:d.example.org");
    let dp = f.party("did:web:d.example.org").clone();
    let fresh = Arc::new(MemoryStore::new());
    let replica = Arc::new(
        vertep_objects::custody::Replica::open(
            f.object.iri(),
            Arc::new(dp.id.signer.clone()),
            fresh,
            f.directory.clone(),
            a.config(),
        )
        .unwrap(),
    );
    f.set_up(A, false);
    let host = CustodyHost::new(
        f.object.clone(),
        Arc::new(dp.id.signer.clone()),
        dp.id.keys.clone() as Arc<dyn KeyManager>,
        dp.id.agreement.clone(),
        replica,
        f.directory.clone(),
    )
    .with_transport(f.party(A).transport.clone())
    .with_transport(f.party(K1).transport.clone())
    .with_transport(f.party(K2).transport.clone());
    host.activate().unwrap();
    assert_eq!(notes(&host), vec!["turn 1", "turn 2", "turn 3"]);
}

// ---- Custody Handoff ----

// @tg: verifies:: [[openspec:object-custody#Custody Handoff#Planned move between organisations]] {level: integration}
#[test]
fn planned_move_between_organisations() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 3, "org1");
    assert_eq!(a.hand_off().unwrap(), Position::new(3));
    assert!(matches!(
        a.append_turn(Position::new(3), &[turn(&f.object.iri(), 4, "late")]),
        Err(AppendError::Fenced { .. })
    ));
    let b = f.custody_host(B);
    assert!(b.activate().unwrap() > 1);
    write(&b, &f, 4, 2, "org2");
    assert_eq!(
        notes(&b),
        vec!["org1 1", "org1 2", "org1 3", "org2 4", "org2 5"],
        "every entry exactly once, in order"
    );
}

// ---- Failover Without A Shared Authority ----

// @tg: verifies:: [[openspec:object-custody#Failover Without A Shared Authority#Writer's organisation goes offline]] {level: integration}
#[test]
fn writers_organisation_goes_offline() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 2, "turn");
    f.set_up(A, false);
    f.set_up(K1, false);
    let b = f.custody_host(B);
    assert!(b.activate().unwrap() > 1);
    write(&b, &f, 3, 1, "resumed");
    assert_eq!(notes(&b).len(), 3);
}

// @tg: verifies:: [[openspec:object-custody#Failover Without A Shared Authority#No quorum reachable]] {level: integration}
#[test]
fn no_quorum_reachable() {
    let f = five();
    for d in [A, C, K1] {
        f.set_up(d, false);
    }
    let b = f.custody_host(B);
    assert!(matches!(b.activate(), Err(CustodyError::NoQuorum { .. })));
    assert_eq!(b.writer_epoch(), None);
    for d in [A, C, K1] {
        f.set_up(d, true);
    }
    assert!(
        b.activate().is_ok(),
        "processing resumes once a quorum is reachable"
    );
}

// ---- Equivocation Evidence ----

#[derive(Default)]
struct Collect(Mutex<Vec<Evidence>>);

impl EvidenceSink for Collect {
    fn report(&self, e: &Evidence) {
        self.0.lock().unwrap().push(e.clone());
    }
}

// @tg: verifies:: [[openspec:object-custody#Equivocation Evidence#Forked writer detected]] {level: integration}
#[test]
fn forked_writer_detected() {
    let f = CustodyFixture::new(&[A], &[K1, K2], Some(2));
    let sink = Arc::new(Collect::default());
    f.party(K1).replica.set_evidence_sink(sink.clone());
    let a = &f.party(A).id;
    let key = ContentKey::generate(1).unwrap();
    let obj = f.object.iri();
    let entry = |note: &str| {
        let link = ChainLink {
            object: obj.clone(),
            position: Position::new(1),
            epoch: 5,
            writer: A.into(),
            prev: None,
        };
        let e = seal(&turn(&obj, 1, note), &link, &a.signer).unwrap();
        WireEntry::Sealed(
            vertep_objects::custody::wire::seal_for_keeper(&e, &key, &a.signer).unwrap(),
        )
    };
    let batch = |e: WireEntry| {
        AppendBatch {
            object: f.object.as_str().into(),
            epoch: 5,
            writer: A.into(),
            after: 0,
            entries: vec![e],
            config: f.config.clone(),
            next: None,
            keys: vec![],
            signature: String::new(),
        }
        .sign(&a.signer)
        .unwrap()
    };
    let k = &f.party(K1).replica;
    assert!(matches!(
        k.handle(&CustodyRequest::Append(batch(entry("X")))),
        CustodyResponse::Ack { .. }
    ));
    assert_eq!(
        k.handle(&CustodyRequest::Append(batch(entry("Y")))),
        CustodyResponse::Diverged {
            position: 1,
            evidence: true
        }
    );
    assert_eq!(k.evidence().len(), 1);
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "reported to the controller"
    );
    assert!(matches!(
        k.handle(&CustodyRequest::Append(batch(entry("X")))),
        CustodyResponse::Refused { .. }
    ));
}

// ---- Custodian Set Changes ----

fn replace_k1_with_k3(f: &mut CustodyFixture, a: &CustodyHost) -> ReconfigurationRequest {
    let k3 = f.make_party(K3);
    a.add_transport(k3.transport.clone());
    f.parties.push(k3);
    let mut cfg = a.config();
    cfg.version += 1;
    cfg.custodians.retain(|c| c.id != K1);
    cfg.custodians.push(Custodian::keeper(K3));
    ReconfigurationRequest {
        object: f.object.as_str().into(),
        config: cfg,
        did_log_entry: Some(
            serde_json::json!({"versionId": "2-Qm", "state": {"service": "custody"}}),
        ),
        signature: String::new(),
    }
}

// @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Replace a keeper]] {level: integration}
#[test]
fn replace_a_keeper() {
    let mut f = CustodyFixture::new(&[A, B], &[K1, K2], Some(3));
    let publisher = Arc::new(RecordingPublisher::default());
    let a = {
        let h = f.custody_host(A);
        Arc::try_unwrap(h)
            .unwrap()
            .with_publisher(publisher.clone())
    };
    a.elect(1).unwrap();
    write(&a, &f, 1, 5, "turn");
    let req = replace_k1_with_k3(&mut f, &a)
        .sign(&f.controller.signer)
        .unwrap();
    a.reconfigure(&req).unwrap();
    assert_eq!(
        f.party(K3).replica.head(),
        Position::new(5),
        "K3 holds the chain"
    );
    assert_eq!(a.config().version, 2);
    assert_eq!(
        publisher.published().len(),
        1,
        "the DID document update is published"
    );
    // The next epoch is won by the new set; K1's promise does not count.
    f.set_up(A, false);
    f.set_up(K2, false);
    let b = f.custody_host(B);
    b.add_transport(f.party(K3).transport.clone());
    assert!(
        matches!(b.activate(), Err(CustodyError::NoQuorum { .. })),
        "B, K3 and the removed K1 are not a quorum of the new set"
    );
    f.set_up(K2, true);
    assert!(b.activate().is_ok());
    assert_eq!(b.epochs().last().unwrap().config.version, 2);
}

// @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Unsigned change refused]] {level: unit}
#[test]
fn unsigned_change_refused() {
    let mut f = CustodyFixture::new(&[A, B], &[K1, K2], Some(3));
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    let rogue = TestHost::new("did:web:rogue.example.org");
    let req = replace_k1_with_k3(&mut f, &a).sign(&rogue.signer).unwrap();
    assert!(matches!(a.reconfigure(&req), Err(CustodyError::Invalid(_))));
    assert_eq!(a.config().version, 1);
    assert_eq!(f.party(K3).replica.head(), Position::ZERO);
}

// @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Old set cannot elect after a change]] {level: integration}
#[test]
fn old_set_cannot_elect_after_a_change() {
    let mut f = CustodyFixture::new(&[A, B], &[K1, K2], Some(3));
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 2, "turn");
    let req = replace_k1_with_k3(&mut f, &a)
        .sign(&f.controller.signer)
        .unwrap();
    a.reconfigure(&req).unwrap();
    // A host that still holds the old set writes at a high epoch with configuration 1.
    let b = &f.party(B).id;
    let stale = AppendBatch {
        object: f.object.as_str().into(),
        epoch: 99,
        writer: B.into(),
        after: 2,
        entries: vec![],
        config: f.config.clone(),
        next: None,
        keys: vec![],
        signature: String::new(),
    }
    .sign(&b.signer)
    .unwrap();
    for d in [K2, K3] {
        match f
            .party(d)
            .replica
            .handle(&CustodyRequest::Append(stale.clone()))
        {
            CustodyResponse::Refused { reason, .. } => assert!(reason.contains("older")),
            other => panic!("{d} accepted a stale configuration: {other:?}"),
        }
    }
    // And no new effects: the writer's journal is unchanged.
    assert_eq!(a.committed(), Position::new(2));
}

/// Regression (joint consensus): shrinking the set must not let stale members of the new set
/// elect a writer that lost a committed entry. The switch needs a quorum of the new set too.
// @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Shrink keeps committed entries]] {level: integration}
#[test]
fn shrink_keeps_committed_entries() {
    let f = CustodyFixture::new(&[A, B, C], &[K1, K2], Some(3));
    let c = f.custody_host(C);
    f.set_up(A, false);
    f.set_up(B, false);
    c.elect(1).unwrap();
    write(&c, &f, 1, 1, "L"); // committed by C, K1, K2
    let mut cfg = f.config.clone();
    cfg.version = 2;
    cfg.custodians.retain(|x| x.id != K1 && x.id != K2);
    cfg.quorum = 2;
    let req = ReconfigurationRequest {
        object: f.object.as_str().into(),
        config: cfg,
        did_log_entry: None,
        signature: String::new(),
    }
    .sign(&f.controller.signer)
    .unwrap();
    assert!(
        matches!(c.reconfigure(&req), Err(CustodyError::NotAccepted(_))),
        "no quorum of the new set holds the journal: the switch must wait"
    );
    // C disappears; A and B come back and one of them takes over.
    f.set_up(C, false);
    f.set_up(A, true);
    f.set_up(B, true);
    let a = f.custody_host(A);
    a.activate().unwrap();
    assert_eq!(notes(&a), vec!["L 1"], "the committed entry survives");
}

/// A link that answers its first `n` requests, then is unreachable.
struct Flaky {
    inner: Arc<LocalTransport>,
    left: Mutex<usize>,
}

impl CustodyTransport for Flaky {
    fn custodian(&self) -> &str {
        self.inner.custodian()
    }
    fn call(&self, r: &CustodyRequest) -> Result<CustodyResponse, LinkError> {
        let mut left = self.left.lock().unwrap();
        if *left == 0 {
            return Err(LinkError("gone".into()));
        }
        *left -= 1;
        self.inner.call(r)
    }
}

/// Regression (joint consensus): a change that was only started must not let a candidate win
/// with members of the new set that never held the journal. Under the old rule (prepare again
/// with the newer set alone) B would be elected by D, F and G with an empty chain and lose L.
// @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Interrupted change keeps one writer]] {level: integration}
#[test]
fn interrupted_change_keeps_one_writer() {
    const D: &str = "did:web:d.example.org";
    const F: &str = "did:web:f.example.net";
    const G: &str = "did:web:g.example.net";
    let mut f = CustodyFixture::new(&[A, B, C], &[], Some(2));
    for d in [D, F, G] {
        let p = f.make_party(d);
        f.parties.push(p);
    }
    let a = f.custody_host(A);
    f.set_up(C, false);
    a.elect(1).unwrap();
    write(&a, &f, 1, 1, "L"); // committed by A and B
    // A starts moving to {A, C, D, F, G} (quorum 3) but reaches only C, then dies.
    f.set_up(C, true);
    for d in [B, D, F, G] {
        f.set_up(d, false);
    }
    let mut cfg = CustodyConfig::new(
        vec![
            Custodian::host(A),
            Custodian::host(C),
            Custodian::host(D),
            Custodian::keeper(F),
            Custodian::keeper(G),
        ],
        3,
    )
    .unwrap();
    cfg.version = 2;
    let req = ReconfigurationRequest {
        object: f.object.as_str().into(),
        config: cfg,
        did_log_entry: None,
        signature: String::new(),
    }
    .sign(&f.controller.signer)
    .unwrap();
    assert!(matches!(
        a.reconfigure(&req),
        Err(CustodyError::NotAccepted(_))
    ));
    f.set_up(A, false);
    for d in [B, D, F, G] {
        f.set_up(d, true);
    }
    // B reaches C for one prepare only: C reports the pending change, then disappears.
    let pb = f.party(B);
    let mut b = CustodyHost::new(
        f.object.clone(),
        Arc::new(pb.id.signer.clone()),
        pb.id.keys.clone() as Arc<dyn KeyManager>,
        pb.id.agreement.clone(),
        pb.replica.clone(),
        f.directory.clone(),
    )
    .with_backoff(vertep_objects::custody::Backoff::immediate(4));
    for p in &f.parties {
        if p.id.did == C {
            b = b.with_transport(Arc::new(Flaky {
                inner: p.transport.clone(),
                left: Mutex::new(1),
            }));
        } else if p.id.did != B {
            b = b.with_transport(p.transport.clone());
        }
    }
    match b.activate() {
        // Joint consensus: without C, the old set {A, B, C} has no quorum, so B must wait.
        Err(CustodyError::NoQuorum { .. }) => {}
        Ok(_) => assert_eq!(
            notes(&b),
            vec!["L 1"],
            "elected only with the committed entry"
        ),
        Err(e) => panic!("{e}"),
    }
    assert_ne!(
        b.read_committed(Position::ZERO)
            .map(|r| r.len())
            .unwrap_or(1),
        0,
        "never a writer with an empty chain"
    );
    // Once C is back, B is elected under both sets and keeps L.
    b.add_transport(f.party(C).transport.clone());
    b.resume();
    b.activate().unwrap();
    assert_eq!(notes(&b), vec!["L 1"]);
}

// ---- durable-objects: Single Writer Per Object ----

// @tg: verifies:: [[openspec:durable-objects#Single Writer Per Object#Stale writer fenced]] {level: integration}
#[test]
fn stale_writer_fenced() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(3).unwrap();
    write(&a, &f, 1, 1, "a");
    let b = f.custody_host(B);
    b.elect(4).unwrap();
    let err = a
        .append_turn(Position::new(1), &[turn(&f.object.iri(), 2, "stale")])
        .unwrap_err();
    assert!(
        matches!(err, AppendError::Fenced { highest: 4, .. }),
        "{err}"
    );
    assert_eq!(a.writer_epoch(), None, "A deactivates the object");
    assert_eq!(notes(&b), vec!["a 1"]);
}

/// Regression: a deposed writer must not count its own replica toward a quorum once that
/// replica promised a newer candidate.
#[test]
fn deposed_writer_does_not_count_its_own_replica() {
    let f = CustodyFixture::new(&[A, B], &[K1], Some(2));
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 1, "a");
    // B wins epoch 2 from B and A's replica while K1 is away.
    f.set_up(K1, false);
    let b = f.custody_host(B);
    b.elect(2).unwrap();
    f.set_up(K1, true);
    f.set_up(B, false);
    // A still believes it writes epoch 1; only K1 is reachable besides its own replica.
    let err = a
        .append_turn(Position::new(1), &[turn(&f.object.iri(), 2, "stale")])
        .unwrap_err();
    assert!(
        matches!(err, AppendError::Fenced { highest: 2, .. }),
        "{err}"
    );
    assert_eq!(a.writer_epoch(), None);
}

// @tg: verifies:: [[openspec:durable-objects#Single Writer Per Object#Concurrent activations]] {level: integration}
#[test]
fn concurrent_activations() {
    for _ in 0..10 {
        let f = Arc::new(five());
        let a = f.custody_host(A);
        let b = f.custody_host(B);
        let (ra, rb) = thread::scope(|s| {
            let ta = s.spawn(|| a.elect(7));
            let tb = s.spawn(|| b.elect(7));
            (ta.join().unwrap(), tb.join().unwrap())
        });
        assert!(
            !(ra.is_ok() && rb.is_ok()),
            "two writers of epoch 7: {ra:?} {rb:?}"
        );
        // Whoever writes, only one chain exists.
        let w = if ra.is_ok() {
            &a
        } else if rb.is_ok() {
            &b
        } else {
            continue;
        };
        write(w, &f, 1, 1, "only");
    }
}

// ---- durable-objects: Activation From A Verified Replica ----

/// A transport that corrupts the entries it reads back.
struct Corrupting(Arc<LocalTransport>);

impl CustodyTransport for Corrupting {
    fn custodian(&self) -> &str {
        self.0.custodian()
    }
    fn call(&self, r: &CustodyRequest) -> Result<CustodyResponse, LinkError> {
        let mut answer = self.0.call(r)?;
        if let CustodyResponse::Entries { entries, .. } = &mut answer
            && let Some(WireEntry::Clear { nquads }) = entries.get_mut(1)
        {
            *nquads = nquads.replace("turn 2", "turn X");
        }
        Ok(answer)
    }
}

// @tg: verifies:: [[openspec:durable-objects#Activation From A Verified Replica#Activation on a new host]] {level: integration}
#[test]
fn activation_on_a_new_host() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 4, "turn");
    f.set_up(A, false);
    let b = f.custody_host(B);
    b.activate().unwrap();
    let before: Vec<_> = a.read_committed(Position::ZERO).unwrap();
    let after: Vec<_> = b.read_committed(Position::ZERO).unwrap();
    assert_eq!(before, after, "B adopted exactly A's committed chain");
}

// @tg: verifies:: [[openspec:durable-objects#Activation From A Verified Replica#Corrupt replica skipped]] {level: integration}
#[test]
fn corrupt_replica_skipped() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    f.set_up(B, false); // B's own copy lags, so C is the first replica B adopts from
    write(&a, &f, 1, 3, "turn");
    f.set_up(A, false);
    f.set_up(B, true);
    // B reaches C only through a corrupting link.
    let b = CustodyHost::new(
        f.object.clone(),
        Arc::new(f.party(B).id.signer.clone()),
        f.party(B).id.keys.clone() as Arc<dyn KeyManager>,
        f.party(B).id.agreement.clone(),
        f.party(B).replica.clone(),
        f.directory.clone(),
    )
    .with_transport(f.party(A).transport.clone())
    .with_transport(Arc::new(Corrupting(f.party(C).transport.clone())))
    .with_transport(f.party(K1).transport.clone())
    .with_transport(f.party(K2).transport.clone());
    let e = b.elect(9).unwrap();
    assert!(
        e.skipped
            .iter()
            .any(|(c, why)| c == C && why.contains("hash-mismatch")),
        "{e:?}"
    );
    assert_eq!(notes(&b), vec!["turn 1", "turn 2", "turn 3"]);
}

// @tg: verifies:: [[openspec:durable-objects#Activation From A Verified Replica#Longer chain from an older epoch loses]] {level: integration}
#[test]
fn longer_chain_from_an_older_epoch_loses() {
    let f = CustodyFixture::new(&[A, B, C], &[], None);
    let a = f.custody_host(A);
    a.elect(3).unwrap();
    write(&a, &f, 1, 45, "turn");
    f.set_up(B, false);
    f.set_up(C, false);
    let obj = f.object.iri();
    let tail: Vec<_> = (46..=50).map(|i| turn(&obj, i, "tail")).collect();
    assert!(a.append_turn(Position::new(45), &tail).is_err());
    assert_eq!(
        f.party(A).replica.head(),
        Position::new(50),
        "A holds 50 accepted in epoch 3"
    );
    f.set_up(A, false);
    f.set_up(B, true);
    f.set_up(C, true);
    let b = f.custody_host(B);
    b.elect(4).unwrap();
    f.set_up(A, true);
    let c = f.custody_host(C);
    c.elect(5).unwrap();
    assert_eq!(
        c.committed(),
        Position::new(45),
        "the chain accepted in epoch 4 wins"
    );
    assert_eq!(
        f.party(A).replica.head(),
        Position::new(45),
        "A's older tail is replaced"
    );
}

// ---- durable-objects: Effects After Durable Commit (custody level) ----

// @tg: verifies:: [[openspec:durable-objects#Effects After Durable Commit#Single-host object]] {level: integration}
#[test]
fn single_host_object() {
    let f = CustodyFixture::new(&[A], &[], Some(1));
    let a = f.custody_host(A);
    a.activate().unwrap();
    write(&a, &f, 1, 3, "turn");
    assert_eq!(a.committed(), Position::new(3));
}

// ---- durable-objects: Durable Object Lineage ----

// @tg: verifies:: [[openspec:durable-objects#Durable Object Lineage#Epoch history queried]] {level: integration}
#[test]
fn epoch_history_queried() {
    let f = five();
    let a = f.custody_host(A);
    a.elect(1).unwrap();
    write(&a, &f, 1, 3, "a");
    a.hand_off().unwrap();
    let b = f.custody_host(B);
    b.elect(2).unwrap();
    write(&b, &f, 4, 2, "b");
    let graph = MemoryStore::new();
    b.record_lineage(&graph).unwrap();
    let rows = graph
        .query(
            "PREFIX vt: <https://w3id.org/vertep/vt#>
             SELECT ?n ?host ?first ?last WHERE { GRAPH ?g {
               ?e a vt:CustodyEpoch ; vt:epochNumber ?n ; vt:epochHost ?host ;
                  vt:firstPosition ?first ; vt:lastPosition ?last } } ORDER BY ?n",
        )
        .unwrap()
        .into_solutions()
        .unwrap();
    assert_eq!(rows.len(), 2);
    let host = |r: usize| match rows.get(r, "host") {
        Some(Term::NamedNode(n)) => n.as_str().to_owned(),
        other => panic!("{other:?}"),
    };
    let int = |r: usize, v: &str| match rows.get(r, v) {
        Some(Term::Literal(l)) => l.value().parse::<u64>().unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (host(0), int(0, "first"), int(0, "last")),
        (A.to_owned(), 1, 3)
    );
    assert_eq!(
        (host(1), int(1, "first"), int(1, "last")),
        (B.to_owned(), 4, 5)
    );
    let _ = content_hash; // re-exported for other tests
    let _: Option<&dyn EntrySigner> = None;
}
