//! A store wrapper that counts what its callers ask of the store.
//!
//! [`Counting`] wraps any [`JournalStore`], [`GraphStore`] or both, and records, per call site,
//! the commits, quads written and read, SPARQL queries and journal reads it forwards. Operation
//! budgets ("a turn commits once", "activation reads only after the snapshot") are asserted
//! against these counts, so they hold on any machine, unlike wall-clock budgets.
//!
//! A call site is a label the caller sets with [`Counters::scope`]; calls outside any scope are
//! counted under `""`. Labels nest: the innermost one counts.
//!
//! ```
//! use std::sync::Arc;
//! use vertep_store::counting::{Counters, Counting};
//! use vertep_store::{GraphStore, MemoryStore};
//!
//! let counters = Arc::new(Counters::default());
//! let store = Counting::new(MemoryStore::new(), counters.clone());
//! {
//!     let _turn = counters.scope("turn");
//!     store.insert_quads(&[]).unwrap();
//! }
//! assert_eq!(counters.get("turn").commits, 1);
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use oxrdf::{NamedNode, Quad};

use crate::error::{AppendError, StoreError};
use crate::graph::{GraphCapabilities, GraphStore, QueryResults};
use crate::journal::{
    AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, Position,
};

/// What was asked of a store at one call site.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Write transactions: each journal append and each graph insert, removal or clear.
    pub commits: u64,
    /// Journal appends (also counted in `commits`).
    pub appends: u64,
    /// Quads written: entry statements, state-delta statements, inserted and removed quads.
    pub quads_written: u64,
    /// Quads read: statements of journal entries read, and rows or triples a query returned.
    pub quads_read: u64,
    /// SPARQL queries.
    pub queries: u64,
    /// Journal reads (`read` and `read_range`).
    pub journal_reads: u64,
    /// Journal entries returned by those reads.
    pub entries_read: u64,
    /// Journal `head` calls.
    pub heads: u64,
    /// Journal `actors` calls.
    pub actor_scans: u64,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.commits += o.commits;
        self.appends += o.appends;
        self.quads_written += o.quads_written;
        self.quads_read += o.quads_read;
        self.queries += o.queries;
        self.journal_reads += o.journal_reads;
        self.entries_read += o.entries_read;
        self.heads += o.heads;
        self.actor_scans += o.actor_scans;
    }
}

impl fmt::Display for Counts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "commits={} appends={} quads_written={} quads_read={} queries={} journal_reads={} \
             entries_read={} heads={} actor_scans={}",
            self.commits,
            self.appends,
            self.quads_written,
            self.quads_read,
            self.queries,
            self.journal_reads,
            self.entries_read,
            self.heads,
            self.actor_scans
        )
    }
}

#[derive(Default)]
struct Inner {
    labels: Vec<String>,
    by_site: BTreeMap<String, Counts>,
}

/// The counts of one or more [`Counting`] wrappers, per call site.
#[derive(Default)]
pub struct Counters {
    inner: Mutex<Inner>,
}

impl fmt::Debug for Counters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.snapshot()).finish()
    }
}

/// Restores the previous call-site label when dropped (see [`Counters::scope`]).
#[must_use = "the label applies only while the guard lives"]
pub struct Scope<'a> {
    counters: &'a Counters,
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.counters.lock().labels.pop();
    }
}

impl Counters {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record(&self, f: impl FnOnce(&mut Counts)) {
        let mut inner = self.lock();
        let label = inner.labels.last().cloned().unwrap_or_default();
        f(inner.by_site.entry(label).or_default());
    }

    /// Counts calls made while the returned guard lives under `label`.
    pub fn scope(&self, label: &str) -> Scope<'_> {
        self.lock().labels.push(label.to_owned());
        Scope { counters: self }
    }

    /// The counts under `label` (zero when nothing was counted there).
    pub fn get(&self, label: &str) -> Counts {
        self.lock().by_site.get(label).copied().unwrap_or_default()
    }

    /// The counts of every call site added together.
    pub fn total(&self) -> Counts {
        let mut t = Counts::default();
        for c in self.lock().by_site.values() {
            t.add(c);
        }
        t
    }

    /// The counts per call site.
    pub fn snapshot(&self) -> BTreeMap<String, Counts> {
        self.lock().by_site.clone()
    }

    /// Forgets every count (labels in force stay).
    pub fn reset(&self) {
        self.lock().by_site.clear();
    }
}

/// A store that forwards every call to `inner` and counts it in shared [`Counters`].
pub struct Counting<S> {
    inner: S,
    counters: Arc<Counters>,
}

impl<S> fmt::Debug for Counting<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Counting")
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

impl<S> Counting<S> {
    /// Wraps `inner`, counting in `counters`.
    pub fn new(inner: S, counters: Arc<Counters>) -> Self {
        Counting { inner, counters }
    }

