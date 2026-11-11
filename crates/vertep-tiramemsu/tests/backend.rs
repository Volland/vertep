//! Tiramemsu-specific requirements: one file for both traits, the append transaction, state
//! with its cause, time travel and one file per shard.

mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::TempShard;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Quad, Term, Triple};
use vertep_store::testkit::{entry_iri, iri, sample_entry, vt};
use vertep_store::{
    AppendError, AppendMeta, GraphStore, JournalStore, Position, QueryResults, StateDelta,
    StoreError, vocab,
};
use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value};
use vertep_tiramemsu::{TiramemsuBackend, TiramemsuShard, shard_file_name, shard_of};

fn active(actor: &NamedNode, state: &str) -> Triple {
    Triple::new(
        actor.clone(),
        vt("activeState"),
        iri(&format!("chart#{state}")),
    )
}

fn active_states(store: &dyn GraphStore, actor: &NamedNode) -> Vec<Term> {
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

fn state(name: &str) -> Vec<Term> {
    vec![Term::from(iri(&format!("chart#{name}")))]
}

fn step(
    shard: &TiramemsuShard,
    actor: &NamedNode,
    expected: u64,
    tag: &str,
    exit: &[&str],
    enter: &[&str],
) -> Result<Position, AppendError> {
    let delta = StateDelta {
        retract: exit.iter().map(|s| active(actor, s)).collect(),
        assert: enter.iter().map(|s| active(actor, s)).collect(),
    };
    shard.append(
        actor,
        Position::new(expected),
        &[sample_entry(actor, tag)],
        &AppendMeta::none().with_state(delta),
    )
}

// @tg: verifies:: [[openspec:storage#Tiramemsu Backend#Both traits from one file]] {level: integration}
#[test]
fn both_traits_from_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shard-0.db");
    let actor = iri("one-file/actor");
    {
        let shard = TiramemsuShard::open(&path).unwrap();
        let journal: &dyn JournalStore = &shard;
        let graph: &dyn GraphStore = &shard;
        journal
            .append(
                &actor,
                Position::ZERO,
                &[sample_entry(&actor, "1")],
                &AppendMeta::none(),
            )
            .unwrap();
        // the graph store sees the journal's entry graph...
        let e = entry_iri(&actor, "1");
        let ask = format!(
            "ASK {{ GRAPH {e} {{ {e} {} {actor} }} }}",
            vocab::node(vocab::ACTOR)
        );
        assert_eq!(graph.query(&ask).unwrap(), QueryResults::Boolean(true));
        // ...and what the graph store writes lives in the same file
        graph
            .insert_quads(&[Quad::new(
                iri("dl/1"),
                vt("receiver"),
                actor.clone(),
                actor.clone(),
            )])
            .unwrap();
        // a second handle on the same file in this process is refused: one writer per file
        let again = TiramemsuShard::open(&path).unwrap_err();
        assert!(matches!(again, StoreError::Unavailable(_)), "{again}");
    }
    // reopened, both halves read the same file
    let shard = TiramemsuShard::open(&path).unwrap();
    assert_eq!(shard.head(&actor).unwrap(), Position::new(1));
    assert_eq!(shard.read(&actor, Position::ZERO).unwrap().len(), 1);
    let ask = format!(
        "ASK {{ GRAPH {actor} {{ {} {} {actor} }} }}",
        iri("dl/1"),
        vt("receiver")
    );
    assert_eq!(shard.query(&ask).unwrap(), QueryResults::Boolean(true));
}

// @tg: verifies:: [[openspec:storage#Tiramemsu Backend#Compare-and-set inside one transaction]] {level: integration}
#[test]
fn compare_and_set_inside_one_transaction() {
    let t = TempShard::new();
    let shard = &t.shard;
    let actor = iri("cas/actor");
    let before = shard.last_tx().unwrap();
    let batch = [
        sample_entry(&actor, "1"),
        sample_entry(&actor, "2"),
        sample_entry(&actor, "3"),
    ];
    let delta = StateDelta {
        retract: vec![],
        assert: vec![active(&actor, "Open")],
    };
    shard
        .append(
            &actor,
            Position::ZERO,
            &batch,
            &AppendMeta::none().with_state(delta),
        )
        .unwrap();
    let after = shard.last_tx().unwrap();
    assert_eq!(
        after,
        before + 1,
        "one append is exactly one tiramemsu transaction"
    );

    // the head, every entry statement and the state were all written by that transaction
    let h = shard.db().history();
    let added: Vec<u64> = h
        .triples(None, None, None)
        .unwrap()
        .into_iter()
        .filter(|row| row.t_add.0 > before)
        .map(|row| row.t_add.0)
        .collect();
    assert!(
        added.len() > 3 * 11,
        "entries, stamps, memberships, head and state"
    );
    assert!(added.iter().all(|&ta| ta == after));

    // a conflicting append is checked in its transaction and rolled back: no transaction row
    let err = shard
        .append(
            &actor,
            Position::new(1),
            &[sample_entry(&actor, "x")],
            &AppendMeta::none(),
        )
        .unwrap_err();
    assert_eq!(
        err,
        AppendError::Conflict {
            expected: Position::new(1),
            actual: Position::new(3)
        }
    );
    assert_eq!(shard.last_tx().unwrap(), after);

    // and racing appends on the single writer: exactly one wins per position
    vertep_store::testkit::concurrent_conflicting_appends(shard);
}

