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

- An unsupported send type or an illegal target raises `error.execution` (W3C 6.2.4, IRP test 199). When the input lists the running sessions, an immediate send to `#_scxml_<id>` of an unlisted session queues `error.communication` with its `sendid` at once and the block continues (IRP 496); every other undeliverable target is reported by the host as an `error.communication` input.
- A failing `<if>` condition queues `error.execution` and stops the enclosing block; later branches are not tried.
- `done.state.*` events are platform events with undefined data. Error events are platform events whose data is `{kind, message}`: `kind` is the name after `error.`, and `message` is the failing datamodel call's error, or the fixed `execution error` / `target session does not exist` when the core raises it itself. Errors from a failed send carry its `sendid`.
- Generated send ids are `<sessionid>.send.<n>`, invoke ids `<stateid>.<n>`; `idlocation` is assigned before anything else in the send is evaluated.
- A delayed `#_internal` send becomes a `Send` effect that the host delivers back.
- The core processes every delivered event; only active invocations run `finalize` or get autoforwarded events, and the host stops delivering a cancelled invocation's events from the `CancelInvoke` on (W3C 6.4, IRP 236). `done.invoke.<id>` completes the invocation, so leaving the state emits no `CancelInvoke`.
- Invoker `param` and `namelist` values arrive as start data and override the initialization of top-level data items with those names (W3C 6.4.4).
- A history state without a default transition gets one to its parent's default entry; the root's initial transition is internal with the root as domain.
- `SessionDone` is emitted exactly once, also on cancel; a `<state>` whose only children are history states is rejected by validation.
- A history target stands for its parent's position in the target-set rule, and cycles through history default transitions are rejected (found by the Lean model).

## IRP suite from a pinned mirror

The W3C IRP files are imported from the verbatim copy in the uscxml repository pinned by commit, transformed with `confEcma.xsl` on SaxonJS, and committed with a SHA-256 manifest that CI checks.

- motivated_by:: [[constraints#W3C serves a bot challenge to scripted clients]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#The IRP stylesheet is XSLT 2.0]] {weight: primary}
- motivated_by:: [[decisions#Full SCXML first]] {weight: secondary}

The mirror's `update-txml.sh` copies the W3C files unchanged; the only local edit it carries is commented out. `crates/vertep-conformance/scripts/import-irp.sh` re-imports, `check-suite-hash.sh` verifies.

## Configuration well-formedness proof is pending

The theorem that `step` preserves legal configurations is stated only in `lean/vertep-lean/Pending/`, outside the build, because the Lean model found two charts that pass validation and break it.

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: secondary}

The counterexamples: a transition targeting a history state and its sibling, which leaves two active children; and an `initial` naming a history state without a default transition, which recursed without bound in Rust. Both are now validator rules in Rust (2026-11-11), to be mirrored in `WellFormed`; the proof then needs lemmas for compile correctness, exit and entry legality, a history invariant and fuel adequacy. No theorem is stated with `sorry` meanwhile.

## Journal entries are stamped by the store

Every append adds `rdf:type vt:JournalEntry`, `vt:actor` and `vt:position` to each entry graph and rejects an entry that states another position or actor, so entries read back are self-describing.

- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: primary, since: "2026-11-11"}

## State changes travel in the append

An append may carry a state delta for the actor's current-state graph; stores that declare `state_in_append` commit it in the same transaction, on tiramemsu as derived statements supported by the entry.

- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary}

A state already active is not re-derived, so it keeps its first cause, and `state_causes` walks back from any state triple to the entries that produced it.

## Stores without fencing refuse tokens

A journal store that does not enforce fencing fails an append carrying a fencing token with `Unsupported` instead of ignoring it, and a failed append never records its epoch.

- motivated_by:: [[decisions#Fencing epochs on journal appends]] {weight: primary, since: "2026-11-11"}

## Single-node shards placed by FNV-1a of the address IRI

On one node an actor lives in shard `fnv1a64(IRI) mod n`, one tiramemsu file `shard-i-of-n.db` each; reopening a directory with another `n` is refused.

- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Shard-based placement]] {weight: secondary}

Cross-shard queries go to the projected external graph store; one projector runs per shard.

## Remote SPARQL endpoints default to RDF 1.1

A remote endpoint's RDF version is declared by the application and defaults to RDF 1.1, so annotations use `rdf:Statement` reification that every endpoint understands.

- motivated_by:: [[constraints#SPARQL Update has no compare-and-set]] {weight: secondary, since: "2026-11-11"}
- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: primary}

## Types ride on vt extension nodes

The IR has no typed fields: `vt:type` on data and send is an extension attribute, param types are one `vt:paramTypes` record on the owner, and declarations are a `vt:types` element on the chart.

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IR is the hub]] {weight: secondary}

The declaration text is `type X = T; event e: T;`. Untyped charts carry none of these and run exactly as before, which every conformance chart confirms.

## Datamodel typers plug into one core checker

`check_chart` takes an `ExprTyper`; the core ships a basic typer for literals and `ctx`/`ev.data` paths, and the CEL and Rust datamodels supply full expression typing, so every surface calls one checker.

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary}

