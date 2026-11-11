# datamodel Specification

## Purpose
Defines how charts evaluate conditions, expressions, assignments and scripts through a pluggable datamodel, with the `null` and `ecmascript` datamodels in M0, so that executable content stays standard SCXML while evaluation remains pure where it must be and deterministic everywhere.

## Requirements

### Requirement: Datamodel Contract
Every datamodel SHALL implement one contract used by the interpreter: initialize data items, evaluate a condition to a boolean, evaluate an expression to a value, assign a value to a location, execute a script, iterate a `foreach` collection, expose system variables, and snapshot and restore its state; each operation SHALL report failure as an evaluation error instead of panicking.

#### Scenario: Evaluation error instead of panic
- **WHEN** an expression references an undefined variable in a datamodel that treats this as an error
- **THEN** the operation returns an evaluation error and the interpreter continues

#### Scenario: Same contract for every datamodel
- **WHEN** a chart runs with the `null` datamodel and an equivalent chart runs with the `ecmascript` datamodel
- **THEN** the interpreter calls both through the same contract

### Requirement: Datamodel Registry
The interpreter SHALL select the datamodel named by the chart from a registry supplied by the host, SHALL use `null` when the chart names none, and SHALL refuse to start a session whose datamodel is not registered.

#### Scenario: Registered datamodel selected
- **WHEN** a chart declares `datamodel="ecmascript"` and the host registered the ecmascript datamodel
- **THEN** the session evaluates all expressions with it

#### Scenario: Missing datamodel refused
- **WHEN** a chart declares a datamodel the host did not register
- **THEN** session start fails with an unsupported-datamodel error

### Requirement: Guard Purity
Condition evaluation SHALL NOT change the datamodel: a condition that attempts to assign, define or mutate any datamodel value SHALL evaluate as an error, be treated as false, and cause `error.execution` to be queued, and the datamodel state after evaluating any condition SHALL equal the state before.

#### Scenario: Mutating condition rejected
- **WHEN** an ecmascript condition is `(x = 5) > 1`
- **THEN** the condition is treated as false, `error.execution` is queued and `x` keeps its previous value

#### Scenario: Read-only condition allowed
- **WHEN** a condition reads `x > 1` and `x` is 2
- **THEN** the condition is true and the datamodel is unchanged

### Requirement: System Variables
Every datamodel that supports data SHALL expose the read-only system variables `_event` (with `name`, `type`, `sendid`, `origin`, `origintype`, `invokeid` and `data`), `_sessionid`, `_name` and `_ioprocessors`, and an attempt to assign to any of them SHALL cause `error.execution` (W3C SCXML 5.10).

#### Scenario: Event fields available
- **WHEN** a transition handling external event `go` with data `{n: 1}` evaluates `_event.data.n`
- **THEN** the value is 1 and `_event.name` is `go`

#### Scenario: Assignment to a system variable
- **WHEN** executable content assigns to `_sessionid`
- **THEN** `error.execution` is queued and `_sessionid` is unchanged

### Requirement: In Predicate
Every datamodel, including `null`, SHALL provide the `In(stateId)` predicate in conditions, true exactly when the named state is in the current configuration.

#### Scenario: Null datamodel condition
- **WHEN** a chart with the `null` datamodel has a transition with `cond="In('b')"` while `b` is active
- **THEN** the condition is true

#### Scenario: State not active
- **WHEN** the same condition is evaluated while `b` is not active
- **THEN** the condition is false

### Requirement: Null Datamodel
The `null` datamodel SHALL support no data, SHALL accept only `In()` predicates as conditions, and SHALL treat any other condition, any `assign`, any `script` and any value expression as an evaluation error (W3C SCXML B.1).

#### Scenario: Assign in a null chart
- **WHEN** a chart with the `null` datamodel executes `assign`
- **THEN** `error.execution` is queued

#### Scenario: Pure control flow runs
- **WHEN** a chart with the `null` datamodel uses only events, `In()` conditions, `raise` and `send` with literal attributes
- **THEN** it runs to completion without errors

### Requirement: ECMAScript Datamodel
The `ecmascript` datamodel SHALL implement W3C SCXML B.2 on an embedded ECMAScript engine: data items become variables of one global scope per session, uninitialized data is `undefined`, inline content that parses as JSON becomes a structured value and otherwise a string, `foreach` iterates arrays in index order with shallow-copied items, and the datamodel is available only when its feature is enabled.

#### Scenario: Data as variables
- **WHEN** a chart declares `<data id="count" expr="0"/>` and an `assign` sets `count` to `count + 1`
- **THEN** a later condition `count == 1` is true

#### Scenario: JSON content
- **WHEN** a data item has inline content `{"a": [1, 2]}`
- **THEN** the expression `x.a[1]` evaluates to 2

#### Scenario: Feature disabled
- **WHEN** the core is built without the ecmascript feature
- **THEN** the ecmascript datamodel is not registered and charts that require it are refused

### Requirement: Data Binding Modes
The datamodel SHALL create and initialize all data items at session start under early binding, and SHALL create all data items at session start but assign their initial values only when their owning state is first entered under late binding (W3C SCXML 5.3).

#### Scenario: Early binding
- **WHEN** a chart with early binding declares data `x` in a state that is not yet entered
- **THEN** `x` already holds its initial value at session start

#### Scenario: Late binding
- **WHEN** a chart with late binding declares data `x` with `expr="1"` in state `s`
- **THEN** `x` is undefined until `s` is first entered and holds 1 afterwards

### Requirement: Deterministic Evaluation
Every datamodel SHALL evaluate deterministically: identical datamodel states and inputs SHALL produce identical results, and sources of nondeterminism such as the current time and random numbers SHALL come only from values supplied by the host through the session input, never from the ambient environment.

#### Scenario: Random numbers are reproducible
- **WHEN** an ecmascript expression calls `Math.random()` in two runs of the same session with the same inputs
- **THEN** both runs produce the same value

#### Scenario: Time comes from the input
- **WHEN** an ecmascript expression reads the current date while processing an event
- **THEN** the value is the timestamp supplied with that event, not the wall clock

### Requirement: Datamodel Snapshot
Every datamodel SHALL serialize its complete state to a datamodel-independent value and restore it from that value, such that restoring a snapshot and continuing produces the same results as never having snapshotted.

#### Scenario: Snapshot and restore
- **WHEN** a session's datamodel is snapshotted after three events, restored into a fresh session and given a fourth event
- **THEN** the result equals the result of the original session processing the fourth event

#### Scenario: Snapshot of the null datamodel
- **WHEN** a `null` datamodel is snapshotted
- **THEN** the snapshot is the empty value
