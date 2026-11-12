# cluster-membership Specification

## Purpose
Lets trusted Vertep nodes form a cluster: who is a member, when a node has failed, and which node holds the lease on each shard, through one pluggable membership interface.

## Requirements

### Requirement: Membership Trait
The system SHALL expose cluster membership through one pluggable interface that reports the current membership view, notifies subscribers of view changes, and grants, renews, releases and revokes shard leases. The runtime MUST NOT depend on which implementation is configured.

#### Scenario: Runtime runs unchanged on any implementation
- **WHEN** the same cluster test suite is run against the static, gossip and external membership implementations
- **THEN** every placement, activation and routing test passes on each of them without changes to runtime code

#### Scenario: Subscribers receive view changes in order
- **WHEN** two membership changes happen one after the other
- **THEN** every subscriber receives both view changes, in the order of their view versions, with no change skipped

### Requirement: Membership View Versions
Every membership view SHALL carry a version number that increases strictly with every change in members or member status, and every node MUST reject a view whose version is not greater than the version it already holds.

#### Scenario: Newer view replaces older view
- **WHEN** a node holding view version 7 receives view version 8
- **THEN** the node adopts version 8 and reports it to subscribers

#### Scenario: Stale view is ignored
- **WHEN** a node holding view version 8 receives view version 6
- **THEN** the node keeps version 8 and does not notify subscribers

### Requirement: Graceful Join
A node SHALL join a cluster by presenting the cluster identifier, its node identifier, its transport endpoint and the cluster's shard count, and the cluster MUST refuse a node whose cluster identifier or shard count differs from the cluster's recorded values.

#### Scenario: Compatible node joins
- **WHEN** a node with the cluster's identifier and shard count asks to join
- **THEN** it appears as an active member in the next view version and becomes eligible for shard leases

#### Scenario: Node with a different shard count is refused
- **WHEN** a node configured with 128 shards asks to join a cluster created with 256 shards
- **THEN** the join is refused with an error naming both shard counts and the node never appears in a view

#### Scenario: Duplicate node identifier is refused
- **WHEN** a node asks to join with the identifier of a member that is still active
- **THEN** the join is refused and the active member is unaffected

### Requirement: Graceful Leave With Drain
A node that leaves gracefully SHALL first drain: stop accepting new activations, finish the macrostep in progress for each of its actors, flush outboxes, hand over every shard it leases, and only then leave the membership view.

#### Scenario: Leaving node hands over its shards
- **WHEN** a node that leases 12 shards is asked to leave
- **THEN** all 12 shards are leased to other members before the node disappears from the view, and no accepted message is lost

#### Scenario: Messages arriving during drain are forwarded
- **WHEN** a message for an actor on a draining shard arrives after that shard's handover started
- **THEN** the message is delivered to the shard's new owner and processed exactly once by the actor

### Requirement: Failure Detection
The system SHALL detect a member that stops responding and mark it suspect, then failed, within bounded times configured per cluster, and a failed member's shard leases MUST become available for reassignment only after those leases have expired.

#### Scenario: Crashed node is marked failed
- **WHEN** a member stops responding to every probe
- **THEN** it is marked suspect within the configured suspicion time and failed within the configured failure time, each in a new view version

#### Scenario: Suspect node that answers is restored
- **WHEN** a suspect member answers a probe before it is marked failed
- **THEN** it returns to active in a new view version and keeps its leases

#### Scenario: Leases of a failed node are not reassigned early
- **WHEN** a member is marked failed while its leases still have time left
- **THEN** no other node receives those shards until each lease has expired

### Requirement: Static Membership
The system SHALL provide a static membership implementation in which the member list comes from configuration, failure detection still runs, and leases are assigned deterministically from the member list.

#### Scenario: Members from configuration
- **WHEN** three nodes start with the same static member list
- **THEN** each reports the same view and the same shard-to-node lease table

#### Scenario: Unlisted node cannot join
- **WHEN** a node that is not in the static member list asks to join
- **THEN** the join is refused

### Requirement: Gossip Membership
The system SHALL provide a self-forming gossip membership implementation based on the SWIM protocol, with indirect probes for failure detection and dissemination of view changes by piggybacked gossip.

#### Scenario: Cluster forms from one seed
- **WHEN** five nodes start, each configured only with the address of one seed node
- **THEN** all five converge on the same view containing five active members

#### Scenario: Indirect probe avoids a false failure
- **WHEN** the direct link between two members drops but both can still reach a third member
- **THEN** neither member is marked failed

### Requirement: External Membership
The system SHALL provide membership implementations backed by an external coordination service, at least etcd and the Kubernetes Lease API, in which the external service is the authority for members and shard leases.

#### Scenario: Leases follow the external service
- **WHEN** a shard lease is granted in the external service to a node
- **THEN** that node becomes the shard owner and every other node routes the shard to it

#### Scenario: Loss of the external service stops new leases
- **WHEN** the external service becomes unreachable
- **THEN** no new lease is granted or renewed, and each node stops activating actors on a shard once its lease expires

### Requirement: Shard Leases With Fencing Epochs
Ownership of a shard SHALL be a time-bounded lease held by exactly one member, and every grant of a shard lease MUST carry a fencing epoch that is strictly greater than every epoch previously granted for that shard.

#### Scenario: New grant increases the epoch
- **WHEN** shard 42 held at epoch 5 is granted to another node
- **THEN** the new lease carries an epoch greater than 5

#### Scenario: Renewal keeps the epoch
- **WHEN** the owner renews its lease on shard 42 before it expires
- **THEN** the lease's expiry moves forward and its epoch is unchanged

#### Scenario: At most one valid lease per shard
- **WHEN** any two nodes inspect their leases at the same instant
- **THEN** at most one of them holds an unexpired lease on a given shard, as measured by the lease authority

### Requirement: Lease Expiry And Handover
An owner whose lease expires or is revoked SHALL stop processing messages for that shard before the lease authority may grant it to another node, and a planned handover MUST transfer the lease only after the old owner has completed its drain of that shard.

#### Scenario: Owner stops at expiry
- **WHEN** an owner cannot renew its lease on a shard before the lease expires
- **THEN** it stops starting macrosteps for actors of that shard no later than the expiry minus the configured safety margin

#### Scenario: Planned handover
- **WHEN** the shard is moved from node A to node B on purpose
- **THEN** A finishes its in-progress macrosteps and flushes its outbox, the lease moves to B with a higher epoch, and B processes the next message

#### Scenario: Old owner rejoins after a partition
- **WHEN** a node isolated by a network partition reconnects after its shard leases were reassigned
- **THEN** it learns the newer epochs from the current view and does not process messages for those shards
