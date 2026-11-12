## Purpose

Lets actors exchange messages across trust boundaries through DIDComm v2 mediators and Trust Spanning Protocol intermediaries, used as pluggable mailboxes and transports that cannot read payloads, never lose a message and never apply it twice.

## ADDED Requirements

### Requirement: DIDComm Mediator Mailbox
The federation layer SHALL provide a mailbox implementation in which a DIDComm v2 mediator stores messages for an actor until they are picked up, so an actor can receive messages while it is deactivated or offline.

#### Scenario: Message stored while the actor is deactivated
- **WHEN** a message is sent to a deactivated virtual actor whose DID document names a DIDComm mediator
- **THEN** the mediator stores the message and the sender's send completes without waiting for the actor

#### Scenario: Messages survive a node restart
- **WHEN** the node hosting the actor restarts while messages wait at the mediator
- **THEN** the messages are picked up after the restart and delivered

### Requirement: DIDComm Forward Routing
The DIDComm transport SHALL deliver to a recipient behind one or more mediators by wrapping the encrypted message in forward messages, one per mediator, following the routing keys in the recipient's DID document.

#### Scenario: One mediator
- **WHEN** a recipient's service entry lists one mediator routing key
- **THEN** the transport encrypts the message to the recipient, wraps it in a forward message encrypted to the mediator, and posts it to the mediator's endpoint

#### Scenario: Chain of mediators
- **WHEN** a recipient's service entry lists two routing keys
- **THEN** the message is wrapped once per mediator in order, and each mediator can unwrap only its own layer

### Requirement: DIDComm Message Pickup
The DIDComm mailbox SHALL fetch stored messages from the mediator with the message pickup protocol, and SHALL acknowledge each message to the mediator only after it has been appended to the receiving actor's journal or rejected and recorded.

#### Scenario: Pickup batch
- **WHEN** an activation polls its mediator and three messages are waiting
- **THEN** the three messages are delivered in the mediator's order, journaled, and then acknowledged

#### Scenario: Crash before acknowledgement
- **WHEN** a node crashes after journaling a picked-up message but before acknowledging it
- **THEN** the message is picked up again after recovery, recognised as a duplicate by its message id, and acknowledged without being processed again

#### Scenario: Live delivery
- **WHEN** an activation keeps a live pickup session open and a message reaches the mediator
- **THEN** the message is delivered without waiting for the next poll

### Requirement: TSP Direct Transport
The federation layer SHALL provide a transport that sends TSP messages directly between two parties' VIDs when the recipient's endpoint is reachable, with the sender's VID authenticated by the TSP envelope.

#### Scenario: Direct exchange
- **WHEN** two DID actors that can reach each other's TSP endpoints exchange events
- **THEN** each event travels as a TSP message from sender VID to receiver VID and arrives with a verified origin

#### Scenario: Relationship required
- **WHEN** a sender has no TSP relationship with the recipient and the recipient requires one
- **THEN** the transport establishes the relationship before sending the event, or fails with `error.communication` if the recipient refuses

### Requirement: TSP Routed Transport Through Intermediaries
The TSP transport SHALL support routed delivery through one or more intermediaries named by the route, with the event payload nested so that only the final recipient can open it.

#### Scenario: Routed through one intermediary
- **WHEN** a recipient is reachable only through an intermediary
- **THEN** the sender sends a routed TSP message to the intermediary, which forwards it to the recipient, and the intermediary cannot read the event

#### Scenario: Intermediary refuses to route
- **WHEN** an intermediary rejects a routed message
- **THEN** the sender's chart receives `error.communication` for that send

### Requirement: End-To-End Encryption Through Relays
Every message crossing a trust boundary SHALL be encrypted to the final recipient's keys, so that mediators and intermediaries can see routing information and nothing else: no event name, payload, send id or origin.

#### Scenario: Mediator sees only routing data
- **WHEN** a mediator stores a message for a recipient
- **THEN** what it holds is the recipient routing key and an encrypted payload, and the event name and data cannot be recovered from it without the recipient's keys

#### Scenario: Tampered message
- **WHEN** a relay modifies a stored encrypted message
- **THEN** the recipient fails to decrypt or authenticate it, does not deliver it, and records the failure

### Requirement: SCXML Event Envelope Mapping
A relay transport SHALL carry an SCXML event so that the receiver reconstructs the same `name`, `data`, `sendid`, `origin` and `origintype`. Typed payloads SHALL keep their declared type, and a payload that fails its type check at the receiver SHALL NOT be delivered.

