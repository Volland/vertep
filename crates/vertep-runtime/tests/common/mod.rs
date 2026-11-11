//! Shared helpers for the runtime's integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use vertep_core::datamodel::{DatamodelRegistry, Event, EventType, Value};
use vertep_core::ir::Chart;
use vertep_runtime::{Address, Node, NodeConfig, Shell, Storage, VirtualClock};
use vertep_store::MemoryStore;

/// The start of the virtual clock: 2026-11-11T00:00:00Z.
pub const T0: i64 = 1_794_355_200_000;

/// A chart type IRI.
pub fn ty(name: &str) -> String {
    format!("https://example.org/charts/{name}")
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

/// An SCXML document with the given body, null datamodel.
pub fn scxml(initial: &str, body: &str) -> Chart {
    chart(&format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" initial="{initial}">{body}</scxml>"#
    ))
}

/// An SCXML document with the ECMAScript datamodel.
pub fn ecma_scxml(initial: &str, body: &str) -> Chart {
    chart(&format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="ecmascript" initial="{initial}">{body}</scxml>"#
    ))
}

/// The ECMAScript registry.
pub fn ecma() -> Arc<dyn DatamodelRegistry + Send + Sync> {
    Arc::new(vertep_dm_ecma::EcmaRegistry)
}

/// A node on `store` with a virtual clock.
pub fn node_on(store: Arc<dyn Storage>, clock: &VirtualClock, config: NodeConfig) -> Node {
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    Node::builder(store, shell)
        .datamodels(ecma())
        .config(config)
        .build()
}

/// A fresh node on a fresh memory store.
pub fn fresh() -> (Node, VirtualClock, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    let clock = VirtualClock::new(T0);
    let node = node_on(store.clone(), &clock, NodeConfig::default());
    (node, clock, store)
}

/// An external event.
pub fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

/// An external event with data.
pub fn ev_data(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        data,
        ..Event::default()
    }
}

/// The virtual actor `key` of chart type `name`.
pub fn va(name: &str, key: &str) -> Address {
    Address::virtual_actor(&ty(name), key).unwrap()
}

/// A JSON object value.
pub fn obj(kv: &[(&str, Value)]) -> Value {
    Value::Object(
        kv.iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

struct NoStates;

impl vertep_core::datamodel::StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

/// The value of expression `expr` in an ECMAScript datamodel snapshot.
pub fn data_of(snapshot: &Value, expr: &str) -> Option<Value> {
    let mut dm = vertep_dm_ecma::create("ecmascript")?;
    dm.restore(snapshot).ok()?;
    dm.eval_value(expr, &NoStates).ok()
}

/// The value of key `k` of a plain object value.
pub fn field(v: &Value, k: &str) -> Option<Value> {
    match v {
        Value::Object(kv) => kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()),
        _ => None,
    }
}
