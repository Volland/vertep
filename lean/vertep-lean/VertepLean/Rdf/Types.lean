import VertepLean.Rdf.NQuads
import VertepLean.Sha256
import VertepLean.Types.Syntax

/-!
# The type triples of a chart

Mirrors `crates/vertep-rdf/src/types/` as far as the chart projection uses it
(`project_chart_types`): a typed chart's projection carries, next to the chart triples,

* a `vt:declaresType` edge from the chart node to every named type of its `vt:types` block that
  has a type IRI;
* each such type's definition graph minted under its type IRI (`ident.rs` `write_definition`);
* each such type's SHACL node shape at its type IRI, with nested shapes minted under it
  (`shapes.rs` `TypeShapes::named_types`).

Type IRIs are content addresses (`ident.rs` `TypeIris::of`): `urn:vertep:type:sha256:` and the
SHA-256 of the canonical N-Quads of the definition minted under `urn:vertep:type:self`, where a
group of mutually recursive types is hashed together, each other member `M` under
`urn:vertep:type:self:M`. Every term here is an IRI or a literal, so the canonical form of a
graph is its sorted N-Quads lines (`canonicalOf`).
-/

namespace Vertep.Rdf

open LeanRDF
open Vertep.Types

/-- The prefix of type IRIs (Rust `TYPE_PREFIX`). -/
def typePrefix : String := "urn:vertep:type:sha256:"
/-- The base a definition is minted under before its IRI is known (Rust `TYPE_PLACEHOLDER`). -/
def typePlaceholder : String := "urn:vertep:type:self"

def shNs : String := "http://www.w3.org/ns/shacl#"
def sh (l : String) : Term := .iri ⟨shNs ++ l⟩
def iriT (s : String) : Term := .iri ⟨s⟩
def rdfFirst : Term := iriT "http://www.w3.org/1999/02/22-rdf-syntax-ns#first"
def rdfRest : Term := iriT "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"
def rdfNil : Term := iriT "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"
def xsdLong : String := "http://www.w3.org/2001/XMLSchema#long"
def xsdDecimal : String := "http://www.w3.org/2001/XMLSchema#decimal"
/-- The namespace of record field predicates (Rust `FIELD_NS`). -/
def fieldNs : String := "https://w3id.org/vertep/field#"

/-- `<base>#<fragment>` (Rust `node_iri`). -/
def nodeAt (base frag : String) : Term := iriT (base ++ "#" ++ frag)

/-- `<path>.<kind>.<percent-encoded name>` (Rust `segment`). -/
def segment (path kind name : String) : String := path ++ "." ++ kind ++ "." ++ percentEncode name

/-- The predicate of record field `name` (Rust `field_predicate`). -/
def fieldPredicate (name : String) : Term := iriT (fieldNs ++ percentEncode name)

/-- The `vt:` individual of a primitive type. -/
def primTerm : Prim → Term
  | .bool => vt "BoolType"
  | .int => vt "IntType"
  | .decimal => vt "DecimalType"
  | .string => vt "StringType"
  | .datetime => vt "DatetimeType"
  | .iri => vt "IriType"

/-! ## Names a type uses -/

/-- The named types a type refers to (Rust `Type::references`, with repetitions). -/
def refs (t : Ty) : List String :=
  match t with
  | .named n => [n]
  | .record fs => fs.attach.flatMap fun ⟨(_, ft), _⟩ => refs ft
  | .union vs => vs.attach.flatMap fun ⟨(_, p), _⟩ =>
    match p with
    | none => []
    | some pt => refs pt
  | .list t => refs t
  | .optional t => refs t
  | _ => []
termination_by sizeOf t
decreasing_by
  all_goals first
    | simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega
    | (rename_i h; have := List.sizeOf_lt_of_mem h
       simp only [Ty.record.sizeOf_spec, Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec,
         Option.some.sizeOf_spec] at this ⊢
       omega)

/-! ## Definitions (`ident.rs`) -/

