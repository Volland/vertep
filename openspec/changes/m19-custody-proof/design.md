## Context

`lean/vertep-lean/VertepLean/CustodyJoint.lean` models durable-object custody with custodian-set changes by joint consensus (M7 design D7). A system state `JSys` holds the custodians' promises and accepted logs, the elections, and the configuration records in force. The moves (`Action`) are `prepare`, `elect`, `write`, `propose` (start a change), `append` (ship entries to a custodian) and `switch` (make the new set effective). A record has a rank: rank `2v` is version `v` alone, and rank `2v+1` is the joint record of `v` and `v+1`, where every quorum needs a majority of both sets (`RQuorum`, with the intersection lemmas `rquorum_inter` and `rquorum_inter_succ` already proven).

What is proven today:
- For the fixed set (`Custody.lean`, `CustodyTheorems.lean`): one writer per epoch, and committed entries survive.
- For runs that never start a change (`ReachableNC`): `one_writer_per_epoch_noChange` and `committed_survives_noChange`, by projecting each move onto the fixed-set model (`exec_noChange_project`).

What M7 task 10.4 found (2026-11-16):
- `CustodyJointStale.lean` refutes `OneWriterPerEpoch` and `CommittedSurvives` as first stated. A candidate counted promises made before it knew the record it elected under, so epoch 3 got two writers, one of them adopting an empty log.
- Rust never does this: `CustodyHost::elect` fixes its record before preparing, and re-prepares when a promise reports a newer one. The one stale path, after the fourth round, was closed in pull request 48.
- The model now records what each promise saw (`JPromise.seenP`, `seenF`) and states the restriction as `RecordFixed`: every promise an election counts already knew the election's record (`knownWhen`).
- The corrected statements range over `ReachableR`, the runs whose elections satisfy `RecordFixed`, and assume majority quorums (`Majorities`):
  - `OneWriterPerEpochR`: two elections in the same epoch have the same host.
  - `CommittedSurvivesR`: a log committed at some epoch is a prefix of every log a later election adopts.
- Both are checked by a build-time random search (no violation in 300 runs; an offline 3 × 3000 runs of 150 steps also found none), by the strict replay of 150 Rust fault histories (145 with set changes), and by the oracle `CustodyJointCheck`. Neither is proven.

## Goals / Non-Goals

**Goals:**
- Machine-checked proofs of `OneWriterPerEpochR` and `CommittedSurvivesR`, with no `sorry` and only the standard axioms (`propext`, `Classical.choice`, `Quot.sound`), pinned in `VertepTests/Axioms.lean`.

**Non-Goals:**
- Changing the custody protocol or the Rust implementation. If the proof finds another counterexample, it is reported and fixed in a separate change, as the stale-record one was.
- Proving the Rust code refines the model. Strict replay of recorded histories stays the bridge.
- Liveness.

## Decisions

### C1. Prove an inductive invariant over `ReachableR`, not a projection

The no-change proofs project onto the fixed-set model. That cannot work once a change starts, because `elect`, `propose` and `switch` add records in force and the fixed-set invariant ("the logs that can still be chosen only shrink") no longer holds. A `switch` at epoch `e₀` creates logs that the new set alone can choose, and an election under a distant rank does the same. Instead, define a single invariant `Inv conf g s` and prove `ReachableR conf g s → Inv conf g s` by induction on the run, with one preservation lemma per `Action`. Both theorems then follow from `Inv` together with the quorum-intersection lemmas.

Rejected: a refinement proof that maps each joint run onto a sequence of fixed-set runs, one per version. The joint ranks make the boundaries between versions overlap, so the mapping would need an invariant at least as strong as `Inv`.

### C2. The invariant has three parts

1. **Versions move forward across epochs.** For each version `v`, let `firstSwitch v` be the epoch of the first `switch` into `v`. Then `firstSwitch` rises with `v`. `RecordFixed` puts the first switch into `v` strictly before any election that uses `v`. And every election at an epoch after `firstSwitch v` uses a rank of at least `2v − 1`. This is the piece the stale-record counterexample breaks.
2. **Rank-aware Paxos safety.** If a log `L` is committed at epoch `e₀` under rank `ρ₀`, then every election at an epoch above `e₀` adopts an extension of `L`. The proof goes through `rquorum_inter` (same or adjacent ranks intersect) and part 1 (no election at a later epoch uses a rank that skips the joint rank between them).
3. **Within one epoch.** A commit at rank `ρ₀` is contained in the writer's log at every later `switch` from a rank of at least `ρ₀ + 2`. The model lets a delayed batch carry an old rank with new entries, so the lemma must be stated about the writer's log, not about the batch.

`OneWriterPerEpochR` follows from part 1 and the intersection of any two quorums that elect in the same epoch. `CommittedSurvivesR` follows from parts 2 and 3.

### C3. Keep the executable checks as the regression net

The build-time search (`VertepTests/CustodyJoint.lean`), the stale-run search that must still find the refuted conjectures' violation, and the strict replay of Rust fault histories all stay. A proof of the model does not reach the Rust code; only replay does.

## Risks / Trade-offs

- **Size.** The M7 attempt estimated several days of proof work. Mitigation: land the invariant parts as separate pinned lemmas (tasks 1.1–1.4), each useful on its own, so partial progress is never lost.
- **The invariant could be false.** Then the proof attempt yields a new counterexample, which is the same kind of finding as M7 10.4: report it, add a Lean witness, and fix the protocol in its own change.
- **Proof brittleness across Lean versions.** The package pins Lean 4.35.0. Prefer `omega`, `simp only` with explicit lemmas, and small lemmas over long tactic scripts.

## Open Questions

- Whether `firstSwitch` is best a function of the state (computed from `inForce`) or a ghost field like `seenP`. A ghost field is simpler to reason about and was the choice for promises.
