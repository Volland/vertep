<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-runtime</h1>

<p align="center">The Vertep actor runtime: SCXML charts as durable, journaled actors</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-runtime"><img src="https://img.shields.io/crates/v/vertep-runtime.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-runtime"><img src="https://img.shields.io/docsrs/vertep-runtime" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-runtime.svg" alt="license"></a>
</p>

`vertep-runtime` turns a W3C SCXML chart into a durable actor. An actor has an IRI address
and a mailbox. Each turn takes one message, runs one SCXML macrostep with the pure `step` of
`vertep-core`, and appends one journal entry with compare-and-set. Only after the append
succeeds are the turn's sends dispatched. Delayed sends are durable timers in the journal, so
they fire after a restart. An actor's state at any past position can be rebuilt by replay.

The runtime is a shell around the pure core: time, spawning and sleeping come in through a
`Shell`, so tests run on a virtual clock. It stores journals through the traits of
`vertep-store`, on `MemoryStore`, on `vertep-tiramemsu` (feature `tiramemsu`) or on any
`Storage` you write. `vertep-agent`, `vertep-cluster` and `vertep-federation` build on it.
The `vertep` facade turns it on with its default `runtime` feature and re-exports it as
`vertep::runtime`, with the common types in `vertep::prelude`. Most applications should depend
on `vertep`. Depend on this crate directly to pick the storage backend and features yourself.

## Install

```toml
[dependencies]
vertep-runtime = { version = "0.1", features = ["tiramemsu"] }
vertep-core = "0.1"    # Event, EventType, Chart
vertep-scxml = "0.1"   # read SCXML documents into a Chart
tokio = { version = "1", features = ["rt", "macros", "time"] }   # for Shell::tokio() and node.run()
```

| Feature | Turns on |
| --- | --- |
| `tiramemsu` | `Storage` for `TiramemsuShard` and `TiramemsuBackend`, the default durable backend |
| `websocket` | `websocket::server`, the server end of the WebSocket Event I/O Processor (tokio-tungstenite). The frames and link state are always built |

With no features, the runtime runs on `vertep_store::MemoryStore` or on a `SplitStorage` you
assemble from any journal store and graph store.

## Example

```rust
use std::sync::Arc;
use std::time::Duration;

use vertep_core::datamodel::{Event, EventType};
use vertep_runtime::{Address, Node, Shell};
use vertep_store::MemoryStore;

const ORDER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="200ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"#;
const ORDER_TYPE: &str = "https://example.org/charts/order";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(MemoryStore::new());
    let chart = || vertep_scxml::read_str(ORDER).map(|p| p.chart).map_err(|e| format!("{e:?}"));

    // A node on a virtual clock, driven by hand: the run is the same every time.
    let (shell, clock) = Shell::virtual_time(0);
    let mut node = Node::builder(store.clone(), shell.clone()).build();
    node.register_type(ORDER_TYPE, chart()?)?;

    // A virtual actor exists for every key: the first message activates it.
    let order = Address::virtual_actor(ORDER_TYPE, "42")?;
    node.send(&order, Event::named("pay", EventType::External))?;
    node.run_until_idle();
    assert_eq!(node.state(&order)?.configuration, ["paid"]);
    drop(node); // the process stops; the `ship` timer is in the journal

    // A new node on the same store. When the clock passes the due time, the timer fires
    // as an ordinary journaled input.
    let mut node = Node::builder(store, shell).build();
    node.register_type(ORDER_TYPE, chart()?)?;
    clock.advance(Duration::from_millis(200));
    node.run_until_idle();

    let now = node.state(&order)?;
    assert_eq!(now.configuration, ["shipped"]);
    assert_eq!(now.position, 2);
    // Time travel by replay: the state after entry 1.
    assert_eq!(node.state_at(&order, 1)?.configuration, ["paid"]);
    Ok(())
}
```

The first node accepts `pay` and writes entry 1, which records the delayed `ship` send as a
durable timer. That node is then dropped. A second node on the same store finds the pending
timer, and when the virtual clock reaches its due time the timer becomes entry 2. `state_at`
rebuilds the state after entry 1 by replay, without touching the live actor. A server uses
`Shell::tokio()` and `node.run().await` inside a `tokio::task::LocalSet` instead of the virtual
clock; see the facade's [quickstart](https://github.com/Volland/vertep/blob/main/crates/vertep/examples/quickstart.rs).

## Key types

| Item | What it is |
| --- | --- |
| `Node`, `NodeBuilder`, `NodeConfig` | the scheduler of one process: `register_type`, `send`, `run`, `run_until_idle`, `state`, `state_at`, `journal`, `migrate`, `fork`, `dead_letters`, `pending_timers` |
| `NodeHandle` | a `Send + Sync` handle to a running node: `send`, `ask` (send and wait for a `vt:reply`), `shutdown` |
| `Address` | an actor's IRI; `Address::virtual_actor(chart_type, key)` |
| `Shell`, `Clock`, `Spawn`, `Sleep`, `VirtualClock` | the async seam: `Shell::tokio()` for servers, `Shell::virtual_time(ms)` for tests |
| `Storage`, `SplitStorage` | a journal store plus the graph stores for current state, snapshots, outbox marks and mailboxes; implemented for `MemoryStore` and, with `tiramemsu`, the tiramemsu types |
| `Mailbox`, `MemoryMailboxes`, `StoreMailboxes` | in-memory or store-backed mailboxes, set per node or per IRI scheme |
| `Transport`, `LocalTransport`, `Resolver` | SCXML Event I/O Processors selected by `<send type="...">`, and the directory of resolvers per IRI scheme |
| `Actor`, `ChartActor`, `FnActor`, `StreamActor` | the actor contract and its adapters for `<invoke>` children and worker pools |
| `ChartRegistry`, `ChartInfo`, `Migration` | content-addressed chart versions (`urn:vertep:chart:sha256:...`) and explicit migrations between them |
| `EntryData`, `EntryBody`, `TimerRecord` | journal entries decoded from RDF |
| `ontology`, `vocab` | the runtime ontology, its SHACL shapes and the `vt:` IRIs |
| `Placement`, `Route` | the hook a cluster uses to place actors on nodes |

The `vt:` extensions the runtime reads from charts include `<vt:request>` (ask another actor
and wait without blocking), `vt:defer` (keep matching events in the mailbox), `vt:unhandled`,
`<vt:reminder>` (recurring durable timers), `vt:restart` supervision on `<invoke>`, and
`<vt:migrate>`.

Known limits: a `Node` runs on one thread because datamodels need not be `Send`; run it in a
`LocalSet` and reach it from elsewhere through `node.handle()`. An invoked child chart has no
journal of its own and is rebuilt by replaying its parent, so a child that must outlive its
node should be a virtual actor reached by `<send>`.

## Documentation

- Website: https://volland.github.io/vertep/ ([The actor runtime](https://volland.github.io/vertep/docs/runtime.html))
- API reference: https://docs.rs/vertep-runtime
- Examples: https://volland.github.io/vertep/examples/ ([durable actor](https://volland.github.io/vertep/examples/durable-actor.html))

## License

MIT OR Apache-2.0, at your option.
