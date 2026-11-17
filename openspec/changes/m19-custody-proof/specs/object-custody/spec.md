## ADDED Requirements

### Requirement: Joint Custody Safety Is Proven
The Lean model of custody under custodian-set changes SHALL carry machine-checked proofs, with no `sorry` and no axioms beyond the standard ones, that every election whose configuration record is fixed before its promises gives each epoch at most one writer and that no committed entry is lost.

#### Scenario: The theorems are pinned
- **WHEN** the Lean package is built and its axiom check runs
- **THEN** `OneWriterPerEpochR` and `CommittedSurvivesR` are proven theorems, pinned with only the standard axioms
