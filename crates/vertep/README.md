<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep</h1>

<p align="center">W3C SCXML statecharts as durable actors, with charts as RDF, deterministic LLM agents and a Lean model of the semantics</p>

<p align="center">
  <a href="https://crates.io/crates/vertep"><img src="https://img.shields.io/crates/v/vertep.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep"><img src="https://img.shields.io/docsrs/vertep" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep.svg" alt="license"></a>
</p>

Vertep runs W3C SCXML statecharts as actors. You write a chart in SCXML, in YAML or with a Rust
macro. Vertep runs it as a long-lived actor with an IRI address that handles one event at a
time and writes every event it accepts to a journal before it acts on it. Because the journal
is complete, an actor can be stopped, moved and replayed, and its state can be read back as it
was at any earlier moment. The same chart also projects to RDF, and its version is the hash of
that RDF form, so a journal always records exactly which chart produced it. A chart can mark a
state as a choice point (`vt:decide`); there a guard, a person or a language model picks the
next event, but only from the events the chart enables in that state. A Lean model of the
semantics checks the Rust core trace by trace.

This crate is the facade. It re-exports the Vertep crates behind cargo features, and most
applications depend on it alone.

## What you get

- **Full W3C SCXML.** The complete interpretation algorithm: compound, parallel, history and
  final states, document-order priority, `<send>`, `<invoke>`, `<cancel>`, and the `null` and
  `ecmascript` datamodels, checked against the W3C IRP conformance suite. CEL and Rust
  datamodels and an algebraic type language are added on top, and a typed chart is
  type-checked before it runs.
- **Durable actors.** Virtual actors in the style of Orleans, addressed by IRI and activated by
  their first message. Every turn is a journal entry; delayed sends are durable timers; replay
  rebuilds any past state; chart versions are pinned per actor and migrations are explicit.
  The core `step` is a pure function, and the actor shell around it runs on tokio.
- **Deterministic agents.** At a `vt:decide` state a decider (a guard, a human, another actor or
  an LLM) picks one of the events the chart offers. The answer is journaled before it is
  checked, and checked against the offer before it is taken. LLM and tool actors can send only
  where their capability grants allow. Replay reads decisions from the journal and calls no
  model.
- **Charts as RDF.** Every chart has an exact RDF 1.2 form under the modular `vt:` ontology,
  checked by SHACL shapes and rebuilt losslessly from the graph. Its version IRI is the SHA-256
  of its RDFC-1.0 canonical form. Journals are RDF too, so SPARQL can ask what an actor believed
  at any past moment.
- **Cluster, federation and browser.** Trusted clusters with membership, shard leases with
  fencing epochs, a QUIC transport and worker pools. Federation across trust boundaries with
  DID identities, grants and DIDComm v2 or TSP relay mailboxes. The same core in the browser
  through WebAssembly, with a durable journal in OPFS.
- **Proven in Lean.** A Lean 4 model proves determinism, legal configurations, transition
  selection, lossless RDF round trips, journal replay and decider safety, and runs as a
  differential oracle: the Rust core and the Lean step must produce identical traces. `vt certify`
  issues a certificate that hangs off a chart's version IRI.

## Install

```toml
[dependencies]
vertep = "0.1"                       # default: runtime, tiramemsu, scxml
tokio = { version = "1", features = ["rt", "macros", "time"] }

# Opt in to more:
# vertep = { version = "0.1", features = ["agent", "ecmascript"] }
```

| Feature | Default | Crate | What it turns on |
|---|---|---|---|
| (always) | yes | `vertep-core`, `vertep-store`, `vertep-rdf` | `vertep::core`, `vertep::store`, `vertep::rdf`: the IR and `step`, the storage traits and in-memory store, RDF and versions |
| `runtime` | yes | `vertep-runtime` | `vertep::runtime`: the actor runtime on tokio |
| `tiramemsu` | yes | `vertep-tiramemsu` | `vertep::tiramemsu`: the default storage backend, with history you can query |
| `scxml` | yes | `vertep-scxml` | `vertep::scxml`: the SCXML reader and writer |
| `ecmascript` | no | `vertep-dm-ecma` | `vertep::ecmascript`: the ECMAScript datamodel on boa |
| `agent` | no | `vertep-agent` | `vertep::agent`: decide states, deciders, LLM and tool actors, grants and plan charts |
| `cluster` | no | `vertep-cluster` | `vertep::cluster`: membership, shard leases, QUIC transport and worker pools |
| `federation` | no | `vertep-federation` | `vertep::federation`: DID identities, grants, DIDComm v2 and TSP relays |
| `objects` | no | `vertep-objects` | `vertep::objects`: durable objects with verifiable journals and decentralised custody over TSP |

