## ADDED Requirements

### Requirement: Joint Custody Safety Is Proven
The Lean model of custody under custodian-set changes SHALL carry machine-checked proofs that, for every run whose elections fix their configuration record before counting promises and with majority quorums, each epoch has at most one writer and no committed entry is lost. The proofs MUST contain no `sorry` and use no axioms beyond `propext`, `Classical.choice` and `Quot.sound`.

#### Scenario: One writer per epoch is proven
- **WHEN** the Lean package is built and its axiom check runs
- **THEN** `OneWriterPerEpochR` is a proven theorem pinned with only the standard axioms

#### Scenario: Committed entries survive is proven
- **WHEN** the Lean package is built and its axiom check runs
- **THEN** `CommittedSurvivesR` is a proven theorem pinned with only the standard axioms

#### Scenario: The refuted statements stay refuted
- **WHEN** the build-time search runs elections that count promises made before their record was known
- **THEN** it still finds the violation of the conjectures as first stated, and the counterexample in `CustodyJointStale.lean` still checks

#### Scenario: Rust histories still replay
- **WHEN** the custody fault-injection tests record their histories
- **THEN** every history replays strictly through the custody oracle, which checks `RecordFixed` and both properties on every step
