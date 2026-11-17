## Why

A Vertep actor's durable history lives in one trust domain today. Cluster replication (M4) needs a shared membership authority (gossip, etcd or Kubernetes). The agent journal is in-memory only. TSP delivery assumes a reachable endpoint, because Vertep ships no TSP mediator and TSP 1.0 itself defines none.

Long-lived agents and workflows that span organisations need **durable objects** with these properties:

- an identity that does not belong to one operator;
- state that survives the loss of any one host or organisation;
- exactly one writer at a time, with no central coordinator;
- delivery that keeps working while every host is offline.

The ToIP Trust Spanning Protocol (Release 1.0) already gives verifiable identifiers, authenticated and confidential messages, and routing through intermediaries. It provides "no delivery guarantee" and defines no store-and-forward or pickup. This change builds decentralised durable objects on TSP and closes that gap with a TSP mailbox mediator of Vertep's own.

## What Changes

- **Durable objects.**
  - A durable object is a virtual actor with a DID (`did:webvh` by default) and a verifiable journal. Its address is stable while the host serving it changes.
  - Journal entries are hash-chained and signed by the writing host, so any party can check a replica without trusting the host it came from.
  - Turns, timers (alarms) and the outbox use the existing journal and durable-timer semantics. No new execution semantics are added.
- **Durable agent journal.** A `JournalStore`-backed `AgentJournal` replaces the in-memory-only default, so agents survive restarts and can be forked from disk. Agents become durable objects.
- **Decentralised custody.**
  - An object's DID document names its custodian set:
    - *hosts* may activate the object and write to it;
    - *keepers* store only encrypted entries.
  - A host becomes the single writer for a custody epoch by collecting promises from a majority of custodians. This works without etcd, gossip or any shared authority.
  - Entries are replicated to custodians as TSP messages and produce effects only after a quorum stored them.
  - Equivocation (two signed entries for one position in one epoch) is detected, and the evidence is kept.
  - Custody moves between hosts and organisations by handoff, or by failover when a host disappears.
- **TSP mailbox mediator.** A new crate `vertep-tsp-mediator` with a `vt-mediator` binary:
  - acts as a TSP 1.0 routed-mode intermediary;
  - provides store-and-forward mailboxes for registered VIDs;
  - speaks a Vertep pickup protocol carried inside TSP (register, status, deliver, received, live session);
  - offers live delivery over WebSocket;
  - stores durably and acknowledges only after the store;
  - enforces quotas and retention;
  - persists no payloads in clear and none of the nested VIDs TSP forbids it to persist.
- **Client side of the mediator.** The TSP relay mailbox gains a pickup client, so a DID actor whose `TSPTransport` service routes through a mediator receives while deactivated. Delivery uses the same acknowledge-after-append rule as DIDComm.
- **Supersedes** the M5 non-goal "Vertep is a client of existing mediators and ships only test relays", for TSP only. DIDComm stays client-only.

## Capabilities

### New Capabilities

- `durable-objects`:
  - DID-addressed durable objects;
  - the hash-chained, signed journal;
  - single writer per object;
  - activation on any host from a verified replica;
  - alarms that survive handoff;
  - the durable agent journal;
  - object lineage in the runtime graph.
- `object-custody`:
  - custodian sets in the DID document, with host and keeper roles;
  - quorum-promised custody epochs;
  - journal replication over TSP;
  - quorum before effects;
  - encrypted replicas at keepers;
  - handoff and failover across organisations;
  - equivocation evidence;
  - custodian set changes.
- `tsp-mediator`:
  - the TSP mailbox mediator service: routed-mode intermediary;
  - VID registration;
  - durable store-and-forward mailboxes;
  - the pickup protocol, with acknowledgement after the store;
  - live delivery sessions;
  - retention and quotas;
  - metadata minimisation;
  - a replicated deployment;
  - interop with the TSP reference implementation.

### Modified Capabilities

- `relay-mailboxes`:
  - adds `TSP Mediator Mailbox`, the client of `tsp-mediator`'s pickup protocol, so TSP actors receive while deactivated;
  - adds `Live Delivery Over WebSocket`.
  - Existing requirements and scenarios are unchanged.

## Impact

- **Code:**
  - `vertep-federation` gets the TSP pickup client and custody messages (feature `tsp`).
  - `vertep-agent` gets `StoreAgentJournal`.
  - `vertep-store` gets the signed and chained entry metadata in `AppendMeta`, plus verification.
  - `vertep-cluster`'s replication logic (`ReplicatedJournal`, `StandbyReplica`) is generalised over a transport, so custody reuses it over TSP.
  - New crate `vertep-tsp-mediator`.
  - New crate `vertep-objects`: the durable-object host and custody protocol, on `vertep-runtime`, `vertep-federation` and `vertep-store`.
  - Lean: a custody model proving one writer per epoch and no lost quorum-acknowledged entry.
  - `vertep-core` is unchanged.
- **Dependencies:** `tsp_sdk` 0.16 (already pinned). An HTTP and WebSocket server for the mediator (axum on tokio), behind the mediator crate only.
- **tg sections touched:**
  - [[roadmap#M5 Federation]] and a new `roadmap#M7 Durable objects`;
  - [[capabilities#Relay mailboxes]] and new capability sections;
  - [[components#vertep-federation]], [[components#vertep-agent]], [[components#vertep-cluster]], [[components#vertep-store]], and new component sections for `vertep-objects` and `vertep-tsp-mediator`;
  - [[glossary#Mailbox]];
  - [[views/distribution#Distribution#Relay mailboxes]];
  - [[views/agents#Agents#Implementation in vertep-agent]], whose "no durable agent journal" limitation this removes.
- **Binding decisions and constraints:**
  - [[decisions#Identity, mailbox and transport layers]]: custody is a journal concern; mailbox and transport stay separate layers.
  - [[decisions#Event-sourced actors]] and [[decisions#State changes travel in the append]]: an object's state is its journal.
  - [[decisions#TSP before DIDComm across trust boundaries]] and [[decisions#TSP direct when reachable, routed otherwise]]: custody and pickup ride on TSP.
  - [[decisions#Fencing epochs on journal appends]], [[decisions#Unreplicated entries stay unreadable until the quorum catches up]] and [[decisions#A standby's fence rises only with entries it stores]]: custody epochs reuse these rules without a membership authority.
  - [[decisions#Leases with fencing epochs from the membership authority]] is **not** used: custody has no shared authority.
  - [[decisions#DIDComm mailboxes acknowledge after the append]]: the same rule for TSP pickup.
  - [[decisions#Keys stay behind a key manager]]: entry signing and replica encryption go through the key manager.
  - [[decisions#The agent host journals agent inputs]]: the durable agent journal stores these inputs.
  - [[constraints#Relays deliver at least once]]: dedup by message id stays the receiver's job.
  - [[constraints#Clock skew beyond the safety margin breaks lease exclusivity]]: custody epochs must not depend on clocks.
- **Requirements added:**
  - `[[openspec:relay-mailboxes#TSP Mediator Mailbox]]`
  - `[[openspec:relay-mailboxes#Live Delivery Over WebSocket]]`
  - every requirement of `durable-objects`, `object-custody` and `tsp-mediator`.
- **Relied on unchanged:** `event-journal`, `storage`, `durable-timers`, `actor-model`, `did-identity`, `shard-placement` and `llm-actors`.
