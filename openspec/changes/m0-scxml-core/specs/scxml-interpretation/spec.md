## Purpose

Defines how Vertep executes a well-formed chart: a pure step function that implements the W3C SCXML interpretation algorithm (Appendix D) and reports every interaction with the outside world as a deterministic effect instead of performing I/O.

## ADDED Requirements

### Requirement: Pure Step Function
The interpreter SHALL expose session start and event processing as pure functions from a session state and an input to a new session state and an ordered list of effects; they SHALL perform no I/O, read no clock, use no randomness, and SHALL return equal outputs for equal inputs.

#### Scenario: Same input, same output
- **WHEN** the same session state and the same external event are given to the step function twice
- **THEN** both calls return equal session states and equal effect lists

#### Scenario: No side effects outside the result
- **WHEN** a step executes a `send`, a `log` and an `invoke`
- **THEN** nothing is sent, logged or started by the step itself, and each appears as an effect in the returned list

### Requirement: Session Initialization
The interpreter SHALL start a session by initializing the datamodel (all data items under early binding, top-level data only under late binding), executing the top-level script, entering the initial configuration through the root's initial transition, and running macrosteps until the configuration is stable before accepting the first external event (W3C SCXML Appendix D, `interpret`).

#### Scenario: Initial configuration entered
- **WHEN** a session starts on a chart whose root initial is compound state `a` with default child `a1`
- **THEN** the resulting configuration is `{a, a1}` and the `onentry` content of `a` then `a1` has run

#### Scenario: Eventless transitions taken at start
- **WHEN** the initial state has an eventless transition whose condition is true
- **THEN** that transition is taken during session start, before any external event is processed

### Requirement: Macrostep Execution
The interpreter SHALL process one external event per macrostep and SHALL run to completion: after each microstep it SHALL take enabled eventless transitions, then dequeue internal events one at a time, until no eventless transition is enabled and the internal queue is empty (W3C SCXML Appendix D, `mainEventLoop`).

#### Scenario: Internal events before the next external event
- **WHEN** processing external event `e1` raises internal events `i1` and `i2`, and external event `e2` is queued
- **THEN** `i1` and `i2` are processed in that order before `e2`

#### Scenario: Eventless transitions take priority over internal events
- **WHEN** after a microstep both an eventless transition is enabled and the internal queue holds an event
- **THEN** the eventless transition is taken first

#### Scenario: Unbounded eventless loop is cut by a budget
- **WHEN** a chart's eventless transitions form a cycle that never stabilizes
- **THEN** the interpreter stops after the configured microstep budget and reports a budget-exceeded error instead of running forever

### Requirement: Event Descriptor Matching
The interpreter SHALL match an event against a transition's event descriptors by tokens: a descriptor matches when it equals the event name or is a dot-separated prefix of it, the descriptor `*` matches every event, and a trailing `.*` is ignored (W3C SCXML 3.12.1).

#### Scenario: Prefix match
- **WHEN** a transition has event descriptor `error` and the event is `error.execution`
- **THEN** the transition matches

#### Scenario: Partial token does not match
- **WHEN** a transition has event descriptor `err` and the event is `error.execution`
- **THEN** the transition does not match

#### Scenario: Wildcard
- **WHEN** a transition has event descriptor `*`
- **THEN** it matches every event

### Requirement: Optimal Enabled Transition Set
The interpreter SHALL select transitions by visiting the atomic states of the configuration in document order and, for each, choosing the first transition in document order on that state or its nearest ancestor whose event descriptors match and whose condition is true, and SHALL then remove conflicting transitions so that a transition whose source is a descendant of another's source wins, and otherwise the transition selected earlier wins (W3C SCXML 3.13 and Appendix D, `selectTransitions` and `removeConflictingTransitions`).

#### Scenario: Descendant wins over ancestor
- **WHEN** atomic state `a1` and its parent `a` both have an enabled transition for event `e`
- **THEN** only the transition of `a1` is selected