// @tg: verifies:: [[openspec:storage#Input And State In One Transaction#No projection lag]] {level: integration}
#[test]
fn no_projection_lag() {
    let t = TempShard::new();
    let actor = iri("lag/actor");
    for (n, (exit, enter)) in [(vec![], vec!["Created"]), (vec!["Created"], vec!["Paid"])]
        .into_iter()
        .enumerate()
    {
        let n = n as u64;
        step(&t.shard, &actor, n, &format!("{}", n + 1), &exit, &enter).unwrap();
        // the very next read, on the store and on a fresh now view, sees the new configuration
        assert_eq!(active_states(&t.shard, &actor), state(enter[0]));
        assert_eq!(active_states(&t.shard.view_now(), &actor), state(enter[0]));
    }
}

// @tg: verifies:: [[openspec:storage#Input And State In One Transaction#Rolled back together]] {level: integration}
#[test]
fn rolled_back_together() {
    let t = TempShard::new();
    let shard = &t.shard;
    let actor = iri("rollback/actor");
    step(shard, &actor, 0, "1", &[], &["Created"]).unwrap();

    // a schema rule makes the state write fail after the entry has been written in the
    // transaction: vt:retries only takes integers
    let retries = vt("retries");
    shard
        .db()
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                Value::iri(retries.as_str()),
                Value::iri("urn:tiramemsu:sys:valueType"),
                Value::iri(xsd::INTEGER.as_str()),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let before = shard.last_tx().unwrap();
    let delta = StateDelta {
        retract: vec![active(&actor, "Created")],
        assert: vec![
            active(&actor, "Paid"),
            Triple::new(actor.clone(), retries, Literal::from("not a number")),
        ],
    };
    let entry = sample_entry(&actor, "2");
    let err = shard
        .append(
            &actor,
            Position::new(1),
            std::slice::from_ref(&entry),
            &AppendMeta::none().with_state(delta),
        )
        .unwrap_err();
    assert!(matches!(err, AppendError::Storage(_)), "{err:?}");

    // neither the entry nor any state change is visible
    assert_eq!(shard.last_tx().unwrap(), before);
    assert_eq!(shard.head(&actor).unwrap(), Position::new(1));
    assert!(shard.read(&actor, Position::new(2)).unwrap().is_empty());
    let e = entry.name();
    let ask = format!("ASK {{ GRAPH {e} {{ ?s ?p ?o }} }}");
    assert_eq!(shard.query(&ask).unwrap(), QueryResults::Boolean(false));
    assert_eq!(active_states(shard, &actor), state("Created"));
}

