## ADDED Requirements

### Requirement: Portable Profile
The system SHALL define a portable profile, a subset of the rules profile whose clauses conclude or report violations from conjunctions of `tm.triple` reads, comparisons and arithmetic on ground numbers and strings, and SHALL decide membership by parsing the program. A program outside it MUST be reported with the first clause and the construct that puts it outside.

#### Scenario: Credit policy is portable
- **WHEN** the credit-decision policy of `examples/credit-decision` is checked
- **THEN** it is inside the portable profile

#### Scenario: findall is not portable
- **WHEN** a program uses `findall` in a clause body
- **THEN** the check reports that clause and `findall` as outside the portable profile

### Requirement: Portable Engine
The system SHALL provide a rules engine written in Rust that runs every program inside the portable profile on every supported target, including `wasm32-unknown-unknown`, Windows and the Python wheels, and that links no native library.

#### Scenario: Decision in the browser
- **WHEN** a chart with a rules decider and a portable program is loaded in the WebAssembly build
- **THEN** it loads, and a decision request returns a decision with a derivation

#### Scenario: Windows build
- **WHEN** `vertep-rules` is built for Windows with default features
- **THEN** it builds, links no native engine library, and the decision vectors pass on the portable engine

### Requirement: Engine Selection
The system SHALL select the engine when a program is loaded: the native engine when it is built in, unless the policy asks for the portable engine; else the portable engine for a portable program. Without the native engine, a program outside the portable profile MUST fail with `rules-unavailable` naming its first non-portable clause, and the system MUST NOT fall back to a decider of another kind.

#### Scenario: Non-portable program without the native engine
- **WHEN** a program using `findall` is loaded on a build without the native engine
- **THEN** loading fails with `rules-unavailable` naming the clause, and no other decider is consulted

#### Scenario: An edit leaves the portable profile
- **WHEN** a superseding edit makes a live program non-portable on a build without the native engine
- **THEN** the edit is refused and the live program keeps running

### Requirement: Engines Agree On Portable Programs
For every program inside the portable profile, the native and the portable engine SHALL give the same conclusions with the same stated support on the same statements, and the same refusal reason when no conclusion is reached. A disagreement MUST fail the conformance suite.

#### Scenario: Decision vectors agree
- **WHEN** the rule-decider conformance vectors run on both engines
- **THEN** every decision, support list and refusal reason is equal

#### Scenario: Random graphs agree
- **WHEN** a portable program runs on a pseudo-random graph in both engines
- **THEN** the conclusions and their support are equal

#### Scenario: Numeric boundary
- **WHEN** a rule compares `D =< 43` and the statement holds `43.0`
- **THEN** both engines conclude the same

### Requirement: Stated Support Checked In Every Engine
Each engine SHALL keep the set of statements a run read and SHALL refuse a conclusion whose stated support names a statement outside it, with refusal reason `forged-support`.

#### Scenario: Forged support on the portable engine
- **WHEN** a portable program states as support a statement its run never read
- **THEN** the portable engine refuses with `forged-support`

### Requirement: Engine Is Recorded
The derivation of a rule decision SHALL record the engine family and version (`portable/<version>` or `lean-elpi/<version>`), and journal replay MUST use the recorded family, failing with `engine-unavailable` when it is not built in. Fuel used MUST be compared only between runs of the same engine.

#### Scenario: Replay on a build without the recorded engine
- **WHEN** a journal decided by `lean-elpi` is replayed on a portable-only build
- **THEN** replay fails with `engine-unavailable` and changes nothing

#### Scenario: Older derivations
- **WHEN** a derivation recorded before this change is read
- **THEN** its engine is read as `lean-elpi/<recorded version>`

### Requirement: Rules In The Bindings
The browser package and the Python package SHALL expose the rules decider, bundle verification and rule-version counterfactuals on the portable engine, with the same records as the native build.

#### Scenario: Verify a server bundle in the browser
- **WHEN** a bundle made by the native engine for a portable program is passed to `verifyBundle` in the browser
- **THEN** it verifies and the report says `cross-engine`

#### Scenario: Counterfactual in Python
- **WHEN** a Python user replays recorded rule decisions under a new portable program version
- **THEN** the report names each decision that would differ and the journal is unchanged
