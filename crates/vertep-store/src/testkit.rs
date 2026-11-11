//! The shared backend test kit: the cases every [`JournalStore`] and [`GraphStore`] backend must
//! pass.
//!
//! A backend runs the kit from its own tests, with a factory that opens a fresh, empty store:
//!
//! ```ignore
//! vertep_store::journal_store_kit!(|| MyStore::open_temp());
//! vertep_store::graph_store_kit!(|| MyStore::open_temp());
//! vertep_store::combined_store_kit!(|| MyStore::open_temp()); // both traits on one store
//! ```
//!
//! Each macro expands to one `#[test]` per case. [`admit_journal_store`] and
//! [`admit_graph_store`] run every case and report the failures, which is how a new backend is
//! admitted. Every case uses its own IRIs, so the cases can also share one store.
//!
//! Journal cases: atomic multi-entry append, failed appends (conflict, invalid entry, stale
//! epoch) write nothing, entries are RDF, read after append, concurrent conflicting appends,
//! independent actors in parallel, duplicate entry IRIs, the three fencing cases. Graph cases:
//! quad round trip, idempotent insert, removal and clearing, querying current state, ASK and
//! query errors, and the declared RDF capability. Combined cases: entry graphs are queryable,
//! state deltas commit with their entries and roll back with them.

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Barrier;

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{BlankNode, GraphName, Literal, NamedNode, Quad, Term, Triple};

use crate::error::AppendError;
use crate::graph::{GraphStore, QueryResults, RdfCapability};
use crate::journal::{AppendMeta, Entry, FencingToken, JournalStore, Position, StateDelta};
use crate::reification::{self, Annotation};
use crate::vocab;

/// The namespace of every IRI the kit writes.
pub const KIT_NS: &str = "urn:vertep:kit:";

/// `urn:vertep:kit:<local>`.
pub fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("{KIT_NS}{local}")).expect("kit IRIs are valid")
}

/// A `vt:` term.
pub fn vt(local: &str) -> NamedNode {
    NamedNode::new(format!("{}{local}", vocab::VT)).expect("vt IRIs are valid")
}

/// The IRI of entry `tag` of `actor`.
pub fn entry_iri(actor: &NamedNode, tag: &str) -> NamedNode {
    NamedNode::new(format!("{}/entry/{tag}", actor.as_str())).expect("entry IRIs are valid")
}

/// A journal entry with the kinds of content the runtime writes: an input event with a name, a
/// JSON payload and a message id, a chart version, a send as a blank node, a timestamp and a
/// language-tagged note.
pub fn sample_entry(actor: &NamedNode, tag: &str) -> Entry {
    let e = entry_iri(actor, tag);
    let input = NamedNode::new(format!("{}#input", e.as_str())).expect("valid");
    let send = BlankNode::default();
    Entry::new(
        e.clone(),
        [
            Triple::new(e.clone(), vt("input"), input.clone()),
            Triple::new(input.clone(), rdf::TYPE, vt("InputEvent")),
            Triple::new(
                input.clone(),
                vt("eventName"),
                Literal::from(format!("ev.{tag}")),
            ),
            Triple::new(
                input.clone(),
                vt("payload"),
                Literal::from(format!(
                    "{{\"tag\":\"{tag}\",\"text\":\"a \\\"quoted\\\"\\nline\"}}"
                )),
            ),
            Triple::new(input, vt("messageId"), Literal::from(format!("msg-{tag}"))),
            Triple::new(e.clone(), vt("chartVersion"), iri("chart/v1")),
            Triple::new(
                e.clone(),
                vt("timestamp"),
                Literal::new_typed_literal("2026-11-11T10:00:00Z", xsd::DATE_TIME),
            ),
            Triple::new(
                e.clone(),
                vt("note"),
                Literal::new_language_tagged_literal_unchecked("entrée", "fr"),
            ),
            Triple::new(e.clone(), vt("send"), send.clone()),
            Triple::new(
                send.clone(),
                vt("sendId"),
                Literal::from(format!("send-{tag}")),
            ),
            Triple::new(
                send,
                vt("order"),
                Literal::new_typed_literal("1", xsd::INTEGER),
            ),
        ],
    )
}

fn names(store: &dyn JournalStore, actor: &NamedNode) -> Vec<NamedNode> {
    store
        .read(actor, Position::ZERO)
        .expect("read succeeds")
        .into_iter()
        .map(|r| r.entry.name().clone())
        .collect()
}

