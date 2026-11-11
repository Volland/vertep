//! Where a system keeps its journals and graphs: the in-memory store (no journal file, no
//! SQLite), or tiramemsu on its WebAssembly host with OPFS or memory storage and the rollback
//! journal. The choice is explicit and never falls back.

use std::sync::Arc;

use serde_json::{Value as J, json};
use vertep_runtime::Storage;
use vertep_store::{GraphStore, MemoryStore, QueryResults};

use crate::error::{ApiError, Code, Res};

/// The storage kind an application chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    /// Volatile; with `journal: true` a tiramemsu file in memory that can be exported.
    Memory,
    /// Durable tiramemsu file in the Origin Private File System (dedicated worker only).
    Opfs,
    /// A tiramemsu file on the native file system (native tests and tools only).
    File,
}

impl StorageKind {
    /// Parses `memory`, `opfs` or `file`.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for anything else, including a missing value: there is no default.
    pub fn parse(name: Option<&str>) -> Res<Self> {
        match name {
            Some("memory") => Ok(StorageKind::Memory),
            Some("opfs") => Ok(StorageKind::Opfs),
            Some("file") => Ok(StorageKind::File),
            _ => Err(ApiError::arg(
                "storage must be chosen explicitly: \"opfs\" or \"memory\"",
            )),
        }
    }
}

/// The storage options of a system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreOptions {
    /// The kind.
    pub kind: StorageKind,
    /// The store path (the tiramemsu file name on its storage).
    pub path: String,
    /// Whether a `memory` store is a tiramemsu file (journaled, exportable). `opfs` and `file`
    /// stores always are.
    pub journal: bool,
}

impl StoreOptions {
    /// Whether these options need the tiramemsu journal (and so the SQLite module).
    pub fn journaled(&self) -> bool {
        self.journal || self.kind != StorageKind::Memory
    }
}

/// An open store.
#[derive(Clone)]
pub enum Store {
    /// The in-memory store of `vertep-store`.
    Memory(Arc<MemoryStore>),
    /// A tiramemsu file on the WebAssembly host.
    #[cfg(feature = "journal")]
    Journal(Arc<journal::JournalStore>),
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Store::Memory(_) => f.write_str("Store::Memory"),
            #[cfg(feature = "journal")]
            Store::Journal(j) => write!(f, "Store::Journal({})", j.path),
        }
    }
}

impl Store {
    /// Opens the store `opts` names. `import` is a journal file to start from (journaled stores
    /// only). OPFS must have been installed by the caller (it is asynchronous).
    ///
    /// # Errors
    ///
    /// `StorageUnavailable` when the build or the host lacks the storage, `StoreLocked` when the
    /// file is already open in this worker, and store errors.
    // @tg: implements:: [[openspec:wasm-bindings#Browser Journal Storage]]
    pub fn open(opts: &StoreOptions, import: Option<&[u8]>) -> Res<Store> {
        if !opts.journaled() {
            if import.is_some() {
                return Err(ApiError::arg(
                    "importing a journal needs a journaled store (journal: true)",
                ));
            }
            return Ok(Store::Memory(Arc::new(MemoryStore::new())));
        }
        #[cfg(feature = "journal")]
        {
            journal::JournalStore::open(opts, import).map(|j| Store::Journal(Arc::new(j)))
        }
        #[cfg(not(feature = "journal"))]
        {
            Err(ApiError::new(
                Code::StorageUnavailable,
                "this build has no journal storage; load the build with the tiramemsu journal",
            ))
        }
    }

    /// The store as the runtime's storage.
    pub fn storage(&self) -> Arc<dyn Storage> {
        match self {
            Store::Memory(m) => m.clone(),
            #[cfg(feature = "journal")]
            Store::Journal(j) => j.shard.clone(),
        }
    }

    /// Runs a SPARQL query on the graph store, now or as believed at wall-clock instant
    /// `as_of_ms` (journaled stores only: the in-memory store keeps no history).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a time-travel query on the in-memory store, and query errors.
    // @tg: implements:: [[openspec:wasm-bindings#Journal Inspection And Time Travel]] {confidence: partial, note: "graph queries; snapshots as of a position are in system"}
    pub fn query(&self, sparql: &str, as_of_ms: Option<i64>) -> Res<J> {
        let r = match self {
            Store::Memory(m) => {
                if as_of_ms.is_some() {
                    return Err(ApiError::arg(
                        "the in-memory store keeps no history: open a journaled store for queries as of a time",
                    ));
                }
                m.query(sparql)
            }
            #[cfg(feature = "journal")]
            Store::Journal(j) => match as_of_ms {
                Some(ms) => j.shard.view_as_of_time(ms).query(sparql),
                None => j.shard.view_now().query(sparql),
            },
        }
        .map_err(|e| ApiError::new(Code::Store, e.to_string()))?;
        Ok(results_json(&r))
    }

