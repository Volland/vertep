//! The SPARQL graph store adapter: embedded Oxigraph and a remote endpoint (served locally by
//! Oxigraph behind a test HTTP server) both pass the graph half of the kit, and a tiramemsu
//! journal projects into them.

mod common;

use std::collections::BTreeSet;

use common::TestEndpoint;
use oxrdf::vocab::rdf;
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};
use vertep_oxigraph::{OxigraphStore, SparqlEndpoint};
use vertep_store::testkit::{iri, sample_entry, vt};
use vertep_store::{
    AppendMeta, GraphCapabilities, GraphStore, JournalStore, Position, Projector, QueryResults,
    RdfCapability, StateDelta, StoreError,
};
use vertep_tiramemsu::TiramemsuBackend;

/// A remote endpoint together with the local server answering it.
struct Remote {
    endpoint: SparqlEndpoint,
    server: TestEndpoint,
}

impl Remote {
    fn start(rdf: RdfCapability) -> Self {
        let server = TestEndpoint::start();
        let endpoint = SparqlEndpoint::new(server.query_url())
            .with_update_url(server.update_url())
            .with_rdf(rdf);
        Remote { endpoint, server }
    }
}

impl GraphStore for Remote {
    fn capabilities(&self) -> GraphCapabilities {
        self.endpoint.capabilities()
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.endpoint.insert_quads(quads)
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.endpoint.remove_quads(quads)
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.endpoint.clear_graph(graph)
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        self.endpoint.query(sparql)
    }
}

mod embedded_kit {
    vertep_store::graph_store_kit!(|| vertep_oxigraph::OxigraphStore::new().unwrap());
}

mod remote_rdf11_kit {
    vertep_store::graph_store_kit!(|| super::Remote::start(vertep_store::RdfCapability::Rdf11));
}

mod remote_rdf12_kit {
    vertep_store::graph_store_kit!(|| super::Remote::start(vertep_store::RdfCapability::Rdf12));
}

#[test]
fn admitted_by_the_graph_kit() {
    assert_eq!(
        vertep_store::testkit::admit_graph_store(|| OxigraphStore::new().unwrap()),
        Ok(())
    );
    assert_eq!(
        vertep_store::testkit::admit_graph_store(|| Remote::start(RdfCapability::Rdf11)),
        Ok(())
    );
}

fn active(actor: &NamedNode, state: &str) -> Triple {
    Triple::new(
        actor.clone(),
        vt("activeState"),
        iri(&format!("chart#{state}")),
    )
}

fn dead_letter(actor: &NamedNode, n: u32) -> Vec<Triple> {
    let dl = NamedNode::new(format!("{}/dead-letter/{n}", actor.as_str())).unwrap();
    vec![
        Triple::new(dl.clone(), rdf::TYPE, vt("DeadLetter")),
        Triple::new(dl.clone(), vt("receiver"), actor.clone()),
        Triple::new(dl, vt("eventName"), Literal::from("unexpected.event")),
    ]
}

