---
type: Decision
status: accepted
date: 2026-11-11
owner: Volodymyr Pavlyshyn
---
# Decisions

Architecture decisions for Vertep, a Rust family of crates for statecharts, actors and agents on RDF, taken in the design session of 2026-11-11.

## SCXML semantics are normative

The runtime executes the W3C SCXML interpretation algorithm; every other format is a dialect lowered onto it, with declared loss.

- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: primary, since: "2026-11-11"}

Rejected: inventing semantics as XState does, which loses the conformance suite as a test oracle and any honest claim of interop. Vertep features live in `vt:` extension namespaces so plain SCXML stays portable.

## IR is the hub

A typed Rust IR is the canonical model; SCXML, YAML and the `statechart!` macro lower into it, and RDF is a lossless projection with IR to RDF to IR as identity.

- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#RDF is open-world]] {weight: secondary, since: "2026-11-11"}
- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: secondary}

Rejected: RDF as the source of truth read by the runtime. Document order is kept with explicit `vt:order`, and SHACL shapes define a valid chart in RDF terms.

## Pluggable datamodel

Executable content goes through a `Datamodel` trait with `rust`, `ecmascript` (boa), `null` and CEL implementations; guards are pure and effects happen only through actions that emit events.

- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary, note: "pure guards make replay deterministic"}

The macro DSL uses `rust`, imported SCXML uses `ecmascript`, YAML charts and LLM-written charts use CEL.

## SCXML-native actors with vt extensions

An actor is anything that can be a target of `<send>` and `<invoke>`; a running chart is the main kind, and Vertep extensions add supervision, virtual actors and remoting.

- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: primary, since: "2026-11-11"}

Rejected: a general actor framework with charts as one behaviour, which would need two lifecycle and addressing models. Rust functions, streams, LLM calls and tools implement the same `Actor` trait.

## IRI identity and lifecycle classes

Every actor address is an IRI, and actors belong to one of three lifecycle classes: virtual, invoked and stateless worker.

- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: primary, since: "2026-11-11", note: "the address is the RDF subject of state and history"}

