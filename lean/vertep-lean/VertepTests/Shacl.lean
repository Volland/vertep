import VertepLean

/-!
# The SHACL projection of types

LeanRDF's SHACL Core validator on the encoding of values agrees with the checker
(`Types.Shacl.validator_agrees`), checked here on the declaration block and values of the Rust
agreement tests (`crates/vertep-rdf/tests/types.rs`); the regular expressions of the three
`sh:pattern`s agree with the lexical rules on sample strings (the `PatternsAgree` hypothesis);
and for a non-recursive block, reading the Rust-mirrored shape triples (`Rdf.namedTypeShapes`,
the triples `vt certify` checks against Rust) with LeanRDF's shapes reader gives the same
verdicts as the shapes the theorem is about.
-/

namespace Vertep.Test.Shacl

open Vertep Vertep.Types Vertep.Types.Shacl Vertep.Rdf LeanRDF LeanRDF.Shacl

def s (x : String) : Value := .str x
def obj (fields : List (String × Value)) : Value := .object fields
def num (m : Int) (e : Nat := 0) : Value := .num m e

/-- `crates/vertep-rdf/tests/types.rs` `NAMED`. -/
def named : Decls :=
  [("Item", .record [("sku", .prim .string), ("qty", .prim .int)]),
   ("Node", .record [("label", .prim .string), ("children", .list (.named "Node"))]),
   ("Pay", .union [("Card", some (.record [("last4", .prim .string)])), ("Cash", none)]),
   ("MaybeInt", .optional (.prim .int))]

/-- LeanRDF's validator on the encoding, shapes unfolded `k` times. -/
def shaclOk (D : Decls) (k : Nat) (t : Ty) (v : Value) : Bool :=
  (results (encGraph v) (shapeOf D k t) (encNode v)).isEmpty

def agree (t : Ty) (v : Value) : Bool := shaclOk named 6 t v == check named [] t v

def cases : List (Ty × Value) :=
  [(.prim .int, num 42), (.prim .int, num 15 1), (.prim .int, num 9223372036854775808),
   (.prim .int, num (-9223372036854775808)), (.prim .decimal, num 15 1), (.prim .decimal, s "12.50"),
   (.prim .decimal, s "1e5"), (.prim .decimal, .bool true), (.prim .bool, .bool false),
   (.prim .string, num 1), (.prim .datetime, s "2026-11-11T10:00:00+02:00"),
   (.prim .datetime, s "2023-02-29T10:00:00Z"), (.prim .iri, s "urn:vertep:order:42"),
   (.prim .iri, s "orders/42"), (.prim .iri, s "a:\u0085"), (.prim .iri, s "a:\u007F"),
   (.named "Item", obj [("sku", s "a"), ("qty", num 1)]),
   (.named "Item", obj [("sku", s "a")]),
   (.named "Item", obj [("sku", s "a"), ("qty", num 1), ("note", s "x")]),
   (.named "Item", obj [("sku", s "a"), ("qty", num 1), ("note", .undefined)]),
   (.named "Item", obj [("sku", s "a"), ("qty", .undefined)]),
   (.named "Item", .array [s "a"]),
   (.named "Pay", obj [("tag", s "Cash")]),
   (.named "Pay", obj [("tag", s "Card")]),
   (.named "Pay", obj [("tag", s "Card"), ("value", obj [("last4", s "1234")])]),
   (.named "Pay", obj [("tag", s "Cash"), ("value", .undefined), ("q", .undefined)]),
   (.named "Pay", obj [("tag", s "Cash"), ("value", num 1)]),
   (.named "Pay", obj [("tag", s "Wire")]),
   (.named "Pay", obj [("tag", num 1)]),
   (.record [("x", .named "MaybeInt")], obj []),
   (.record [("x", .named "MaybeInt")], obj [("x", .null)]),
   (.record [("x", .named "MaybeInt")], obj [("x", s "1")]),
   (.named "Node", obj [("label", s "r"), ("children",
      .array [obj [("label", s "a"), ("children", .array [])]])]),
   (.named "Node", obj [("label", s "r"), ("children", .array [obj [("label", num 1)]])]),
   (.list (.prim .int), .array [num 1, num 2, num 3]),
   (.list (.prim .int), .array [num 1, s "2"]),
   (.list (.prim .int), obj []),
   (.list (.prim .int), .array []),
   (.record [], .array []),
   (.optional (.prim .string), .null),
   (.optional (.prim .string), .undefined),
   (.value, obj [("a", .array [.null])])]

-- The validator and the checker agree on every case, and both verdicts occur.
#guard cases.all fun (t, v) => agree t v
#guard (cases.filter fun (t, v) => check named [] t v).length ≥ 15
#guard (cases.filter fun (t, v) => !check named [] t v).length ≥ 15

-- Non-contractive recursion: no unfolding accepts `"x"` for `type A = optional<A>`.
#guard (List.range 8).all fun k => !shaclOk [("A", .optional (.named "A"))] k (.named "A") (s "x")
#guard shaclOk [("A", .optional (.named "A"))] 1 (.named "A") .null

/-! ## The `PatternsAgree` hypothesis on samples -/

