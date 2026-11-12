## Purpose

Keeps a durable object's journal across independently operated custodians that talk only over TSP, choosing a single writer by quorum promises instead of a shared authority, so no one organisation or host is needed for the object to survive and progress.

## ADDED Requirements

### Requirement: Custodian Set In The DID Document
A durable object's DID document SHALL name its custodian set, each custodian by DID with a role of `host` or `keeper`, together with the replication quorum. Hosts SHALL be able to activate and write the object. Keepers SHALL only store and return entries. The quorum SHALL be a majority of the custodian set.

#### Scenario: Custodian set resolved
- **WHEN** a host resolves an object's DID document listing three hosts and two keepers
- **THEN** it obtains five custodians with their roles and TSP endpoints and a quorum of three

#### Scenario: Quorum below a majority refused
- **WHEN** a DID document declares a quorum of two for a custodian set of five
- **THEN** hosts refuse to activate the object and report the custody declaration as invalid

### Requirement: Quorum-Promised Custody Epochs
A host SHALL become the writer of an object for epoch `e` only after a quorum of custodians promised `e`. Each promise SHALL be signed by the custodian and SHALL report the custodian's journal head and the epoch in which it accepted that journal. A custodian SHALL promise `e` only if `e` is higher than every epoch it has promised for that object, and SHALL refuse appends from lower epochs afterwards. No clock or lease time SHALL be part of this decision.

#### Scenario: Epoch won
- **WHEN** a host asks five custodians to promise epoch 7 and three promise it
- **THEN** the host is writer of epoch 7, and the object's lineage records epoch 7 with the three signed promises

#### Scenario: Epoch lost to a higher one
- **WHEN** a custodian has promised epoch 8 and a host asks it to promise epoch 7
- **THEN** the custodian refuses and returns 8, and the host retries with an epoch above 8 or gives up

#### Scenario: Clock skew does not matter
- **WHEN** the hosts' clocks differ by an hour
- **THEN** the writer chosen and the entries accepted are the same as with synchronised clocks

### Requirement: Journal Replication Over TSP
The writer SHALL replicate every entry to every custodian as TSP messages between the writer's VID and the custodian's VID, direct or routed, and custodians SHALL acknowledge an entry only after storing it durably. Entries SHALL be shipped as the suffix each custodian lacks.

#### Scenario: Custodian behind a mediator
- **WHEN** a keeper is reachable only through its TSP mediator
- **THEN** replication messages reach it through the mediator's mailbox, and the keeper's acknowledgement arrives the same way

#### Scenario: Lagging custodian catches up
- **WHEN** a custodian was offline for 40 entries and comes back
- **THEN** the writer ships the missing suffix in order, and the custodian acknowledges the new head once it has stored all of it

### Requirement: Quorum Before Effects
An entry SHALL count as committed only when a quorum of custodians (the writer included if it is a custodian) has accepted a journal containing it in the writer's epoch. An entry inherited from an earlier epoch SHALL therefore count as committed only once a quorum has accepted it again in the new writer's epoch. An entry that never reached a quorum SHALL never produce effects. After a failover, every committed entry SHALL be in the new writer's journal. A custodian SHALL replace an uncommitted tail from a lower epoch when the writer of a higher epoch ships a different suffix from an earlier position.

#### Scenario: Inherited entries commit with the new epoch
- **WHEN** a new writer of epoch 6 adopts entries 40 to 42 written in epoch 5 that only one other custodian stored
- **THEN** entries 40 to 42 are not readable by the runtime and produce no effects until a quorum has accepted them in epoch 6

#### Scenario: Stale tail replaced
- **WHEN** a custodian holds an uncommitted entry 41 from epoch 5 and the writer of epoch 6 ships a different entry 41
- **THEN** the custodian replaces its entry 41 and acknowledges, and this is not recorded as equivocation

#### Scenario: Committed entry survives failover
- **WHEN** an entry was acknowledged by three of five custodians and the writer then fails
- **THEN** any host that wins the next epoch adopts a chain that contains that entry

#### Scenario: Uncommitted entry discarded
- **WHEN** an entry reached only the writer before it failed
- **THEN** the next writer may adopt a chain without it, and no message from that entry was ever sent

### Requirement: Encrypted Replicas At Keepers
Entries stored at keepers SHALL be encrypted so that keepers cannot read state, events or outgoing messages. Hosts of the current epoch SHALL be able to decrypt them. Keepers SHALL see only the object DID, position, epoch, previous hash and signature in clear, which is enough to store, order and verify.

#### Scenario: Keeper cannot read state
- **WHEN** a keeper inspects everything it stores for an object
- **THEN** it finds no event name, payload or datamodel value in clear

#### Scenario: New host decrypts keeper replicas
- **WHEN** a host added to the custodian set activates the object from keeper replicas only
- **THEN** it decrypts and replays them, because the content keys were re-wrapped to it when it was added

### Requirement: Custody Handoff
A writer SHALL be able to hand an object to another host on purpose. The handoff SHALL quiesce the object after a committed entry, the target SHALL win the next epoch, and no event SHALL be lost or processed twice.

#### Scenario: Planned move between organisations
- **WHEN** organisation 1's host hands an object to organisation 2's host while events keep arriving
- **THEN** events accepted before the handoff are processed by the old host, the others by the new host, and each exactly once

### Requirement: Failover Without A Shared Authority
When the writer is unreachable, any other host SHALL be able to take over by winning a higher epoch. Takeover SHALL proceed as long as a quorum of custodians is reachable, and SHALL need no coordinator, membership service or lease authority.

#### Scenario: Writer's organisation goes offline
- **WHEN** the writer's whole organisation becomes unreachable and three of five custodians remain
- **THEN** a remaining host wins a higher epoch, adopts the committed chain and resumes processing

#### Scenario: No quorum reachable
- **WHEN** only two of five custodians are reachable
- **THEN** no host becomes writer, events wait in the object's mailbox, and processing resumes once a quorum is reachable

### Requirement: Equivocation Evidence
A custodian or host that observes two validly signed entries for the same object, epoch and position with different hashes SHALL record both as equivocation evidence in its runtime graph and SHALL refuse further entries from that epoch's writer.

#### Scenario: Forked writer detected
- **WHEN** a keeper receives entry 30 of epoch 5 with hash X and later another entry 30 of epoch 5 with hash Y, both signed by the epoch's writer
- **THEN** the keeper stores both signed entries as evidence, refuses further epoch-5 entries, and reports the evidence to the object's controller

### Requirement: Custodian Set Changes
The object's controller SHALL be able to change the custodian set with a request signed by its DID update key. The writer SHALL replicate the new set as custody state with the journal, and the change SHALL take effect when a quorum of the old set has accepted it and the new custodians have stored the committed chain. Epochs won after that SHALL need a quorum of the new set. The writer SHALL then publish the new set in the DID document. The journal is authoritative for custody, and the DID document is used for discovery.

#### Scenario: Replace a keeper
- **WHEN** the controller replaces keeper K1 with K3
- **THEN** K3 receives the committed chain, the old quorum accepts the new set, the DID document lists K3, the next epoch is won with promises from the new set, and K1's later promises no longer count

#### Scenario: Unsigned change refused
- **WHEN** a host asks to change the custodian set without the controller's signature
- **THEN** the writer refuses, and the custodian set does not change

#### Scenario: Old set cannot elect after a change
- **WHEN** a host gathers a quorum of promises only from custodians of the previous set after the change took effect
- **THEN** other custodians refuse its entries, and no effects occur
