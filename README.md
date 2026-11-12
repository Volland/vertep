<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="site/assets/img/logo.svg" alt="Vertep" width="160"></a>
</p>

<h1 align="center">Vertep</h1>

<p align="center">
  Statecharts that run as durable actors, write down every step they take, let a language model choose only among the moves the chart allows, and agree step for step with a model proven in Lean.
</p>

<p align="center">
  <a href="https://volland.github.io/vertep/"><b>Website</b></a> ·
  <a href="https://volland.github.io/vertep/docs/index.html">Getting started</a> ·
  <a href="https://volland.github.io/vertep/examples/">Examples</a> ·
  <a href="https://volland.github.io/vertep/articles/">Articles</a> ·
  <a href="https://docs.rs/vertep">API docs</a>
</p>

<p align="center">
  <a href="https://crates.io/crates/vertep"><img src="https://img.shields.io/crates/v/vertep.svg" alt="crates.io"></a>
  <a href="https://www.npmjs.com/package/@vertep/web"><img src="https://img.shields.io/npm/v/@vertep/web.svg" alt="npm"></a>
  <a href="https://docs.rs/vertep"><img src="https://img.shields.io/docsrs/vertep" alt="docs.rs"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="license"></a>
</p>

---

Vertep is a family of Rust crates, with JavaScript bindings, for long-lived processes that must behave exactly as specified: orders, workflows, devices, and AI agents. The name comes from the *vertep*, the travelling puppet theatre of Ukrainian Christmas, whose puppets move only along slots cut in the stage floor. A Vertep chart is that stage: actors move only along its transitions, and whoever picks the next move, a guard, a person or a language model, can never cut a new slot.

## What you get

| | |
|---|---|
| **Full SCXML** | The W3C SCXML algorithm in full, checked against the W3C conformance suite. Write charts in SCXML, YAML or the `statechart!` macro; all lower to one typed IR. |
| **Durable actors** | Virtual actors addressed by IRI. Every accepted input is journaled before it is processed, with compare-and-set appends, a transactional outbox, durable timers and time travel. |
| **Deterministic agents** | At a `vt:decide` state an LLM, a person or a guard picks among the enabled events. Decisions and model results are journaled, so a run replays without any model. |
| **Charts as RDF** | Every chart projects losslessly to RDF 1.2 with SHACL shapes; its version is the hash of its canonical form, and journals can be queried with SPARQL as of any moment. |
| **Anywhere it must run** | Clusters with leased shards over QUIC, federation across trust boundaries with DIDs and relay mailboxes, and the same core in the browser through WebAssembly. |
| **Proven in Lean** | A Lean 4 model of the semantics replays the Rust traces as a differential oracle. Decider safety, grant enforcement, legal configurations and type soundness of `step` are theorems. |

## Quick start

```toml
[dependencies]
vertep = "0.1"
```

A chart is ordinary SCXML. `checkout` is a compound state, so its `cancel` transition covers every state inside it:

```xml
<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"
       datamodel="ecmascript" initial="idle" name="order">
  <datamodel><data id="total" expr="0"/></datamodel>
  <state id="idle"><transition event="start" target="checkout"/></state>
  <state id="checkout" initial="cart">
    <transition event="cancel" target="cancelled"/>
    <state id="cart">
      <transition event="pay" cond="_event.data.amount &gt; 0" target="payment">
        <assign location="total" expr="_event.data.amount"/>
      </transition>
    </state>
    <state id="payment">
      <transition event="authorized" cond="total &gt; 100" target="review"/>
      <transition event="authorized" target="paid"/>
    </state>
    <state id="review"><transition event="approve" target="paid"/></state>
  </state>
  <final id="paid"/>
  <final id="cancelled"/>
</scxml>
```

Check it and name it with the `vt` command:

```sh
cargo install vertep-cli
vt validate order.scxml     # well-formedness and SHACL conformance
vt hash order.scxml         # urn:vertep:chart:sha256:… — the chart's content-addressed version
```

