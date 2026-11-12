import VertepLean.Types.Check

/-!
# The JSON Schema projection

Mirrors `crates/vertep-rdf/src/types/json_schema.rs`: the JSON Schema (draft 2020-12) of a type,
over the keyword subset structured-output interfaces support (`type`, `properties`,
`required`, `additionalProperties: false`, `items`, `anyOf`, `const`, `pattern`, `minimum`,
`exclusiveMaximum`, `$ref` into `$defs`, and the annotation `format`), and a validator for that
subset.

* `Schema` is a keyword tree: `all kws` is one schema object (the conjunction of its keywords),
  and `properties ps closed` is the `properties` keyword together with
  `additionalProperties: false` when `closed`, since that keyword reads its sibling.
* `validate` follows draft 2020-12 for these keywords: assertions on a kind of instance hold
  vacuously on other kinds; `integer` is a number with no fraction; `minimum` and
  `exclusiveMaximum` compare exact decimals. A `pattern` is interpreted by the lexical rule its
  regular expression encodes (`Pattern.matches`; the regular expressions are Rust's
  `DECIMAL_PATTERN`, `DATETIME_PATTERN`, `IRI_PATTERN`, whose equivalence to these rules is not
  proven here). A `$ref` that is already being followed at the same instance location fails:
  the schema of `type A = optional<A>` is `anyOf [null, $ref A]`, which a validator would
  otherwise follow forever, and the least-fixed-point reading of the schema has no derivation
  through such a loop.
* `jsonSchema_agrees`: for every JSON value (no `undefined` anywhere) and every type of a block
  whose unions have distinct tags (which `validate` of declarations enforces), the value
  validates against the projected schema with the block's `$defs` exactly when it has the type.
-/

namespace Vertep.Types.JsonSchema

open Vertep

/-- The instance types of the `type` keyword. -/
inductive JType where
  | null
  | boolean
  | integer
  | number
  | string
  | array
  | object
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The patterns the projection emits. -/
inductive Pattern where
  | decimal
  | datetime
  | iri
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The lexical rule each pattern's regular expression encodes. -/
def Pattern.matches : Pattern → String → Bool
  | .decimal, s => Lex.isDecimal s
  | .datetime, s => Lex.isDatetime s
  | .iri, s => Lex.isIri s

/-- A JSON Schema of the supported subset, as a keyword tree. -/
inductive Schema where
  /-- A schema object: all its keywords hold. -/
  | all (kws : List Schema)
  | type (t : JType)
  /-- `properties`, and `additionalProperties: false` when `closed`. -/
  | properties (ps : List (String × Schema)) (closed : Bool)
  | required (names : List String)
  | items (s : Schema)
  | anyOf (ss : List Schema)
  /-- `const` with a string. -/
  | const (c : String)
  | pattern (p : Pattern)
  | minimum (m : Int)
  | exclusiveMaximum (m : Int)
  /-- An annotation. -/
  | format (f : String)
  /-- `{"$ref": "#/$defs/<name>"}`. -/
  | ref (name : String)
  deriving Repr, Inhabited

/-- The `type` keyword. -/
def typeOk : JType → Value → Bool
  | .null, .null => true
  | .boolean, .bool _ => true
  | .integer, .num m e => m % (10 ^ e : Int) == 0
  | .number, .num _ _ => true
  | .string, .str _ => true
  | .array, .array _ => true
  | .object, .object _ => true
  | _, _ => false