// @tg: verifies:: [[openspec:storage#State Facts Carry Their Cause#Why is the actor in this state]] {level: integration}
#[test]
fn why_is_the_actor_in_this_state() {
    let t = TempShard::new();
    let shard = &t.shard;
    let actor = iri("why/order-7");
    step(shard, &actor, 0, "1", &[], &["Paid"]).unwrap();
    step(shard, &actor, 1, "2", &["Paid"], &["Shipped"]).unwrap();
    step(shard, &actor, 2, "3", &[], &["Invoiced"]).unwrap();

    // following the support of the active-state statement leads to entry 2 and its input
    let shipped = active(&actor, "Shipped");
    let causes = shard.state_causes(&actor, &shipped).unwrap();
    assert_eq!(causes, vec![entry_iri(&actor, "2")]);
    let rec = shard.read(&actor, Position::new(2)).unwrap().remove(0);
    assert_eq!(rec.entry.name(), &causes[0]);
    let input = NamedNode::new(format!("{}#input", causes[0].as_str())).unwrap();
    let name: Vec<_> = rec
        .entry
        .triples()
        .objects_for_subject_predicate(&input, &vt("eventName"))
        .map(|o| o.into_owned())
        .collect();
    assert_eq!(name, vec![Term::from(Literal::from("ev.2"))]);

    // the same link in tiramemsu's own terms: the state statement is derived from the entry's
    // rdf:type statement, and View::dependents of that statement contains the state statement
    let now = shard.db().now();
    let enc = |v: &str| now.encode(&Value::iri(v)).unwrap().unwrap();
    let state_rows = now
        .triples(
            Some(enc(actor.as_str())),
            Some(enc(vt("activeState").as_str())),
            Some(enc(iri("chart#Shipped").as_str())),
        )
        .unwrap();
    assert_eq!(state_rows.len(), 1);
    let state_eid = state_rows[0].eid;
    let anchor = now
        .triples(
            Some(enc(entry_iri(&actor, "2").as_str())),
            Some(enc(vocab::RDF_TYPE)),
            Some(enc(vocab::JOURNAL_ENTRY)),
        )
        .unwrap();
    assert_eq!(anchor.len(), 1);
    let supports: Vec<_> = now
        .triples(
            Some(state_eid.oid()),
            Some(enc("urn:tiramemsu:sys:derivedFrom")),
            None,
        )
        .unwrap()
        .into_iter()
        .map(|r| r.o)
        .collect();
    assert_eq!(supports, vec![anchor[0].eid.oid()]);
    assert!(now.dependents(anchor[0].eid).unwrap().contains(&state_eid));
    // a state never re-entered keeps its first cause
    assert_eq!(
        shard
            .state_causes(&actor, &active(&actor, "Invoiced"))
            .unwrap(),
        vec![entry_iri(&actor, "3")]
    );
}

// @tg: verifies:: [[openspec:storage#State Facts Carry Their Cause#Superseded state keeps history]] {level: integration}
#[test]
fn superseded_state_keeps_history() {
    let t = TempShard::new();
    let shard = &t.shard;
    let actor = iri("history/order-8");
    step(shard, &actor, 0, "1", &[], &["Shipped"]).unwrap();
    let t1 = shard.last_tx().unwrap();
    step(shard, &actor, 1, "2", &["Shipped"], &["Delivered"]).unwrap();
    let t2 = shard.last_tx().unwrap();

    assert_eq!(active_states(shard, &actor), state("Delivered"));
    // retracted, not deleted: its lifetime is recorded and the past view still shows it
    let shipped = active(&actor, "Shipped");
    assert_eq!(
        shard.state_history(&actor, &shipped).unwrap(),
        vec![(t1, Some(t2))]
    );
    assert_eq!(
        active_states(&shard.view_as_of(t1), &actor),
        state("Shipped")
    );
    let h = shard.db().history();
    let enc = |v: &str| h.encode(&Value::iri(v)).unwrap().unwrap();
    let rows = h
        .triples(
            Some(enc(actor.as_str())),
            Some(enc(vt("activeState").as_str())),
            None,
        )
        .unwrap();
    assert_eq!(rows.len(), 2, "both states remain in the history view");
}

// @tg: verifies:: [[openspec:storage#Time Travel Views#State as of a past transaction]] {level: integration}
#[test]
fn state_as_of_a_past_transaction() {
    let t = TempShard::new();
    let shard = &t.shard;
    let actor = iri("travel/actor");
    let other = iri("travel/other");
    step(shard, &actor, 0, "1", &[], &["A"]).unwrap();
    let t1 = shard.last_tx().unwrap();
    step(shard, &actor, 1, "2", &["A"], &["B"]).unwrap();
    let t2 = shard.last_tx().unwrap();
    // later transactions that are not this actor's appends
    step(shard, &other, 0, "1", &[], &["X"]).unwrap();
    let t2b = shard.last_tx().unwrap();
    step(shard, &actor, 2, "3", &["B"], &["C"]).unwrap();
    let t3 = shard.last_tx().unwrap();

    assert_eq!(active_states(&shard.view_as_of(t1), &actor), state("A"));
    assert_eq!(active_states(&shard.view_as_of(t2), &actor), state("B"));
    // the states after the last append committed at or before t
    assert_eq!(active_states(&shard.view_as_of(t2b), &actor), state("B"));
    assert_eq!(active_states(&shard.view_as_of(t3), &actor), state("C"));
    assert_eq!(active_states(shard, &actor), state("C"));
    assert!(active_states(&shard.view_as_of(t1 - 1), &actor).is_empty());
    // views are read-only
    assert!(matches!(
        shard.view_as_of(t1).insert_quads(&[]),
        Err(StoreError::Unsupported(_))
    ));
}

