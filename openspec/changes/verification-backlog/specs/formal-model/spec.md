## MODIFIED Requirements

### Requirement: Differential Oracle
The Lean package SHALL build an oracle executable that reads a chart, an input sequence and a datamodel answer tape in a documented exchange format and writes the resulting trace of configurations, effects and datamodel requests, so that Rust and Lean traces can be compared exactly. The oracle SHALL terminate on every input, charging the microstep budget exactly as the Rust core does and writing the partial trace with the budget error.

#### Scenario: Oracle trace
- **WHEN** the oracle is run on a chart, three events and their tape
- **THEN** it writes one trace record per macrostep with the configuration, the effects and the datamodel requests

#### Scenario: Trace mismatch detected
- **WHEN** the Rust core and the oracle produce traces that differ in any record
- **THEN** the comparison reports the first differing record

#### Scenario: Budget exhaustion agrees with Rust
- **WHEN** a chart exhausts the microstep budget in the Rust core
- **THEN** the oracle stops at the same microstep and writes the same partial trace and budget error

### Requirement: Chart Certificates
The project SHALL provide a Lean-compiled checker, invoked as `vt certify`, that decides `WellFormed` for a chart given as RDF and, when it holds, emits a certificate as RDF stating `vt:certifiedBy` on the chart version IRI with the theorem names relied on, the Lean toolchain, the LeanRDF revision and the hash of the checker binary. For a typed chart the checker SHALL read the chart's `vt:types` block, derive its type triples (`vt:declaresType`, the definitions and the SHACL shapes under content-addressed type IRIs) as the Rust projection does, require them in the graph and include them in the chart version.

#### Scenario: Certificate for a well-formed chart
- **WHEN** `vt certify` is run on the RDF of a well-formed chart
- **THEN** it emits a certificate whose subject is the chart's version IRI and which lists the toolchain, the LeanRDF revision and the checker hash

#### Scenario: No certificate for an ill-formed chart
- **WHEN** `vt certify` is run on a graph that is not the projection of a well-formed chart
- **THEN** it emits no certificate and exits with an error naming the violated rule

#### Scenario: Typed chart certified
- **WHEN** `vt certify` is run on the RDF of a well-formed chart with a `vt:types` block
- **THEN** it emits a certificate whose subject is the version IRI Rust computes for the chart, types included, and refuses a graph that lacks one of the chart's type triples

### Requirement: Shapes Equivalence Theorem
The Lean package SHALL prove, using LeanRDF's proven SHACL Core validator, that validating the projection of a chart against the chart shapes conforms exactly when the chart is `WellFormed`. The chart shapes in the theorem SHALL be a Lean term (`Rdf.ShapesL.chartShapesL`), because the Turtle parser and the shapes reader are `partial` and say nothing in the logic; a test SHALL check that reading `vt-chart-shapes.ttl` gives that term up to identifiers, messages, severities and order. The part of the theorem over the node shapes (every shape with a class target, the initial-reference scope shape, the history shape without its cycle component) SHALL be proven first, for charts with unique state identifiers; the graph-wide shapes (selection, history-default cycles, unique data identifiers) and the step from conformance to unique state identifiers stay open in `Pending/ShapesEquiv.lean` until proven.

#### Scenario: Forward direction
- **WHEN** a chart is `WellFormed`
- **THEN** the theorem gives that its projection conforms to the chart shapes

#### Scenario: Backward direction
- **WHEN** a chart's projection conforms to the chart shapes
- **THEN** the theorem gives that the chart is `WellFormed`

#### Scenario: Shapes term
- **WHEN** `vt-chart-shapes.ttl` is read with LeanRDF's Turtle parser and shapes reader
- **THEN** it gives the shapes of `chartShapesL`, up to identifiers, messages, severities and order

#### Scenario: Node shapes
- **WHEN** a chart has unique state identifiers
- **THEN** the theorem gives that every node shape holds on its projection exactly when the chart satisfies every `WellFormed` rule except unique identifiers, legal target sets, unique data identifiers and acyclic history defaults

## ADDED Requirements

