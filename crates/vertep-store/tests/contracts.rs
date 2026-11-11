//! The storage contracts proven on the in-memory reference implementation, one test per spec
//! scenario, plus the whole kit through its macros.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};
use vertep_store::reification::{self, Annotation};
use vertep_store::testkit::{self, iri, sample_entry, vt};
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphCapabilities, GraphStore, JournalCapabilities,
    JournalRecord, JournalStore, MemoryStore, Position, Projector, QueryResults, RdfCapability,
    StoreError, vocab,
};

mod kit {
    use vertep_store::MemoryStore;
    vertep_store::journal_store_kit!(MemoryStore::new);
    vertep_store::graph_store_kit!(MemoryStore::new);
    vertep_store::combined_store_kit!(MemoryStore::new);
}

mod kit_without_fencing_rdf11 {
    use vertep_store::{MemoryStore, RdfCapability};
    fn open() -> MemoryStore {
        MemoryStore::new()
            .with_fencing(false)
            .with_state_in_append(false)
            .with_rdf(RdfCapability::Rdf11)
    }
    vertep_store::journal_store_kit!(open);
    vertep_store::graph_store_kit!(open);
    vertep_store::combined_store_kit!(open);
}

// @tg: verifies:: [[openspec:storage#Journal Store Contract#Atomic multi-entry append]] {level: unit}
#[test]
fn atomic_multi_entry_append() {
    testkit::atomic_multi_entry_append(&MemoryStore::new());
}

// @tg: verifies:: [[openspec:storage#Journal Store Contract#Failed append writes nothing]] {level: unit}
#[test]
fn failed_append_writes_nothing() {
    let store = MemoryStore::new();
    testkit::failed_append_writes_nothing(&store);
    // nor does a failed append leave its entry graph in the graph half of the store
    let actor = iri("failed2/actor");
    let late = sample_entry(&actor, "late");
    let err = store
        .append(
            &actor,
            Position::new(4),
            std::slice::from_ref(&late),
            &AppendMeta::none(),
        )
        .unwrap_err();
    assert!(err.is_conflict());
    let ask = format!("ASK {{ GRAPH {} {{ ?s ?p ?o }} }}", late.name());
    assert_eq!(store.query(&ask).unwrap(), QueryResults::Boolean(false));
}

// @tg: verifies:: [[openspec:storage#Journal Store Contract#Entries are RDF]] {level: unit}
#[test]
fn entries_are_rdf() {
    testkit::entries_are_rdf(&MemoryStore::new());
}

// @tg: verifies:: [[openspec:storage#Append Fencing Token#Unfenced append unchanged]] {level: unit}
#[test]
fn unfenced_append_unchanged() {
    testkit::unfenced_append_unchanged(&MemoryStore::new());
    testkit::unfenced_append_unchanged(&MemoryStore::new().with_fencing(false));
}

// @tg: verifies:: [[openspec:storage#Append Fencing Token#Stale epoch rejected]] {level: unit}
#[test]
fn stale_epoch_rejected() {
    testkit::stale_epoch_rejected(&MemoryStore::new());
}

// @tg: verifies:: [[openspec:storage#Append Fencing Token#Fencing support is declared]] {level: unit}
#[test]
fn fencing_support_is_declared() {
    let fencing = MemoryStore::new();
    let plain = MemoryStore::new().with_fencing(false);
    assert!(fencing.supports_fencing());
    assert!(!plain.supports_fencing());
    testkit::fencing_support_is_declared(&fencing);
    testkit::fencing_support_is_declared(&plain);
}

// @tg: verifies:: [[openspec:storage#Graph Store Contract#Query current state]] {level: unit}
#[test]
fn query_current_state() {
    let store = MemoryStore::new();
    testkit::query_current_state(&store);
    // and the state written by an append is queried the same way
    testkit::state_commits_with_entries(&store);
}

// @tg: verifies:: [[openspec:storage#Graph Store Contract#Capability is declared]] {level: unit}
#[test]
fn capability_is_declared() {
    let v12 = MemoryStore::new();
    let v11 = MemoryStore::new().with_rdf(RdfCapability::Rdf11);
    assert_eq!(v12.rdf_capability(), RdfCapability::Rdf12);
    assert_eq!(v11.rdf_capability(), RdfCapability::Rdf11);
    testkit::capability_is_declared(&v12);
    testkit::capability_is_declared(&v11);
    // a declared RDF 1.1 store refuses triple terms rather than mangling them
    let tt = Quad::new(
        iri("r"),
        NamedNode::new(reification::RDF_REIFIES).unwrap(),
        Term::Triple(Box::new(Triple::new(iri("s"), iri("p"), iri("o")))),
        iri("g"),
    );
    assert!(matches!(
        v11.insert_quads(&[tt]),
        Err(StoreError::Unsupported(_))
    ));
}

/// A backend that ignores the expected position: it appends at its head whatever the caller
/// expected. It must not be admitted.
struct NoCompareAndSet(MemoryStore);

