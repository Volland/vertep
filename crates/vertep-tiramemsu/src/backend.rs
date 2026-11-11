//! Several shards, one file each, with actors placed by a hash of their address.

use std::path::{Path, PathBuf};

use oxrdf::NamedNode;
use vertep_store::{
    AppendError, AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, Position,
    StoreError,
};

use crate::shard::{CAPABILITIES, TiramemsuShard};

/// The shard of an actor: the FNV-1a 64-bit hash of the UTF-8 bytes of its address IRI,
/// modulo the shard count. The function is fixed: changing it moves actors between files.
pub fn shard_of(actor: &str, shard_count: usize) -> usize {
    assert!(shard_count > 0, "at least one shard");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in actor.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h % shard_count as u64) as usize
}

/// The file name of shard `index` of `count`.
pub fn shard_file_name(index: usize, count: usize) -> String {
    format!("shard-{index:04}-of-{count:04}.db")
}

/// The default backend of a single node: a directory of tiramemsu files, one per shard, each
/// with its own single writer, and actors placed by [`shard_of`] their address IRI.
///
/// Appends of actors in different shards run on different writers and never wait for each
/// other. The journal of an actor lives entirely in its shard's file, with its current-state
/// graph, so [`TiramemsuBackend::shard_for`] is also the graph store for that actor; queries
/// across shards go to an external graph store fed by the projector.
// @tg: implements:: [[openspec:storage#One File Per Shard]]
#[derive(Debug)]
pub struct TiramemsuBackend {
    dir: PathBuf,
    shards: Vec<TiramemsuShard>,
}

impl TiramemsuBackend {
    /// Opens (or creates) `shard_count` shard files in `dir`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for a zero shard count, or when `dir` already holds shard files
    /// of another shard count (actors would be looked up in the wrong files); the errors of
    /// [`TiramemsuShard::open`].
    pub fn open(dir: impl AsRef<Path>, shard_count: usize) -> Result<Self, StoreError> {
        if shard_count == 0 {
            return Err(StoreError::invalid("a backend has at least one shard"));
        }
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)
            .map_err(|e| StoreError::Backend(format!("{}: {e}", dir.display())))?;
        let suffix = format!("-of-{shard_count:04}.db");
        for f in std::fs::read_dir(&dir).map_err(StoreError::backend)? {
            let name = f.map_err(StoreError::backend)?.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("shard-") && name.ends_with(".db") && !name.ends_with(&suffix) {
                return Err(StoreError::Invalid(format!(
                    "{} holds shard files of another shard count ({name})",
                    dir.display()
                )));
            }
        }
        let shards = (0..shard_count)
            .map(|i| TiramemsuShard::open(dir.join(shard_file_name(i, shard_count))))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TiramemsuBackend { dir, shards })
    }

    /// The directory holding the shard files.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The number of shards.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// Every shard, by index.
    pub fn shards(&self) -> &[TiramemsuShard] {
        &self.shards
    }

    /// The shard index of `actor`.
    pub fn shard_index(&self, actor: &NamedNode) -> usize {
        shard_of(actor.as_str(), self.shards.len())
    }

    /// The shard holding `actor`'s journal and current-state graph.
    pub fn shard_for(&self, actor: &NamedNode) -> &TiramemsuShard {
        &self.shards[self.shard_index(actor)]
    }
}

impl JournalStore for TiramemsuBackend {
    fn capabilities(&self) -> JournalCapabilities {
        CAPABILITIES
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        // entry IRIs are unique per file; the runtime derives them from the actor address, so
        // an entry never collides with one in another shard
        self.shard_for(actor).append(actor, expected, entries, meta)
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.shard_for(actor).read(actor, from)
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.shard_for(actor).head(actor)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        let mut all = Vec::new();
        for s in &self.shards {
            all.extend(s.actors()?);
        }
        all.sort();
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_of_is_fixed_and_spreads() {
        // fixed values: changing the function moves actors between files
        assert_eq!(shard_of("", 8), (0xcbf2_9ce4_8422_2325_u64 % 8) as usize);
        assert_eq!(shard_of("urn:a", 1), 0);
        let mut counts = [0usize; 8];
        for i in 0..800 {
            counts[shard_of(&format!("urn:vertep:actor:order/{i}"), 8)] += 1;
        }
        assert!(counts.iter().all(|&c| c > 50), "{counts:?}");
    }
}
