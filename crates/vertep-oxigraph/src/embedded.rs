//! The graph store on an embedded Oxigraph store.

use oxigraph::sparql::SparqlEvaluator;
use oxigraph::store::Store;
use oxrdf::{NamedNode, Quad};
use vertep_store::sparql::collect_results;
use vertep_store::{GraphCapabilities, GraphStore, QueryResults, RdfCapability, StoreError};

/// A [`GraphStore`] on an embedded Oxigraph store, with RDF 1.2 triple terms.
///
/// Each insert or removal is one Oxigraph transaction; queries see committed data.
// @tg: implements:: [[openspec:storage#SPARQL Graph Store Adapter]]
#[derive(Clone)]
pub struct OxigraphStore {
    store: Store,
}

impl std::fmt::Debug for OxigraphStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OxigraphStore").finish_non_exhaustive()
    }
}

impl OxigraphStore {
    /// A new, empty in-memory Oxigraph store.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] when Oxigraph cannot create it.
    pub fn new() -> Result<Self, StoreError> {
        Ok(OxigraphStore {
            store: Store::new().map_err(StoreError::backend)?,
        })
    }

    /// The graph store on an already opened Oxigraph store.
    pub fn from_store(store: Store) -> Self {
        OxigraphStore { store }
    }

    /// The Oxigraph store, for its own API.
    pub fn store(&self) -> &Store {
        &self.store
    }
}

impl GraphStore for OxigraphStore {
    fn capabilities(&self) -> GraphCapabilities {
        GraphCapabilities::plain(RdfCapability::Rdf12)
    }

    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let mut t = self
            .store
            .start_transaction()
            .map_err(StoreError::backend)?;
        t.extend(quads);
        t.commit().map_err(StoreError::backend)
    }

    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let mut t = self
            .store
            .start_transaction()
            .map_err(StoreError::backend)?;
        for q in quads {
            t.remove(q);
        }
        t.commit().map_err(StoreError::backend)
    }

    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.store.clear_graph(graph).map_err(StoreError::backend)
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        let prepared = SparqlEvaluator::new()
            .parse_query(sparql)
            .map_err(|e| StoreError::invalid(e.to_string()))?;
        let results = prepared
            .on_store(&self.store)
            .execute()
            .map_err(StoreError::backend)?;
        collect_results(results)
    }
}
