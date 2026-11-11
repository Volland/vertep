//! Scenarios of the `runtime-ontology` capability.
mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vertep_runtime::ontology::{rdfs_closure, runtime_ontology, validate, validate_entry};
use vertep_runtime::{Address, Migration, Node, NodeConfig, Storage, VirtualClock, vocab};
use vertep_store::oxrdf::{
    Graph, GraphNameRef, NamedNode, NamedOrBlankNodeRef, Quad, TermRef, Triple,
};
use vertep_store::{GraphStore, JournalStore, MemoryStore, Position, Projector, RdfCapability};

const SINK: &str = r##"<state id="s"><transition event="*"/></state>"##;

/// Exercises every kind of runtime record: entries with sends and timers, snapshots, dead
/// letters, a migration and a fork.
fn exercise(node: &mut Node, clock: &VirtualClock) -> (Address, Address) {
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let b = va("sink", "b");
    let v1 = node
        .register_type(
            &ty("busy"),
            scxml(
                "a",
                &format!(
                    r##"<state id="a"><transition event="go" target="b"><send event="hi" target="{b}"/><send event="later" delay="1s" id="t1"/></transition></state>
                    <state id="b"><transition event="later"/><transition event="back" target="a"/></state>"##
                ),
            ),
        )
        .unwrap();
    let a = va("busy", "a");
    node.send(&a, ev("go")).unwrap();
    node.send(&a, ev("unknown")).unwrap();
    node.run_until_idle();
    clock.advance(Duration::from_secs(1));
    node.run_until_idle();
    let v2 = node
        .register_type(
            &ty("busy"),
            scxml(
                "a",
                r##"<state id="a"><onentry><log label="v2"/></onentry><transition event="go" target="b"/></state>
                <state id="b"><transition event="back" target="a"/></state>"##,
            ),
        )
        .unwrap();
    node.register_migration(Migration::identity("urn:vertep:migration:m", v1, v2));
    node.migrate(&a, "urn:vertep:migration:m").unwrap();
    node.send(&a, ev("back")).unwrap();
    node.run_until_idle();
    node.fork(&a, 2, &Address::parse("urn:vertep:test:fork").unwrap())
        .unwrap();
    (a, b)
}

fn snapshotting(store: Arc<dyn Storage>, clock: &VirtualClock) -> Node {
    node_on(
        store,
        clock,
        NodeConfig {
            snapshot_interval: 1,
            ..NodeConfig::default()
        },
    )
}

