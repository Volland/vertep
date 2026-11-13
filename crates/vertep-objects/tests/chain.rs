//! Verifiable journal entries: sealing, verification and the round trip through journal stores.

use oxrdf::{Literal, NamedNode, Triple};
use vertep_objects::testkit::{StaticHostKeys, TestHost};
use vertep_objects::{
    ChainHead, ChainLink, DurableObjectId, FailureReason, content_hash, read_link, seal,
    verify_chain,
};
use vertep_store::{AppendMeta, Entry, JournalRecord, JournalStore, MemoryStore, Position};

fn object() -> DurableObjectId {
    DurableObjectId::new("did:webvh:QmScid:example.org:objects:counter").unwrap()
}

fn payload(object: &NamedNode, n: u64, note: &str) -> Entry {
    let name = NamedNode::new(format!("{}/entry/{n}", object.as_str())).unwrap();
    let p = NamedNode::new("https://w3id.org/vertep/vt#note").unwrap();
    Entry::new(
        name.clone(),
        [Triple::new(name, p, Literal::new_simple_literal(note))],
    )
}

/// Seals `n` entries in epoch 1 by `host`, each linked to the previous.
fn sealed_chain(host: &TestHost, object: &NamedNode, n: u64) -> Vec<Entry> {
    let mut prev = None;
    (1..=n)
        .map(|i| {
            let link = ChainLink {
                object: object.clone(),
                position: Position::new(i),
                epoch: 1,
                writer: host.did.clone(),
                prev,
            };
            let e = seal(
                &payload(object, i, &format!("turn {i}")),
                &link,
                &host.signer,
            )
            .unwrap();
            prev = Some(content_hash(&e));
            e
        })
        .collect()
}

fn records(entries: &[Entry]) -> Vec<JournalRecord> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| JournalRecord {
            position: Position::new(i as u64 + 1),
            entry: e.clone(),
        })
        .collect()
}

// @tg: verifies:: [[openspec:durable-objects#Verifiable Journal Entries#Chain verified from genesis]] {level: unit}
#[test]
fn chain_verified_from_genesis() {
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let chain = sealed_chain(&host, &id.iri(), 5);
    let keys = StaticHostKeys::new().with(&host, 1);
    let head = verify_chain(&id.iri(), ChainHead::EMPTY, &records(&chain), &keys).unwrap();
    assert_eq!(head.position, Position::new(5));
    assert_eq!(head.epoch, 1);
    assert_eq!(head.hash, Some(content_hash(&chain[4])));
    let link = read_link(&chain[2]).unwrap();
    assert_eq!(link.link.prev, Some(content_hash(&chain[1])));
    assert_eq!(link.link.writer, host.did);
}

// @tg: verifies:: [[openspec:durable-objects#Verifiable Journal Entries#Tampered entry rejected]] {level: unit}
#[test]
fn tampered_entry_rejected() {
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let mut chain = sealed_chain(&host, &id.iri(), 4);
    // Change one byte of entry 3's payload, keeping its seal.
    let tampered: Vec<Triple> = chain[2]
        .triples()
        .iter()
        .map(|t| {
            let t = t.into_owned();
            if t.predicate.as_str().ends_with("#note") {
                Triple::new(
                    t.subject,
                    t.predicate,
                    Literal::new_simple_literal("turn 4"),
                )
            } else {
                t
            }
        })
        .collect();
    chain[2] = Entry::new(chain[2].name().clone(), tampered);
    let keys = StaticHostKeys::new().with(&host, 1);
    let err = verify_chain(&id.iri(), ChainHead::EMPTY, &records(&chain), &keys).unwrap_err();
    assert_eq!(err.position, Position::new(3));
    assert_eq!(err.reason, FailureReason::HashMismatch);
    assert_eq!(err.reason.code(), "hash-mismatch");
}

// @tg: verifies:: [[openspec:durable-objects#Verifiable Journal Entries#Unauthorised signer rejected]] {level: unit}
#[test]
fn unauthorised_signer_rejected() {
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let intruder = TestHost::new("did:web:intruder.example.org");
    let mut chain = sealed_chain(&host, &id.iri(), 2);
    let link = ChainLink {
        object: id.iri(),
        position: Position::new(3),
        epoch: 1,
        writer: intruder.did.clone(),
        prev: Some(content_hash(&chain[1])),
    };
    chain.push(seal(&payload(&id.iri(), 3, "forged"), &link, &intruder.signer).unwrap());
    let keys = StaticHostKeys::new().with(&host, 1);
    let err = verify_chain(&id.iri(), ChainHead::EMPTY, &records(&chain), &keys).unwrap_err();
    assert_eq!(err.position, Position::new(3));
    assert_eq!(err.reason.code(), "not-a-host");
}

#[test]
fn signature_by_wrong_key_is_bad_signature() {
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let impostor = TestHost::new("did:web:host-a.example.org");
    let chain = sealed_chain(&impostor, &id.iri(), 1);
    let keys = StaticHostKeys::new().with(&host, 1);
    let err = verify_chain(&id.iri(), ChainHead::EMPTY, &records(&chain), &keys).unwrap_err();
    assert_eq!(err.reason, FailureReason::BadSignature);
}

fn round_trip(store: &dyn JournalStore) {
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let chain = sealed_chain(&host, &id.iri(), 3);
    store
        .append(&id.iri(), Position::ZERO, &chain[..2], &AppendMeta::none())
        .unwrap();
    store
        .append(
            &id.iri(),
            Position::new(2),
            &chain[2..],
            &AppendMeta::none(),
        )
        .unwrap();
    let back = store.read(&id.iri(), Position::ZERO).unwrap();
    let keys = StaticHostKeys::new().with(&host, 1);
    let head = verify_chain(&id.iri(), ChainHead::EMPTY, &back, &keys).unwrap();
    assert_eq!(head.position, Position::new(3));
}

#[test]
fn sealed_entries_round_trip_through_the_memory_store() {
    round_trip(&MemoryStore::new());
}

#[test]
fn sealed_entries_round_trip_through_tiramemsu() {
    let dir = tempfile::tempdir().unwrap();
    let shard = vertep_tiramemsu::TiramemsuShard::open(dir.path().join("o.db")).unwrap();
    round_trip(&shard);
}

#[test]
fn kit_sample_entry_with_blank_nodes_round_trips_through_tiramemsu() {
    // The kit's sample entry has a blank node; the hash relabels with RDFC-1.0, so a store
    // that renames blank nodes keeps the hash.
    let id = object();
    let host = TestHost::new("did:web:host-a.example.org");
    let raw = vertep_store::testkit::sample_entry(&id.iri(), "1");
    let link = ChainLink {
        object: id.iri(),
        position: Position::new(1),
        epoch: 1,
        writer: host.did.clone(),
        prev: None,
    };
    let sealed = seal(&raw, &link, &host.signer).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let shard = vertep_tiramemsu::TiramemsuShard::open(dir.path().join("o.db")).unwrap();
    shard
        .append(&id.iri(), Position::ZERO, &[sealed], &AppendMeta::none())
        .unwrap();
    let back = shard.read(&id.iri(), Position::ZERO).unwrap();
    let keys = StaticHostKeys::new().with(&host, 1);
    verify_chain(&id.iri(), ChainHead::EMPTY, &back, &keys).unwrap();
}