impl JournalStore for NoCompareAndSet {
    fn capabilities(&self) -> JournalCapabilities {
        JournalStore::capabilities(&self.0)
    }
    fn append(
        &self,
        actor: &NamedNode,
        _expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        loop {
            let head = self.0.head(actor)?;
            match self.0.append(actor, head, entries, meta) {
                Err(AppendError::Conflict { .. }) => continue,
                other => return other,
            }
        }
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.0.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.0.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.0.actors()
    }
}

/// A graph store that claims RDF 1.2 but drops every triple term it is given.
struct FalseCapability(MemoryStore);

impl GraphStore for FalseCapability {
    fn capabilities(&self) -> GraphCapabilities {
        GraphCapabilities::plain(RdfCapability::Rdf12)
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let kept: Vec<Quad> = quads
            .iter()
            .filter(|q| !matches!(q.object, Term::Triple(_)))
            .cloned()
            .collect();
        self.0.insert_quads(&kept)
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.0.remove_quads(quads)
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.0.clear_graph(graph)
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        self.0.query(sparql)
    }
}

// @tg: verifies:: [[openspec:storage#Shared Backend Test Kit#Backend admitted by the kit]] {level: unit}
#[test]
fn backend_admitted_by_the_kit() {
    assert_eq!(testkit::admit_journal_store(MemoryStore::new), Ok(()));
    assert_eq!(testkit::admit_graph_store(MemoryStore::new), Ok(()));
    assert_eq!(
        testkit::admit_journal_store(|| MemoryStore::new().with_fencing(false)),
        Ok(())
    );

    let failures = testkit::admit_journal_store(|| NoCompareAndSet(MemoryStore::new()))
        .expect_err("a store without compare-and-set is refused");
    let failed: Vec<&str> = failures
        .iter()
        .map(|f| f.split(':').next().unwrap())
        .collect();
    assert!(
        failed.contains(&"concurrent_conflicting_appends"),
        "{failures:?}"
    );
    assert!(
        failed.contains(&"failed_append_writes_nothing"),
        "{failures:?}"
    );

    let failures = testkit::admit_graph_store(|| FalseCapability(MemoryStore::new()))
        .expect_err("a false capability declaration is refused");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("capability_is_declared"),
        "{failures:?}"
    );
}

/// A graph store that can be taken offline, recording the entry graphs written to it in order.
struct Flaky {
    inner: MemoryStore,
    down: AtomicBool,
    written: Mutex<Vec<NamedNode>>,
}

impl Flaky {
    fn new() -> Self {
        Flaky {
            inner: MemoryStore::new(),
            down: AtomicBool::new(false),
            written: Mutex::new(Vec::new()),
        }
    }
    fn check(&self) -> Result<(), StoreError> {
        if self.down.load(Ordering::SeqCst) {
            Err(StoreError::Unavailable("endpoint unreachable".into()))
        } else {
            Ok(())
        }
    }
}

impl GraphStore for Flaky {
    fn capabilities(&self) -> GraphCapabilities {
        GraphStore::capabilities(&self.inner)
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.check()?;
        if let Some(GraphName::NamedNode(g)) = quads.first().map(|q| &q.graph_name)
            && g.as_str().contains("/entry/")
        {
            self.written.lock().unwrap().push(g.clone());
        }
        self.inner.insert_quads(quads)
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.check()?;
        self.inner.remove_quads(quads)
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.check()?;
        self.inner.clear_graph(graph)
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        self.check()?;
        self.inner.query(sparql)
    }
}

fn append_n(journal: &MemoryStore, actor: &NamedNode, from: u64, n: u64) {
    for i in from..from + n {
        journal
            .append(
                actor,
                Position::new(i),
                &[sample_entry(actor, &format!("{}", i + 1))],
                &AppendMeta::none(),
            )
            .unwrap();
    }
}

// @tg: verifies:: [[openspec:storage#Eventual Projection To External Graph Stores#Projection catches up]] {level: unit}
#[test]
fn projection_catches_up() {
    let journal = MemoryStore::new();
    let target = Flaky::new();
    let actor = iri("proj/actor");
    let projector = Projector::new(&journal, &target);

    append_n(&journal, &actor, 0, 3);
    assert_eq!(
        projector.project_actor(&actor).unwrap().to,
        Position::new(3)
    );

    target.down.store(true, Ordering::SeqCst);
    append_n(&journal, &actor, 3, 50);
    let err = projector.project_actor(&actor).unwrap_err();
    assert!(err.is_transient(), "{err}");
    assert_eq!(
        journal.head(&actor).unwrap(),
        Position::new(53),
        "the journal is unaffected"
    );

    target.down.store(false, Ordering::SeqCst);
    target.written.lock().unwrap().clear();
    let report = projector.project_actor(&actor).unwrap();
    assert_eq!(report.from, Position::new(3));
    assert_eq!(report.to, Position::new(53));
    assert_eq!(report.entries, 50);
    let expected: Vec<NamedNode> = (4..=53)
        .map(|n| testkit::entry_iri(&actor, &n.to_string()))
        .collect();
    assert_eq!(
        *target.written.lock().unwrap(),
        expected,
        "written in journal order"
    );
    assert_eq!(
        projector.projected_position(&actor).unwrap(),
        Position::new(53)
    );
    // the position is one statement per actor
    let q = format!(
        "SELECT ?n WHERE {{ GRAPH <{}> {{ {actor} <{}> ?n }} }}",
        vocab::PROJECTION_GRAPH,
        vocab::PROJECTED_POSITION
    );
    let s = target.query(&q).unwrap().into_solutions().unwrap();
    assert_eq!(
        s.values("n"),
        vec![Term::from(Literal::new_typed_literal("53", xsd::INTEGER))]
    );
    // nothing more to do
    assert_eq!(projector.project_actor(&actor).unwrap().entries, 0);
}

