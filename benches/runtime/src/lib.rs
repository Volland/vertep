//! Shared fixtures of the Vertep benchmark suite: charts, nodes on each store, and history.
//!
//! The suite itself is in `benches/`; `scripts/bench-gate.py` collects its results with the
//! commit, the machine and the durability mode and compares them with `benches/baseline.json`.
//!
//! @tg: implements:: [[openspec:runtime-performance#Hot Path Benchmarks]]
//! @tg: implements:: [[openspec:production-readiness#Benchmark Suite]] {confidence: partial}

use std::sync::Arc;

use vertep_core::datamodel::{DatamodelRegistry, Event, EventType};
use vertep_core::ir::Chart;
use vertep_runtime::shell::TokioSpawn;
use vertep_runtime::{Address, Node, NodeConfig, Shell, Storage, VirtualClock};
use vertep_store::MemoryStore;
use vertep_tiramemsu::TiramemsuShard;

/// 2026-11-11T00:00:00Z, the virtual clock's start.
pub const T0: i64 = 1_794_355_200_000;

/// The durability mode the tiramemsu cases run with, recorded with every result.
///
/// The backend opens shards with tiramemsu's default, `synchronous = NORMAL`, until M18
/// phase 1 makes immediate durability the default.
pub const TIRAMEMSU_DURABILITY: &str = "normal";

/// A counter: one transition, one assignment.
pub const COUNTER: &str = r#"<datamodel><data id="n" expr="0"/></datamodel>
<state id="s"><transition event="inc"><assign location="n" expr="n + 1"/></transition></state>"#;

/// A chart type IRI.
pub fn ty(name: &str) -> String {
    format!("https://example.org/bench/{name}")
}

/// A virtual actor of type `name` with key `key`.
pub fn va(name: &str, key: &str) -> Address {
    Address::virtual_actor(&ty(name), key).expect("virtual actor address")
}

/// An external event named `name`.
pub fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

/// Reads an SCXML document (any datamodel).
pub fn chart(xml: &str) -> Chart {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(xml, &opts)
        .unwrap_or_else(|e| panic!("chart reads: {e:?}"))
        .chart
}

/// An SCXML document with `body`, in datamodel `datamodel` (`null` or `ecmascript`).
pub fn scxml(datamodel: &str, initial: &str, body: &str) -> Chart {
    chart(&format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="{datamodel}" initial="{initial}">{body}</scxml>"#
    ))
}

/// The ECMAScript registry.
pub fn ecma() -> Arc<dyn DatamodelRegistry + Send + Sync> {
    Arc::new(vertep_dm_ecma::EcmaRegistry)
}

/// A node on `storage` with a virtual clock and `config`.
pub fn node_on(storage: Arc<dyn Storage>, config: NodeConfig) -> Node {
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock),
    };
    Node::builder(storage, shell)
        .datamodels(ecma())
        .config(config)
        .build()
}

/// The stores the suite runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The in-memory reference store.
    Memory,
    /// One tiramemsu shard file in a temporary directory.
    Tiramemsu,
}

impl Backend {
    /// Both backends.
    pub const ALL: [Backend; 2] = [Backend::Memory, Backend::Tiramemsu];

    /// The name used in benchmark ids.
    pub fn name(self) -> &'static str {
        match self {
            Backend::Memory => "memory",
            Backend::Tiramemsu => "tiramemsu",
        }
    }
}

/// An open store of a [`Backend`], with the directory that holds its file.
pub struct Store {
    /// The storage, for a node.
    pub storage: Arc<dyn Storage>,
    /// The temporary directory of a file-backed store, removed on drop.
    pub dir: Option<tempfile::TempDir>,
}

impl Store {
    /// Opens a fresh, empty store.
    pub fn open(backend: Backend) -> Store {
        match backend {
            Backend::Memory => Store {
                storage: Arc::new(MemoryStore::new()),
                dir: None,
            },
            Backend::Tiramemsu => {
                let dir = tempfile::tempdir().expect("temporary directory");
                let shard =
                    TiramemsuShard::open(dir.path().join("bench.db")).expect("tiramemsu shard");
                Store {
                    storage: Arc::new(shard),
                    dir: Some(dir),
                }
            }
        }
    }
}

/// A node on `store` with the counter chart registered and the actor `counter/<key>` given
/// `entries` turns of history.
pub fn counter_with_history(
    store: &Store,
    key: &str,
    entries: u64,
    config: NodeConfig,
) -> (Node, Address) {
    let mut node = node_on(store.storage.clone(), config);
    node.register_type(&ty("counter"), scxml("ecmascript", "s", COUNTER))
        .expect("counter registers");
    let a = va("counter", key);
    let mut left = entries;
    while left > 0 {
        let batch = left.min(1000);
        for _ in 0..batch {
            node.send(&a, ev("inc")).expect("send");
        }
        node.run_until_idle();
        left -= batch;
    }
    (node, a)
}
