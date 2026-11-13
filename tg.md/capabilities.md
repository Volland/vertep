---
type: Capability
status: draft
openspec: [chart-model, scxml-interpretation, scxml-format, datamodel, chart-rdf, formal-model, scxml-conformance, type-system, rust-dsl, yaml-format, cel-datamodel, actor-model, messaging, event-journal, storage, durable-timers, chart-versioning, runtime-ontology, deciders, llm-actors, plan-charts, cluster-membership, shard-placement, cluster-transport, worker-pools, did-identity, relay-mailboxes, wasm-bindings, durable-objects, object-custody, tsp-mediator, webvh-identity, webvh-hosting]
---
# Capabilities

Every Vertep capability, the milestone that delivers it, and the OpenSpec capability that specifies it under `openspec/specs/<name>/` once archived.

## Chart model

The canonical typed IR of a statechart: states, transitions, executable content, document order, and the well-formedness rules every surface must satisfy. OpenSpec: `chart-model`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## SCXML interpretation

The pure `step` function implementing the W3C SCXML interpretation algorithm: macrosteps, microsteps, transition selection, entry and exit order, history and invoke lifecycle. OpenSpec: `scxml-interpretation`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## SCXML format

Parsing and serializing SCXML documents to and from the IR, with `vt:` extension namespaces, a reserved `vl:` layout namespace dropped as formatting, and declared loss for unsupported constructs. OpenSpec: `scxml-format`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## Datamodel

The pluggable `Datamodel` trait and its `null` and `ecmascript` implementations, system variables, and pure guard evaluation. OpenSpec: `datamodel`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## Chart RDF

The `vt:` chart ontology module, the lossless IR to RDF 1.2 projection, SHACL shapes and RDFC-1.0 content addressing of charts. OpenSpec: `chart-rdf`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## Formal model

The Lean package that models the IR, types and semantics, proves the target theorems, serves as a differential oracle and issues chart certificates. OpenSpec: `formal-model`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## SCXML conformance

Running the W3C SCXML IRP conformance suite against the Rust core and the Lean model, with an allow-list of expected deviations. OpenSpec: `scxml-conformance`.

