---
type: Capability
status: draft
openspec: [chart-model, scxml-interpretation, scxml-format, datamodel, chart-rdf, formal-model, scxml-conformance, type-system, rust-dsl, yaml-format, cel-datamodel, actor-model, messaging, event-journal, storage, durable-timers, chart-versioning, runtime-ontology, deciders, llm-actors, plan-charts, cluster-membership, shard-placement, cluster-transport, worker-pools, did-identity, relay-mailboxes, wasm-bindings, durable-objects, object-custody, tsp-mediator, webvh-identity, webvh-hosting, agent-wallet, credential-exchange, oid4vc, agent-protocols, protocol-conformance, protocol-spaces, python-bindings, decision-records, journal-redaction, production-readiness, journal-shipping, rule-programs, rule-decider, rule-invariants, derived-facts, rule-verification, rule-authoring]
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

Mailbox backends are chosen per address by ordered rules, in code (`RoutedMailboxes`) or from a JSON file (`MailboxConfig`, checked offline by `vt mailbox check`); specified by the change `mailbox-backends`. Receive cost under deferral, bounded mailboxes and durability by origin are planned in [[roadmap#M18 Runtime performance]].

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

## Python bindings

Charts, actors, journals, time travel and SPARQL from Python, with Python guards and actions, installed with `pip install vertep`. OpenSpec: `python-bindings`.

- scheduled_in:: [[roadmap#M11 Python bindings]]

Specified by the change `m11-python-bindings`; built on the command bridge of [[capabilities#WASM bindings]], so the semantics, the journal format and the error codes are the JavaScript bindings'.

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

## Agent wallet

A wallet per identity: a sealed credential store, holder-binding keys, five plugin interfaces, credential formats, consent as a decide state, status lists, lifecycle and `vt wallet`. OpenSpec: `agent-wallet`.

- scheduled_in:: [[roadmap#M9 Agent wallet]]

Implemented in [[components#vertep-wallet]] and [[components#vertep-cli]].

## Credential exchange

One exchange vocabulary for every protocol, role charts, the native protocol over TSP, the Lean exchange model with its theorems and checker, and the plugin conformance kit. OpenSpec: `credential-exchange`.

- scheduled_in:: [[roadmap#M9 Agent wallet]]

Implemented in [[components#vertep-wallet]] and [[components#vertep-lean]].

## OID4VC

OpenID4VCI issuance with the pre-authorised code flow and OpenID4VP presentation with DCQL and `direct_post`, as exchange-protocol plugins for issuers, holders and verifiers. OpenSpec: `oid4vc`.

- scheduled_in:: [[roadmap#M9 Agent wallet]]

Implemented in [[components#vertep-wallet]] behind feature `oid4vc`.

## Agent protocols

Role-based protocols as restricted JSON-LD documents: hash identity, payload shapes, well-formedness, role projection, signed offers, DID-document discovery and the RDF bridge. OpenSpec: `agent-protocols`.

- scheduled_in:: [[roadmap#M10 Agent protocols]]

Implemented in [[components#vertep-kolyada]]: every requirement carries `@tg: implements::` and every scenario a `@tg: verifies::` test, including the agreement of the payload checker with full SHACL. The design is in [[views/kolyada#Kolyada]].

## Protocol conformance

The signed message envelope, the pure `accept` check, causal delivery, violation and equivocation proofs, language-neutral test vectors and the Lean protocol model. OpenSpec: `protocol-conformance`.

- scheduled_in:: [[roadmap#M10 Agent protocols]]

Implemented in [[components#vertep-kolyada]], with the Lean protocol model in `lean/vertep-lean` (`Protocol*.lean`) replaying recorded Rust histories strictly. The vectors are checked in Rust, with `vt kolyada vectors`, and, for envelopes, with Node alone.

## Protocol spaces

Protocol instances shared by agents: signed genesis, countersigned joins, join policy, role-derived authority, sealed fan-out, hash-linked history, sub-protocols and the chart and mailbox binding. OpenSpec: `protocol-spaces`.

- scheduled_in:: [[roadmap#M10 Agent protocols]]

Implemented in [[components#vertep-kolyada]] (instances, joins, role-derived authority, the hash-linked history, sub-protocol genesis) and [[components#vertep-federation]] (the join policy, pairwise sealed fan-out over the relays, and the chart and mailbox binding through `vt:plays`). A `SpaceMember` still keeps its own journal beside the actor's.

## Decision records

The journal alone explains every decision: disposition and offer on each decision entry, a derived explanation view, a bundle verified offline, chained agent journals and counterfactual replay. OpenSpec: `decision-records`.

- scheduled_in:: [[roadmap#M12 Decision records]]

Implemented in [[components#vertep-agent]], [[components#vertep-objects]] and [[components#vertep-cli]]: every decision and refusal entry carries a disposition, the offer and its outcome; `AgentHost::explain` derives an explanation by replay with the rationale kept as a labelled claim and a rule derivation shown as a derivation; `AgentHost::export_bundle`, `Bundle::verify`, `vt explain` and `vt verify-bundle` export and check a bundle offline, and `@vertep/web` and the Python package verify bundles too; `ChainedAgentSealer` chains and signs agent journals; `AgentHost::counterfactual` replays under another chart version. Dispositions replay strictly through the Lean model. See [[views/decision-records]].

## Journal redaction

Sensitive fields sealed under per-subject keys, erasure by key destruction that keeps the journal valid, replay marked `recorded` after erasure, and retention policy. OpenSpec: `journal-redaction`.

- scheduled_in:: [[roadmap#M12 Decision records]]

Implemented in [[components#vertep-agent]] (`redaction`) and [[components#vertep-rules]] (`erasure`): `SealingJournal` seals the fields a `SealPolicy` names, derived from `@sensitive`/`@subject`/`@retain` marks by `SealPolicy::from_chart`, under per-subject ChaCha20-Poly1305 keys behind a `KeyStore`; `erase` destroys the key, retracts rule-derived facts through an `ErasureHook` and appends an `erasure` entry listing them; `expire` erases past a retention horizon. Entries record their outcome, so replay after an erasure follows it and is marked `recorded`. Limits: datamodel marks are not sealed, history in the store is not purged, and values computed from erased data outside guards are not restored. See [[constraints#Erasure rules]] and [[decisions#Snapshots never hold opened sealed values]].

## Production readiness

Stability tiers, a reproducible toolchain, a supply-chain gate, a docs-versus-behaviour audit and a tested ten-minute path. OpenSpec: `production-readiness`.

- scheduled_in:: [[roadmap#M13 Production readiness]]

Implemented: tiers in every manifest rendered by `site/tools/tiers.py`, `scripts/build.sh` with the pinned container, `deny.toml` with SBOMs and signed provenance at release, `scripts/docs-audit.py`, and `scripts/ten-minute-path.py`, each checked in CI. The benchmark suite and its gate moved to M18 (`m18-runtime-performance`).

## Runtime performance

Benchmarks and counted-operation budgets for the hot paths, a regression gate, and bounded memory and storage over a long run. OpenSpec: `runtime-performance`.

- scheduled_in:: [[roadmap#M18 Runtime performance]]

Not implemented: the change `m18-runtime-performance` is a draft. The review behind it is in [[views/performance#Performance]].

## Journal shipping

A pull protocol that replicates a journal between cluster nodes in order with digests, resumes after interruption and bounds memory under a slow follower. OpenSpec: `journal-shipping`.

- scheduled_in:: [[roadmap#M13 Production readiness]]

Implemented in [[components#vertep-cluster]] (`shipping`): a follower pulls entries after its durable head; the leader answers in order with SHA-256 digests under its fencing token, over QUIC or in-process. Resumption after a dropped link or a restart and a byte window per pull are tested, and fault histories replay strictly through the Lean lease model. Not yet wired into `ClusterNode` standby replication or shard moves; it closes the gap noted in [[roadmap#M4 Cluster]].

## Rule programs

Rule programs stored in the shard beside the journal, versioned by supersede, pinned by digest and engine version on every decision, bounded by fuel, and available only where the native engine is. OpenSpec: `rule-programs`.

- scheduled_in:: [[roadmap#M14 Rules decider]]

Implemented in [[components#vertep-rules]]: storage, supersede, closure digest, engine pinning, fuel and the typed unavailable error; the browser-build check is a dependency-graph gate.


## Rule decider

A decider whose answer is a derivation over the offer and state written as statements; its support is the rationale, no applicable rule is a refusal, bundles carry the program and counterfactuals vary its version. OpenSpec: `rule-decider`.

- scheduled_in:: [[roadmap#M14 Rules decider]]

Implemented in [[components#vertep-rules]]: the decider, derivation support, refusals, bundles that verify offline and counterfactual replay; the entry field and the whole-bundle verification wait on M12.


## Rule invariants

Enforced rule programs that refuse a violating journal append at commit, with the refusal dead-lettered, a full check on enabling and a delta check proven equal to it. OpenSpec: `rule-invariants`.

- scheduled_in:: [[roadmap#M15 Rule invariants]]

Implemented in [[components#vertep-rules]]: `enforce`, the refusal path through a running node (dead letter, no entry), the full check on enabling and the delta-versus-full property test.


## Derived facts

Facts concluded by rules and kept in the actor's graph with their support, retracted with it and with erasure. OpenSpec: `derived-facts`.

- scheduled_in:: [[roadmap#M15 Rule invariants]]

Implemented in [[components#vertep-rules]]: `materialize` writes derived facts with support into the graph `<subject>/derived`; they fall with their support and with the program. Retraction on erasure is a requirement of [[capabilities#Journal redaction]].


## Rule verification

A rules profile, totality and support-necessity checks, a differential oracle against Datalog and derivation certificates checked in Lean. OpenSpec: `rule-verification`.

- scheduled_in:: [[roadmap#M16 Verified rules]]

Implemented in [[components#vertep-rules]]: the rules profile, support necessity, sampled totality, the Datalog oracle and Lean certificates for clauses of `tm.triple` atoms and integer comparisons.


## Rule authoring

Agent-proposed rules behind an ordered gate with structured errors, shadow evaluation, staged journaled promotion and a refusal of self-governance. OpenSpec: `rule-authoring`.

- scheduled_in:: [[roadmap#M17 Agent rules]]

Implemented in [[components#vertep-rules]] (`authoring`, `tools`): the gate with structured errors, authorship, shadow runs, staged journaled promotion, scoped-live deciders, self-governance, diff and the agent tool.