    /// The journal file's bytes (journaled stores only).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for the in-memory store, and host errors.
    pub fn export(&self) -> Res<Vec<u8>> {
        match self {
            Store::Memory(_) => Err(ApiError::arg(
                "the in-memory store has no journal file: open it with journal: true to export",
            )),
            #[cfg(feature = "journal")]
            Store::Journal(j) => j.export(),
        }
    }
}

/// Query results as JSON: `{variables, rows}` (each row an object of N-Triples terms),
/// `{boolean}` or `{triples}`.
pub fn results_json(r: &QueryResults) -> J {
    match r {
        QueryResults::Solutions(s) => {
            let rows: Vec<J> = s
                .rows
                .iter()
                .map(|row| {
                    let mut o = serde_json::Map::new();
                    for (v, t) in s.variables.iter().zip(row) {
                        if let Some(t) = t {
                            o.insert(v.clone(), J::String(t.to_string()));
                        }
                    }
                    J::Object(o)
                })
                .collect();
            json!({ "variables": s.variables, "rows": rows })
        }
        QueryResults::Boolean(b) => json!({ "boolean": b }),
        QueryResults::Graph(g) => {
            json!({ "triples": g.iter().map(ToString::to_string).collect::<Vec<_>>() })
        }
    }
}

#[cfg(feature = "journal")]
pub mod journal {
    //! The tiramemsu journal on its WebAssembly host (`tm-wasm`).

    use std::sync::Arc;

    use tm_wasm::{Journal, Storage, WasmClock, WasmHost};
    use vertep_tiramemsu::TiramemsuShard;
    use vertep_tiramemsu::tiramemsu::{Db, OpenOptions};

    use super::{StorageKind, StoreOptions};
    use crate::error::{ApiError, Code, Res};

    /// One tiramemsu file, its host and its name.
    pub struct JournalStore {
        /// The shard (journal and graph store).
        pub shard: Arc<TiramemsuShard>,
        /// The WebAssembly host the file lives on; `None` for a file opened with tiramemsu's
        /// native host.
        pub host: Option<WasmHost>,
        /// The file name.
        pub path: String,
    }

    impl JournalStore {
        /// Opens (creating, or importing `import` first) the file `opts` names, with the
        /// rollback journal.
        ///
        /// # Errors
        ///
        /// `StorageUnavailable`, `StoreLocked` and store errors.
        pub fn open(opts: &StoreOptions, import: Option<&[u8]>) -> Res<JournalStore> {
            let storage = match opts.kind {
                StorageKind::Memory => Storage::Memory,
                StorageKind::Opfs => {
                    if !cfg!(all(target_family = "wasm", target_os = "unknown")) {
                        return Err(ApiError::new(
                            Code::StorageUnavailable,
                            "OPFS exists only in a browser's dedicated worker",
                        ));
                    }
                    Storage::Opfs
                }
                StorageKind::File => {
                    if cfg!(all(target_family = "wasm", target_os = "unknown")) {
                        return Err(ApiError::new(
                            Code::StorageUnavailable,
                            "file storage exists only natively",
                        ));
                    }
                    Storage::File
                }
            };
            let host = WasmHost::new(storage, Journal::Rollback);
            if let Some(bytes) = import {
                host.import_file(&opts.path, bytes)
                    .map_err(|e| ApiError::new(Code::Store, format!("import: {e}")))?;
            }
            let options = OpenOptions {
                clock: Arc::new(WasmClock),
                ..OpenOptions::default()
            };
            let db = Db::open_with_host(host.clone(), &opts.path, options)
                .map_err(|e| ApiError::new(Code::StorageUnavailable, e.to_string()))?;
            let shard = TiramemsuShard::from_db(db, opts.path.clone()).map_err(|e| match e {
                vertep_store::StoreError::Unavailable(m) => ApiError::new(Code::StoreLocked, m),
                other => ApiError::new(Code::Store, other.to_string()),
            })?;
            Ok(JournalStore {
                shard: Arc::new(shard),
                host: Some(host),
                path: opts.path.clone(),
            })
        }

        /// Opens the file at `path` with tiramemsu's native host (WAL), as the native runtime
        /// does: the other side of the browser-to-native round trip.
        ///
        /// # Errors
        ///
        /// `StoreLocked` and store errors.
        #[cfg(not(target_family = "wasm"))]
        pub fn native(path: &str) -> Res<JournalStore> {
            let shard = TiramemsuShard::open(path).map_err(|e| match e {
                vertep_store::StoreError::Unavailable(m) => ApiError::new(Code::StoreLocked, m),
                other => ApiError::new(Code::Store, other.to_string()),
            })?;
            Ok(JournalStore {
                shard: Arc::new(shard),
                host: None,
                path: path.to_owned(),
            })
        }

        /// The committed bytes of the file.
        ///
        /// # Errors
        ///
        /// Host errors.
        pub fn export(&self) -> Res<Vec<u8>> {
            match &self.host {
                Some(h) => h
                    .export_file(&self.path)
                    .map_err(|e| ApiError::new(Code::Store, format!("export: {e}"))),
                None => std::fs::read(&self.path)
                    .map_err(|e| ApiError::new(Code::Store, format!("export: {e}"))),
            }
        }
    }
}
