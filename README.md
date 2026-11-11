# Vertep

Vertep is a family of Rust crates for statecharts and actors:

- **W3C SCXML semantics**, implemented in full, as the normative behaviour of every chart
- **A typed chart IR as the hub** between SCXML, YAML, a Rust macro DSL and RDF 1.2
- **Charts stored as RDF** with a modular `vt:` ontology, SHACL shapes and content-addressed versions, independent of the RDF store
- **Actors in the style of Orleans virtual actors**, with IRI addresses, pluggable mailboxes and transports, event sourcing and durable timers, running locally or across a cluster
- **AI agents** whose choice points can be decided by guards, LLMs, humans or other actors
- **A Lean model** of the semantics that acts as a differential oracle and issues chart certificates

## Status

Design and specs are complete; implementation is proceeding milestone by milestone.

| Milestone | Scope |
|---|---|
| M0 | SCXML core: IR, interpretation, SCXML format, datamodels, RDF, Lean model, conformance |
| M1 | Typed authoring: type system, Rust DSL, YAML, CEL |
| M2 | Actor runtime: actors, messaging, journal, storage, timers, versioning |
| M3 | Agents: deciders, LLM actors, plan charts |
| M4 | Cluster: membership, shards, QUIC transport, worker pools |
| M5 | Federation: DID identity, relay mailboxes |
| M6 | WebAssembly and JavaScript bindings |

## Where things live

- `openspec/` — requirements and scenarios, one change per milestone ([OpenSpec](https://github.com/Fission-AI/OpenSpec))
- `tg.md/` — decisions, constraints, components and architecture views as a typed graph, checked with `tg check`
- `crates/` — the Rust workspace
- `lean/` — the Lean model and proofs

Code links to the requirements it implements with `// @tg: implements::` comments, and tests link to the scenarios they verify with `// @tg: verifies::` comments.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
