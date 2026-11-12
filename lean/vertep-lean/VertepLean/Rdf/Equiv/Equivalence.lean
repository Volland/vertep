import VertepLean.Rdf.Equiv.Selection

/-!
# The Shapes Equivalence Theorem

**Validating the projection of a chart against the chart shapes conforms exactly when the chart
is `WellFormed`**, for every base IRI (`shapes_equivalence`). The shapes are the Lean term
`Rdf.ShapesL.chartShapesL`; the Turtle parser and LeanRDF's shapes reader are `partial`, so the
shapes read from `vt-chart-shapes.ttl` cannot appear in a proof, and `VertepTests/ShapesEquiv.lean`
checks that they are this term up to identifiers, messages, severities and order.

The proof splits `WellFormed` (`wellFormed_iff`) and conformance (`shapes_equiv_nodes`) alike:

* unique state identifiers: conformance forces them (`conforms_nodup`), every projection satisfies
  `vt:UniqueStateIdShape` (`uniqueStateId_ok`);
* the per-node rules: the node shapes (`nodeShapes_iff`);
* legal target sets: the selection nodes (`selection_iff`);
* acyclic history defaults: the cycle component of `vt:HistoryShape` (`cycle_iff`);
* unique data identifiers: `vt:UniqueDataIdShape` (`dataId_iff`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-- **The Shapes Equivalence Theorem.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {note: "over the Lean term chartShapesL, for every base"}
theorem shapes_equivalence (c : Chart) (base : String) :
    (Shacl.validate (toRDF c base) chartShapesL).conforms = true ↔ WellFormed c := by
  rw [wellFormed_iff]
  constructor
  · intro h
    have hnd := conforms_nodup c base h
    obtain ⟨h1, h2, h3, h4⟩ := (shapes_equiv_nodes c base hnd).1 h
    obtain ⟨h5, h6⟩ := (selection_iff c base hnd).1 h3
    exact ⟨hnd, h1, h5, h6, (dataId_iff c base hnd).1 h4, (cycle_iff c base hnd).1 h2⟩
  · rintro ⟨hnd, h1, h5, h6, h7, h8⟩
    exact (shapes_equiv_nodes c base hnd).2 ⟨h1, (cycle_iff c base hnd).2 h8,
      (selection_iff c base hnd).2 ⟨h5, h6⟩, (dataId_iff c base hnd).2 h7⟩

/-- Forward direction: a well-formed chart's projection conforms. -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {note: "forward direction"}
theorem shapes_forward {c : Chart} (base : String) (h : WellFormed c) :
    (Shacl.validate (toRDF c base) chartShapesL).conforms = true :=
  (shapes_equivalence c base).2 h

/-- Backward direction: a chart whose projection conforms is well-formed. -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {note: "backward direction"}
theorem shapes_backward {c : Chart} (base : String)
    (h : (Shacl.validate (toRDF c base) chartShapesL).conforms = true) : WellFormed c :=
  (shapes_equivalence c base).1 h

end Vertep.Rdf.Equiv
