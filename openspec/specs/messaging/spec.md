# messaging Specification

## Purpose
Defines how messages travel between actors: pluggable mailboxes and transports, resolution from an address to a mailbox, request and reply without blocking, deferred events, and what happens to events no state handles.

## Requirements

### Requirement: Pluggable Mailboxes
The runtime SHALL deliver every message for an actor through a mailbox contract with enqueue, ordered receive, and acknowledge operations, and SHALL allow different mailbox implementations per actor or per address scheme without changes to charts.

#### Scenario: Swapping the mailbox implementation
- **WHEN** an application configures a store-backed mailbox instead of the in-memory mailbox for an address scheme
- **THEN** charts addressed by that scheme run unchanged and receive the same events in the same order

#### Scenario: Unacknowledged message is redelivered
- **WHEN** an actor fails before its turn for a message is durable
- **THEN** the message is not acknowledged and the mailbox delivers it again on the next activation

### Requirement: Mailbox Ordering
A mailbox SHALL preserve the order of messages from a single sender to a single receiver, and the runtime SHALL NOT promise any order between messages from different senders.

#### Scenario: Per-sender order
- **WHEN** actor A sends events e1 then e2 to actor B
- **THEN** B processes e1 before e2

### Requirement: Mailboxes Outlive Activations
A mailbox SHALL accept messages for an actor that is not active, and the arrival of a message in the mailbox of an inactive virtual actor SHALL trigger its activation.

#### Scenario: Message to a deactivated actor
- **WHEN** a message is sent to a virtual actor that was deactivated for idleness
- **THEN** the message is stored in its mailbox and the actor is activated to process it

#### Scenario: Durable mailbox survives a restart
- **WHEN** the node restarts while a store-backed mailbox holds unprocessed messages
- **THEN** after restart those messages are delivered in their original order

### Requirement: Pluggable Transports
The runtime SHALL carry messages between mailboxes through a transport contract, and each transport SHALL be registered as an SCXML Event I/O Processor with a type IRI that `<send type="...">` selects.

#### Scenario: Local transport as Event I/O Processor
- **WHEN** a chart sends with no `type` or with the SCXML event processor type to an actor on the same node
- **THEN** the local in-process transport delivers it, and `_ioprocessors` lists that transport with its location

#### Scenario: Unsupported send type
- **WHEN** a `<send>` names a type no registered transport handles
- **THEN** the sending session raises `error.execution` and nothing is delivered

### Requirement: Address Resolution
The runtime SHALL resolve a target address IRI to the mailbox that serves it through a directory, and resolution SHALL depend only on the address so that an actor that is reactivated or relocated keeps the same address.

#### Scenario: Resolve a local actor
- **WHEN** a `<send>` targets the address of an actor hosted on this node
- **THEN** the directory returns its local mailbox and the message is enqueued there

#### Scenario: Unresolvable address
- **WHEN** the directory cannot resolve a target address
- **THEN** the sending session receives `error.communication` with the send id and the message is recorded as a dead letter

#### Scenario: Pluggable resolvers
- **WHEN** a resolver for a further address scheme is registered
- **THEN** addresses of that scheme are resolved by it without changing charts or other resolvers

### Requirement: Event Origin
Every delivered event SHALL carry the sender's address IRI in `_event.origin` and the transport's type IRI in `_event.origintype`, so that a receiver can reply with `<send target="_event.origin">`.

#### Scenario: Reply to origin
- **WHEN** actor B handles an event from A and sends to `_event.origin`
- **THEN** the reply is delivered to A

### Requirement: Non-Blocking Request
A chart SHALL be able to declare `vt:request`, which the IR expands into standard SCXML: a `<send>` carrying a correlation id, a waiting substate, a transition on the matching reply, and a timeout transition; no part of a request SHALL block the actor's turn.

#### Scenario: Request and reply
- **WHEN** a state performs `vt:request` to actor B with event `price.get` and B replies with the same correlation id
- **THEN** the requesting actor leaves the waiting substate on the `vt:reply` transition and the reply payload is available as `_event.data`

#### Scenario: Other events processed while waiting
- **WHEN** an unrelated event arrives while the actor is in a request's waiting substate
- **THEN** the actor processes it in a normal turn, without waiting for the reply

