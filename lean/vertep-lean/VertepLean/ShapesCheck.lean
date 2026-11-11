import VertepLean.Rdf.Shapes
import VertepLean.Oracle

/-!
# Empirical agreement of the chart shapes with `WellFormed`

`vertep-shapes FILE.nq…` validates each chart graph against the chart shapes with LeanRDF's
SHACL Core validator and, when the graph decodes, decides `WellFormed` on the decoded chart; it
prints one line per file and exits `1` if any verdicts disagree. This is the empirical side of
the Shapes Equivalence Theorem, which is not proven (see `Pending/ShapesEquiv.lean`).
-/

namespace Vertep.ShapesCheck

open LeanRDF

/-- `(shapes conform, well-formed)` for a graph given as triples. -/
def verdicts (ts : List Triple) : Option Bool × Option Bool :=
  let sh := Rdf.shapesConform (Graph.ofList ts)
  let wf := match Rdf.fromRDF ts with
    | .ok (c, _) => some (checkWF c)
    | .error _ => none
  (sh, wf)

/-- A state with children and transitions. -/
def st (id : String) (el : StateElement := .state) (children : List State := [])
    (ts : List Transition := []) (initial : List String := []) (data : List Data := []) : State :=
  .mk id false el initial none children ts [] [] data [] none []

/-- Sample charts for the agreement check: their names, and whether they are well-formed. -/
def samples : List (String × Chart) :=
  [ ("wf-basic", { states := [st "a" (children := [st "a1" (ts := [{ events := ["e"], targets := ["a2"] }]),
        st "a2"]), st "done" .final] }),
    ("wf-parallel-targets", { states := [st "p" .parallel [st "r1" (children := [st "x1", st "x2"]),
        st "r2" (children := [st "y1", st "y2"])], st "s" (ts := [{ targets := ["x2", "y2"] }])] }),
    ("bad-target-set", { states := [st "a" (children := [st "a1" (ts := [{ targets := ["a1", "a2"] }]),
        st "a2"])] }),
    ("bad-history-sibling", { initial := ["b"], states := [st "a" (children := [st "h" (.history .shallow),
        st "a1", st "a2"]), st "b" (ts := [{ targets := ["h", "a2"] }])] }),
    ("bad-history-cycle", { states := [st "a" (initial := ["h"]) (children := [st "h" (.history .shallow),
        st "a1"])] }),
    ("bad-final-transition", { states := [st "f" .final (ts := [{ targets := ["f"] }])] }),
    ("bad-history-at-root", { states := [st "s", st "h" (.history .deep)] }),
    ("bad-initial-not-descendant", { states := [st "a" (initial := ["b"]) (children := [st "a1"]), st "b"] }),
    ("bad-parallel-history-only", { states := [st "p" .parallel [st "h" (.history .shallow)]] }),
    ("bad-duplicate-data", { data := [{ id := "x" }], states := [st "s" (data := [{ id := "x" }])] }),
    ("bad-unknown-target", { states := [st "s" (ts := [{ targets := ["nowhere"] }])] }) ]

/-- Write the samples' projections as N-Quads into `dir`. -/
def writeSamples (dir : String) : IO Unit := do
  IO.FS.createDirAll dir
  for (n, c) in samples do
    IO.FS.writeFile s!"{dir}/{n}.nq" (Rdf.publishNQuads c)

/-- The command line. -/
def run (args : List String) : IO UInt32 := do
  if let ["--samples", dir] := args then
    writeSamples dir
    return 0
  let mut disagree := false
  for path in args do
    let src ← IO.FS.readFile path
    match NQuads.parse src with
    | .error e => IO.println s!"{path}: N-Quads error {e}"; disagree := true
    | .ok ds =>
      let (sh, wf) := verdicts (ds.toList.map (·.triple))
      let agree := match sh, wf with
        | some a, some b => a == b
        | some a, none => !a
        | none, _ => false
      unless agree do disagree := true
      IO.println s!"{path}: shapes={sh} wellFormed={wf} {if agree then "agree" else "DISAGREE"}"
  return (if disagree then 1 else 0)

end Vertep.ShapesCheck