fn append_ok(
    store: &dyn JournalStore,
    actor: &NamedNode,
    expected: u64,
    tags: &[&str],
) -> Position {
    let entries: Vec<Entry> = tags.iter().map(|t| sample_entry(actor, t)).collect();
    store
        .append(
            actor,
            Position::new(expected),
            &entries,
            &AppendMeta::none(),
        )
        .unwrap_or_else(|e| panic!("append at {expected} failed: {e}"))
}

// ---------------------------------------------------------------------------------------------
// Journal cases

/// An append of three entries at expected position 10 succeeds, and a later read from 11
/// returns exactly those three entries, in order.
pub fn atomic_multi_entry_append(store: &dyn JournalStore) {
    let actor = iri("atomic/actor");
    let first: Vec<String> = (1..=10).map(|i| format!("pre{i}")).collect();
    let first: Vec<&str> = first.iter().map(String::as_str).collect();
    assert_eq!(append_ok(store, &actor, 0, &first), Position::new(10));

    let batch: Vec<Entry> = ["a", "b", "c"]
        .iter()
        .map(|t| sample_entry(&actor, t))
        .collect();
    let head = store
        .append(&actor, Position::new(10), &batch, &AppendMeta::none())
        .expect("append at the expected position succeeds");
    assert_eq!(head, Position::new(13));
    assert_eq!(store.head(&actor).unwrap(), Position::new(13));

    let read = store.read(&actor, Position::new(11)).unwrap();
    assert_eq!(read.len(), 3, "exactly the three appended entries");
    for (i, (rec, sent)) in read.iter().zip(&batch).enumerate() {
        let pos = Position::new(11 + i as u64);
        assert_eq!(rec.position, pos);
        assert_eq!(rec.entry.position(), Some(pos));
        let stamped = sent.stamp(&actor, pos).unwrap();
        assert!(
            rec.entry.is_isomorphic(&stamped),
            "entry {} read back differs:\n{:?}\nexpected\n{:?}",
            pos.get(),
            rec.entry,
            stamped
        );
    }
}

/// Appends that fail (a position conflict, an invalid entry in the middle of a batch) leave no
/// trace, and the fencing epoch of a failed append is not recorded either.
pub fn failed_append_writes_nothing(store: &dyn JournalStore) {
    let actor = iri("failed/actor");
    append_ok(store, &actor, 0, &["1", "2"]);
    let before = names(store, &actor);

    // position conflict
    let late = sample_entry(&actor, "late");
    let err = store
        .append(
            &actor,
            Position::new(1),
            std::slice::from_ref(&late),
            &AppendMeta::none(),
        )
        .expect_err("a stale expected position conflicts");
    assert_eq!(
        err,
        AppendError::Conflict {
            expected: Position::new(1),
            actual: Position::new(2)
        }
    );

    // a batch whose third entry is malformed: it states a position it will not get
    let bad = Entry::new(
        entry_iri(&actor, "bad"),
        [Triple::new(
            entry_iri(&actor, "bad"),
            vocab::node(vocab::POSITION),
            Literal::new_typed_literal("99", xsd::INTEGER),
        )],
    );
    let batch = [sample_entry(&actor, "x"), sample_entry(&actor, "y"), bad];
    let err = store
        .append(&actor, Position::new(2), &batch, &AppendMeta::none())
        .expect_err("a malformed entry fails the whole append");
    assert!(matches!(err, AppendError::Invalid(_)), "got {err:?}");

    assert_eq!(store.head(&actor).unwrap(), Position::new(2));
    assert_eq!(
        names(store, &actor),
        before,
        "no entry of a failed append is visible"
    );
    assert!(store.read(&actor, Position::new(3)).unwrap().is_empty());

    // the entry IRIs of failed appends are still free
    append_ok(store, &actor, 2, &["late", "x", "y"]);
    assert_eq!(store.head(&actor).unwrap(), Position::new(5));

    if store.supports_fencing() {
        let scope = iri("failed/shard");
        let meta = |epoch| AppendMeta::none().with_fencing(FencingToken::new(scope.clone(), epoch));
        let e = sample_entry(&actor, "epoch9");
        let err = store
            .append(&actor, Position::new(1), &[e], &meta(9))
            .expect_err("conflict");
        assert!(err.is_conflict());
        // epoch 9 was not recorded, so epoch 7 is still the highest accepted
        store
            .append(
                &actor,
                Position::new(5),
                &[sample_entry(&actor, "epoch7")],
                &meta(7),
            )
            .expect("the epoch of a failed append is not recorded");
    }
}