- scheduled_in:: [[roadmap#M0 SCXML core]]

## Type system

One algebraic type language in the IR for event payloads and context, projected to Rust, JSON Schema, SHACL and CEL; opt-in per chart. OpenSpec: `type-system`.

- scheduled_in:: [[roadmap#M1 Typed authoring]]

## Rust DSL

The `statechart!` and `include_chart!` macros with compile-time checks, and the `rust` datamodel of typed context and named guard and action functions. OpenSpec: `rust-dsl`.

- scheduled_in:: [[roadmap#M1 Typed authoring]]

## YAML format

A YAML surface for charts that maps one to one onto the IR, with a `types:` block and an ignored `layout:` block for a designer's layout. OpenSpec: `yaml-format`.

- scheduled_in:: [[roadmap#M1 Typed authoring]]

## CEL datamodel

A side-effect-free, terminating CEL datamodel for YAML charts and LLM-written charts, type-checked against the chart's types. OpenSpec: `cel-datamodel`.

- scheduled_in:: [[roadmap#M1 Typed authoring]]

## Actor model

The `Actor` trait, IRI addresses and the three lifecycle classes (virtual, invoked, stateless worker), mapped onto `<send>` and `<invoke>`, with supervision extensions. OpenSpec: `actor-model`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Messaging

Pluggable mailboxes and transports, address resolution, `vt:request`, `vt:defer` and the unhandled-event policy with dead letters. OpenSpec: `messaging`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Event journal

Event-sourced actor state: journal entries, compare-and-set append, deduplication, transactional outbox, snapshots and deterministic replay. OpenSpec: `event-journal`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Storage

The `JournalStore` and `GraphStore` traits, the tiramemsu backend implementing both, the Oxigraph and SPARQL `GraphStore` adapters, and the RDF 1.1 fallback. OpenSpec: `storage`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Durable timers

Delayed sends and reminders recorded in the journal and fired as input events, reactivating virtual actors when due. OpenSpec: `durable-timers`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Chart versioning

Content-addressed chart versions pinned in journals, pin-until-final policy, and explicit migrations recorded as journal entries. OpenSpec: `chart-versioning`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Runtime ontology

The `vt:` runtime module (actor, address, mailbox, journal entry, macrostep, snapshot, migration) with SHACL shapes and PROV-O alignment. OpenSpec: `runtime-ontology`.

- scheduled_in:: [[roadmap#M2 Actor runtime]]

## Deciders

`vt:decide` states and the pluggable `Decider` trait choosing among enabled events, with typed payload schemas and journaled decisions. OpenSpec: `deciders`.

- scheduled_in:: [[roadmap#M3 Agents]]

## LLM actors

LLM calls as invoked actors and deciders, tool actors, capability grants, the agent ontology module, and journaled model results. OpenSpec: `llm-actors`.

- scheduled_in:: [[roadmap#M3 Agents]]

## Plan charts

Chart fragments written by an LLM, validated, restricted to the CEL datamodel, content-addressed and run as sandboxed invoked children. OpenSpec: `plan-charts`.

- scheduled_in:: [[roadmap#M3 Agents]]

## Cluster membership

The pluggable `Membership` trait with static, gossip and external implementations, and shard leases. OpenSpec: `cluster-membership`.

- scheduled_in:: [[roadmap#M4 Cluster]]

## Shard placement

Hashing actor IRIs to shards, one journal file per shard, single activation, and rebalancing by file shipping or lease transfer. OpenSpec: `shard-placement`.

- scheduled_in:: [[roadmap#M4 Cluster]]

## Cluster transport

The QUIC transport between trusted cluster nodes, routing messages to the node that owns a shard. OpenSpec: `cluster-transport`.

- scheduled_in:: [[roadmap#M4 Cluster]]

## Worker pools

Stateless worker activations pooled across nodes with load-balanced delivery. OpenSpec: `worker-pools`.

- scheduled_in:: [[roadmap#M4 Cluster]]

## DID identity

DID-backed actor identities, the DID resolver for address resolution, authenticated `_event.origin`, and capability grants across trust boundaries. OpenSpec: `did-identity`.

- scheduled_in:: [[roadmap#M5 Federation]]

Implemented in [[components#vertep-federation]]: every `did-identity` requirement carries `@tg: implements::` annotations in code and every scenario a `@tg: verifies::` test, most of them on runtime nodes exchanging events through a test mediator.

## Relay mailboxes

DIDComm v2 and TSP mediators as mailboxes and transports, with routed envelopes through intermediaries and end-to-end encryption. OpenSpec: `relay-mailboxes`.

- scheduled_in:: [[roadmap#M5 Federation]]
- scheduled_in:: [[roadmap#M7 Durable objects]] {note: "TSP Mediator Mailbox and Live Delivery Over WebSocket"}

Implemented in [[components#vertep-federation]] on the runtime's `Resolver`, `Mailbox` and `Transport` traits, with TSP interop against the reference wallet. Interop with a third-party DIDComm mediator runs against Identus in Docker. Since M7, a TSP actor's mailbox can live at a [[components#vertep-tsp-mediator]] through `TspMailboxClient`. Both DIDComm and TSP mailboxes take live sessions (`vertep_federation::live`), and acknowledgement follows the journal append either way.

## WASM bindings

The core and a browser shell in WebAssembly with a TypeScript API, generated typings and a tiramemsu journal in OPFS. OpenSpec: `wasm-bindings`.

- scheduled_in:: [[roadmap#M6 WASM and JS]]

## Durable objects

DID-addressed, single-writer virtual actors whose hash-chained, signed journal outlives any host; durable alarms, activation from verified replicas, the durable agent journal and lineage. OpenSpec: `durable-objects`.

- scheduled_in:: [[roadmap#M7 Durable objects]]

## Object custody

Custodian sets of hosts and keepers named in the DID document, epochs won by quorum promises, replication over TSP, encrypted keeper replicas, handoff, failover and equivocation evidence. OpenSpec: `object-custody`.

- scheduled_in:: [[roadmap#M7 Durable objects]]

## TSP mediator

A TSP 1.0 routed-mode intermediary with durable store-and-forward mailboxes, a pickup protocol inside TSP, live WebSocket sessions, quotas and metadata minimisation. OpenSpec: `tsp-mediator`.

- scheduled_in:: [[roadmap#M7 Durable objects]]

Implemented in [[components#vertep-tsp-mediator]] (library and `vt-mediator` binary) with the client in [[components#vertep-federation]] (`tsp_mailbox`, `live`). Every requirement carries `@tg: implements::` and every scenario a `@tg: verifies::` test. The tests cover:

- interop with the reference wallet in process;
- a replicated deployment on M4 standby replication;
- the binary over real HTTP and WebSocket sockets;
- an opt-in Docker test of two mediators in a route.

The architecture is in [[views/distribution#Distribution#TSP mediator]].

## webvh identity

Creating `did:webvh` DIDs from Vertep identities with key-manager keys, updates, pre-rotated key rotation, deactivation, custodians witnessing their object's log, and custody changes published as witnessed log entries. OpenSpec: `webvh-identity`.

- scheduled_in:: [[roadmap#M8 did:webvh hosting]]

Implemented in [[components#vertep-webvh]]; the custody side of witnessing is in [[components#vertep-objects]].

## webvh hosting

The `did:webvh` 1.0 file layout, directory, git and command publishers, serving logs over HTTP, the `vt webvh` commands, `vt-mediator` under `did:webvh`, and interop with third-party logs. OpenSpec: `webvh-hosting`.

- scheduled_in:: [[roadmap#M8 did:webvh hosting]]

Implemented in [[components#vertep-webvh]], [[components#vertep-cli]] and [[components#vertep-tsp-mediator]].