### Requirement: Type Checker Correctness
The Lean package SHALL define the type language, the typing relation `HasType` as the least relation closed under the typing rules of the type-system specification, and an executable checker `check` that terminates on every declaration block, and SHALL prove `check T v = true ↔ HasType v T`. The checker SHALL stop on non-contractive recursion as the Rust checker does: a named type expanded again at the same value position is a mismatch.

#### Scenario: Checker decides the typing relation
- **WHEN** `check` is applied to any declaration block, type and value
- **THEN** it returns `true` exactly when `HasType` holds, by a theorem whose axioms are pinned

#### Scenario: Recursion through optional only terminates
- **WHEN** `type A = optional<A>` is checked against `null` and against a string
- **THEN** `check` accepts `null` and rejects the string, as the Rust checker does

### Requirement: JSON Schema Projection Agreement
The Lean package SHALL define the JSON Schema projection of types for the keyword subset the Rust projection emits and a validator for that subset, and SHALL prove that a JSON value is valid against the projection of a type exactly when it has the type, for declaration blocks whose unions have distinct tags.

#### Scenario: Schema agrees with the checker
- **WHEN** a JSON value is validated against the projected schema of a type with the block's `$defs`
- **THEN** it is valid exactly when `HasType` holds, by a theorem whose axioms are pinned

### Requirement: SHACL Projection Agreement
The Lean package SHALL define the RDF encoding of values and the SHACL Core projection of types as LeanRDF shapes, mirroring `vertep-rdf`, and SHALL prove with LeanRDF's SHACL Core semantics, which its validator is proven to decide, that the encoding of a value conforms to the projection of a type exactly when the value has the type, for declaration blocks whose unions have distinct tags and values whose objects have distinct keys. Recursion through named types SHALL be unfolded, since SHACL leaves recursive shapes undefined. The agreement of LeanRDF's XPath regular expressions with the lexical rules of `decimal`, `datetime` and `iri` MAY be an explicit, tested hypothesis.

#### Scenario: Encoding conforms exactly when typed
- **WHEN** the encoding of a value is validated against the unfolded shapes of a type
- **THEN** it conforms to some unfolding exactly when `HasType` holds, by a theorem whose axioms are pinned

#### Scenario: Unfolding is monotone
- **WHEN** the encoding of a value conforms to the shapes of a type unfolded `k` times
- **THEN** it conforms to the shapes unfolded `k + 1` times

### Requirement: Chart Type Soundness
The Lean package SHALL prove type soundness of `start` and `step` relative to the static typing a chart passed the chart type checker with: the declared types of locations and events and the static types the checker gives expressions, inline contents and `foreach` items. Given a typed answer tape, a datamodel kept typed by typed requests, a session whose queued events are typed and a typed input event, every successful `start` and `step`, and the partial result of one that exhausts the microstep budget, SHALL write to a typed location only values of its type, SHALL keep the datamodel typed, and SHALL send, forward and queue only events whose payloads have their declared types. A typed answer tape is the datamodel's obligation, stated as an assumption: an answer to a request for an expression, an inline content or the items of a `foreach` array whose static type is `T` is a value of type `T`. The values the core generates itself, `undefined` for an `<assign>` without value, `idlocation` identifiers, `foreach` items and indices, the platform's `error.*` payloads and `done.state.*` data, SHALL be checked by the chart type checker. Expressions and contents the datamodel's typer cannot type are outside the statement: their values reach typed locations through the datamodel's checked conversion.

#### Scenario: Typed step keeps the datamodel typed
- **WHEN** the theorem is applied to `step` on a checked chart with a typed tape, a typed datamodel, a typed session and a typed input event
- **THEN** every `assignValue` and `bindVar` to a typed location carries a value of that location's type and the datamodel after the step's requests is typed

#### Scenario: Sent and queued events carry their declared payloads
- **WHEN** the same theorem is applied
- **THEN** every `send` and `forward` effect and every event left in the internal queue whose name has a declared payload type carries a payload of that type

#### Scenario: Generated values are checked
- **WHEN** a chart writes `undefined` with a bare `<assign>` to a location of a required type, stores an `idlocation` in a location that does not accept strings, or binds `foreach` items or indices to locations of other types
- **THEN** the chart type checker rejects it, and the Lean step writes the value that breaks the location's type
