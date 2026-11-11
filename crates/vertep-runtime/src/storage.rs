//! Where a node keeps journals and graphs.
//!
//! A [`Storage`] pairs one [`JournalStore`] with the [`GraphStore`] that holds each actor's
//! current-state graph (on stores that commit state with appends, the same store), its
//! snapshots, outbox marks, dead letters and store-backed mailbox.

use oxrdf::{NamedNode, Quad, Term};
use vertep_store::{GraphStore, JournalStore, MemoryStore, Solutions, StoreError};

/// The journal store and graph stores of a node.
pub trait Storage: Send + Sync {
    /// The journal store.
    fn journal(&self) -> &dyn JournalStore;
    /// The graph store holding `actor`'s current state, snapshots and records.
    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore;
    /// Every graph store, for scans across actors (pending timers, mailboxes).
    fn graphs(&self) -> Vec<&dyn GraphStore>;
}

impl<S: Storage + ?Sized> Storage for std::sync::Arc<S> {
    fn journal(&self) -> &dyn JournalStore {
        (**self).journal()
    }
    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore {
        (**self).graph(actor)
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        (**self).graphs()
    }
}

impl Storage for MemoryStore {
    fn journal(&self) -> &dyn JournalStore {
        self
    }
    fn graph(&self, _actor: &NamedNode) -> &dyn GraphStore {
        self
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        vec![self]
    }
}

#[cfg(feature = "tiramemsu")]
impl Storage for vertep_tiramemsu::TiramemsuShard {
    fn journal(&self) -> &dyn JournalStore {
        self
    }
    fn graph(&self, _actor: &NamedNode) -> &dyn GraphStore {
        self
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        vec![self]
    }
}

#[cfg(feature = "tiramemsu")]
impl Storage for vertep_tiramemsu::TiramemsuBackend {
    fn journal(&self) -> &dyn JournalStore {
        self
    }
    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore {
        self.shard_for(actor)
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        self.shards().iter().map(|s| s as &dyn GraphStore).collect()
    }
}

/// A journal store and a separate graph store (for example a journal store without state in
/// appends and an Oxigraph graph store). Current state is then written after each append.
pub struct SplitStorage<J, G> {
    /// The journal store.
    pub journal: J,
    /// The graph store.
    pub graph: G,
}

impl<J, G> std::fmt::Debug for SplitStorage<J, G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitStorage").finish_non_exhaustive()
    }
}

impl<J: JournalStore, G: GraphStore> Storage for SplitStorage<J, G> {
    fn journal(&self) -> &dyn JournalStore {
        &self.journal
    }
    fn graph(&self, _actor: &NamedNode) -> &dyn GraphStore {
        &self.graph
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        vec![&self.graph]
    }
}

/// Runs a SELECT query.
pub(crate) fn select(g: &dyn GraphStore, sparql: &str) -> Result<Solutions, StoreError> {
    g.query(sparql)?.into_solutions()
}

/// The lexical form of a literal or the IRI of a named node.
pub(crate) fn text(t: &Term) -> Option<String> {
    match t {
        Term::Literal(l) => Some(l.value().to_owned()),
        Term::NamedNode(n) => Some(n.as_str().to_owned()),
        _ => None,
    }
}

/// Every quad of graph `graph` in `g`.
pub(crate) fn graph_quads(g: &dyn GraphStore, graph: &NamedNode) -> Result<Vec<Quad>, StoreError> {
    let q = format!("SELECT ?s ?p ?o WHERE {{ GRAPH {graph} {{ ?s ?p ?o }} }}");
    let sol = select(g, &q)?;
    let mut out = Vec::new();
    for i in 0..sol.len() {
        let (Some(s), Some(p), Some(o)) = (sol.get(i, "s"), sol.get(i, "p"), sol.get(i, "o"))
        else {
            continue;
        };
        let s = match s {
            Term::NamedNode(n) => oxrdf::NamedOrBlankNode::from(n.clone()),
            Term::BlankNode(b) => oxrdf::NamedOrBlankNode::from(b.clone()),
            _ => continue,
        };
        let Term::NamedNode(p) = p else { continue };
        out.push(Quad::new(s, p.clone(), o.clone(), graph.clone()));
    }
    Ok(out)
}
