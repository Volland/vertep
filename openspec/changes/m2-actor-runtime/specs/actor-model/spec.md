## Purpose

Defines what an actor is in Vertep: anything that can be the target of an SCXML `<send>` or `<invoke>`, addressed by an IRI, and living in one of three lifecycle classes (virtual, invoked, stateless worker) with turn-based, run-to-completion processing and supervision.

## ADDED Requirements

### Requirement: Actor Trait
The runtime SHALL accept as an actor any implementation of a single actor contract that receives SCXML events one at a time and produces zero or more outgoing events, and a running chart SHALL be one such implementation, so that charts, plain functions, stream sources, tool adapters and LLM adapters are interchangeable as targets of `<send>` and `<invoke>`.

#### Scenario: Chart is an actor
- **WHEN** a chart is registered with the runtime and started
- **THEN** it is reachable by `<send>` and `<invoke>` exactly like any other actor implementation, and its events are processed by the SCXML interpretation algorithm

#### Scenario: Non-chart actor is invokable
- **WHEN** a chart state declares `<invoke>` whose type names a registered non-chart actor kind (for example an async function actor)
- **THEN** the runtime starts that actor as a child of the invoking session, routes `#_<invokeid>` sends to it and delivers its completion to the parent as `done.invoke.<invokeid>`

#### Scenario: Unknown actor kind
- **WHEN** a chart invokes an actor type that no registered actor kind handles
- **THEN** the runtime raises `error.execution` in the invoking session and starts nothing

### Requirement: IRI Actor Addresses
Every actor SHALL have exactly one address that is an absolute IRI, and the same IRI SHALL be the RDF subject under which the actor's state, journal and relationships are stored.

#### Scenario: Address is an IRI
- **WHEN** an actor is created or activated
- **THEN** its address is an absolute IRI and `_sessionid`-based targets and `#_parent` resolve through that IRI

#### Scenario: Address names the graph subject
- **WHEN** the current state of an actor is queried in the graph store
- **THEN** the statements describing that actor use its address IRI as subject or as named-graph name

#### Scenario: Relative or malformed address rejected
- **WHEN** a `<send>` target or an actor registration supplies a relative reference or a string that is not a valid IRI
- **THEN** the send fails with `error.execution` when the target is not syntactically a legal target (W3C SCXML 6.2.4), or with `error.communication` when it is an IRI the runtime cannot resolve, or the registration is rejected; in every case nothing is enqueued

### Requirement: Virtual Actor Identity
A virtual actor SHALL be identified by the pair of a chart type IRI and a key, from which its address IRI is derived deterministically, and SHALL logically exist for every such pair without being created or destroyed.

#### Scenario: Same type and key give the same address
- **WHEN** two callers on the same node address a virtual actor of chart type T with key K
- **THEN** both resolve to the same address IRI and their messages reach the same journal

#### Scenario: First message to a never-used key
- **WHEN** a message is sent to a virtual actor whose journal is empty
- **THEN** the runtime activates it in the chart's initial configuration and processes the message, without any explicit create call

### Requirement: Virtual Actor Activation
The runtime SHALL activate a virtual actor on demand when a message for it is available in its mailbox, restoring its state from the journal, and SHALL guarantee at most one activation of a given virtual actor per node at any time.

#### Scenario: Activation on message
- **WHEN** a message arrives for a virtual actor that is not active
- **THEN** the runtime restores the actor from its latest snapshot plus later journal entries and then processes the message

#### Scenario: Concurrent messages during activation
- **WHEN** several messages for the same inactive virtual actor arrive while it is being activated
- **THEN** exactly one activation is created and all messages are processed by it in mailbox order

### Requirement: Virtual Actor Deactivation
The runtime SHALL deactivate a virtual actor after a configurable idle period with no pending messages and no running turn, releasing its memory without losing state, because its state is fully recoverable from the journal.

#### Scenario: Idle actor is deactivated
- **WHEN** a virtual actor has had an empty mailbox and no running turn for longer than its idle timeout
- **THEN** the runtime deactivates it and a later message reactivates it in the same configuration and datamodel