/// An entry read back is an RDF dataset with exactly one named graph, named by the entry IRI,
/// typed `vt:JournalEntry` and stamped with its actor and position.
pub fn entries_are_rdf(store: &dyn JournalStore) {
    let actor = iri("rdf/actor");
    append_ok(store, &actor, 0, &["only"]);
    let rec = store.read(&actor, Position::new(1)).unwrap().remove(0);
    let ds = rec.entry.to_dataset();
    let graphs: HashSet<GraphName> = ds.iter().map(|q| q.graph_name.into_owned()).collect();
    assert_eq!(
        graphs,
        HashSet::from([GraphName::NamedNode(entry_iri(&actor, "only"))]),
        "one named graph per entry, named by the entry IRI"
    );
    let e = entry_iri(&actor, "only");
    let g = rec.entry.triples();
    assert!(g.contains(&Triple::new(
        e.clone(),
        rdf::TYPE,
        vocab::node(vocab::JOURNAL_ENTRY)
    )));
    assert!(g.contains(&Triple::new(
        e.clone(),
        vocab::node(vocab::ACTOR),
        actor.clone()
    )));
    assert!(g.contains(&Triple::new(
        e.clone(),
        vocab::node(vocab::POSITION),
        Literal::new_typed_literal("1", xsd::INTEGER)
    )));
    assert!(
        g.iter()
            .all(|t| t.predicate.as_str().starts_with(vocab::VT) || t.predicate == rdf::TYPE),
        "every predicate is a vt: term or rdf:type"
    );
    assert_eq!(Entry::from_dataset(&ds).unwrap(), rec.entry);
    assert!(
        rec.entry.is_isomorphic(
            &sample_entry(&actor, "only")
                .stamp(&actor, Position::new(1))
                .unwrap()
        )
    );
}

/// Reads see every completed append, from any position, and the actor is listed.
pub fn read_after_append(store: &dyn JournalStore) {
    let actor = iri("read/actor");
    assert_eq!(store.head(&actor).unwrap(), Position::ZERO);
    assert!(store.read(&actor, Position::ZERO).unwrap().is_empty());
    append_ok(store, &actor, 0, &["1"]);
    assert_eq!(names(store, &actor), vec![entry_iri(&actor, "1")]);
    append_ok(store, &actor, 1, &["2", "3"]);
    let tail: Vec<u64> = store
        .read(&actor, Position::new(2))
        .unwrap()
        .iter()
        .map(|r| r.position.get())
        .collect();
    assert_eq!(tail, vec![2, 3]);
    assert!(store.read(&actor, Position::new(4)).unwrap().is_empty());
    assert_eq!(store.read(&actor, Position::new(1)).unwrap().len(), 3);
    assert_eq!(store.head(&actor).unwrap(), Position::new(3));
    assert!(store.actors().unwrap().contains(&actor));
}

/// Several threads append at the same expected position: exactly one wins each round, the
/// others get a conflict, and the journal holds only the winners' entries.
pub fn concurrent_conflicting_appends(store: &dyn JournalStore) {
    const THREADS: usize = 8;
    const ROUNDS: u64 = 5;
    let actor = iri("race/actor");
    let mut winners = Vec::new();
    for round in 0..ROUNDS {
        let barrier = Barrier::new(THREADS);
        let results: Vec<(NamedNode, Result<Position, AppendError>)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|t| {
                    let actor = actor.clone();
                    let barrier = &barrier;
                    s.spawn(move || {
                        let e = sample_entry(&actor, &format!("r{round}t{t}"));
                        barrier.wait();
                        let r = store.append(
                            &actor,
                            Position::new(round),
                            std::slice::from_ref(&e),
                            &AppendMeta::none(),
                        );
                        (e.name().clone(), r)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("no panic"))
                .collect()
        });
        let ok: Vec<&NamedNode> = results
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            ok.len(),
            1,
            "round {round}: exactly one append wins, got {results:?}"
        );
        for (_, r) in &results {
            match r {
                Ok(p) => assert_eq!(*p, Position::new(round + 1)),
                Err(e) => assert_eq!(
                    *e,
                    AppendError::Conflict {
                        expected: Position::new(round),
                        actual: Position::new(round + 1)
                    }
                ),
            }
        }
        winners.push(ok[0].clone());
    }
    assert_eq!(store.head(&actor).unwrap(), Position::new(ROUNDS));
    assert_eq!(
        names(store, &actor),
        winners,
        "only the winners are in the journal"
    );
}