#### Scenario: Mutual requests do not deadlock
- **WHEN** actor A requests from B while B requests from A
- **THEN** both requests complete or time out, and neither actor stops processing other events

### Requirement: Deterministic Correlation Ids
The correlation id of a `vt:request` SHALL be derived from the requesting actor's address and the journal position of the macrostep that issued it, so that replaying the journal reproduces the same id.

#### Scenario: Replay reproduces correlation ids
- **WHEN** an actor's journal is replayed from the beginning
- **THEN** every request issued during replay carries the same correlation id it carried originally

#### Scenario: Ids are unique per actor
- **WHEN** an actor issues two requests in different macrosteps or twice in the same macrostep
- **THEN** the two correlation ids differ

### Requirement: Request Timeout
A `vt:request` SHALL accept a `vt:timeout` duration, and when no matching reply arrives in time the requesting actor SHALL receive `error.timeout` carrying the correlation id, scheduled as a durable timer.

#### Scenario: Timeout fires
- **WHEN** no reply arrives within the declared timeout
- **THEN** the actor receives `error.timeout` with the correlation id and leaves the waiting substate on its timeout transition

#### Scenario: Late reply after timeout
- **WHEN** a reply arrives after `error.timeout` was processed
- **THEN** the reply is not matched to any waiting state and is handled by the unhandled-event policy

### Requirement: Duplicate Reply Suppression
The runtime SHALL deliver at most one reply per correlation id to the requesting actor and SHALL drop further replies with the same correlation id.

#### Scenario: Redelivered reply dropped
- **WHEN** a relay redelivers a reply whose correlation id was already processed
- **THEN** the second copy is acknowledged and dropped without starting a turn

### Requirement: Deferred Events
A state SHALL be able to declare `vt:defer` with event descriptors; an external event matching a descriptor of an active state SHALL stay in the mailbox without being consumed, and SHALL be offered again, in its original order relative to other deferred events, once no active state defers it.

#### Scenario: Event deferred while busy
- **WHEN** the active state defers `order.*` and an `order.cancel` event arrives
- **THEN** no transition runs for it and it remains queued

#### Scenario: Redelivery after leaving the deferring state
- **WHEN** the actor transitions to a state that does not defer `order.*`
- **THEN** the deferred `order.cancel` is processed before newer non-deferred events, in the order deferred events arrived

#### Scenario: Deferral survives deactivation
- **WHEN** a virtual actor with deferred events is deactivated and later reactivated
- **THEN** the deferred events are still queued in their original order

### Requirement: Unhandled Event Policy
Each chart SHALL declare a policy for external events that enable no transition and are not deferred, one of `discard`, `dead-letter` or `error`; `discard` SHALL be the default for charts run as plain SCXML sessions and `dead-letter` SHALL be the default for virtual actors.

#### Scenario: Discard keeps SCXML conformance
- **WHEN** a chart with policy `discard` receives an event no transition handles
- **THEN** the event is dropped exactly as the SCXML algorithm specifies

#### Scenario: Dead letter for a virtual actor
- **WHEN** a virtual actor without an explicit policy receives an unhandled event
- **THEN** the event is recorded as a dead letter and the actor's configuration is unchanged

#### Scenario: Error policy
- **WHEN** a chart with policy `error` receives an unhandled event
- **THEN** `error.unhandled` is raised in the chart with the original event as data

### Requirement: Queryable Dead Letters
Every dead letter SHALL be recorded in the graph as an instance of `vt:DeadLetter` linked to the intended receiver, the sender, the event name and payload, the reason and the time it was recorded, in a named graph per receiver.

#### Scenario: Query what an actor ignored
- **WHEN** an operator runs a SPARQL query for `vt:DeadLetter` instances of an actor
- **THEN** every unhandled and unresolvable event for that actor is returned with its sender and reason

### Requirement: External Blocking Ask
The runtime SHALL offer a blocking request call with a timeout only to callers outside the actor system (such as HTTP handlers and tests), and SHALL NOT offer any blocking call inside a chart's executable content.

#### Scenario: Ask from outside
- **WHEN** an HTTP handler calls the external ask with a timeout of 2 s on an actor that replies after 100 ms
- **THEN** the call returns the reply payload

#### Scenario: Ask times out
- **WHEN** the actor does not reply within the timeout
- **THEN** the external ask returns a timeout error and the actor's late reply is dropped