A node runs all its actors on one thread (some datamodels are not `Send`), so use tokio's
current-thread runtime and a `LocalSet`, as below.

## Quickstart

One virtual actor on tiramemsu, a durable delayed send, and two kinds of time travel. This is
`crates/vertep/examples/quickstart.rs` (run it with `cargo run -p vertep --example quickstart`):

```rust
use std::sync::Arc;
use std::time::Duration;

use vertep::prelude::*;

const ORDER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="200ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"#;

const ORDER_TYPE: &str = "https://example.org/charts/order";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("vertep-readme-{}", std::process::id()));
    let backend = Arc::new(TiramemsuBackend::open(&dir, 1)?);

    let mut node = Node::builder(backend.clone(), Shell::tokio()).build();
    let chart = vertep::scxml::read_str(ORDER)?.chart;
    let version = node.register_type(ORDER_TYPE, chart)?;
    println!("chart version {}", version.as_str());

    // A virtual actor exists for every key: no create call, the first message activates it.
    let order = Address::virtual_actor(ORDER_TYPE, "42")?;
    node.send(&order, Event::named("pay", EventType::External))?;

    // Run the node for 400 ms on this thread; the handle works from anywhere.
    let handle = node.handle();
    let local = tokio::task::LocalSet::new();
    let mut node = local
        .run_until(async move {
            let runner = tokio::task::spawn_local(async move {
                node.run().await;
                node
            });
            tokio::time::sleep(Duration::from_millis(400)).await;
            handle.shutdown();
            runner.await
        })
        .await?;

    // The delayed `ship` fired as an ordinary journaled input.
    let now = node.state(&order)?;
    assert_eq!(now.configuration, ["shipped"]);
    for e in node.journal(&order)? {
        if let vertep::runtime::EntryBody::Input { input, effects } = &e.body {
            let timers = effects.timers.len();
            println!("entry {}: {} (timers scheduled: {timers})", e.position, input.event.name);
        }
    }

    // Time travel by replay: the state after entry 1, without touching the live actor.
    let then = node.state_at(&order, 1)?;
    assert_eq!(then.configuration, ["paid"]);

    // Time travel in the store: tiramemsu never deletes, so every past transaction is queryable.
    let shard = backend.shard_for(order.node());
    let query = format!(
        "SELECT ?s WHERE {{ GRAPH {o} {{ {o} <{p}> ?s }} }}",
        o = order.node(),
        p = vertep::runtime::vocab::IN_STATE
    );
    for t in 1..=shard.last_tx()? {
        let states = shard.view_as_of(t).query(&query)?.into_solutions()?;
        println!("tiramemsu tx {t}: {:?}", states.values("s"));
    }

    drop(node);
    drop(backend);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
```

The chart waits in `open`. `pay` moves it to `paid` and schedules `ship` 200 ms later; that
timer is written to the journal, so it would still fire if the process stopped and restarted.
`node.state` reads the current configuration, `node.journal` lists every accepted input, and
`node.state_at` rebuilds the state after a given entry by replay. The last loop asks tiramemsu
what the actor's state graph said after each store transaction, with SPARQL and no replay. The
full example prints:

```text
chart version urn:vertep:chart:sha256:6b74f6c5d780053619e9d360bbd73331f41fd52709d0bb7f5310d175b1272b08
now: ["shipped"] after 2 entries
  entry 1: pay (timers scheduled: 1)
  entry 2: ship (timers scheduled: 0)
as of entry 1: ["paid"]
  tiramemsu tx 2: in ["paid"]
  ...
  tiramemsu tx 6: in ["shipped"]
```

