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
- The microstep budget charges every microstep record: selections that fire transitions and internal events consumed without enabling any, so a failing eventless guard that keeps raising `error.execution` ends in `BudgetExceeded` (found by differential testing).
- Transition domains are computed once per microstep, before `exitStates` records history, and entry uses the same domains; Appendix D recomputes them from the history just recorded, which left a single-region parallel state inactive under its active region (found by the configuration proof).
- Entry never enters a state that is not a proper descendant of the transition's domain: Appendix D's `addAncestorStatesToEnter(s, history.parent)` re-entered active states above a domain lying below the history's parent and completed their parallel ancestors' other regions next to the regions still active, leaving two active children (found by the configuration proof, twice: a history default inside a parallel region, and one reached through a second history state).

## IRP suite from a pinned mirror

The W3C IRP files are imported from the verbatim copy in the uscxml repository pinned by commit, transformed with `confEcma.xsl` on SaxonJS, and committed with a SHA-256 manifest that CI checks.

- motivated_by:: [[constraints#W3C serves a bot challenge to scripted clients]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#The IRP stylesheet is XSLT 2.0]] {weight: primary}
- motivated_by:: [[decisions#Full SCXML first]] {weight: secondary}

The mirror's `update-txml.sh` copies the W3C files unchanged; the only local edit it carries is commented out. `crates/vertep-conformance/scripts/import-irp.sh` re-imports, `check-suite-hash.sh` verifies.

## Configuration well-formedness proof is pending

Superseded on 2026-11-12 by [[decisions#Configuration well-formedness is proven]]: the theorem had been stated only outside the build while the Lean model kept finding charts that pass validation and break it.

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: secondary}

## Configuration well-formedness is proven

The Lean package proves that every running session reachable by `start` and `step` from a well-formed chart has a legal configuration, with no `sorry` and only the standard axioms.

- supersedes:: [[decisions#Configuration well-formedness proof is pending]] {reason: "proven 2026-11-12"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[constraints#SCXML interpretation algorithm]] {weight: secondary}

`configuration_wellformedness` (`lean/vertep-lean/VertepLean/Config/Theorem.lean`) is built from compile correctness, the exit lemma, the history invariant, fuel adequacy, the entry lemma and the interpreter invariant through every action of `start` and `step`. The proof found five counterexamples, all runtime bugs in Rust, now validator rules or semantic fixes in both cores with regression tests (see [[decisions#Appendix D readings in the Rust core]]): a history target next to its sibling; an `initial` naming a history state without a default transition; the domain recomputed on entry (a deep history of a single-region parallel state); and twice, entry climbing above a domain that lies below a history's parent. The shapes-equivalence theorem is still pending.

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

IRP templates become `ecmascript`, `null` and `cel` charts once, by a script; the results and their hashes are committed, every expected deviation has a reason, and an unexpected pass fails (M0 design D10).

- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IRP suite from a pinned mirror]] {weight: secondary}

## Minimal dependencies per M0 crate

`vertep-core` uses only `alloc` (its bitset is in-crate), `vertep-scxml` quick-xml, `vertep-dm-ecma` boa, `vertep-rdf` oxrdf and oxttl with RDF 1.2 and RDFC-1.0, and `vertep-lean` a pinned LeanRDF (M0 design D11).

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: secondary}

## No throwaway journal spike

The planned M0 spike of a tiramemsu journal with an LLM decider was not built, because M2 was implemented directly on the finished core and storage, which answered the same integration questions with real code (replaces M0 design D13).

- motivated_by:: [[decisions#Full SCXML first]] {weight: primary, since: "2026-11-11"}

## Type IRIs hash the definition graph

A named type's IRI is `urn:vertep:type:sha256:<hex>` over the RDFC-1.0 canonical N-Quads of its definition graph, with fields and variants keyed by name and references written as the referenced types' IRIs.

- motivated_by:: [[decisions#Content-addressed chart versions]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#One IR type language]] {weight: secondary}

Source order does not change a type's IRI, a type's IRI covers every type it uses, and mutually recursive types are hashed as a group. The name is part of the identity.

## Type vocabulary is its own ontology module

Type terms live in `vt-types.ttl` next to the chart module, so the chart module keeps exactly one class per IR node kind and a typed chart's projection adds type definitions and shapes under their type IRIs.

- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: primary, since: "2026-11-11"}

## SCXML type annotations are canonicalized on read

The SCXML reader rewrites every `vt:type` annotation and `vt:types` block into canonical type text, so the same typed chart hashes to the same version from any surface or spelling.

- motivated_by:: [[decisions#Types ride on vt extension nodes]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Content-addressed chart versions]] {weight: secondary}

## vt certify runs the Lean checker as a process

`vt certify` writes the chart's published N-Quads to a file and runs the Lean-compiled `vt-certify --chart <file>`, found through `VT_CERTIFY` or on `PATH`; without it no certificate is produced.

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}

Since 2026-11-12 typed charts certify too, see [[decisions#The Lean checker recomputes a chart's type triples]].

## The Lean checker recomputes a chart's type triples

`vt-certify` reads a typed chart's `vt:types` block from the decoded chart, derives its type triples as `vertep-rdf` does, requires them in the graph and hashes them into the chart version, so typed charts certify under the version Rust computes.

- motivated_by:: [[decisions#vt certify runs the Lean checker as a process]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Persistent vt namespace and blank-node-free charts]] {weight: secondary, note: "type cells and shapes are minted under the type IRI, so the canonical form stays a sorted line set"}
- motivated_by:: [[decisions#Types ride on vt extension nodes]] {weight: secondary}

The type triples are ground (no blank nodes), so certifying them needs no RDFC-1.0 in Lean: the earlier belief that it did was wrong. `VertepLean/Types/Syntax.lean` mirrors the declaration parser and the `vt:types` text reading, and `VertepLean/Rdf/Types.lean` mirrors `project_chart_types`: content-addressed type IRIs (strongly connected groups hashed together under `urn:vertep:type:self`), definitions and SHACL node shapes. A graph without one of these triples is refused, and `Certify.check_certified_wellFormed` states that the graph holds the chart's whole projection. 1,000 generated declaration blocks (random layouts, CDATA, entities, quoted names, recursive groups, unknown names and corrupted text, about a fifth of which do not parse) certify under the Rust version, as do the typed charts of the test suites.

## Rust actions lower to a foreach over the action IRI

A `rust`-datamodel action is one `<foreach array="<action IRI>" item="_vt_emit">` around one `<send>`: evaluating the array runs the action and yields its emitted events, so raises and sends stay visible in the IR without changing the interpreter.

- motivated_by:: [[decisions#IR is the hub]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary}

The datamodel trait cannot enqueue events; a core hook would allow one node per action, but nothing needs it yet.

## Default function IRIs are vt:guard and vt:action paths

Guards and actions are named `vt:guard/<chart>/<name>` and `vt:action/<chart>/<name>`; the registry treats `vt:` and the expanded namespace as one IRI, and `include_chart!` binds by the IRI's last segment.

- motivated_by:: [[decisions#Pluggable datamodel]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IR is the hub]] {weight: secondary}

## Generated chart versions are computed at first use

`statechart!` emits `CHART` and `VERSION` as lazily built statics, and `VERSION` is the chart hash from `vertep-rdf` computed at run time, because the macro never goes through RDF.

- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Content-addressed chart versions]] {weight: secondary}

## Macro findings are compile errors or deprecation lints

Structural and type errors are `compile_error!` at the offending tokens; unreachable states and unhandled events are `deprecated` lints at the token, raised with `#[deny(...)]` and silenced with `#[allow(...)]` in the chart.

- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: primary, since: "2026-11-11"}

Stable proc macros have no other way to emit warnings, so the lint text starts with "use of deprecated function".

## The rust datamodel evaluates only literals, paths and function IRIs

Guard and action logic is Rust; the `rust` datamodel evaluates JSON literals, context, system and loop-variable paths, `In()` and function IRIs, and its typer leaves the functions to the Rust compiler.

- motivated_by:: [[decisions#Datamodel typers plug into one core checker]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary}

## Types are IR values checked in the core

Types are ordinary IR values in `vertep-core`, checked by one `no_std` checker that every surface and the runtime call, so a chart is typed the same way however it was written (M1 design 1).

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IR is the hub]] {weight: secondary}

## Type projections must agree with the checker

The RDF, SHACL, JSON Schema and CEL projections of a type are tested to give the same verdict as the core checker on generated valid and near-miss values, and the Lean model is to prove the SHACL agreement (M1 design 3).

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary}

## Undefined object entries are absent

An object entry whose value is `undefined` is absent for every check of the type checker, the unknown-field check of records and unions included, as the SHACL encoding of values leaves such entries out.

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary, note: "counterexample found by the SHACL projection proof, verification backlog 4.2"}

Before, `{a: 1, q: undefined}` failed `{a: int}` with `UnknownField` while its RDF encoding conformed to the type's shapes, so the SHACL projection disagreed with the checker. A declared field holding `undefined` was already absent; the unknown-field check now reads entries the same way, in `crates/vertep-core/src/types/check.rs` and in the Lean `check` and `HasType`. Rejected: encoding `undefined` entries as `vt:Null`, which makes a payload-free union variant with `value: undefined` fail its `sh:maxCount 0`.

## List members get a member predicate

A non-empty list's RDF node links each item with `vt:listMember`, and the SHACL shape of `list<T>` checks members on that predicate, so every path of the type shapes is a predicate path and the shapes stay blank-node free.

- motivated_by:: [[decisions#Persistent vt namespace and blank-node-free charts]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Type projections must agree with the checker]] {weight: primary}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary, note: "found by the cross-check of the SHACL projection proof, verification backlog 4.2; fixed in 4.7"}

The shape of `list<T>` used to check members on `rdf:rest*/rdf:first`, with the `sh:zeroOrMorePath` node and the sequence list minted as IRIs under the type IRI. SHACL §2.3.1 reads an IRI as a predicate path, so a standard engine never reached the members and accepted `[1, "2"]` as a `list<int>`; only `vertep-rdf`'s validator, which read such IRIs structurally, agreed with the checker. Now `value.rs` adds `<first cell> vt:listMember <item>` for every item, the closed collection cell of the shape allows `rdf:first`, `rdf:rest` and `vt:listMember`, and its `vt:listMember` property shape carries `sh:node` of the item type; the Rust validator reads every IRI path as a predicate. The RDF collection stays: the member triples are a set, and the collection keeps the order and the repeated items, so the encoding still determines the value. Only the list's own node carries members, so the encoding stays linear in the list's length. Type IRIs do not change (they hash definitions, not shapes); chart versions of charts whose types use lists do. `vt:listMember` is declared in `ontology/vt-types.ttl`; `Types/Shacl.lean` mirrors the encoding and the shapes, and `shacl_agrees` no longer needs the encoding's subjects to be blank nodes. Rejected: blank nodes for the path nodes, which would make typed chart projections non-ground and require RDFC-1.0 in the Lean certifier.

## Type checking stops on non-contractive recursion

`type A = optional<A>` stays valid, as the type-system spec allows recursion through an optional; the value checker reports a mismatch instead of expanding a named type again at the same value position.

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary, note: "found by the protocol-model review; the Lean checker must mirror it"}

The requirement `Recursive and Cyclic Type Definitions` admits recursion "only through a list or an optional", so rejecting `optional<A>` in `validate()` would contradict it; the type is well founded, its only value is `null`. The checker's `walk` used to recurse through `Named` and `Optional` without consuming the value and overflowed the stack on any non-null value. It now keeps the named types it is expanding at the current position, clears them when it descends into a list item, record field or union payload, and reports `Mismatch` when a name repeats: between two expansions of the same name at the same position only names and optionals can intervene, so a repeat means the expansion never ends. Rejected: rejecting non-contractive cycles in `validate()`, which the spec does not support, and a depth bound, which would accept or reject by an arbitrary constant.

## One macro pipeline for inline and included charts

`statechart!` and `include_chart!` share one pipeline: tokens or a `.scxml`/`.yaml` file become IR, which is validated, type-checked and turned into the same generated types (M1 design 4).

- motivated_by:: [[decisions#IR is the hub]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Proc macros run at compile time]] {weight: secondary}

## A node runs its actors on one thread

A node owns its activations and schedules turns synchronously and deterministically; only function, stream and worker turns run as `Send` futures, and outside callers use a `Send` handle.

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Pluggable datamodel]] {weight: secondary, note: "datamodels such as boa are not Send"}

Run a node in a `LocalSet` or on a dedicated thread; tests drive `run_until_idle` on a virtual clock.

## Entry one carries the start macrostep

The first input entry of an actor also records the session start, so replay begins with `start` under that entry's chart version, time and seed.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}

## Deduplication reads the whole journal on activation

An activation collects every accepted message id from its journal, snapshot or not, and a turn acknowledges without running any message whose id is already there.

- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary}

The cost grows with the journal; an index of message ids in the current-state graph is the planned fix.

## Outbox progress lives in an outbox graph

Each actor has a graph `<A>/outbox` holding `vt:dispatchedThrough n` and `vt:dispatched` marks on fired timers; restart and activation dispatch every send after the mark again, and receivers deduplicate.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

## Pending timers live in the current-state graph

Scheduled timers are statements of the actor's current-state graph committed with their entry through the state delta, and a restarted node re-arms them with one `vt:` query.

- motivated_by:: [[decisions#State changes travel in the append]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: secondary}

## Transition annotations go to the graph store

Entries record `vt:tookTransition` as plain triples; the annotated statement with `vt:causedBy <entry>` goes to graph `<entry>/annotations` with a blank-node reifier.

- motivated_by:: [[constraints#Tiramemsu cannot hold unasserted triple terms or reifier IRIs]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: secondary}

## Only journal entries carry vt:actor and vt:position

Snapshots name their actor with `vt:snapshotOf` and timers carry no position, and the tiramemsu journal read selects entries inside their own graph by `rdf:type vt:JournalEntry`.

- motivated_by:: [[decisions#Journal entries are stamped by the store]] {weight: primary, since: "2026-11-11"}

The tiramemsu read once matched any subject with both properties in any graph; it is scoped to entry graphs since 2026-11-11, with a regression test.

## Replies inherit the request's correlation id

The runtime assigns `vt:request` correlation ids `<A>/request/<position>.<k>` and copies the input's correlation id onto every `vt:reply` sent in that turn, so charts need no datamodel to correlate.

- motivated_by:: [[decisions#Non-blocking request and deferred events]] {weight: primary, since: "2026-11-11"}

The lowering of `vt:request`, `vt:reminder` and `vt:migrate` lives in `vertep-runtime` until `vertep-core` takes it.

## Virtual actor addresses append the key to the type

A virtual actor of chart type `T` and key `K` is `<T>/<percent-encoded K>`, and its records are named under that address (`/entry/n`, `/snapshot/n`, `/invoke/id`).

- motivated_by:: [[decisions#IRI identity and lifecycle classes]] {weight: primary, since: "2026-11-11"}

## Randomness is seeded from the entry IRI

Each macrostep's random seed is derived from its entry IRI, so random values are not journaled one by one; the entry's configuration digest covers the datamodel they produced.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}

## Decide bindings ride on vt extension attributes

A `vt:decide` binding is read from `vt:` extension attributes, or a `vt:decide` element, on an ordinary state; the candidate set comes from the pure `enabled_events` in `vertep-agent` over the core's transition selection.

- motivated_by:: [[decisions#Types ride on vt extension nodes]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Offer-time decider safety]] {weight: secondary}

The M3 tasks placed the binding in the IR and `enabled_events` in `vertep-core`. Keeping both in `vertep-agent` leaves the no-std core, its compiled charts and the Lean model untouched, and every surface keeps the binding losslessly because SCXML and YAML already keep `vt:` extensions.

## The agent host journals agent inputs

`vertep-agent` runs agent actors in its own deterministic shell, `AgentHost`, over an `AgentJournal` of typed inputs: events, decisions, refusals, fired timers, `llm.result`, LLM errors and tool results.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Every interaction is an effect]] {weight: secondary}

The pure `AgentMachine` applies one input and returns effects; the host performs them, so replay and forks consult no decider, provider or tool. Agent actors reach runtime actors through a `NodeHandle`, and invoked runtime actors are restricted by `GrantedActor`, which filters their sends before they leave the actor.

## Correlation ids derive from the journal position

An offer's correlation id is `<actor>/decision/<position>.<k>`, the journal position of the input whose macrostep made the offer, so replay reproduces it and a fork renames only the actor prefix.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Offer-time decider safety]] {weight: secondary}

## vt:retry lowers to journaled retry timers

`vt:retry="n"` with `vt:backoff` on an LLM invoke retries retryable error classes after a doubling durable backoff timer; every attempt and every fired timer is a journal entry.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}

The spec describes the extension as sugar for a generated retry substate. The machine-level lowering produces the same journal and the same `error.llm` (last class, attempt count) without rewriting the chart, so the chart's version hash is the one its author wrote.

## LLM decisions count as LLM calls

The model call behind an LLM decision is added to the actor's cumulative usage and recorded as a `vt-agent:LlmCall` keyed by the offer's correlation id, whether or not the decision is accepted.

- motivated_by:: [[decisions#Deciders at choice points]] {weight: primary, since: "2026-11-11"}

## Agent error events extend the error payload

Agent `error.*` events carry the predefined `kind` and `message` fields plus their own, such as `reason`, `class` and `paths`; a typed chart may declare the richer payload under the event's exact name.

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Typed charts fix the error payload]] {weight: secondary}

`vertep-core` gives an exactly declared system event its declaration when typing transitions, as it already did when looking up payloads.

## The agent ontology ships with vertep-rdf

`vt-agent.ttl` and its SHACL shapes live in `vertep-rdf` beside the chart and runtime modules; `vertep-agent` writes the records and closes them under the module's PROV-O subclass and subproperty axioms.

- motivated_by:: [[decisions#Modular vt ontology on RDF 1.2]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#The SHACL validator bounds only integers]] {weight: secondary}

Decision annotations are RDF 1.2 reifiers of `decision vt-agent:chose event`, and confidence ranges are checked in Rust before a decision is accepted.

## Enumerations are tagged unions in structured output

A structured-output enumeration is a union of payload-free tags, so a model answers `{"tag": "tech"}` and guards read `ev.data.category.tag`.

- motivated_by:: [[constraints#The type language has no string enumerations]] {weight: primary, since: "2026-11-11"}

## DID resolution is a directory chosen by address scheme

The runtime resolves `did:` addresses through a federation resolver registered for that scheme, which turns DID document services into routes and keys; `vt:` actors never reach it (M5 design D1).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#IRI identity and lifecycle classes]] {weight: secondary, note: "a DID is one IRI scheme"}

Rejected: a DID for every actor, which would put keys and a signature on every in-cluster message.

## Method resolvers behind one trait

`did:key` (offline), `did:peer` (pairwise) and `did:webvh` (verifiable history) ship behind one resolver trait and a registry that applications extend or override (M5 design D2).

- motivated_by:: [[decisions#DID resolution is a directory chosen by address scheme]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: secondary}

Rejected: a universal resolver over HTTP, which adds a network trust dependency to every resolution and fails offline.

## Keys stay behind a key manager

Signing, decryption and key agreement go through a `KeyManager` trait; journals, snapshots, datamodels and the graph hold only public keys and key ids (M5 design D3).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary, note: "a never-forget store could only crypto-shred a key"}

Hardware modules and remote key services plug in at the same seam.

## Verification outcome is journaled with the input

Sender authentication runs in the shell before `step`; the verified origin is journaled with the input event, so replay after a key rotation uses the recorded outcome (M5 design D4).

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: secondary}

Rejected: verifying in the datamodel or a guard, which would make `step` depend on resolution and cryptography.

## Verified origin carried by origin presence

Superseded interim rule: before `_event` had a verified flag, a relay delivery set `_event.origin` only to a verified sender DID, so an origin's presence meant a verified sender.

- motivated_by:: [[decisions#Verification outcome is journaled with the input]] {weight: primary, since: "2026-11-11"}

Replaced by [[decisions#Verified origin flag in the event]]. The rule that an unverified event has no origin stays; what changed is that guards no longer infer verification from presence.

## Verified origin flag in the event

`_event.originverified` is true only when the shell verified the sender DID in `_event.origin` against its DID document; every datamodel exposes it, the journal stores it, and the Lean model carries it.

- supersedes:: [[decisions#Verified origin carried by origin presence]] {since: "2026-11-11", reason: "m5-federation 4.1 added the field to vertep-core"}
- motivated_by:: [[decisions#Verification outcome is journaled with the input]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: secondary}

The field is a `bool` defaulting to false, so `Event { .., ..Event::default() }` literals keep compiling and `Event::with_verified_origin` sets origin and flag together. It is read as false from older journals (`vt:originVerified` absent), ECMAScript snapshots and oracle tapes, and written to the journal and snapshots only when true, so existing data is unchanged. `VerifiedOrigin::apply` is the one place that sets it; the runtime clears it wherever it assigns a cluster origin itself, since a trusted-cluster origin is not a verified DID. ECMAScript and the `rust` datamodel expose `_event.originverified`, CEL `ev.originverified` (typed `bool`), and `Sys::verified_origin` gives Rust actions the verified sender; the `null` datamodel has no `_event`. Rejected: an origin-kind enum, which would change every datamodel's `_event` shape for no extra information.

## DIDComm mailboxes acknowledge after the append

A DIDComm mediator stores forwards for an actor; the actor's mailbox picks up in batches or live and acknowledges each message to the mediator only when the runtime acknowledges it after the journal append (M5 design D6).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary}

The relay mailbox implements the runtime's `Mailbox`, so the runtime's own acknowledgement after a durable turn, or after it drops a duplicate message id, is what releases the mediator's copy.

## Remote mediators through Pickup 3.0

A mailbox picks up from any DIDComm v2 mediator with `MediatorClient`: Mediator Coordination 2.0 and Message Pickup 3.0 requests authcrypted from the actor with `return_route: "all"`, replies read from the response.

- motivated_by:: [[decisions#DIDComm mailboxes acknowledge after the append]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

A reply counts only when it is authcrypted by the mediator's DID and threads to the request. `delivery-request` is not an acknowledgement; `messages-received` goes out only when the runtime acknowledges after the append, and the mediator may leave it unanswered (Identus does). Senders need nothing new: the recipient's DIDComm service names the mediator's endpoint and its key-agreement key as routing key, and the usual forward wrapping applies. Interop is checked against the Hyperledger Identus mediator (`ghcr.io/hyperledger/identus-mediator:1.0.0` with `mongo:6.0.19`, task 10.1) in an opt-in Docker test, and the in-process test mediator answers the same messages so CI covers the binding. The HTTP client is the deployment's `RelayNetwork`; live delivery (`live-delivery-change` over WebSocket) is not bound yet.

## TSP direct when reachable, routed otherwise

The TSP transport maps an actor's DID to its VID, sets up a relationship before the first direct event, and nests the event for the recipient inside routed messages through intermediaries (M5 design D7).

- motivated_by:: [[decisions#TSP before DIDComm across trust boundaries]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

The recipient's `TSPTransport` service lists its intermediaries as routing keys; the last intermediary delivers a nested message the recipient opens and verifies against the original sender.

## Protocol-neutral event envelope

One JSON-LD envelope carries an SCXML event over DIDComm and TSP alike: name, send id, typed data, sender chart version and message id; `origin` and `origintype` come from the verified sender and the I/O processor (M5 design D8).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary, note: "the message id in the envelope is the deduplication key"}

Rejected: per-protocol native message types, which would mean two mappings and protocol code that knows event semantics.

## Transport selection follows the DID document

A cross-trust send tries the recipient's services in a configured preference order, falls back to the next compatible one when an endpoint is unreachable, and records the fallback (M5 design D9).

- motivated_by:: [[decisions#TSP before DIDComm across trust boundaries]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Non-blocking request and deferred events]] {weight: secondary, note: "retries never block the chart"}
- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: secondary}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary, note: "a fallback re-send keeps the message id"}

## Relay outbox owns cross-trust retries

The runtime outbox hands a cross-trust send to a relay outbox queue in the same storage; that queue applies outbound grants, selection, fallback and the retry policy, and raises `error.communication` in the sender.

- motivated_by:: [[decisions#Transport selection follows the DID document]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Non-blocking request and deferred events]] {weight: primary}

The runtime's own retry loop knows no policy or reason; keeping cross-trust delivery in one place lets the error carry the send id and the reason, and leaves in-cluster delivery unchanged.

## Pinned relay protocol libraries

DIDComm v2 uses Affinidi's DIDComm 2.1 and JOSE crates, TSP the OpenWallet Foundation `tsp_sdk`, and `did:webvh` the DIF `didwebvh-rs`, each behind a cargo feature, with private-key steps through the key manager.

- motivated_by:: [[decisions#Keys stay behind a key manager]] {weight: primary, since: "2026-11-11"}

The libraries' own pack and unpack take raw private keys, so the JWE and TSP envelopes are composed from their primitives; output is checked wire-compatible with Affinidi's decrypt and `tsp_sdk`'s open.

## Shards by IRI hash modulo a fixed count

An actor's shard is a fixed, versioned 64-bit hash of its IRI modulo a power-of-two shard count recorded at cluster creation (M4 design D1).

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: primary}
- motivated_by:: [[decisions#Power-of-two shard counts]] {weight: secondary}

Hash version 1 is the FNV-1a placement of single-node tiramemsu backends, so a single node becomes a one-member cluster without moving data. Rejected: an Orleans per-actor directory, and consistent hashing with virtual nodes, which ties an actor's shard to the member set.

## Leases with fencing epochs from the membership authority

Every shard lease has an expiry and an epoch that increases strictly on each grant and stays on renewal; an owner stops a safety margin before expiry by its own monotonic clock (M4 design D2).

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Fencing epochs on journal appends]] {weight: secondary}

Rejected: journal compare-and-set alone, which neither stops a stale owner's outbox nor works across shipped file copies.

## Local shard journals shipped on move

In v1 a shard's journal is a tiramemsu file on its owner: a move drains, ships the missing suffix, verifies the end versions and then transfers the lease; failover uses optional standbys (M4 design D3).

- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Optional standby journal replication]] {weight: primary}

Tiramemsu never mutates history, so shipping and replication are append-only copies. Rejected: a networked journal from day one; the lease-transfer move is specified and tested against a shared in-memory store so such a backend needs no spec change.

## Fencing on appends and outbound frames

Every append and every outbound frame of a virtual actor carries its shard and lease epoch; stores and standbys reject lower epochs, and receivers drop lower-epoch frames into the dead-letter graph (M4 design D4).

- motivated_by:: [[decisions#Fencing epochs on journal appends]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Shard-based placement]] {weight: secondary}

The store side is the M2 fencing token with the shard IRI as its scope; M4 is the first caller that sets it.

## Membership behind one trait

Static, gossip (SWIM) and external (etcd, Kubernetes Lease) membership implement one trait: view, subscribe, acquire, renew, release, revoke and hand over leases; one test kit runs on all (M4 design D5).

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}

Rejected: embedding Raft. Gossip lease granting is weaker than consensus under partitions; fencing keeps safety there. The gossip implementation is a sans-IO SWIM in the crate rather than the `foca` crate, so the simulated network can drive it deterministically.

## QUIC with mutual TLS and a stream per actor pair

Each node keeps one QUIC connection per peer with cluster-issued certificates; messages from actor X to actor Y use a stream keyed by the pair, up to a cap, then share streams in order (M4 design D6).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}

Rejected: TCP with TLS (head-of-line blocking across actors) and gRPC (a schema layer and weaker control over streams).

## At-least-once inside the cluster with journal deduplication

Senders retry until the receiver acknowledges that the message is stored; receivers deduplicate by message id, through the journal for virtual actors and a bounded id cache otherwise (M4 design D7).

- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary}

Rejected: an exactly-once transport, impossible across failures and redundant with the journal.

## Rebalancing moves the fewest shards and resharding is offline

The rebalancer keeps each shard with its live owner while that owner is within its share and moves only the rest; resharding doubles the shard count offline, splitting shard s into s and s plus N (M4 design D8).

- motivated_by:: [[decisions#Shard-based placement]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Power-of-two shard counts]] {weight: primary}

Each move is drain, ship or confirm the replica, verify the end version, hand the lease over at epoch plus one, resume. Online resharding is a non-goal of M4.

## Workers are pooled, not sharded

Stateless worker activations go to any member declaring the worker type, balanced by round robin, least outstanding or prefer local; they never lease a shard or touch a journal (M4 design D9).

- motivated_by:: [[decisions#IRI identity and lifecycle classes]] {weight: primary, since: "2026-11-11"}

Results reach a virtual actor caller as journaled reply events matched by correlation id. Failures are error events or request timeouts; only types declared idempotent are retried, once, on another activation.

## Placement is a runtime hook with three answers

A clustered runtime node asks a `Placement` where a virtual actor or worker lives before delivering or activating: local, remote through a mailbox that carries the message, or held in the local mailbox.

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Shard-based placement]] {weight: primary}

Without a placement the runtime is unchanged. The hold answer is what keeps a non-owner from activating an actor whose shard has no live owner, and what makes messages wait in mailboxes with replication factor one.

## A free shard goes only to a node holding its journal

A node claims a shard whose lease is free only if it holds the shard's journal (old owner, shipped copy or standby replica) or the shard was never owned.

- motivated_by:: [[decisions#Local shard journals shipped on move]] {weight: primary, since: "2026-11-11"}

With replication factor one this keeps a failed owner's shards unavailable instead of restarting them from an empty journal on another node.

## Unreplicated entries stay unreadable until the quorum catches up

An owner appends locally, then ships to standbys; without the quorum the append fails and the actor's journal is neither read nor extended until a later shipment brings enough standbys level.

- motivated_by:: [[decisions#Local shard journals shipped on move]] {weight: primary, since: "2026-11-11"}

So an entry that missed the quorum never produces effects, and a standby taking over holds every entry whose effects anyone saw.

## Worker pools dispatch from the sending node

Each node runs its own dispatcher per worker type and balances its own requests over activations it creates on the members that declare the type in configuration; each reply passing through frees its activation's capacity.

- motivated_by:: [[decisions#Workers are pooled, not sharded]] {weight: primary, since: "2026-11-11"}

No cluster-wide pool state is needed. Per-node limits are therefore per dispatcher; the worker node's runtime pool bounds the true concurrency.

## Fault-injection histories export as JSON lines

The fault-injection harness records grants, appends, frames and acknowledgements, checks them in Rust, and can write them as JSON lines for the Lean lease and fencing model to replay as an oracle.

- motivated_by:: [[decisions#Lean verification bridge]] {weight: primary, since: "2026-11-11"}

The format is documented in `vertep-cluster` `history.rs`; the Lean side reads it in a later change.

## Offline resharding reads a lease dump

`vt cluster reshard` refuses to run while the data directory's `leases` file, dumped from the lease authority, lists a live lease, and names the nodes holding them.

- motivated_by:: [[decisions#Rebalancing moves the fewest shards and resharding is offline]] {weight: primary, since: "2026-11-11"}

The tool writes the doubled shard files and metadata to a new directory, so rolling back is keeping the old one.

## External authorities over etcd and the Kubernetes Lease API

`EtcdAuthority` and `KubeAuthority` implement the lease authority over etcd's v3 JSON gateway and the `coordination.k8s.io/v1` Lease API, behind features `etcd` and `kubernetes`, with a small blocking HTTP/1.1 client of the crate's own.

- motivated_by:: [[decisions#Membership behind one trait]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#QUIC with mutual TLS and a stream per actor pair]] {weight: secondary, note: "the same rustls with ring, no aws-lc"}

A node's membership is `ExternalMembership` over either, exactly as the static membership is over its hosted authority. No gRPC, async client stack or Kubernetes client crate is needed for a few small JSON requests per tick; the available `ureq` 2 would pull rustls's aws-lc provider.

## etcd expires shard grants and a permanent key holds each shard's epoch

A shard lease is an etcd key attached to an etcd lease whose TTL is the term, so etcd itself expires it; a never-deleted epoch key per shard makes every grant's epoch exceed the last.

- motivated_by:: [[decisions#Leases with fencing epochs from the membership authority]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#etcd lease TTLs are whole seconds]] {weight: secondary}

Grants and handovers are transactions comparing the `mod_revision` of the shard key and the epoch key. Members are keys on two etcd leases (suspicion and failure time), so a member is active, suspect or failed by which of its keys etcd still holds. View versions are etcd revisions, taken only when the view changes.

## Kubernetes grants expire by observation

A node takes a shard another node holds in Kubernetes only after seeing the holder's record unchanged for a whole term on its own monotonic clock, as client-go leader election does.

- motivated_by:: [[constraints#Kubernetes stores leases but does not expire them]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Clock skew beyond the safety margin breaks lease exclusivity]] {weight: secondary}

The node first saw the record after the holder sent that renewal, so the holder's local deadline has passed by then whatever the wall clocks say. `leaseTransitions` is the epoch and every write carries the `resourceVersion` read. Any node seeing a member's heartbeat unchanged for the suspicion or failure time writes the new status, guarded the same way, so a heartbeat in between wins.

## An owner checks its standbys before writing its own copy

Before a replicated append, the owner ships what its standbys lack and requires the quorum under its epoch; a standby that stored a newer epoch fences the owner, and an unreachable quorum stops the append, both before the owner writes.

- motivated_by:: [[decisions#Unreplicated entries stay unreadable until the quorum catches up]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Fencing on appends and outbound frames]] {weight: secondary}

A copy that only the owner holds can still arise from a race between the check and the shipment, which the unreadable-until-caught-up rule covers; it no longer arises from a standby that was simply down or newer.

## A standby's fence rises only with entries it stores

A standby replica refuses batches below the highest epoch it accepted, but that epoch rises only when it stores entries, as a fenced journal store's does; a batch that only checks or catches up fences no one.

- motivated_by:: [[decisions#Fencing on appends and outbound frames]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Fault-injection histories export as JSON lines]] {weight: secondary, note: "replicated histories then match the Lean model's single fenced store"}

With replication factor 3 and every standby required, the replica set then fences exactly when the model's one store would.

## A new owner replicates to the shard's current standbys

`ClusterStorage` asks for a shard's standbys again whenever its node gets a new lease epoch on the shard, so after a failover or a move the new owner replicates to the next active members after itself.

- motivated_by:: [[decisions#A free shard goes only to a node holding its journal]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Local shard journals shipped on move]] {weight: secondary}

A member that replaces a failed standby receives the whole journal with the next shipment. With fewer live members than the replication factor, appends wait rather than acknowledge with fewer copies than configured: a three-node cluster with factor 3 stops writing a shard while one node is down.

## Gossip leader epochs carry a per-start tag

A gossip leader's epoch tag comes from its identifier and its start, and a node keeps the highest epochs it granted across its terms as leader, so a leader that restarted or lost and regained leadership never grants an epoch again.

- motivated_by:: [[decisions#Fencing on appends and outbound frames]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Membership behind one trait]] {weight: secondary}

The fault matrix found a restarted leader granting an epoch it had already granted, which no node it could reach remembered: two writers per epoch, which fencing cannot tell apart. Uniqueness across starts is by tag, so it is probabilistic over the 15-bit tag space, as uniqueness across leaders already was.

## A leaving node ships each shard before handing it over

A graceful leave drains each shard, ships it to its successor through `Drain::ship_shard` (for journals kept in local files), then hands the lease over; a shipment that fails stops the leave with the lease kept.

- motivated_by:: [[decisions#Local shard journals shipped on move]] {weight: primary, since: "2026-11-11"}

`NodeDrain::with_shipping` carries the shipment, which `ship_journal` does between stores; a network copy plugs in the same way.

## Frames to other nodes are kept until settled

With `ClusterNode::keep_outbound`, a frame handed to another node is stored in the sender's storage until it is settled, and a restarted node sends again what it kept.

Settled means acknowledged, redirected, rejected or past its delivery deadline; the hand-off counts as done only once the frame is stored.

- motivated_by:: [[decisions#At-least-once inside the cluster with journal deduplication]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

The kept frame is the encoded frame itself, so a resend keeps its identifier and the fence it was produced under; receivers discard what they already stored. A stopped node (`ClusterNode::stop`) abandons its retries, as a crashed process does.

## Container fault injection runs every node as a process

The container harness runs each node as `vertep-fault-node` in its own container, with etcd or k3s beside them, and injects partitions, pauses, crashes and restarts through the container engine.

- motivated_by:: [[decisions#Fault-injection histories export as JSON lines]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#The Docker Desktop kernel has no netem]] {weight: secondary}

The world (client messages, the fenced store, receivers and the history) runs in the test process, so the history is checked and replayed through the Lean model exactly as the in-process one. One-way loss uses `tc netem` where the kernel has it and otherwise drops the node's side-effect datagrams at the receiver; clock skew runs one node's lease clock at half speed, since containers share the host kernel's clock.

## One worker hosts the core, the shell and the journal

In a browser the whole actor system (core, shell, tiramemsu journal and the application's guards and actions) runs in one dedicated worker; the page holds a thin typed client (M6 design D1).

- motivated_by:: [[constraints#OPFS sync access handles need a dedicated worker]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#A node runs its actors on one thread]] {weight: secondary}
- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: secondary, note: "steps call guards synchronously, run to completion"}

A core on the main thread with storage in a worker would make every journal append an asynchronous round trip inside a macrostep; a SharedWorker has no OPFS sync access handles. `vertep-wasm` implements this as `System` behind a JSON command bridge; `@vertep/web` posts commands and receives notifications.

## The browser shell implements the runtime shell trait

The browser shell maps spawn to `spawn_local`, sleep to a `setTimeout` future and the clock to `Date.now()`, read only when stamping an input; Node.js worker threads use the same shell (M6 design D2).

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: primary, since: "2026-11-11"}

The node is driven by `run_until_idle`, `Node::next_deadline` and `NodeHandle::wakeup` rather than tokio; the first scheduling round, which fires overdue durable timers, waits for the first command so that the application's chart types are registered first. The sleep future holds only shared state, so it is `Send` without `unsafe`.

## Implementations are a module imported by the worker

Applications pass a module URL whose `guards` and `actions` exports are bound, by IRI, into the `rust`-style named-function registry; the worker imports it and steps call the functions synchronously (M6 design D3).

- motivated_by:: [[decisions#Pluggable datamodel]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Rust actions lower to a foreach over the action IRI]] {weight: secondary}

Functions cannot cross `postMessage`. Purity is a documented contract; development mode replays every new journal entry and reports a guard or action whose replay disagrees with the live step. CEL charts need no module.

## Browser storage is explicit, exclusive and native-format

Storage is `opfs` or `memory`, chosen explicitly with no fallback; an `opfs` store takes a Web Lock named after its path before it opens, and journal files keep tiramemsu's native format (M6 design D4).

- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#OPFS sync access handles need a dedicated worker]] {weight: primary}
- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: secondary}

A plain `memory` store is `vertep-store`'s in-memory store and holds no SQLite; `memory` with `journal: true` is a tiramemsu file in the memory VFS that exports and imports, which is how Node.js persists. OPFS without sync access handles fails with `StorageUnavailable`; a second holder gets `StoreLocked`.

## TypeScript is one more projection of the type language

`vt gen ts` emits the event union, the context interface, the state-id union, a typed chart spec and a typed `create<Name>Actor` from the same IR types as the other projections (M6 design D5).

- motivated_by:: [[decisions#One IR type language]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Type projections must agree with the checker]] {weight: primary}

TypeScript cannot state integer ranges or string patterns, so it agrees with JSON Schema on structure while scalar refinements are checked when the event reaches the chart; a property test checks the agreement on structure.

## JavaScript ergonomics follow XState and semantics follow SCXML

The API borrows XState's names (`createActorSystem`, `spawn`, `send`, `request`, `subscribe`, `getSnapshot`, `matches`, `can`) while execution is exactly the core step (M6 design D6).

- motivated_by:: [[decisions#SCXML semantics are normative]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Non-blocking request and deferred events]] {weight: secondary, note: "request is the external ask"}

`can` selects enabled transitions, or a deferring active state, on a copy of the datamodel and journals nothing.

## WebSocket is a transport and an Event I/O Processor

Browser actors reach server actors through a WebSocket transport carrying Vertep envelopes and acknowledgements, with outbox redelivery after reconnect and deduplication by message id (M6 design D7).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary}

Implemented 2026-11-11. The processor's type IRI is `vt:WebSocketEventProcessor` (alias `websocket`); received events carry it as `_event.origintype` and the sender actor's IRI as `_event.origin`. Frames are JSON: `hello` (node IRI, credential, hosted prefixes), `welcome` or `refused`, `message` (id, target, origin, event, payload, send id, correlation) and `ack`. The frames and the per-peer link live in `vertep-runtime::websocket`, shared by both ends; the server end is `vertep-runtime`'s `websocket` feature (tokio-tungstenite), so the default build and the WebAssembly builds stay without a WebSocket server. The browser end in `vertep-wasm` uses the platform `WebSocket` (browsers, Node.js 22 and later) and asks the main thread's credential provider before every connection. A refused credential rejects the pending JavaScript sends with `TransportRefused`, and an actor's send to the server becomes a dead letter with reason `transport-refused`.

## WebSocket sends stay owed until the peer acknowledges them

A send over WebSocket counts as delivered only when the peer's `ack` for its message id arrives; until then the link reports it undelivered, so the runtime's transactional outbox keeps it owed and retries it.

- motivated_by:: [[decisions#WebSocket is a transport and an Event I/O Processor]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, note: "the outbox mark passes only delivered sends"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

The link sends each message once per connection and again after every reconnect; receivers deduplicate by the journaled message id, so each side processes a message once. Holding the outbox mark until the acknowledgement makes redelivery survive a crash of the sender as well as a dropped connection, without a second queue. The cost is that the durable outbox mark trails delivery by up to one failure back-off. A send from JavaScript to a server actor does not go through an outbox: its Promise resolves on the acknowledgement.

## Separate packages keep the bundle budget

`@vertep/web` ships a core build without SQLite and a journal build loaded only for journaled stores; the boa ECMAScript datamodel ships as `@vertep/dm-ecma`, a worker build the application passes in (M6 design D8).

- motivated_by:: [[decisions#Pluggable datamodel]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: secondary, since: "2026-11-11", note: "SPARQL-only reads set the floor of the storage-free build"}

Measured 2026-11-11 (release, opt-level z, fat LTO, one codegen unit, gzip -9, WebAssembly plus glue plus client and worker JavaScript): core 1,159.7 KB (budget now 1.2 MB, formerly 600 KB), journal 2,198.0 KB within 2.5 MB. The core WebAssembly alone is 1,120 KB. The test-only exports moved to a test build saved 38 KB, and the WebSocket client added about 24 KB.

The budget is out of reach while the spec holds. Probe builds of the WebAssembly alone show:

- The core, the SCXML reader and CEL take 341 KB.
- `vertep-store`'s in-memory store with its SPARQL engine (spargebra, spareval, regex, oxrdf) takes 637 KB on its own.
- The two together take 785 KB, before the runtime, the YAML reader (56 KB) and the RDF chart reader (72 KB).

SPARQL cannot leave the core build: `GraphStore` reads only through SPARQL, and the runtime itself queries its graphs for timers, store mailboxes and dead letters. `wasm-opt -Oz` makes the gzip larger (1,205 KB), and opt-level `s` or `2` are larger too.

The storage-free budget was therefore raised from 600 KB to 1.2 MB gzip on 2026-11-11, by the user's choice, so the CI check passes and guards against growth. Reaching the original 600 KB needs a storage contract with a non-SPARQL read path and a lazily loaded SPARQL module, plus lazily loaded YAML and RDF readers: a change to the `storage` capability, left as follow-up work.

## Test-only WebAssembly exports live in a test build

Step traces, differential records and the W3C IRP suite runner are exported only by a test build of `vertep-wasm` (features `trace` and `conformance`). That build is written to `bindings/js/.native/wasm-test` and is never published.

- motivated_by:: [[decisions#Separate packages keep the bundle budget]] {weight: primary, since: "2026-11-11"}

The published core, journal and ECMAScript builds hold the same core and datamodels without the harnesses, which saves 38 KB gzip in the core build. The IRP runner takes the suite's documents as text because the target has no file system. It reuses `vertep-conformance`'s host and verdicts, and its verdicts must equal the native harness's.

## Agent machines record their core macrosteps on request

`AgentMachine::start_with` and `StartOptions::record_core` record each core macrostep of the actor's session with its answer tape (`CoreTape`, the `vertep-oracle` record format), so agent runs replay through the Lean oracle.

- motivated_by:: [[decisions#The Lean model sees the datamodel as an answer tape]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary}

Recording is opt-in, so `start` and hosts pay nothing. Only the core is recorded: offers, decision validation on the probe datamodel and `vt_usage` binding stay outside the session, and plan charts run their own sessions; the events the agent layer raises are the inputs of the macrosteps they cause. A macrostep that runs out of the microstep budget is recorded with its partial trace and flagged; `StartOptions::microstep_budget` lets generated runs use the small budget of the plain differential runs, so eventless loops end quickly.

## The decision oracle checks payload conformance itself

`vertep-decide` decides whether a decision payload conforms to the candidate's schema with its own port of the agent's payload validator, instead of taking the answer from the Rust run.

- motivated_by:: [[decisions#Offer-time decider safety]] {weight: primary, since: "2026-11-11"}
- motivated_by:: [[decisions#Lean verification bridge]] {weight: secondary}

Each observation line carries the candidate's JSON Schema and the payload. The port covers the keywords of the type system's JSON Schema projection except `pattern`, which it reports as an input error, so the generated payload types avoid `decimal`, `datetime` and `iri`. Whether selection still takes the offered transition remains an answer from the Rust run.

## Documentation site is plain static HTML

The website in `site/` is hand-written HTML with one stylesheet and SVG marks: no generator, no scripts, relative links only, so it opens from `file://` and hosts on any static server.

- motivated_by:: [[glossary#Vertep]] {weight: secondary, since: "2026-11-12", note: "the Alatyr star and the vertep goat are its marks"}

The look is woodcut gravure in three inks, black, white and orange, with paper and ink swapped in dark mode; the tokens and components are in `site/assets/css/vertep.css`. Code on the pages is taken from files in the repository, and command output is pasted from real runs, so a page that disagrees with the code is a bug in the page. The example pages are written by `site/tools/sync-examples.py` from the sources and recorded outputs that `cargo test -p vertep-examples` checks; the pages are committed, so viewing the site still needs no build. The workflow `.github/workflows/pages.yml` deploys `site/` unchanged to GitHub Pages at https://volland.github.io/vertep/ on every push that touches it; package READMEs and crate metadata (`homepage`) link there, and load the logo from it.

## Legal pages mirror the author's own site

The site carries a German Impressum, Datenschutzerklärung and AGB, plus an About page, all taken from pavlyshyn.me, and links to lean4knowledge.com and pavlyshyn.me from every footer.

- motivated_by:: [[decisions#Documentation site is plain static HTML]] {weight: secondary, since: "2026-11-12"}

The legal identity (name, Berlin address, supervisory authority) is the one on pavlyshyn.me, so the three sites state one provider. Like the author's site, these pages load no third-party resource: the fonts are self-hosted woff2 files in `site/assets/fonts/`, declared in `site/assets/css/fonts.css`, so the only personal data is the GitHub Pages server log and the Datenschutz says nothing more. The AGB cover consulting, architecture and training services only; the software stays under MIT or Apache-2.0. `about.html` is built from the author's home page, CV and book list. The footer and the main navigation, including those written by `site/tools/sync-examples.py`, link all four pages.

## Site code is highlighted at build time

Rust, XML (SCXML) and YAML blocks on the site are highlighted by `site/tools/highlight.py`, which writes plain `<span class="k">` markup into the committed pages, so the pages still need no script.

- motivated_by:: [[decisions#Documentation site is plain static HTML]] {weight: primary, since: "2026-11-12"}

Pygments does the lexing and is needed only when the tool runs, never to view the site; the tool is idempotent and leaves the text of every block unchanged. `site/tools/sync-examples.py` calls the same function, so regenerated example pages keep their colours. The theme is in the "code highlighting" section of `site/assets/css/vertep.css`: ink and the rubric orange only, separated by weight and slant too, and it follows the dark-mode tokens. Blocks of other languages and the recorded outputs stay plain.

## Hosts instantiate CEL charts typed

Every host creates a chart's datamodel through `DatamodelRegistry::create_for`, and `CelRegistry` instantiates a chart that declares types with its projected CEL type environment, refusing one with CEL type errors.

- motivated_by:: [[decisions#Datamodel typers plug into one core checker]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Own CEL interpreter in vertep-dm-cel]] {weight: secondary}
- motivated_by:: [[decisions#The Lean model sees the datamodel as an answer tape]] {weight: secondary, note: "the payload check stays outside the kernel and its tape"}

Before, only `vt typecheck` checked CEL against the chart's types: the actor runtime, the agent host and the WebAssembly worker created an untyped CEL datamodel by name, so a `decimal` field initialised to `"0"` held an int, decimal payload fields stayed strings, and a chart with CEL type errors ran anyway. Now the projection (`cel_type`, `type_env`) and the typed load (`instantiate`: the core checker, then every CEL expression against the environment) live in `vertep-dm-cel`, and `vertep-cli` re-exports them. `vertep-core` gives the registry a `create_for(chart)` and a `check(chart)` with name-based defaults, and `create_datamodel_for` turns a refusal into `StartError::ChartRefused`. The runtime refuses such a chart when it is registered (`RegisterError::Refused`, `ChartInvalid` in JavaScript) and the agent host when an actor is spawned (`InstantiateError::Start`). A chart without type declarations still gets a fresh untyped datamodel, so it runs exactly as before typing.

A payload sent with a declared event must convert to the declared type: hosts pass each event through `datamodel::admit`, which asks `Datamodel::check_event` and delivers `error.execution` instead of an event whose payload fails, such as a union value without the payload its tag requires. Platform events are not checked, because the platform defines their payloads ([[decisions#Agent error events extend the error payload]]). The check is a host step rather than part of the kernel, so the interpreter, its datamodel answer tape and the Lean model are unchanged.

## The chart checker types the values the core generates

`check_chart` checks the values the interpreter writes or raises without an expression of the chart: `undefined` of a bare `<assign>`, `idlocation` identifiers, `foreach` items and indices, and platform event payloads.

- motivated_by:: [[decisions#Datamodel typers plug into one core checker]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[constraints#Typed charts fix the error payload]] {weight: secondary}

A bare `<assign>` stores `undefined`, so its location must be optional or `value`; the `idlocation` of a `<send>` or `<invoke>` receives a string identifier; a `foreach` item location must accept the array's item type and an index location an `int`; a `namelist` entry is keyed by the whole location, as `eval_payload` builds the payload (the checker used to read `namelist="ctx.total"` as the field `total`); a declared `error.execution` or `error.communication` must accept `{ kind: string, message: string }`, and a declared `done.state.<id>` must be optional, since a failed `<donedata>` and a parallel state raise it without data, and must accept the `<donedata>` of the final children of `<id>`. The Lean type soundness theorem found these (verification-backlog 4.4): the checker accepted charts whose own generated values broke a typed location or a declared payload, which the CEL datamodel's checked conversion turned into `error.execution` at runtime. Regressions: `bare_assign_to_a_required_location_is_rejected`, `idlocation_must_hold_a_string`, `foreach_item_and_index_locations_are_checked`, `namelist_entries_are_keyed_by_the_whole_location` and `declared_platform_events_accept_the_platform_payloads` in `crates/vertep-core/src/types/tests.rs`.

## Type soundness is stated relative to the checker's typing

The Lean type soundness theorem takes a chart's static typing as data and states the checks of `check_chart` as properties (`ChartOk`); the datamodel's evaluation enters as the assumption `TapeTyped` on the answer tape.

- motivated_by:: [[decisions#The Lean model sees the datamodel as an answer tape]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Datamodel typers plug into one core checker]] {weight: secondary}

The Lean `step` has no expression typer, and the typers belong to the datamodels, so the theorem is relative to what the checker computed: the declared types of locations and events and the static type of each expression, inline content and `foreach` array. A typed tape (an answer to a request for an expression of static type `T` is a `T`) is what the typed CEL and `rust` datamodels promise; a datamodel kept typed by typed requests is what their checked conversion on every store (`CelDatamodel::store`, `Context::set`) gives. Three limits are explicit in `VertepLean/TypedStep/Basic.lean`: expression types are given per expression text, so an expression read under event scopes with different payload types needs a type valid in all of them; a value whose expression or content the typer cannot type (`Unknown`) is left to the datamodel's checked conversion, as the Rust checker leaves it; and a `<send eventexpr>`, whose event name is known only at runtime, must carry a payload that fits every declared event. The agreement of Rust `is_assignable` with assignability of values (`Sub`) is not proven.

## Core errors are standard errors

`vertep-core` and `vertep-dm-cel` errors implement `Display` and `core::error::Error`, so callers can use `?` into `Box<dyn Error>`; chart validation keeps returning `Vec<Violation>`, and `Violations` wraps it as one error.

- motivated_by:: [[decisions#No-std core and tokio shell]] {weight: secondary, since: "2026-11-12", note: "core::error::Error keeps the core no_std"}

`StartError`, `StepError`, `EvalError`, `Violation` and `Violations` (`From<Vec<Violation>>`) in the core, and `CelError`, `LoadError` and `LoadErrors` in the CEL datamodel. Messages are short and lowercase and name the offending state, transition or location. `StartError::source` returns the datamodel's `EvalError`. `CelValue` also derives `PartialEq`, which is structural equality (`Int(1) != Uint(1)`, NaN unequal to itself), not CEL `==` (`cel_eq`), and `ir::Data` derives `Default`. Existing signatures are unchanged, so the change is additive; it came out of writing the crate READMEs, whose examples had to format every error by hand.

## Vertep uses third-party relays only

M5 position: Vertep is a client of existing DIDComm mediators and TSP intermediaries and ships only in-process test relays for CI (M5 design, non-goals).

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary, since: "2026-11-11"}

Superseded for TSP by [[decisions#Vertep ships its own TSP mediator]]; it still holds for DIDComm, where `MediatorClient` talks to third-party mediators such as Identus.

## Durable objects are DID virtual actors with verifiable journals

A durable object is a virtual actor with a `did:webvh` address and a hash-chained, signed journal; its state stays the chart's datamodel, written only by journaled turns (M7 design D1).

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#State changes travel in the append]] {weight: primary}
- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: secondary}

Rejected: a key-value storage API beside the chart, which would be a second state model outside replay, certificates and time travel. `did:key` and `did:peer` are refused for durable objects because neither can change custodians or endpoints without changing the address.

## Journal entries are hash-chained and host-signed

Each durable-object entry states, inside its graph, the previous entry's hash, object DID, position, custody epoch, writer DID, its content hash and the writer's signature over it (M7 design D2).

- motivated_by:: [[decisions#Durable objects are DID virtual actors with verifiable journals]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Keys stay behind a key manager]] {weight: primary}

The hash is SHA-256 over the sorted N-Triples lines of the graph without the seal statements; an entry with blank nodes is relabelled with RDFC-1.0 first. Keeping the link in the graph means every journal store round-trips it untouched. Rejected for now: a Merkle transparency log (signed checkpoints can add Merkle proofs later), RDFC-1.0 on every entry (too slow per turn) and metadata in `AppendMeta`, which every store would have to learn.

## Custody epochs are won by quorum promises

A host becomes writer of epoch e after a majority of custodians promise e; it adopts the chain accepted in the highest epoch, then the longest, and an entry commits once a quorum accepts it in one epoch (M7 design D3).

- motivated_by:: [[constraints#Clock skew beyond the safety margin breaks lease exclusivity]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Fencing epochs on journal appends]] {weight: primary}
- motivated_by:: [[decisions#A standby's fence rises only with entries it stores]] {weight: secondary}
- motivated_by:: [[decisions#Unreplicated entries stay unreadable until the quorum catches up]] {weight: secondary}

This is Paxos over the whole log, with the epoch as ballot and the custodian's chain as accepted value, so an entry from an older epoch is never declared committed by counting copies alone: it commits when a quorum re-accepts it in the new epoch. Runtime replay decodes every entry as a turn, which is why there is no Raft-style epoch-start entry; promises go to the lineage graph. Rejected: leases from etcd or Kubernetes (a shared authority is what crossing organisations removes), time-based leases (clock skew) and anchoring on a ledger (cost and latency, not needed for single-writer safety). Duelling candidates only delay progress; a randomised, growing back-off resolves them.

## Replication rides TSP through a replica link

The cluster's suffix-shipping replication is factored over a replica link; custody uses a TSP link between the writer's host VID and each custodian's VID, direct or through a mediator (M7 design D4).

- motivated_by:: [[decisions#TSP before DIDComm across trust boundaries]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#TSP direct when reachable, routed otherwise]] {weight: secondary}
- motivated_by:: [[decisions#Optional standby journal replication]] {weight: secondary, note: "the same shipping and fence rules"}

Rejected: a separate custody protocol over HTTP or gRPC, which would duplicate the authentication and confidentiality TSP gives and bypass grants.

## Keepers store ciphertext under a per-object content key

Keeper replicas hold a clear header (DID, position, epoch, previous hash, signature) and an encrypted body; the object's content key is wrapped to every host and rotated when a host leaves (M7 design D5).

- motivated_by:: [[decisions#Journal entries are hash-chained and host-signed]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Keys stay behind a key manager]] {weight: secondary}

The header is enough to order, store and verify. A removed host can still read entries written before its removal; that limit is accepted. Rejected: clear replicas at every custodian, which would make storage-only parties as trusted as hosts.

## Equivocation is kept as evidence

Two validly signed entries for one object, epoch and position with different hashes are recorded as evidence, the epoch is refused and the controller is told; the protocol detects a cheating host but does not tolerate it (M7 design D6).

- motivated_by:: [[decisions#Journal entries are hash-chained and host-signed]] {weight: primary, since: "2026-11-12"}

A different entry at the same position from a higher epoch is the normal replacement of an uncommitted tail, not equivocation. Byzantine fault tolerance among hosts is out of scope.

## Custodian set changes are replicated, then published

A controller-signed reconfiguration is custody state replicated with every batch; it takes effect once a quorum of the old set accepts it and the new custodians hold the chain, and then the `did:webvh` log is updated (M7 design D7).

- motivated_by:: [[decisions#Custody epochs are won by quorum promises]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[constraints#did:webvh updates need the web host]] {weight: secondary}

Promises carry the accepted configuration, so a later candidate adopts the configuration accepted in the highest epoch and prepares again with it when it is newer. One change at a time; Raft-style joint consensus is not needed at this rate of change.

## The agent journal is stored in a journal store

`StoreAgentJournal` keeps each agent input as an entry of the agent's journal in a `JournalStore`, appended at head plus one; `MemoryJournal` stays the default for tests (M7 design D8).

- motivated_by:: [[decisions#The agent host journals agent inputs]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Two storage traits with tiramemsu as default]] {weight: secondary}

An agent declared as a durable object gets the custody journal, which is a `JournalStore` too.

## Vertep ships its own TSP mediator

`vertep-tsp-mediator` and its `vt-mediator` binary route TSP messages and keep store-and-forward mailboxes, because TSP 1.0 defines none and no TSP mediator exists to depend on (M7 design D9).

- supersedes:: [[decisions#Vertep uses third-party relays only]] {since: "2026-11-12", reason: "TSP 1.0 has no store-and-forward; durable objects need mailboxes while every host is offline"}
- motivated_by:: [[constraints#TSP provides no delivery guarantee]] {weight: primary, since: "2026-11-12"}

DIDComm stays client-only against third-party mediators through Pickup 3.0.

## TSP pickup is a versioned protocol inside TSP

Registration, status, delivery, acknowledgement and live sessions are TSP application messages typed under a versioned Vertep IRI, sent over the relationship between a VID and its mediator (M7 design D10).

- motivated_by:: [[constraints#TSP provides no delivery guarantee]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[constraints#TSP routing needs neighbour relationships]] {weight: primary, note: "registration is the last-hop neighbour relationship"}
- motivated_by:: [[decisions#Remote mediators through Pickup 3.0]] {weight: secondary, note: "same semantics as DIDComm Pickup 3.0"}

Rejected: HTTP polling outside TSP (unauthenticated and it leaks who picks up) and holding messages in TSP relationship control messages, which the spec does not allow.

## The mediator stores opaque sealed bytes in its own journal

The mediator stores the re-sealed last-hop message per recipient VID in a store-backed drop box, accepts from the previous hop only after the append and deletes on acknowledgement (M7 design D11).

- motivated_by:: [[constraints#TSP intermediaries do not persist inner VIDs]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: primary}
- motivated_by:: [[decisions#Optional standby journal replication]] {weight: secondary, note: "replicated deployment"}

Its message id is a SHA-256 digest of the post as the previous hop sent it, so a retried post is stored once even though the re-sealed bytes differ. Receivers deduplicate by the envelope's own message id inside the ciphertext. Acknowledged ciphertext stays in the journal history until `vt-mediator compact` rewrites the store.

## Live delivery is a WebSocket binding of pickup

A live request upgrades to a WebSocket on which the mediator pushes batches and the client acknowledges after its journal append; DIDComm mailboxes get the same binding for Pickup 3.0 live mode (M7 design D12).

- motivated_by:: [[decisions#WebSocket is a transport and an Event I/O Processor]] {weight: secondary, since: "2026-11-12"}
- motivated_by:: [[decisions#DIDComm mailboxes acknowledge after the append]] {weight: primary}

## Hosts share the object VID and only the writer acknowledges

Every host of a durable object holds the object's TSP keys in its key manager, so any host can open its messages; only the current writer journals and acknowledges them (M7 design D13).

- motivated_by:: [[decisions#Custody epochs are won by quorum promises]] {weight: primary, since: "2026-11-12"}
- motivated_by:: [[decisions#Keys stay behind a key manager]] {weight: secondary}

Rejected: one VID per host behind a routing indirection, which would make senders follow custody changes. Keys rotate through the `did:webvh` log when a host leaves.
