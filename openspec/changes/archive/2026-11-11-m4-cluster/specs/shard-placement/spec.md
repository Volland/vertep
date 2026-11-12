## Purpose

Places virtual actors on cluster nodes by shard: every actor IRI maps to one fixed shard, each shard has one journal and one owner, and the cluster guarantees a single activation per actor while shards move between nodes.

## ADDED Requirements

### Requirement: Fixed Shard Count
A cluster SHALL have a shard count fixed at creation, recorded in the cluster's metadata, and that count MUST be a power of two so that resharding can split shards instead of reshuffling every actor.

#### Scenario: Shard count recorded at creation
- **WHEN** a cluster is created with 256 shards
- **THEN** the cluster metadata records 256 and every node that joins reports 256

#### Scenario: Non power of two is rejected
- **WHEN** a cluster is created with 300 shards
- **THEN** creation fails with an error stating that the shard count must be a power of two

### Requirement: Shard Assignment By Actor IRI Hash
The shard of a virtual actor SHALL be the value of a fixed, versioned, platform-independent hash of the actor's IRI string modulo the shard count, so that every node computes the same shard for the same IRI.

#### Scenario: Same IRI, same shard everywhere
- **WHEN** nodes on different operating systems and CPU architectures compute the shard of the same actor IRI
- **THEN** they all compute the same shard number

#### Scenario: Hash function version is recorded
- **WHEN** a cluster is created
- **THEN** its metadata records the version of the hash function, and a node built with a different hash version is refused at join

### Requirement: One Journal File Per Shard
Each shard SHALL store the journals of all its actors in one journal store instance with one writer, and no two shards MUST share a journal store instance.

#### Scenario: Actors of one shard share a store
- **WHEN** two actors whose IRIs hash to shard 7 each commit a macrostep
- **THEN** both entries are written to shard 7's journal store and nothing is written to any other shard's store

#### Scenario: Shards write independently
- **WHEN** actors of shard 7 and shard 8 on the same node commit macrosteps at the same time
- **THEN** neither commit waits on the other's writer

### Requirement: Activation On Shard Owner Only
A virtual actor SHALL be activated only on the node that holds an unexpired lease on its shard, and a node MUST refuse to activate an actor of a shard it does not lease.

#### Scenario: Owner activates on first message
- **WHEN** the first message for a deactivated actor reaches the owner of its shard
- **THEN** the owner activates the actor by replaying its journal and processes the message

#### Scenario: Non-owner refuses activation
- **WHEN** a node that does not lease the actor's shard is asked to activate it
- **THEN** it refuses and forwards the message to the current owner

### Requirement: Single Activation Guarantee
The cluster SHALL guarantee that for every virtual actor at most one activation can commit macrosteps under any fencing epoch, and that no two activations commit conflicting entries to the actor's journal.

#### Scenario: Concurrent activation attempts
- **WHEN** two nodes try to activate the same actor at the same moment because of a routing race
- **THEN** only the node holding the lease activates it, and the other forwards its message

#### Scenario: Fault injection never yields two writers
- **WHEN** a fault-injection test partitions, pauses and kills nodes at random while messages flow
- **THEN** no actor journal ever contains two entries for the same journal version

### Requirement: Journal Fencing Backstop
Every journal append SHALL carry the fencing epoch of the lease under which it was made, and a journal store MUST reject an append whose epoch is lower than the highest epoch it has already accepted for that shard, independently of the version check of the append.

#### Scenario: Stale owner append fails after split brain
- **WHEN** node A loses its lease on shard 9 during a partition, node B is granted the lease at a higher epoch and appends, and A then tries to append under the old epoch
- **THEN** A's append is rejected with a fencing error, A deactivates the shard's actors, and B's entries are kept

#### Scenario: Fenced outbound messages are dropped
- **WHEN** a node receives a message produced by a shard under an epoch lower than the highest epoch it has seen for that shard
- **THEN** it drops the message and records it as fenced in the dead-letter graph

### Requirement: Message Forwarding To Shard Owner
A message for a virtual actor that reaches a node which is not the owner of the actor's shard SHALL be forwarded to the current owner, and forwarding MUST stop after a configured hop limit with a delivery error.

#### Scenario: Message sent to the wrong node
- **WHEN** a message for an actor of shard 3 arrives at a node that does not own shard 3
- **THEN** the node forwards it to the owner and the actor processes it once

#### Scenario: Hop limit reached
- **WHEN** a message has been forwarded as many times as the hop limit allows without reaching an owner
- **THEN** forwarding stops and the sender receives a delivery error event

### Requirement: Rebalancing By Journal File Shipping
When shards are moved between nodes whose journals are local files, the system SHALL drain the shard on the old owner, ship the shard's append-only journal to the new owner, verify that the shipped journal ends at the same version, and only then transfer the lease.

#### Scenario: Shard moved to a new node
- **WHEN** a new node joins and the rebalancer moves shard 5 to it
- **THEN** the new node holds a journal for shard 5 identical to the old owner's, then receives the lease at a higher epoch, and every actor of shard 5 resumes from its last committed state

#### Scenario: Shipping interrupted
- **WHEN** the connection breaks while shard 5's journal is being shipped
- **THEN** the old owner keeps the lease and resumes processing, and shipping restarts from the last verified position

### Requirement: Journal Replication To Standbys
The system SHALL allow a per-cluster replication factor for local shard journals, and with a factor above one an append MUST be acknowledged only after the configured number of standbys have stored it, so that a standby can take over a failed owner's shard.

#### Scenario: Failover to a standby
- **WHEN** the owner of shard 11 fails with a replication factor of three
- **THEN** after the lease expires a standby holding every acknowledged entry is granted the lease at a higher epoch and the shard's actors resume without losing acknowledged entries

#### Scenario: No standby with replication factor one
- **WHEN** the owner of a shard fails and the replication factor is one
- **THEN** the shard stays unavailable until the failed node's journal is recovered, and messages for its actors wait in their mailboxes

### Requirement: Rebalancing By Lease Transfer
When the cluster uses a networked journal store shared by all nodes, the system SHALL move a shard by draining it and transferring the lease alone, with no copying of journal data.

#### Scenario: Shard moved with a shared journal
- **WHEN** shard 5 is moved between nodes that share a networked journal store
- **THEN** no journal data is copied, and the new owner continues from the journal store under a higher epoch

### Requirement: Resharding Procedure
The system SHALL provide a documented, offline resharding procedure that doubles the shard count by splitting every shard s into shards s and s plus the old count, moving each actor's journal to the shard its IRI now hashes to, and recording the new count in the cluster metadata.

#### Scenario: Doubling the shard count
- **WHEN** a cluster with 64 shards is resharded to 128
- **THEN** every actor's journal is in the shard its IRI hashes to under 128 shards, its replayed state is unchanged, and the metadata records 128

#### Scenario: Resharding refuses a running cluster
- **WHEN** resharding is started while any node holds a shard lease
- **THEN** the procedure refuses to start and names the nodes still holding leases
