## 1. Workspace and setup

- [ ] 1.1 Delete the M0 integration spike code and record its findings in design.md Open Questions or tg.md decisions
- [ ] 1.2 Add crates `vertep-store`, `vertep-tiramemsu`, `vertep-oxigraph`, `vertep-runtime` and the facade `vertep` to the workspace with `unsafe_code = "forbid"`; add `tokio`, `tiramemsu`, `oxigraph` and an RDFC-1.0 canonicalizer as workspace dependencies
- [ ] 1.3 Add the `Spawn`, `Clock` and `Sleep` shell traits with a tokio implementation and a deterministic test implementation (virtual clock)

Every implementation task below includes: annotate the realizing declaration with `// @tg: implements:: [[openspec:<capability>#<Requirement>]]` (with `{confidence: partial}` while incomplete), and annotate one test per scenario with `// @tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]] {level: unit|integration}` directly above the test.

## 2. Runtime ontology (vertep-rdf)

- [ ] 2.1 Runtime Vocabulary: write the `vt:` runtime module (actor classes, address, mailbox, activation, journal entry, macrostep, snapshot, timer, migration, dead letter) with annotations for `[[openspec:runtime-ontology#Runtime Vocabulary]]`
- [ ] 2.2 Runtime SHACL Shapes: write one shape per class and a validation helper; annotate `[[openspec:runtime-ontology#Runtime SHACL Shapes]]` and its scenarios
- [ ] 2.3 PROV-O Alignment Without Dependency: add the subclass and subproperty alignment triples; test the PROV-O query on an RDFS-aware store and the runtime queries on a store without inference
- [ ] 2.4 Triple Term Annotations: encode annotations as RDF 1.2 reifiers
- [ ] 2.5 Stable Runtime IRIs: derive entry, snapshot, timer and dead-letter IRIs from address and position or send id; property test that re-projection yields equal IRIs
- [ ] 2.6 Named Graph Layout: write current state into the actor's graph and entries into entry graphs

## 3. Storage contracts and test kit (vertep-store)

- [ ] 3.1 Journal Store Contract: define `JournalStore` (append with expected position, read from position) over RDF dataset entries
- [ ] 3.2 Graph Store Contract: define `GraphStore` (quads in, SPARQL out, RDF 1.2 capability flag)
- [ ] 3.3 Shared Backend Test Kit: write the backend test kit (atomic multi-entry append, concurrent conflicting appends, read-after-append, quad round trip, declared capability) as a reusable test module
- [ ] 3.4 RDF 1.1 Reification Fallback: implement the `rdf:Statement` encoding and decoding of annotations, selected by the capability flag
- [ ] 3.5 Eventual Projection To External Graph Stores: implement the projector with a per-actor projected position and idempotent writes
- [ ] 3.6 Append Fencing Token: add optional append metadata with a (scope IRI, epoch) fencing token and a declared fencing capability; implement highest-epoch-per-scope rejection in the tiramemsu backend (one cardinality-one epoch statement per scope, checked in the append transaction); add fencing cases to the test kit. Clustering itself stays in M4

## 4. Tiramemsu backend (vertep-tiramemsu)

- [ ] 4.1 Tiramemsu Backend: implement `JournalStore` and `GraphStore` on one `tiramemsu::Db`; pass the test kit
- [ ] 4.2 One File Per Shard: open one `Db` per shard file and route by the hash of the address IRI; test that appends in different shards do not wait on each other
- [ ] 4.3 Input And State In One Transaction: in one `Db::transact`, check the `vt:head` position, assert the entry graph, write state with `Tx::derive`, retract exited states and advance the head
- [ ] 4.4 State Facts Carry Their Cause: test that `View::dependents` and support links lead from an active-state statement to its entry
- [ ] 4.5 Time Travel Views: expose `now` and `as_of` views of the current-state graph; test state as of a past transaction
- [ ] 4.6 Benchmark appends per second per shard and replay speed, and record the default shard count and snapshot interval in design.md

## 5. Oxigraph and SPARQL adapter (vertep-oxigraph)

- [ ] 5.1 SPARQL Graph Store Adapter: implement `GraphStore` on embedded Oxigraph and on a remote SPARQL 1.1 endpoint; pass the graph half of the test kit
- [ ] 5.2 Configure tiramemsu journal plus Oxigraph graph store in an integration test with projection catch-up after an outage

## 6. Event journal (vertep-runtime)

- [ ] 6.1 Journal Of Accepted Inputs and Journal Entry Content: build entries from `step` effects (input, message id, origin, chart version, sends, timers, configuration digest, position)
- [ ] 6.2 Compare-And-Set Append: append with expected position; on conflict step down without dispatching
- [ ] 6.3 Message Deduplication: check message ids against the journal before running `step`
- [ ] 6.4 Transactional Outbox: dispatch sends only after append; retry undispatched sends on activation with original ids
- [ ] 6.5 Deterministic Replay: replay with recorded versions, compare digests, fail activation on divergence, never re-dispatch replayed sends
- [ ] 6.6 Inputs Only From The Journal: stamp time at dequeue, seed randomness from the entry IRI, forbid other inputs in executable content
- [ ] 6.7 External Results As Inputs: journal invoke results (including LLM completions) as full input events
- [ ] 6.8 Snapshots: write and validate snapshots at a configurable interval, fall back on invalid snapshots
- [ ] 6.9 Journal Read Access: implement state-as-of-position and fork-at-position

## 7. Actor model (vertep-runtime)