theorem sizeOf_lookup {entries : List (String × Value)} {k : String} {v : Value}
    (h : entries.lookup k = some v) : sizeOf v < sizeOf (Value.object entries) := by
  have := List.sizeOf_lt_of_mem (lookup_mem' h)
  simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢
  omega

/-- The validator: `validate defs seen s v` when the instance `v` is valid against `s` with the
`$defs` `defs`; `seen` holds the `$ref`s being followed at this instance location. -/
-- @tg: implements:: [[openspec:formal-model#JSON Schema Projection Agreement]]
def validate (defs : List (String × Schema)) (seen : List String) (s : Schema) (v : Value) :
    Bool :=
  match s with
  | .all kws => kws.attach.all fun ⟨k, _hk⟩ => validate defs seen k v
  | .type t => typeOk t v
  | .properties ps closed =>
    match v with
    | .object entries =>
      (ps.all fun p =>
        match _h : entries.lookup p.1 with
        | some x => validate defs [] p.2 x
        | none => true) &&
      (!closed || entries.all fun e => ps.any (·.1 == e.1))
    | _ => true
  | .required names =>
    match v with
    | .object entries => names.all fun n => (entries.lookup n).isSome
    | _ => true
  | .items s =>
    match v with
    | .array xs => xs.attach.all fun ⟨x, _hx⟩ => validate defs [] s x
    | _ => true
  | .anyOf ss => ss.attach.any fun ⟨s', _hs⟩ => validate defs seen s' v
  | .const c =>
    match v with
    | .str x => x == c
    | _ => false
  | .pattern p =>
    match v with
    | .str x => p.matches x
    | _ => true
  | .minimum m =>
    match v with
    | .num x e => decide (m * 10 ^ e ≤ x)
    | _ => true
  | .exclusiveMaximum m =>
    match v with
    | .num x e => decide (x < m * 10 ^ e)
    | _ => true
  | .format _ => true
  | .ref n =>
    if _h : seen.contains n then false else
    match _hl : defs.lookup n with
    | some s' => validate defs (n :: seen) s' v
    | none => false
termination_by (sizeOf v, unexpanded defs seen, sizeOf s)
decreasing_by
  · exact Prod.Lex.right _ (Prod.Lex.right _ (by
      have := List.sizeOf_lt_of_mem _hk
      simp only [Schema.all.sizeOf_spec]; omega))
  · exact Prod.Lex.left _ _ (sizeOf_lookup _h)
  · exact Prod.Lex.left _ _ (by
      have := List.sizeOf_lt_of_mem _hx
      simp only [Value.array.sizeOf_spec]; omega)
  · exact Prod.Lex.right _ (Prod.Lex.right _ (by
      have := List.sizeOf_lt_of_mem _hs
      simp only [Schema.anyOf.sizeOf_spec]; omega))
  · exact Prod.Lex.right _ (Prod.Lex.left _ _ (unexpanded_lt _hl _h))

/-! ## The projection -/

/-- The schema of one union variant: a closed object with the tag as a `const` and, for a
variant with a payload type `pt` (whose schema is `sp`), the payload under `value`, required
unless it may be absent. -/
def variant (D : Decls) (tag : String) (p : Option Ty) (sp : Option Schema) : Schema :=
  match p, sp with
  | some pt, some s =>
    .all [.type .object, .properties [("tag", .all [.const tag]), ("value", s)] true,
      .required (if optish D pt then ["tag"] else ["tag", "value"])]
  | _, _ => .all [.type .object, .properties [("tag", .all [.const tag])] true, .required ["tag"]]

/-- The schema of one type expression; named types are `$ref`s into `$defs` (Rust `schema`). -/
-- @tg: implements:: [[openspec:formal-model#JSON Schema Projection Agreement]]
def proj (D : Decls) : Ty → Schema
  | .value => .all []
  | .named n => .all [.ref n]
  | .prim .bool => .all [.type .boolean]
  | .prim .string => .all [.type .string]
  | .prim .int => .all [.type .integer, .minimum (-(2 ^ 63)), .exclusiveMaximum (2 ^ 63)]
  | .prim .decimal => .all [.anyOf [.all [.type .number], .all [.type .string, .pattern .decimal]]]
  | .prim .datetime => .all [.type .string, .format "date-time", .pattern .datetime]
  | .prim .iri => .all [.type .string, .pattern .iri]
  | .optional t => .all [.anyOf [.all [.type .null], proj D t]]
  | .list t => .all [.type .array, .items (proj D t)]
  | .record fs =>
    .all [.type .object, .properties (fs.attach.map fun ⟨(n, t), _⟩ => (n, proj D t)) true,
      .required ((fs.filter fun f => !optish D f.2).map (·.1))]
  | .union vs =>
    .all [.anyOf (vs.attach.map fun ⟨(tag, p), _⟩ =>
      match p with
      | none => variant D tag none none
      | some pt => variant D tag (some pt) (some (proj D pt)))]
termination_by t => sizeOf t
decreasing_by
  all_goals first
    | simp only [Ty.optional.sizeOf_spec, Ty.list.sizeOf_spec]; omega
    | (rename_i h; have := List.sizeOf_lt_of_mem h
       simp only [Ty.record.sizeOf_spec, Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec,
         Option.some.sizeOf_spec] at this ⊢
       omega)

/-- The `$defs` of a block: each named type's schema, first declaration first. -/
def defsOf (D : Decls) : List (String × Schema) := D.map fun d => (d.1, proj D d.2)

/-! ## Agreement with the checker -/

/-- A JSON value: no `undefined` anywhere. -/
def jsonValue (v : Value) : Bool :=
  match v with
  | .undefined => false
  | .array items => items.attach.all fun ⟨x, _hx⟩ => jsonValue x
  | .object entries => entries.attach.all fun ⟨e, _he⟩ => jsonValue e.2
  | _ => true
termination_by sizeOf v
decreasing_by
  · have := List.sizeOf_lt_of_mem _hx
    simp only [Value.array.sizeOf_spec]; omega
  · have := List.sizeOf_lt_of_mem _he
    obtain ⟨k, x⟩ := e
    simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢; omega

/-- Every union in the type has distinct tags (Rust `validate` reports `DuplicateTag`). -/
def tagsOk (t : Ty) : Bool :=
  match t with
  | .union vs =>
    decide ((vs.map (·.1)).Nodup) && vs.attach.all fun ⟨v, _hv⟩ =>
      match _hp : v.2 with
      | none => true
      | some pt => tagsOk pt
  | .record fs => fs.attach.all fun ⟨f, _hf⟩ => tagsOk f.2
  | .list t => tagsOk t
  | .optional t => tagsOk t
  | _ => true
termination_by sizeOf t
decreasing_by
  · have := List.sizeOf_lt_of_mem _hv
    obtain ⟨tag, p⟩ := v
    simp only at _hp; subst _hp
    simp only [Ty.union.sizeOf_spec, Prod.mk.sizeOf_spec, Option.some.sizeOf_spec] at this ⊢
    omega
  · have := List.sizeOf_lt_of_mem _hf
    obtain ⟨n, ft⟩ := f
    simp only [Ty.record.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢; omega
  · simp only [Ty.list.sizeOf_spec]; omega
  · simp only [Ty.optional.sizeOf_spec]; omega

theorem tagsOk_union {vs : List (String × Option Ty)} (h : tagsOk (.union vs) = true) :
    (vs.map (·.1)).Nodup ∧ ∀ v ∈ vs, ∀ pt, v.2 = some pt → tagsOk pt = true := by
  rw [tagsOk] at h
  simp only [Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true] at h
  refine ⟨h.1, fun v hv pt hp => ?_⟩
  have := h.2 ⟨v, hv⟩ (List.mem_attach _ _)
  split at this
  · simp_all
  · rename_i pt' hp'; rw [hp] at hp'; cases hp'; exact this

theorem tagsOk_record {fs : List (String × Ty)} (h : tagsOk (.record fs) = true) :
    ∀ f ∈ fs, tagsOk f.2 = true := by
  rw [tagsOk] at h
  simp only [List.all_eq_true] at h
  exact fun f hf => h ⟨f, hf⟩ (List.mem_attach _ _)

theorem jsonValue_items {items : List Value} (h : jsonValue (.array items) = true) :
    ∀ x ∈ items, jsonValue x = true := by
  rw [jsonValue] at h
  simp only [List.all_eq_true] at h
  exact fun x hx => h ⟨x, hx⟩ (List.mem_attach _ _)

theorem jsonValue_lookup {entries : List (String × Value)} (h : jsonValue (.object entries) = true)
    {k : String} {x : Value} (hl : entries.lookup k = some x) : jsonValue x = true := by
  rw [jsonValue] at h
  simp only [List.all_eq_true] at h
  exact h ⟨(k, x), lookup_mem' hl⟩ (List.mem_attach _ _)

theorem jsonValue_present {entries : List (String × Value)} (h : jsonValue (.object entries) = true)
    (k : String) : present entries k = entries.lookup k := by
  unfold present
  split
  · rename_i hl; have := jsonValue_lookup h hl; simp [jsonValue] at this
  · rfl

theorem jsonValue_undef {entries : List (String × Value)} (h : jsonValue (.object entries) = true)
    (e : String × Value) (he : e ∈ entries) : isUndef e.2 = false := by
  rw [jsonValue] at h
  simp only [List.all_eq_true] at h
  have := h ⟨e, he⟩ (List.mem_attach _ _)
  obtain ⟨k, x⟩ := e
  cases x <;> simp_all [jsonValue, isUndef]

theorem jsonValue_absent {v : Value} (h : jsonValue v = true) : absent v = typeOk .null v := by
  cases v <;> simp_all [absent, typeOk, jsonValue]

theorem lookup_of_mem {β : Type} {l : List (String × β)} (hnd : (l.map (·.1)).Nodup)
    {k : String} {b : β} (hm : (k, b) ∈ l) : l.lookup k = some b := by
  induction l with
  | nil => simp at hm
  | cons d l ih =>
    obtain ⟨k', b'⟩ := d
    simp only [List.map_cons, List.nodup_cons, List.mem_map] at hnd
    rcases List.mem_cons.1 hm with h | h
    · cases h; simp [List.lookup]
    · have hk : k ≠ k' := fun e => hnd.1 ⟨(k, b), h, by simp [e]⟩
      have : (k == k') = false := by simpa using hk
      simp only [List.lookup, this]
      exact ih hnd.2 h

theorem lookup_map {β γ : Type} (f : β → γ) (l : List (String × β)) (n : String) :
    (l.map fun d => (d.1, f d.2)).lookup n = (l.lookup n).map f := by
  induction l with
  | nil => simp [List.lookup]
  | cons d l ih =>
    obtain ⟨k, t⟩ := d
    by_cases h : n = k
    · subst h; simp [List.lookup]
    · have : (n == k) = false := by simpa using h
      simp only [List.map_cons, List.lookup, this]
      exact ih

theorem lookup_some_of_mem {β : Type} {l : List (String × β)} {k : String} {b : β}
    (hm : (k, b) ∈ l) : ∃ y, l.lookup k = some y := by
  induction l with
  | nil => simp at hm
  | cons d l ih =>
    obtain ⟨k', b'⟩ := d
    by_cases h : k = k'
    · subst h; simp [List.lookup]
    · have : (k == k') = false := by simpa using h
      simp only [List.lookup, this]
      rcases List.mem_cons.1 hm with e | e
      · cases e; exact absurd rfl h
      · exact ih e

theorem all_attach {α : Type} (l : List α) (f : α → Bool) :
    (l.attach.all fun x => f x.1) = l.all f := by
  apply Bool.eq_iff_iff.2
  simp only [List.all_eq_true, List.mem_attach, true_implies, Subtype.forall]

theorem defsOf_lookup (D : Decls) (n : String) :
    (defsOf D).lookup n = (D.lookup n).map (proj D) :=
  lookup_map (proj D) D n

theorem proj_record (D : Decls) (fs : List (String × Ty)) :
    proj D (.record fs) = .all [.type .object,
      .properties (fs.map fun f => (f.1, proj D f.2)) true,
      .required ((fs.filter fun f => !optish D f.2).map (·.1))] := by
  rw [proj]
  simp only [Schema.all.injEq, List.cons.injEq, Schema.properties.injEq, and_true, true_and]
  rw [show (fun x : {x // x ∈ fs} => (x.1.fst, proj D x.1.snd)) =
      (fun f => (f.1, proj D f.2)) ∘ Subtype.val from rfl, ← List.map_map,
    List.attach_map_subtype_val]

theorem proj_union (D : Decls) (vs : List (String × Option Ty)) :
    proj D (.union vs) = .all [.anyOf (vs.map fun v => variant D v.1 v.2 (v.2.map (proj D)))] := by
  rw [proj]
  simp only [Schema.all.injEq, List.cons.injEq, Schema.anyOf.injEq, and_true]
  conv => rhs; rw [← List.attach_map_subtype_val vs, List.map_map]
  apply List.map_congr_left
  intro ⟨⟨tag, p⟩, h⟩ _
  cases p <;> rfl

section
variable {defs : List (String × Schema)} {seen : List String}

theorem validate_all (kws : List Schema) (v : Value) :
    validate defs seen (.all kws) v = kws.all fun k => validate defs seen k v := by
  rw [validate]; simp

theorem validate_anyOf (ss : List Schema) (v : Value) :
    validate defs seen (.anyOf ss) v = ss.any fun s => validate defs seen s v := by
  rw [validate]; simp

theorem validate_type (t : JType) (v : Value) : validate defs seen (.type t) v = typeOk t v := by
  rw [validate]

theorem validate_items (s : Schema) (xs : List Value) :
    validate defs seen (.items s) (.array xs) = xs.all fun x => validate defs [] s x := by
  rw [validate]; simp

theorem validate_const (c : String) (v : Value) :
    validate defs seen (.const c) v = true ↔ v = .str c := by
  rw [validate.eq_def]; cases v <;> simp

theorem validate_props (ps : List (String × Schema)) (closed : Bool)
    (entries : List (String × Value)) :
    validate defs seen (.properties ps closed) (.object entries) = true ↔
      (∀ p ∈ ps, ∀ x, entries.lookup p.1 = some x → validate defs [] p.2 x = true) ∧
      (closed = true → ∀ e ∈ entries, ∃ p ∈ ps, p.1 = e.1) := by
  rw [validate]
  simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, Bool.not_eq_true',
    List.any_eq_true, beq_iff_eq]
  constructor
  · rintro ⟨hp, hc⟩
    refine ⟨fun p hp' x hx => ?_, fun hcl => ?_⟩
    · have := hp p hp'
      split at this
      · rename_i x' hx'; rw [hx] at hx'; cases hx'; exact this
      · simp_all
    · rcases hc with hc | hc
      · simp_all
      · exact hc
  · rintro ⟨hp, hc⟩
    refine ⟨fun p hp' => ?_, ?_⟩
    · split
      · rename_i x hx; exact hp p hp' x hx
      · rfl
    · cases closed
      · exact .inl rfl
      · exact .inr (hc rfl)

theorem validate_required (names : List String) (entries : List (String × Value)) :
    validate defs seen (.required names) (.object entries) = true ↔
      ∀ n ∈ names, (entries.lookup n).isSome = true := by
  rw [validate]; simp

theorem validate_ref_seen {n : String} {v : Value} (hs : seen.contains n = true) :
    validate defs seen (.ref n) v = false := by
  rw [validate]; split
  · rfl
  · contradiction

theorem validate_ref_none {n : String} {v : Value} (hl : defs.lookup n = none) :
    validate defs seen (.ref n) v = false := by
  rw [validate]; split
  · rfl
  · split <;> simp_all

theorem validate_ref_some {n : String} {v : Value} {s : Schema}
    (hs : ¬seen.contains n = true) (hl : defs.lookup n = some s) :
    validate defs seen (.ref n) v = validate defs (n :: seen) s v := by
  rw [validate]; split
  · contradiction
  · split <;> simp_all

/-- A variant's schema on an object. -/
theorem validate_variant (D : Decls) (tag : String) (p : Option Ty)
    (entries : List (String × Value)) :
    validate defs seen (variant D tag p (p.map (proj D))) (.object entries) = true ↔
      (∃ x, entries.lookup "tag" = some x) ∧ (∀ x, entries.lookup "tag" = some x → x = .str tag) ∧
      match p with
      | none => ∀ e ∈ entries, e.1 = "tag"
      | some pt => (∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value") ∧
          (∀ y, entries.lookup "value" = some y → validate defs [] (proj D pt) y = true) ∧
          (optish D pt = false → ∃ y, entries.lookup "value" = some y) := by
  cases p with
  | none =>
    simp only [variant, validate_all, List.all_cons, List.all_nil, Bool.and_true,
      validate_type, typeOk, Bool.true_and, Bool.and_eq_true, validate_props, validate_required,
      List.mem_cons, List.not_mem_nil, or_false, forall_eq, forall_const,
      exists_eq_left, Option.isSome_iff_exists]
    constructor
    · rintro ⟨⟨ht, hc⟩, hr⟩
      refine ⟨hr, fun x hx => ?_, fun e he => (hc e he).symm⟩
      exact (validate_const tag x).1 (ht x hx)
    · rintro ⟨hr, ht, hc⟩
      refine ⟨⟨fun x hx => ?_, fun e he => (hc e he).symm⟩, hr⟩
      exact (validate_const tag x).2 (ht x hx)
  | some pt =>
    simp only [variant, Option.map_some, validate_all, List.all_cons, List.all_nil, Bool.and_true,
      validate_type, typeOk, Bool.true_and, Bool.and_eq_true, validate_props, validate_required,
      List.mem_cons, List.not_mem_nil, or_false, forall_eq_or_imp, forall_eq, forall_const,
      Option.isSome_iff_exists]
    simp only [validate_const]
    constructor
    · rintro ⟨⟨⟨ht, hv⟩, hcl⟩, hr⟩
      refine ⟨?_, ht, ?_, hv, fun ho => ?_⟩
      · split at hr <;> simp_all
      · intro e he
        obtain ⟨q, hq, hqe⟩ := hcl e he
        try simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
        rcases hq with rfl | rfl
        · exact .inl hqe.symm
        · exact .inr hqe.symm
      · split at hr
        · simp_all
        · simp only [List.mem_cons, List.not_mem_nil, or_false, forall_eq_or_imp,
            forall_eq] at hr
          exact hr.2
    · rintro ⟨hr, ht, hcl, hv, ho⟩
      refine ⟨⟨⟨ht, hv⟩, fun e he => ?_⟩, ?_⟩
      · rcases hcl e he with h | h
        · exact ⟨("tag", .all [.const tag]), by simp, h.symm⟩
        · exact ⟨("value", proj D pt), by simp, h.symm⟩
      · split
        · simp only [List.mem_cons, List.not_mem_nil, or_false, forall_eq]; exact hr
        · rename_i hno
          simp only [List.mem_cons, List.not_mem_nil, or_false, forall_eq_or_imp, forall_eq]
          exact ⟨hr, ho (by simpa using hno)⟩

end

theorem check_record_iff {D : Decls} {seen : List String} {fs : List (String × Ty)}
    {entries : List (String × Value)} :
    check D seen (.record fs) (.object entries) = true ↔
      (∀ e ∈ entries, isUndef e.2 = true ∨ ∃ f ∈ fs, f.1 = e.1) ∧
      (∀ f ∈ fs, present entries f.1 = none → optish D f.2 = true) ∧
      (∀ f ∈ fs, ∀ fv, present entries f.1 = some fv → check D [] f.2 fv = true) := by
  rw [check]
  simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, List.any_eq_true, beq_iff_eq]
  constructor
  · rintro ⟨hk, hf⟩
    refine ⟨hk, fun f hf' hp => ?_, fun f hf' fv hp => ?_⟩
    · have := hf f hf'
      split at this
      · exact this
      · simp_all
    · have := hf f hf'
      split at this
      · simp_all
      · rename_i fv' hp'; rw [hp] at hp'; cases hp'; exact this
  · rintro ⟨hk, ho, hc⟩
    refine ⟨hk, fun f hf => ?_⟩
    split
    · rename_i hp; exact ho f hf hp
    · rename_i fv hp; exact hc f hf fv hp

theorem check_union_iff {D : Decls} {seen : List String} {vs : List (String × Option Ty)}
    {entries : List (String × Value)} :
    check D seen (.union vs) (.object entries) = true ↔
      ∃ tag, entries.lookup "tag" = some (.str tag) ∧
        (∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value" ∨ isUndef e.2 = true) ∧
        ((vs.lookup tag = some none ∧ present entries "value" = none) ∨
          ∃ pt, vs.lookup tag = some (some pt) ∧
            ((present entries "value" = none ∧ optish D pt = true) ∨
              ∃ pv, present entries "value" = some pv ∧ check D [] pt pv = true)) := by
  rw [check]
  split
  · rename_i tag htag
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    constructor
    · rintro ⟨hk, hm⟩
      refine ⟨tag, htag, hk, ?_⟩
      split at hm
      · simp at hm
      · rename_i hv; exact .inl ⟨hv, by simpa using hm⟩
      · rename_i pt hv
        refine .inr ⟨pt, hv, ?_⟩
        split at hm
        · rename_i hp; exact .inl ⟨hp, hm⟩
        · rename_i pv hp; exact .inr ⟨pv, hp, hm⟩
    · rintro ⟨tag', htag', hk, hm⟩
      rw [htag] at htag'; cases htag'
      refine ⟨hk, ?_⟩
      rcases hm with ⟨hv, hp⟩ | ⟨pt, hv, ⟨hp, ho⟩ | ⟨pv, hp, hc⟩⟩
      · simp [hv, hp]
      · simp only [hv]; split
        · exact ho
        · simp_all
      · simp only [hv]; split
        · simp_all
        · rename_i pv' hp'; rw [hp] at hp'; cases hp'; exact hc
  · rename_i hno
    simp only [Bool.false_eq_true, false_iff, not_exists, not_and]
    intro tag htag; exact absurd htag (hno tag)

/-- **The projection agrees with the checker** on JSON values, for every set of names under
expansion (the `$ref`s followed are the names expanded). -/
-- @tg: implements:: [[openspec:formal-model#JSON Schema Projection Agreement]]
theorem validate_proj (D : Decls) (hD : ∀ d ∈ D, tagsOk d.2 = true) (seen : List String)
    (t : Ty) (v : Value) :
    tagsOk t = true → jsonValue v = true →
      validate (defsOf D) seen (proj D t) v = check D seen t v := by
  induction seen, t, v using check.induct D with
  | case1 seen v => intro _ _; rw [proj, validate_all, check]; rfl
  | case2 seen v p =>
    intro _ hj
    rw [check]
    cases p <;> cases v <;>
      simp_all [proj, validate_all, validate_anyOf, validate_type, typeOk, primOk, Lex.intOk,
        Bool.and_assoc,
        jsonValue, validate, Pattern.matches]
  | case3 seen v n hs =>
    intro _ _
    rw [check_named_seen hs, proj, validate_all]
    simp [validate_ref_seen hs]
  | case4 seen v n hs hl =>
    intro _ _
    rw [check_named_none hl, proj, validate_all]
    simp [validate_ref_none (by rw [defsOf_lookup, hl]; rfl)]
  | case5 seen v n hs u hl ih =>
    intro _ hj
    rw [check_named_some hs hl, proj, validate_all]
    simp only [List.all_cons, List.all_nil, Bool.and_true]
    rw [validate_ref_some hs (by rw [defsOf_lookup, hl]; rfl)]
    exact ih (hD _ (lookup_mem hl)) hj
  | case6 seen v t₀ ih =>
    intro ht hj
    rw [check, proj]
    simp only [validate_all, List.all_cons, List.all_nil, Bool.and_true, validate_anyOf,
      List.any_cons, List.any_nil, Bool.or_false, validate_type]
    rw [jsonValue_absent hj, ih (by rw [tagsOk] at ht; exact ht) hj]
  | case7 seen t₀ items ih =>
    intro ht hj
    rw [check, proj, validate_all]
    simp only [List.all_cons, List.all_nil, Bool.and_true, validate_type, typeOk, Bool.true_and,
      validate_items]
    have ht' : tagsOk t₀ = true := by rw [tagsOk] at ht; exact ht
    rw [← all_attach items]
    apply congrArg
    funext ⟨x, hx⟩
    exact ih x hx ht' (jsonValue_items hj x hx)
  | case8 seen v t₀ hv =>
    intro _ _
    rw [check, proj, validate_all]
    · cases v <;> simp_all [validate_type, typeOk]
    · exact hv
  | case9 seen fs entries ih =>
    intro ht hj
    have htf := tagsOk_record ht
    have hpres := jsonValue_present hj
    rw [proj_record, validate_all]
    simp only [List.all_cons, List.all_nil, Bool.and_true, validate_type, typeOk, Bool.true_and]
    apply Bool.eq_iff_iff.2
    rw [Bool.and_eq_true, validate_props, validate_required, check_record_iff]
    simp only [hpres]
    have ihf : ∀ f ∈ fs, ∀ x, entries.lookup f.1 = some x →
        validate (defsOf D) [] (proj D f.2) x = check D [] f.2 x := fun f hf x hx =>
      ih f x (by rw [hpres]; exact hx) (htf f hf) (jsonValue_lookup hj hx)
    constructor
    · rintro ⟨⟨hp, hc⟩, hr⟩
      refine ⟨fun e he => ?_, fun f hf hn => ?_, fun f hf fv hl => ?_⟩
      · obtain ⟨p, hp', hpe⟩ := hc trivial e he
        obtain ⟨f, hf, rfl⟩ := List.mem_map.1 hp'
        exact .inr ⟨f, hf, hpe⟩
      · cases ho : optish D f.2
        · have := hr f.1 (List.mem_map.2 ⟨f, List.mem_filter.2 ⟨hf, by simp [ho]⟩, rfl⟩)
          simp [hn] at this
        · rfl
      · rw [← ihf f hf fv hl]
        exact hp (f.1, proj D f.2) (List.mem_map.2 ⟨f, hf, rfl⟩) fv hl
    · rintro ⟨hc, ho, hk⟩
      refine ⟨⟨fun p hp x hl => ?_, fun _ e he => ?_⟩, fun r hr => ?_⟩
      · obtain ⟨f, hf, rfl⟩ := List.mem_map.1 hp
        rw [ihf f hf x hl]; exact hk f hf x hl
      · rcases hc e he with hu | ⟨f, hf, hfe⟩
        · simp [jsonValue_undef hj e he] at hu
        · exact ⟨(f.1, proj D f.2), List.mem_map.2 ⟨f, hf, rfl⟩, hfe⟩
      · obtain ⟨f, hf, rfl⟩ := List.mem_map.1 hr
        obtain ⟨hf, hno⟩ := List.mem_filter.1 hf
        cases hl : entries.lookup f.1
        · have := ho f hf hl; simp_all
        · rfl
  | case10 seen v fs hv =>
    intro _ _
    rw [check, proj_record, validate_all]
    · cases v <;> simp_all [validate_type, typeOk]
    · exact hv
  | case11 seen vs entries tag htag ih =>
    intro ht hj
    obtain ⟨hnd, hpt⟩ := tagsOk_union ht
    have hpres := jsonValue_present hj
    rw [proj_union, validate_all]
    simp only [List.all_cons, List.all_nil, Bool.and_true, validate_anyOf]
    apply Bool.eq_iff_iff.2
    rw [check_union_iff]
    simp only [List.any_eq_true, List.mem_map, hpres]
    constructor
    · rintro ⟨_, ⟨⟨tag', p⟩, hm, rfl⟩, hval⟩
      rw [validate_variant] at hval
      obtain ⟨_, htg, hrest⟩ := hval
      have e : tag = tag' := by simpa using htg _ htag
      subst e
      have hlk := lookup_of_mem hnd hm
      refine ⟨tag, htag, ?_, ?_⟩
      · cases p with
        | none => intro e he; exact .inl (hrest e he)
        | some pt => exact fun e he => (hrest.1 e he).imp_right .inl
      · cases p with
        | none =>
          refine .inl ⟨hlk, ?_⟩
          cases hv : entries.lookup "value"
          · rfl
          · have := hrest ("value", _) (lookup_mem' hv); simp at this
        | some pt =>
          obtain ⟨_, hval, hreq⟩ := hrest
          refine .inr ⟨pt, hlk, ?_⟩
          cases hv : entries.lookup "value" with
          | none =>
            refine .inl ⟨rfl, ?_⟩
            cases ho : optish D pt
            · obtain ⟨y, hy⟩ := hreq ho; simp [hv] at hy
            · rfl
          | some pv =>
            refine .inr ⟨pv, rfl, ?_⟩
            rw [← ih pt pv (by rw [hpres]; exact hv) (hpt _ hm pt rfl) (jsonValue_lookup hj hv)]
            exact hval pv hv
    · rintro ⟨tag', htag', hk, hm⟩
      rw [htag] at htag'; cases htag'
      have hval : ∀ x : Value, entries.lookup "tag" = some x → x = .str tag := fun x hx => by
        rw [htag] at hx; cases hx; rfl
      rcases hm with ⟨hv, hp⟩ | ⟨pt, hv, hm⟩
      · refine ⟨_, ⟨(tag, none), lookup_mem' hv, rfl⟩, ?_⟩
        rw [validate_variant]
        refine ⟨⟨_, htag⟩, hval, fun e he => ?_⟩
        rcases hk e he with h | h | h
        · exact h
        · obtain ⟨k, x⟩ := e; simp only at h; subst h
          obtain ⟨y, hy⟩ := lookup_some_of_mem he
          simp_all
        · simp [jsonValue_undef hj e he] at h
      · refine ⟨_, ⟨(tag, some pt), lookup_mem' hv, rfl⟩, ?_⟩
        rw [validate_variant]
        have hk' : ∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value" := fun e he => by
          rcases hk e he with h | h | h
          · exact .inl h
          · exact .inr h
          · simp [jsonValue_undef hj e he] at h
        refine ⟨⟨_, htag⟩, hval, hk', fun y hy => ?_, fun ho => ?_⟩
        · rw [ih pt y (by rw [hpres]; exact hy) (hpt _ (lookup_mem' hv) pt rfl)
            (jsonValue_lookup hj hy)]
          rcases hm with ⟨hp, _⟩ | ⟨pv, hp, hc⟩
          · simp_all
          · rw [hy] at hp; cases hp; exact hc
        · rcases hm with ⟨_, ho'⟩ | ⟨pv, hp, _⟩
          · simp_all
          · exact ⟨pv, hp⟩
  | case12 seen vs entries htag =>
    intro _ _
    rw [proj_union, validate_all]
    simp only [List.all_cons, List.all_nil, Bool.and_true, validate_anyOf]
    apply Bool.eq_iff_iff.2
    rw [check_union_iff]
    simp only [List.any_eq_true, List.mem_map]
    constructor
    · rintro ⟨_, ⟨⟨tag', p⟩, _, rfl⟩, hval⟩
      rw [validate_variant] at hval
      obtain ⟨⟨x, hx⟩, htg, _⟩ := hval
      exact absurd (htg x hx ▸ hx) (htag tag')
    · rintro ⟨tag, h, _⟩; exact absurd h (htag tag)
  | case13 seen v vs hv =>
    intro _ _
    rw [check, proj_union, validate_all]
    · simp only [List.all_cons, List.all_nil, Bool.and_true, validate_anyOf, List.any_eq_false,
        List.mem_map]
      rintro _ ⟨⟨tag, p⟩, _, rfl⟩
      cases p <;> cases v <;> simp_all [variant, validate_all, validate_type, typeOk]
    · exact hv

/-- **JSON Schema projection agreement** (verification backlog 4.3): for a declaration block
and a type whose unions have distinct tags, a JSON value is valid against the projected schema
with the block's `$defs` exactly when it has the type. -/
-- @tg: implements:: [[openspec:formal-model#JSON Schema Projection Agreement]]
theorem jsonSchema_agrees (D : Decls) (hD : ∀ d ∈ D, tagsOk d.2 = true) (t : Ty)
    (ht : tagsOk t = true) (v : Value) (hv : jsonValue v = true) :
    validate (defsOf D) [] (proj D t) v = true ↔ HasType D v t := by
  rw [validate_proj D hD [] t v ht hv]
  exact check_iff D t v

end Vertep.Types.JsonSchema