/// Appends of different actors never conflict, also when they run in parallel.
pub fn independent_actors_in_parallel(store: &dyn JournalStore) {
    const ACTORS: usize = 4;
    const EACH: u64 = 10;
    std::thread::scope(|s| {
        for a in 0..ACTORS {
            s.spawn(move || {
                let actor = iri(&format!("parallel/actor{a}"));
                for n in 0..EACH {
                    append_ok(store, &actor, n, &[&format!("{n}")]);
                }
            });
        }
    });
    for a in 0..ACTORS {
        let actor = iri(&format!("parallel/actor{a}"));
        assert_eq!(store.head(&actor).unwrap(), Position::new(EACH));
        let positions: Vec<u64> = store
            .read(&actor, Position::ZERO)
            .unwrap()
            .iter()
            .map(|r| r.position.get())
            .collect();
        assert_eq!(
            positions,
            (1..=EACH).collect::<Vec<_>>(),
            "gap-free positions"
        );
    }
}

/// An entry IRI is unique in the store: appending it again, to any actor, is invalid.
pub fn duplicate_entry_rejected(store: &dyn JournalStore) {
    let a = iri("dup/a");
    let b = iri("dup/b");
    append_ok(store, &a, 0, &["1"]);
    let again = sample_entry(&a, "1");
    let err = store
        .append(
            &a,
            Position::new(1),
            std::slice::from_ref(&again),
            &AppendMeta::none(),
        )
        .expect_err("an entry IRI is used once");
    assert!(matches!(err, AppendError::Invalid(_)), "got {err:?}");
    let err = store
        .append(&b, Position::ZERO, &[again], &AppendMeta::none())
        .expect_err("also in another actor's journal");
    assert!(matches!(err, AppendError::Invalid(_)), "got {err:?}");
    assert_eq!(store.head(&a).unwrap(), Position::new(1));
    assert_eq!(store.head(&b).unwrap(), Position::ZERO);
}

/// An append without a token is checked only by compare-and-set, whatever epochs other appends
/// carried.
pub fn unfenced_append_unchanged(store: &dyn JournalStore) {
    let actor = iri("unfenced/actor");
    if store.supports_fencing() {
        let token = FencingToken::new(iri("unfenced/shard"), 50);
        store
            .append(
                &iri("unfenced/other"),
                Position::ZERO,
                &[sample_entry(&iri("unfenced/other"), "1")],
                &AppendMeta::none().with_fencing(token),
            )
            .expect("a fenced append on another actor");
    }
    append_ok(store, &actor, 0, &["1"]);
    append_ok(store, &actor, 1, &["2"]);
    let err = store
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "3")],
            &AppendMeta::none(),
        )
        .expect_err("only compare-and-set decides");
    assert!(err.is_conflict(), "got {err:?}");
    assert_eq!(store.head(&actor).unwrap(), Position::new(2));
}

/// After a store accepted (shard S, epoch 5), an append with (shard S, epoch 4) fails with a
/// fencing error and writes nothing, even though its expected position matches. Stores without
/// fencing refuse tokens instead of ignoring them.
pub fn stale_epoch_rejected(store: &dyn JournalStore) {
    let actor = iri("fenced/actor");
    let shard = iri("fenced/shard-S");
    let meta = |epoch| AppendMeta::none().with_fencing(FencingToken::new(shard.clone(), epoch));
    if !store.supports_fencing() {
        let err = store
            .append(
                &actor,
                Position::ZERO,
                &[sample_entry(&actor, "1")],
                &meta(5),
            )
            .expect_err("a store without fencing refuses tokens");
        assert!(matches!(err, AppendError::Unsupported(_)), "got {err:?}");
        assert_eq!(store.head(&actor).unwrap(), Position::ZERO);
        return;
    }
    store
        .append(
            &actor,
            Position::ZERO,
            &[sample_entry(&actor, "1")],
            &meta(5),
        )
        .expect("epoch 5 accepted");
    let err = store
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "2")],
            &meta(4),
        )
        .expect_err("epoch 4 is stale");
    assert_eq!(
        err,
        AppendError::Fenced {
            scope: shard.clone(),
            epoch: 4,
            highest: 5
        }
    );
    assert_eq!(store.head(&actor).unwrap(), Position::new(1));
    assert_eq!(names(store, &actor), vec![entry_iri(&actor, "1")]);
    // the same epoch and higher ones are accepted; other scopes are independent
    store
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "2")],
            &meta(5),
        )
        .expect("the same epoch is accepted");
    store
        .append(
            &actor,
            Position::new(2),
            &[sample_entry(&actor, "3")],
            &meta(6),
        )
        .expect("a higher epoch is accepted");
    let err = store
        .append(
            &actor,
            Position::new(3),
            &[sample_entry(&actor, "4")],
            &meta(5),
        )
        .expect_err("5 is now stale");
    assert!(err.is_fenced());
    store
        .append(
            &actor,
            Position::new(3),
            &[sample_entry(&actor, "4")],
            &AppendMeta::none().with_fencing(FencingToken::new(iri("fenced/shard-T"), 1)),
        )
        .expect("scopes are independent");
}

