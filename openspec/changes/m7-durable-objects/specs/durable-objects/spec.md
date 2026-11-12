## Purpose

Lets an actor become a durable object: one DID-addressed, single-writer, event-sourced entity whose verifiable journal outlives any host that serves it, so agents and workflows keep their identity and state across restarts, hosts and organisations.

## ADDED Requirements

### Requirement: Durable Object Identity
A durable object SHALL be a virtual actor with a DID identity. Its address SHALL stay the same when the host serving it changes. `did:webvh` is the default method, and `did:key` and `did:peer` addresses SHALL be refused for durable objects because they cannot change the custodian set or service endpoints without changing the address.

#### Scenario: Address survives a host change
- **WHEN** a durable object moves from host A to host B
- **THEN** senders keep sending to the same DID, and the events reach the object on host B without any change at the sender

#### Scenario: Immutable DID method refused
- **WHEN** an application declares a durable object with a `did:key` address
- **THEN** declaration fails with an error naming the method, and no object is created

### Requirement: Verifiable Journal Entries
Every journal entry of a durable object SHALL carry the hash of the previous entry, the object DID, its position, the custody epoch and the writing host's DID, and SHALL be signed by the writing host's key. The hash SHALL cover the canonical form of the whole entry, including its state changes and outgoing messages.

#### Scenario: Chain verified from genesis
- **WHEN** a party holding the object's DID document reads a replica of its journal
- **THEN** it can check every signature, every previous-hash link and that each signer was a host of the epoch named in the entry, without contacting any host

#### Scenario: Tampered entry rejected
- **WHEN** one byte of an entry's payload is changed in a replica
- **THEN** verification fails at that position and reports the position and the reason `hash-mismatch`

#### Scenario: Unauthorised signer rejected
- **WHEN** an entry is signed by a DID that was not a host of the entry's custody epoch
- **THEN** verification fails at that position with the reason `not-a-host`

### Requirement: Single Writer Per Object
At most one host SHALL append to a durable object's journal in any custody epoch, and an append from an epoch lower than the highest epoch a custodian has promised SHALL be refused by that custodian.

#### Scenario: Stale writer fenced
- **WHEN** host A, writer of epoch 3, appends after host B has obtained epoch 4 from a quorum of custodians
- **THEN** the append fails to reach a quorum, host A's turn produces no effects, and host A deactivates the object

#### Scenario: Concurrent activations
- **WHEN** two hosts try to activate the same object at the same time
- **THEN** at most one of them reaches a quorum of promises for a given epoch, and only that one processes events

### Requirement: Activation From A Verified Replica
Any host in an object's custodian set SHALL be able to activate the object from replicas held by custodians. Before processing any input, it SHALL verify the chain it adopts. Among the verified chains reported by a quorum, it SHALL adopt the one accepted in the highest custody epoch, and the longest of those.

#### Scenario: Activation on a new host
- **WHEN** host B activates an object last served by host A, with A unreachable
- **THEN** B reads the custodians' heads, adopts the chain chosen by the rule above, replays it to the same configuration and datamodel A had, and then processes new events

#### Scenario: Longer chain from an older epoch loses
- **WHEN** one custodian holds 50 entries it accepted in epoch 3 and two hold 45 entries they accepted in epoch 4
- **THEN** the activating host adopts the 45-entry chain accepted in epoch 4

#### Scenario: Corrupt replica skipped
- **WHEN** one custodian returns a replica that fails verification
- **THEN** the activating host ignores that replica, records the failure, and activates from the others if a quorum of valid replicas remains

### Requirement: Effects After Durable Commit
A durable object's turn SHALL produce outgoing messages, timer changes and decider offers only after its journal entry has been committed. Committed means accepted by a quorum of custodians, or by the local store when the custodian set is the host alone.

#### Scenario: No quorum, no effects
- **WHEN** the host appends a turn and fewer than a quorum of custodians acknowledge it
- **THEN** no message from that turn is sent, and the entry is unreadable to new turns until a later replication brings a quorum level

#### Scenario: Single-host object
- **WHEN** a durable object's custodian set contains only its host
- **THEN** the turn's effects are produced after the local store's append, as for any virtual actor

### Requirement: Durable Alarms
Timers of a durable object SHALL be stored in its journal and SHALL fire on whichever host serves the object when they are due, including after a handoff or failover. A timer due while no host serves the object SHALL fire on the next activation.

#### Scenario: Alarm fires after failover
- **WHEN** an object sets a timer for 10 minutes and its host fails after 2 minutes
- **THEN** the host that activates it next fires the timer once when it is due, or at activation if it is already overdue

#### Scenario: Alarm not fired twice
- **WHEN** a timer fired and was journaled on host A, and the object then moves to host B
- **THEN** host B does not fire it again

### Requirement: Durable Agent Journal
The agent host SHALL support a journal backed by the journal store, so an agent's inputs survive host restarts and an agent can be replayed or forked from storage. The in-memory journal SHALL remain available for tests.

#### Scenario: Agent survives a restart
- **WHEN** an agent with a store-backed journal is waiting on a decision and its process restarts
- **THEN** the restarted host replays its journal without calling any provider or decider, and the open offer is still open with the same correlation id

#### Scenario: Fork from stored journal
- **WHEN** an agent is forked from position 12 of its stored journal
- **THEN** the fork's configuration and datamodel equal those the original had after position 12

### Requirement: Durable Object Lineage
The runtime graph SHALL record, for each durable object, its DID, its custody epochs with their hosts and custodian sets, and the journal positions each epoch wrote, so "who served this object when" is a query.

#### Scenario: Epoch history queried
- **WHEN** an object has been served by host A in epoch 1 and host B in epoch 2
- **THEN** a SPARQL query over the runtime graph returns both epochs with their hosts and first and last positions
