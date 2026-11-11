## Purpose

Runs Vertep charts and actors in browsers and Node.js. The same SCXML semantics, typed events and event-sourced journals as the native runtime are exposed through a TypeScript API, with storage in OPFS or memory and interop with server actors.

## ADDED Requirements

### Requirement: Core Builds For WebAssembly Unchanged
The chart core (IR, type language, datamodel trait and step function) SHALL compile for the `wasm32-unknown-unknown` target from the same source as the native build, with no target-specific code paths. For the same chart and the same journaled inputs, it MUST produce the same configurations, datamodel values and effects as the native build.

#### Scenario: Core compiles for wasm32 without changes
- **WHEN** the core is built for `wasm32-unknown-unknown` with its default features
- **THEN** the build succeeds without any conditional compilation on the target architecture inside the core
- **AND** the core links neither `std` nor any browser API

#### Scenario: Native and WebAssembly steps agree
- **WHEN** the same chart is stepped over the same sequence of journaled input events natively and in WebAssembly
- **THEN** both produce identical configurations, datamodel values and effect lists after every macrostep

### Requirement: Browser Runtime Shell
The browser shell SHALL implement the runtime's spawn, clock and sleep contract on the JavaScript event loop. Wall-clock time MUST enter actors only as journaled timestamps.

#### Scenario: Timer fires through the event loop
- **WHEN** an actor executes `<send event="tick" delay="2s"/>` in the browser
- **THEN** the shell records the timer in the journal and delivers `tick` as an input event no earlier than two seconds later
- **AND** the delivered event carries the journaled timestamp, not a clock read inside the step

#### Scenario: Due timers fire after reopening
- **WHEN** a page with a durable journal is closed while a timer is pending and reopened after its due time
- **THEN** the shell fires the overdue timer as an input event once, after the actor system has started

#### Scenario: Step never reads the clock
- **WHEN** a journal recorded in the browser is replayed later
- **THEN** replay yields the same states regardless of the current wall-clock time

### Requirement: Worker-Hosted Actor System
In a browser, the actor system SHALL run inside a dedicated Web Worker. The main thread MUST communicate with it only through asynchronous messages, so that no call to the API blocks the page.

#### Scenario: Actor system starts in a worker
- **WHEN** an application creates an actor system in a browser
- **THEN** the system, its actors and its journal run in a dedicated Web Worker
- **AND** every API call on the main thread returns a Promise or delivers results through a subscription

#### Scenario: Implementations load inside the worker
- **WHEN** an application registers named guard and action implementations as a module
- **THEN** the worker imports that module and calls the implementations synchronously during steps, without a round trip to the main thread

#### Scenario: Worker failure is reported
- **WHEN** the worker terminates unexpectedly
- **THEN** every pending Promise rejects with a `WorkerTerminated` error and later calls fail with the same error

### Requirement: Browser Journal Storage
The browser runtime SHALL store actor journals and graphs with tiramemsu on its WebAssembly host. Storage MUST be either `opfs` (durable) or `memory` (volatile), chosen explicitly at creation, and the runtime MUST NOT fall back silently from one to the other.

#### Scenario: Durable journal survives reload
- **WHEN** an actor system with `opfs` storage processes events and the page is reloaded
- **THEN** reactivated virtual actors replay their journals and reach the same configuration and context as before the reload

#### Scenario: Unavailable OPFS is an error
- **WHEN** `opfs` storage is requested where OPFS sync access handles are unavailable
- **THEN** creation fails with a `StorageUnavailable` error and no memory store is opened in its place

#### Scenario: Second writer is refused
- **WHEN** a second tab or worker opens an `opfs` store that another worker holds open
- **THEN** the open fails with a `StoreLocked` error and the first worker's journal is unaffected

#### Scenario: Journal file round-trips with native
- **WHEN** a journal file exported from the browser is opened by the native tiramemsu backend
- **THEN** native replay of each actor yields the same states as in the browser, and a file written natively can be imported into the browser in the same way

### Requirement: Actor Creation From Chart Sources
The TypeScript API SHALL create actors from SCXML documents, YAML charts and RDF chart graphs. Each chart source MUST be validated and content-addressed exactly as in the native runtime before any actor runs it.

#### Scenario: Create from SCXML text
- **WHEN** an application spawns an actor from a valid SCXML document
- **THEN** the actor starts in the chart's initial configuration and its snapshot reports the chart version IRI

