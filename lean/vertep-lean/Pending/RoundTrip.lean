import VertepLean

/-!
# PENDING: RDF Round Trip Theorem, round-trip half (formal-model, task 7.7)

Not part of the build or the axiom gate; statements only, as `Prop` definitions.

Proven in the library: `Rdf.fromRDF_sound` (whatever reconstruction returns, the graph contains
its whole projection) and `Rdf.canonical_determined` / `Rdf.version_determined` (the canonical
N-Quads and the version IRI are functions of the chart). Checked, not proven: on all 201 IRP
root charts, decoding the Rust projection, re-projecting in Lean and hashing gives the Rust
version IRI, and the Lean projection of every test chart decodes back to the chart.

## Missing lemmas
* Node IRIs are injective: `percentEncode` is injective and never yields `#`, `:` or `/`, and the
  fragment grammar (`s.<id>`, `<owner>:<step><n>`, `:sel:`) is prefix-free, so distinct nodes of
  the projection get distinct IRIs (needs `ids.Nodup` from `WellFormed`).
* Lookup lemmas: in `Index.of (project c b)`, the objects of `(node, p)` are exactly the projected
  ones (a characterization of `project` per node and property).
* `vt:order` lemmas: the projected orders of a collection are `0..n-1` in list order, so
  `Decode.ordered` returns the members in document order.
* Decoding each IR node type inverts projecting it (mutual induction over `State`, `Exec`,
  branches), and the closed-world check holds trivially on `project c b` itself.
-/

namespace Vertep.Pending

/-- Reconstruction after projection is the identity on well-formed charts. -/
def RoundTrip : Prop :=
  ∀ c : Chart, WellFormed c → ∀ base : String,
    Rdf.fromRDF (Rdf.project c base) = .ok (c, base)

end Vertep.Pending
