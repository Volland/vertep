import VertepLean.Rdf.Project
import VertepLean.Sha256

/-!
# Canonical N-Quads, chart hash and chart version IRI

A projection has no blank nodes, so its RDFC-1.0 canonical form is its set of N-Quads lines
(default graph) sorted by code point, written as `crates/vertep-rdf/src/canon.rs` writes them.
The chart hash is the SHA-256 of the canonical N-Quads of the projection under the placeholder
base, and the version IRI is `urn:vertep:chart:sha256:<hex>` (design D7).
-/

namespace Vertep.Rdf

open LeanRDF

/-- `canon.rs` `write_literal` escapes. -/
def escapeLiteral (v : String) : String :=
  let hex := "0123456789ABCDEF".toList
  String.join (v.toList.map fun c =>
    match c with
    | '\x08' => "\\b"
    | '\t' => "\\t"
    | '\n' => "\\n"
    | '\x0C' => "\\f"
    | '\r' => "\\r"
    | '"' => "\\\""
    | '\\' => "\\\\"
    | c =>
      if c.toNat < 0x20 || c.toNat == 0x7F then
        "\\u" ++ String.ofList ((List.range 4).reverse.map fun i => hex[(c.toNat >>> (4 * i)) % 16]!)
      else c.toString)

/-- One term in N-Quads syntax (no blank nodes or language tags arise in projections; they
are written anyway for completeness). -/
def writeTerm : Term → String
  | .iri i => "<" ++ i.val ++ ">"
  | .bnode b => "_:" ++ b.label
  | .lit (.typed v dt) =>
    "\"" ++ escapeLiteral v ++ "\"" ++ (if dt == xsdString then "" else "^^<" ++ dt.val ++ ">")
  | .lit (.lang v tag) => "\"" ++ escapeLiteral v ++ "\"@" ++ tag
  | .lit (.dirLang v tag d) => "\"" ++ escapeLiteral v ++ "\"@" ++ tag ++ "--" ++ d.toString
  | .triple s p o => "<<( " ++ writeTerm s ++ " " ++ writeTerm p ++ " " ++ writeTerm o ++ " )>>"

/-- One N-Quads line in the default graph. -/
def writeLine (t : Triple) : String :=
  writeTerm t.s ++ " " ++ writeTerm t.p ++ " " ++ writeTerm t.o ++ " .\n"

/-- Remove adjacent duplicates. -/
def dedupSorted : List String → List String
  | a :: b :: rest => if a == b then dedupSorted (b :: rest) else a :: dedupSorted (b :: rest)
  | l => l

/-- Canonical N-Quads of a blank-node-free set of triples: lines sorted and deduplicated. -/
def canonicalOf (ts : List Triple) : String :=
  String.join (dedupSorted ((ts.map writeLine).mergeSort (fun a b => compare a b != .gt)))

/-- The canonical N-Quads of a chart (its projection under the placeholder base). -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "projection mirrored from vertep-rdf; round trip pending"}
def canonicalNQuads (c : Chart) : String := canonicalOf (project c placeholderBase)

/-- The lowercase hex SHA-256 of the canonical N-Quads. -/
def chartHash (c : Chart) : String := Sha256.hex (canonicalNQuads c)

/-- The chart version IRI. -/
def chartVersion (c : Chart) : String := versionPrefix ++ chartHash c

/-- The published projection: node IRIs minted under the version IRI. -/
def publish (c : Chart) : List Triple := project c (chartVersion c)

/-- The published projection as N-Quads, in the named graph of the version (design D6). -/
def publishNQuads (c : Chart) : String :=
  let v := chartVersion c
  String.join (dedupSorted (((project c v).map fun t =>
      writeTerm t.s ++ " " ++ writeTerm t.p ++ " " ++ writeTerm t.o ++ " <" ++ v ++ "> .\n").mergeSort
    (fun a b => compare a b != .gt)))

end Vertep.Rdf
