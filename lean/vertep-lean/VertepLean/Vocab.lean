import LeanRDF.Core.Term
import LeanRDF.Core.Dataset
import LeanRDF.Parse.NQuads.Parser
import VertepLean.IR

/-!
# The `vt:` vocabulary on LeanRDF (groundwork for the RDF phase)

Charts cross from Rust to Lean as N-Quads of their RDF projection (design D6), read with
LeanRDF's N-Quads parser into an RDF 1.2 dataset. This module fixes the IRIs the projection and
the certificates use; the decoding `fromRDF`, the projection `toRDF`, the shapes and their
theorems (tasks 7.7, 7.8) build on it once the Rust `vt:` vocabulary (task 6.1) exists.
-/

namespace Vertep.Vocab

open LeanRDF

/-- A term of the Vertep vocabulary `https://w3id.org/vertep/vt#`. -/
def vt (localName : String) : IRI := ⟨vtNs ++ localName⟩

/-- The version IRI of a chart from the hex SHA-256 of its canonical N-Quads (design D7). -/
def chartVersionIri (hex : String) : IRI := ⟨"urn:vertep:chart:sha256:" ++ hex⟩

/-- Read an N-Quads document into an RDF 1.2 dataset. -/
def readNQuads (src : String) : Except String Dataset :=
  match NQuads.parse src with
  | .ok d => .ok d
  | .error e => .error (toString e)

end Vertep.Vocab
