//! A shard in a temporary directory, removed when dropped.

#![allow(dead_code)]

use oxrdf::{NamedNode, Quad};
use tempfile::TempDir;
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphCapabilities, GraphStore, JournalCapabilities,
    JournalRecord, JournalStore, Position, QueryResults, StoreError,
};
use vertep_tiramemsu::TiramemsuShard;

/// A shard file in its own temporary directory. The shard is dropped before the directory.
pub struct TempShard {
    pub shard: TiramemsuShard,
    pub dir: TempDir,
}

impl TempShard {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let shard = TiramemsuShard::open(dir.path().join("shard.db")).expect("open shard");
        TempShard { shard, dir }
    }
}

impl JournalStore for TempShard {
    fn capabilities(&self) -> JournalCapabilities {
        JournalStore::capabilities(&self.shard)
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.shard.append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.shard.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.shard.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.shard.actors()
    }
}

impl GraphStore for TempShard {
    fn capabilities(&self) -> GraphCapabilities {
        GraphStore::capabilities(&self.shard)
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.shard.insert_quads(quads)
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        self.shard.remove_quads(quads)
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.shard.clear_graph(graph)
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        self.shard.query(sparql)
    }
}