#### Scenario: Parallel regions take transitions together
- **WHEN** two regions of a parallel state each have an atomic state with a non-conflicting enabled transition for `e`
- **THEN** both transitions are selected and taken in one microstep

#### Scenario: Conflict resolved by document order
- **WHEN** two regions of a parallel state have enabled transitions for `e` whose exit sets intersect and neither source is a descendant of the other
- **THEN** only the transition selected first in document order is kept

#### Scenario: Condition false skips the transition
- **WHEN** the first matching transition has a condition that evaluates to false and a later matching transition has none
- **THEN** the later transition is selected

### Requirement: Exit And Entry Order
The interpreter SHALL compute the exit set and entry set of a microstep from the transitions' domains, SHALL exit states in reverse document order with descendants before ancestors, executing `onexit` content and cancelling invocations, SHALL then execute the transitions' content in document order, and SHALL enter states in document order with ancestors before descendants, executing `onentry` content (W3C SCXML Appendix D, `microstep`).

#### Scenario: Exit deepest first
- **WHEN** a transition leaves configuration `{a, a1}` for a state outside `a`
- **THEN** the `onexit` content of `a1` runs before that of `a`

#### Scenario: Transition content between exit and entry
- **WHEN** a transition with executable content moves from `a1` to `b1`
- **THEN** `onexit` of `a1` runs first, then the transition content, then `onentry` of `b1`

#### Scenario: Parallel regions entered in document order
- **WHEN** a parallel state with regions `r1` and `r2` is entered
- **THEN** `r1` and its descendants are entered before `r2`

### Requirement: Internal Transitions
The interpreter SHALL NOT exit or re-enter the source state of a transition of type internal whose source is a compound state and whose targets are all its proper descendants, and SHALL treat every other transition as external (W3C SCXML 3.13).

#### Scenario: Internal transition keeps the source
- **WHEN** an internal transition on compound state `a` targets its child `a2` while `a1` is active
- **THEN** `a1` is exited and `a2` entered, and the `onexit` and `onentry` content of `a` do not run

#### Scenario: External transition re-enters the source
- **WHEN** the same transition is declared external
- **THEN** `a` is exited and re-entered

### Requirement: History Restoration
The interpreter SHALL record, when exiting a state that has history children, the active atomic descendants for deep history and the active children for shallow history, and SHALL, when a transition targets a history state, enter the recorded states, or the targets of the history's default transition when nothing is recorded (W3C SCXML 3.10).

#### Scenario: Shallow history restores the child
- **WHEN** compound state `a` with shallow history `h` is exited while `a2` (with default child `a2x`) is active, and later a transition targets `h`
- **THEN** `a2` is entered and its own default child `a2x` is entered by default

#### Scenario: Deep history restores the leaf
- **WHEN** the same history is deep and `a2y` was the active leaf under `a2`
- **THEN** re-entering through the history enters `a`, `a2` and `a2y`

#### Scenario: Default transition when no history recorded
- **WHEN** a transition targets a history state whose parent was never active
- **THEN** the targets of the history's default transition are entered and its content runs

### Requirement: Final States And Done Events
The interpreter SHALL raise `done.state.<id>` for a compound state when one of its final children is entered and for a parallel state when all of its regions are in final states, with the evaluated `donedata` as event data, and SHALL end the session when a top-level final state is entered (W3C SCXML 3.7 and 5.5).

#### Scenario: Compound done event
- **WHEN** final child `f` of compound state `a` is entered
- **THEN** the internal event `done.state.a` is queued with the evaluated `donedata` of `f`

#### Scenario: Parallel done event
- **WHEN** the last region of parallel state `p` reaches a final state
- **THEN** the internal event `done.state.p` is queued

#### Scenario: Top-level final ends the session
- **WHEN** a final child of the root is entered
- **THEN** the session ends after the macrostep and a session-done effect carries the `donedata`

