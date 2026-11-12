## 1. Graph first: components, milestone, decisions

- [x] 1.1 Add `M7 Durable objects` to `tg.md/roadmap.md` (typed `Milestone`) with `precedes::` edges from M3, M4 and M5, and add it to the roadmap Mermaid diagram
- [x] 1.2 Add Component sections `vertep-objects` and `vertep-tsp-mediator` to `tg.md/components.md` with `provides::` (durable-objects, object-custody, tsp-mediator), `uses::` (vertep-runtime, vertep-store, vertep-federation, vertep-cluster, vertep-tiramemsu), `realizes::` and `scheduled_in:: [[roadmap#M7 Durable objects]]`, before any code lands
- [x] 1.3 Add Capability sections `Durable objects`, `Object custody` and `TSP mediator` to `tg.md/capabilities.md` and list `durable-objects`, `object-custody` and `tsp-mediator` in its `openspec:` frontmatter
- [x] 1.4 Record design decisions D1–D13 as `Decision` sections in `tg.md/decisions.md` with `motivated_by::` edges as named in design.md; D9 gets `supersedes::` against the M5 "client of existing mediators" position (add that position as a Decision section first if it is not one yet)
- [x] 1.5 Record the constraints `TSP provides no delivery guarantee`, `TSP intermediaries do not persist inner VIDs`, `TSP routing needs neighbour relationships` and `did:webvh updates need the web host` in `tg.md/constraints.md` with `source:` and `constrains::` edges ({strength: must|should}) to the decisions and requirements design.md names
- [x] 1.6 Add glossary entries `Durable object`, `Custodian`, `Custody epoch` and `Keeper`, and extend `Mailbox` with the TSP mediator

## 2. Verifiable journal entries

- [x] 2.1 Carry the chain metadata (`prev`, object DID, epoch, writer DID, hash, signature) as `vt:` statements inside the entry graph so stores keep it opaque without changes (design D2, refined), with round-trip tests on the memory and tiramemsu journal stores (oxigraph is a graph store only, with no journal store)
- [x] 2.2 Implement canonical entry hashing (JCS) and signing through `KeyManager` in a new `vertep-objects` crate (feature `objects`)
- [x] 2.3 Implement chain verification from genesis with the failure reasons `hash-mismatch`, `bad-signature` and `not-a-host`
- [x] 2.4 Annotate with `@tg: implements:: [[openspec:durable-objects#Verifiable Journal Entries]]`; tests for the scenarios `Chain verified from genesis`, `Tampered entry rejected` and `Unauthorised signer rejected` carry `@tg: verifies::` {level: unit}

## 3. Durable agent journal

- [x] 3.1 Implement `StoreAgentJournal` over `JournalStore` in `vertep-agent` (CAS at head plus one, the same JSON text as `MemoryJournal`) and let `AgentHost::builder` accept it
- [x] 3.2 Run the existing `deciders`, `llm_actors` and `plan_charts` test suites against both journals
- [x] 3.3 Tests: `Agent survives a restart` (reopen the tiramemsu file, open offer keeps its correlation id, no provider call) and `Fork from stored journal` (`@tg: verifies::` {level: integration}); annotate `implements:: [[openspec:durable-objects#Durable Agent Journal]]`
- [x] 3.4 Remove the "no durable agent journal" limitation from `tg.md/views/agents.md#Implementation in vertep-agent` and describe the store-backed journal

## 4. Replication over a link trait

- [x] 4.1 Factor `ReplicatedJournal` and `StandbyReplica` in `vertep-cluster` over a `ReplicaLink` trait; keep the QUIC link as the cluster implementation
- [x] 4.2 Prove the refactor changes nothing: the cluster fault-injection suite and the Lean lease-model replay pass unchanged

## 5. Custody protocol

- [x] 5.1 Parse and validate the custodian set and quorum from the DID document (`Custodian Set In The DID Document`); refuse quorums below a majority and `did:key`/`did:peer` objects (`Durable Object Identity`)
- [x] 5.2 Define the custody envelope types (`prepare`, `promise`, `refuse`, `append`, `ack`, `read`, `entries`, `evidence`) under `https://w3id.org/vertep/custody/1.0#` and declare them, with the `vt:` custody terms, in the objects ontology module (`crates/vertep-objects/ontology/custody.ttl`)
- [x] 5.3 Implement the custodian side: highest-promised epoch per object, signed promises with head and last epoch, stale-tail replacement from higher epochs, refusing lower epochs
- [x] 5.4 Implement the candidate side: prepare to all custodians, quorum of promises, chain and configuration adoption (highest accepted epoch, then longest), re-acceptance of the adopted chain in the new epoch, promises recorded for lineage, randomised exponential back-off
- [x] 5.5 Implement the writer: suffix shipping over a `ReplicaLink`, commit when a quorum accepted in the writer's epoch (inherited entries included), effects gated on commit (`Quorum Before Effects`, `Effects After Durable Commit`)
- [x] 5.6 Implement the TSP `ReplicaLink` over the relationship between host and custodian VIDs, direct or through a mediator
- [x] 5.7 Implement activation from verified replicas, skipping invalid ones (`Activation From A Verified Replica`)
- [x] 5.8 Implement planned handoff (quiesce after a committed entry, target wins the next epoch) and failover without a shared authority
- [x] 5.9 Implement equivocation detection, evidence journaling into the custodian's graph and `custody.evidence` to the controller
- [x] 5.10 Implement controller-signed reconfiguration as replicated custody state, catch-up of new custodians, the switch to the new quorum and publication through the `did:webvh` log (`Custodian Set Changes`)
- [x] 5.11 Implement keeper encryption: a per-object content key wrapped (HPKE) to hosts in a custody entry, a clear header and a ciphertext body, rotation on host removal (`Encrypted Replicas At Keepers`)
- [x] 5.12 Share the object's TSP keys among hosts through their key managers, and acknowledge mediator pickups only from the writer (D13)
- [x] 5.13 Annotate every `object-custody` and `durable-objects` requirement with `@tg: implements::`

