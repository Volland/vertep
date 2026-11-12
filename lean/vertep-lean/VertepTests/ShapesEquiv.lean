import VertepLean

/-!
# The chart shapes as a Lean term

`Rdf.ShapesL.chartShapesL`, the term the Shapes Equivalence theorems are stated over, is the
chart shapes Turtle (`Rdf.chartShapesTtl`, kept identical to
`crates/vertep-rdf/shapes/vt-chart-shapes.ttl`) read with LeanRDF's Turtle parser and SHACL
shapes reader, up to what SHACL validation ignores: shape identifiers, severities, messages, and
the order of targets, components and top-level shapes. The parser and the reader are `partial`,
so this is a test, not a proof.
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

end Vertep.Test.ShapesEquiv