Then run it as a durable actor, give it a decider, or load it in the browser: the [getting started guide](https://volland.github.io/vertep/docs/index.html) walks through each, and the [examples](https://volland.github.io/vertep/examples/) are complete programs with their real output.

In JavaScript:

```sh
npm install @vertep/web @vertep/dm-ecma   # dm-ecma only for datamodel="ecmascript"; null and CEL are built in
```

```js
import { createActorSystem } from "@vertep/web";
import { ecmascript } from "@vertep/dm-ecma";
const system = await createActorSystem({ storage: "memory", datamodel: ecmascript });
const order = await system.spawn(ORDER_SCXML, { type: "https://example.org/charts/order", key: "42" });
await order.send({ type: "start" });
console.log((await order.getSnapshot()).configuration);
```

## Packages

| Crate | What it is |
|---|---|
| [`vertep`](https://crates.io/crates/vertep) | The facade: one dependency with features for every layer |
| [`vertep-core`](https://crates.io/crates/vertep-core) | The `no_std` chart IR, type checker and pure `step` function |
| [`vertep-scxml`](https://crates.io/crates/vertep-scxml) · [`vertep-yaml`](https://crates.io/crates/vertep-yaml) · [`vertep-macros`](https://crates.io/crates/vertep-macros) | The SCXML, YAML and `statechart!` surfaces |
| [`vertep-dm-ecma`](https://crates.io/crates/vertep-dm-ecma) · [`vertep-dm-cel`](https://crates.io/crates/vertep-dm-cel) · [`vertep-dm-rust`](https://crates.io/crates/vertep-dm-rust) | The ECMAScript, CEL and Rust datamodels |
| [`vertep-rdf`](https://crates.io/crates/vertep-rdf) | The RDF projection, the `vt:` ontology, SHACL shapes and chart versions |
| [`vertep-store`](https://crates.io/crates/vertep-store) · [`vertep-tiramemsu`](https://crates.io/crates/vertep-tiramemsu) · [`vertep-oxigraph`](https://crates.io/crates/vertep-oxigraph) | Journal and graph storage traits and backends |
| [`vertep-runtime`](https://crates.io/crates/vertep-runtime) | The actor runtime: mailboxes, transports, the journal driver, durable timers |
| [`vertep-agent`](https://crates.io/crates/vertep-agent) | Deciders, LLM and tool actors, capability grants, plan charts |
| [`vertep-cluster`](https://crates.io/crates/vertep-cluster) · [`vertep-federation`](https://crates.io/crates/vertep-federation) | Clusters with leased shards; DID identities and relay mailboxes |
| [`vertep-cli`](https://crates.io/crates/vertep-cli) | The `vt` command |

| npm package | What it is |
|---|---|
| [`@vertep/web`](https://www.npmjs.com/package/@vertep/web) | The actor system in a worker, compiled to WebAssembly, with a typed client for browsers and Node.js |
| [`@vertep/dm-ecma`](https://www.npmjs.com/package/@vertep/dm-ecma) | The ECMAScript datamodel for `@vertep/web` |

## Read more

- [The shape of behaviour](https://volland.github.io/vertep/articles/statecharts.html): why statecharts, and what a Lean proof found in the SCXML entry algorithm
- [The goat does not choose the play](https://volland.github.io/vertep/articles/deterministic-agents.html): where a language model belongs in a program

## Where things live

- `crates/`: the Rust workspace
- `bindings/js/`: the npm packages
- `examples/`: runnable examples, held to their recorded output by tests
- `site/`: the website, plain static HTML
- `lean/`: the Lean model and proofs
- `openspec/`: requirements and scenarios, one change per milestone ([OpenSpec](https://github.com/Fission-AI/OpenSpec))
- `tg.md/`: decisions, constraints, components and architecture views as a typed graph, checked with `tg check`

Code links to the requirements it implements with `// @tg: implements::` comments, and tests link to the scenarios they verify with `// @tg: verifies::` comments.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