## 6. Durable objects in the runtime

- [x] 6.1 Add a durable-object host to `vertep-objects` that plugs into the runtime's virtual-actor activation, with the custody journal as the actor's `JournalStore`
- [x] 6.2 Keep timers in the journal across handoff, fire overdue timers on activation and never fire a journaled one twice (`Durable Alarms`)
- [x] 6.3 Record custody epochs, hosts, custodian sets and position ranges in the runtime graph (`Durable Object Lineage`), with a SPARQL test
- [x] 6.4 Add an example under `crates/vertep-objects/examples`: a durable support agent with three custodians in three organisations, a mediator, and failover of the writer's organisation

## 7. TSP mediator

- [x] 7.1 Create `vertep-tsp-mediator` (library plus the `vt-mediator` binary) on tokio and axum, with its own DID document, TSP keys through `KeyManager`, and configuration from a file and environment (`Mediator Operability`)
- [x] 7.2 Implement the routed-mode intermediary over HTTP POST using `forward_routed`: middle hop, last hop to an online recipient, refusal for unknown exits (`TSP Routed-Mode Intermediary`)
- [x] 7.3 Implement registration over a TSP relationship (`register`, `registered`, `unregister`) and operator revocation (`Mailbox Registration`)
- [x] 7.4 Implement the durable mailbox store: event-sourced mailbox journals (`MailboxStore`) over any journal store, tiramemsu shard files by default, one writer per file, accepting from the previous hop only after the append (`Durable Store-And-Forward`)
- [x] 7.5 Implement the pickup protocol (`status`, `deliver` and `batch`, `received`) under `https://w3id.org/vertep/tsp/mailbox/1.0#`, with delete-on-received and owner-only pickup (`Pickup Protocol`)
- [x] 7.6 Implement the WebSocket live session with the same acknowledgement rule (`Live Delivery Session`)
- [x] 7.7 Implement per-VID quotas, size limits, retention expiry and expiry reporting (`Retention And Quotas`)
- [x] 7.8 Enforce metadata minimisation: stored fields only, no content logging, and a test that inspects the store (`Metadata Minimisation`)
- [x] 7.9 Shard mailboxes by recipient VID with M4 standby replication for a replicated deployment, plus a node-loss test (`Replicated Mediator Deployment`)
- [x] 7.10 Add health and per-mailbox metrics
- [x] 7.11 Run interop in CI against the `tsp_sdk` reference wallet and intermediary: a reference sender through `vt-mediator` to a reference receiver (scenario `Interop with the reference implementation`)
- [x] 7.12 Add a container image and a Docker compose example of two mediators in a route
- [x] 7.13 Annotate every `tsp-mediator` requirement with `@tg: implements::` and every scenario's test with `@tg: verifies::`

## 8. Relay mailbox client

- [x] 8.1 Implement the TSP pickup client as a `PickupSource` in `vertep-federation` (feature `tsp`), acknowledging after the journal append (`TSP Mediator Mailbox`)
- [x] 8.2 Implement live delivery over WebSocket for TSP and for DIDComm Pickup 3.0 `live-delivery-change` (`Live Delivery Over WebSocket`)
- [x] 8.3 Tests for `Message waits for a deactivated actor`, `Pickup activates and acknowledges after the append`, `Crash between append and acknowledgement`, `Live TSP session` and `Session reconnects`, each with `@tg: verifies::` {level: integration}
- [x] 8.4 Update `tg.md/views/distribution.md#Relay mailboxes` and `tg.md/capabilities.md#Relay mailboxes` for the mediator mailbox and live delivery

## 9. Scenario tests

- [x] 9.1 One test per scenario of `durable-objects`, each carrying `@tg: verifies:: [[openspec:durable-objects#<Requirement>#<Scenario>]]` with a level
- [x] 9.2 One test per scenario of `object-custody`, run on an in-process `LocalNetwork` with fault injection (partitions, delays, crashes, clock offsets) and replayable fault histories
- [x] 9.3 One test per scenario of `tsp-mediator`
- [x] 9.4 A randomised custody fault test: many seeds, checking one writer per epoch, no lost committed entry and no effect from uncommitted entries

## 10. Formal model

- [ ] 10.1 Model custody in Lean (`vertep-lean`): prepare and promise, append and ack, chain adoption and reconfiguration (OPEN: everything but reconfiguration is modelled in `VertepLean/Custody.lean`; custodian-set changes are not)
- [x] 10.2 Prove one writer per epoch and that every committed entry appears in every later writer's chain
- [x] 10.3 Replay the custody fault histories from 9.2 and 9.4 through the Lean model in strict mode, as the lease model does

## 11. Views and closing checks

- [x] 11.1 Write `tg.md/views/durable-objects.md` (typed `View`, `openspec: [durable-objects, object-custody, tsp-mediator]`) with `describes::` edges and Mermaid diagrams for the architecture, the custody epoch sequence, the mediator store-and-forward and the pickup sequence (no `[[` or `]]` inside Mermaid)
- [x] 11.2 Add the new view to `tg.md/views/views.md`
- [x] 11.3 Update the public site and package READMEs for `vertep-objects` and `vertep-tsp-mediator`
- [x] 11.4 `tg check`, `tg ontology check` and `tg trace durable-objects object-custody tsp-mediator relay-mailboxes --gaps` show no new gaps