/-- The term of a type and the triples defining it, minted under `base` at `path` (Rust
`type_term`); `resolve` names the node of a named type, `vt:ValueType` when it has none. -/
def typeTerm (resolve : String → Option Term) (base path : String) (t : Ty) : Term × List Triple :=
  match t with
  | .prim p => (primTerm p, [])
  | .value => (vt "ValueType", [])
  | .named n => ((resolve n).getD (vt "ValueType"), [])
  | .record fs =>
    let n := nodeAt base path
    (n, ⟨n, rdfType, vt "RecordType"⟩ :: fs.attach.flatMap fun ⟨(name, ft), _⟩ =>
      let fpath := segment path "f" name
      let fnode := nodeAt base fpath
      let r := typeTerm resolve base (fpath ++ ".t") ft
      [⟨n, vt "typeField", fnode⟩, ⟨fnode, rdfType, vt "TypeField"⟩,
       ⟨fnode, vt "fieldName", strLit name⟩, ⟨fnode, vt "fieldType", r.1⟩] ++ r.2)
  | .union vs =>
    let n := nodeAt base path
    (n, ⟨n, rdfType, vt "UnionType"⟩ :: vs.attach.flatMap fun ⟨(tag, p), _⟩ =>
      let vpath := segment path "v" tag
      let vnode := nodeAt base vpath
      [⟨n, vt "typeVariant", vnode⟩, ⟨vnode, rdfType, vt "TypeVariant"⟩,
       ⟨vnode, vt "variantTag", strLit tag⟩] ++
      match p with
      | none => []
      | some pt =>
        let r := typeTerm resolve base (vpath ++ ".t") pt
        ⟨vnode, vt "payloadType", r.1⟩ :: r.2)
  | .list it =>
    let n := nodeAt base path
    let r := typeTerm resolve base (path ++ ".l") it
    (n, ⟨n, rdfType, vt "ListType"⟩ :: ⟨n, vt "itemType", r.1⟩ :: r.2)
  | .optional it =>
    let n := nodeAt base path
    let r := typeTerm resolve base (path ++ ".o") it
    (n, ⟨n, rdfType, vt "OptionalType"⟩ :: ⟨n, vt "someType", r.1⟩ :: r.2)
termination_by sizeOf t
decreasing_by
  all_goals first
    | simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega
    | (rename_i h; have := List.sizeOf_lt_of_mem h
       simp only [Ty.record.sizeOf_spec, Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec,
         Option.some.sizeOf_spec] at this ⊢
       omega)

/-- The definition of the named type `name = t` under `base` (Rust `write_definition`). -/
def writeDefinition (resolve : String → Option Term) (base name : String) (t : Ty) :
    List Triple :=
  let node := iriT base
  let r := typeTerm resolve base "t" t
  [⟨node, rdfType, vt "NamedType"⟩, ⟨node, vt "typeName", strLit name⟩,
   ⟨node, vt "definition", r.1⟩] ++ r.2

/-! ## Type IRIs (`TypeIris::of`) -/

/-- The declared names, first declarations first. -/
def declaredNames (D : Decls) : List String := (D.map (·.1)).eraseDups

/-- The definition of a declared name. -/
def defOf (D : Decls) (n : String) : Ty := (D.lookup n).getD .value

/-- The declared names a declared type refers to. -/
def declaredRefs (D : Decls) (n : String) : List String :=
  (refs (defOf D n)).filter (declaredNames D).contains

/-- The declared names reachable from `n` (including `n`). -/
def reach (D : Decls) (n : String) : List String :=
  let step := fun (seen : List String) => (seen ++ seen.flatMap (declaredRefs D)).eraseDups
  (List.range ((declaredNames D).length + 1)).foldl (fun s _ => step s) [n]

/-- Every name reachable from `n` is declared (Rust: `missing[n]` is `None`). -/
def resolvable (D : Decls) (n : String) : Bool :=
  (reach D n).all fun m => (refs (defOf D m)).all (declaredNames D).contains

/-- The strongly connected group of `n`: the names `n` reaches that reach `n`. -/
def group (D : Decls) (n : String) : List String :=
  (declaredNames D).filter fun m => (reach D n).contains m && (reach D m).contains n

/-- The placeholder base of a member of the group being hashed (Rust `member_base`). -/
def memberBase (name : String) (isRoot : Bool) : String :=
  if isRoot then typePlaceholder else typePlaceholder ++ ":" ++ percentEncode name

