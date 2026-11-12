import VertepLean

/-!
# PENDING: Shapes Equivalence Theorem (formal-model, task 7.8)

Not part of the build or the axiom gate; statements only.

The chart shapes are transcribed into LeanRDF verbatim (`Rdf.chartShapesTtl`, kept identical to
`crates/vertep-rdf/shapes/vt-chart-shapes.ttl` by a test) and read with LeanRDF's Turtle parser
and SHACL shapes reader; `Rdf.shapesConform` runs LeanRDF's SHACL Core validator, which
`LeanRDF.Shacl.validate_conforms` proves equal to the SHACL Core semantics. `vertep-shapes`
checks agreement empirically: on 11 sample charts (2 well-formed, 9 each breaking one rule) and 6
IRP charts the shapes verdict equals `checkWF`. It is slow (about a minute per small chart).

## Missing pieces for the proof
* The shapes as a Lean term with a proof that `Rdf.chartShapes = .ok shapes` (evaluating the
  Turtle parser and the shapes reader in the kernel, or a hand transcription proven equal).
* Path lemmas on `toRDF c` (the value nodes of every shape path are those of the IR); the
  capture and lookup lemmas behind `Rdf.fromRDF_project` (`VertepLean/Rdf/RoundTrip/`) give
  them for the decoder's own paths.
* One lemma per shape tying its `DataConforms` condition on `toRDF c` to the `WellFormed` rule it
  encodes; the derived triples (`vt:ancestor`, `vt:scope`, `vt:choice`, `vt:historyDefault`)
  need lemmas tying them to `descIds`, `targetPosition` and `historyDefaultEdges`.
-/

namespace Vertep.Pending

open LeanRDF

/-- `validate shapes (toRDF c) = conforms ↔ WellFormed c`. -/
def ShapesEquivalence : Prop :=
  ∀ c : Chart, ∀ shapes, Rdf.chartShapes = .ok shapes →
    ((Shacl.validate (Rdf.toRDF c) shapes).conforms = true ↔ WellFormed c)

end Vertep.Pending