#### Scenario: Round trip of an event
- **WHEN** a chart sends `order.placed` with a typed payload and send id `s-17` over DIDComm
- **THEN** the receiver's `_event` has name `order.placed`, the same payload, send id `s-17`, the sender's DID as origin and the DIDComm I/O processor URI as origin type

#### Scenario: Payload fails the receiver's type
- **WHEN** a received payload does not match the receiving chart's declared type for that event
- **THEN** the event is not delivered, the rejection is recorded, and the sender receives `error.communication` if a reply path exists

#### Scenario: Unknown fields from a newer sender
- **WHEN** an envelope carries extension headers the receiver does not understand
- **THEN** the receiver ignores them and delivers the event if its required fields are intact

### Requirement: At-Least-Once Delivery With Deduplication
Relay mailboxes and transports SHALL deliver every accepted message at least once, and the receiving actor SHALL process each message id at most once, so redelivery after crashes, timeouts or relay retries never changes state twice.

#### Scenario: Relay retries delivery
- **WHEN** a relay delivers the same message twice because an acknowledgement was lost
- **THEN** the second copy is recognised by its message id, acknowledged, and not processed

#### Scenario: Sender retries after timeout
- **WHEN** a sender's outbox re-sends a message after a transport timeout although the first copy arrived
- **THEN** the receiver processes the message once

### Requirement: Shared And Per-Actor Mediator Mailboxes
The DIDComm and TSP mailboxes SHALL support a mediator mailbox dedicated to one actor and a mailbox shared by many actors of one node or shard, demultiplexing messages to the right actor by recipient.

#### Scenario: Shared mailbox for a shard
- **WHEN** a shard's actors share one mediator mailbox and messages for three of them are waiting
- **THEN** one pickup session fetches them and each is delivered to its own actor's journal

#### Scenario: Dedicated mailbox
- **WHEN** an actor has its own mediator mailbox
- **THEN** only messages for that actor are fetched from it

### Requirement: Activation On Delivery
A message picked up for a deactivated virtual actor SHALL activate the actor, and the message SHALL be delivered as an ordinary input event once activation completes.

#### Scenario: Message wakes an actor
- **WHEN** a message for a deactivated virtual actor is picked up from its mediator
- **THEN** the actor is activated from its journal and snapshots, and the message is processed in its next macrostep

#### Scenario: Activation fails
- **WHEN** activation of the recipient fails
- **THEN** the message is not acknowledged to the mediator and stays available for a later pickup

### Requirement: Transport Selection From The DID Document
The federation layer SHALL choose the transport for a cross-trust send from the services in the recipient's resolved DID document, in a configurable order of preference, and SHALL fall back to the next compatible service if one fails.

#### Scenario: Recipient supports both protocols
- **WHEN** a recipient's DID document lists both a TSP service and a DIDComm service, and TSP is preferred
- **THEN** the message is sent over TSP

#### Scenario: Fallback to the next service
- **WHEN** the preferred service's endpoint is unreachable and a second compatible service exists
- **THEN** the message is sent through the second service, and the fallback is recorded

### Requirement: Undeliverable Message Reporting
When a cross-trust send cannot be delivered within its retry policy, the runtime SHALL raise `error.communication` in the sending chart with the send id and the reason. It SHALL NOT block the chart while retrying.

#### Scenario: Recipient unreachable
- **WHEN** every compatible endpoint of a recipient stays unreachable until the retry policy is exhausted
- **THEN** the sending chart receives `error.communication` carrying the original send id and the reason

#### Scenario: Chart keeps running while retrying
- **WHEN** a send is being retried in the background
- **THEN** the sending chart continues to process other events, and the outbox entry stays pending until the send succeeds or fails

### Requirement: Mailbox Ordering
A relay mailbox SHALL deliver the messages it holds for one actor in the order the mailbox received them, and SHALL NOT promise any ordering between messages that travel through different relays or senders.

#### Scenario: Order preserved within one mailbox
- **WHEN** a mediator holds messages m1 then m2 for an actor
- **THEN** the actor journals m1 before m2

#### Scenario: No cross-relay ordering
- **WHEN** a sender sends m1 over TSP and m2 over DIDComm to the same actor
- **THEN** the actor may journal m2 before m1, and charts that depend on order must use correlation ids or sequence data
