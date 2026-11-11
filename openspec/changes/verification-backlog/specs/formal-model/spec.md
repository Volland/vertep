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