/-- The IRI of `root`, given the IRIs of the types outside its group. -/
def hashMember (D : Decls) (known : List (String × String)) (root : String) : String :=
  let g := group D root
  let resolve := fun (name : String) =>
    if g.contains name then some (iriT (memberBase name (name == root)))
    else (known.lookup name).map iriT
  let graph := g.flatMap fun m => writeDefinition resolve (memberBase m (m == root)) m (defOf D m)
  typePrefix ++ Sha256.hex (canonicalOf graph)

/-- One round: every resolvable name without an IRI whose group's outside references all have
IRIs gets its IRI. -/
def irisRound (D : Decls) (known : List (String × String)) : List (String × String) :=
  (declaredNames D).foldl (fun acc n =>
    if (known.lookup n).isSome || !resolvable D n then acc else
    let g := group D n
    let outside := (g.flatMap (declaredRefs D)).filter (!g.contains ·)
    if outside.all fun m => (known.lookup m).isSome then acc ++ [(n, hashMember D known n)]
    else acc) known

/-- The IRIs of the named types of a block that have one, in declaration order (Rust
`TypeIris::of`): groups are hashed after the groups they use, one layer per round. -/
def typeIris (D : Decls) : List (String × String) :=
  let all := (List.range ((declaredNames D).length + 1)).foldl (fun k _ => irisRound D k) []
  (declaredNames D).filterMap fun n => (all.lookup n).map (n, ·)

/-- The published definitions (Rust `TypeIris::definitions`). -/
def typeDefinitions (D : Decls) (iris : List (String × String)) : List Triple :=
  let resolve := fun (name : String) => (iris.lookup name).map iriT
  iris.flatMap fun (name, iri) =>
    match D.lookup name with
    | some t => writeDefinition resolve iri name t
    | none => []

/-! ## SHACL shapes (`shapes.rs`) -/

/-- Rust `DECIMAL_PATTERN`. -/
def decimalPattern : String := r"^-?[0-9]+(\.[0-9]+)?$"

/-- Rust `DATETIME_PATTERN`. -/
def datetimePattern : String :=
  r"^(?:[0-9]{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12][0-9]|3[01])" ++
  r"|(?:0[469]|11)-(?:0[1-9]|[12][0-9]|30)|02-(?:0[1-9]|1[0-9]|2[0-8]))" ++
  r"|(?:[0-9]{2}(?:0[48]|[2468][048]|[13579][26])|(?:[02468][048]|[13579][26])00)-02-29)" ++
  r"[Tt](?:[01][0-9]|2[0-3]):[0-5][0-9]:(?:[0-5][0-9]|60)(?:\.[0-9]+)?" ++
  r"(?:[Zz]|[+-](?:[01][0-9]|2[0-3]):[0-5][0-9])$"

/-- Rust `SHACL_IRI_PATTERN`: the IRI rule in XPath regular expression syntax (`\p{Cc}` for the
control characters, the C1 controls allowed again). -/
def iriPattern : String :=
  "^[A-Za-z][A-Za-z0-9+.\\-]*:(?:[^\\p{Cc} <>\"{}|\\\\^`%]|[\u0080-\u009F]|%[0-9A-Fa-f]{2})*$"

def nodeShape (n : Term) : Triple := ⟨n, rdfType, sh "NodeShape"⟩

/-- An RDF collection of `items` with cells at `<base>#<path>.<i>` (Rust `TypeShapes::list`). -/
def collection (base path : String) (items : List Term) : Term × List Triple :=
  items.zipIdx.foldr (fun (item, i) (rest, ts) =>
    let cell := nodeAt base (path ++ "." ++ toString i)
    (cell, ⟨cell, rdfFirst, item⟩ :: ⟨cell, rdfRest, rest⟩ :: ts)) (rdfNil, [])

/-- A property shape at `<base>#<path>` of `owner` (Rust `TypeShapes::property`). -/
def property (owner : Term) (base path : String) (pred : Term) (min max : Option Nat) :
    Term × List Triple :=
  let p := nodeAt base path
  (p, [⟨owner, sh "property", p⟩, ⟨p, rdfType, sh "PropertyShape"⟩, ⟨p, sh "path", pred⟩] ++
    (match min with | some n => [⟨p, sh "minCount", intLit n⟩] | none => []) ++
    (match max with | some n => [⟨p, sh "maxCount", intLit n⟩] | none => []))

