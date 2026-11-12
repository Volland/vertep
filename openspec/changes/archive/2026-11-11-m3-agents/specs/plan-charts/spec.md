## Purpose

Plan charts let an LLM write a statechart fragment, such as a plan of steps, and run it safely. The fragment is validated, restricted to the CEL datamodel, content-addressed, executed as a sandboxed invoked child with bounded resources and granted capabilities, and recorded in the graph with its lineage.

## ADDED Requirements

### Requirement: Plan Submission
The system SHALL accept a plan as a chart fragment produced by an LLM actor or a decider, in YAML or RDF form, together with the parent actor IRI and the invoking state. A submitted plan SHALL be parsed into the IR. A plan that fails parsing SHALL be rejected with `error.plan`, the class `parse` and the parse diagnostics.

#### Scenario: Plan in YAML accepted for validation
- **WHEN** an LLM actor returns a YAML chart fragment as the payload of `vt.plan.proposed`
- **THEN** the fragment is parsed into the IR and passed to plan validation

#### Scenario: Unparseable plan rejected
- **WHEN** the returned fragment is not valid YAML
- **THEN** the parent receives `error.plan` with the class `parse` and the diagnostics, and nothing is executed

### Requirement: Plan Validation
The system SHALL validate every plan with the same IR well-formedness checks and SHACL shapes as any chart, plus the plan restrictions, before anything is executed. Any violation SHALL reject the plan with `error.plan`, the class `invalid` and all violations.

#### Scenario: Plan with dangling target rejected
- **WHEN** a plan contains a transition that targets a state id that doesn't exist
- **THEN** the plan is rejected with `error.plan`, listing the dangling target

#### Scenario: Valid plan accepted
- **WHEN** a plan passes the IR checks, the SHACL shapes and the plan restrictions
- **THEN** it is accepted, content-addressed and becomes eligible for execution

### Requirement: CEL-only Plans
A plan SHALL declare the CEL datamodel or the null datamodel, and every guard, assignment and expression in it SHALL be CEL. Plans SHALL NOT contain ECMAScript, Rust-bound guards or actions, scripts, or references to datamodel functions outside the CEL standard library and the explicitly granted functions.

#### Scenario: ECMAScript plan rejected
- **WHEN** a plan declares `datamodel="ecmascript"` or contains a `<script>` element
- **THEN** the plan is rejected with `error.plan`, the class `invalid` and the reason `datamodel`

#### Scenario: CEL expression type-checked
- **WHEN** a plan's guard is a CEL expression that compares a string field with an integer
- **THEN** the plan is rejected with a CEL type error before execution

#### Scenario: Rust-bound action rejected
- **WHEN** a plan references a named Rust action IRI that is not in the granted function set
- **THEN** the plan is rejected with the reason `ungranted-function`

### Requirement: Content-addressed Plans
Every accepted plan SHALL receive a chart version IRI computed from the hash of its canonical RDF, by the same procedure as any chart. Submitting an identical plan twice SHALL yield the same IRI.

#### Scenario: Identical plans share an IRI
- **WHEN** two plans that differ only in whitespace and key order are submitted
- **THEN** both receive the same chart version IRI

#### Scenario: Different plans get different IRIs
- **WHEN** two plans differ in one transition target
- **THEN** they receive different chart version IRIs

### Requirement: Sandboxed Plan Execution
An accepted plan SHALL execute only as an invoked child of the state that requested it. Its effective capability grants SHALL be the intersection of the invoking state's grants for plans and the plan's own declared needs, and it SHALL be cancelled when that state exits. A plan SHALL NOT be activated as a virtual actor and SHALL NOT change its parent's chart, context or journal except by sending events to its parent.

#### Scenario: Plan runs as invoked child
- **WHEN** the parent state invokes an accepted plan
- **THEN** the plan runs as an invoked actor with the address of the parent IRI plus the invoke id, and the parent's configuration is unchanged by the start

#### Scenario: Plan cancelled with its state
- **WHEN** the parent leaves the invoking state while the plan is running
- **THEN** the plan is cancelled, and its remaining steps do not execute

#### Scenario: Plan cannot exceed grants
- **WHEN** a plan sends to a target that the invoking state did not grant for plans
- **THEN** `error.communication` is raised in the plan with the reason `not-granted`, and the message is not delivered

### Requirement: Plan Execution Bounds
Every plan execution SHALL be bounded by a maximum number of macrosteps, a maximum wall-clock duration enforced by a durable timer, and a maximum number of outgoing sends, as declared by the invoking state. Exceeding any bound SHALL stop the plan and deliver `error.plan` with the class `bounds` and the bound that was exceeded to the parent.

#### Scenario: Step bound exceeded
- **WHEN** a plan with a bound of 50 macrosteps loops without reaching a final state
- **THEN** the plan is stopped after its 50th macrostep, and the parent receives `error.plan` with the bound `macrosteps`

#### Scenario: Duration bound survives deactivation
- **WHEN** the parent is a virtual actor that is deactivated while a plan runs, and the duration bound passes
- **THEN** the bound fires on reactivation, and the parent receives `error.plan` with the bound `duration`

### Requirement: Plan Results Returned to Parent
When a plan reaches a top-level final state, the parent SHALL receive `done.invoke.<id>` carrying the plan's `<donedata>`. When the plan's chart is typed, the done data SHALL be validated against the plan's declared result type. When validation fails, the parent SHALL receive `error.plan` with the class `result` instead.

#### Scenario: Plan completes with result
- **WHEN** a plan reaches its final state with donedata `{ "booked": true }`
- **THEN** the parent receives `done.invoke.<id>` with that data, journaled in the parent's journal

#### Scenario: Result violates declared type
- **WHEN** a plan declares the result type `{ booked: bool }` and finishes with `{ "booked": "yes" }`
- **THEN** the parent receives `error.plan` with the class `result`

### Requirement: Plan Lineage
The system SHALL record the lineage of every plan in the graph: the proposing LLM call or decision, the plan version IRI, the validation outcome, the parent actor and state, each execution, and its outcome. The record SHALL use `vt-agent:Plan` and PROV-O-aligned properties such as `prov:wasGeneratedBy` and `prov:used`.

#### Scenario: Lineage query
- **WHEN** a SPARQL query asks which LLM call generated the plan that produced a given parent's result
- **THEN** the query returns the `vt-agent:LlmCall`, with its model identifier and prompt hash

#### Scenario: Rejected plans recorded
- **WHEN** a plan is rejected by validation
- **THEN** the graph records the rejected plan's content hash, the generating call and the violations, so failed proposals can be audited

### Requirement: Self-modification Through Versioning
An agent SHALL be able to change its own chart only by proposing a new chart version together with a migration from its current version. The proposal SHALL go through validation, content addressing and the chart-versioning migration procedure, and SHALL be recorded as a journal entry. No other path SHALL change a running actor's chart.

#### Scenario: Accepted self-modification
- **WHEN** an agent proposes a new version of its own chart with a migration that maps every reachable configuration, and both pass validation
- **THEN** the migration is recorded as a journal entry, and the actor continues under the new version IRI from the next macrostep

#### Scenario: Migration with unmapped configuration rejected
- **WHEN** the proposed migration leaves a reachable configuration of the current version without a mapping
- **THEN** the proposal is rejected, the actor stays on its current version, and the rejection is journaled

#### Scenario: No in-place mutation
- **WHEN** a decider or LLM result tries to alter the running chart's structure in any other way
- **THEN** no structural change takes effect, because only migration journal entries can change an actor's chart version
