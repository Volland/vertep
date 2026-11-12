import VertepLean.Rdf.Types

/-!
# Canonical N-Quads, chart hash and chart version IRI

The projection of a chart is the chart projection (`project`) together with the type triples of
its `vt:types` block (`chartTypes`, Rust `project_chart_types`), so the version covers the
chart's types. It has no blank nodes, so its RDFC-1.0 canonical form is its set of N-Quads lines
(`canonicalOf`). The chart hash is the SHA-256 of the canonical N-Quads of the projection under
the placeholder base, and the version IRI is `urn:vertep:chart:sha256:<hex>` (design D7).
-/

namespace Vertep.Rdf

open LeanRDF

/-- The whole projection of a chart under `base` (Rust `project_with_base`): the chart triples
and its type triples. -/
def projectFull (c : Chart) (base : String) : List Triple := project c base ++ chartTypes c base

/-- The canonical N-Quads of a chart (its projection under the placeholder base). -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "projection mirrored from vertep-rdf; round trip pending"}
def canonicalNQuads (c : Chart) : String := canonicalOf (projectFull c placeholderBase)

/-- The lowercase hex SHA-256 of the canonical N-Quads. -/
def chartHash (c : Chart) : String := Sha256.hex (canonicalNQuads c)

/-- The chart version IRI. -/
def chartVersion (c : Chart) : String := versionPrefix ++ chartHash c

/-- The published projection: node IRIs minted under the version IRI. -/
def publish (c : Chart) : List Triple := projectFull c (chartVersion c)

/-- The published projection as N-Quads, in the named graph of the version (design D6). -/
def publishNQuads (c : Chart) : String :=
  let v := chartVersion c
  String.join (dedupSorted (((projectFull c v).map fun t =>
      writeTerm t.s ++ " " ++ writeTerm t.p ++ " " ++ writeTerm t.o ++ " <" ++ v ++ "> .\n").mergeSort
    (fun a b => compare a b != .gt)))

end Vertep.Rdf
