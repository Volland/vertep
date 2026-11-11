# rust-dsl Specification

## Purpose
Let Rust developers author statecharts inline with the `statechart!` macro or import SCXML and YAML files with `include_chart!`, both producing the same typed Rust API checked at compile time, and run them on the `rust` datamodel of typed context and named functions.

## Requirements

### Requirement: Statechart Macro
The `statechart!` macro SHALL accept a chart written in Rust token syntax covering every SCXML state kind (atomic, compound, parallel, final, shallow and deep history), transitions with event descriptors, guards, targets and internal or external type, entry and exit actions, `<invoke>`, `<send>`, `<raise>`, `<data>` and the type declarations of the type language, and SHALL build the same IR the SCXML reader would build for the equivalent document.

#### Scenario: Equivalent to SCXML
- **WHEN** a chart is written with `statechart!` and the equivalent chart is written in SCXML
- **THEN** both produce the same IR and the same chart version IRI

#### Scenario: Parallel and history states
- **WHEN** a `statechart!` chart declares a parallel state with two regions and a deep history state
- **THEN** the generated IR contains the parallel state, both regions and the history state with its default transition

#### Scenario: Syntax error is located
- **WHEN** a `statechart!` invocation contains a malformed transition
- **THEN** compilation fails with an error whose span points at the malformed transition

### Requirement: Include Chart Macro
The `include_chart!` macro SHALL read an SCXML or YAML file at compile time, relative to the crate's manifest directory, and SHALL generate exactly the same typed Rust API that `statechart!` generates for an equivalent chart, and the build SHALL be rerun when the file changes.

#### Scenario: Same API from SCXML and from the macro
- **WHEN** `include_chart!("order.scxml")` and an equivalent `statechart!` are compiled in two modules
- **THEN** both modules expose the same public type names, variants and function signatures

#### Scenario: File change triggers rebuild
- **WHEN** the included YAML file is edited
- **THEN** the next `cargo build` recompiles the crate that includes it

#### Scenario: Missing file
- **WHEN** the path given to `include_chart!` does not exist
- **THEN** compilation fails with an error naming the resolved absolute path

### Requirement: Generated Typed API
For every chart, the macros SHALL generate a state enum mirroring the state hierarchy, an event enum with one variant per typed event carrying its payload type, a context struct, the chart IR as a constant, the chart version IRI as a constant and a typed constructor that binds the chart's guard and action functions, and sending an event through the typed API SHALL be impossible with a payload of the wrong type.

#### Scenario: Wrong payload does not compile
- **WHEN** code sends `Event::Pay { amount: "ten" }` where `amount` is `decimal`
- **THEN** the code fails to compile

#### Scenario: State enum reflects hierarchy
- **WHEN** the chart has compound state `checkout` with children `cart` and `payment`
- **THEN** the generated state type lets code test whether the active configuration includes `checkout`, `cart` or `payment` without string comparison

#### Scenario: Chart version constant
- **WHEN** a crate is compiled with a chart
- **THEN** the generated chart version IRI constant equals the IRI that `vt hash` computes for the same chart

### Requirement: Compile-Time Structural Checks
The macros SHALL reject at compile time any chart with a transition target that names no state, a duplicate state id, a compound state without a reachable initial child, a history state outside a compound or parallel state, or any other violation of the chart model's well-formedness rules, reporting each violation with a source span.

#### Scenario: Unknown target
- **WHEN** a transition targets `paymnt` and no state `paymnt` exists
- **THEN** compilation fails with an error at the transition naming `paymnt` and suggesting `payment`

#### Scenario: Duplicate state id
- **WHEN** two states are both named `done`
- **THEN** compilation fails with errors at both declarations

#### Scenario: Several violations in one chart
- **WHEN** a chart contains an unknown target and a duplicate id
- **THEN** compilation reports both errors

### Requirement: Compile-Time Reachability and Coverage Diagnostics
The macros SHALL report states that are unreachable from the initial configuration and typed events that no transition in any state handles, each as a warning by default, and a chart attribute SHALL turn either diagnostic into a compile error.

