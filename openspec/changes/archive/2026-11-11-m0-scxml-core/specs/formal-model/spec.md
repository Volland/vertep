## Purpose

Models the chart IR and the SCXML semantics in Lean 4 on top of LeanRDF, proves the M0 theorems about them, runs as an executable oracle that the Rust core is checked against, and issues certificates for individual charts, so that Vertep's core behavior is proven rather than only tested.

## ADDED Requirements

### Requirement: Lean Package
The project SHALL provide the Lean 4 package `vertep-lean` with a pinned Lean toolchain, depending on LeanRDF for the RDF 1.2 data model and the SHACL Core validator, and building with `lake build` from a clean checkout.

#### Scenario: Clean build
- **WHEN** `lake build` runs in the package on a clean checkout with the pinned toolchain
- **THEN** the build succeeds and checks every theorem

#### Scenario: Pinned dependency
- **WHEN** the package manifest is inspected
- **THEN** the Lean toolchain and the LeanRDF revision are pinned exactly

### Requirement: Axiom Hygiene
The Lean package SHALL contain no `sorry` and no axioms beyond `propext`, `Quot.sound` and `Classical.choice`, and its test suite SHALL pin the axioms of every main theorem so that a new axiom fails the build.

#### Scenario: Sorry rejected
- **WHEN** a proof in the package is replaced by `sorry`
- **THEN** the build fails

#### Scenario: Axioms pinned
- **WHEN** the axiom check runs over the main theorems
- **THEN** each reports only the allowed axioms

### Requirement: IR Model In Lean
The Lean package SHALL define the chart IR, a `WellFormed` predicate equivalent to the chart model's well-formedness rules, and a decision procedure for `WellFormed` proven correct.

#### Scenario: Decision procedure correct
- **WHEN** the decision procedure for `WellFormed` is applied to a chart
- **THEN** a theorem states it returns true exactly when `WellFormed` holds

#### Scenario: Same verdict as Rust validation
- **WHEN** generated charts are validated by the Rust core and by the Lean decision procedure
- **THEN** the verdicts are equal for every chart

### Requirement: Executable Step Semantics
The Lean package SHALL define an executable `step` function that follows the W3C SCXML interpretation algorithm and has the same inputs and outputs as the Rust step function, with the datamodel abstracted as an oracle that answers condition, expression and assignment requests.

#### Scenario: Null datamodel evaluated natively
- **WHEN** a chart with the `null` datamodel is stepped in Lean
- **THEN** `In()` conditions are evaluated by the Lean model itself without an oracle

#### Scenario: Datamodel answers from a tape
- **WHEN** a chart with the `ecmascript` datamodel is stepped in Lean with a tape of datamodel answers recorded by the Rust core
- **THEN** the Lean step consumes the answers in order and fails if it requests a different operation than the one recorded

### Requirement: Step Determinism Theorem
The Lean package SHALL prove that `step` is a function of its inputs: for equal session states, equal inputs and equal datamodel answers, it returns equal session states and equal effect lists.

#### Scenario: Theorem stated over all inputs
- **WHEN** the determinism theorem is inspected
- **THEN** it quantifies over all well-formed charts, session states, inputs and datamodel answer tapes

#### Scenario: Effects included
- **WHEN** the theorem's conclusion is inspected
- **THEN** it covers the effect list as well as the new session state

### Requirement: Configuration Well-Formedness Theorem
The Lean package SHALL prove that from a legal configuration `step` reaches only legal configurations, where a configuration is legal when it contains the root, every active compound state has exactly one active child, every active parallel state has all of its children active, and every active state's parent is active.

#### Scenario: Compound exclusivity
- **WHEN** the theorem is applied to any step of a well-formed chart
- **THEN** no compound state has two active children afterwards

#### Scenario: Parallel completeness
- **WHEN** a parallel state is active after a step
- **THEN** all of its regions are active

### Requirement: Transition Selection Theorem
The Lean package SHALL prove that the transition set chosen in each microstep is enabled, is conflict-free, and is optimal in the sense of W3C SCXML 3.13: no omitted enabled transition has priority over a chosen one.

#### Scenario: Conflict-free set
- **WHEN** the theorem is applied to a selected transition set
- **THEN** the exit sets of any two chosen transitions do not intersect, unless one source is an ancestor of the other

#### Scenario: Priority respected
- **WHEN** an enabled transition was omitted
- **THEN** a chosen transition preempts it by descendant priority or by document order

### Requirement: Shapes Equivalence Theorem
The Lean package SHALL prove, using LeanRDF's proven SHACL Core validator, that validating the projection of a chart against the chart shapes conforms exactly when the chart is `WellFormed`.

#### Scenario: Forward direction
- **WHEN** a chart is `WellFormed`
- **THEN** the theorem gives that its projection conforms to the chart shapes

#### Scenario: Backward direction
- **WHEN** a chart's projection conforms to the chart shapes
- **THEN** the theorem gives that the chart is `WellFormed`

### Requirement: RDF Round Trip Theorem
The Lean package SHALL prove that reconstruction after projection is the identity on well-formed charts, and that the projection of equal charts has equal RDFC-1.0 canonical forms.

#### Scenario: Round trip
- **WHEN** the theorem is applied to a well-formed chart
- **THEN** reconstructing its projection yields the chart itself

#### Scenario: Canonical form determined by the chart
- **WHEN** two charts are equal
- **THEN** the theorem gives equal canonical N-Quads for their projections

### Requirement: Type Soundness Statement
The Lean package SHALL state type soundness for charts over dynamically typed values in M0, proving that `step` never produces a datamodel request for a location or data item the chart does not declare, and SHALL leave a named extension point for the full typed theorem delivered with the M1 type system.

#### Scenario: No undeclared locations
- **WHEN** the theorem is applied to a well-formed chart
- **THEN** every assignment request produced by `step` names a declared data item or a location expression of the chart

#### Scenario: Extension point present
- **WHEN** the package is inspected
- **THEN** it contains the named, documented statement that the M1 typed theorem will strengthen

### Requirement: Differential Oracle
The Lean package SHALL build an oracle executable that reads a chart, an input sequence and a datamodel answer tape in a documented exchange format and writes the resulting trace of configurations, effects and datamodel requests, so that Rust and Lean traces can be compared exactly.

#### Scenario: Oracle trace
- **WHEN** the oracle is run on a chart, three events and their tape
- **THEN** it writes one trace record per macrostep with the configuration, the effects and the datamodel requests

#### Scenario: Trace mismatch detected
- **WHEN** the Rust core and the oracle produce traces that differ in any record
- **THEN** the comparison reports the first differing record

### Requirement: Chart Certificates
The project SHALL provide a Lean-compiled checker, invoked as `vt certify`, that decides `WellFormed` for a chart given as RDF and, when it holds, emits a certificate as RDF stating `vt:certifiedBy` on the chart version IRI with the theorem names relied on, the Lean toolchain, the LeanRDF revision and the hash of the checker binary.

#### Scenario: Certificate for a well-formed chart
- **WHEN** `vt certify` is run on the RDF of a well-formed chart
- **THEN** it emits a certificate whose subject is the chart's version IRI and which lists the toolchain, the LeanRDF revision and the checker hash

#### Scenario: No certificate for an ill-formed chart
- **WHEN** `vt certify` is run on a graph that is not the projection of a well-formed chart
- **THEN** it emits no certificate and exits with an error naming the violated rule