def datatype (n : Term) (dt : String) (pattern : Option String) : List Triple :=
  ⟨n, sh "datatype", iriT dt⟩ :: match pattern with
    | some p => [⟨n, sh "pattern", strLit p⟩]
    | none => []

def closedBlank (n : Term) : List Triple :=
  [⟨n, sh "nodeKind", sh "BlankNode"⟩, ⟨n, sh "closed", boolLit true⟩]

/-- The shape a nested type is checked against (Rust `shape_ref`): a named type's IRI, or a
node shape at `<base>#<path>` with the constraints `cs` of the type at that node. -/
def shapeRef (iris : List (String × String)) (base path : String) (t : Ty) (cs : List Triple) :
    Term × List Triple :=
  let mk := (nodeAt base path, nodeShape (nodeAt base path) :: cs)
  match t with
  | .named n =>
    match iris.lookup n with
    | some i => (iriT i, [])
    | none => mk
  | _ => mk

/-- The constraints of `t` on the shape `node`, nested shapes under `<base>#<path>…` (Rust
`TypeShapes::constrain`). -/
def constrain (D : Decls) (iris : List (String × String)) (node : Term) (base path : String)
    (t : Ty) : List Triple :=
  match t with
  | .value => []
  | .named n =>
    match iris.lookup n with
    | some i => [⟨node, sh "node", iriT i⟩]
    | none =>
      let empty := nodeAt base (path ++ ".unknown")
      [nodeShape empty, ⟨node, sh "not", empty⟩]
  | .prim .bool => datatype node xsdBoolean.val none
  | .prim .int => datatype node xsdLong none
  | .prim .string => datatype node xsdString.val none
  | .prim .datetime => datatype node xsdString.val (some datetimePattern)
  | .prim .iri => datatype node xsdString.val (some iriPattern)
  | .prim .decimal =>
    let alts := [(xsdLong, none), (xsdDecimal, none), (xsdString.val, some decimalPattern)]
    let shapes := alts.zipIdx.map fun ((dt, pat), i) =>
      let a := nodeAt base (path ++ ".d" ++ toString i)
      (a, nodeShape a :: datatype a dt pat)
    let l := collection base (path ++ ".dor") (shapes.map (·.1))
    shapes.flatMap (·.2) ++ l.2 ++ [⟨node, sh "or", l.1⟩]
  | .optional inner =>
    let null := nodeAt base (path ++ ".null")
    let sm := shapeRef iris base (path ++ ".o") inner
      (constrain D iris (nodeAt base (path ++ ".o")) base (path ++ ".o") inner)
    let l := collection base (path ++ ".oor") [null, sm.1]
    [nodeShape null, ⟨null, sh "hasValue", vt "Null"⟩] ++ sm.2 ++ l.2 ++ [⟨node, sh "or", l.1⟩]
  | .list item =>
    let nil := nodeAt base (path ++ ".nil")
    let cons := nodeAt base (path ++ ".cons")
    let first := property cons base (path ++ ".cons.first") rdfFirst (some 1) (some 1)
    let rest := property cons base (path ++ ".cons.rest") rdfRest (some 1) (some 1)
    let m := property cons base (path ++ ".cons.member") (vt "listMember") none none
    let s := shapeRef iris base (path ++ ".l") item
      (constrain D iris (nodeAt base (path ++ ".l")) base (path ++ ".l") item)
    let l := collection base (path ++ ".lor") [nil, cons]
    [nodeShape nil, ⟨nil, sh "hasValue", rdfNil⟩, nodeShape cons] ++ closedBlank cons ++
      first.2 ++ rest.2 ++ m.2 ++ s.2 ++ [⟨m.1, sh "node", s.1⟩] ++ l.2 ++ [⟨node, sh "or", l.1⟩]
  | .record fs =>
    closedBlank node ++ fs.attach.flatMap fun ⟨(name, ft), _⟩ =>
      let fpath := segment path "f" name
      let p := property node base fpath (fieldPredicate name)
        (if optish D ft then none else some 1) (some 1)
      let s := shapeRef iris base (fpath ++ ".t") ft
        (constrain D iris (nodeAt base (fpath ++ ".t")) base (fpath ++ ".t") ft)
      p.2 ++ s.2 ++ [⟨p.1, sh "node", s.1⟩]
  | .union vs =>
    let tag := fieldPredicate "tag"
    let value := fieldPredicate "value"
    let t := property node base (path ++ ".tag") tag (some 1) (some 1)
    let v := property node base (path ++ ".value") value none (some 1)
    let alts := vs.attach.map fun ⟨(vtag, p), _⟩ =>
      let vpath := segment path "v" vtag
      let alt := nodeAt base vpath
      let tp := property alt base (vpath ++ ".tag") tag none none
      let payload : List Triple := match p with
        | none => (property alt base (vpath ++ ".value") value none (some 0)).2
        | some pt =>
          let pp := property alt base (vpath ++ ".value") value
            (if optish D pt then none else some 1) none
          let s := shapeRef iris base (vpath ++ ".t") pt
            (constrain D iris (nodeAt base (vpath ++ ".t")) base (vpath ++ ".t") pt)
          pp.2 ++ s.2 ++ [(⟨pp.1, sh "node", s.1⟩ : Triple)]
      ((alt, nodeShape alt :: tp.2 ++ [(⟨tp.1, sh "hasValue", strLit vtag⟩ : Triple)] ++ payload) :
        Term × List Triple)
    let l := collection base (path ++ ".vor") (alts.map (·.1))
    closedBlank node ++ t.2 ++ [⟨t.1, sh "datatype", iriT xsdString.val⟩] ++ v.2 ++
      alts.flatMap (·.2) ++ l.2 ++ [⟨node, sh "or", l.1⟩]