def strings : List String :=
  ["", "x", "true", "12.50", "12,50", "-0.10", "1.", ".5", "-", "007", "1e5",
   "2026-11-11T10:00:00Z", "2026-11-11T10:00:00", "2026-02-30T10:00:00Z",
   "2024-02-29t23:59:60.5+02:00", "2023-02-29T10:00:00Z", "2000-02-29T00:00:00z",
   "1900-02-29T00:00:00Z", "2026-11-11T10:00:00+24:00", "2026-11-11T24:00:00Z",
   "2026-11-11T10:00:00.Z", "2026-13-01T10:00:00Z", "https://example.org/a", "urn:uuid:1",
   "orders/42", "https://ex.org/a b", "http://ex.org/%zz", "http://ex.org/%41",
   "https://ex.org/\u00fc", "a:\u0085", "a:\u009f", "a:\u00a0", "a:\u007f", "a:\u0001",
   "a:x^y", "a:x`y", "a:{", "a:|", "a:\\", "a:\"", "1a:x", "a-b.c+d:x", ":x", "a:"]

#guard strings.all fun x =>
  (TermTest.pattern decimalPattern "").ok (strLit x) == Lex.isDecimal x &&
  (TermTest.pattern datetimePattern "").ok (strLit x) == Lex.isDatetime x &&
  (TermTest.pattern iriPattern "").ok (strLit x) == Lex.isIri x

/-! ## The Rust-mirrored shape triples give the same verdicts -/

/-- The shapes graph of `crates/vertep-rdf/src/types/shapes.rs` `value_shapes` for a block
without recursion: the named types' shapes and a root shape on `rdf:value` of
`<urn:vertep:value>`. -/
def rustShapes (D : Decls) (t : Ty) : List Triple :=
  let iris := typeIris D
  let base := "urn:vertep:value-shape"
  let root := iriT base
  let p := property root base "p" (iriT "http://www.w3.org/1999/02/22-rdf-syntax-ns#value")
    (some 1) (some 1)
  let r := shapeRef iris base "t" t (constrain D iris (nodeAt base "t") base "t" t)
  namedTypeShapes D iris ++ [nodeShape root, ⟨root, sh "targetNode", iriT "urn:vertep:value"⟩] ++
    p.2 ++ r.2 ++ [⟨p.1, sh "node", r.1⟩]

def rustOk (D : Decls) (t : Ty) (v : Value) : Option Bool := do
  let shapes ← (readShapes (Graph.ofList (rustShapes D t))).toOption
  let data := Graph.ofList (⟨iriT "urn:vertep:value",
    iriT "http://www.w3.org/1999/02/22-rdf-syntax-ns#value", encNode v⟩ :: encT [] v)
  pure (validate data shapes).conforms

def flat : Decls := named.filter (·.1 != "Node")

/-- Types whose Rust shapes are well-formed SHACL: no `list` (see below). -/
def listFree (t : Ty) : Bool := !(reprStr t).contains "Node" && !(reprStr t).contains "list"

#guard (cases.filter fun (t, _) => listFree t).all fun (t, v) =>
  rustOk flat t v == some (check flat [] t v)

/- **Known disagreement** (verification backlog 4.2): `shapes.rs` mints the `rdf:rest*` path
node and the sequence path of a list's member property as IRIs, and SHACL §2.3.1 reads an IRI
as a predicate path, so LeanRDF (and any SHACL engine) never reaches the members: the
Rust-published shapes accept `[1, "2"]` as a `list<int>`. The shapes of `shapeOf` (and the
theorem) use the intended path `rdf:rest*/rdf:first`. -/
#guard rustOk flat (.list (.prim .int)) (.array [num 1, s "2"]) == some true
#guard shaclOk flat 1 (.list (.prim .int)) (.array [num 1, s "2"]) == false

/-! ## The theorem, applied -/

theorem named_tags : ∀ d ∈ named, JsonSchema.tagsOk d.2 = true := by decide +kernel

/-- An `Item` value has its type exactly when its encoding conforms to some unfolding of the
shapes, by LeanRDF's SHACL semantics. -/
-- @tg: verifies:: [[openspec:formal-model#SHACL Projection Agreement#Encoding conforms exactly when typed]] {level: unit}
theorem item_agrees (hP : PatternsAgree) (v : Value) (hv : keysOk v = true) :
    (∃ k, results (encGraph v) (shapeOf named k (.named "Item")) (encNode v) = []) ↔
      HasType named v (.named "Item") :=
  validator_agrees hP named named_tags _ (by decide +kernel) v hv

/-- Conformance to the `Node` shapes (recursive through a list) survives one more unfolding. -/
-- @tg: verifies:: [[openspec:formal-model#SHACL Projection Agreement#Unfolding is monotone]] {level: unit}
theorem node_mono (hP : PatternsAgree) (k : Nat) (v : Value) (hv : keysOk v = true) :
    Conforms (encGraph v) (shapeOf named k (.named "Node")) (encNode v) →
      Conforms (encGraph v) (shapeOf named (k + 1) (.named "Node")) (encNode v) :=
  conforms_mono hP named named_tags k _ (by decide +kernel) v hv

end Vertep.Test.Shacl