More complete programs, each checked against its recorded output, are in
[`examples/`](https://github.com/Volland/vertep/tree/main/examples): a traffic light on the
pure core, a chart written with the Rust macro, a durable actor that survives a restart, a
decider agent replayed without its decider, a typed order on CEL, and `@vertep/web` in Node.js.

## Crates

| Crate | What it is |
|---|---|
| [`vertep`](https://crates.io/crates/vertep) | This facade: the crates below behind cargo features |
| [`vertep-core`](https://crates.io/crates/vertep-core) | Chart IR, validation, type checker and the pure SCXML `step` function (`no_std`) |
| [`vertep-scxml`](https://crates.io/crates/vertep-scxml) | SCXML 1.0 reader and writer with source locations and a loss report |
| [`vertep-yaml`](https://crates.io/crates/vertep-yaml) | The YAML surface: a lossless mirror of the IR with located errors |
| [`vertep-macros`](https://crates.io/crates/vertep-macros) | The `statechart!` and `include_chart!` macros (use them through `vertep-dm-rust`) |
| [`vertep-dm-rust`](https://crates.io/crates/vertep-dm-rust) | The `rust` datamodel and the typed API the macros generate |
| [`vertep-dm-cel`](https://crates.io/crates/vertep-dm-cel) | The CEL datamodel: deterministic, bounded and statically type-checked (`no_std`) |
| [`vertep-dm-ecma`](https://crates.io/crates/vertep-dm-ecma) | The W3C `ecmascript` datamodel on boa |
| [`vertep-rdf`](https://crates.io/crates/vertep-rdf) | RDF 1.2 projection, `vt:` ontology, SHACL shapes and content-addressed versions |
| [`vertep-store`](https://crates.io/crates/vertep-store) | Storage traits: a compare-and-set `JournalStore` and a SPARQL `GraphStore` |
| [`vertep-tiramemsu`](https://crates.io/crates/vertep-tiramemsu) | The default backend: journal and graphs in one file per shard, with queryable history |
| [`vertep-oxigraph`](https://crates.io/crates/vertep-oxigraph) | Oxigraph and SPARQL 1.1 endpoints as graph stores fed by a projector |
| [`vertep-runtime`](https://crates.io/crates/vertep-runtime) | The actor runtime: activations, mailboxes, journal, durable timers, versions and migrations |
| [`vertep-agent`](https://crates.io/crates/vertep-agent) | Deciders, LLM and tool actors, capability grants and sandboxed plan charts |
| [`vertep-cluster`](https://crates.io/crates/vertep-cluster) | Membership, shard leases, QUIC transport and worker pools |
| [`vertep-federation`](https://crates.io/crates/vertep-federation) | DID identities, grants and DIDComm v2 and TSP relay mailboxes |
| [`vertep-objects`](https://crates.io/crates/vertep-objects) | Durable objects: verifiable journals and decentralised custody over TSP |
| [`vertep-tsp-mediator`](https://crates.io/crates/vertep-tsp-mediator) | A TSP mailbox mediator and the `vt-mediator` binary |
| [`vertep-cli`](https://crates.io/crates/vertep-cli) | The `vt` command: `typecheck`, `schema`, `validate`, `hash`, `convert`, `gen ts`, `certify` |

For JavaScript, two npm packages wrap the core compiled to WebAssembly:

| Package | What it is |
|---|---|
| `@vertep/web` | Vertep actors in browsers and Node.js: SCXML semantics, typed events and a time-travelling journal, running in a worker, with storage in memory or OPFS |
| `@vertep/dm-ecma` | The ECMAScript datamodel for `@vertep/web`: a build of the worker with the boa engine |

## Documentation

- Website: https://volland.github.io/vertep/ (start with https://volland.github.io/vertep/docs/index.html, Getting started)
- Concepts: https://volland.github.io/vertep/docs/concepts.html
- Writing charts: https://volland.github.io/vertep/docs/charts.html
- The actor runtime: https://volland.github.io/vertep/docs/runtime.html
- Agents and deciders: https://volland.github.io/vertep/docs/agents.html
- RDF and versions: https://volland.github.io/vertep/docs/rdf.html
- Verification: https://volland.github.io/vertep/docs/verification.html
- API reference: https://docs.rs/vertep
- Examples: https://volland.github.io/vertep/examples/ (the quickstart's bigger sibling is https://volland.github.io/vertep/examples/durable-actor.html)
- Articles: https://volland.github.io/vertep/articles/ (statecharts, and deterministic agents)

## License

MIT OR Apache-2.0, at your option.
