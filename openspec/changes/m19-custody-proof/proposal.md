## Why

M7 task 10.4 asked for a Lean proof that custody under custodian-set changes keeps one writer per epoch and never loses a committed entry. The attempt (2026-11-16) refuted the conjectures as first stated: `CustodyJointStale.lean` shows an election that counts promises made before the candidate knew its configuration record giving an epoch two writers. Rust fixes the record before it prepares, and a stale-record path in `CustodyHost::elect` was closed. The corrected statements, `OneWriterPerEpochR` and `CommittedSurvivesR` over runs whose elections fix their record first, are checked by search and by strict replay of every fault history, but not proven.

## What Changes

- Prove `OneWriterPerEpochR` and `CommittedSurvivesR` in `CustodyJoint.lean` for the joint-consensus model, with no `sorry` and no new axioms, and pin them.
- The expected route: an invariant that versions only move forward across epochs (the first switch into a version rises with the version and precedes every election using it), plus a within-epoch lemma that a commit at a rank is contained in the writer's log at every later switch.

Out of scope: changes to the custody protocol itself.

## Capabilities

### Modified Capabilities

- `object-custody`: adds a requirement that its joint-consensus safety is proven.

## Impact

- `lean/vertep-lean` only (`CustodyJoint.lean`, `VertepTests/Axioms.lean`); `tg.md/views/verification.md`.
- Follows `m7-durable-objects` task 10.4; not a new feature, so it does not wait on `m13-production-readiness`.