termination_by sizeOf t
decreasing_by
  all_goals first
    | simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega
    | (rename_i h; have := List.sizeOf_lt_of_mem h
       simp only [Ty.record.sizeOf_spec, Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec,
         Option.some.sizeOf_spec] at this ⊢
       omega)

/-- The node shape of every named type with an IRI, at its IRI (Rust
`TypeShapes::named_types`). -/
def namedTypeShapes (D : Decls) (iris : List (String × String)) : List Triple :=
  iris.flatMap fun (name, iri) =>
    match D.lookup name with
    | some t =>
      [nodeShape (iriT iri), ⟨iriT iri, rdfType, vt "NamedType"⟩] ++
        constrain D iris (iriT iri) iri "s" t
    | none => []

/-! ## The chart's types (`project_chart_types`) -/

/-- The serialized `vt:types` element of a chart: its first extension element `vt:types`
(Rust `declarations_element`). -/
def typesElement (c : Chart) : Option String :=
  c.extensions.findSome? fun e =>
    if e.ns == vtNs && e.localName == "types" then
      match e.value with
      | .element x => some x
      | .attribute _ => none
    else none

/-- The named types of a chart's `vt:types` block, if it has one that parses (Rust
`declarations(chart)` = `Some(Ok(decls))`). -/
def chartDecls (c : Chart) : Option Decls :=
  match typesElement c with
  | none => none
  | some xml =>
    match Syntax.parseTypesElement xml with
    | .ok b => some b.types
    | .error _ => none

/-- The type triples of a chart's projection under `base` (Rust `project_chart_types`):
`vt:declaresType` edges, definitions and shapes of every named type with an IRI. Empty for an
untyped chart, a block that does not parse, or a block without such types. -/
-- @tg: implements:: [[openspec:formal-model#Chart Certificates]] {note: "typed charts: the type triples the chart version covers"}
def chartTypes (c : Chart) (base : String) : List Triple :=
  match chartDecls c with
  | none => []
  | some D =>
    let iris := typeIris D
    let chart := nodeAt base "chart"
    iris.map (fun (_, i) => ⟨chart, vt "declaresType", iriT i⟩) ++ typeDefinitions D iris ++
      namedTypeShapes D iris

end Vertep.Rdf