/// The runtime can read whether a store enforces fencing, and the answer matches what the
/// store does with a stale token.
pub fn fencing_support_is_declared(store: &dyn JournalStore) {
    let declared = store.supports_fencing();
    assert_eq!(declared, store.capabilities().fencing);
    let actor = iri("declared/actor");
    let scope = iri("declared/shard");
    let fenced = |epoch| AppendMeta::none().with_fencing(FencingToken::new(scope.clone(), epoch));
    let first = store.append(
        &actor,
        Position::ZERO,
        &[sample_entry(&actor, "1")],
        &fenced(2),
    );
    if declared {
        first.expect("a fencing store accepts a first token");
        let stale = store.append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "2")],
            &fenced(1),
        );
        assert!(stale.expect_err("declared fencing is enforced").is_fenced());
    } else {
        assert!(matches!(first, Err(AppendError::Unsupported(_))));
    }
}

// ---------------------------------------------------------------------------------------------
// Graph cases

fn construct(store: &dyn GraphStore, graph: &NamedNode) -> HashSet<Triple> {
    store
        .query(&format!(
            "CONSTRUCT {{ ?s ?p ?o }} WHERE {{ GRAPH {graph} {{ ?s ?p ?o }} }}"
        ))
        .expect("CONSTRUCT runs")
        .into_graph()
        .expect("a graph")
        .into_iter()
        .collect()
}

fn sample_triples(base: &str) -> Vec<Triple> {
    let s = iri(&format!("{base}/s"));
    vec![
        Triple::new(s.clone(), iri("p/iri"), iri(&format!("{base}/o"))),
        Triple::new(
            s.clone(),
            iri("p/str"),
            Literal::from("plain \"text\"\nwith escapes"),
        ),
        Triple::new(
            s.clone(),
            iri("p/lang"),
            Literal::new_language_tagged_literal_unchecked("bonjour", "fr"),
        ),
        Triple::new(
            s.clone(),
            iri("p/int"),
            Literal::new_typed_literal("42", xsd::INTEGER),
        ),
        Triple::new(
            s.clone(),
            iri("p/bool"),
            Literal::new_typed_literal("true", xsd::BOOLEAN),
        ),
        Triple::new(
            s.clone(),
            iri("p/date"),
            Literal::new_typed_literal("2026-11-11T10:00:00Z", xsd::DATE_TIME),
        ),
        Triple::new(
            s,
            iri("p/custom"),
            Literal::new_typed_literal("x-1", iri("datatype/custom")),
        ),
    ]
}

/// Quads written to a named graph come back unchanged from a `CONSTRUCT` over that graph, and a
/// quad in the default graph is matched by a query without `GRAPH`.
pub fn quad_round_trip(store: &dyn GraphStore) {
    let g = iri("roundtrip/g");
    let triples = sample_triples("roundtrip");
    let quads: Vec<Quad> = triples
        .iter()
        .map(|t| t.clone().in_graph(g.clone()))
        .collect();
    store.insert_quads(&quads).expect("insert");
    assert_eq!(
        construct(store, &g),
        triples.into_iter().collect::<HashSet<_>>()
    );

    let d = Triple::new(iri("roundtrip/d"), iri("p/iri"), iri("roundtrip/e"));
    store
        .insert_quads(&[d.clone().in_graph(GraphName::DefaultGraph)])
        .expect("insert into the default graph");
    let ask = format!("ASK {{ {} {} {} }}", d.subject, d.predicate, d.object);
    assert!(store.query(&ask).unwrap().into_boolean().unwrap());
}

