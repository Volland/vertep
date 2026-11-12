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