// @tg: verifies:: [[openspec:storage#Eventual Projection To External Graph Stores#Re-projection is idempotent]] {level: unit}
#[test]
fn reprojection_is_idempotent() {
    // entries without blank nodes, as the runtime writes them (stable IRIs)
    let journal = MemoryStore::new();
    let actor = iri("reproj/actor");
    for n in 1..=5u64 {
        let e = testkit::entry_iri(&actor, &n.to_string());
        let entry = Entry::new(
            e.clone(),
            [
                Triple::new(e.clone(), vt("messageId"), Literal::from(format!("m{n}"))),
                Triple::new(e, vt("chartVersion"), iri("chart/v1")),
            ],
        );
        let state = vertep_store::StateDelta {
            retract: vec![],
            assert: vec![Triple::new(
                actor.clone(),
                vt("seen"),
                Literal::from(n as i64),
            )],
        };
        journal
            .append(
                &actor,
                Position::new(n - 1),
                &[entry],
                &AppendMeta::none().with_state(state),
            )
            .unwrap();
    }
    let target = MemoryStore::new();
    let projector = Projector::new(&journal, &target).with_state_source(&journal);
    projector.project_all().unwrap();
    let once = target.dataset();
    assert!(
        once.len() > 5 * 5,
        "entries, state and position were written"
    );

    // write every entry again, as a projector that lost its position would
    for rec in journal.read(&actor, Position::ZERO).unwrap() {
        target.insert_quads(&rec.entry.quads()).unwrap();
    }
    assert_eq!(
        target.dataset(),
        once,
        "re-projected entries change nothing"
    );

    // and a full re-run from position 0 into a store holding the projection changes nothing
    let fresh_position = MemoryStore::new();
    for q in once.iter() {
        if q.predicate.as_str() != vocab::PROJECTED_POSITION {
            fresh_position.insert_quads(&[q.into_owned()]).unwrap();
        }
    }
    Projector::new(&journal, &fresh_position)
        .with_state_source(&journal)
        .project_all()
        .unwrap();
    assert_eq!(fresh_position.dataset(), once);
}

// @tg: verifies:: [[openspec:storage#RDF 1.1 Reification Fallback#Annotation on an RDF 1.1 store]] {level: unit}
#[test]
fn annotation_on_an_rdf11_store() {
    let store = MemoryStore::new().with_rdf(RdfCapability::Rdf11);
    let g = iri("fallback/actor");
    let taken = Triple::new(
        iri("fallback/t1"),
        vt("transitionTaken"),
        Literal::from(true),
    );
    let confidence = Literal::new_typed_literal("0.9", xsd::DECIMAL);
    let a = Annotation::new(g.clone(), taken.clone())
        .with_reifier(iri("fallback/r"))
        .with(vt("confidence"), confidence.clone());
    reification::write_annotations(&store, std::slice::from_ref(&a)).unwrap();

    // it wrote an rdf:Statement with subject, predicate and object plus the confidence
    let ds = store.dataset();
    let r = iri("fallback/r");
    let has = |p: oxrdf::NamedNodeRef<'_>, o: Term| {
        ds.contains(&Quad::new(r.clone(), p.into_owned(), o, g.clone()))
    };
    assert!(has(rdf::TYPE, rdf::STATEMENT.into_owned().into()));
    assert!(has(rdf::SUBJECT, iri("fallback/t1").into()));
    assert!(has(rdf::PREDICATE, vt("transitionTaken").into()));
    assert!(has(rdf::OBJECT, Literal::from(true).into()));
    assert!(has(vt("confidence").as_ref(), confidence.clone().into()));
    assert!(
        !ds.iter()
            .any(|q| matches!(q.object, oxrdf::TermRef::Triple(_))),
        "no triple terms on an RDF 1.1 store"
    );

    // a later read returns the same annotation, through SPARQL and from the raw quads
    let back = reification::read_annotations(&store, &GraphName::NamedNode(g), &taken).unwrap();
    assert_eq!(back, vec![a.clone()]);
    let decoded = reification::decode(&ds);
    assert_eq!(decoded, vec![a.clone()]);

    // the same annotation on an RDF 1.2 store uses a reifier, and also reads back
    let v12 = MemoryStore::new();
    reification::write_annotations(&v12, std::slice::from_ref(&a)).unwrap();
    assert!(
        v12.dataset()
            .iter()
            .any(|q| q.predicate.as_str() == reification::RDF_REIFIES)
    );
    let back12 = reification::read_annotations(&v12, &a.graph, &taken).unwrap();
    assert_eq!(back12, vec![a]);
}