#### Scenario: Same chart, same version IRI
- **WHEN** the same chart is supplied once as SCXML and once as its RDF projection
- **THEN** both actors report the same chart version IRI

#### Scenario: Invalid chart is rejected
- **WHEN** an application spawns an actor from a chart that fails validation
- **THEN** the call rejects with a `ChartInvalid` error that lists each violation with its location, and no actor is created

#### Scenario: Virtual actor addressed by type and key
- **WHEN** an application addresses a virtual actor by chart type and key that has never been activated
- **THEN** the runtime activates it on the first message, and later calls with the same type and key reach the same actor

### Requirement: Event Sending From JavaScript
The API SHALL let JavaScript send events to any actor address and make promise-based requests. Each external message MUST carry a message id, so that a duplicate delivery is processed once.

#### Scenario: Send is processed in one macrostep
- **WHEN** JavaScript sends an event to a running actor
- **THEN** the actor processes it in exactly one macrostep after any events already queued in its mailbox

#### Scenario: Request resolves with the reply
- **WHEN** JavaScript calls `request` on an actor whose chart replies to the request event
- **THEN** the returned Promise resolves with the typed reply payload

#### Scenario: Request times out
- **WHEN** no reply arrives within the request's timeout
- **THEN** the Promise rejects with a `RequestTimeout` error, and a late reply is not delivered to the caller

#### Scenario: Duplicate send is deduplicated
- **WHEN** the same message id is sent twice to a virtual actor
- **THEN** the actor journals and processes it once

### Requirement: Snapshot And Transition Subscriptions
The API SHALL let JavaScript subscribe to an actor's snapshots and transitions. Notifications MUST arrive in journal order, at most once per macrostep, and MUST NOT expose intermediate microstep states.

#### Scenario: Snapshot after each macrostep
- **WHEN** a subscriber is attached and the actor completes a macrostep that changes its configuration or context
- **THEN** the subscriber receives one snapshot with the configuration, typed context, journal position and chart version IRI

#### Scenario: Transition notification lists taken transitions
- **WHEN** a subscriber listens for transitions and an event causes transitions to be taken
- **THEN** it receives the triggering event and the taken transitions with their source and target states, in execution order

#### Scenario: Unsubscribe stops notifications
- **WHEN** a subscriber unsubscribes
- **THEN** it receives no further notifications, and the actor's processing is unaffected

### Requirement: Journal Inspection And Time Travel
The API SHALL let JavaScript read an actor's journal and get its snapshot as of any journal position or transaction time. Time travel MUST be read-only.

#### Scenario: Read journal entries
- **WHEN** JavaScript reads an actor's journal from a position
- **THEN** it receives the entries in order, each with its position, input event, message id, timestamp and chart version IRI

#### Scenario: Snapshot as of a past position
- **WHEN** JavaScript requests an actor's snapshot as of an earlier journal position
- **THEN** it receives the configuration and context the actor had after that position, and the live actor is unchanged

#### Scenario: Graph query over history
- **WHEN** JavaScript runs a SPARQL query against the actor system's graph store as of a past time
- **THEN** the results reflect the current-state graph believed at that time

### Requirement: Generated TypeScript Types
The tooling SHALL generate TypeScript declarations from a chart's IR types: a union of its events with typed payloads, its context type, and a union of its state identifiers. A typed chart MUST make the API reject ill-typed events at compile time.

#### Scenario: Ill-typed event is a compile error
- **WHEN** TypeScript code sends an event whose name is not in the generated event union, or whose payload does not match its declared type
- **THEN** type checking fails

#### Scenario: Generated types match the other projections
- **WHEN** types are generated for a chart in TypeScript and in JSON Schema
- **THEN** every value that one projection accepts is accepted by the other

#### Scenario: Untyped chart falls back to dynamic values
- **WHEN** declarations are generated for a chart without types
- **THEN** event payloads and context are typed as a dynamic JSON value, and the chart still runs

### Requirement: SCXML Semantics In JavaScript
The JavaScript API SHALL offer XState-inspired ergonomics (snapshots, a `matches` check on state identifiers, a `can` check on events, subscriptions) while executing SCXML semantics exactly. The API MUST NOT offer a mode that changes execution semantics.

#### Scenario: Document order decides conflicts
- **WHEN** a chart has two enabled transitions on the same event in conflicting states
- **THEN** the actor takes the transition that SCXML's optimal-enabled-transition-set rules select, the same as the native runtime

