import VertepLean.CustodyJoint

/-!
# The joint-consensus conjectures are false as stated (m7 10.4)

`OneWriterPerEpoch` and `CommittedSurvives` (`VertepLean.CustodyJoint`) do not hold in the
joint-consensus model, even when every configuration's quorum is a majority. The model lets a
candidate be elected under any record it knows of **at the election**, with promises it gathered
**before** it knew of that record. A promise made to epoch `e` before a later epoch moved the
custodian set on then counts towards a quorum of the new set in `e`:

* `h1` wins epoch 1 from `{a,b}` and commits `[x]`.
* `d` and `f`, still fresh, promise epoch 3 to `h3`; `a` and `b` promise epoch 3 to `h2`.
* `h5` wins epoch 5 from `{a,b}`, moves to the joint record, ships `[x]` to `{a,b}` and `{d,f}`
  and switches to version 1 (`{c,d,f}`).
* `h3` is elected in epoch 3 under version 1 by the stale promises of `{d,f}`, which report
  empty logs: it adopts `[]` (`stale_record_loses_committed`), and `h2` is elected in epoch 3
  under version 0 by `{a,b}` (`stale_record_two_writers`).

Rust does not do this: `CustodyHost::elect` fixes the record it prepares with before it sends
`custody.prepare`, takes promises from that round only, and a newer record reported by a
promise starts a new round at a higher epoch. In the model, a promise remembers how much of
the system had happened when it was made (`JPromise.seenP`, `JPromise.seenF`), and
`RecordFixed` asks an election's record to be known to each of its promises; the corrected
conjectures (`OneWriterPerEpochR`, `CommittedSurvivesR` in `VertepLean.CustodyJoint`) range
over the runs that keep it (`ReachableR`). This run breaks it: `h3`'s record was not known when
`d` and `f` promised (`stale_run_not_recordFixed`).
-/

namespace Vertep.Custody.Joint.Stale

open Vertep.Custody Vertep.Custody.Joint

/-- Run moves in order; `none` when one is not enabled. -/
def run (conf : Confs) (g : Nat) (as : List Action) (s : JSys) : Option JSys :=
  as.foldlM (exec conf g) s

theorem run_reachable (conf : Confs) (g : Nat) :
    ∀ (as : List Action) {s t : JSys}, Reachable conf g s → run conf g as s = some t →
      Reachable conf g t
  | [], s, t, hs, h => by
    simp only [run, List.foldlM, Option.pure_def, Option.some.injEq] at h; exact h ▸ hs
  | a :: as, s, t, hs, h => by
    simp only [run, List.foldlM] at h
    cases hx : exec conf g s a with
    | none => rw [hx] at h; cases h
    | some s' =>
      rw [hx] at h
      exact run_reachable conf g as (Reachable.step a hs hx) h

/-- Version 0: `{a,b,c}`; every later version: `{c,d,f}`; quorum 2. -/
def stale : Confs := fun v =>
  if v = 0 then ⟨["a", "b", "c"], 2⟩ else ⟨["c", "d", "f"], 2⟩

theorem stale_majorities : Majorities stale := by
  intro v; unfold stale Config.Majority; split <;> decide

/-- The run of the module documentation. -/
def staleRun : List Action :=
  [.prepare "a" 1 "h1", .prepare "b" 1 "h1",
   .elect "h1" 1 0 ["a", "b"] [],
   .write "h1" 1 ["x"],
   .append "a" "h1" 1 0 0 ["x"], .append "b" "h1" 1 0 0 ["x"],
   .prepare "d" 3 "h3", .prepare "f" 3 "h3",
   .prepare "a" 3 "h2", .prepare "b" 3 "h2",
   .prepare "a" 5 "h5", .prepare "b" 5 "h5",
   .elect "h5" 5 0 ["a", "b"] ["x"],
   .propose "h5" 5,
   .append "a" "h5" 5 1 1 ["x"], .append "b" "h5" 5 1 1 ["x"],
   .append "d" "h5" 5 0 1 ["x"], .append "f" "h5" 5 0 1 ["x"],
   .switch "h5" 5,
   .elect "h3" 3 2 ["d", "f"] [],
   .elect "h2" 3 0 ["a", "b"] ["x"]]

theorem staleRun_some : (run stale 0 staleRun (JSys.empty 0)).isSome = true := by decide

def staleState : JSys := (run stale 0 staleRun (JSys.empty 0)).get staleRun_some

theorem stale_reachable : Reachable stale 0 staleState :=
  run_reachable stale 0 staleRun Reachable.empty (Option.some_get _).symm

theorem stale_committed : Committed stale staleState ["x"] 1 := by
  refine ⟨0, by decide, ["a", "b"], by decide, fun c hc => ?_⟩
  simp only [List.mem_cons, List.not_mem_nil, or_false] at hc
  rcases hc with rfl | rfl <;> exact ⟨["x"], by decide, List.prefix_refl _⟩

/-- **Committed survival fails in the model**: `[x]`, committed in epoch 1, is missing from the
log adopted in epoch 3, with every quorum a majority. -/
theorem stale_record_loses_committed : ¬ CommittedSurvives stale 0 := by
  intro h
  have := (h staleState stale_reachable ["x"] 1 stale_committed).1 3 "h3" 2 []
    (by decide) (by decide)
  exact absurd this (by decide)

/-- **One writer per epoch fails in the model**: `h3` and `h2` are both elected in epoch 3. -/
theorem stale_record_two_writers : ¬ OneWriterPerEpoch stale 0 := by
  intro h
  exact absurd (h staleState stale_reachable 3 "h3" "h2" 2 0 [] ["x"] (by decide) (by decide))
    (by decide)

/-- The state just before `h3` is elected. -/
def beforeH3 : Option JSys := run stale 0 (staleRun.take 19) (JSys.empty 0)

/-- **The run is not Rust's**: version 1 was not known when `d` and `f` promised epoch 3 to
`h3`. -/
theorem stale_run_not_recordFixed :
    (beforeH3.map (fun s => decide (RecordFixed 0 s (.elect "h3" 3 2 ["d", "f"] [])))) =
      some false := by decide

end Vertep.Custody.Joint.Stale
