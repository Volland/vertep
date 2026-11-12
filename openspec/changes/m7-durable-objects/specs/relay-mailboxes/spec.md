## ADDED Requirements

### Requirement: TSP Mediator Mailbox
The federation layer SHALL provide a mailbox in which a TSP mediator stores messages for an actor until they are picked up, so a TSP actor can receive while deactivated or offline. The mailbox SHALL acknowledge each message to the mediator only after the receiving journal append, or after a recorded rejection.

#### Scenario: Message waits for a deactivated actor
- **WHEN** a message is routed to a deactivated virtual actor whose `TSPTransport` service routes through its mediator
- **THEN** the mediator holds the message, and the sender's send completes without waiting for the actor

#### Scenario: Pickup activates and acknowledges after the append
- **WHEN** the actor's node picks up the waiting message
- **THEN** the actor is activated, the message is journaled with its verified origin, and only then is it acknowledged to the mediator

#### Scenario: Crash between append and acknowledgement
- **WHEN** the node crashes after the append and before acknowledging
- **THEN** the message is delivered again after restart, recognised by its message id, acknowledged, and not processed twice

### Requirement: Live Delivery Over WebSocket
The DIDComm and TSP mediator mailboxes SHALL support a live delivery session over WebSocket in which the mediator pushes waiting and newly arriving messages. Acknowledgement SHALL still follow the journal append.

#### Scenario: Live TSP session
- **WHEN** a node holds a live session with a TSP mediator and a message arrives for one of its actors
- **THEN** the message reaches the actor's journal without a pickup poll, and is acknowledged after the append

#### Scenario: Session reconnects
- **WHEN** the live session drops and reconnects
- **THEN** messages pushed but not acknowledged before the drop are delivered again and deduplicated by message id