#### Scenario: Unreachable state warning
- **WHEN** state `archived` has no incoming transition and is not initial
- **THEN** compilation succeeds with a warning naming `archived`

#### Scenario: Unhandled event as error
- **WHEN** the chart sets the attribute that makes unhandled events errors and event `refund` is declared but never handled
- **THEN** compilation fails with an error naming `refund`

#### Scenario: Eventless transitions count toward reachability
- **WHEN** a state is reachable only through an eventless transition
- **THEN** it is not reported as unreachable

### Requirement: Guard and Action Signature Checking
Each guard and action named in a chart SHALL be bound to a Rust function whose signature matches the chart's types: a guard takes a shared reference to the context and the typed event and returns `bool`, and an action takes the context mutably and the typed event and returns either nothing or a list of emitted events, and a mismatch SHALL be a compile error.

#### Scenario: Guard with wrong event type
- **WHEN** guard `is_large` is declared for transitions on `pay` but its function takes the payload of `cancel`
- **THEN** compilation fails with an error naming `is_large` and both types

#### Scenario: Missing function
- **WHEN** a chart names action `notify` and no function is bound to it
- **THEN** compilation fails with an error naming `notify`

#### Scenario: Guard tries to mutate context
- **WHEN** a guard function takes the context mutably
- **THEN** compilation fails because guards receive only a shared reference

### Requirement: Rust Datamodel
The `rust` datamodel SHALL evaluate guards and actions by calling the Rust functions bound to them, hold context as the generated context struct, and expose SCXML system variables `_event`, `_sessionid`, `_name` and `_ioprocessors` through typed accessors.

#### Scenario: Guard evaluation
- **WHEN** an event is processed and a transition's guard is bound to a Rust function
- **THEN** the datamodel calls that function with the current context and the typed event and uses its result for transition selection

#### Scenario: System variables
- **WHEN** an action reads the current event's name and session id
- **THEN** it obtains them through typed accessors without string lookups

### Requirement: Functions Registered by IRI
Every guard and action SHALL be identified in the chart's IR and RDF by an IRI, and a runtime registry SHALL resolve these IRIs to Rust functions so that a chart loaded from RDF or from a file at runtime can be bound to compiled functions, failing when an IRI has no registered function.

#### Scenario: Chart loaded from RDF binds functions
- **WHEN** a chart compiled with `statechart!` is stored as RDF and later loaded from the store at runtime in a process that registered the same functions
- **THEN** every guard and action IRI resolves and the chart runs as compiled

#### Scenario: Unregistered IRI
- **WHEN** a chart references guard IRI `vt:guard/isPaid` and no function is registered under it
- **THEN** instantiation fails with an error naming the IRI

#### Scenario: Registry rejects duplicates
- **WHEN** two different functions are registered under the same IRI
- **THEN** registration fails with an error naming the IRI

### Requirement: Pure Guards
Guards SHALL be pure: a guard SHALL NOT mutate context, send events or perform I/O, and repeated evaluation with the same context and event SHALL return the same result; the Rust datamodel SHALL enforce this by construction for context mutation and by documented contract for I/O.

#### Scenario: Repeated evaluation
- **WHEN** the same guard is evaluated twice with the same context and event
- **THEN** both evaluations return the same result

#### Scenario: Guard cannot emit events
- **WHEN** a guard function attempts to obtain an event emitter
- **THEN** the code fails to compile because guards receive no emitter

### Requirement: Actions Emit Effects as Events
Actions SHALL produce side effects only by mutating context and by emitting `<raise>` or `<send>` events through the action interface, so that every effect of a macrostep is visible in its output and can be journaled.

#### Scenario: Action sends a message
- **WHEN** an action emits a `<send>` to another actor
- **THEN** the send appears in the effects returned by the step rather than being delivered directly from the action

#### Scenario: Action raises an internal event
- **WHEN** an action raises `checked`
- **THEN** `checked` is placed on the internal queue and processed in the same macrostep
