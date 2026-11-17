## 1. Proof

- [ ] 1.1 State the cross-epoch version-monotonicity invariant and prove it is preserved by every step of `ReachableR`
- [ ] 1.2 Prove the within-epoch lemma: a commit at rank r is in the writer's log at every later switch from a rank at least r + 2
- [ ] 1.3 Prove `OneWriterPerEpochR` and `CommittedSurvivesR`; pin them in `VertepTests/Axioms.lean`
- [ ] 1.4 Keep the build-time search and the strict replay; update their comments

## 2. Graph

- [ ] 2.1 Update `tg.md/views/verification.md` and `tg.md/views/durable-objects.md`; annotate the theorems with `@tg: verifies::` for the new scenario
