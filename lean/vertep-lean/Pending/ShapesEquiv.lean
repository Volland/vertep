import VertepLean

/-!
# PENDING: Shapes Equivalence Theorem, the shapes over the whole chart (verification backlog 2.3)

Not part of the build or the axiom gate; statements only.

## What changed and what is proven
* The original statement quantified over `shapes` with `Rdf.chartShapes = .ok shapes`. The Turtle
  parser and LeanRDF's shapes reader are `partial`, so that hypothesis says nothing in the logic
  and the statement can be neither proven nor refuted. The theorem is now stated over
  `Rdf.ShapesL.chartShapesL`, the shapes as a Lean term; `VertepTests/ShapesEquiv.lean` checks
  (`#guard`) that reading the Turtle gives the same shapes up to identifiers, messages,
  severities and order.
* Proven (`VertepLean/Rdf/Equiv/`): `nodeShapes_iff` (every shape with a class target, the
  initial-reference scope shape and the history shape without its cycle component hold on the
  projection of a chart with unique state identifiers exactly when `WFNodes c`),
  `uniqueStateId_ok`, `wellFormed_iff` (`WellFormed` = unique identifiers ∧ `WFNodes` ∧ target
  sets ∧ unique data identifiers ∧ acyclic history defaults) and `shapes_equiv_nodes`
  (conformance = `WFNodes` ∧ the three graph-wide shapes below).
* No counterexample was found: the projection materializes `vt:ancestor`, `vt:scope`,
  `vt:choice` selection nodes and `vt:historyDefault`, so the rules over the whole chart are
  expressible as SHACL Core over the projection, and the analysis below suggests every
  remaining lemma is true.

## Remaining lemmas
-/

namespace Vertep.Pending

open LeanRDF Vertep.Rdf Vertep.Rdf.ShapesL

/-- Conformance forces unique state identifiers: two occurrences with one identifier share a
node, which then has two containers (`[^vt:child] maxCount 1`) or two `vt:order`s. -/
def ConformsNodup : Prop :=
  ∀ c : Chart, (Shacl.validate (toRDF c) chartShapesL).conforms = true → c.ids.Nodup

/-- The selection nodes of every target list have one `vt:choice` exactly when every target set
is legal (the chains of `Ctx.targets` against `targetPosition`, `ancestors` and `LcaParallel`). -/
def SelectionIff : Prop :=
  ∀ c : Chart, c.ids.Nodup → Equiv.WFNodes c →
    ((∀ x o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ toRDF c → Shacl.Conforms (toRDF c) selectionS x) ↔
      (TargetSetOk c c.initial ∧ ∀ p ∈ c.parented, Equiv.TSets c p.2))

/-- The cycle component of `vt:HistoryShape` holds at every history node exactly when history
defaults are acyclic (paths of any length against `historyReach` bounded by the state count). -/
def CycleIff : Prop :=
  ∀ c : Chart, c.ids.Nodup →
    ((∀ x, Ty (toRDF c) x "History" → Shacl.CompHolds (toRDF c) none cycleC x) ↔ HistoryDefaultsAcyclic c)

/-- `vt:UniqueDataIdShape` holds exactly when data identifiers are unique. -/
def DataIdIff : Prop :=
  ∀ c : Chart, c.ids.Nodup →
    ((∀ x s, (⟨s, .iri (vi "dataId"), x⟩ : Triple) ∈ toRDF c → Shacl.Conforms (toRDF c) uniqueDataIdS x) ↔
      (c.allData.map (·.id)).Nodup)

/-- `validate chartShapesL (toRDF c) = conforms ↔ WellFormed c`; follows from the four lemmas
above with `Equiv.shapes_equiv_nodes` and `Equiv.wellFormed_iff`. -/
def ShapesEquivalence : Prop :=
  ∀ c : Chart, (Shacl.validate (toRDF c) chartShapesL).conforms = true ↔ WellFormed c

end Vertep.Pending