/// An actor IRI that lands in shard `want` of `count`.
fn actor_in(want: usize, count: usize, tag: &str) -> NamedNode {
    (0..)
        .map(|i| iri(&format!("shards/{tag}/{i}")))
        .find(|a| shard_of(a.as_str(), count) == want)
        .unwrap()
}

// @tg: verifies:: [[openspec:storage#One File Per Shard#Shards append independently]] {level: integration}
#[test]
fn shards_append_independently() {
    let dir = tempfile::tempdir().unwrap();
    let backend = TiramemsuBackend::open(dir.path(), 2).unwrap();
    let a0 = actor_in(0, 2, "a");
    let b0 = actor_in(0, 2, "b");
    let a1 = actor_in(1, 2, "a");

    std::thread::scope(|s| {
        // hold shard 0's single writer in a long transaction
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = s.spawn(|| {
            backend.shards()[0]
                .db()
                .transact(TxOptions::default(), move |_tx| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        started_rx.recv().unwrap();

        // an append in shard 1 completes while shard 0's writer is busy
        let (done1_tx, done1_rx) = mpsc::channel();
        let (backend, a1r, b0r) = (&backend, &a1, &b0);
        s.spawn(move || {
            let a1 = a1r;
            let r = backend.append(
                a1,
                Position::ZERO,
                &[sample_entry(a1, "1")],
                &AppendMeta::none(),
            );
            done1_tx.send(r).unwrap();
        });
        let r = done1_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("shard 1 does not wait for shard 0's writer");
        assert_eq!(r.unwrap(), Position::new(1));

        // an append in shard 0 does wait for its own writer
        let (done0_tx, done0_rx) = mpsc::channel();
        s.spawn(move || {
            let r = backend.append(
                b0r,
                Position::ZERO,
                &[sample_entry(b0r, "1")],
                &AppendMeta::none(),
            );
            done0_tx.send(r).unwrap();
        });
        assert!(done0_rx.recv_timeout(Duration::from_millis(300)).is_err());
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(
            done0_rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap(),
            Position::new(1)
        );
    });
    assert_eq!(backend.head(&a0).unwrap(), Position::ZERO);
    assert_eq!(backend.head(&a1).unwrap(), Position::new(1));
    assert_eq!(backend.head(&b0).unwrap(), Position::new(1));
}

// @tg: verifies:: [[openspec:storage#One File Per Shard#Single node uses a configurable shard count]] {level: integration}
#[test]
fn single_node_uses_a_configurable_shard_count() {
    let dir = tempfile::tempdir().unwrap();
    let actors: Vec<NamedNode> = (0..64).map(|i| iri(&format!("count/order-{i}"))).collect();
    {
        let backend = TiramemsuBackend::open(dir.path(), 8).unwrap();
        assert_eq!(backend.shard_count(), 8);
        for a in &actors {
            backend
                .append(
                    a,
                    Position::ZERO,
                    &[sample_entry(a, "1")],
                    &AppendMeta::none(),
                )
                .unwrap();
        }
        let mut used = [false; 8];
        for a in &actors {
            let i = shard_of(a.as_str(), 8);
            assert_eq!(backend.shard_index(a), i);
            used[i] = true;
            // the actor's journal is in its shard's file and in no other
            for (j, shard) in backend.shards().iter().enumerate() {
                let expected = if i == j { 1 } else { 0 };
                assert_eq!(shard.head(a).unwrap(), Position::new(expected));
            }
        }
        assert!(used.iter().all(|u| *u), "64 actors spread over all 8 files");
        assert_eq!(backend.actors().unwrap().len(), actors.len());
    }
    let mut files: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".db"))
        .collect();
    files.sort();
    let expected: Vec<String> = (0..8).map(|i| shard_file_name(i, 8)).collect();
    assert_eq!(files, expected, "8 files, one per shard");
    // the placement is part of the layout: reopening with another count is refused
    assert!(matches!(
        TiramemsuBackend::open(dir.path(), 4),
        Err(StoreError::Invalid(_))
    ));
    let again = TiramemsuBackend::open(dir.path(), 8).unwrap();
    assert!(
        actors
            .iter()
            .all(|a| again.head(a).unwrap() == Position::new(1))
    );
}