#### Scenario: No deactivation with pending work
- **WHEN** the idle timeout elapses while a message is queued or a turn is running
- **THEN** the actor is not deactivated

### Requirement: Virtual Actors Are Not Invokable
A virtual actor SHALL NOT be the target of `<invoke>`, because no state owns its lifetime; it SHALL be reachable only by `<send>` and `vt:request`.

#### Scenario: Invoke of a virtual actor rejected
- **WHEN** a chart declares `<invoke>` whose target resolves to a virtual actor address
- **THEN** chart validation reports an error, or at runtime the invoking session raises `error.execution` and the virtual actor is neither started nor cancelled

#### Scenario: Cancelling a parent does not touch a virtual actor
- **WHEN** a state that has sent messages to a virtual actor exits
- **THEN** the virtual actor keeps running and its state is unchanged

### Requirement: Invoked Actor Lifecycle
An invoked actor SHALL be started when its invoking state is entered at the end of a macrostep, SHALL have the address formed from the parent's address and the invoke id, and SHALL be cancelled when the invoking state exits, as defined by SCXML `<invoke>` semantics.

#### Scenario: Started with the state
- **WHEN** a macrostep ends with a state containing `<invoke id="search">` active
- **THEN** a child actor is started whose address is derived from the parent address and the id `search`

#### Scenario: Cancelled on state exit
- **WHEN** the invoking state exits before the child completes
- **THEN** the child is cancelled, no further events from it are delivered to the parent, and no `done.invoke.search` is raised

#### Scenario: Completion reported to the parent
- **WHEN** the child reaches a final state or its function completes
- **THEN** the parent receives `done.invoke.search` with the child's done data

### Requirement: Stateless Worker Lifecycle
A stateless worker SHALL be an actor kind with no journal and no per-instance identity beyond its address, of which the runtime MAY run several activations per address, delivering each message to exactly one activation.

#### Scenario: Parallel activations
- **WHEN** many messages are sent to a stateless worker address with a pool size of four
- **THEN** up to four activations process messages concurrently and each message is processed by exactly one of them

#### Scenario: Worker keeps no journal
- **WHEN** a stateless worker processes a message
- **THEN** nothing is appended to any journal on its behalf; its reply is journaled only by the caller that receives it

### Requirement: Turn Is One Macrostep
Each actor SHALL process its external events one at a time, and a turn SHALL consist of dequeuing one external event and running exactly one SCXML macrostep to completion before the next external event is taken.

#### Scenario: No interleaving within a turn
- **WHEN** two external events arrive for the same actor while a macrostep is running
- **THEN** the second event is not observed until the macrostep of the first has completed and its results are durable

#### Scenario: Internal events stay within the turn
- **WHEN** a transition raises an internal event
- **THEN** the event is processed within the same macrostep, before the next external event is dequeued

### Requirement: Supervision With Restart
An `<invoke>` SHALL accept a `vt:restart` extension specifying a restart policy (never, on-failure, always), a maximum number of restarts within a window and an exponential backoff, and the runtime SHALL apply it when the invoked actor fails.

#### Scenario: Restart with backoff
- **WHEN** an invoked actor with `vt:restart="on-failure"`, a limit of 3 and a base backoff of 100 ms fails twice
- **THEN** it is restarted after about 100 ms and then about 200 ms, and the parent receives no error event

#### Scenario: Restart limit exceeded
- **WHEN** the same actor fails a fourth time within the window
- **THEN** it is not restarted and the parent receives `error.execution` with the invoke id and the failure cause

#### Scenario: No policy means SCXML default
- **WHEN** an invoked actor without `vt:restart` fails
- **THEN** the parent receives `error.execution` and the child is not restarted

### Requirement: Actor Failure Isolation
A failure inside one actor's turn SHALL NOT corrupt other actors or the actor's own durable state; the failed turn SHALL leave the journal unchanged.

#### Scenario: Panic during a turn
- **WHEN** an action implementation panics during a macrostep
- **THEN** nothing from that macrostep is appended to the journal, the actor is deactivated, other actors keep running, and the triggering message remains unacknowledged for redelivery