## YAML on saphyr-parser with a self-verifying writer

`vertep-yaml` reads with saphyr-parser's YAML 1.2 event API into an ordered, positioned node tree; the writer uses shorthands only when they lower back to the same IR node and reads its own output back before returning.

- motivated_by:: [[decisions#IR is the hub]] {weight: primary, since: "2026-11-11"}

Rejected: serde-based YAML crates, which lose positions or key order or resolve scalars the YAML 1.1 way, turning the `on:` transition key into `true`.

## Own CEL interpreter in vertep-dm-cel

Vertep evaluates CEL with its own `no_std` interpreter: a bounded-depth parser, a gradual static checker, a step-budgeted evaluator, exact decimals, key-ordered maps, integer time and linear-time regex.

- motivated_by:: [[decisions#Pluggable datamodel]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: secondary}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary, note: "deterministic replay"}

A probe on 2026-11-11 rejected `cel` 0.15 (hash-ordered maps gave five key orders in five runs, no decimal, no step limit, needs std), `cel-core` 0.5 (no decimal, no limits, stack overflow on deep nesting, needs std), `rscel` 1.0.8 (nightly only, non-deterministic `now()`) and `cel-cxx` (C++ FFI, no wasm, no decimal). Decimals hold 34 significant digits; more is an overflow error.

## Session state is a value and the datamodel a component

`step` takes the compiled chart, a session value, one input and the datamodel, and returns a new session and its effects; the datamodel keeps its own mutable state behind the trait (M0 design D1).

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary}

Rejected: an immutable datamodel value, which would copy the ECMAScript heap every microstep, and an async `step`, which would break purity and the Lean correspondence.

## Compiled charts are derived and never serialized

Validation turns a well-formed IR into a compiled chart with states numbered in document order, ancestor tables and precomputed domains; only the IR is ever serialized or hashed (M0 design D2).

- motivated_by:: [[decisions#IR is the hub]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: secondary}

## Every interaction is an effect

The step emits `Send`, `Cancel`, `Invoke`, `CancelInvoke`, `Forward`, `Log` and `SessionDone` effects in document order, and delays are durations in effects, so one kernel runs under the test host, the runtime and the Lean oracle (M0 design D3).

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary}

## Guard purity is enforced by the datamodel

A datamodel evaluates guards against a read-only view whose writes fail, so an impure guard is an evaluation error and false; ECMAScript also seeds `Math.random` and reads `Date` from the input (M0 design D4).

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11", note: "replay must be sound"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary}

Rejected: snapshot-compare after every guard as the main mechanism, which costs a heap walk; it remains a debug backstop.

## The Lean model sees the datamodel as an answer tape

The Rust run records every datamodel request and answer; the Lean `step` consumes the tape and must issue identical requests, and evaluates `In()` itself, so its theorems hold for every datamodel (M0 design D5).

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}

## Charts cross to Lean as N-Quads

The oracle and the certifier read a chart as the N-Quads of its RDF projection, decoded by the Lean reconstruction the round-trip theorem covers, and traces are JSON Lines with one record per macrostep (M0 design D6).

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: secondary}

Rejected: a separate JSON chart format for Lean, a second, unproven codec.

## Persistent vt namespace and blank-node-free charts

The namespace is `https://w3id.org/vertep/vt#`, chart versions are `urn:vertep:chart:sha256:<hex>`, and node IRIs are minted under the version with no blank nodes, so canonicalization stays linear (M0 design D7).

- motivated_by:: [[decisions#Content-addressed chart versions]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#RDF is open-world]] {weight: secondary}

## Document order is explicit in RDF

Every ordered member of a projected chart carries `vt:order`; `rdf:List` is not used, because transition priority is document order and lists are awkward in SPARQL and fragile under partial updates (M0 design D8).

- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: primary, since: "2026-11-11"}

## Shapes are SHACL Core over derived triples

Whole-chart rules are SHACL Core over triples the projection materializes, never SHACL-SPARQL, and their equivalence with `WellFormed` is a Lean theorem and an empirical test (M0 design D9).

- motivated_by:: [[constraints#RDF is open-world]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary}

## Conformance variants are generated once and pinned

IRP templates become `ecmascript` and `null` charts once, by a script; the results and their hashes are committed, every expected deviation has a reason, and an unexpected pass fails (M0 design D10).

- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IRP suite from a pinned mirror]] {weight: secondary}

## Minimal dependencies per M0 crate

`vertep-core` uses only `alloc` (its bitset is in-crate), `vertep-scxml` quick-xml, `vertep-dm-ecma` boa, `vertep-rdf` oxrdf and oxttl with RDF 1.2 and RDFC-1.0, and `vertep-lean` a pinned LeanRDF (M0 design D11).

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: secondary}

## No throwaway journal spike

The planned M0 spike of a tiramemsu journal with an LLM decider was not built, because M2 was implemented directly on the finished core and storage, which answered the same integration questions with real code (replaces M0 design D13).

- motivated_by:: [[decisions#Full SCXML first]] {weight: primary, since: "2026-11-11"}