fn states(store: &dyn GraphStore, actor: &NamedNode) -> Vec<Term> {
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

fn dead_letters(store: &dyn GraphStore) -> BTreeSet<String> {
    let q = format!(
        "SELECT ?dl WHERE {{ GRAPH ?g {{ ?dl a {} ; {} ?g }} }}",
        vt("DeadLetter"),
        vt("receiver")
    );
    store
        .query(&q)
        .unwrap()
        .into_solutions()
        .unwrap()
        .values("dl")
        .into_iter()
        .map(|t| t.to_string())
        .collect()
}

/// Runs a little order workflow on three actors through the tiramemsu backend.
fn run_workflow(backend: &TiramemsuBackend, actors: &[NamedNode], from: u64, steps: u64) {
    let names = ["Created", "Paid", "Packed", "Shipped", "Delivered"];
    for (i, actor) in actors.iter().enumerate() {
        for n in from..from + steps {
            let exit = (n > 0).then(|| active(actor, names[(n as usize - 1) % names.len()]));
            let mut assert = vec![active(actor, names[n as usize % names.len()])];
            if n % 2 == 1 {
                assert.extend(dead_letter(actor, n as u32));
            }
            let delta = StateDelta {
                retract: exit.into_iter().collect(),
                assert,
            };
            backend
                .append(
                    actor,
                    Position::new(n),
                    &[sample_entry(actor, &format!("{i}-{}", n + 1))],
                    &AppendMeta::none().with_state(delta),
                )
                .unwrap();
        }
    }
}

fn project(backend: &TiramemsuBackend, target: &dyn GraphStore) -> Result<usize, StoreError> {
    let mut written = 0;
    for shard in backend.shards() {
        for report in Projector::new(shard, target)
            .with_state_source(shard)
            .project_all()?
        {
            written += report.entries;
        }
    }
    Ok(written)
}

// @tg: verifies:: [[openspec:storage#SPARQL Graph Store Adapter#Journal elsewhere, graph in Oxigraph]] {level: integration}
#[test]
fn journal_elsewhere_graph_in_oxigraph() {
    let dir = tempfile::tempdir().unwrap();
    let backend = TiramemsuBackend::open(dir.path(), 2).unwrap();
    let oxigraph = OxigraphStore::new().unwrap();
    let actors: Vec<NamedNode> = (0..3).map(|i| iri(&format!("orders/{i}"))).collect();
    run_workflow(&backend, &actors, 0, 4);

    assert_eq!(project(&backend, &oxigraph).unwrap(), 12);
    let mut expected_dead = BTreeSet::new();
    for actor in &actors {
        // the current-state graph in Oxigraph equals the one in tiramemsu
        let shard = backend.shard_for(actor);
        assert_eq!(states(&oxigraph, actor), states(shard, actor));
        assert_eq!(
            states(&oxigraph, actor),
            vec![Term::from(iri("chart#Shipped"))]
        );
        expected_dead.extend(dead_letters(shard));
    }
    // dead letters are queryable in Oxigraph across all actors and shards
    assert_eq!(expected_dead.len(), 6);
    assert_eq!(dead_letters(&oxigraph), expected_dead);
    // entries are there too, in their named graphs
    let entry = vertep_store::testkit::entry_iri(&actors[0], "0-1");
    let ask = format!("ASK {{ GRAPH {entry} {{ {entry} a ?t }} }}");
    assert_eq!(oxigraph.query(&ask).unwrap(), QueryResults::Boolean(true));

    // journal correctness depends only on tiramemsu: compare-and-set is decided there, and
    // writing anything into Oxigraph changes nothing about it
    oxigraph
        .insert_quads(&[Quad::new(
            actors[0].clone(),
            NamedNode::new(vertep_store::vocab::HEAD).unwrap(),
            Literal::from(99),
            GraphName::DefaultGraph,
        )])
        .unwrap();
    let stale = backend.append(
        &actors[0],
        Position::new(2),
        &[sample_entry(&actors[0], "stale")],
        &AppendMeta::none(),
    );
    assert!(stale.unwrap_err().is_conflict());
    assert_eq!(backend.head(&actors[0]).unwrap(), Position::new(4));
}

// @tg: verifies:: [[openspec:storage#SPARQL Graph Store Adapter#Remote endpoint]] {level: integration}
#[test]
fn remote_endpoint() {
    let remote = Remote::start(RdfCapability::Rdf11);
    let g = iri("remote/actor");
    let quads = vec![
        Quad::new(g.clone(), vt("activeState"), iri("chart#Idle"), g.clone()),
        Quad::new(
            g.clone(),
            vt("note"),
            Literal::from("hello \"world\""),
            g.clone(),
        ),
    ];
    remote.endpoint.insert_quads(&quads).unwrap();
    let writes = remote.server.take_requests();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].method, "POST");
    assert_eq!(writes[0].path, "/update");
    assert_eq!(writes[0].content_type, "application/sparql-update");
    assert!(
        writes[0].body.starts_with("INSERT DATA {"),
        "{}",
        writes[0].body
    );
    for q in &quads {
        assert!(
            remote.server.store.contains(q).unwrap(),
            "written by SPARQL Update"
        );
    }

    let query = format!(
        "SELECT ?s WHERE {{ GRAPH {g} {{ {g} {} ?s }} }}",
        vt("activeState")
    );
    assert_eq!(
        states(&remote.endpoint, &g),
        vec![Term::from(iri("chart#Idle"))]
    );
    let reads = remote.server.take_requests();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].path, "/query");
    assert_eq!(reads[0].content_type, "application/sparql-query");
    assert!(reads[0].body.starts_with("SELECT ?s"));
    let s = remote
        .endpoint
        .query(&query)
        .unwrap()
        .into_solutions()
        .unwrap();
    assert_eq!(s.values("s"), vec![Term::from(iri("chart#Idle"))]);
    remote.server.take_requests();

    remote.endpoint.remove_quads(&quads[..1]).unwrap();
    remote.endpoint.clear_graph(&g).unwrap();
    let bodies: Vec<String> = remote
        .server
        .take_requests()
        .into_iter()
        .map(|r| r.body)
        .collect();
    assert!(bodies[0].starts_with("DELETE DATA {"));
    assert_eq!(bodies[1], format!("CLEAR SILENT GRAPH {g}"));
    assert!(remote.server.store.is_empty().unwrap());

    // endpoint errors map to store errors
    assert!(matches!(
        remote.endpoint.query("SELECT ?x WHERE {"),
        Err(StoreError::Invalid(_))
    ));
    remote.server.set_down(true);
    let down = remote.endpoint.query(&query).unwrap_err();
    assert!(down.is_transient(), "{down}");
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/sparql", closed.local_addr().unwrap());
    drop(closed);
    let unreachable = SparqlEndpoint::new(url).query("ASK {}").unwrap_err();
    assert!(unreachable.is_transient(), "{unreachable}");
}

