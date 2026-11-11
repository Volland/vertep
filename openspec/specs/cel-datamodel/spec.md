# cel-datamodel Specification

## Purpose
Provide a safe, deterministic CEL datamodel for guards, assignments and expressions in YAML charts and in LLM-written charts, with expressions that cannot cause side effects, always terminate and are type-checked against the chart before it runs.

## Requirements

### Requirement: CEL Datamodel Selection
A chart SHALL use the CEL datamodel when it declares `datamodel="cel"` in SCXML or `datamodel: cel` in YAML, and the CEL datamodel SHALL be the default for YAML charts.

#### Scenario: SCXML chart declares CEL
- **WHEN** an SCXML document declares `datamodel="cel"`
- **THEN** its `cond`, `expr` and `location` attributes are evaluated as CEL

#### Scenario: YAML default
- **WHEN** a YAML chart declares no datamodel
- **THEN** it uses the CEL datamodel

### Requirement: CEL Evaluation Scope
CEL expressions SHALL see the chart context as `ctx`, the current event as `ev` with fields `name`, `type`, `sendid`, `origin`, `origintype`, `invokeid` and `data`, the session as `session` with `id` and `name`, and a predicate `In(stateId)` that is true when the state is in the active configuration.

#### Scenario: Guard on event data
- **WHEN** guard `ev.data.amount > ctx.limit` is evaluated with event data amount 150 and limit 100
- **THEN** it evaluates to true

#### Scenario: In predicate
- **WHEN** guard `In("payment")` is evaluated while state `payment` is active
- **THEN** it evaluates to true

### Requirement: No Side Effects
CEL expressions SHALL NOT be able to mutate context, send events, read clocks, generate randomness or perform I/O; the only state change a CEL datamodel performs SHALL be the assignment of an expression's value to a location by `<assign>`.

#### Scenario: No clock function
- **WHEN** an expression calls `now()`
- **THEN** type checking fails because no such function exists in the environment

#### Scenario: Assign is the only mutation
- **WHEN** a chart runs a macrostep containing guards, `<log>` and one `<assign>`
- **THEN** the context after the macrostep differs from before only at the assigned location

### Requirement: Guaranteed Termination
Every CEL expression evaluation SHALL terminate, and the datamodel SHALL bound evaluation cost by a configurable limit on evaluation steps and on the size of produced values, raising `error.execution` when a limit is exceeded.

#### Scenario: Comprehension over a large list
- **WHEN** an expression maps over a list large enough to exceed the configured step limit
- **THEN** evaluation stops and `error.execution` is raised with a reason naming the exceeded limit

#### Scenario: Limits are deterministic
- **WHEN** the same expression is evaluated twice with the same inputs and limits
- **THEN** it either succeeds both times with the same value or exceeds the limit both times

### Requirement: Static Type Checking of CEL
Every CEL expression in a typed chart SHALL be type-checked against the chart's projected CEL type environment when the chart loads, a guard SHALL have type `bool`, and an assignment's expression SHALL have a type assignable to its location; a chart with CEL type errors SHALL NOT be instantiated.

#### Scenario: Non-boolean guard
- **WHEN** a guard expression is `ctx.total + 1`
- **THEN** loading fails with an error stating that a guard must have type `bool`

#### Scenario: Assignment type mismatch
- **WHEN** `<assign location="ctx.count" expr="'many'"/>` targets an `int` field
- **THEN** loading fails with an error naming `ctx.count`, `int` and `string`

#### Scenario: Untyped chart checks dynamically
- **WHEN** a chart without type declarations uses CEL
- **THEN** expressions are parsed when the chart loads and typed as dynamic, and type errors surface at runtime as `error.execution`

### Requirement: Runtime Errors Raise Error Execution
A CEL evaluation failure at runtime, such as division by zero, a missing map key, an overflow, a failed dynamic conversion or an exceeded limit, SHALL place `error.execution` on the internal queue with data naming the expression's location and the failure, and a failed guard SHALL be treated as false, as SCXML requires.

#### Scenario: Division by zero in a guard
- **WHEN** guard `ctx.total / ctx.count > 10` is evaluated with `count` equal to 0
- **THEN** the guard is treated as false and `error.execution` is raised with the guard's location

#### Scenario: Failed assignment leaves context unchanged
- **WHEN** an `<assign>` expression fails
- **THEN** the location keeps its previous value and `error.execution` is raised

### Requirement: Deterministic Evaluation
CEL evaluation SHALL be deterministic across platforms and runs: map iteration order SHALL be defined, decimal arithmetic SHALL be exact, and the same expression with the same inputs SHALL produce the same value or the same error on every target, including `no_std` and WebAssembly.

#### Scenario: Same result on two targets
- **WHEN** a corpus of expressions is evaluated natively and in WebAssembly
- **THEN** every result and every error is identical

#### Scenario: Map iteration order
- **WHEN** an expression iterates over the keys of a map
- **THEN** the keys are visited in a defined order independent of insertion history

### Requirement: Safe Datamodel for Generated Charts
The CEL datamodel SHALL be usable as the only datamodel permitted for charts produced by untrusted authors such as LLMs, and loading a chart with a policy that restricts it to CEL SHALL reject any chart that declares another datamodel or contains `<script>` or references to registered Rust functions.

#### Scenario: Restricted chart with a script
- **WHEN** a chart containing `<script>` is loaded under a CEL-only policy
- **THEN** loading fails with an error naming the `<script>` element

#### Scenario: Restricted chart with a Rust function reference
- **WHEN** a chart referencing guard IRI `vt:guard/isPaid` is loaded under a CEL-only policy
- **THEN** loading fails with an error naming the IRI

#### Scenario: Restricted chart in pure CEL
- **WHEN** a chart that uses only CEL expressions is loaded under a CEL-only policy
- **THEN** it loads and runs

### Requirement: SCXML Datamodel Contract
The CEL datamodel SHALL satisfy the datamodel contract that SCXML defines for `<data>`, `<assign>`, `<donedata>`, `<content>`, `<param>`, `<foreach>`, `<if>` conditions, `<log>` and `_event`, with the semantics of each documented in terms of CEL values.

#### Scenario: Foreach over a list
- **WHEN** `<foreach array="ctx.items" item="it" index="i">` runs over a three-element list
- **THEN** its body runs three times with `it` and `i` bound in order

#### Scenario: Donedata
- **WHEN** a final state with `<donedata>` containing a `<param name="result" expr="ctx.total"/>` is entered
- **THEN** the `done.state` event carries `result` with the value of `ctx.total`
