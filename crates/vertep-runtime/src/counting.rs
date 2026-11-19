//! [`CountingStorage`]: any [`Storage`] behind the counting wrapper of the store test kit.
//!
//! Operation budgets in [[openspec:runtime-performance#Operation Budgets]] are asserted against
//! these counts ("a turn commits once"), so they hold on any machine. Built with the `testkit`
//! feature.

use std::fmt;
use std::sync::Arc;

use oxrdf::{NamedNode, Quad};
use vertep_store::counting::{Counters, Counting};
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphCapabilities, GraphStore, JournalCapabilities,
    JournalRecord, JournalStore, Position, QueryResults, StoreError,
};

use crate::storage::Storage;

/// The journal store of `S`, owned through an `Arc`.
struct JournalOf<S>(Arc<S>);

impl<S: Storage> JournalStore for JournalOf<S> {
    fn capabilities(&self) -> JournalCapabilities {
        self.0.journal().capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.0.journal().append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.0.journal().read(actor, from)
    }
    fn read_range(
        &self,
        actor: &NamedNode,
        from: Position,
        limit: usize,
    ) -> Result<Vec<JournalRecord>, StoreError> {
        self.0.journal().read_range(actor, from, limit)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.0.journal().head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.0.journal().actors()
    }
}

/// The `index`-th graph store of `S`.
struct GraphOf<S> {
    storage: Arc<S>,
    index: usize,
}

impl<S: Storage> GraphOf<S> {
    fn with<T>(&self, f: impl FnOnce(&dyn GraphStore) -> T) -> T {
        f(self.storage.graphs()[self.index])
    }
}

impl<S: Storage> GraphStore for GraphOf<S> {
    fn capabilities(&self) -> GraphCapabilities {
        self.with(|g| g.capabilities())
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.with(|g| g.insert_quads(quads))
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.with(|g| g.remove_quads(quads))
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.with(|g| g.clear_graph(graph))
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        self.with(|g| g.query(sparql))
    }
}

/// A [`Storage`] that forwards to `S` and counts every journal and graph call, per call site,
/// in shared [`Counters`].
///
/// ```
/// use std::sync::Arc;
/// use vertep_runtime::counting::CountingStorage;
/// use vertep_runtime::Storage;
/// use vertep_store::MemoryStore;
///
/// let storage = CountingStorage::new(Arc::new(MemoryStore::new()));
/// let counters = storage.counters().clone();
/// {
///     let _s = counters.scope("probe");
///     storage.journal().head(&vertep_store::oxrdf::NamedNode::new("urn:x:a").unwrap()).unwrap();
/// }
/// assert_eq!(counters.get("probe").heads, 1);
/// ```
// @tg: implements:: [[openspec:runtime-performance#Operation Budgets]] {confidence: partial}
pub struct CountingStorage<S> {
    inner: Arc<S>,
    counters: Arc<Counters>,
    journal: Counting<JournalOf<S>>,
    graphs: Vec<Counting<GraphOf<S>>>,
}

impl<S> fmt::Debug for CountingStorage<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CountingStorage")
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

impl<S: Storage> CountingStorage<S> {
    /// Wraps `inner` with fresh counters.
    pub fn new(inner: Arc<S>) -> Self {
        Self::with_counters(inner, Arc::new(Counters::default()))
    }

    /// Wraps `inner`, counting in `counters`.
    pub fn with_counters(inner: Arc<S>, counters: Arc<Counters>) -> Self {
        let journal = Counting::new(JournalOf(inner.clone()), counters.clone());
        let graphs = (0..inner.graphs().len())
            .map(|index| {
                Counting::new(
                    GraphOf {
                        storage: inner.clone(),
                        index,
                    },
                    counters.clone(),
                )
            })
            .collect();
        CountingStorage {
            inner,
            counters,
            journal,
            graphs,
        }
    }

    /// The counters.
    pub fn counters(&self) -> &Arc<Counters> {
        &self.counters
    }

    /// The wrapped storage.
    pub fn inner(&self) -> &Arc<S> {
        &self.inner
    }
}

impl<S: Storage> Storage for CountingStorage<S> {
    fn journal(&self) -> &dyn JournalStore {
        &self.journal
    }

    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore {
        let target: *const dyn GraphStore = self.inner.graph(actor);
        let index = self
            .inner
            .graphs()
            .iter()
            .position(|g| std::ptr::addr_eq(*g as *const dyn GraphStore, target))
            .unwrap_or(0);
        &self.graphs[index]
    }

    fn graphs(&self) -> Vec<&dyn GraphStore> {
        self.graphs.iter().map(|g| g as &dyn GraphStore).collect()
    }
}