#### Scenario: Can reflects enabled transitions
- **WHEN** JavaScript asks whether an actor can accept an event in its current configuration
- **THEN** the answer is true exactly when that event enables at least one transition or is deferred by an active state, evaluated without changing state or journaling anything

#### Scenario: Matches checks active states
- **WHEN** JavaScript calls `matches` with a state identifier on a snapshot
- **THEN** it returns true exactly when that state is in the snapshot's configuration

### Requirement: Node.js Support
The same packages SHALL run in Node.js on the current active LTS release and later, with the same API, hosting the actor system in a worker thread and supporting `memory` storage with journal file import and export.

#### Scenario: Same program runs in Node.js
- **WHEN** a program that uses the API with `memory` storage runs in Node.js
- **THEN** it behaves exactly as in a browser and produces the same snapshots and journal entries

#### Scenario: OPFS requested in Node.js
- **WHEN** `opfs` storage is requested in Node.js
- **THEN** creation fails with a `StorageUnavailable` error

#### Scenario: Persist through export
- **WHEN** a Node.js program exports the journal file and a later process imports it
- **THEN** the virtual actors in the later process resume with the same states

### Requirement: npm Package Distribution
The project SHALL publish the JavaScript packages to npm as ES modules with bundled WebAssembly, TypeScript declarations and the worker entry point. Every package version MUST equal the version of the Rust crates it was built from.

#### Scenario: Install and import
- **WHEN** an application installs the main package from npm and imports it in a bundler or directly in a browser
- **THEN** it can create an actor system without building Rust or fetching extra assets by hand

#### Scenario: ECMAScript datamodel is a separate package
- **WHEN** an application needs the ECMAScript datamodel
- **THEN** it installs a separate package for it, and the main package does not contain that engine

#### Scenario: Version matches the crates
- **WHEN** a package is published
- **THEN** its version equals the workspace crate version, and the release fails otherwise

### Requirement: Bundle Size Budget
The main package's WebAssembly and JavaScript SHALL stay within a size budget checked in CI: at most 600 KB gzip-compressed without journal storage, and at most 2.5 MB gzip-compressed with the tiramemsu journal. A release MUST fail when a budget is exceeded.

#### Scenario: Budget exceeded fails CI
- **WHEN** a change makes the gzip-compressed main package without storage larger than 600 KB
- **THEN** the CI size check fails and reports the size and the budget

#### Scenario: Storage-free build excludes SQLite
- **WHEN** an application uses only `memory` actors without a journal
- **THEN** it can load the core and shell without loading the SQLite WebAssembly module

### Requirement: Conformance Suite In WebAssembly
The W3C SCXML conformance suite and the differential traces against the Lean model SHALL run in WebAssembly in headless Chrome and in Node.js. Results MUST match the native run exactly, including the allow-list of expected deviations.

#### Scenario: Suite passes in headless Chrome
- **WHEN** CI runs the conformance suite in headless Chrome
- **THEN** every test passes except those on the shared allow-list, and an unexpected pass or failure fails the run

#### Scenario: Traces equal native traces
- **WHEN** the differential test generator produces charts and event sequences and runs them in WebAssembly and natively
- **THEN** both traces are identical for every generated case

### Requirement: Server Interop Over WebSocket
The runtime SHALL provide a WebSocket transport, registered as an SCXML Event I/O Processor, through which browser and Node.js actors exchange messages with server actors by IRI. Delivery MUST be at least once, and messages MUST be deduplicated by message id at the receiver.

#### Scenario: Browser actor sends to a server actor
- **WHEN** a browser actor executes `<send>` targeting a server actor's IRI through the WebSocket processor
- **THEN** the server actor receives the event with `_event.origintype` naming the WebSocket processor and `_event.origin` naming the browser actor's IRI

#### Scenario: Server actor replies to a browser actor
- **WHEN** a server actor sends an event back to the browser actor's IRI while the connection is open
- **THEN** the browser actor receives it as an external event in its mailbox

#### Scenario: Reconnect redelivers the outbox
- **WHEN** the connection drops after a browser actor journaled a send but before the server acknowledged it
- **THEN** the send is redelivered after reconnecting, and the server processes it once

#### Scenario: Connection authentication is pluggable
- **WHEN** an application supplies a credential provider for the WebSocket processor
- **THEN** each connection presents that credential on connect, and a refused credential rejects pending sends with a `TransportRefused` error
