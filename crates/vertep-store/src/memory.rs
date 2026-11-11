//! The in-memory reference implementation of both contracts.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use oxrdf::{Dataset, GraphName, NamedNode, Quad, Term};
use spareval::QueryEvaluator;
use spargebra::SparqlParser;

use crate::error::{AppendError, StoreError};
use crate::graph::{GraphCapabilities, GraphStore, QueryResults, RdfCapability};
use crate::journal::{
    AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, Position, prepare_append,
};
use crate::sparql::collect_results;

#[derive(Default)]
struct Inner {
    /// The graph store contents: entry graphs, current-state graphs and anything inserted.
    dataset: Dataset,
    /// Stamped entries per actor, position i + 1 at index i.
    journals: HashMap<NamedNode, Vec<Entry>>,
    /// Every entry IRI ever appended.
    entry_names: HashSet<NamedNode>,
    /// The highest accepted fencing epoch per scope.
    epochs: HashMap<NamedNode, u64>,
}

/// The in-memory reference implementation of [`JournalStore`] and [`GraphStore`] on one
/// dataset, used by the test kit's self-test and by tests of code above the storage layer.
///
/// Like the tiramemsu backend, it holds the journal and the graphs in one store: an append
/// writes the entry graphs and the state delta into the dataset under one lock. By default it
/// enforces fencing, accepts state deltas and declares RDF 1.2; each can be switched off to
/// exercise the other side of a capability.
pub struct MemoryStore {
    inner: Mutex<Inner>,
    journal_caps: JournalCapabilities,
    rdf: RdfCapability,
}

impl Default for MemoryStore {
    fn default() -> Self {
        MemoryStore::new()
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("journal_caps", &self.journal_caps)
            .field("rdf", &self.rdf)
            .finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// An empty store with fencing, state deltas and RDF 1.2.
    pub fn new() -> Self {
        MemoryStore {
            inner: Mutex::new(Inner::default()),
            journal_caps: JournalCapabilities {
                fencing: true,
                state_in_append: true,
            },
            rdf: RdfCapability::Rdf12,
        }
    }

    /// This store with fencing enforced or not.
    pub fn with_fencing(mut self, on: bool) -> Self {
        self.journal_caps.fencing = on;
        self
    }

    /// This store with state deltas accepted or not.
    pub fn with_state_in_append(mut self, on: bool) -> Self {
        self.journal_caps.state_in_append = on;
        self
    }

    /// This store declaring `rdf`. With [`RdfCapability::Rdf11`] it refuses triple terms.
    pub fn with_rdf(mut self, rdf: RdfCapability) -> Self {
        self.rdf = rdf;
        self
    }

    /// A copy of every quad in the store.
    pub fn dataset(&self) -> Dataset {
        self.lock().dataset.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // a panic while holding the lock leaves the data consistent (every write is applied
        // after all checks), so a poisoned lock is still usable
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check_terms(&self, quads: &[Quad]) -> Result<(), StoreError> {
        if self.rdf == RdfCapability::Rdf11
            && quads.iter().any(|q| matches!(q.object, Term::Triple(_)))
        {
            return Err(StoreError::Unsupported(
                "triple terms on a store that declares RDF 1.1".into(),
            ));
        }
        Ok(())
    }
}

impl JournalStore for MemoryStore {
    fn capabilities(&self) -> JournalCapabilities {
        self.journal_caps
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        let stamped = prepare_append(actor, expected, entries, meta, self.journal_caps)?;
        let mut inner = self.lock();
        // fencing first: a stale node is rejected even when its position matches
        if let Some(token) = &meta.fencing {
            let highest = inner.epochs.get(&token.scope).copied().unwrap_or(0);
            if token.epoch < highest {
                return Err(AppendError::Fenced {
                    scope: token.scope.clone(),
                    epoch: token.epoch,
                    highest,
                });
            }
        }
        let actual = Position::new(inner.journals.get(actor).map_or(0, |j| j.len() as u64));
        if actual != expected {
            return Err(AppendError::Conflict { expected, actual });
        }
        if let Some(dup) = stamped
            .iter()
            .find(|e| inner.entry_names.contains(e.name()))
        {
            return Err(AppendError::Invalid(format!(
                "entry {} is already in the store",
                dup.name()
            )));
        }
        for e in &stamped {
            self.check_terms(&e.quads())?;
        }
        if let Some(delta) = &meta.state {
            let g = GraphName::NamedNode(actor.clone());
            let quads: Vec<Quad> = delta
                .assert
                .iter()
                .map(|t| t.clone().in_graph(g.clone()))
                .collect();
            self.check_terms(&quads)?;
        }
        // every check passed: apply all writes
        if let Some(token) = &meta.fencing {
            inner.epochs.insert(token.scope.clone(), token.epoch);
        }
        for e in &stamped {
            for q in e.quads() {
                inner.dataset.insert(&q);
            }
            inner.entry_names.insert(e.name().clone());
        }
        if let Some(delta) = &meta.state {
            let g = GraphName::NamedNode(actor.clone());
            for t in &delta.retract {
                inner.dataset.remove(&t.clone().in_graph(g.clone()));
            }
            for t in &delta.assert {
                inner.dataset.insert(&t.clone().in_graph(g.clone()));
            }
        }
        let journal = inner.journals.entry(actor.clone()).or_default();
        journal.extend(stamped);
        Ok(Position::new(journal.len() as u64))
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        let inner = self.lock();
        let Some(journal) = inner.journals.get(actor) else {
            return Ok(Vec::new());
        };
        let start = from.get().max(1);
        Ok(journal
            .iter()
            .enumerate()
            .skip((start - 1) as usize)
            .map(|(i, e)| JournalRecord {
                position: Position::new(i as u64 + 1),
                entry: e.clone(),
            })
            .collect())
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        Ok(Position::new(
            self.lock()
                .journals
                .get(actor)
                .map_or(0, |j| j.len() as u64),
        ))
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        let mut actors: Vec<NamedNode> = self.lock().journals.keys().cloned().collect();
        actors.sort();
        Ok(actors)
    }
}

impl GraphStore for MemoryStore {
    fn capabilities(&self) -> GraphCapabilities {
        GraphCapabilities::plain(self.rdf)
    }

    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.check_terms(quads)?;
        let mut inner = self.lock();
        for q in quads {
            inner.dataset.insert(q);
        }
        Ok(())
    }

    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let mut inner = self.lock();
        for q in quads {
            inner.dataset.remove(q);
        }
        Ok(())
    }

    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        let mut inner = self.lock();
        let doomed: Vec<Quad> = inner
            .dataset
            .quads_for_graph_name(graph)
            .map(|q| q.into_owned())
            .collect();
        for q in &doomed {
            inner.dataset.remove(q);
        }
        Ok(())
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        let query = SparqlParser::new()
            .parse_query(sparql)
            .map_err(|e| StoreError::invalid(e.to_string()))?;
        let inner = self.lock();
        let evaluator = QueryEvaluator::new();
        let results = evaluator
            .prepare(&query)
            .execute(&inner.dataset)
            .map_err(StoreError::backend)?;
        collect_results(results)
    }
}
