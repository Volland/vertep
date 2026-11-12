import VertepLean

/-!
# The chart shapes as a Lean term

`Rdf.ShapesL.chartShapesL`, the term the Shapes Equivalence theorems are stated over, is the
chart shapes Turtle (`Rdf.chartShapesTtl`, kept identical to
`crates/vertep-rdf/shapes/vt-chart-shapes.ttl`) read with LeanRDF's Turtle parser and SHACL
shapes reader, up to what SHACL validation ignores: shape identifiers, severities, messages, and
the order of targets, components and top-level shapes. The parser and the reader are `partial`,
so this is a test, not a proof.

The forward and backward directions of `Equiv.shapes_equivalence` are applied to a well-formed
sample and to ill-formed charts (a duplicate identifier, an illegal target set), and the
executable validator is run on the same projections.
-/

namespace Vertep.Test.ShapesEquiv

open Vertep Vertep.Rdf LeanRDF

-- The Turtle shapes read to `chartShapesL`.
-- @tg: verifies:: [[openspec:formal-model#Shapes Equivalence Theorem#Shapes term]] {level: unit}
#guard match Rdf.chartShapes with
  | .ok s => ShapesL.canon s == ShapesL.canon ShapesL.chartShapesL
  | .error _ => false

/-- A well-formed sample with unique identifiers: the node-shape theorem applies. -/
def sample : Chart :=
  { states := [.mk "a" false .state [] none [.new "a1" .state, .new "a2" .state]
      [{ events := ["e"], targets := ["b"] }] [] [] [] [] none [], .new "b" .final] }

-- @tg: verifies:: [[openspec:formal-model#Shapes Equivalence Theorem#Node shapes]] {level: unit}
example : (∀ x, ShapesL.Good (toRDF sample) x) ↔ Equiv.WFNodes sample :=
  Equiv.nodeShapes_iff sample placeholderBase (by decide)

-- @tg: verifies:: [[openspec:formal-model#Shapes Equivalence Theorem#Forward direction]] {level: unit}
example : (Shacl.validate (toRDF sample) ShapesL.chartShapesL).conforms = true :=
  Equiv.shapes_forward placeholderBase (by decide)

#guard (Shacl.validate (toRDF sample) ShapesL.chartShapesL).conforms

/-- Two states with one identifier. -/
def dupId : Chart := { states := [.new "a" .state, .new "a" .final] }

/-- A compound state's transition targeting both of its children: an illegal target set. -/
def badSet : Chart :=
  { states := [.mk "a" false .state [] none [.new "a1" .state, .new "a2" .state]
      [{ events := ["e"], targets := ["a1", "a2"] }] [] [] [] [] none []] }

-- @tg: verifies:: [[openspec:formal-model#Shapes Equivalence Theorem#Backward direction]] {level: unit}
example : (Shacl.validate (toRDF dupId) ShapesL.chartShapesL).conforms ≠ true :=
  fun h => (by decide : ¬ WellFormed dupId) (Equiv.shapes_backward placeholderBase h)

example : (Shacl.validate (toRDF badSet) ShapesL.chartShapesL).conforms ≠ true :=
  fun h => (by decide : ¬ WellFormed badSet) (Equiv.shapes_backward placeholderBase h)

#guard !(Shacl.validate (toRDF dupId) ShapesL.chartShapesL).conforms
#guard !(Shacl.validate (toRDF badSet) ShapesL.chartShapesL).conforms

end Vertep.Test.ShapesEquiv
