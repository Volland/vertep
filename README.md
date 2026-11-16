<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="site/assets/img/logo.svg" alt="Vertep" width="160"></a>
</p>

<h1 align="center">Vertep</h1>

<p align="center">
  <b>Let a language model choose, but only among the moves your policy allows.</b><br>
  Every step is written down before it happens, so any decision can be replayed and explained later, without the model.
</p>

<p align="center">
  <a href="https://volland.github.io/vertep/why.html"><b>Why Vertep</b></a> ·
  <a href="https://volland.github.io/vertep/regulated.html">For regulated work</a> ·
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

## Why

An AI agent is usually a loop around a prompt. Its state is a transcript, it cannot be replayed, and its explanation is written afterwards by the same kind of system that gave the answer. That is a poor basis for a loan, a claim or a treatment request, where someone will later ask *why*.

Vertep puts the model at a **choice point** inside a statechart it does not control:

- **Bounded.** At a decide state the chart offers only the moves enabled right now. A model cannot pick one the chart does not offer, and that is a theorem in the Lean model, not a prompt instruction.
- **Recorded.** Inputs, decisions and model results are journaled before they take effect.
- **Replayable.** Replay the journal with no model bound and reach the same states. The thing under examination is not needed to examine it.
- **Explainable.** The record answers: what state, what was offered, who chose, under which version of the rules, and what followed. See [Explainable by construction](https://volland.github.io/vertep/articles/explainable-by-construction.html).

### See it in ten minutes

```sh
git clone https://github.com/Volland/vertep && cd vertep
cargo run -p vertep-examples --bin credit-decision
```

Three loan applications are decided by a scripted stand-in for a model. On the second, the model answers *approve* for an amount over the policy ceiling; the chart never offered it, refuses the answer, and an officer decides. Then an auditor opens the same journal with **no decider bound** and reads what happened, entry by entry. Walkthrough: [examples/credit-decision](https://volland.github.io/vertep/examples/credit-decision.html). The same pattern for [healthcare prior authorisation and insurance claims](https://volland.github.io/vertep/articles/regulated-decisions.html) is shown as charts.

### Where it stands

Version **0.1.x**, built in the open and quickly. The SCXML core, the journal, deciders and the Lean model are the strong parts; clusters, federation, durable objects, wallets, agent protocols and rules are built and specified but younger, with open items on the [roadmap](tg.md/roadmap.md). Each package's [stability tier](#packages-and-stability-tiers) says which is which. It is not a compliance product: it gives you an honest record and structural limits, and [what it contributes to the rules you answer to](https://volland.github.io/vertep/regulated.html) is stated next to what it does not. Decision records (explanations, offline-verified bundles, signed agent journals, counterfactual replay and erasure by key destruction) are built in Rust and not yet exposed in JavaScript or Python; what remains of them is in [`openspec/changes/m12-decision-records`](openspec/changes/m12-decision-records). Benchmarks, a reproducible build, a supply-chain gate and journal shipping between nodes are in [`m13-production-readiness`](openspec/changes/m13-production-readiness). Feedback and issues are the most useful contribution right now.

---

Vertep is a family of Rust crates, with JavaScript bindings, for long-lived processes that must behave exactly as specified: orders, workflows, devices, and AI agents. The name comes from the *vertep*, the travelling puppet theatre of Ukrainian Christmas, whose puppets move only along slots cut in the stage floor. A Vertep chart is that stage: actors move only along its transitions, and whoever picks the next move, a guard, a person or a language model, can never cut a new slot.

## What you get

| | |
| --- | --- |
| **Full SCXML** | The W3C SCXML algorithm in full, checked against the W3C conformance suite. Write charts in SCXML, YAML or the `statechart!` macro; all lower to one typed IR. |
| **Durable actors** | Virtual actors addressed by IRI. Every accepted input is journaled before it is processed, with compare-and-set appends, a transactional outbox, durable timers and time travel. |
| **Deterministic agents** | At a `vt:decide` state an LLM, a person or a guard picks among the enabled events. Decisions and model results are journaled, so a run replays without any model. |
| **Charts as RDF** | Every chart projects losslessly to RDF 1.2 with SHACL shapes; its version is the hash of its canonical form, and journals can be queried with SPARQL as of any moment. |
| **Durable objects** | An actor with a `did:webvh` address whose journal is kept by custodians in several organisations, talking over the Trust Spanning Protocol (TSP). One writer per epoch, won by a majority of promises: no coordinator and no clock. It survives the loss of any one organisation. A TSP mailbox mediator (`vt-mediator`) holds messages for parties that are offline. |
| **Anywhere it must run** | Clusters with leased shards over QUIC, federation across trust boundaries with DIDs and relay mailboxes, and the same core in the browser through WebAssembly. |
| **Proven in Lean** | A Lean 4 model of the semantics replays the Rust traces as a differential oracle. Decider safety, grant enforcement, legal configurations, type soundness of `step`, and one writer per custody epoch are theorems. |

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
npm install @vertep/web@next @vertep/dm-ecma@next   # dm-ecma only for datamodel="ecmascript"; null and CEL are built in
```

The npm `latest` tag still points at 0.1.2; `next` carries the current line.

```js
import { createActorSystem } from "@vertep/web";
import { ecmascript } from "@vertep/dm-ecma";
const system = await createActorSystem({ storage: "memory", datamodel: ecmascript });
const order = await system.spawn(ORDER_SCXML, { type: "https://example.org/charts/order", key: "42" });
await order.send({ type: "start" });
console.log((await order.getSnapshot()).configuration);
```

In Python, `pip install vertep` (CPython 3.9 or later): the same actors, journals and time travel; see the [Python guide](https://volland.github.io/vertep/docs/python.html).

## Packages and stability tiers

Every crate and package declares a tier in its manifest. **Stable**: checked against an outside reference (the W3C SCXML suite, the Lean model as an oracle, or the storage contract kit) and used by the tested examples; APIs may still change between 0.x minor versions, with a CHANGELOG entry. **Preview**: well tested and used by examples or bindings, but with no outside reference, recent changes or open tasks. **Experimental**: tested by its own suite only, with open items on the roadmap and no user outside the project yet.

<!-- tiers:begin -->
| Package | Tier | What it is |
| --- | --- | --- |
| [`vertep`](https://crates.io/crates/vertep) | stable | SCXML statecharts, virtual actors and AI agents with charts as RDF: the Vertep facade crate |
| [`vertep-core`](https://crates.io/crates/vertep-core) | stable | Chart IR, validation and the pure SCXML step kernel for Vertep (no_std) |
| [`vertep-scxml`](https://crates.io/crates/vertep-scxml) | stable | SCXML reader and writer for Vertep charts |
| [`vertep-dm-ecma`](https://crates.io/crates/vertep-dm-ecma) | stable | ECMAScript datamodel for Vertep on the boa engine |
| [`vertep-rdf`](https://crates.io/crates/vertep-rdf) | stable | RDF 1.2 projection, vt ontology, SHACL shapes and canonical hashing for Vertep charts |
| [`vertep-dm-cel`](https://crates.io/crates/vertep-dm-cel) | stable | CEL datamodel for Vertep |
| [`vertep-store`](https://crates.io/crates/vertep-store) | stable | Storage contracts for Vertep: JournalStore, GraphStore and their test kit |
| [`vertep-tiramemsu`](https://crates.io/crates/vertep-tiramemsu) | stable | Tiramemsu backend for Vertep journals and graphs |
| [`vertep-runtime`](https://crates.io/crates/vertep-runtime) | stable | Event-sourced actor runtime for Vertep: activations, mailboxes, journal, timers and versioning |
| [`vertep-yaml`](https://crates.io/crates/vertep-yaml) | preview | YAML surface for Vertep charts |
| [`vertep-oxigraph`](https://crates.io/crates/vertep-oxigraph) | preview | Oxigraph and SPARQL 1.1 graph store adapter for Vertep |
| [`vertep-macros`](https://crates.io/crates/vertep-macros) | preview | The statechart! macro: Rust DSL for Vertep charts |
| [`vertep-dm-rust`](https://crates.io/crates/vertep-dm-rust) | preview | Rust datamodel for Vertep charts written with the statechart! macro |
| [`vertep-cli`](https://crates.io/crates/vertep-cli) | preview | The vt command line: typecheck, schema, validate, hash, convert, certify, generate code, clusters, did:webvh, wallets, explanations, bundles, rules and agent protocols |
| [`vertep-agent`](https://crates.io/crates/vertep-agent) | preview | Deciders, LLM actors, tools with grants and sandboxed plan charts for Vertep agents |
| [`@vertep/dm-ecma`](https://www.npmjs.com/package/@vertep/dm-ecma) | preview | The ECMAScript datamodel for @vertep/web: a build of the Vertep worker with the boa engine |
| [`@vertep/web`](https://www.npmjs.com/package/@vertep/web) | preview | Vertep statecharts as durable actors in browsers and Node.js: SCXML semantics, typed events and a time-travelling journal, running in a worker |
| [`vertep` (Python)](https://pypi.org/project/vertep/) | preview | W3C SCXML statecharts as durable, journaled actors, from Python: typed events, time travel, SPARQL over the state graph |
| [`vertep-cluster`](https://crates.io/crates/vertep-cluster) | experimental | Cluster membership, shard leases, QUIC transport and worker pools for Vertep |
| [`vertep-federation`](https://crates.io/crates/vertep-federation) | experimental | DID identity, grants and relay mailboxes (TSP, DIDComm v2) for Vertep |
| [`vertep-rules`](https://crates.io/crates/vertep-rules) | experimental | Rule programs beside the journal: a rules decider whose rationale is a derivation, run by lean-elpi through tiramemsu |
| [`vertep-objects`](https://crates.io/crates/vertep-objects) | experimental | Durable objects for Vertep: DID-addressed actors with verifiable journals and decentralised custody over TSP |
| [`vertep-tsp-mediator`](https://crates.io/crates/vertep-tsp-mediator) | experimental | A TSP mailbox mediator: routed-mode intermediary with durable store-and-forward mailboxes and pickup |
| [`vertep-webvh`](https://crates.io/crates/vertep-webvh) | experimental | did:webvh for Vertep: create, update, witness, publish and serve DID logs with key-manager keys |
| [`vertep-wallet`](https://crates.io/crates/vertep-wallet) | experimental | Agent wallets for Vertep: sealed credentials, consent, pluggable exchange protocols and formats, and an exchange model checked in Lean |
| [`vertep-kolyada`](https://crates.io/crates/vertep-kolyada) | experimental | Kolyada: verifiable role-based agent protocols for Vertep, with signed messages, projection and conformance checks |

Generated by `python3 site/tools/tiers.py` from the manifests; the reasons for each tier are on [the tiers page](https://volland.github.io/vertep/docs/tiers.html).
<!-- tiers:end -->

## Read more

- [The shape of behaviour](https://volland.github.io/vertep/articles/statecharts.html): why statecharts, and what a Lean proof found in the SCXML entry algorithm
- [The goat does not choose the play](https://volland.github.io/vertep/articles/deterministic-agents.html): where a language model belongs in a program

## Where things live

- `crates/`: the Rust workspace
- `bindings/js/`: the npm packages
- `bindings/python/`: the `vertep` Python package (PyO3 extension in `crates/vertep-py`; release steps in `RELEASING.md`)
- `examples/`: runnable examples, held to their recorded output by tests
- `site/`: the website, plain static HTML
- `lean/`: the Lean model and proofs
- `openspec/`: requirements and scenarios, one change per milestone ([OpenSpec](https://github.com/Fission-AI/OpenSpec))
- `tg.md/`: decisions, constraints, components and architecture views as a typed graph, checked with `tg check`
- `CHANGELOG.md`: what changed in each release

Code links to the requirements it implements with `// @tg: implements::` comments, and tests link to the scenarios they verify with `// @tg: verifies::` comments.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