/// Inserting the same quads again changes nothing.
pub fn idempotent_insert(store: &dyn GraphStore) {
    let g = iri("idem/g");
    let quads: Vec<Quad> = sample_triples("idem")
        .into_iter()
        .map(|t| t.in_graph(g.clone()))
        .collect();
    store.insert_quads(&quads).unwrap();
    let once = construct(store, &g);
    store.insert_quads(&quads).unwrap();
    store.insert_quads(&quads[..2]).unwrap();
    assert_eq!(construct(store, &g), once);
    let rows = store
        .query(&format!(
            "SELECT ?s ?p ?o WHERE {{ GRAPH {g} {{ ?s ?p ?o }} }}"
        ))
        .unwrap()
        .into_solutions()
        .unwrap();
    assert_eq!(rows.len(), quads.len(), "no duplicate rows");
}

/// Removed quads and cleared graphs are gone from `GRAPH` queries; other graphs are untouched.
pub fn remove_and_clear(store: &dyn GraphStore) {
    let g = iri("remove/g");
    let h = iri("remove/h");
    let t: Vec<Triple> = sample_triples("remove");
    let in_g: Vec<Quad> = t.iter().map(|x| x.clone().in_graph(g.clone())).collect();
    let in_h: Vec<Quad> = t.iter().map(|x| x.clone().in_graph(h.clone())).collect();
    store.insert_quads(&in_g).unwrap();
    store.insert_quads(&in_h).unwrap();
    store.remove_quads(&in_g[..1]).unwrap();
    let left = construct(store, &g);
    assert_eq!(left.len(), t.len() - 1);
    assert!(!left.contains(&t[0]));
    // removing an absent quad is not an error
    store.remove_quads(&in_g[..1]).unwrap();
    store.clear_graph(&g).unwrap();
    assert!(construct(store, &g).is_empty());
    assert_eq!(
        construct(store, &h).len(),
        t.len(),
        "other graphs are untouched"
    );
}

/// A SPARQL query for the active states of an actor in its current-state graph returns them,
/// and only them.
pub fn query_current_state(store: &dyn GraphStore) {
    let actor = iri("state/order-17");
    let other = iri("state/order-18");
    let active = vt("activeState");
    let quads = vec![
        Quad::new(
            actor.clone(),
            active.clone(),
            iri("chart#Paid"),
            actor.clone(),
        ),
        Quad::new(
            actor.clone(),
            active.clone(),
            iri("chart#Shipping"),
            actor.clone(),
        ),
        Quad::new(
            other.clone(),
            active.clone(),
            iri("chart#Cancelled"),
            other.clone(),
        ),
    ];
    store.insert_quads(&quads).unwrap();
    let q = format!(
        "SELECT ?state WHERE {{ GRAPH {actor} {{ {actor} {active} ?state }} }} ORDER BY ?state"
    );
    let s = store.query(&q).unwrap().into_solutions().unwrap();
    assert_eq!(s.variables, vec!["state".to_string()]);
    assert_eq!(
        s.values("state"),
        vec![
            Term::from(iri("chart#Paid")),
            Term::from(iri("chart#Shipping"))
        ]
    );
}

/// ASK answers, and invalid SPARQL is an `Invalid` error rather than a panic or a backend error.
pub fn ask_and_errors(store: &dyn GraphStore) {
    let g = iri("ask/g");
    store
        .insert_quads(&[Quad::new(
            iri("ask/s"),
            iri("ask/p"),
            iri("ask/o"),
            g.clone(),
        )])
        .unwrap();
    let yes = format!(
        "ASK {{ GRAPH {g} {{ {} {} ?o }} }}",
        iri("ask/s"),
        iri("ask/p")
    );
    assert!(matches!(store.query(&yes), Ok(QueryResults::Boolean(true))));
    let no = format!(
        "ASK {{ GRAPH {g} {{ {} {} ?o }} }}",
        iri("ask/x"),
        iri("ask/p")
    );
    assert!(matches!(store.query(&no), Ok(QueryResults::Boolean(false))));
    let bad = store.query("SELECT ?x WHERE { ?x");
    assert!(
        matches!(bad, Err(crate::StoreError::Invalid(_))),
        "invalid SPARQL is Invalid, got {bad:?}"
    );
}

