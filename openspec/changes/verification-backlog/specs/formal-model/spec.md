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