- [ ] 7.1 Actor Trait: define the actor contract and adapt M0 charts, async functions and streams to it
- [ ] 7.2 IRI Actor Addresses: validate addresses and bind `#_parent`, `_sessionid` and invoke targets to IRIs
- [ ] 7.3 Virtual Actor Identity: derive addresses from chart type IRI and key
- [ ] 7.4 Virtual Actor Activation: activate on message from snapshot plus tail, one activation per node
- [ ] 7.5 Virtual Actor Deactivation: idle timeout with pending-work checks
- [ ] 7.6 Virtual Actors Are Not Invokable: reject at validation and at runtime
- [ ] 7.7 Invoked Actor Lifecycle: start at macrostep end, cancel on exit, `done.invoke` on completion
- [ ] 7.8 Stateless Worker Lifecycle: bounded pool per address, no journal
- [ ] 7.9 Turn Is One Macrostep: one external event per turn, durable before the next
- [ ] 7.10 Supervision With Restart: `vt:restart` policy, limit, window and exponential backoff
- [ ] 7.11 Actor Failure Isolation: catch panics per turn, leave the journal unchanged and the message unacknowledged

## 8. Messaging (vertep-runtime)

- [ ] 8.1 Pluggable Mailboxes and Mailbox Ordering: define `Mailbox`; implement the in-memory and the store-backed mailbox
- [ ] 8.2 Mailboxes Outlive Activations: activation trigger from the store-backed mailbox; durable across restart
- [ ] 8.3 Pluggable Transports: define `Transport`; register the local transport as an Event I/O Processor and list it in `_ioprocessors`
- [ ] 8.4 Address Resolution: implement `Directory` with scheme-keyed resolvers and the local shard resolver
- [ ] 8.5 Event Origin: fill `_event.origin` and `_event.origintype`
- [ ] 8.6 Non-Blocking Request: lower `vt:request` into send, waiting substate, reply transition and timeout timer in the IR
- [ ] 8.7 Deterministic Correlation Ids: derive from address, position and request index
- [ ] 8.8 Request Timeout: durable timer raising `error.timeout`; late replies go to the unhandled policy
- [ ] 8.9 Duplicate Reply Suppression: drop a second reply per correlation id
- [ ] 8.10 Deferred Events: deferral-aware receive in the mailbox, order-preserving, surviving deactivation
- [ ] 8.11 Unhandled Event Policy: `discard`, `dead-letter`, `error` with per-class defaults
- [ ] 8.12 Queryable Dead Letters: write `vt:DeadLetter` resources per receiver graph; test the SPARQL query
- [ ] 8.13 External Blocking Ask: blocking ask with timeout for callers outside the actor system only

## 9. Durable timers (vertep-runtime)

- [ ] 9.1 Delayed Send Is Journaled: record timers in entries
- [ ] 9.2 Timer Fires As Input: per-shard timer service, message id from send id
- [ ] 9.3 Timer Cancellation: journal cancellations, ignore late cancels
- [ ] 9.4 Timers Reactivate Virtual Actors
- [ ] 9.5 Timer Time Source: due times from journaled timestamps; replay never re-fires
- [ ] 9.6 Recurring Reminders: `vt:reminder` registration, periodic firing, stop at final
- [ ] 9.7 Missed Timers After Downtime: fire in due-time order, collapse missed reminder occurrences

## 10. Chart versioning (vertep-runtime)

- [ ] 10.1 Version Recorded Per Journal Segment: version IRI on every entry
- [ ] 10.2 Replay With Recorded Version: per-segment replay, missing-version error
- [ ] 10.3 Chart Registry: versions keyed by IRI, removal refused while in use
- [ ] 10.4 Pin Until Final: existing actors pinned, new actors on latest
- [ ] 10.5 Explicit Migration: migration entry with snapshot before, migration IRI, snapshot after; atomic
- [ ] 10.6 Migration Compatibility Check: enumerate reachable configurations per region, refuse unmapped ones, derive identity mappings
- [ ] 10.7 Fork Before Migration
- [ ] 10.8 Self-Migration Requests: queue as runtime input, apply after the requesting entry

## 11. Facade (vertep)

- [ ] 11.1 Re-export the runtime, storage traits and tiramemsu backend behind cargo features; write a quickstart example with one virtual actor, a delayed send and a time-travel query

## 12. Lean journal protocol (vertep-lean)

- [ ] 12.1 Model journal entries, compare-and-set append, deduplication, the outbox and replay in Lean
- [ ] 12.2 Prove replay determinism: replaying a journal yields the recorded configurations and effects
- [ ] 12.3 Prove deduplication idempotence: processing a message id twice equals processing it once
- [ ] 12.4 Prove the outbox property: every recorded send is dispatched at least once and no unrecorded send is dispatched
- [ ] 12.5 Prove compare-and-set single activation: of two activations appending at the same expected position at most one succeeds
- [ ] 12.6 Add property tests in Rust that generate crashes, redeliveries and concurrent activations and compare the shell's journal and dispatch log with the Lean model as oracle

## 13. Documentation and traceability

- [ ] 13.1 Record any decision taken during implementation in `tg.md/decisions.md` as a `Decision` section with `motivated_by` edges, and new external limits in `tg.md/constraints.md` with `constrains` edges to the affected requirements, such as `- constrains:: [[openspec:event-journal#Compare-And-Set Append]] {strength: must}`
- [ ] 13.2 Update the capability sections in `tg.md/capabilities.md` for actor-model, messaging, event-journal, storage, durable-timers, chart-versioning and runtime-ontology with `specified_by` edges to their requirements, and update `tg.md/components.md` `provides` edges from `{confidence: partial}` to full where complete
- [ ] 13.3 Run `tg check`, `tg ontology check` and `tg trace actor-model messaging event-journal storage durable-timers chart-versioning runtime-ontology --gaps`, and close every gap they report