/// The store declares its RDF capability, and the declaration is true: on an RDF 1.2 store an
/// annotation written through a reifier is found by an annotation query; on an RDF 1.1 store
/// the `rdf:Statement` fallback round-trips.
pub fn capability_is_declared(store: &dyn GraphStore) {
    let caps = store.capabilities();
    assert_eq!(caps.rdf, store.rdf_capability());
    let g = iri("annot/g");
    let taken = Triple::new(iri("annot/transition-3"), vt("taken"), Literal::from(true));
    let annotation = Annotation::new(g.clone(), taken.clone())
        .with_reifier(iri("annot/r1"))
        .with(
            vt("confidence"),
            Literal::new_typed_literal("0.75", xsd::DECIMAL),
        )
        .with(vt("causedBy"), iri("annot/entry-9"));
    reification::write_annotations(store, std::slice::from_ref(&annotation)).expect("write");
    let back = reification::read_annotations(store, &GraphName::NamedNode(g.clone()), &taken)
        .expect("read");
    assert_eq!(back.len(), 1, "one annotation, got {back:?}");
    assert!(back[0].same_content(&annotation), "got {back:?}");
    let (s, p, o) = (&taken.subject, &taken.predicate, &taken.object);
    match caps.rdf {
        RdfCapability::Rdf12 => {
            let q = format!(
                "SELECT ?c WHERE {{ GRAPH {g} {{ {s} {p} {o} {{| {c} ?c |}} }} }}",
                c = vt("confidence")
            );
            let sol = store.query(&q).unwrap().into_solutions().unwrap();
            assert_eq!(
                sol.len(),
                1,
                "an RDF 1.2 annotation query finds the annotation"
            );
        }
        RdfCapability::Rdf11 => {
            let q = format!(
                "ASK {{ GRAPH {g} {{ ?r a {st} ; {rs} {s} ; {rp} {p} ; {ro} {o} }} }}",
                st = rdf::STATEMENT,
                rs = rdf::SUBJECT,
                rp = rdf::PREDICATE,
                ro = rdf::OBJECT,
            );
            assert!(store.query(&q).unwrap().into_boolean().unwrap());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Combined cases: both traits on one store

/// After an append, each entry is a named graph of the same store, queryable by SPARQL.
pub fn entry_graphs_are_queryable<S: JournalStore + GraphStore>(store: &S) {
    let actor = iri("combined/actor");
    append_ok(store, &actor, 0, &["1", "2"]);
    for (n, tag) in [(1, "1"), (2, "2")] {
        let e = entry_iri(&actor, tag);
        let q = format!(
            "SELECT ?n WHERE {{ GRAPH {e} {{ {e} {p} ?n }} }}",
            p = vocab::node(vocab::POSITION)
        );
        let s = store.query(&q).unwrap().into_solutions().unwrap();
        assert_eq!(
            s.values("n"),
            vec![Term::from(Literal::new_typed_literal(
                n.to_string(),
                xsd::INTEGER
            ))]
        );
    }
}

fn active_states<S: GraphStore>(store: &S, actor: &NamedNode) -> Vec<Term> {
    let q = format!(
        "SELECT ?s WHERE {{ GRAPH {actor} {{ {actor} {a} ?s }} }} ORDER BY ?s",
        a = vt("activeState")
    );
    store
        .query(&q)
        .unwrap()
        .into_solutions()
        .unwrap()
        .values("s")
}

/// On a store that commits state with entries, the current-state graph changes with the append
/// that causes it, and not at all when the append fails.
pub fn state_commits_with_entries<S: JournalStore + GraphStore>(store: &S) {
    let actor = iri("state-tx/actor");
    let active =
        |s: &str| Triple::new(actor.clone(), vt("activeState"), iri(&format!("chart#{s}")));
    let delta = StateDelta {
        retract: vec![],
        assert: vec![active("Paid")],
    };
    let meta = AppendMeta::none().with_state(delta);
    if !JournalStore::capabilities(store).state_in_append {
        let err = store
            .append(&actor, Position::ZERO, &[sample_entry(&actor, "1")], &meta)
            .expect_err("a delta is refused when not supported");
        assert!(matches!(err, AppendError::Unsupported(_)));
        return;
    }
    store
        .append(&actor, Position::ZERO, &[sample_entry(&actor, "1")], &meta)
        .unwrap();
    assert_eq!(
        active_states(store, &actor),
        vec![Term::from(iri("chart#Paid"))]
    );

    let next = AppendMeta::none().with_state(StateDelta {
        retract: vec![active("Paid")],
        assert: vec![active("Shipped")],
    });
    // a conflicting append carries the delta too, and changes nothing
    let err = store
        .append(&actor, Position::ZERO, &[sample_entry(&actor, "x")], &next)
        .expect_err("conflict");
    assert!(err.is_conflict());
    assert_eq!(
        active_states(store, &actor),
        vec![Term::from(iri("chart#Paid"))]
    );

    store
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "2")],
            &next,
        )
        .unwrap();
    assert_eq!(
        active_states(store, &actor),
        vec![Term::from(iri("chart#Shipped"))]
    );
}