    /// The wrapped store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// The counters.
    pub fn counters(&self) -> &Arc<Counters> {
        &self.counters
    }
}

fn result_size(r: &QueryResults) -> u64 {
    match r {
        QueryResults::Solutions(s) => s.len() as u64,
        QueryResults::Boolean(_) => 0,
        QueryResults::Graph(t) => t.len() as u64,
    }
}

fn entries_size(records: &[JournalRecord]) -> u64 {
    records.iter().map(|r| r.entry.triples().len() as u64).sum()
}

// @tg: implements:: [[openspec:runtime-performance#Operation Budgets]] {confidence: partial}
impl<S: JournalStore> JournalStore for Counting<S> {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        let written = entries
            .iter()
            .map(|e| e.triples().len() as u64)
            .sum::<u64>()
            + meta
                .state
                .as_ref()
                .map_or(0, |d| (d.retract.len() + d.assert.len()) as u64);
        self.counters.record(|c| {
            c.commits += 1;
            c.appends += 1;
            c.quads_written += written;
        });
        self.inner.append(actor, expected, entries, meta)
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        let out = self.inner.read(actor, from)?;
        let (n, q) = (out.len() as u64, entries_size(&out));
        self.counters.record(|c| {
            c.journal_reads += 1;
            c.entries_read += n;
            c.quads_read += q;
        });
        Ok(out)
    }

    fn read_range(
        &self,
        actor: &NamedNode,
        from: Position,
        limit: usize,
    ) -> Result<Vec<JournalRecord>, StoreError> {
        let out = self.inner.read_range(actor, from, limit)?;
        let (n, q) = (out.len() as u64, entries_size(&out));
        self.counters.record(|c| {
            c.journal_reads += 1;
            c.entries_read += n;
            c.quads_read += q;
        });
        Ok(out)
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.counters.record(|c| c.heads += 1);
        self.inner.head(actor)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.counters.record(|c| c.actor_scans += 1);
        self.inner.actors()
    }
}

impl<S: GraphStore> GraphStore for Counting<S> {
    fn capabilities(&self) -> GraphCapabilities {
        self.inner.capabilities()
    }

    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let n = quads.len() as u64;
        self.counters.record(|c| {
            c.commits += 1;
            c.quads_written += n;
        });
        self.inner.insert_quads(quads)
    }

    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        let n = quads.len() as u64;
        self.counters.record(|c| {
            c.commits += 1;
            c.quads_written += n;
        });
        self.inner.remove_quads(quads)
    }

    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.counters.record(|c| c.commits += 1);
        self.inner.clear_graph(graph)
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        let out = self.inner.query(sparql)?;
        let n = result_size(&out);
        self.counters.record(|c| {
            c.queries += 1;
            c.quads_read += n;
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use crate::testkit::iri;
    use oxrdf::{Literal, Triple};

    fn entry(n: &str) -> Entry {
        Entry::new(
            iri(n),
            [Triple::new(iri("s"), iri("p"), Literal::from("o"))],
        )
    }

    #[test]
    fn counts_per_call_site_and_in_total() {
        let counters = Arc::new(Counters::default());
        let store = Counting::new(MemoryStore::new(), counters.clone());
        let a = iri("counting-a");
        {
            let _s = counters.scope("append");
            store
                .append(
                    &a,
                    Position::new(0),
                    &[entry("e1"), entry("e2")],
                    &AppendMeta::none(),
                )
                .unwrap();
        }
        {
            let _s = counters.scope("read");
            assert_eq!(store.read(&a, Position::new(0)).unwrap().len(), 2);
            store.head(&a).unwrap();
            store.query("ASK { ?s ?p ?o }").unwrap();
        }
        let w = counters.get("append");
        assert_eq!((w.commits, w.appends), (1, 1));
        assert!(w.quads_written >= 2);
        let r = counters.get("read");
        assert_eq!(
            (r.journal_reads, r.entries_read, r.heads, r.queries),
            (1, 2, 1, 1)
        );
        assert_eq!(r.commits, 0);
        assert_eq!(counters.total().commits, 1);
        counters.reset();
        assert_eq!(counters.total(), Counts::default());
    }

    #[test]
    fn nested_scopes_count_innermost_and_restore() {
        let counters = Arc::new(Counters::default());
        let store = Counting::new(MemoryStore::new(), counters.clone());
        let outer = counters.scope("outer");
        {
            let _inner = counters.scope("inner");
            store.clear_graph(&iri("g")).unwrap();
        }
        store.clear_graph(&iri("g")).unwrap();
        drop(outer);
        store.clear_graph(&iri("g")).unwrap();
        assert_eq!(counters.get("inner").commits, 1);
        assert_eq!(counters.get("outer").commits, 1);
        assert_eq!(counters.get("").commits, 1);
    }
}