A [[glossary#Virtual actor]] is never created or destroyed and is reached only by messages. An [[glossary#Invoked actor]] lives while its invoking state is active. A [[glossary#Stateless worker]] is pooled and keeps no journal. One Orleans turn equals one SCXML macrostep.

## Identity, mailbox and transport layers

Identity, mailbox and transport are three separately pluggable layers: an IRI resolves to mailboxes, and mailboxes deliver over transports that are SCXML Event I/O Processors.

- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary, since: "2026-11-11"}
- motivated_by:: [[decisions#IRI identity and lifecycle classes]] {weight: primary}

A [[glossary#Mailbox]] outlives activations and can be a local queue, a store-backed queue, or a DIDComm or TSP mediator holding messages for many actors. DIDs are used only for actors that cross a trust boundary; there `_event.origin` becomes an authenticated sender.

## Event-sourced actors

A virtual actor's state is a deterministic replay of its journal of accepted inputs, with snapshots, deduplication by message id and a transactional outbox.

- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary, since: "2026-11-11"}

Delivery is at least once and processing is effectively once. LLM results are journaled as input events so replay never calls the model again. Rejected: Orleans-style snapshot persistence with idempotent handlers.

## Two storage traits with tiramemsu as default

Storage is two RDF-agnostic traits, `JournalStore` with per-actor compare-and-set append and `GraphStore` for RDF and SPARQL, and tiramemsu implements both by default.

- motivated_by:: [[constraints#SPARQL Update has no compare-and-set]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: secondary}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary}

Tiramemsu is append-only and bitemporal, so an input event and the state it causes commit in one transaction: `now()` is the current-state graph, `as_of` is time travel, and derived facts carry support links to their event. Oxigraph and SPARQL endpoints get a `GraphStore` adapter; a networked journal comes later for clusters.

## Content-addressed chart versions

A chart version is the IRI of the hash of its RDFC-1.0 canonical RDF, every journal segment records its version, actors stay pinned until final, and upgrades are explicit migrations written as journal entries.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}

Rejected: replaying old journals against the latest chart, which breaks determinism.

## Deciders at choice points

A `vt:decide` state hands the enabled events to a pluggable `Decider`, which may be guards, an LLM, a human or another actor; whatever it decides arrives as a journaled event.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}

The LLM is one option, not a special case: it can be an invoked actor, a narrow decider, the main driver of an agent loop, or the author of sub-charts and new chart versions through migrations. A [[glossary#Decider]] can only choose an enabled transition.

## Modular vt ontology on RDF 1.2

Vertep has its own modular ontology (chart, runtime, identity, agent), each module with SHACL shapes, on RDF 1.2, aligned to PROV-O and DID Core without depending on them.

- motivated_by:: [[constraints#RDF is open-world]] {weight: primary, since: "2026-11-11"}

RDF 1.2 triple terms annotate transitions and decisions and map to tiramemsu statement layers. `GraphStore` adapters for RDF 1.1 stores fall back to classic reification.

## One IR type language

Event payloads and context use one algebraic type language in the IR, projected to Rust types, JSON Schema, SHACL shapes and a CEL type environment; typing is opt-in per chart.

- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: secondary, since: "2026-11-11"}
- motivated_by:: [[decisions#Deciders at choice points]] {weight: primary, note: "LLM structured output and chart share one schema"}

`include_chart!` gives SCXML and YAML charts the same typed Rust API as `statechart!`. Untyped charts run on a dynamic value type so the conformance suite still passes.

## Lean verification bridge

A Lean package on LeanRDF models the type system, the semantics and the journal protocol with proofs; the executable Lean `step` is a differential oracle for the Rust core, and Lean-checked certificates are stored in the graph.

- motivated_by:: [[constraints#Aeneas supports a Rust subset]] {weight: secondary, since: "2026-11-11"}
- motivated_by:: [[decisions#One IR type language]] {weight: primary}

Target theorems: type soundness, deterministic and well-formed `step`, SHACL shapes equivalent to the Lean well-formedness predicate, the RDF round trip, journal replay and deduplication, and decider safety. Rejected for now: proving the Rust code with Aeneas from day one; it stays possible later for the pure kernel.

## No-std core and tokio shell

The core (IR, types, `step`, datamodel trait) is `no_std` with `alloc` and has no I/O; the runtime shell uses tokio behind a small spawn, clock and sleep trait.

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary, note: "time enters only as journaled input"}

Delayed sends and reminders are durable timers written in the journal. WASM and JS bindings are a later target.

## Shard-based placement

In a cluster an actor's shard is the hash of its IRI, shards are leased to nodes through a pluggable `Membership` trait, and one shard is one journal file with one writer.

- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: secondary}

The shard lease and the journal compare-and-set both guard single activation. Rejected: an Orleans per-actor directory. A [[glossary#Shard]] moves by shipping its append-only file now and by moving a lease once the journal is networked.

## Non-blocking request and deferred events

A chart never blocks: `vt:request` expands to a send with a deterministic correlation id, a waiting state and a timeout; `vt:defer` keeps events in the mailbox; unhandled events follow a per-chart policy.

- motivated_by:: [[constraints#SCXML discards unmatched events]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary, note: "keeps step pure"}

The unhandled-event policy is `discard`, `dead-letter` or `error`, with dead-letter as the default for virtual actors. A blocking `ask` exists only for callers outside the actor system.

## Full SCXML first

Milestone 0 is the complete SCXML core with the W3C conformance suite, the Lean model of the full semantics as an oracle, and IR to RDF with SHACL; runtime, agents, cluster and federation follow.

- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: primary, since: "2026-11-11"}

A throwaway spike of the journal with an LLM decider runs alongside Milestone 0 to surface integration risk early. Rejected: a tracer-bullet slice through every layer first.

## TSP before DIDComm across trust boundaries

The first cross-trust transport is TSP; DIDComm v2 follows as a configurable alternative behind the same mailbox and transport traits. Ratified 2026-11-11 from the M5 spec.

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}

TSP is smaller and keeps the relay role thin; DIDComm stays selectable per peer.

## Fencing epochs on journal appends

Each shard lease carries a monotonically increasing epoch that is checked on every journal append and every outbound frame, so a node that lost its lease cannot leak side effects. Ratified 2026-11-11 from the M4 spec.

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: secondary}

The lease and the compare-and-set append remain; the epoch closes the window in which a stale node still holds an open file.

## Optional standby journal replication

A shard may have a standby that tails its journal for failover; without one, a failed node's shards stay unavailable until it returns or its file is shipped. Ratified 2026-11-11 from the M4 spec.

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}

Replication is opt-in so a single node or a small cluster pays nothing for it.

## Power-of-two shard counts

The shard count is a power of two, so resharding is always a split of each shard into two and an actor's new shard is derivable from its old one. Ratified 2026-11-11 from the M4 spec.

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}

## Grants enforced before the journal

Traffic from a peer without a grant for the target actor is refused at the mailbox, before any journal append, so ungranted input never becomes part of an actor's history. Ratified 2026-11-11 from the M5 spec.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: secondary}

## Offer-time decider safety

A decider's answer is safe when it names an event that was enabled when the choice was offered; when the enabled set changes the offer is re-issued. Ratified 2026-11-11 from the M3 spec.

Each offer has a fingerprint of its enabled set, and `vt.decide.none` is always a legal answer.

- motivated_by:: [[decisions#Deciders at choice points]] {weight: primary, since: "2026-11-11"}

## LLM calls as invoked actors first

In M3 an LLM call is an invoked actor of the deciding chart; pooled stateless LLM workers arrive with the M4 worker pools. Ratified 2026-11-11 from the M3 spec.

- motivated_by:: [[decisions#IRI identity and lifecycle classes]] {weight: primary, since: "2026-11-11"}

## Plan charts may use the null datamodel

LLM-authored plan charts run in a sandbox with either the CEL or the `null` datamodel; ECMAScript and Rust datamodels are never available to a plan. Ratified 2026-11-11 from the M3 spec.

- motivated_by:: [[decisions#Pluggable datamodel]] {weight: primary, since: "2026-11-11"}

## Appendix D readings in the Rust core

Where the SCXML Recommendation leaves room, `vertep-core` reads it as follows; the Lean model must make the same choices, and a change here is a semantic change.

- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: secondary}

- An unsupported send type or an illegal target raises `error.execution` (W3C 6.2.4, IRP test 199); `error.communication` is reserved for delivery failures the host reports back as input.
- A failing `<if>` condition queues `error.execution` and stops the enclosing block; later branches are not tried.
- `done.state.*` and error events are platform events with undefined data; errors from a failed send carry its `sendid`.
- Generated send ids are `<sessionid>.send.<n>`, invoke ids `<stateid>.<n>`; `idlocation` is assigned before anything else in the send is evaluated.
- A delayed `#_internal` send becomes a `Send` effect that the host delivers back.
- Events whose `invokeid` matches no active invocation are discarded; `done.invoke.<id>` completes the invocation, so leaving the state emits no `CancelInvoke`.
- A history state without a default transition gets one to its parent's default entry; the root's initial transition is internal with the root as domain.
- `SessionDone` is emitted exactly once, also on cancel; a `<state>` whose only children are history states is rejected by validation.

## IRP suite from a pinned mirror

The W3C IRP files are imported from the verbatim copy in the uscxml repository pinned by commit, transformed with `confEcma.xsl` on SaxonJS, and committed with a SHA-256 manifest that CI checks.

- motivated_by:: [[constraints#W3C serves a bot challenge to scripted clients]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#The IRP stylesheet is XSLT 2.0]] {weight: primary}
- motivated_by:: [[decisions#Full SCXML first]] {weight: secondary}

The mirror's `update-txml.sh` copies the W3C files unchanged; the only local edit it carries is commented out. `crates/vertep-conformance/scripts/import-irp.sh` re-imports, `check-suite-hash.sh` verifies.