### Requirement: Internal And External Queues
The interpreter SHALL keep an internal queue fed by `raise`, by `send` with target `#_internal` and by platform events, and SHALL accept external events only as step inputs, processing internal events strictly before the next external event.

#### Scenario: Raise is internal
- **WHEN** executable content runs `<raise event="x"/>`
- **THEN** `x` is placed on the internal queue and processed in the same macrostep

#### Scenario: Send to the internal target
- **WHEN** executable content runs a `send` with target `#_internal` and no delay
- **THEN** the event is placed on the internal queue, not emitted as an effect

### Requirement: Execution Errors
The interpreter SHALL place `error.execution` on the internal queue when an expression fails to evaluate or an assignment target is invalid and SHALL stop the enclosing executable content block at that element, SHALL place `error.communication` when a send target or type cannot be handled, and SHALL continue processing (W3C SCXML 5.10 and 6.2).

#### Scenario: Failed expression stops the block
- **WHEN** the second of three elements of an `onentry` block fails to evaluate
- **THEN** the third element does not run and `error.execution` is queued

#### Scenario: Invalid send type
- **WHEN** a `send` names a type that no Event I/O Processor supports
- **THEN** `error.communication` is queued and the session continues

### Requirement: Send And Cancel Effects
The interpreter SHALL turn every `send` whose target is not `#_internal` into a send effect carrying the evaluated event name, data, target, type, delay and a send identifier, SHALL generate send identifiers deterministically from the session identifier and a per-session counter when none is given, and SHALL turn `cancel` into a cancel effect for that send identifier.

#### Scenario: Delayed send becomes an effect
- **WHEN** executable content runs `<send event="tick" delay="5s"/>`
- **THEN** a send effect with a delay of five seconds and a generated send identifier is emitted, and no timer is started by the interpreter

#### Scenario: Deterministic send identifiers
- **WHEN** two runs of the same session process the same events
- **THEN** they generate the same send identifiers in the same order

#### Scenario: Cancel by send identifier
- **WHEN** executable content runs `<cancel sendid="s1"/>`
- **THEN** a cancel effect for send identifier `s1` is emitted

### Requirement: Invoke Lifecycle
The interpreter SHALL emit an invoke effect at the end of each macrostep for every state entered and not exited during it, SHALL emit a cancel-invoke effect when the invoking state is exited, SHALL run the `finalize` content of an invoke before processing an external event that carries its invoke identifier, SHALL request forwarding of each external event to every invoke with `autoforward`, and SHALL accept `done.invoke.<id>` from the host as an external event that marks the invocation as completed so it is not cancelled on exit (W3C SCXML 6.4).

#### Scenario: Invoke started after the macrostep
- **WHEN** a macrostep enters state `s` that declares an invoke and stays in `s`
- **THEN** an invoke effect for it is emitted at the end of the macrostep

#### Scenario: Invoke not started when the state is left in the same macrostep
- **WHEN** a macrostep enters `s` and an eventless transition leaves it in the same macrostep
- **THEN** no invoke effect is emitted for `s`

#### Scenario: Finalize runs before the event
- **WHEN** an external event from invocation `i1` arrives and `i1` declares a `finalize` block
- **THEN** the `finalize` content runs before transitions are selected for that event

#### Scenario: Exiting the state cancels the invocation
- **WHEN** the invoking state is exited
- **THEN** a cancel-invoke effect for its invocation is emitted

### Requirement: Session Termination
The interpreter SHALL terminate a session on entering a top-level final state or on a cancel request by exiting every active state in exit order, running `onexit` content, emitting cancel-invoke effects for active invocations, and emitting one session-done effect, after which it SHALL reject further events.

#### Scenario: Exit content runs on termination
- **WHEN** a session is cancelled while in configuration `{a, a1}`
- **THEN** `onexit` of `a1` then `a` runs and a session-done effect is emitted

#### Scenario: Events after termination are rejected
- **WHEN** an event is given to a terminated session
- **THEN** the step returns a session-terminated error and no effects