// ---------------------------------------------------------------------------------------------
// Running the kit

/// A named kit case on stores of type `T`.
pub type Case<T> = (&'static str, fn(&T));

/// A named journal case.
pub type JournalCase = Case<dyn JournalStore>;

/// A named graph case.
pub type GraphCase = Case<dyn GraphStore>;

/// Every journal case, by name.
pub const JOURNAL_CASES: &[JournalCase] = &[
    ("atomic_multi_entry_append", atomic_multi_entry_append),
    ("failed_append_writes_nothing", failed_append_writes_nothing),
    ("entries_are_rdf", entries_are_rdf),
    ("read_after_append", read_after_append),
    (
        "concurrent_conflicting_appends",
        concurrent_conflicting_appends,
    ),
    (
        "independent_actors_in_parallel",
        independent_actors_in_parallel,
    ),
    ("duplicate_entry_rejected", duplicate_entry_rejected),
    ("unfenced_append_unchanged", unfenced_append_unchanged),
    ("stale_epoch_rejected", stale_epoch_rejected),
    ("fencing_support_is_declared", fencing_support_is_declared),
];

/// Every graph case, by name.
pub const GRAPH_CASES: &[GraphCase] = &[
    ("quad_round_trip", quad_round_trip),
    ("idempotent_insert", idempotent_insert),
    ("remove_and_clear", remove_and_clear),
    ("query_current_state", query_current_state),
    ("ask_and_errors", ask_and_errors),
    ("capability_is_declared", capability_is_declared),
];

fn run_cases<T: ?Sized>(cases: &[Case<T>], open: &dyn Fn() -> Box<T>) -> Result<(), Vec<String>> {
    let mut failed = Vec::new();
    for (name, case) in cases {
        let store = open();
        let outcome = catch_unwind(AssertUnwindSafe(|| case(&*store)));
        if let Err(panic) = outcome {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            failed.push(format!("{name}: {msg}"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed)
    }
}

// @tg: implements:: [[openspec:storage#Shared Backend Test Kit]]
/// Runs every journal case, each on a fresh store from `open`, and admits the backend only if
/// all pass. The error lists each failed case with its message.
///
/// # Errors
///
/// The failed cases.
pub fn admit_journal_store<S: JournalStore + 'static>(
    open: impl Fn() -> S,
) -> Result<(), Vec<String>> {
    run_cases::<dyn JournalStore>(JOURNAL_CASES, &|| Box::new(open()))
}

// @tg: implements:: [[openspec:storage#Shared Backend Test Kit]]
/// Runs every graph case, each on a fresh store from `open`, and admits the backend only if all
/// pass.
///
/// # Errors
///
/// The failed cases.
pub fn admit_graph_store<S: GraphStore + 'static>(open: impl Fn() -> S) -> Result<(), Vec<String>> {
    run_cases::<dyn GraphStore>(GRAPH_CASES, &|| Box::new(open()))
}

/// One `#[test]` per journal case, each on a fresh store from `$open`.
#[macro_export]
macro_rules! journal_store_kit {
    ($open:expr) => {
        $crate::__kit_tests!(journal, $open;
            atomic_multi_entry_append, failed_append_writes_nothing, entries_are_rdf,
            read_after_append, concurrent_conflicting_appends, independent_actors_in_parallel,
            duplicate_entry_rejected, unfenced_append_unchanged, stale_epoch_rejected,
            fencing_support_is_declared);
    };
}

/// One `#[test]` per graph case, each on a fresh store from `$open`.
#[macro_export]
macro_rules! graph_store_kit {
    ($open:expr) => {
        $crate::__kit_tests!(graph, $open;
            quad_round_trip, idempotent_insert, remove_and_clear, query_current_state,
            ask_and_errors, capability_is_declared);
    };
}

/// One `#[test]` per combined case (both traits on one store), each on a fresh store.
#[macro_export]
macro_rules! combined_store_kit {
    ($open:expr) => {
        $crate::__kit_tests!(combined, $open; entry_graphs_are_queryable, state_commits_with_entries);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __kit_tests {
    ($kind:ident, $open:expr; $($case:ident),+ $(,)?) => {
        $(
            #[test]
            fn $case() {
                let store = ($open)();
                $crate::testkit::$case(&store);
            }
        )+
    };
}
