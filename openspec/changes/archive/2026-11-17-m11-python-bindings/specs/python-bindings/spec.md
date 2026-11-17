## Purpose

Runs Vertep charts and actors from Python. The same SCXML semantics, typed events and event-sourced journals as the native runtime and the JavaScript bindings are exposed through a synchronous Python API, installed with `pip install vertep`.

## ADDED Requirements

### Requirement: Native Extension Module
The package SHALL ship a native extension module built from the Rust workspace, whose version MUST equal the workspace version. A system MUST belong to the thread that created it.

#### Scenario: Version matches the workspace
- **WHEN** the package is imported
- **THEN** `vertep.__version__` equals the version of the Rust workspace
- **AND** the release tooling fails when the package and the workspace versions differ

#### Scenario: Another thread is refused
- **WHEN** a thread other than the creator calls into an actor system
- **THEN** the call fails without touching the system's state

### Requirement: Actor Creation And Event Sending From Python
The API SHALL create actors from SCXML, YAML or RDF chart sources, validated exactly as natively, address virtual actors by chart type and key, and send events whose payload is given as mapping fields or keyword arguments. A repeated message id MUST be applied once.

#### Scenario: Create from SCXML text
- **WHEN** an application spawns an actor from SCXML text with a chart type and a key
- **THEN** its address is the chart type and the key, its snapshot holds the initial configuration at position 0, and its version is a content-addressed chart IRI

#### Scenario: Payload as keyword arguments
- **WHEN** an application sends `add` with `qty=2` and, separately, the mapping `{"type": "add", "qty": 1}`
- **THEN** both are the same kind of event and the datamodel sees `qty` as a payload field

#### Scenario: Repeated message id is applied once
- **WHEN** an application sends two events with the same message id
- **THEN** the journal holds one entry and the state reflects one application

#### Scenario: Registered type activates by key
- **WHEN** a chart type is registered and an application addresses it with a key it has not used before
- **THEN** the first message activates that actor, and the actor appears in the system's actor list

### Requirement: Snapshots And Subscriptions
The API SHALL deliver one snapshot per macrostep that changed the configuration or the context, in journal order, and MAY deliver one notice per macrostep that took a transition. Callbacks MUST be able to send events.

#### Scenario: One snapshot per changing macrostep
- **WHEN** a subscribed actor receives events of which one no transition takes
- **THEN** the subscriber hears a snapshot for each other macrostep and none for that one, and hears nothing after cancelling

#### Scenario: Callbacks may send events
- **WHEN** a subscriber's callback sends an event to the actor it observes
- **THEN** the event is applied after the current delivery completes and its snapshot is delivered in order

### Requirement: Journal Inspection And Time Travel
The API SHALL list journal entries with the transitions they took, rebuild the state after any position without disturbing the live actor, report whether an event would take a transition without sending it, and run SPARQL on the state graph.

#### Scenario: As of a position
- **WHEN** an application asks for the state as of position 1 of an actor with later entries
- **THEN** it receives the configuration and context after entry 1 and the live actor is unchanged

#### Scenario: Can does not send
- **WHEN** an application asks whether an event can be taken
- **THEN** the answer reflects the current configuration and the journal does not grow

#### Scenario: Graph query
- **WHEN** an application runs a SPARQL query on a system that has actors
- **THEN** it receives rows of N-Triples terms, a boolean or triples according to the query form
- **AND** a query as of a time on the in-memory store fails with `InvalidArgument`

### Requirement: Explicit Time
The system's clock SHALL be chosen explicitly as the system clock or a virtual clock. Time MUST pass only inside calls, wall-clock time MUST enter actors only as journaled timestamps, and a journal MUST replay to the same states under any clock.

#### Scenario: Virtual clock fires timers when advanced
- **WHEN** an actor schedules a delayed send on a virtual clock and the clock is advanced past the delay
- **THEN** the delayed event is delivered as a journaled input at the advanced time, and not before

#### Scenario: Journal replays under a different clock
- **WHEN** a journal written under one clock is opened by a system whose clock starts at another time
- **THEN** the actor's state equals the state it had

#### Scenario: System clock waits in real time
- **WHEN** an application waits on the system clock while a timer is pending
- **THEN** the call returns after the requested time with the timer fired
- **AND** advancing a system clock fails with `InvalidArgument`

### Requirement: Durable File Storage
Storage SHALL be chosen explicitly as memory or a file. A file store MUST be a tiramemsu file in the native format, exclusive to one open system, and its bytes MUST be exportable.

#### Scenario: Journal survives reopening
- **WHEN** a system on a file is closed and another opens the same file and registers the chart
- **THEN** the actor's state and the state graph are those the first system left

#### Scenario: File is exclusive and closing releases it
- **WHEN** a second system opens a file a first one holds
- **THEN** it fails with `StoreLocked`, and succeeds once the first is closed

#### Scenario: Export journal bytes
- **WHEN** an application exports the journal of a file system
- **THEN** it receives the bytes of the tiramemsu file
- **AND** exporting from a memory system fails with `InvalidArgument`

### Requirement: Typed Errors
Every failure SHALL raise an exception that derives from `VertepError` and carries the bridge's stable error code. An invalid chart MUST list every violation with its position.

#### Scenario: Invalid chart lists its violations
- **WHEN** an application registers a malformed or invalid chart
- **THEN** `ChartInvalid` is raised with a violation per problem, each with a line where one is known

#### Scenario: Misuse raises a typed error
- **WHEN** an application addresses an unregistered chart type, passes a malformed type IRI, an unknown storage kind or a reserved payload field
- **THEN** `NotFound` or `InvalidArgument` is raised as appropriate

#### Scenario: Closed system raises
- **WHEN** an application calls into a closed system or an actor of it
- **THEN** `SystemClosed` is raised

### Requirement: Python Guards And Actions
Python callables SHALL bind as the named guards and actions a chart's `rust` datamodel declares, with the contract of the JavaScript implementations. A development mode MUST report a guard or action whose replay differs.

#### Scenario: Guard and action bound by IRI
- **WHEN** a system is created with a guard and an action bound by IRI and a chart uses them
- **THEN** the guard decides the transition, and the action's returned context changes and raised events take effect in the same macrostep

#### Scenario: Impure guard is reported in dev mode
- **WHEN** a system in development mode runs a guard that answers differently when replayed
- **THEN** an `ImpurityWarning` is issued

#### Scenario: Callback must be callable
- **WHEN** a system is created with a binding that is not callable
- **THEN** creation fails with `ValueError`

### Requirement: Runnable Examples
The repository SHALL include Python examples whose output is recorded next to them and compared by the test suite, so that an example that drifts fails the build.

#### Scenario: Examples print their recorded output
- **WHEN** the test suite runs each example in a fresh interpreter
- **THEN** its output equals the recorded output

### Requirement: Package Publishing
The package SHALL be published to PyPI as one `abi3` wheel per supported platform and a source distribution, built and uploaded by a workflow that uses Trusted Publishing and tests each wheel in a clean environment before upload.

#### Scenario: Wheels are smoke-tested before upload
- **WHEN** the release workflow builds a wheel
- **THEN** it installs the wheel into a fresh virtual environment and runs an actor through the public API before any upload

#### Scenario: Tag must match the package version
- **WHEN** a release is published whose tag differs from the package version
- **THEN** the workflow stops before building
