import VertepTests.Support

/-! # `WellFormed` and its decision procedure -/

namespace Vertep.Test

/-- Root `a` (compound: `a1`, `a2`) and final `done`. -/
def wfChart : Chart :=
  { states := [node "a" (children := [node "a1" (ts := [tr ["e"] ["a2"]]), node "a2"]),
               node "done" .final] }

/-- A duplicate identifier and an `initial` attribute naming a non-descendant. -/
def badChart : Chart :=
  { states := [node "a" (initial := ["b"]) (children := [node "a1", node "a1"]), node "b"] }

/-- Targets `a1 a2` of one transition: siblings under a compound state, an illegal set. -/
def badTargets : Chart :=
  { states := [node "a" (children := [node "a1" (ts := [tr ["e"] ["a1", "a2"]]), node "a2"])] }

/-- The same targets under a parallel state are a legal set. -/
def parallelTargets : Chart :=
  { states := [node "p" .parallel [node "r1" (children := [node "x1", node "x2"]),
                                   node "r2" (children := [node "y1", node "y2"])],
               node "s" (ts := [tr ["e"] ["x2", "y2"]])] }

/-- The decision procedure agrees with `WellFormed`, by its correctness theorem, on charts
that are and are not well-formed. -/
-- @tg: verifies:: [[openspec:formal-model#IR Model In Lean#Decision procedure correct]] {level: unit}
theorem decision_procedure_correct :
    WellFormed wfChart ∧ ¬WellFormed badChart ∧ ¬WellFormed badTargets ∧
      WellFormed parallelTargets :=
  ⟨(checkWF_iff _).1 (by decide), fun h => absurd ((checkWF_iff _).2 h) (by decide),
   fun h => absurd ((checkWF_iff _).2 h) (by decide), (checkWF_iff _).1 (by decide)⟩

#guard checkWF wfChart
#guard !checkWF badChart
#guard !checkWF badTargets
#guard checkWF parallelTargets
-- the Rust validator's rules: empty chart, history at the root, final with transitions
#guard !checkWF {}
#guard !checkWF { states := [node "h" (.history .shallow)] }
#guard !checkWF { states := [node "f" .final (ts := [tr ["e"] ["f"]])] }
#guard !checkWF { states := [node "s" (children := [node "h" (.history .deep)])] }

/-! Known gaps in the well-formedness rules (shared with the Rust validator), recorded so a fix
shows up here; see `Pending/ConfigWF.lean`. -/

/-- A history target next to a sibling target passes the target-set rule. -/
def historySibling : Chart :=
  { initial := ["b"],
    states := [node "a" (children := [node "h" (.history .shallow), node "a1", node "a2"]),
               node "b" (ts := [tr ["e"] ["h", "a2"]])] }
#guard checkWF historySibling
-- ... and entering it leaves compound `a` with two active children
#guard idsAfter historySibling ["e"] == ["a", "a1", "a2"]

/-- An `initial` naming a history state without a default transition: entry never ends. -/
def historyInitial : Chart :=
  { states := [node "a" (initial := ["h"]) (children := [node "h" (.history .shallow), node "a1"])] }
#guard checkWF historyInitial
#guard match start (cc historyInitial) { info := { sessionid := "s" } } [] with
  | .error (.illFormed _) => true
  | _ => false

end Vertep.Test