fn runtime_classes() -> BTreeSet<String> {
    let g = runtime_ontology();
    let owl_class = NamedNode::new("http://www.w3.org/2002/07/owl#Class").unwrap();
    g.subjects_for_predicate_object(vertep_store::oxrdf::vocab::rdf::TYPE, &owl_class)
        .filter_map(|s| match s {
            NamedOrBlankNodeRef::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

fn types_of(store: &MemoryStore, s: &str) -> BTreeSet<String> {
    store
        .dataset()
        .iter()
        .filter(|q| {
            q.subject.to_string() == format!("<{s}>")
                && q.predicate == vertep_store::oxrdf::vocab::rdf::TYPE
        })
        .filter_map(|q| match q.object {
            TermRef::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

// @tg: verifies:: [[openspec:runtime-ontology#Runtime Vocabulary#Every runtime record is typed]] {level: integration}
#[test]
fn every_runtime_record_is_typed() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = snapshotting(store.clone(), &clock);
    let (a, _) = exercise(&mut node, &clock);
    let classes = runtime_classes();
    let ds = store.dataset();
    let mut records: BTreeSet<String> = BTreeSet::new();
    for q in ds.iter() {
        let p = q.predicate.as_str();
        if let NamedOrBlankNodeRef::NamedNode(s) = q.subject
            && [
                vocab::MESSAGE_ID,
                vocab::DUE_AT,
                vocab::SESSION_STATE,
                vocab::REASON,
            ]
            .contains(&p)
        {
            records.insert(s.as_str().to_owned());
        }
        if p == vocab::MIGRATION_P
            && let TermRef::NamedNode(m) = q.object
        {
            records.insert(m.as_str().to_owned());
        }
    }
    let kinds = [
        ("/entry/", "entry"),
        ("/snapshot/", "snapshot"),
        ("/timer/", "timer"),
        ("/dead-letter/", "dead letter"),
        ("urn:vertep:migration:", "migration"),
    ];
    for (needle, what) in kinds {
        assert!(
            records.iter().any(|r| r.contains(needle)),
            "a {what} was written"
        );
    }
    for r in &records {
        let t = types_of(&store, r);
        assert!(
            t.iter().any(|c| classes.contains(c)),
            "{r} has a runtime type, has {t:?}"
        );
    }
    assert!(types_of(&store, a.as_str()).contains(vocab::VIRTUAL_ACTOR));
}

// @tg: verifies:: [[openspec:runtime-ontology#Runtime Vocabulary#Chart-only users need no runtime module]] {level: integration}
#[test]
fn chart_only_users_need_no_runtime_module() {
    let chart = scxml(
        "a",
        r##"<state id="a"><transition event="go" target="b"><send event="x" delay="1s"/></transition></state><state id="b"/>"##,
    );
    let projected = vertep_rdf::project(&chart);
    let report = vertep_rdf::validate(&projected.graph);
    assert!(report.conforms(), "{report}");
    let chart_terms: BTreeSet<String> = {
        let g = vertep_rdf::chart_ontology();
        g.iter()
            .filter_map(|t| match t.subject {
                NamedOrBlankNodeRef::NamedNode(n) => Some(n.as_str().to_owned()),
                _ => None,
            })
            .collect()
    };
    let runtime_only: BTreeSet<String> = runtime_ontology()
        .iter()
        .filter_map(|t| match t.subject {
            NamedOrBlankNodeRef::NamedNode(n) if n.as_str().starts_with(vocab::VT) => {
                Some(n.as_str().to_owned())
            }
            _ => None,
        })
        .filter(|n| !chart_terms.contains(n))
        .collect();
    assert!(!runtime_only.is_empty());
    for t in projected.graph.iter() {
        assert!(!runtime_only.contains(t.predicate.as_str()), "{t}");
        if let TermRef::NamedNode(o) = t.object {
            assert!(!runtime_only.contains(o.as_str()), "{t}");
        }
    }
}

fn graphs_of(ds: &vertep_store::oxrdf::Dataset) -> std::collections::BTreeMap<String, Graph> {
    let mut out: std::collections::BTreeMap<String, Graph> = Default::default();
    for q in ds.iter() {
        if let GraphNameRef::NamedNode(g) = q.graph_name {
            out.entry(g.as_str().to_owned())
                .or_default()
                .insert(&Triple::new(
                    q.subject.into_owned(),
                    q.predicate.into_owned(),
                    q.object.into_owned(),
                ));
        }
    }
    out
}

// @tg: verifies:: [[openspec:runtime-ontology#Runtime SHACL Shapes#Written entries conform]] {level: integration}
#[test]
fn written_entries_conform() {
    // the in-memory reference backend: every graph the runtime wrote
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = snapshotting(store.clone(), &clock);
    exercise(&mut node, &clock);
    let mut checked = 0;
    for (name, g) in graphs_of(&store.dataset()) {
        let r = validate(&g);
        assert!(r.conforms(), "{name}: {r}");
        checked += 1;
    }
    assert!(checked > 10);
    // the tiramemsu backend: entries read back conform
    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(vertep_tiramemsu::TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    let clock = VirtualClock::new(T0);
    let mut node = snapshotting(shard.clone(), &clock);
    exercise(&mut node, &clock);
    let actors = shard.actors().unwrap();
    assert!(actors.len() >= 3);
    let mut entries = 0;
    for actor in actors {
        for rec in shard.read(&actor, Position::ZERO).unwrap() {
            let r = validate_entry(&rec.entry);
            assert!(r.conforms(), "{}: {r}", rec.entry.name());
            entries += 1;
        }
    }
    assert!(entries >= 5);
    // transition annotations reached tiramemsu's graph store (as annotation syntax)
    let q = format!(
        "SELECT ?t WHERE {{ GRAPH ?g {{ ?a <{p}> ?t }} FILTER(STRENDS(STR(?g), \"/annotations\")) }}",
        p = vocab::TOOK_TRANSITION
    );
    let sol = shard.query(&q).unwrap().into_solutions().unwrap();
    assert!(!sol.is_empty());
}

// @tg: verifies:: [[openspec:runtime-ontology#Runtime SHACL Shapes#Malformed entry detected]] {level: integration}
#[test]
fn malformed_entry_detected() {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "a");
    node.send(&a, ev("x")).unwrap();
    node.run_until_idle();
    let rec = store.read(a.node(), Position::ZERO).unwrap().remove(0);
    let e = rec.entry.name().clone();
    for missing in [vocab::MESSAGE_ID, vocab::CHART_VERSION] {
        let mut g = rec.entry.triples().clone();
        let drop: Vec<Triple> = g
            .iter()
            .filter(|t| t.subject == e.as_ref().into() && t.predicate.as_str() == missing)
            .map(|t| t.into_owned())
            .collect();
        assert_eq!(drop.len(), 1);
        for t in &drop {
            g.remove(t);
        }
        let r = validate(&g);
        assert!(!r.conforms());
        assert!(
            r.results.iter().any(|v| v.path.as_ref().is_some_and(|p| p
                .to_string()
                .contains(missing.rsplit('#').next().unwrap_or(missing)))),
            "{r}"
        );
    }
}

// @tg: verifies:: [[openspec:runtime-ontology#PROV-O Alignment Without Dependency#Standard provenance query]] {level: integration}
#[test]
fn standard_provenance_query() {
    let (mut node, _clock, store) = fresh();
    node.register_type(&ty("sink"), scxml("s", SINK)).unwrap();
    let a = va("sink", "a");
    node.send(&a, ev("x")).unwrap();
    node.run_until_idle();
    let rec = store.read(a.node(), Position::ZERO).unwrap().remove(0);
    let input = format!("{}/input", rec.entry.name().as_str());
    // what an RDFS-aware store holds: the entry plus the entailments of the runtime module
    let closed = rdfs_closure(rec.entry.triples(), &runtime_ontology());
    let rdfs_store = MemoryStore::new();
    let quads: Vec<Quad> = closed
        .iter()
        .map(|t| {
            t.into_owned()
                .in_graph(vertep_store::oxrdf::GraphName::DefaultGraph)
        })
        .collect();
    rdfs_store.insert_quads(&quads).unwrap();
    let q = format!(
        "PREFIX prov: <http://www.w3.org/ns/prov#>
         SELECT ?act WHERE {{ ?act a prov:Activity ; prov:used <{input}> }}"
    );
    let sol = rdfs_store.query(&q).unwrap().into_solutions().unwrap();
    assert_eq!(sol.len(), 1);
    assert_eq!(
        sol.get(0, "act").unwrap().to_string(),
        rec.entry.name().to_string()
    );
    // without inference the same query finds nothing: the data itself names only vt: terms
    let plain = MemoryStore::new();
    plain.insert_quads(&rec.entry.quads()).unwrap();
    let q2 = q.replace("SELECT ?act WHERE {", "SELECT ?act WHERE { GRAPH ?g {") + " }";
    assert!(
        plain
            .query(&q2)
            .unwrap()
            .into_solutions()
            .unwrap()
            .is_empty()
    );
}

// @tg: verifies:: [[openspec:runtime-ontology#PROV-O Alignment Without Dependency#Runtime works without inference]] {level: integration}
#[test]
fn runtime_works_without_inference() {
    // MemoryStore does no inference; snapshots, dead letters, timer recovery and stored
    // mailboxes are all read back with vt: queries alone
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut n1 = snapshotting(store.clone(), &clock);
    let (a, _) = exercise(&mut n1, &clock);
    assert!(!n1.dead_letters(&a).unwrap().is_empty());
    n1.register_type(
        &ty("t"),
        scxml("s", r##"<state id="s"><transition event="arm"><send event="ring" delay="5s"/></transition><transition event="ring" target="rung"/></state><state id="rung"/>"##),
    )
    .unwrap();
    let t = va("t", "x");
    n1.send(&t, ev("arm")).unwrap();
    n1.run_until_idle();
    drop(n1);
    let mut n2 = snapshotting(store.clone(), &clock);
    n2.register_type(
        &ty("t"),
        scxml("s", r##"<state id="s"><transition event="arm"><send event="ring" delay="5s"/></transition><transition event="ring" target="rung"/></state><state id="rung"/>"##),
    )
    .unwrap();
    clock.advance(Duration::from_secs(5));
    n2.run_until_idle();
    assert_eq!(n2.state(&t).unwrap().configuration, vec!["rung"]);
    let replayed = n2.stats().replayed;
    n2.deactivate(&t);
    n2.activate_actor(&t).unwrap();
    assert_eq!(
        n2.stats().replayed,
        replayed,
        "activated from the snapshot found by a vt: query"
    );
    let prov = store
        .dataset()
        .iter()
        .filter(|q| {
            q.predicate
                .as_str()
                .starts_with("http://www.w3.org/ns/prov#")
        })
        .count();
    assert_eq!(prov, 0, "no PROV-O term is written or needed");
}

// @tg: verifies:: [[openspec:runtime-ontology#Triple Term Annotations#Annotating a transition taken]] {level: integration}
#[test]
fn annotating_a_transition_taken() {
    for rdf in [RdfCapability::Rdf12, RdfCapability::Rdf11] {
        let store = Arc::new(MemoryStore::new().with_rdf(rdf));
        let clock = VirtualClock::new(T0);
        let mut node = node_on(store.clone(), &clock, NodeConfig::default());
        let v = node
            .register_type(
                &ty("t"),
                scxml(
                    "a",
                    r##"<state id="a"><transition event="go" target="b"/></state><state id="b"/>"##,
                ),
            )
            .unwrap();
        let a = va("t", "x");
        node.send(&a, ev("go")).unwrap();
        node.run_until_idle();
        let rec = store.read(a.node(), Position::ZERO).unwrap().remove(0);
        let e = rec.entry.name().clone();
        let notes = NamedNode::new(format!("{}/annotations", e.as_str())).unwrap();
        let g = &graphs_of(&store.dataset())[notes.as_str()];
        let took = NamedNode::new(vocab::TOOK_TRANSITION).unwrap();
        let transition = NamedNode::new(format!("{}#s.a:t0", v.as_str())).unwrap();
        let stmt = Triple::new(a.node().clone(), took.clone(), transition.clone());
        assert!(g.contains(&stmt), "the statement is asserted");
        assert!(
            rec.entry
                .triples()
                .contains(&Triple::new(e.clone(), took.clone(), transition.clone())),
            "the entry records the transition without triple terms"
        );
        let caused = NamedNode::new(vocab::CAUSED_BY).unwrap();
        let reifier = g
            .subjects_for_predicate_object(&caused, &e)
            .next()
            .expect("a reifier names the entry as cause")
            .into_owned();
        match rdf {
            RdfCapability::Rdf12 => {
                let reifies = NamedNode::new(vertep_store::reification::RDF_REIFIES).unwrap();
                let t = g.object_for_subject_predicate(&reifier, &reifies).unwrap();
                assert_eq!(t, TermRef::Triple(&stmt));
            }
            RdfCapability::Rdf11 => {
                let subject = g
                    .object_for_subject_predicate(
                        &reifier,
                        vertep_store::oxrdf::vocab::rdf::SUBJECT,
                    )
                    .unwrap();
                assert_eq!(subject, TermRef::NamedNode(a.node().as_ref()));
                assert!(g.contains(&Triple::new(
                    reifier.clone(),
                    vertep_store::oxrdf::vocab::rdf::TYPE,
                    vertep_store::oxrdf::vocab::rdf::STATEMENT
                )));
            }
        }
    }
}

fn iris_of(store: &MemoryStore) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for q in store.dataset().iter() {
        if let NamedOrBlankNodeRef::NamedNode(n) = q.subject {
            out.insert(n.as_str().to_owned());
        }
        if let TermRef::NamedNode(n) = q.object {
            out.insert(n.as_str().to_owned());
        }
        if let GraphNameRef::NamedNode(n) = q.graph_name {
            out.insert(n.as_str().to_owned());
        }
    }
    out
}

// @tg: verifies:: [[openspec:runtime-ontology#Stable Runtime IRIs#Re-projection yields the same IRIs]] {level: integration}
#[test]
fn re_projection_yields_the_same_iris() {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let mut node = node_on(store.clone(), &clock, NodeConfig::default());
    exercise(&mut node, &clock);
    let first = MemoryStore::new();
    let second = MemoryStore::new();
    Projector::new(store.as_ref(), &first)
        .project_all()
        .unwrap();
    Projector::new(store.as_ref(), &second)
        .project_all()
        .unwrap();
    let (x, y) = (iris_of(&first), iris_of(&second));
    assert!(x.iter().any(|i| i.contains("/entry/")));
    assert!(x.iter().any(|i| i.contains("/timer/")));
    assert_eq!(x, y);
    // and replay derives the same entry IRIs: positions, not counters
    let a = va("busy", "a");
    let j = node.journal(&a).unwrap();
    for e in &j {
        assert_eq!(e.iri(), a.entry_iri(e.position));
    }
}

// @tg: verifies:: [[openspec:runtime-ontology#Named Graph Layout#Scope a query to one actor]] {level: integration}
#[test]
fn scope_a_query_to_one_actor() {
    let (mut node, _clock, store) = fresh();
    node.register_type(
        &ty("t"),
        scxml(
            "a",
            r##"<state id="a"><transition event="go" target="b"/></state><state id="b"/>"##,
        ),
    )
    .unwrap();
    let a = va("t", "a");
    let b = va("t", "b");
    node.send(&a, ev("go")).unwrap();
    node.send(&b, ev("noop")).unwrap();
    node.run_until_idle();
    let q = format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{a}> {{ ?s ?p ?o }} }}");
    let sol = store.query(&q).unwrap().into_solutions().unwrap();
    assert!(!sol.is_empty());
    for i in 0..sol.len() {
        assert_eq!(sol.get(i, "s").unwrap().to_string(), format!("<{a}>"));
        assert!(!sol.get(i, "o").unwrap().to_string().contains(b.as_str()));
    }
    let in_state: Vec<String> = sol
        .values("o")
        .iter()
        .map(ToString::to_string)
        .filter(|o| o.contains("#s."))
        .collect();
    assert_eq!(in_state.len(), 1);
    assert!(in_state[0].ends_with("#s.b>"), "exactly a's current state");
    // each entry is its own graph
    let e = a.entry_iri(1);
    let q = format!(
        "ASK {{ GRAPH {e} {{ {e} <{p}> ?m }} }}",
        p = vocab::MESSAGE_ID
    );
    assert!(store.query(&q).unwrap().into_boolean().unwrap());
}
