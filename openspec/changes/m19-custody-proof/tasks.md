## 1. Graph first

- [ ] 1.1 Confirm milestone `M19 Custody proof` in `tg.md/roadmap.md` with `precedes::` from M7 (added in release 0.1.13)
- [ ] 1.2 Record C1–C3 as `Decision` sections in `tg.md/decisions.md`, `motivated_by::` the joint-consensus decision

## 2. Invariant

- [ ] 2.1 Define `firstSwitch` (C2 part 1; decide ghost field or computed, and record the choice) and prove it rises with the version along `ReachableR`
- [ ] 2.2 Prove that `RecordFixed` puts the first switch into a version before every election that uses it
- [ ] 2.3 Prove that every election after `firstSwitch v` uses a rank of at least `2v − 1`
- [ ] 2.4 Prove the within-epoch lemma (C2 part 3): a commit at rank `ρ₀` is in the writer's log at every later switch from a rank of at least `ρ₀ + 2`
- [ ] 2.5 Prove rank-aware safety (C2 part 2): every election above the commit epoch adopts an extension of the committed log
- [ ] 2.6 Assemble `Inv` and prove `ReachableR conf g s → Inv conf g s`, one preservation lemma per `Action`

## 3. Theorems

- [ ] 3.1 Prove `OneWriterPerEpochR` from `Inv` and quorum intersection
- [ ] 3.2 Prove `CommittedSurvivesR` from `Inv`
- [ ] 3.3 Pin every new theorem and lemma in `VertepTests/Axioms.lean`; `scripts/check.sh` passes with no `sorry` and only the standard axioms

## 4. Regression net and docs

- [ ] 4.1 Keep the build-time searches and the strict replay of Rust fault histories; update their comments to say the statements are proven
- [ ] 4.2 Update `tg.md/views/verification.md` (theorem inventory) and `tg.md/views/durable-objects.md`; flip the docs-audit claims that call the proof open
- [ ] 4.3 Annotate the theorems with `@tg: verifies::` for the scenarios of `Joint Custody Safety Is Proven`; `tg trace object-custody --gaps` shows no gap
