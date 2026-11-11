## Purpose

Carries actor messages between trusted nodes of one cluster over QUIC: authenticated, framed, ordered per pair of actors, flow-controlled, deduplicated and routed to the node that owns the target shard.

## ADDED Requirements

### Requirement: QUIC Cluster Transport
The system SHALL provide a cluster transport that carries actor messages between cluster nodes over QUIC, and it MUST plug into the runtime as a transport and SCXML Event I/O Processor, so that charts reach actors on other nodes with ordinary send.

#### Scenario: Send reaches an actor on another node
- **WHEN** a chart on node A sends an event to a virtual actor whose shard is owned by node B
- **THEN** the event is carried to B over the cluster transport and the target actor receives it as an external event

#### Scenario: Event origin names the sender
- **WHEN** the target actor processes an event delivered over the cluster transport
- **THEN** the event's origin is the sending actor's IRI and its origin type identifies the cluster Event I/O Processor

### Requirement: Mutual TLS Within The Cluster
Every cluster transport connection SHALL be authenticated in both directions with certificates issued for the cluster, and a node MUST refuse a connection whose peer certificate does not identify a current member of the same cluster.

#### Scenario: Member connects
- **WHEN** a current member connects with a certificate issued for the cluster
- **THEN** the connection is established and messages flow

#### Scenario: Foreign certificate refused
- **WHEN** a node presents a certificate that was not issued for the cluster
- **THEN** the connection is refused before any message is read

#### Scenario: Removed member refused
- **WHEN** a node that has left the membership view tries to open a connection
- **THEN** the connection is refused

### Requirement: Message Framing
The cluster transport SHALL carry each message as one self-delimiting frame that contains the message identifier, the sender and target IRIs, the shard and fencing epoch of the sender when it is a virtual actor, the event name and the event data, and it MUST reject a frame larger than the configured maximum size.

#### Scenario: Frame decodes to the same event
- **WHEN** an event with structured data is sent and received
- **THEN** the received event has the same identifier, name, data, sender and target as the sent one

#### Scenario: Oversized frame rejected
- **WHEN** a frame larger than the configured maximum arrives
- **THEN** it is rejected, the sender receives a delivery error, and the connection stays open for other frames

### Requirement: Ordering Per Sender And Receiver
The cluster transport SHALL deliver messages from one sending actor to one receiving actor in the order they were sent, and it MUST NOT impose any order between messages of different sender and receiver pairs.

#### Scenario: Two sends arrive in order
- **WHEN** actor X sends events e1 and then e2 to actor Y on another node
- **THEN** Y's mailbox receives e1 before e2

#### Scenario: Independent pairs do not block each other
- **WHEN** delivery from X to Y is stalled by a full mailbox
- **THEN** messages from X to Z and from W to Y keep flowing

### Requirement: Flow Control And Backpressure
The cluster transport SHALL apply backpressure per receiving mailbox, so that a sender's dispatch slows when the receiver's mailbox is at its configured bound, and it MUST NOT drop accepted messages because of load.

#### Scenario: Full mailbox slows the sender
- **WHEN** a receiving mailbox reaches its bound while a sender keeps sending
- **THEN** the sender's outbox dispatch waits until the mailbox has room and no message is dropped

#### Scenario: Backpressure is per mailbox
- **WHEN** one receiving mailbox is full
- **THEN** messages to other mailboxes on the same node are still delivered

### Requirement: At-Least-Once Delivery With Deduplication
The cluster transport SHALL retry a message until the receiving node acknowledges that it is stored in the target mailbox or journal, and the receiving side MUST discard a message whose identifier it has already accepted for that actor, so that each message is processed once.

#### Scenario: Lost acknowledgement causes a duplicate that is discarded
- **WHEN** a message is stored by the receiver but its acknowledgement is lost and the sender retries
- **THEN** the retry is acknowledged and discarded, and the actor processes the message once

#### Scenario: Retries stop at the deadline
- **WHEN** a message cannot be acknowledged before its delivery deadline
- **THEN** retries stop and the sending actor receives a delivery error event

### Requirement: Routing Via The Shard Table
Each node SHALL route a message for a virtual actor by computing the actor's shard and looking up the shard's owner in its copy of the lease table, and it MUST update that copy whenever it learns a newer membership view or lease epoch.

#### Scenario: Message routed to the owner
- **WHEN** a node sends a message to an actor of a shard owned by node C
- **THEN** the message is sent directly to C

#### Scenario: Routing follows a handover
- **WHEN** a shard moves from C to D and a node learns the new lease
- **THEN** its next message for that shard goes to D

### Requirement: Redirect On Stale Routing
A node that receives a message for a shard it does not own SHALL answer with a redirect naming the current owner and epoch, when it knows them, in addition to forwarding, so that the sender corrects its routing table.

#### Scenario: Sender learns the new owner
- **WHEN** a node sends to the old owner of a shard after a handover
- **THEN** it receives a redirect naming the new owner and its epoch, and its following messages for that shard go to the new owner

#### Scenario: Redirect with an older epoch is ignored
- **WHEN** a node receives a redirect whose epoch is lower than the epoch it already knows for that shard
- **THEN** it keeps its current routing entry