/// Task 5.2: tiramemsu journal plus an Oxigraph graph store, with projection catching up after
/// the graph store was down.
#[test]
fn tiramemsu_journal_with_oxigraph_catch_up_after_outage() {
    let dir = tempfile::tempdir().unwrap();
    let backend = TiramemsuBackend::open(dir.path(), 4).unwrap();
    let remote = Remote::start(RdfCapability::Rdf11);
    let actors: Vec<NamedNode> = (0..3).map(|i| iri(&format!("outage/{i}"))).collect();

    run_workflow(&backend, &actors, 0, 2);
    assert_eq!(project(&backend, &remote).unwrap(), 6);

    remote.server.set_down(true);
    run_workflow(&backend, &actors, 2, 3); // the journal keeps accepting appends
    let err = project(&backend, &remote).unwrap_err();
    assert!(err.is_transient(), "{err}");
    // behind the endpoint, the graph store is stale but intact
    let behind = OxigraphStore::from_store(remote.server.store.clone());
    for a in &actors {
        assert_eq!(backend.head(a).unwrap(), Position::new(5));
        assert_eq!(states(&behind, a), vec![Term::from(iri("chart#Paid"))]);
    }

    remote.server.set_down(false);
    assert_eq!(
        project(&backend, &remote).unwrap(),
        9,
        "exactly the missed entries"
    );
    for a in &actors {
        assert_eq!(states(&remote, a), states(backend.shard_for(a), a));
        let shard = backend.shard_for(a);
        assert_eq!(
            Projector::new(shard, &remote)
                .projected_position(a)
                .unwrap(),
            Position::new(5)
        );
    }
    assert_eq!(project(&backend, &remote).unwrap(), 0, "caught up");
}
