import VertepLean.Rdf.Reconstruct

/-!
# RDF theorems

Proven: reconstruction is sound for the projection (`Rdf.fromRDF_sound`, in
`VertepLean.Rdf.Reconstruct`), and the canonical form and version IRI are functions of the chart
(`canonical_determined`, `version_determined`). The round-trip identity
`fromRDF (project c base) = .ok (c, base)` on well-formed charts is `Rdf.fromRDF_project`, in
`VertepLean.Rdf.RoundTrip.Chart`.
-/

namespace Vertep.Rdf

/-- **Canonical form determined by the chart.** Equal charts have equal canonical N-Quads. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "canonical form determined; the round trip is fromRDF_project"}
theorem canonical_determined (c₁ c₂ : Chart) (h : c₁ = c₂) :
    canonicalNQuads c₁ = canonicalNQuads c₂ := by subst h; rfl

/-- Equal charts have equal version IRIs and equal published projections. -/
theorem version_determined (c₁ c₂ : Chart) (h : c₁ = c₂) :
    chartVersion c₁ = chartVersion c₂ ∧ publish c₁ = publish c₂ := by subst h; exact ⟨rfl, rfl⟩

end Vertep.Rdf
