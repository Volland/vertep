## Purpose

Provides a TSP mailbox mediator: a Trust Spanning Protocol intermediary that routes per TSP 1.0 and, beyond the protocol, stores messages durably for registered VIDs until they pick them up, so TSP parties can receive while offline without trusting the mediator with content.

## ADDED Requirements

### Requirement: TSP Routed-Mode Intermediary
The mediator SHALL act as a TSP 1.0 intermediary for routed mode. It SHALL open only its own layer, forward to the next hop in the route, remove itself from the hop list, and deliver to the exit VID as a direct message when it is the last hop. It SHALL never be able to open the nested message for the final recipient.

#### Scenario: Middle hop
- **WHEN** the mediator receives a routed message whose remaining route is another intermediary and then the recipient
- **THEN** it forwards a routed message to that intermediary with itself removed from the route

#### Scenario: Last hop to an online recipient
- **WHEN** the mediator is the last hop and the recipient's endpoint is reachable
- **THEN** it delivers the message to the recipient directly and keeps no copy after the recipient accepts it

#### Scenario: Interop with the reference implementation
- **WHEN** a sender using the TSP reference implementation routes through the mediator to a reference-implementation receiver
- **THEN** the receiver opens and verifies the message

### Requirement: Mailbox Registration
A VID SHALL obtain a mailbox at the mediator by registering over a TSP relationship with the mediator's VID. The mediator SHALL accept routed messages for a recipient only if that recipient has registered. Registration SHALL be revocable by the VID and by the operator.

#### Scenario: Register a mailbox
- **WHEN** an actor's VID forms a relationship with the mediator and sends a registration request
- **THEN** the mediator replies with the registered recipient VID and the routing details the actor publishes in its DID document's `TSPTransport` service

#### Scenario: Unregistered recipient refused
- **WHEN** a routed message names an exit VID that has no mailbox at the mediator and is not reachable directly
- **THEN** the mediator refuses it, and the sending transport reports `error.communication`

### Requirement: Durable Store-And-Forward
When a registered recipient is not reachable, the mediator SHALL store the message durably and accept it from the previous hop only after the store has completed. Stored messages SHALL survive mediator restarts.

#### Scenario: Recipient offline
- **WHEN** a message arrives for a registered recipient whose endpoint is unreachable
- **THEN** the mediator stores it, accepts it, and the sender's outbox entry completes

#### Scenario: Mediator restarts
- **WHEN** the mediator restarts with 100 stored messages
- **THEN** all 100 messages are available for pickup afterwards

#### Scenario: Store fails
- **WHEN** the durable store fails while a message is being stored
- **THEN** the mediator refuses the message, and the previous hop keeps it for retry

### Requirement: Pickup Protocol
The mediator SHALL serve a pickup protocol carried as TSP messages over the relationship between a registered VID and the mediator, with these messages:

- `status`: count and oldest age of waiting messages;
- `deliver`: a batch, up to a requested limit;
- `received`: acknowledges message ids.

The protocol SHALL be identified by a versioned type IRI. A message SHALL be deleted only after the recipient acknowledges it with `received`. Delivery in a batch SHALL NOT count as acknowledgement.

#### Scenario: Batch pickup
- **WHEN** a recipient with 3 waiting messages sends `deliver` with a limit of 10
- **THEN** it receives the 3 messages, and they stay at the mediator until a `received` names them

#### Scenario: Lost acknowledgement
- **WHEN** a recipient stores two delivered messages but its `received` never reaches the mediator
- **THEN** the next pickup delivers them again

#### Scenario: Only the owner picks up
- **WHEN** a VID other than the registered recipient asks to pick up its messages
- **THEN** the mediator refuses, and delivers nothing

### Requirement: Live Delivery Session
The mediator SHALL offer a live session over WebSocket in which messages for a registered VID are pushed as they arrive. Live delivery SHALL use the same acknowledgement rule as batch pickup, so a dropped connection loses nothing.

#### Scenario: Push while connected
- **WHEN** a recipient holds a live session and a message arrives for it
- **THEN** the mediator pushes it on the session without waiting for a pickup request

#### Scenario: Connection drops before acknowledgement
- **WHEN** the session drops after a push and before the recipient's `received`
- **THEN** the message stays at the mediator and is delivered again on the next session or pickup

### Requirement: Retention And Quotas
The mediator SHALL enforce, per registered VID, a maximum number of waiting messages, a maximum total size and a retention period, all configured by the operator. When a limit is reached, it SHALL refuse new messages rather than drop stored ones. When the retention period expires, it SHALL report the expiry to the recipient on its next pickup.

#### Scenario: Mailbox full
- **WHEN** a recipient's mailbox holds its maximum number of messages and another arrives
- **THEN** the mediator refuses it, and the sender's retry policy applies

#### Scenario: Retention expiry reported
- **WHEN** a message expires before pickup
- **THEN** it is deleted, and the next `status` or `deliver` reports how many messages expired and the oldest expiry time

### Requirement: Metadata Minimisation
The mediator SHALL persist, per stored message, only the recipient VID, the opaque sealed bytes, a message id, the arrival time and the size. It SHALL NOT persist nested VIDs, payloads in clear, or sender identities beyond its own neighbour relationship. It SHALL NOT log message contents.

#### Scenario: Store inspected
- **WHEN** an auditor reads the mediator's durable store
- **THEN** it finds no event name, payload or inner sender VID

### Requirement: Replicated Mediator Deployment
The mediator SHALL support running as several nodes that share mailboxes, so the loss of one node loses no stored message and interrupts pickup only until another node serves that mailbox.

#### Scenario: Node lost
- **WHEN** one of three mediator nodes fails with stored messages
- **THEN** another node serves those mailboxes, and every message acknowledged to a sender is still delivered

### Requirement: Mediator Operability
The mediator SHALL run as one binary configured by a file and environment variables, SHALL publish its own DID document with a `TSPTransport` service, and SHALL expose health and per-mailbox metrics (waiting count, oldest age, refusals) without revealing identities beyond VIDs.

#### Scenario: Start from configuration
- **WHEN** an operator starts the mediator binary with a configuration file naming its DID, keys, store path and limits
- **THEN** it serves TSP on the configured endpoint, and its DID document resolves with that endpoint
