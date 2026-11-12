import LeanRDFShacl
import VertepLean.Types.Check
import VertepLean.Types.JsonSchema
import VertepLean.Rdf.Types
import VertepLean.Rdf.RoundTrip.Basic

/-!
# The SHACL projection of types

Mirrors `crates/vertep-rdf/src/types/value.rs` (the RDF encoding of values) and
`crates/vertep-rdf/src/types/shapes.rs` (the SHACL Core shapes of types) on LeanRDF, and proves
that LeanRDF's SHACL Core semantics (`Shacl.Conforms`, which `Shacl.results_nil` ties to its
validator) agrees with the typing relation `HasType`.

* `encGraph v` is the encoding of a value: `vt:Null` for `null` and `undefined`, typed literals
  for booleans, numbers (`xsd:long` when integral and in the signed 64-bit range, else
  `xsd:decimal`) and strings, RDF collections for arrays, and for objects a blank node with one
  `<https://w3id.org/vertep/field#name>` triple per entry that is not `undefined`. Blank nodes
  are labelled by their position in the value.
* `shapeOf D k t` is the node shape of `t` as LeanRDF shapes, component for component as
  `TypeShapes::constrain` writes them. LeanRDF shapes are finite trees (SHACL leaves recursive
  shapes undefined, §3.4, and LeanRDF's reader rejects them), while the Rust shapes graph refers
  to a named type's shape by its IRI, so recursion through named types is unfolded `k` times;
  past that a named type is `sh:not` of the empty shape, which nothing conforms to.

`shacl_agrees`: for blocks and types whose unions have distinct tags and values whose objects
have distinct keys, a value has a type exactly when its encoding conforms to some unfolding of
the type's shape, given `PatternsAgree`: LeanRDF's XPath regular expressions decide the three
`sh:pattern`s of `shapes.rs` as the lexical rules of `Types.Lex` do (checked on samples in the
tests, not proven). The unfoldings are monotone (`conforms_mono`).
-/

namespace Vertep.Types.Shacl

open LeanRDF LeanRDF.Shacl Vertep.Rdf

/-! ## Vocabulary -/

def fieldIri (k : String) : IRI := ⟨fieldNs ++ percentEncode k⟩
def firstIri : IRI := ⟨"http://www.w3.org/1999/02/22-rdf-syntax-ns#first"⟩
def restIri : IRI := ⟨"http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"⟩
def nilT : Term := .iri ⟨"http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"⟩
def nullT : Term := .iri ⟨vtNs ++ "Null"⟩
def longIri : IRI := ⟨xsdLong⟩
def decimalIri : IRI := ⟨xsdDecimal⟩

/-! ## The encoding of values (`value.rs`) -/

/-- The lexical form of the decimal `m × 10^-e`: digits with `e` of them after a `.`. -/
def padded (m : Int) (e : Nat) : List Char :=
  List.replicate (e + 1 - (Xsd.digitsOf m.natAbs).length) '0' ++ Xsd.digitsOf m.natAbs

/-- The digits of `|m| × 10^-e`, with a `.` before the last `e` of them. -/
def decimalBody (m : Int) (e : Nat) : List Char :=
  if e = 0 then padded m e
  else (padded m e).take ((padded m e).length - e) ++ '.' :: (padded m e).drop ((padded m e).length - e)

def decimalChars (m : Int) (e : Nat) : List Char :=
  (if m < 0 then ['-'] else []) ++ decimalBody m e

def boolTerm (b : Bool) : Term := .lit (.typed (if b then "true" else "false") xsdBoolean)

/-- A number: `xsd:long` when it is an integer in the signed 64-bit range (Rust `f64_to_i64`),
otherwise `xsd:decimal`. -/
def numTerm (m : Int) (e : Nat) : Term :=
  if Lex.intOk m e then .lit (.typed (Xsd.canonicalInteger (m / 10 ^ e)) longIri)
  else .lit (.typed (String.ofList (decimalChars m e)) decimalIri)

/-- The blank node label of a position: each index `i` as `i` times `a`, then `b`. -/
def code : List Nat → List Char
  | [] => []
  | i :: p => List.replicate i 'a' ++ 'b' :: code p

def bn (pos : List Nat) : Term := .bnode ⟨String.ofList (code pos)⟩

/-- The term standing for the value `v` at position `pos`. -/
def node (pos : List Nat) : Value → Term
  | .undefined | .null => nullT
  | .bool b => boolTerm b
  | .num m e => numTerm m e
  | .str s => strLit s
  | .array [] => nilT
  | .array (_ :: _) => bn pos
  | .object _ => bn pos

/-- The triples whose subject is the node at `pos`: a collection cell's `rdf:first` and
`rdf:rest`, an object's entries that are not `undefined`. -/
def locals (pos : List Nat) : Value → List Triple
  | .array (x :: xs) =>
    [⟨bn pos, .iri firstIri, node (pos ++ [0]) x⟩, ⟨bn pos, .iri restIri, node (pos ++ [1]) (.array xs)⟩]
  | .object es =>
    es.zipIdx.flatMap fun ((k, x), i) =>
      if isUndef x then [] else [⟨bn pos, .iri (fieldIri k), node (pos ++ [i]) x⟩]
  | _ => []

/-- The value at a relative position: `0` and `1` step to a cell's item and the rest of the
collection, `i` to an object's `i`-th entry unless it is `undefined`. -/
def sub : Value → List Nat → Option Value
  | v, [] => some v
  | .array (x :: _), 0 :: r => sub x r
  | .array (_ :: xs), 1 :: r => sub (.array xs) r
  | .object es, i :: r =>
    match es[i]? with
    | some (_, x) => if isUndef x then none else sub x r
    | none => none
  | _, _ => none

/-- The encoding of `v` at `pos`: the local triples of every position below it. -/
def encT (pos : List Nat) (v : Value) : List Triple :=
  match v with
  | .array (x :: xs) => locals pos v ++ encT (pos ++ [0]) x ++ encT (pos ++ [1]) (.array xs)
  | .object es =>
    locals pos v ++ es.zipIdx.attach.flatMap fun ⟨((_, x), i), _hm⟩ =>
      if isUndef x then [] else encT (pos ++ [i]) x
  | _ => []
termination_by sizeOf v
decreasing_by
  · simp only [Value.array.sizeOf_spec, List.cons.sizeOf_spec]; omega
  · simp only [Value.array.sizeOf_spec, List.cons.sizeOf_spec]; omega
  · have := List.sizeOf_lt_of_mem (List.fst_mem_of_mem_zipIdx _hm)
    simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢; omega

/-- The RDF encoding of a value (Rust `value_graph`, without its root triple). -/
-- @tg: implements:: [[openspec:formal-model#SHACL Projection Agreement]]
def encGraph (v : Value) : Graph := Graph.ofList (encT [] v)

/-- The node standing for the whole value. -/
def encNode (v : Value) : Term := node [] v

/-- Objects have distinct keys, at every depth. -/
def keysOk (v : Value) : Bool :=
  match v with
  | .array items => items.attach.all fun ⟨x, _⟩ => keysOk x
  | .object es => decide (es.map (·.1)).Nodup && es.attach.all fun ⟨e, _⟩ => keysOk e.2
  | _ => true
termination_by sizeOf v
decreasing_by
  · rename_i h; have := List.sizeOf_lt_of_mem h
    simp only [Value.array.sizeOf_spec]; omega
  · rename_i h; have := List.sizeOf_lt_of_mem h
    obtain ⟨k, x⟩ := e
    simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢; omega

/-! ## The shapes (`shapes.rs`) -/

/-- A shape without targets, node shape when `path` is `none`. -/
def mkShape (path : Option Path) (comps : List Component) : Shape :=
  .mk (.iri ⟨"urn:vertep:value-shape"⟩) path [] comps false (.iri sh.Violation) []

def nodeS (comps : List Component) : Shape := mkShape none comps
def propS (p : IRI) (comps : List Component) : Shape := mkShape (some (.pred p)) comps

/-- `rdf:rest*/rdf:first`: the members of a collection. -/
def membersPath : Path := .seq [.zeroOrMore (.pred restIri), .pred firstIri]

/-- A closed blank node with these predicates (Rust `closed_blank` and the property paths). -/
def closedBlank (allowed : List IRI) : List Component := [.test (.nodeKind .bnode), .closed allowed]

/-- At most one value on `p`, and at least one unless `opt`. -/
def counts (opt : Bool) : List Component := (if opt then [] else [.minCount 1]) ++ [.maxCount 1]

/-- The components of the node shape of `t`, named types unfolded `k` times (Rust
`TypeShapes::constrain`). -/
-- @tg: implements:: [[openspec:formal-model#SHACL Projection Agreement]]
def comps (D : Decls) (k : Nat) (t : Ty) : List Component :=
  match t with
  | .value => []
  | .named n =>
    match k with
    | 0 => [.not (nodeS [])]
    | k + 1 =>
      match D.lookup n with
      | none => [.not (nodeS [])]
      | some u => [.node (nodeS (comps D k u))]
  | .prim .bool => [.test (.datatype xsdBoolean)]
  | .prim .int => [.test (.datatype longIri)]
  | .prim .string => [.test (.datatype xsdString)]
  | .prim .datetime => [.test (.datatype xsdString), .test (.pattern datetimePattern "")]
  | .prim .iri => [.test (.datatype xsdString), .test (.pattern iriPattern "")]
  | .prim .decimal =>
    [.or [nodeS [.test (.datatype longIri)], nodeS [.test (.datatype decimalIri)],
          nodeS [.test (.datatype xsdString), .test (.pattern decimalPattern "")]]]
  | .optional t => [.or [nodeS [.hasValue nullT], nodeS (comps D k t)]]
  | .list t =>
    [.or [nodeS [.hasValue nilT],
          nodeS (closedBlank [firstIri, restIri] ++
            [.property (propS firstIri (counts false)), .property (propS restIri (counts false))])],
     .property (mkShape (some membersPath) [.node (nodeS (comps D k t))])]
  | .record fs =>
    closedBlank (fs.map fun f => fieldIri f.1) ++ fs.attach.map fun ⟨(name, ft), _⟩ =>
      .property (propS (fieldIri name) (counts (optish D ft) ++ [.node (nodeS (comps D k ft))]))
  | .union vs =>
    closedBlank [fieldIri "tag", fieldIri "value"] ++
      [.property (propS (fieldIri "tag") (counts false ++ [.test (.datatype xsdString)])),
       .property (propS (fieldIri "value") [.maxCount 1]),
       .or (vs.attach.map fun ⟨(tag, p), _⟩ =>
         nodeS (.property (propS (fieldIri "tag") [.hasValue (strLit tag)]) ::
           match p with
           | none => [.property (propS (fieldIri "value") [.maxCount 0])]
           | some pt =>
             [.property (propS (fieldIri "value")
               ((if optish D pt then [] else [.minCount 1]) ++ [.node (nodeS (comps D k pt))]))]))]
termination_by (k, sizeOf t)
decreasing_by
  all_goals first
    | exact Prod.Lex.left _ _ (by omega)
    | (apply Prod.Lex.right; simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega)
    | (apply Prod.Lex.right; rename_i h; have := List.sizeOf_lt_of_mem h
       simp only [Ty.record.sizeOf_spec, Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec,
         Option.some.sizeOf_spec] at this ⊢
       omega)

/-- The node shape of `t`, named types unfolded `k` times. -/
def shapeOf (D : Decls) (k : Nat) (t : Ty) : Shape := nodeS (comps D k t)

/-- LeanRDF's XPath regular expressions decide the `sh:pattern`s of `shapes.rs` as the lexical
rules of the checker do. -/
def PatternsAgree : Prop :=
  ∀ s : String,
    (TermTest.pattern decimalPattern "").ok (strLit s) = Lex.isDecimal s ∧
    (TermTest.pattern datetimePattern "").ok (strLit s) = Lex.isDatetime s ∧
    (TermTest.pattern iriPattern "").ok (strLit s) = Lex.isIri s

/-! ## The checker with unfolding fuel -/

/-- `check` with a bound `k` on the named types expanded along any path, instead of the names
under expansion at the current position. -/
def checkF (D : Decls) (k : Nat) (t : Ty) (v : Value) : Bool :=
  match t with
  | .value => true
  | .prim p => primOk p v
  | .named n =>
    match k with
    | 0 => false
    | k + 1 =>
      match D.lookup n with
      | none => false
      | some u => checkF D k u v
  | .optional t₀ => absent v || checkF D k t₀ v
  | .list t₀ =>
    match v with
    | .array items => items.all fun x => checkF D k t₀ x
    | _ => false
  | .record fs =>
    match v with
    | .object entries =>
      entries.all (fun e => isUndef e.2 || fs.any (·.1 == e.1)) &&
      fs.attach.all fun ⟨f, _⟩ =>
        match present entries f.1 with
        | none => optish D f.2
        | some fv => checkF D k f.2 fv
    | _ => false
  | .union vs =>
    match v with
    | .object entries =>
      match entries.lookup "tag" with
      | some (.str tag) =>
        entries.all (fun e => e.1 == "tag" || (e.1 == "value" || isUndef e.2)) &&
        match _h : vs.lookup tag with
        | none => false
        | some none => (present entries "value").isNone
        | some (some pt) =>
          match present entries "value" with
          | none => optish D pt
          | some pv => checkF D k pt pv
      | _ => false
    | _ => false
termination_by (k, sizeOf t)
decreasing_by
  all_goals first
    | exact Prod.Lex.left _ _ (by omega)
    | (apply Prod.Lex.right; simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega)
    | (apply Prod.Lex.right; rename_i h; have := List.sizeOf_lt_of_mem h
       obtain ⟨n, ft⟩ := f
       simp only [Ty.record.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢
       omega)
    | (apply Prod.Lex.right
       have := List.sizeOf_lt_of_mem (lookup_mem' _h)
       simp only [Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec, Option.some.sizeOf_spec] at this ⊢
       omega)

/-! ## SHACL semantics of the projected components -/

section Sem

variable (G : Graph)

theorem compsHold_nil (path : Option Path) (x : Term) : CompsHold G path [] x ↔ True := by
  simp [CompsHold]

theorem compsHold_cons (path : Option Path) (c : Component) (cs : List Component) (x : Term) :
    CompsHold G path (c :: cs) x ↔ CompHolds G path c x ∧ CompsHold G path cs x := by
  simp [CompsHold]

theorem compsHold_append (path : Option Path) (a b : List Component) (x : Term) :
    CompsHold G path (a ++ b) x ↔ CompsHold G path a x ∧ CompsHold G path b x := by
  induction a with
  | nil => simp [CompsHold]
  | cons c a ih => simp only [List.cons_append, compsHold_cons, ih, and_assoc]

theorem conforms_mkShape (path : Option Path) (cs : List Component) (x : Term) :
    Conforms G (mkShape path cs) x ↔ CompsHold G path cs x := by
  simp [mkShape, Conforms]

theorem conforms_nodeS (cs : List Component) (x : Term) :
    Conforms G (nodeS cs) x ↔ CompsHold G none cs x := conforms_mkShape G none cs x

theorem conforms_propS (p : IRI) (cs : List Component) (x : Term) :
    Conforms G (propS p cs) x ↔ CompsHold G (some (.pred p)) cs x := conforms_mkShape G _ cs x

theorem someConforms_iff (ss : List Shape) (x : Term) :
    SomeConforms G ss x ↔ ∃ s ∈ ss, Conforms G s x := by
  induction ss with
  | nil => simp [SomeConforms]
  | cons s ss ih => simp [SomeConforms, ih]

theorem holds_test (tt : TermTest) (x : Term) :
    CompHolds G none (.test tt) x ↔ tt.ok x = true := by simp [CompHolds, VN]

theorem holds_node (s : Shape) (x : Term) :
    CompHolds G none (.node s) x ↔ Conforms G s x := by simp [CompHolds, VN]

theorem holds_property (s : Shape) (x : Term) :
    CompHolds G none (.property s) x ↔ Conforms G s x := by simp [CompHolds, VN]

theorem holds_not (s : Shape) (x : Term) :
    CompHolds G none (.not s) x ↔ ¬ Conforms G s x := by simp [CompHolds, VN]

theorem holds_hasValue (t x : Term) : CompHolds G none (.hasValue t) x ↔ x = t := by
  simp [CompHolds, VN, eq_comm]

theorem holds_or (ss : List Shape) (x : Term) :
    CompHolds G none (.or ss) x ↔ ∃ s ∈ ss, Conforms G s x := by
  simp [CompHolds, VN, someConforms_iff]

theorem holds_closed (allowed : List IRI) (x : Term) :
    CompHolds G none (.closed allowed) x ↔
      ∀ p o, (⟨x, p, o⟩ : Triple) ∈ G → ∃ q ∈ allowed, p = .iri q := by
  simp [CompHolds, VN]

theorem vn_pred (p : IRI) (x y : Term) : VN G (some (.pred p)) x y ↔ (⟨x, .iri p, y⟩ : Triple) ∈ G :=
  pred_iff G

theorem holds_minCount1 (p : IRI) (x : Term) :
    CompHolds G (some (.pred p)) (.minCount 1) x ↔ ∃ y, (⟨x, .iri p, y⟩ : Triple) ∈ G := by
  simp only [CompHolds, vn_pred]
  constructor
  · rintro ⟨l, _, hl, hn⟩
    cases l with
    | nil => simp at hn
    | cons y l => exact ⟨y, hl y (List.mem_cons_self ..)⟩
  · rintro ⟨y, hy⟩
    exact ⟨[y], List.nodup_cons.2 ⟨List.not_mem_nil, List.nodup_nil⟩,
      fun z hz => by simp at hz; subst hz; exact hy, by simp⟩

theorem holds_maxCount1 (p : IRI) (x : Term) :
    CompHolds G (some (.pred p)) (.maxCount 1) x ↔
      ∀ y z, (⟨x, .iri p, y⟩ : Triple) ∈ G → (⟨x, .iri p, z⟩ : Triple) ∈ G → y = z := by
  simp only [CompHolds, vn_pred]
  constructor
  · intro h y z hy hz
    by_cases hne : y = z
    · exact hne
    · have := h [y, z] (by simp [hne]) (by
        intro w hw; simp at hw; rcases hw with rfl | rfl <;> assumption)
      simp at this
  · intro h l hl hm
    match l, hl, hm with
    | [], _, _ => simp
    | [_], _, _ => simp
    | a :: b :: _, hl, hm =>
      have := h a b (hm a (by simp)) (hm b (by simp))
      subst this; simp at hl

theorem holds_maxCount0 (p : IRI) (x : Term) :
    CompHolds G (some (.pred p)) (.maxCount 0) x ↔
      ∀ y, (⟨x, .iri p, y⟩ : Triple) ∉ G := by
  simp only [CompHolds, vn_pred]
  constructor
  · intro h y hy
    have := h [y] (by simp) (by intro w hw; simp at hw; subst hw; exact hy)
    simp at this
  · intro h l _ hm
    match l, hm with
    | [], _ => simp
    | a :: _, hm => exact absurd (hm a (by simp)) (h a)

theorem holds_pnode (p : IRI) (s : Shape) (x : Term) :
    CompHolds G (some (.pred p)) (.node s) x ↔
      ∀ y, (⟨x, .iri p, y⟩ : Triple) ∈ G → Conforms G s y := by
  simp only [CompHolds, vn_pred]

theorem holds_phasValue (p : IRI) (t x : Term) :
    CompHolds G (some (.pred p)) (.hasValue t) x ↔ (⟨x, .iri p, t⟩ : Triple) ∈ G := by
  simp only [CompHolds, vn_pred]

theorem holds_ptest (p : IRI) (tt : TermTest) (x : Term) :
    CompHolds G (some (.pred p)) (.test tt) x ↔
      ∀ y, (⟨x, .iri p, y⟩ : Triple) ∈ G → tt.ok y = true := by
  simp only [CompHolds, vn_pred]

end Sem

/-! ## The literals of the encoding pass their datatype tests -/

theorem localName_string : Xsd.localName? xsdString = some "string" := by decide +kernel
theorem localName_boolean : Xsd.localName? xsdBoolean = some "boolean" := by decide +kernel
theorem localName_long : Xsd.localName? longIri = some "long" := by decide +kernel
theorem localName_decimal : Xsd.localName? decimalIri = some "decimal" := by decide +kernel
theorem find_long : Xsd.integerTypes.find? (·.1 == "long") =
    some ("long", some (-9223372036854775808), some 9223372036854775807) := by decide +kernel

theorem datatype_ok_iff (d : IRI) (lex : String) (dt : IRI) :
    (TermTest.datatype d).ok (.lit (.typed lex dt)) = true ↔
      dt = d ∧ (recognized d = false ∨ (Literal.typed lex dt).value?.isSome = true) := by
  simp [TermTest.ok, Literal.datatype]

theorem str_ok (s : String) : (TermTest.datatype xsdString).ok (strLit s) = true := by
  rw [strLit, datatype_ok_iff]
  refine ⟨rfl, .inr ?_⟩
  simp [Literal.value?, localName_string]

theorem bool_ok (b : Bool) : (TermTest.datatype xsdBoolean).ok (boolTerm b) = true := by
  cases b <;> decide +kernel

theorem pow10_pos (e : Nat) : (0 : Int) < 10 ^ e := by
  have : 0 < (10 : Nat) ^ e := Nat.pow_pos (by decide)
  exact_mod_cast this

theorem long_ok {m : Int} {e : Nat} (h : Lex.intOk m e = true) :
    (TermTest.datatype longIri).ok (numTerm m e) = true := by
  rw [numTerm, ite_eq_left h, datatype_ok_iff]
  refine ⟨rfl, .inr ?_⟩
  simp only [Lex.intOk, Bool.and_eq_true, beq_iff_eq, decide_eq_true_eq] at h
  obtain ⟨⟨hd, hlo⟩, hhi⟩ := h
  have hp := pow10_pos e
  have hm : m / 10 ^ e * 10 ^ e = m := Int.ediv_mul_cancel (Int.dvd_of_emod_eq_zero hd)
  have h1 : -9223372036854775808 ≤ m / 10 ^ e := by
    have : (-9223372036854775808 : Int) * 10 ^ e ≤ m / 10 ^ e * 10 ^ e := by
      rw [hm]; simpa using hlo
    exact Int.le_of_mul_le_mul_right this hp
  have h2 : m / 10 ^ e ≤ 9223372036854775807 := by
    have : m / 10 ^ e * 10 ^ e < 9223372036854775808 * 10 ^ e := by
      rw [hm]; simpa using hhi
    have := Int.lt_of_mul_lt_mul_right this (Int.le_of_lt hp)
    omega
  simp only [Literal.value?, localName_long]
  split
  all_goals first | (rename_i h; simp at h; done) | skip
  rename_i heq
  simp only [Option.some.injEq] at heq
  subst heq
  rw [find_long]
  have h3 : ¬ (m / 10 ^ e < -9223372036854775808) := by omega
  have h4 : ¬ (m / 10 ^ e > 9223372036854775807) := by omega
  simp [Xsd.parseInteger_canonical, h3, h4]

theorem isWS_ne {c : Char} (h : c = '-' ∨ c = '.' ∨ c.isDigit = true) : Xsd.isWS c = false := by
  rcases h with rfl | rfl | h
  · decide
  · decide
  · exact Xsd.digit_not_ws h

theorem padded_digits (m : Int) (e : Nat) : ∀ c ∈ padded m e, c.isDigit = true := by
  intro c hc
  rcases List.mem_append.1 hc with hc | hc
  · rw [List.eq_of_mem_replicate hc]; decide
  · exact Xsd.digitsOf_isDigit _ c hc

theorem padded_length (m : Int) (e : Nat) : e + 1 ≤ (padded m e).length := by
  simp only [padded, List.length_append, List.length_replicate]; omega

theorem parseDigits_padded (m : Int) (e : Nat) :
    Xsd.parseDigits (padded m e) = some m.natAbs := by
  rw [padded, Xsd.parseDigits_zeros, Xsd.parseDigits_digitsOf]

theorem take_padded_ne (m : Int) (e : Nat) :
    (padded m e).take ((padded m e).length - e) ≠ [] := by
  have := padded_length m e
  intro ht
  have := congrArg List.length ht
  simp only [List.length_take, List.length_nil] at this
  omega

theorem decimalBody_chars (m : Int) (e : Nat) :
    ∀ c ∈ decimalBody m e, c = '-' ∨ c = '.' ∨ c.isDigit = true := by
  intro c hc
  unfold decimalBody at hc
  split at hc
  · exact .inr (.inr (padded_digits m e c hc))
  · rcases List.mem_append.1 hc with hc | hc
    · exact .inr (.inr (padded_digits m e c (List.mem_of_mem_take hc)))
    · rcases List.mem_cons.1 hc with rfl | hc
      · exact .inr (.inl rfl)
      · exact .inr (.inr (padded_digits m e c (List.mem_of_mem_drop hc)))

theorem decimalBody_head (m : Int) (e : Nat) :
    ∃ c rest, decimalBody m e = c :: rest ∧ c.isDigit = true := by
  unfold decimalBody
  split
  · cases hp : padded m e with
    | nil => have := padded_length m e; rw [hp] at this; simp at this
    | cons c rest => exact ⟨c, rest, rfl, padded_digits m e c (by rw [hp]; simp)⟩
  · cases ht : (padded m e).take ((padded m e).length - e) with
    | nil => exact absurd ht (take_padded_ne m e)
    | cons c rest =>
      exact ⟨c, rest ++ '.' :: (padded m e).drop ((padded m e).length - e), rfl,
        padded_digits m e c (List.mem_of_mem_take (by rw [ht]; simp))⟩

theorem signSplit_decimal (m : Int) (e : Nat) :
    Xsd.signSplit (decimalChars m e) = (decide (m < 0), decimalBody m e) := by
  unfold decimalChars
  by_cases hm : m < 0
  · simp [hm, Xsd.signSplit]
  · obtain ⟨c, rest, hb, hc⟩ := decimalBody_head m e
    simp only [hm, ↓reduceIte, List.nil_append, decide_false]
    rw [hb, Xsd.signSplit_digit rest hc]

theorem trimWS_decimal (m : Int) (e : Nat) : Xsd.trimWS (decimalChars m e) = decimalChars m e := by
  apply Xsd.trimWS_of_noWS
  intro c hc
  apply isWS_ne
  unfold decimalChars at hc
  rcases List.mem_append.1 hc with hc | hc
  · split at hc
    · simp at hc; exact .inl hc
    · simp at hc
  · exact decimalBody_chars m e c hc

theorem parseDecimal_isSome (m : Int) (e : Nat) :
    (Xsd.parseDecimalChars (decimalChars m e)).isSome = true := by
  unfold Xsd.parseDecimalChars
  rw [trimWS_decimal, signSplit_decimal]
  simp only []
  unfold decimalBody
  by_cases he : e = 0
  · rw [ite_eq_left he, Xsd.splitDot_digits (padded_digits m e)]
    have : (padded m e).isEmpty = false := by
      cases hp : padded m e with
      | nil => have := padded_length m e; rw [hp] at this; simp at this
      | cons => rfl
    simp [this, parseDigits_padded]
  · rw [ite_eq_right he, Xsd.splitDot_append _ (fun c hc => padded_digits m e c (List.mem_of_mem_take hc))]
    have : ((padded m e).take ((padded m e).length - e)).isEmpty = false := by
      cases ht : (padded m e).take ((padded m e).length - e) with
      | nil => exact absurd ht (take_padded_ne m e)
      | cons => rfl
    simp [this, List.take_append_drop, parseDigits_padded]

theorem decimal_ok {m : Int} {e : Nat} (h : Lex.intOk m e = false) :
    (TermTest.datatype decimalIri).ok (numTerm m e) = true := by
  rw [numTerm, ite_eq_right (by simp [h]), datatype_ok_iff]
  refine ⟨rfl, .inr ?_⟩
  simp only [Literal.value?, localName_decimal, Xsd.parseDecimal, String.toList_ofList,
    Option.isSome_map]
  exact parseDecimal_isSome m e

/-! ## Graphs that encode a value -/

/-- `Enc G v n`: in `G`, the node `n` stands for the value `v`: literals and `vt:Null` as in
`numTerm`, `boolTerm`, `strLit`; a collection cell's only triples are its `rdf:first` and
`rdf:rest`; an object's node has exactly one triple per entry that is not `undefined`. -/
inductive Enc (G : Graph) : Value → Term → Prop where
  | undef : Enc G .undefined nullT
  | null : Enc G .null nullT
  | bool (b : Bool) : Enc G (.bool b) (boolTerm b)
  | num (m : Int) (e : Nat) : Enc G (.num m e) (numTerm m e)
  | str (s : String) : Enc G (.str s) (strLit s)
  | nil : Enc G (.array []) nilT
  | cons (x : Value) (xs : List Value) (b : BlankNode) (nx nr : Term) :
      (∀ p o, (⟨.bnode b, p, o⟩ : Triple) ∈ G ↔
        (p = .iri firstIri ∧ o = nx) ∨ (p = .iri restIri ∧ o = nr)) →
      Enc G x nx → Enc G (.array xs) nr → Enc G (.array (x :: xs)) (.bnode b)
  | object (es : List (String × Value)) (b : BlankNode) (f : String → Term) :
      (∀ p o, (⟨.bnode b, p, o⟩ : Triple) ∈ G ↔
        ∃ k x, present es k = some x ∧ p = .iri (fieldIri k) ∧ o = f k) →
      (∀ k x, present es k = some x → Enc G x (f k)) →
      Enc G (.object es) (.bnode b)

/-- Every subject of the graph is a blank node. -/
def BNodeSubjects (G : Graph) : Prop := ∀ t ∈ G, ∃ b, t.s = .bnode b

/-! ## What an encoded node reveals -/

theorem fieldIri_inj {k k' : String} : fieldIri k = fieldIri k' ↔ k = k' := by
  constructor
  · intro h
    simp only [fieldIri, IRI.mk.injEq] at h
    have h' := congrArg String.toList h
    simp only [String.toList_append, List.append_cancel_left_eq] at h'
    exact Vertep.Rdf.RT.percentEncode_inj (String.toList_inj.1 h')
  · rintro rfl; rfl

theorem fieldIri_ne_first (k : String) : fieldIri k ≠ firstIri := by
  simp [fieldIri, firstIri, fieldNs, ← String.toList_inj, String.toList_append]

theorem fieldIri_ne_rest (k : String) : fieldIri k ≠ restIri := by
  simp [fieldIri, restIri, fieldNs, ← String.toList_inj, String.toList_append]

@[simp] theorem dt_ok_iri (d : IRI) (i : IRI) : (TermTest.datatype d).ok (.iri i) = false := rfl
@[simp] theorem dt_ok_bnode (d : IRI) (b : BlankNode) : (TermTest.datatype d).ok (.bnode b) = false :=
  rfl

theorem iri_ne_1 : xsdBoolean ≠ xsdString := by decide
theorem iri_ne_2 : longIri ≠ xsdString := by decide
theorem iri_ne_3 : decimalIri ≠ xsdString := by decide
theorem iri_ne_4 : xsdString ≠ xsdBoolean := by decide
theorem iri_ne_5 : longIri ≠ xsdBoolean := by decide
theorem iri_ne_6 : decimalIri ≠ xsdBoolean := by decide
theorem iri_ne_7 : xsdString ≠ longIri := by decide
theorem iri_ne_8 : xsdBoolean ≠ longIri := by decide
theorem iri_ne_9 : decimalIri ≠ longIri := by decide
theorem iri_ne_10 : xsdString ≠ decimalIri := by decide
theorem iri_ne_11 : xsdBoolean ≠ decimalIri := by decide
theorem iri_ne_12 : longIri ≠ decimalIri := by decide
theorem nullT_ne_nilT : nullT ≠ nilT := by decide

section EncFacts

variable {G : Graph}

/-- The datatype test of a typed literal. -/
theorem dt_ok_lit (d dt : IRI) (lex : String) :
    (TermTest.datatype d).ok (.lit (.typed lex dt)) = true ↔
      dt = d ∧ (recognized d = false ∨ (Literal.typed lex dt).value?.isSome = true) :=
  datatype_ok_iff d lex dt

theorem enc_string {v : Value} {n : Term} (h : Enc G v n) :
    (TermTest.datatype xsdString).ok n = true ↔ ∃ s, v = .str s := by
  cases h with
  | str s => simp [str_ok]
  | bool b => simp [boolTerm, dt_ok_lit, iri_ne_1]
  | num m e =>
    unfold numTerm; split <;> simp [dt_ok_lit, iri_ne_2, iri_ne_3]
  | _ => simp [nullT, nilT]

theorem enc_bool {v : Value} {n : Term} (h : Enc G v n) :
    (TermTest.datatype xsdBoolean).ok n = true ↔ ∃ b, v = .bool b := by
  cases h with
  | bool b => simp [bool_ok]
  | str s => simp [strLit, dt_ok_lit, iri_ne_4]
  | num m e =>
    unfold numTerm; split <;> simp [dt_ok_lit, iri_ne_5, iri_ne_6]
  | _ => simp [nullT, nilT]

theorem enc_long {v : Value} {n : Term} (h : Enc G v n) :
    (TermTest.datatype longIri).ok n = true ↔ ∃ m e, v = .num m e ∧ Lex.intOk m e = true := by
  cases h with
  | num m e =>
    by_cases hi : Lex.intOk m e = true
    · simp only [long_ok hi, true_iff]; exact ⟨m, e, rfl, hi⟩
    · have : (TermTest.datatype longIri).ok (numTerm m e) = false := by
        unfold numTerm; rw [ite_eq_right hi]; simp [TermTest.ok, Literal.datatype, iri_ne_9]
      simp [this, hi]
  | bool b => simp [boolTerm, dt_ok_lit, iri_ne_8]
  | str s => simp [strLit, dt_ok_lit, iri_ne_7]
  | _ => simp [nullT, nilT]

theorem enc_decimal {v : Value} {n : Term} (h : Enc G v n) :
    (TermTest.datatype decimalIri).ok n = true ↔ ∃ m e, v = .num m e ∧ Lex.intOk m e = false := by
  cases h with
  | num m e =>
    by_cases hi : Lex.intOk m e = true
    · have : (TermTest.datatype decimalIri).ok (numTerm m e) = false := by
        unfold numTerm; rw [ite_eq_left hi]; simp [TermTest.ok, Literal.datatype, iri_ne_12]
      simp [this, hi]
    · have hf : Lex.intOk m e = false := by simpa using hi
      simp only [decimal_ok hf, true_iff]; exact ⟨m, e, rfl, hf⟩
  | bool b => simp [boolTerm, dt_ok_lit, iri_ne_11]
  | str s => simp [strLit, dt_ok_lit, iri_ne_10]
  | _ => simp [nullT, nilT]

theorem enc_bnode {v : Value} {n : Term} (h : Enc G v n) :
    NodeKind.bnode.holds n = true ↔
      (∃ es, v = .object es) ∨ ∃ x xs, v = .array (x :: xs) := by
  cases h with
  | num m e => unfold numTerm; split <;> simp [NodeKind.holds]
  | _ => simp [NodeKind.holds, nullT, nilT, boolTerm, strLit]

theorem enc_null {v : Value} {n : Term} (h : Enc G v n) : n = nullT ↔ absent v = true := by
  cases h with
  | num m e => unfold numTerm; split <;> simp [nullT, absent]
  | nil => simp [absent, Ne.symm nullT_ne_nilT]
  | _ => simp [absent, nullT, boolTerm, strLit]

theorem enc_nil {v : Value} {n : Term} (h : Enc G v n) : n = nilT ↔ v = .array [] := by
  cases h with
  | num m e => unfold numTerm; split <;> simp [nilT]
  | undef => simp [nullT_ne_nilT]
  | null => simp [nullT_ne_nilT]
  | _ => simp [nilT, boolTerm, strLit]

theorem enc_strLit {v : Value} {s : String} (h : Enc G v (strLit s)) : v = .str s := by
  have := (enc_string h).1 (str_ok s)
  obtain ⟨s', rfl⟩ := this
  cases h
  rfl

end EncFacts

/-! ## Collections and objects in an encoding graph -/

section Graphs

variable {G : Graph}

theorem compsHold_iff (path : Option Path) (cs : List Component) (x : Term) :
    CompsHold G path cs x ↔ ∀ c ∈ cs, CompHolds G path c x := by
  induction cs with
  | nil => simp [CompsHold]
  | cons c cs ih => simp [CompsHold, ih]

theorem no_nilT_subject (hG : BNodeSubjects G) (p o : Term) : (⟨nilT, p, o⟩ : Triple) ∉ G := by
  intro h
  obtain ⟨b, hb⟩ := hG _ h
  simp [nilT] at hb

theorem star_rest (hG : BNodeSubjects G) {items : List Value} {n z : Term}
    (h : Enc G (.array items) n) (hz : ValueNode G (.zeroOrMore (.pred restIri)) n z) :
    ∃ j, Enc G (.array (items.drop j)) z := by
  generalize hp : Path.zeroOrMore (.pred restIri) = P at hz
  revert h
  induction hz with
  | starRefl => intro h; exact ⟨0, by simpa using h⟩
  | starTail h1 h2 ih =>
    intro h
    cases hp
    obtain ⟨j, hj⟩ := ih rfl h
    rw [pred_iff] at h2
    generalize hd : items.drop j = l at hj
    cases hj with
    | nil => exact absurd h2 (no_nilT_subject hG _ _)
    | cons x xs b nx nr hiff _ hr =>
      have := (hiff _ _).1 h2
      rcases this with ⟨he, _⟩ | ⟨_, rfl⟩
      · simp [firstIri, restIri] at he
      · refine ⟨j + 1, ?_⟩
        rw [← List.drop_drop, hd]
        simpa using hr
  | _ => cases hp

theorem rest_star {items : List Value} {n : Term} (h : Enc G (.array items) n) :
    ∀ j, j ≤ items.length → ∃ z, ValueNode G (.zeroOrMore (.pred restIri)) n z ∧
      Enc G (.array (items.drop j)) z := by
  intro j
  induction j with
  | zero => intro _; exact ⟨n, .starRefl, by simpa using h⟩
  | succ j ih =>
    intro hj
    obtain ⟨z, hz, he⟩ := ih (by omega)
    generalize hd : items.drop j = l at he
    cases he with
    | nil =>
      have : (items.drop j).length = 0 := by rw [hd]; rfl
      simp at this; omega
    | cons x xs b nx nr hiff _ hr =>
      refine ⟨nr, .starTail hz ((pred_iff G).2 ((hiff _ _).2 (.inr ⟨rfl, rfl⟩))), ?_⟩
      rw [← List.drop_drop, hd]
      simpa using hr

theorem member_enc (hG : BNodeSubjects G) {items : List Value} {n y : Term}
    (h : Enc G (.array items) n) (hy : ValueNode G membersPath n y) : ∃ x ∈ items, Enc G x y := by
  unfold membersPath at hy
  cases hy with
  | seqCons hz hy =>
    cases hy with
    | seqCons hf hn =>
      cases hn
      obtain ⟨j, hj⟩ := star_rest hG h hz
      rw [pred_iff] at hf
      generalize hd : items.drop j = l at hj
      cases hj with
      | nil => exact absurd hf (no_nilT_subject hG _ _)
      | cons x xs b nx nr hiff hx _ =>
        rcases (hiff _ _).1 hf with ⟨_, rfl⟩ | ⟨he, _⟩
        · refine ⟨x, ?_, hx⟩
          exact List.mem_of_mem_drop (by rw [hd]; simp)
        · simp [firstIri, restIri] at he

theorem enc_member {items : List Value} {n : Term} (h : Enc G (.array items) n) {x : Value}
    (hx : x ∈ items) : ∃ y, ValueNode G membersPath n y ∧ Enc G x y := by
  obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hx
  obtain ⟨z, hz, he⟩ := rest_star h j (by omega)
  rw [List.drop_eq_getElem_cons hj] at he
  cases he with
  | cons _ _ b nx nr hiff hx' _ =>
    exact ⟨nx, .seqCons hz (.seqCons ((pred_iff G).2 ((hiff _ _).2 (.inl ⟨rfl, rfl⟩))) .seqNil), hx'⟩

/-- An object's entry is present exactly when it is in the entries and not `undefined`, when the
keys are distinct. -/
theorem present_some_mem {es : List (String × Value)} {k : String} {x : Value}
    (h : present es k = some x) : (k, x) ∈ es ∧ isUndef x = false := by
  unfold present at h
  cases hl : es.lookup k with
  | none => rw [hl] at h; cases h
  | some y =>
    rw [hl] at h
    cases y <;> (simp only at h) <;> cases h <;> exact ⟨lookup_mem' hl, rfl⟩

theorem present_of_mem {es : List (String × Value)} (hnd : (es.map (·.1)).Nodup) {k : String}
    {x : Value} (hm : (k, x) ∈ es) (hu : isUndef x = false) : present es k = some x := by
  unfold present
  rw [JsonSchema.lookup_of_mem hnd hm]
  cases x <;> simp_all [isUndef]

theorem keysOk_object {es : List (String × Value)} (h : keysOk (.object es) = true) :
    (es.map (·.1)).Nodup ∧ ∀ e ∈ es, keysOk e.2 = true := by
  rw [keysOk] at h
  simp only [Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true] at h
  exact ⟨h.1, fun e he => h.2 ⟨e, he⟩ (List.mem_attach _ _)⟩

theorem keysOk_array {items : List Value} (h : keysOk (.array items) = true) :
    ∀ x ∈ items, keysOk x = true := by
  rw [keysOk] at h
  simp only [List.all_eq_true] at h
  exact fun x hx => h ⟨x, hx⟩ (List.mem_attach _ _)

theorem keysOk_present {es : List (String × Value)} (h : keysOk (.object es) = true) {k : String}
    {x : Value} (hp : present es k = some x) : keysOk x = true :=
  (keysOk_object h).2 _ (present_some_mem hp).1

/-- The values on a field predicate of an encoded object's node. -/
theorem object_field {es : List (String × Value)} {b : BlankNode} {f : String → Term}
    (hiff : ∀ p o, (⟨.bnode b, p, o⟩ : Triple) ∈ G ↔
      ∃ k x, present es k = some x ∧ p = .iri (fieldIri k) ∧ o = f k) (k : String) (y : Term) :
    (⟨.bnode b, .iri (fieldIri k), y⟩ : Triple) ∈ G ↔ ∃ x, present es k = some x ∧ y = f k := by
  rw [hiff]
  constructor
  · rintro ⟨k', x, hp, he, rfl⟩
    simp only [Term.iri.injEq, fieldIri_inj] at he
    subst he
    exact ⟨x, hp, rfl⟩
  · rintro ⟨x, hp, rfl⟩
    exact ⟨k, x, hp, rfl, rfl⟩

/-- A field predicate of an encoded object has at most one value. -/
theorem object_field_unique {es : List (String × Value)} {b : BlankNode} {f : String → Term}
    (hiff : ∀ p o, (⟨.bnode b, p, o⟩ : Triple) ∈ G ↔
      ∃ k x, present es k = some x ∧ p = .iri (fieldIri k) ∧ o = f k) (k : String) :
    ∀ y z, (⟨.bnode b, .iri (fieldIri k), y⟩ : Triple) ∈ G →
      (⟨.bnode b, .iri (fieldIri k), z⟩ : Triple) ∈ G → y = z := by
  intro y z hy hz
  obtain ⟨_, _, rfl⟩ := (object_field hiff k y).1 hy
  obtain ⟨_, _, rfl⟩ := (object_field hiff k z).1 hz
  rfl

end Graphs

/-! ## The shapes of composite types on encoded nodes -/

section Composite

variable {G : Graph}

/-- The encoded object's field triples, as the decoder of an `Enc.object` sees them. -/
abbrev ObjIff (G : Graph) (es : List (String × Value)) (b : BlankNode) (fn : String → Term) :
    Prop :=
  ∀ p o, (⟨.bnode b, p, o⟩ : Triple) ∈ G ↔
    ∃ k x, present es k = some x ∧ p = .iri (fieldIri k) ∧ o = fn k

/-- `sh:minCount 1` when required, `sh:maxCount 1` when asked. -/
def pre (req withMax : Bool) : List Component :=
  (if req then [.minCount 1] else []) ++ (if withMax then [.maxCount 1] else [])

theorem counts_eq (opt : Bool) : counts opt = pre (!opt) true := by cases opt <;> rfl

theorem minPart_eq (opt : Bool) :
    ((if opt then [] else [.minCount 1]) : List Component) ++ [] = pre (!opt) false := by
  cases opt <;> rfl

theorem object_prop {es : List (String × Value)} {b : BlankNode} {fn : String → Term}
    (hiff : ObjIff G es b fn) (key : String) (req withMax : Bool) (S : Shape) :
    Conforms G (propS (fieldIri key) (pre req withMax ++ [.node S])) (.bnode b) ↔
      match present es key with
      | none => req = false
      | some _ => Conforms G S (fn key) := by
  rw [conforms_propS, compsHold_append, compsHold_cons, compsHold_nil, and_true, holds_pnode]
  have huniq := object_field_unique hiff key
  have hfield := object_field hiff key
  have hpre : CompsHold G (some (.pred (fieldIri key))) (pre req withMax) (.bnode b) ↔
      (req = true → ∃ y, (⟨.bnode b, .iri (fieldIri key), y⟩ : Triple) ∈ G) := by
    cases req <;> cases withMax <;>
      simp [pre, compsHold_cons, compsHold_nil, holds_minCount1, holds_maxCount1] <;>
      first | exact huniq | exact fun _ _ => huniq
  rw [hpre]
  cases hp : present es key with
  | none =>
    have hno : ∀ y, (⟨.bnode b, .iri (fieldIri key), y⟩ : Triple) ∉ G := by
      intro y hy; obtain ⟨x, hx, _⟩ := (hfield y).1 hy; simp [hp] at hx
    simp only
    constructor
    · rintro ⟨h1, _⟩
      cases req
      · rfl
      · obtain ⟨y, hy⟩ := h1 rfl; exact absurd hy (hno y)
    · intro ho
      exact ⟨fun hf => by simp [ho] at hf, fun y hy => absurd hy (hno y)⟩
  | some x =>
    simp only
    constructor
    · rintro ⟨_, h2⟩
      exact h2 _ ((hfield _).2 ⟨x, hp, rfl⟩)
    · intro hc
      refine ⟨fun _ => ⟨fn key, (hfield _).2 ⟨x, hp, rfl⟩⟩, fun y hy => ?_⟩
      obtain ⟨_, _, rfl⟩ := (hfield y).1 hy
      exact hc

/-- The shape of a non-empty collection's cell. -/
def consS : Shape :=
  nodeS (closedBlank [firstIri, restIri] ++
    [.property (propS firstIri (counts false)), .property (propS restIri (counts false))])

theorem cons_iff {v : Value} {n : Term} (h : Enc G v n) :
    Conforms G consS n ↔ ∃ x xs, v = .array (x :: xs) := by
  simp only [consS, conforms_nodeS, closedBlank, counts, List.cons_append, List.nil_append,
    compsHold_cons, compsHold_nil, holds_test, holds_closed, holds_property, conforms_propS,
    holds_minCount1, holds_maxCount1, Bool.false_eq_true, ↓reduceIte, and_true]
  constructor
  · intro hh
    obtain ⟨y, hy⟩ := hh.2.2.1.1
    rcases (enc_bnode h).1 hh.1 with ⟨es, rfl⟩ | hc
    · cases h with
      | object _ b fn hiff _ =>
        obtain ⟨k, x, _, he, _⟩ := (hiff _ _).1 hy
        simp only [Term.iri.injEq] at he
        exact absurd he.symm (fieldIri_ne_first k)
    · exact hc
  · rintro ⟨x, xs, rfl⟩
    cases h with
    | cons _ _ b nx nr hiff _ _ =>
      have hf : ∀ y, (⟨.bnode b, .iri firstIri, y⟩ : Triple) ∈ G ↔ y = nx := by
        intro y; rw [hiff]; simp [firstIri, restIri]
      have hr : ∀ y, (⟨.bnode b, .iri restIri, y⟩ : Triple) ∈ G ↔ y = nr := by
        intro y; rw [hiff]; simp [firstIri, restIri]
      refine ⟨rfl, fun p o ho => ?_, ⟨⟨nx, (hf _).2 rfl⟩, fun y z hy hz => ?_⟩,
        ⟨⟨nr, (hr _).2 rfl⟩, fun y z hy hz => ?_⟩⟩
      · rcases (hiff _ _).1 ho with ⟨rfl, _⟩ | ⟨rfl, _⟩
        · exact ⟨firstIri, by simp, rfl⟩
        · exact ⟨restIri, by simp, rfl⟩
      · rw [(hf _).1 hy, (hf _).1 hz]
      · rw [(hr _).1 hy, (hr _).1 hz]

/-- A collection cell is not closed to predicates other than `rdf:first`. -/
theorem cons_not_closed {x : Value} {xs : List Value} {n : Term} (h : Enc G (.array (x :: xs)) n)
    (allowed : List IRI) (hne : firstIri ∉ allowed) :
    ¬ CompHolds G none (.closed allowed) n := by
  rw [holds_closed]
  intro hc
  cases h with
  | cons _ _ b nx nr hiff _ _ =>
    obtain ⟨q, hq, he⟩ := hc _ _ ((hiff _ _).2 (.inl ⟨rfl, rfl⟩))
    simp only [Term.iri.injEq] at he
    exact hne (he ▸ hq)

theorem first_not_field (ks : List String) : firstIri ∉ ks.map fieldIri := by
  intro h
  obtain ⟨k, _, hk⟩ := List.mem_map.1 h
  exact fieldIri_ne_first k hk

end Composite

/-! ## Agreement, type by type -/

section Agree

variable (D : Decls) (G : Graph)

/-- The shapes of `t` unfolded `k` times hold at the node of a value exactly when `checkF`
accepts it. -/
def Agrees (k : Nat) (t : Ty) : Prop :=
  JsonSchema.tagsOk t = true → ∀ v n, keysOk v = true → Enc G v n →
    (CompsHold G none (comps D k t) n ↔ checkF D k t v = true)

variable {D G}

theorem agrees_value (k : Nat) : Agrees D G k .value := by
  intro _ v n _ _
  simp [comps, checkF, compsHold_nil]

theorem not_string_ok {v : Value} {n : Term} (h : Enc G v n) (hs : ¬ ∃ s, v = .str s) :
    (TermTest.datatype xsdString).ok n = false := by
  cases hx : (TermTest.datatype xsdString).ok n
  · rfl
  · exact absurd ((enc_string h).1 hx) hs

theorem agrees_prim (hP : PatternsAgree) (k : Nat) (p : Prim) : Agrees D G k (.prim p) := by
  intro _ v n _ h
  cases p
  all_goals rw [comps, checkF]
  case bool =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_test, enc_bool h]
    cases v <;> simp [primOk]
  case int =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_test, enc_long h]
    cases v <;> simp [primOk, and_assoc]
  case string =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_test, enc_string h]
    cases v <;> simp [primOk]
  case datetime =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_test]
    by_cases hs : ∃ s, v = .str s
    · obtain ⟨s, rfl⟩ := hs
      cases h
      simp [str_ok, (hP s).2.1, primOk]
    · simp only [not_string_ok h hs, Bool.false_eq_true, false_and, false_iff]
      cases v <;> simp_all [primOk]
  case iri =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_test]
    by_cases hs : ∃ s, v = .str s
    · obtain ⟨s, rfl⟩ := hs
      cases h
      simp [str_ok, (hP s).2.2, primOk]
    · simp only [not_string_ok h hs, Bool.false_eq_true, false_and, false_iff]
      cases v <;> simp_all [primOk]
  case decimal =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_or, List.mem_cons, List.not_mem_nil,
      or_false, exists_eq_or_imp, exists_eq_left, conforms_nodeS, holds_test, enc_long h,
      enc_decimal h]
    by_cases hs : ∃ s, v = .str s
    · obtain ⟨s, rfl⟩ := hs
      cases h
      simp [str_ok, (hP s).1, primOk]
    · simp only [not_string_ok h hs, Bool.false_eq_true, false_and, or_false]
      cases v with
      | num m e =>
        cases hi : Lex.intOk m e <;> simp [primOk, hi, and_assoc]
      | _ => simp_all [primOk]

theorem agrees_named0 (n : String) : Agrees D G 0 (.named n) := by
  intro _ v x _ _
  rw [comps, checkF]
  simp [compsHold_cons, compsHold_nil, holds_not, conforms_nodeS]

theorem agrees_named (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) (k : Nat)
    (ih : ∀ u, Agrees D G k u) (n : String) : Agrees D G (k + 1) (.named n) := by
  intro _ v x hv h
  rw [comps, checkF]
  cases hl : D.lookup n with
  | none => simp [compsHold_cons, compsHold_nil, holds_not, conforms_nodeS]
  | some u =>
    simp only [compsHold_cons, compsHold_nil, and_true, holds_node, conforms_nodeS]
    exact ih u (hD _ (lookup_mem hl)) v x hv h

theorem agrees_optional (k : Nat) (t : Ty) (ih : Agrees D G k t) : Agrees D G k (.optional t) := by
  intro ht v n hv h
  have ht' : JsonSchema.tagsOk t = true := by rw [JsonSchema.tagsOk] at ht; exact ht
  rw [comps, checkF]
  simp only [compsHold_cons, compsHold_nil, and_true, holds_or, List.mem_cons, List.not_mem_nil,
    or_false, exists_eq_or_imp, exists_eq_left, conforms_nodeS, holds_hasValue, enc_null h,
    ih ht' v n hv h, Bool.or_eq_true]

theorem agrees_list (hG : BNodeSubjects G) (k : Nat) (t : Ty) (ih : Agrees D G k t) :
    Agrees D G k (.list t) := by
  intro ht v n hv h
  have ht' : JsonSchema.tagsOk t = true := by rw [JsonSchema.tagsOk] at ht; exact ht
  rw [comps]
  have hcons : nodeS (closedBlank [firstIri, restIri] ++
      [.property (propS firstIri (counts false)), .property (propS restIri (counts false))]) =
      consS := rfl
  simp only [hcons, compsHold_cons, compsHold_nil, and_true, holds_or, List.mem_cons,
    List.not_mem_nil, or_false, exists_eq_or_imp, exists_eq_left, conforms_nodeS, holds_hasValue,
    holds_property, conforms_mkShape, cons_iff h, enc_nil h]
  simp only [CompHolds, VN, conforms_nodeS]
  cases v with
  | array items =>
    rw [checkF]
    simp only [List.all_eq_true]
    have hok := keysOk_array hv
    constructor
    · rintro ⟨_, hm⟩ x hx
      obtain ⟨y, hy, he⟩ := enc_member h hx
      exact (ih ht' x y (hok x hx) he).1 (hm y hy)
    · intro hall
      refine ⟨by cases items <;> simp, fun y hy => ?_⟩
      obtain ⟨x, hx, he⟩ := member_enc hG h hy
      exact (ih ht' x y (hok x hx) he).2 (hall x hx)
  | _ => rw [checkF]; simp; try (intro _ hh; cases hh)

theorem record_comps_iff (k : Nat) (fs : List (String × Ty)) (n : Term) :
    CompsHold G none (comps D k (.record fs)) n ↔
      NodeKind.bnode.holds n = true ∧ CompHolds G none (.closed (fs.map fun f => fieldIri f.1)) n ∧
      ∀ f ∈ fs, Conforms G (propS (fieldIri f.1)
        (counts (optish D f.2) ++ [.node (nodeS (comps D k f.2))])) n := by
  rw [comps]
  simp only [compsHold_append, closedBlank, compsHold_cons, compsHold_nil, and_true, holds_test,
    TermTest.ok, and_assoc]
  apply and_congr Iff.rfl
  apply and_congr Iff.rfl
  rw [compsHold_iff]
  simp only [List.mem_map, List.mem_attach, true_and, Subtype.exists, forall_exists_index,
    Prod.forall]
  constructor
  · intro h a b hab
    exact (holds_property G _ _).1 (h _ a b hab rfl)
  · intro h c a b hab hc
    subst hc
    exact (holds_property G _ _).2 (h a b hab)

theorem agrees_record (k : Nat) (fs : List (String × Ty))
    (ih : ∀ f ∈ fs, Agrees D G k f.2) : Agrees D G k (.record fs) := by
  intro ht v n hv h
  have htf := JsonSchema.tagsOk_record ht
  rw [record_comps_iff]
  cases h with
  | object es b fn hiff hch =>
    obtain ⟨hnd, _⟩ := keysOk_object hv
    rw [checkF]
    simp only [NodeKind.holds, true_and, Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true,
      List.any_eq_true, beq_iff_eq]
    apply and_congr
    · rw [holds_closed]
      constructor
      · intro hc e he
        cases hu : isUndef e.2
        · obtain ⟨q, hq, hpq⟩ := hc _ _ ((hiff _ _).2
            ⟨e.1, e.2, present_of_mem hnd he hu, rfl, rfl⟩)
          obtain ⟨f, hf, rfl⟩ := List.mem_map.1 hq
          simp only [Term.iri.injEq, fieldIri_inj] at hpq
          exact .inr ⟨f, hf, hpq.symm⟩
        · exact .inl rfl
      · intro hc p o ho
        obtain ⟨k', x, hp, rfl, rfl⟩ := (hiff _ _).1 ho
        obtain ⟨hm, hu⟩ := present_some_mem hp
        rcases hc _ hm with h' | ⟨f, hf, hfk⟩
        · simp [hu] at h'
        · exact ⟨fieldIri f.1, List.mem_map.2 ⟨f, hf, rfl⟩, by simp [hfk]⟩
    · have hf1 : ∀ f ∈ fs, Conforms G (propS (fieldIri f.1)
          (counts (optish D f.2) ++ [.node (nodeS (comps D k f.2))])) (.bnode b) ↔
          (match present es f.1 with
           | none => optish D f.2
           | some fv => checkF D k f.2 fv) = true := by
        intro f hf
        rw [counts_eq, object_prop hiff]
        cases hp : present es f.1 with
        | none => simp
        | some x =>
          simp only [conforms_nodeS]
          exact ih f hf (htf f hf) x (fn f.1) (keysOk_present hv hp) (hch _ _ hp)
      constructor
      · intro hall ⟨f, hf⟩ _
        exact (hf1 f hf).1 (hall f hf)
      · intro hall f hf
        exact (hf1 f hf).2 (hall ⟨f, hf⟩ (List.mem_attach _ _))
  | cons x xs b nx nr hiff hx hr =>
    rw [checkF]
    simp only [Bool.false_eq_true, iff_false, not_and]
    intro _ hc _
    refine cons_not_closed (.cons x xs b nx nr hiff hx hr) _ ?_ hc
    have := first_not_field (fs.map (·.1))
    simpa [List.map_map, Function.comp_def] using this
    intro _ hh; cases hh
  | num m e =>
    rw [checkF] <;> first | (intro _ hh; cases hh) | (unfold numTerm; split <;> simp [NodeKind.holds])
  | _ =>
    rw [checkF] <;> first | (intro _ hh; cases hh) | simp [NodeKind.holds, nullT, nilT, boolTerm, strLit]

/-- The shape of one variant of a union. -/
def variantS (D : Decls) (k : Nat) (tag : String) (p : Option Ty) : Shape :=
  nodeS (.property (propS (fieldIri "tag") [.hasValue (strLit tag)]) ::
    match p with
    | none => [.property (propS (fieldIri "value") [.maxCount 0])]
    | some pt =>
      [.property (propS (fieldIri "value")
        ((if optish D pt then [] else [.minCount 1]) ++ [.node (nodeS (comps D k pt))]))])

theorem union_comps_iff (k : Nat) (vs : List (String × Option Ty)) (n : Term) :
    CompsHold G none (comps D k (.union vs)) n ↔
      NodeKind.bnode.holds n = true ∧
      CompHolds G none (.closed [fieldIri "tag", fieldIri "value"]) n ∧
      Conforms G (propS (fieldIri "tag") (counts false ++ [.test (.datatype xsdString)])) n ∧
      Conforms G (propS (fieldIri "value") [.maxCount 1]) n ∧
      ∃ x ∈ vs, Conforms G (variantS D k x.1 x.2) n := by
  rw [comps]
  simp only [compsHold_append, closedBlank, compsHold_cons, compsHold_nil, and_true, holds_test,
    TermTest.ok, and_assoc, holds_property, holds_or]
  apply and_congr Iff.rfl; apply and_congr Iff.rfl; apply and_congr Iff.rfl
  apply and_congr Iff.rfl
  constructor
  · rintro ⟨s, hs, hc⟩
    obtain ⟨⟨⟨a, p⟩, hx⟩, _, rfl⟩ := List.mem_map.1 hs
    refine ⟨(a, p), hx, ?_⟩
    cases p <;> exact hc
  · rintro ⟨⟨a, p⟩, hx, hc⟩
    refine ⟨_, List.mem_map.2 ⟨⟨(a, p), hx⟩, List.mem_attach _ _, rfl⟩, ?_⟩
    cases p <;> exact hc

theorem present_str {es : List (String × Value)} {k s : String} :
    present es k = some (.str s) ↔ es.lookup k = some (.str s) := by
  unfold present
  cases hl : es.lookup k with
  | none => simp
  | some y => cases y <;> simp

theorem minPart_node (o : Bool) (S : Shape) :
    ((if o then [] else [.minCount 1]) : List Component) ++ [.node S] =
      pre (!o) false ++ [.node S] := by
  cases o <;> rfl

theorem agrees_union (k : Nat) (vs : List (String × Option Ty))
    (ih : ∀ x ∈ vs, ∀ pt, x.2 = some pt → Agrees D G k pt) : Agrees D G k (.union vs) := by
  intro ht v n hv h
  obtain ⟨hnd, hpt⟩ := JsonSchema.tagsOk_union ht
  rw [union_comps_iff]
  cases h with
  | object es b fn hiff hch =>
    obtain ⟨hnde, _⟩ := keysOk_object hv
    have hfield := object_field hiff
    -- the tag value
    have htagv : ∀ a, (⟨.bnode b, .iri (fieldIri "tag"), strLit a⟩ : Triple) ∈ G ↔
        present es "tag" = some (.str a) := by
      intro a
      rw [hfield]
      constructor
      · rintro ⟨x, hx, he⟩
        have := hch _ _ hx
        rw [← he] at this
        rw [hx, enc_strLit this]
      · intro hx
        refine ⟨_, hx, ?_⟩
        have := hch _ _ hx
        generalize fn "tag" = z at this ⊢
        cases this; rfl
    have htag : Conforms G (propS (fieldIri "tag") (counts false ++ [.test (.datatype xsdString)]))
        (.bnode b) ↔ ∃ s, present es "tag" = some (.str s) := by
      rw [conforms_propS]
      simp only [counts, Bool.false_eq_true, ↓reduceIte, List.cons_append, List.nil_append,
        compsHold_cons, compsHold_nil, and_true, holds_minCount1, holds_maxCount1, holds_ptest]
      constructor
      · rintro ⟨⟨y, hy⟩, _, hs⟩
        obtain ⟨x, hx, rfl⟩ := (hfield _ _).1 hy
        obtain ⟨s, rfl⟩ := (enc_string (hch _ _ hx)).1 (hs _ hy)
        exact ⟨s, hx⟩
      · rintro ⟨s, hx⟩
        refine ⟨⟨_, (hfield _ _).2 ⟨_, hx, rfl⟩⟩, object_field_unique hiff "tag", fun y hy => ?_⟩
        obtain ⟨x', hx', rfl⟩ := (hfield _ _).1 hy
        rw [hx] at hx'; cases hx'
        exact (enc_string (hch _ _ hx)).2 ⟨s, rfl⟩
    have hval : Conforms G (propS (fieldIri "value") [.maxCount 1]) (.bnode b) := by
      simp only [conforms_propS, compsHold_cons, compsHold_nil, and_true, holds_maxCount1]
      exact object_field_unique hiff "value"
    have hvar : ∀ x ∈ vs, Conforms G (variantS D k x.1 x.2) (.bnode b) ↔
        present es "tag" = some (.str x.1) ∧
        match x.2 with
        | none => present es "value" = none
        | some pt =>
          (match present es "value" with
           | none => optish D pt
           | some pv => checkF D k pt pv) = true := by
      intro x hx
      obtain ⟨a, p⟩ := x
      unfold variantS
      cases p with
      | none =>
        simp only [conforms_nodeS, compsHold_cons, compsHold_nil, and_true, holds_property,
          conforms_propS, holds_phasValue, htagv, holds_maxCount0]
        apply and_congr Iff.rfl
        constructor
        · intro hno
          cases hp : present es "value" with
          | none => rfl
          | some y => exact absurd ((hfield _ _).2 ⟨y, hp, rfl⟩) (hno _)
        · intro hp y hy
          obtain ⟨_, hx, _⟩ := (hfield _ _).1 hy
          simp [hp] at hx
      | some pt =>
        simp only [conforms_nodeS, compsHold_cons, compsHold_nil, and_true, holds_property,
          conforms_propS, holds_phasValue, htagv]
        apply and_congr Iff.rfl
        rw [minPart_node, ← conforms_propS, object_prop hiff]
        cases hp : present es "value" with
        | none => simp
        | some pv =>
          simp only [conforms_nodeS]
          exact ih _ hx pt rfl (hpt _ hx pt rfl) pv (fn "value") (keysOk_present hv hp)
            (hch _ _ hp)
    have hclosed : CompHolds G none (.closed [fieldIri "tag", fieldIri "value"]) (.bnode b) ↔
        ∀ e ∈ es, e.1 = "tag" ∨ e.1 = "value" ∨ isUndef e.2 = true := by
      rw [holds_closed]
      constructor
      · intro hc e he
        cases hu : isUndef e.2
        · obtain ⟨q, hq, hpq⟩ := hc _ _ ((hiff _ _).2
            ⟨e.1, e.2, present_of_mem hnde he hu, rfl, rfl⟩)
          simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
          rcases hq with rfl | rfl <;> simp only [Term.iri.injEq, fieldIri_inj] at hpq
          · exact .inl hpq
          · exact .inr (.inl hpq)
        · exact .inr (.inr rfl)
      · intro hc p o ho
        obtain ⟨k', x, hp, rfl, rfl⟩ := (hiff _ _).1 ho
        obtain ⟨hm, hu⟩ := present_some_mem hp
        rcases hc _ hm with h' | h' | h'
        · exact ⟨fieldIri "tag", by simp, by simp only at h'; rw [h']⟩
        · exact ⟨fieldIri "value", by simp, by simp only at h'; rw [h']⟩
        · simp [hu] at h'
    simp only [NodeKind.holds, true_and, hclosed, htag, hval]
    rw [checkF]
    cases htg : present es "tag" with
    | none =>
      have hno : ∀ s, es.lookup "tag" ≠ some (.str s) := fun s hs => by
        rw [← present_str] at hs; simp [htg] at hs
      simp only [reduceCtorEq, exists_false, false_and, and_false]
    | some y =>
      cases y with
      | str t =>
        rw [present_str.1 htg]
        simp only [Option.some.injEq, Value.str.injEq, exists_eq', true_and, Bool.and_eq_true,
          List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
        apply and_congr Iff.rfl
        constructor
        · rintro ⟨x, hx, hc⟩
          rw [hvar x hx, htg] at hc
          obtain ⟨he, hc⟩ := hc
          simp only [Option.some.injEq, Value.str.injEq] at he
          obtain ⟨a, p⟩ := x
          simp only at he hc; subst he
          rw [JsonSchema.lookup_of_mem hnd hx]
          cases p with
          | none => simpa using hc
          | some pt => exact hc
        · intro hc
          split at hc
          · exact absurd hc (by simp)
          · rename_i hl
            refine ⟨(t, none), lookup_mem' hl, ?_⟩
            rw [hvar _ (lookup_mem' hl), htg]
            simpa using hc
          · rename_i pt hl
            refine ⟨(t, some pt), lookup_mem' hl, ?_⟩
            rw [hvar _ (lookup_mem' hl), htg]
            exact ⟨rfl, hc⟩
      | _ =>
        have hno : ∀ s, es.lookup "tag" ≠ some (.str s) := fun s hs => by
          rw [← present_str] at hs; simp [htg] at hs
        simp only [reduceCtorEq, exists_false, false_and, and_false, Option.some.injEq]
  | cons x xs b nx nr hiff hx hr =>
    rw [checkF]
    simp only [Bool.false_eq_true, iff_false, not_and]
    intro _ hc
    exact absurd hc (cons_not_closed (.cons x xs b nx nr hiff hx hr) _
      (first_not_field ["tag", "value"]))
    intro _ hh; cases hh
  | num m e =>
    rw [checkF] <;> first | (intro _ hh; cases hh) | (unfold numTerm; split <;> simp [NodeKind.holds])
  | _ =>
    rw [checkF] <;> first | (intro _ hh; cases hh) | simp [NodeKind.holds, nullT, nilT, boolTerm, strLit]

end Agree

/-! ## Every type, every unfolding -/

section Main

variable {D : Decls} {G : Graph}

/-- Measure facts for the induction on types. -/
theorem sizeOf_field {fs : List (String × Ty)} {f : String × Ty} (hf : f ∈ fs) :
    sizeOf f.2 < sizeOf (Ty.record fs) := by
  have := List.sizeOf_lt_of_mem hf
  obtain ⟨n, ft⟩ := f
  simp only [Ty.record.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢; omega

theorem sizeOf_payload {vs : List (String × Option Ty)} {x : String × Option Ty} (hx : x ∈ vs)
    {pt : Ty} (hp : x.2 = some pt) : sizeOf pt < sizeOf (Ty.union vs) := by
  have := List.sizeOf_lt_of_mem hx
  obtain ⟨tag, p⟩ := x
  simp only at hp; subst hp
  simp only [Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec, Option.some.sizeOf_spec] at this ⊢; omega

theorem agrees_at (hP : PatternsAgree) (hG : BNodeSubjects G) (k : Nat)
    (hn : ∀ n, Agrees D G k (.named n)) : ∀ t, Agrees D G k t := by
  suffices ∀ m t, sizeOf t = m → Agrees D G k t from fun t => this _ t rfl
  intro m
  induction m using Nat.strongRecOn with
  | ind m ih =>
  intro t ht
  subst ht
  cases t with
  | value => exact agrees_value k
  | prim p => exact agrees_prim hP k p
  | named n => exact hn n
  | optional t => exact agrees_optional k t (ih _ (by simp) t rfl)
  | list t => exact agrees_list hG k t (ih _ (by simp) t rfl)
  | record fs => exact agrees_record k fs (fun f hf => ih _ (sizeOf_field hf) f.2 rfl)
  | union vs => exact agrees_union k vs (fun x hx pt hp => ih _ (sizeOf_payload hx hp) pt rfl)

/-- **The shapes agree with `checkF`** at every unfolding. -/
theorem agrees_all (hP : PatternsAgree) (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true)
    (hG : BNodeSubjects G) : ∀ k t, Agrees D G k t := by
  intro k
  induction k with
  | zero => exact agrees_at hP hG 0 agrees_named0
  | succ k ih => exact agrees_at hP hG (k + 1) (agrees_named hD k ih)

/-- The same statement for `checkF` at all types, from its named case. -/
theorem checkF_at (P : Nat → Ty → Prop) (k : Nat) (hn : ∀ n, P k (.named n))
    (hstep : ∀ t, (∀ t', sizeOf t' < sizeOf t → P k t') → (∀ n, t ≠ .named n) → P k t) :
    ∀ t, P k t := by
  suffices ∀ m t, sizeOf t = m → P k t from fun t => this _ t rfl
  intro m
  induction m using Nat.strongRecOn with
  | ind m ih =>
  intro t ht
  subst ht
  cases t with
  | named n => exact hn n
  | _ => exact hstep _ (fun t' h' => ih _ h' t' rfl) (by simp)

/-- `checkF` accepts only values of the type. -/
theorem checkF_sound (D : Decls) : ∀ k t v, checkF D k t v = true → HasType D v t := by
  intro k
  induction k with
  | zero =>
    refine checkF_at (fun k t => ∀ v, checkF D k t v = true → HasType D v t) 0
      (fun n v h => by rw [checkF] at h; cases h) ?_
    intro t ih hnn
    exact sound_step D 0 t ih hnn
  | succ k ihk =>
    refine checkF_at (fun k t => ∀ v, checkF D k t v = true → HasType D v t) (k + 1)
      (fun n v h => ?_) ?_
    · rw [checkF] at h
      cases hl : D.lookup n with
      | none => rw [hl] at h; cases h
      | some u => rw [hl] at h; exact .named n u v hl (ihk u v h)
    · intro t ih hnn
      exact sound_step D (k + 1) t ih hnn
where
  sound_step (D : Decls) (k : Nat) (t : Ty)
      (ih : ∀ t', sizeOf t' < sizeOf t → ∀ v, checkF D k t' v = true → HasType D v t')
      (hnn : ∀ n, t ≠ .named n) :
      ∀ v, checkF D k t v = true → HasType D v t := by
    intro v h
    cases t with
    | value => exact .value v
    | prim p => rw [checkF] at h; exact .prim p v h
    | named n => exact absurd rfl (hnn n)
    | optional t₀ =>
      rw [checkF] at h
      cases ha : absent v
      · rw [ha, Bool.false_or] at h; exact .optSome t₀ v ha (ih t₀ (by simp) v h)
      · exact .optNone t₀ v ha
    | list t₀ =>
      cases v with
      | array items =>
        rw [checkF] at h
        simp only [List.all_eq_true] at h
        exact .list t₀ items fun x hx => ih t₀ (by simp) x (h x hx)
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)
    | record fs =>
      cases v with
      | object entries =>
        rw [checkF] at h
        simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, List.any_eq_true,
          beq_iff_eq] at h
        obtain ⟨hk, hf⟩ := h
        refine .record fs entries hk (fun f hf' hp => ?_) (fun f hf' fv hp => ?_)
        · have := hf ⟨f, hf'⟩ (List.mem_attach _ _)
          simp only [hp] at this; exact this
        · have := hf ⟨f, hf'⟩ (List.mem_attach _ _)
          simp only [hp] at this
          exact ih f.2 (sizeOf_field hf') fv this
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)
    | union vs =>
      cases v with
      | object entries =>
        rw [checkF] at h
        split at h
        · rename_i tag htag
          simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq] at h
          obtain ⟨hk, hm⟩ := h
          split at hm
          · cases hm
          · rename_i hl
            exact .tagOnly vs entries tag htag hk hl (by simpa using hm)
          · rename_i pt hl
            cases hp : present entries "value" with
            | none =>
              rw [hp] at hm
              exact .payloadAbsent vs entries tag pt htag hk hl hp hm
            | some pv =>
              rw [hp] at hm
              exact .payload vs entries tag pt pv htag hk hl hp
                (ih pt (sizeOf_payload (lookup_mem' hl) rfl) pv hm)
        · cases h
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)

/-- More unfolding accepts at least as much. -/
theorem checkF_mono (D : Decls) : ∀ k t v, checkF D k t v = true → checkF D (k + 1) t v = true := by
  intro k
  induction k with
  | zero =>
    refine checkF_at (fun k t => ∀ v, checkF D k t v = true → checkF D (k + 1) t v = true) 0
      (fun n v h => by rw [checkF] at h; cases h) ?_
    intro t ih hnn
    exact mono_step D 0 t ih hnn
  | succ k ihk =>
    refine checkF_at (fun k t => ∀ v, checkF D k t v = true → checkF D (k + 1) t v = true)
      (k + 1) (fun n v h => ?_) ?_
    · rw [checkF] at h ⊢
      cases hl : D.lookup n with
      | none => rw [hl] at h; cases h
      | some u => rw [hl] at h; simp only; exact ihk u v h
    · intro t ih hnn
      exact mono_step D (k + 1) t ih hnn
where
  mono_step (D : Decls) (k : Nat) (t : Ty)
      (ih : ∀ t', sizeOf t' < sizeOf t → ∀ v, checkF D k t' v = true → checkF D (k + 1) t' v = true)
      (hnn : ∀ n, t ≠ .named n) :
      ∀ v, checkF D k t v = true → checkF D (k + 1) t v = true := by
    intro v h
    cases t with
    | value => rw [checkF]
    | prim p => rw [checkF] at h ⊢; exact h
    | named n => exact absurd rfl (hnn n)
    | optional t₀ =>
      rw [checkF] at h ⊢
      simp only [Bool.or_eq_true] at h ⊢
      exact h.imp_right (ih t₀ (by simp) v)
    | list t₀ =>
      cases v with
      | array items =>
        rw [checkF] at h ⊢
        simp only [List.all_eq_true] at h ⊢
        exact fun x hx => ih t₀ (by simp) x (h x hx)
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)
    | record fs =>
      cases v with
      | object entries =>
        rw [checkF] at h ⊢
        simp only [Bool.and_eq_true, List.all_eq_true] at h ⊢
        refine ⟨h.1, fun ⟨f, hf'⟩ _ => ?_⟩
        have := h.2 ⟨f, hf'⟩ (List.mem_attach _ _)
        simp only at this ⊢
        cases hp : present entries f.1 with
        | none => rw [hp] at this; exact this
        | some fv => rw [hp] at this; exact ih f.2 (sizeOf_field hf') fv this
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)
    | union vs =>
      cases v with
      | object entries =>
        rw [checkF] at h ⊢
        split at h
        · rename_i tag htag
          simp only [Bool.and_eq_true] at h ⊢
          refine ⟨h.1, ?_⟩
          have hm := h.2
          split
          · rename_i hl
            split at hm
            · cases hm
            · rename_i hl'; rw [hl] at hl'; cases hl'
            · rename_i _ hl'; rw [hl] at hl'; cases hl'
          · rename_i hl
            split at hm
            · rename_i hl'; rw [hl] at hl'; cases hl'
            · exact hm
            · rename_i _ hl'; rw [hl] at hl'; cases hl'
          · rename_i pt hl
            split at hm
            · rename_i hl'; rw [hl] at hl'; cases hl'
            · rename_i hl'; rw [hl] at hl'; cases hl'
            · rename_i pt' hl'
              rw [hl] at hl'; cases hl'
              cases hp : present entries "value" with
              | none => rw [hp] at hm; exact hm
              | some pv =>
                rw [hp] at hm
                exact ih pt (sizeOf_payload (lookup_mem' hl) rfl) pv hm
        · cases h
      | _ => rw [checkF] at h <;> first | cases h | (intro _ hh; cases hh)

theorem checkF_mono_le (D : Decls) {k k' : Nat} (hk : k ≤ k') {t : Ty} {v : Value}
    (h : checkF D k t v = true) : checkF D k' t v = true := by
  obtain ⟨j, rfl⟩ := Nat.exists_eq_add_of_le hk
  induction j with
  | zero => exact h
  | succ j ih => exact checkF_mono D _ t v (ih (by omega))

/-- Finitely many witnesses of a monotone property have a common bound. -/
theorem common_fuel {α : Type} (P : Nat → α → Prop) (mono : ∀ k k' a, k ≤ k' → P k a → P k' a) :
    ∀ l : List α, (∀ a ∈ l, ∃ k, P k a) → ∃ k, ∀ a ∈ l, P k a
  | [], _ => ⟨0, by simp⟩
  | a :: l, h => by
    obtain ⟨k₁, h₁⟩ := h a (by simp)
    obtain ⟨k₂, h₂⟩ := common_fuel P mono l (fun b hb => h b (by simp [hb]))
    refine ⟨max k₁ k₂, fun b hb => ?_⟩
    rcases List.mem_cons.1 hb with rfl | hb
    · exact mono _ _ _ (Nat.le_max_left _ _) h₁
    · exact mono _ _ _ (Nat.le_max_right _ _) (h₂ b hb)

/-- Every value of the type is accepted at some unfolding. -/
theorem checkF_complete (D : Decls) {v : Value} {t : Ty} (h : HasType D v t) :
    ∃ k, checkF D k t v = true := by
  induction h with
  | value v => exact ⟨0, by rw [checkF]⟩
  | prim p v hp => exact ⟨0, by rw [checkF]; exact hp⟩
  | named n u v hl _ ih =>
    obtain ⟨k, hk⟩ := ih
    exact ⟨k + 1, by rw [checkF, hl]; exact hk⟩
  | optNone t v ha => exact ⟨0, by rw [checkF]; simp [ha]⟩
  | optSome t v _ _ ih =>
    obtain ⟨k, hk⟩ := ih
    exact ⟨k, by rw [checkF]; simp [hk]⟩
  | list t items _ ih =>
    obtain ⟨k, hk⟩ := common_fuel (fun k x => checkF D k t x = true)
      (fun _ _ _ hle hx => checkF_mono_le D hle hx) items ih
    exact ⟨k, by rw [checkF]; simpa using hk⟩
  | record fs entries hk hopt _ ih =>
    obtain ⟨k, hkf⟩ := common_fuel
      (fun k (f : String × Ty) => ∀ fv, present entries f.1 = some fv → checkF D k f.2 fv = true)
      (fun _ _ _ hle hx fv hp => checkF_mono_le D hle (hx fv hp)) fs
      (fun f hf => by
        cases hp : present entries f.1 with
        | none => exact ⟨0, fun fv h => by cases h⟩
        | some fv =>
          obtain ⟨k, hk⟩ := ih f hf fv hp
          exact ⟨k, fun fv' h => by cases h; exact hk⟩)
    refine ⟨k, ?_⟩
    rw [checkF]
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, List.any_eq_true, beq_iff_eq]
    refine ⟨hk, fun ⟨f, hf⟩ _ => ?_⟩
    simp only
    cases hp : present entries f.1 with
    | none => exact hopt f hf hp
    | some fv => exact hkf f hf fv hp
  | tagOnly vs entries tag htag hk hl hp =>
    refine ⟨0, ?_⟩
    rw [checkF, htag]
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    refine ⟨hk, ?_⟩
    split
    · rename_i hl'; rw [hl] at hl'; cases hl'
    · simp [hp]
    · rename_i _ hl'; rw [hl] at hl'; cases hl'
  | payloadAbsent vs entries tag pt htag hk hl hp ho =>
    refine ⟨0, ?_⟩
    rw [checkF, htag]
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    refine ⟨hk, ?_⟩
    split
    · rename_i hl'; rw [hl] at hl'; cases hl'
    · rename_i hl'; rw [hl] at hl'; cases hl'
    · rename_i pt' hl'
      rw [hl] at hl'; cases hl'
      simp [hp, ho]
  | payload vs entries tag pt pv htag hk hl hp _ ih =>
    obtain ⟨k, hkp⟩ := ih
    refine ⟨k, ?_⟩
    rw [checkF, htag]
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    refine ⟨hk, ?_⟩
    split
    · rename_i hl'; rw [hl] at hl'; cases hl'
    · rename_i hl'; rw [hl] at hl'; cases hl'
    · rename_i pt' hl'
      rw [hl] at hl'; cases hl'
      simp [hp, hkp]

end Main

/-! ## Agreement on graphs that encode the value -/

/-- On a graph that encodes the value, the `k`-th unfolding of the shapes of `t` holds at the
value's node exactly when `checkF` accepts the value. -/
theorem conforms_iff_checkF (hP : PatternsAgree) {D : Decls}
    (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) {G : Graph} (hG : BNodeSubjects G) (k : Nat)
    {t : Ty} (ht : JsonSchema.tagsOk t = true) {v : Value} {n : Term} (hv : keysOk v = true)
    (h : Enc G v n) : Conforms G (shapeOf D k t) n ↔ checkF D k t v = true := by
  rw [shapeOf, conforms_nodeS]
  exact agrees_all hP hD hG k t ht v n hv h

/-- **SHACL agreement on encoding graphs**: the value has the type exactly when its node conforms
to some unfolding of the type's shapes. -/
theorem shacl_agrees_enc (hP : PatternsAgree) {D : Decls}
    (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) {G : Graph} (hG : BNodeSubjects G) {t : Ty}
    (ht : JsonSchema.tagsOk t = true) {v : Value} {n : Term} (hv : keysOk v = true)
    (h : Enc G v n) : (∃ k, Conforms G (shapeOf D k t) n) ↔ HasType D v t := by
  constructor
  · rintro ⟨k, hk⟩
    exact checkF_sound D k t v ((conforms_iff_checkF hP hD hG k ht hv h).1 hk)
  · intro hty
    obtain ⟨k, hk⟩ := checkF_complete D hty
    exact ⟨k, (conforms_iff_checkF hP hD hG k ht hv h).2 hk⟩

/-- Unfolding further keeps conformance. -/
theorem conforms_mono_enc (hP : PatternsAgree) {D : Decls}
    (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) {G : Graph} (hG : BNodeSubjects G) (k : Nat)
    {t : Ty} (ht : JsonSchema.tagsOk t = true) {v : Value} {n : Term} (hv : keysOk v = true)
    (h : Enc G v n) : Conforms G (shapeOf D k t) n → Conforms G (shapeOf D (k + 1) t) n := by
  rw [conforms_iff_checkF hP hD hG k ht hv h, conforms_iff_checkF hP hD hG (k + 1) ht hv h]
  exact checkF_mono D k t v

/-! ## The concrete encoding encodes -/

section Concrete

theorem code_cons_inj {i j : Nat} {l l' : List Char}
    (h : List.replicate i 'a' ++ 'b' :: l = List.replicate j 'a' ++ 'b' :: l') : i = j ∧ l = l' := by
  induction i generalizing j with
  | zero =>
    cases j with
    | zero => simpa using h
    | succ j => simp [List.replicate_succ] at h
  | succ i ih =>
    cases j with
    | zero => simp [List.replicate_succ] at h
    | succ j =>
      simp only [List.replicate_succ, List.cons_append, List.cons.injEq, true_and] at h
      obtain ⟨h1, h2⟩ := ih h
      exact ⟨by omega, h2⟩

theorem code_inj : ∀ {p q : List Nat}, code p = code q → p = q
  | [], [], _ => rfl
  | [], _ :: _, h => by simp [code] at h
  | _ :: _, [], h => by simp [code] at h
  | i :: p, j :: q, h => by
    obtain ⟨rfl, h'⟩ := code_cons_inj h
    rw [code_inj h']

theorem bn_inj {p q : List Nat} (h : bn p = bn q) : p = q := by
  simp only [bn, Term.bnode.injEq, BlankNode.mk.injEq] at h
  have := congrArg String.toList h
  simp only [String.toList_ofList] at this
  exact code_inj this

/-- The value at a position below a position. -/
theorem sub_append : ∀ (v : Value) (r r' : List Nat),
    sub v (r ++ r') = (sub v r).bind (fun w => sub w r')
  | v, [], r' => by simp [sub]
  | .array [], i :: r, r' => by simp [sub]
  | .array (x :: xs), 0 :: r, r' => by simp only [List.cons_append, sub]; exact sub_append x r r'
  | .array (x :: xs), 1 :: r, r' => by
    simp only [List.cons_append, sub]; exact sub_append (.array xs) r r'
  | .array (_ :: _), (_ + 2) :: r, r' => by simp [sub]
  | .object es, i :: r, r' => by
    simp only [List.cons_append, sub]
    cases es[i]? with
    | none => rfl
    | some e =>
      simp only
      split
      · rfl
      · exact sub_append e.2 r r'
  | .undefined, _ :: _, _ | .null, _ :: _, _ | .bool _, _ :: _, _ | .num _ _, _ :: _, _
  | .str _, _ :: _, _ => by simp [sub]

theorem locals_subject {q : List Nat} {w : Value} {t : Triple} (h : t ∈ locals q w) : t.s = bn q := by
  unfold locals at h
  split at h
  · simp at h; rcases h with rfl | rfl <;> rfl
  · simp only [List.mem_flatMap] at h
    obtain ⟨⟨⟨k, x⟩, i⟩, _, ht⟩ := h
    split at ht
    · simp at ht
    · simp at ht; subst ht; rfl
  · simp at h

theorem mem_encT : ∀ (v : Value) (pos : List Nat) (t : Triple),
    t ∈ encT pos v ↔ ∃ r w, sub v r = some w ∧ t ∈ locals (pos ++ r) w
  | .array [], pos, t => by
    rw [encT]
    · simp only [List.not_mem_nil, false_iff, not_exists, not_and]
      intro r w hs ht
      cases r with
      | nil => simp [sub] at hs; subst hs; simp [locals] at ht
      | cons i r => simp [sub] at hs
    all_goals first | (intro _ _ hh; cases hh) | (intro _ hh; cases hh)
  | .array (x :: xs), pos, t => by
    rw [encT]
    simp only [List.mem_append, mem_encT x, mem_encT (.array xs)]
    constructor
    · rintro ((h | ⟨r, w, hs, ht⟩) | ⟨r, w, hs, ht⟩)
      · exact ⟨[], _, rfl, by simpa using h⟩
      · exact ⟨0 :: r, w, by simpa [sub] using hs, by simpa using ht⟩
      · exact ⟨1 :: r, w, by simpa [sub] using hs, by simpa using ht⟩
    · rintro ⟨r, w, hs, ht⟩
      match r, hs with
      | [], hs => simp [sub] at hs; subst hs; exact .inl (.inl (by simpa using ht))
      | 0 :: r, hs => exact .inl (.inr ⟨r, w, by simpa [sub] using hs, by simpa using ht⟩)
      | 1 :: r, hs => exact .inr ⟨r, w, by simpa [sub] using hs, by simpa using ht⟩
      | (_ + 2) :: r, hs => simp [sub] at hs
  | .object es, pos, t => by
    rw [encT]
    simp only [List.mem_append, List.mem_flatMap, List.mem_attach, true_and, Subtype.exists]
    constructor
    · rintro (h | ⟨⟨⟨k, x⟩, i⟩, hm, ht⟩)
      · exact ⟨[], _, rfl, by simpa using h⟩
      · simp only at ht
        split at ht
        · simp at ht
        · rename_i hu
          have hx : x ∈ es.map (·.2) := List.mem_map.2 ⟨(k, x), List.fst_mem_of_mem_zipIdx hm, rfl⟩
          have := List.sizeOf_lt_of_mem (List.fst_mem_of_mem_zipIdx hm)
          obtain ⟨r, w, hs, ht⟩ := (mem_encT x (pos ++ [i]) t).1 ht
          refine ⟨i :: r, w, ?_, by simpa using ht⟩
          rw [List.mk_mem_zipIdx_iff_getElem?] at hm
          simp [sub, hm, hu, hs]
    · rintro ⟨r, w, hs, ht⟩
      match r, hs with
      | [], hs => simp [sub] at hs; subst hs; exact .inl (by simpa using ht)
      | i :: r, hs =>
        simp only [sub] at hs
        cases he : es[i]? with
        | none => rw [he] at hs; cases hs
        | some e =>
          obtain ⟨k, x⟩ := e
          rw [he] at hs
          simp only at hs
          split at hs
          · cases hs
          · rename_i hu
            have hm : ((k, x), i) ∈ es.zipIdx := List.mk_mem_zipIdx_iff_getElem?.2 he
            have := List.sizeOf_lt_of_mem (List.fst_mem_of_mem_zipIdx hm)
            refine .inr ⟨((k, x), i), hm, ?_⟩
            simp only [hu, Bool.false_eq_true, ↓reduceIte]
            exact (mem_encT x (pos ++ [i]) t).2 ⟨r, w, hs, by simpa using ht⟩
  | .undefined, pos, t | .null, pos, t | .bool _, pos, t | .num _ _, pos, t | .str _, pos, t => by
    rw [encT]
    · simp only [List.not_mem_nil, false_iff, not_exists, not_and]
      intro r w hs ht
      cases r with
      | nil => simp [sub] at hs; subst hs; simp [locals] at ht
      | cons i r => simp [sub] at hs
    all_goals first | (intro _ _ hh; cases hh) | (intro _ hh; cases hh)
termination_by v => sizeOf v
decreasing_by
  all_goals first
    | (simp only [Value.array.sizeOf_spec, List.cons.sizeOf_spec]; omega)
    | (simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at *; omega)

theorem mem_encGraph_bn (v0 : Value) {q : List Nat} {w : Value} (hq : sub v0 q = some w)
    (p o : Term) :
    (⟨bn q, p, o⟩ : Triple) ∈ encGraph v0 ↔ (⟨bn q, p, o⟩ : Triple) ∈ locals q w := by
  rw [encGraph, Graph.mem_ofList, mem_encT]
  constructor
  · rintro ⟨r, w', hs, ht⟩
    have := locals_subject ht
    simp only [List.nil_append] at this
    have hr := bn_inj this
    subst hr
    rw [hq] at hs; cases hs
    simpa using ht
  · intro ht
    exact ⟨q, w, hq, by simpa using ht⟩

/-- Every subject of an encoding is a blank node. -/
theorem encGraph_bnodes (v0 : Value) : BNodeSubjects (encGraph v0) := by
  intro t ht
  rw [encGraph, Graph.mem_ofList, mem_encT] at ht
  obtain ⟨r, w, _, ht⟩ := ht
  exact ⟨_, locals_subject ht⟩

/-- The node of an object's entry named `k`, entries numbered from `i`. -/
def entryNode (q : List Nat) : List (String × Value) → Nat → String → Term
  | [], _, _ => nilT
  | (k', x) :: es, i, k => if k' = k then node (q ++ [i]) x else entryNode q es (i + 1) k

theorem entryNode_eq (q : List Nat) : ∀ (es : List (String × Value)) (i j : Nat) (k : String)
    (x : Value), (es.map (·.1)).Nodup → es[j]? = some (k, x) →
    entryNode q es i k = node (q ++ [i + j]) x
  | [], _, _, _, _, _, h => by simp at h
  | (k', x') :: es, i, 0, k, x, _, h => by
    simp only [List.getElem?_cons_zero, Option.some.injEq, Prod.mk.injEq] at h
    obtain ⟨rfl, rfl⟩ := h
    simp [entryNode]
  | (k', x') :: es, i, j + 1, k, x, hnd, h => by
    simp only [List.getElem?_cons_succ] at h
    simp only [List.map_cons, List.nodup_cons] at hnd
    have hk : k' ≠ k := by
      intro hkk; subst hkk
      exact hnd.1 (List.mem_map.2 ⟨(k', x), List.mem_of_getElem? h, rfl⟩)
    simp only [entryNode, hk, ↓reduceIte]
    rw [entryNode_eq q es (i + 1) j k x hnd.2 h]
    congr 3; omega

theorem keysOk_array_iff {items : List Value} :
    keysOk (.array items) = true ↔ ∀ x ∈ items, keysOk x = true := by
  rw [keysOk]
  simp only [List.all_eq_true]
  exact ⟨fun h x hx => h ⟨x, hx⟩ (List.mem_attach _ _), fun h x _ => h x.1 x.2⟩

/-- Every value below the root is encoded at its position. -/
theorem enc_of_sub (v0 : Value) : ∀ (w : Value) (q : List Nat), sub v0 q = some w →
    keysOk w = true → Enc (encGraph v0) w (node q w)
  | .undefined, _, _, _ => .undef
  | .null, _, _, _ => .null
  | .bool b, _, _, _ => .bool b
  | .num m e, _, _, _ => .num m e
  | .str s, _, _, _ => .str s
  | .array [], _, _, _ => .nil
  | .array (x :: xs), q, hq, hv => by
    have hsub : ∀ r, sub v0 (q ++ r) = sub (.array (x :: xs)) r := fun r => by
      rw [sub_append, hq]; rfl
    have hok := keysOk_array_iff.1 hv
    refine .cons x xs _ (node (q ++ [0]) x) (node (q ++ [1]) (.array xs)) (fun p o => ?_)
      (enc_of_sub v0 x (q ++ [0]) (by rw [hsub]; rfl) (hok x (by simp)))
      (enc_of_sub v0 (.array xs) (q ++ [1]) (by rw [hsub]; rfl)
        (keysOk_array_iff.2 fun y hy => hok y (by simp [hy])))
    have := mem_encGraph_bn v0 hq p o
    simp only [bn] at this
    rw [this]
    simp [locals, bn]
  | .object es, q, hq, hv => by
    obtain ⟨hnd, _⟩ := keysOk_object hv
    have hsub : ∀ r, sub v0 (q ++ r) = sub (.object es) r := fun r => by
      rw [sub_append, hq]; rfl
    have hidx : ∀ k x, present es k = some x → ∃ i : Nat, es[i]? = some (k, x) ∧ isUndef x = false := by
      intro k x hp
      obtain ⟨hm, hu⟩ := present_some_mem hp
      obtain ⟨i, hi⟩ := List.getElem?_of_mem hm
      exact ⟨i, hi, hu⟩
    refine .object es _ (entryNode q es 0) (fun p o => ?_) (fun k x hp => ?_)
    · have := mem_encGraph_bn v0 hq p o
      simp only [bn] at this
      rw [this]
      simp only [locals, List.mem_flatMap]
      constructor
      · rintro ⟨⟨⟨k, x⟩, i⟩, hm, ht⟩
        rw [List.mk_mem_zipIdx_iff_getElem?] at hm
        simp only at ht
        split at ht
        · simp at ht
        · rename_i hu
          simp only [List.mem_singleton, Triple.mk.injEq] at ht
          obtain ⟨_, rfl, rfl⟩ := ht
          refine ⟨k, x, present_of_mem hnd (List.mem_of_getElem? hm) (by simpa using hu), rfl, ?_⟩
          rw [entryNode_eq q es 0 i k x hnd hm]; simp
      · rintro ⟨k, x, hp, rfl, rfl⟩
        obtain ⟨i, hi, hu⟩ := hidx k x hp
        refine ⟨((k, x), i), List.mk_mem_zipIdx_iff_getElem?.2 hi, ?_⟩
        simp only [hu, Bool.false_eq_true, ↓reduceIte, List.mem_singleton]
        rw [entryNode_eq q es 0 i k x hnd hi]; simp [bn]
    · obtain ⟨i, hi, hu⟩ := hidx k x hp
      have := List.sizeOf_lt_of_mem (List.mem_of_getElem? hi)
      rw [entryNode_eq q es 0 i k x hnd hi, Nat.zero_add]
      exact enc_of_sub v0 x (q ++ [i]) (by rw [hsub]; simp [sub, hi, hu])
        (keysOk_present hv hp)
termination_by w => sizeOf w
decreasing_by
  all_goals first
    | (simp only [Value.array.sizeOf_spec, List.cons.sizeOf_spec]; omega)
    | (simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at *; omega)

/-- The encoding of a value with distinct keys encodes it. -/
theorem enc_encGraph (v : Value) (hv : keysOk v = true) : Enc (encGraph v) v (encNode v) :=
  enc_of_sub v v [] rfl hv

end Concrete

/-! ## The theorem -/

/-- **SHACL projection agreement** (verification backlog 4.2): for a declaration block and a
type whose unions have distinct tags, a value whose objects have distinct keys has the type
exactly when its RDF encoding conforms, under LeanRDF's SHACL Core semantics, to some unfolding
of the type's shapes; given that LeanRDF's regular expressions decide the three patterns as the
lexical rules do. -/
-- @tg: implements:: [[openspec:formal-model#SHACL Projection Agreement]]
theorem shacl_agrees (hP : PatternsAgree) (D : Decls) (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true)
    (t : Ty) (ht : JsonSchema.tagsOk t = true) (v : Value) (hv : keysOk v = true) :
    (∃ k, Conforms (encGraph v) (shapeOf D k t) (encNode v)) ↔ HasType D v t :=
  shacl_agrees_enc hP hD (encGraph_bnodes v) ht hv (enc_encGraph v hv)

/-- The same with LeanRDF's validator: no validation results at some unfolding. -/
-- @tg: implements:: [[openspec:formal-model#SHACL Projection Agreement]]
theorem validator_agrees (hP : PatternsAgree) (D : Decls)
    (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) (t : Ty) (ht : JsonSchema.tagsOk t = true)
    (v : Value) (hv : keysOk v = true) :
    (∃ k, results (encGraph v) (shapeOf D k t) (encNode v) = []) ↔ HasType D v t := by
  simp only [results_nil]
  exact shacl_agrees hP D hD t ht v hv

/-- Unfolding the named types further keeps conformance. -/
-- @tg: implements:: [[openspec:formal-model#SHACL Projection Agreement]]
theorem conforms_mono (hP : PatternsAgree) (D : Decls)
    (hD : ∀ d ∈ D, JsonSchema.tagsOk d.2 = true) (k : Nat) (t : Ty)
    (ht : JsonSchema.tagsOk t = true) (v : Value) (hv : keysOk v = true) :
    Conforms (encGraph v) (shapeOf D k t) (encNode v) →
      Conforms (encGraph v) (shapeOf D (k + 1) t) (encNode v) :=
  conforms_mono_enc hP hD (encGraph_bnodes v) k ht hv (enc_encGraph v hv)

end Vertep.Types.Shacl
