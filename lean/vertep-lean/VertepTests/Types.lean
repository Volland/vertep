import VertepLean

/-!
# The type language, `HasType` and the checker

`#guard`s mirroring `crates/vertep-core/src/types/tests.rs` (the value checker's verdicts), and
theorems about the least-fixed-point reading of non-contractive recursion.
-/

namespace Vertep.Test.Types

open Vertep Vertep.Types

def s (x : String) : Value := .str x
def obj (fields : List (String × Value)) : Value := .object fields
def num (m : Int) (e : Nat := 0) : Value := .num m e

def INT : Ty := .prim .int
def STRING : Ty := .prim .string
def DECIMAL : Ty := .prim .decimal
def DATETIME : Ty := .prim .datetime
def IRI : Ty := .prim .iri
def BOOL : Ty := .prim .bool

/-- `check_value`: no named types in scope. -/
def ok (t : Ty) (v : Value) : Bool := check [] [] t v

/-! ## Primitives -/

-- integer_outside_64_bit_range_is_rejected
#guard !ok (.record [("n", INT)]) (obj [("n", num 9223372036854775808)])
#guard ok INT (num (-9223372036854775808))
#guard ok INT (num 42)
#guard ok INT (num 420 1)
#guard !ok INT (num 15 1)
#guard !ok INT (num 9223372036854775808)

-- datetime_without_offset_is_rejected
#guard !ok DATETIME (s "2026-11-11T10:00:00")
#guard ok DATETIME (s "2026-11-11T10:00:00Z")
#guard ok DATETIME (s "2026-11-11T10:00:00.25+02:00")
#guard !ok DATETIME (s "2026-02-30T10:00:00Z")
#guard ok DATETIME (s "2024-02-29t23:59:60z")
#guard !ok DATETIME (s "2023-02-29T10:00:00Z")
#guard !ok DATETIME (s "2026-11-11T10:00:00.Z")
#guard !ok DATETIME (s "2026-11-11T10:00:00+24:00")
#guard !ok DATETIME (s "2026-11-11T10:00:00Zx")

-- relative_iri_is_rejected
#guard !ok IRI (s "orders/42")
#guard ok IRI (s "https://example.org/orders/42")
#guard ok IRI (s "urn:uuid:1234")
#guard ok IRI (s "https://w3id.org/vertep/vt#Chart")
#guard !ok IRI (s "https://ex.org/a b")
#guard ok IRI (s "https://ex.org/a%20b")
#guard !ok IRI (s "https://ex.org/a%2")
#guard !ok IRI (s "1http://ex.org")

-- primitives_reject_wrong_kinds_and_decimals_are_exact
#guard ok BOOL (.bool true)
#guard !ok BOOL (s "true")
#guard ok DECIMAL (s "12.50")
#guard ok DECIMAL (num 3)
#guard !ok DECIMAL (s "12,50")
#guard ok DECIMAL (s "-0.10")
#guard !ok DECIMAL (s "1.")
#guard !ok DECIMAL (s ".5")
#guard !ok STRING .null
#guard ok .value .undefined

/-! ## Composites -/

def order : Ty := .record [("id", STRING), ("amount", DECIMAL)]

-- record_with_missing_required_field, record_with_unknown_field
#guard !ok order (obj [("id", s "o-1")])
#guard !ok order (obj [("id", s "o-1"), ("amount", s "3"), ("note", s "x")])
#guard ok order (obj [("amount", s "3"), ("id", s "o-1")])
#guard !ok order (s "o-1")

/-- `Card { last4: string } | Cash`. -/
def payment : Ty := .union [("Card", some (.record [("last4", STRING)])), ("Cash", none)]

-- tagged_union_selects_its_variant_by_tag
#guard ok payment (obj [("tag", s "Cash")])
#guard ok payment (obj [("tag", s "Card"), ("value", obj [("last4", s "4242")])])
#guard !ok payment (obj [("tag", s "Card")])
#guard !ok payment (obj [("tag", s "Card"), ("value", obj [("last4", num 1)])])
#guard !ok payment (obj [("tag", s "Cheque")])
#guard !ok payment (obj [("tag", s "Cash"), ("value", .null)])
#guard ok payment (obj [("tag", s "Cash"), ("value", .undefined)])
#guard !ok payment (obj [("tag", s "Cash"), ("other", s "x")])
#guard !ok payment (obj [("tag", num 1)])

-- optional_accepts_absence
def withNote : Ty := .record [("id", STRING), ("note", .optional STRING)]
#guard ok withNote (obj [("id", s "o-1")])
#guard ok withNote (obj [("id", s "o-1"), ("note", .null)])
#guard ok withNote (obj [("id", s "o-1"), ("note", s "n")])
#guard !ok withNote (obj [("id", s "o-1"), ("note", num 1)])
#guard !ok withNote (obj [("note", s "n")])

-- constructors_nest_and_report_every_error_with_paths
def nested : Ty := .list (.record [("items", .list (.optional INT)), ("at", DATETIME)])
#guard !ok nested (.array [obj [("items", .array [num 1, .null])],
  obj [("items", .array [s "x"]), ("at", s "2026-11-11T10:00:00")]])
#guard ok nested (.array [obj [("items", .array [num 1, .null]), ("at", s "2026-11-11T10:00:00Z")]])

/-! ## Recursion -/

/-- `type Node = { label: string, children: list<Node> };` -/
def treeDecls : Decls := [("Node", .record [("label", STRING), ("children", .list (.named "Node"))])]

-- recursive_tree_through_a_list
#guard
  let leaf := obj [("label", s "b"), ("children", .array [])]
  check treeDecls [] (.named "Node") (obj [("label", s "a"), ("children", .array [leaf])])
#guard !check treeDecls [] (.named "Node")
  (obj [("label", s "a"), ("children", .array [obj [("label", s "b")]])])
#guard !check treeDecls [] (.named "Nod") (obj [("label", s "a"), ("children", .array [])])

/-- `type A = optional<A>; type B = optional<C>; type C = optional<B>;
type D = optional<optional<D>>; type R = { a: A, n: int };` -/
def loopDecls : Decls :=
  [("A", .optional (.named "A")), ("B", .optional (.named "C")), ("C", .optional (.named "B")),
   ("D", .optional (.optional (.named "D"))), ("R", .record [("a", .named "A"), ("n", INT)])]

-- recursion_through_optional_only_terminates
-- @tg: verifies:: [[openspec:formal-model#Type Checker Correctness#Recursion through optional only terminates]] {level: unit}
#guard ["A", "B", "D"].all fun n =>
  check loopDecls [] (.named n) .null &&
  !check loopDecls [] (.named n) (s "x") &&
  !check loopDecls [] (.named n) (obj [])
#guard !check loopDecls [] (.named "R") (obj [("a", .bool true), ("n", num 1)])
#guard check loopDecls [] (.named "R") (obj [("a", .null), ("n", num 1)])
#guard check loopDecls [] (.named "R") (obj [("n", num 1)])
-- an alias cycle (rejected by `validate`) has no values, and the checker still terminates
#guard !check [("P", .named "Q"), ("Q", .named "P")] [] (.named "P") .null

/-- The least fixed point: a value of `type A = optional<A>` is absent. -/
theorem optional_loop_absent {v : Value} {t : Ty} (h : HasType loopDecls v t)
    (ht : t = .named "A" ∨ t = .optional (.named "A")) : absent v = true := by
  induction h with
  | named n u v hl _ ih =>
    rcases ht with ht | ht <;> cases ht
    simp [loopDecls, Decls.lookup] at hl
    exact ih (.inr hl.symm)
  | optNone _ _ ha => exact ha
  | optSome t v ha _ ih =>
    rcases ht with ht | ht <;> cases ht
    exact absurd (ih (.inl rfl)) (by simp [ha])
  | _ => rcases ht with ht | ht <;> cases ht

/-- `"x"` is no value of `type A = optional<A>`, and `null` is, by the checker's correctness
theorem in both directions. -/
-- @tg: verifies:: [[openspec:formal-model#Type Checker Correctness#Checker decides the typing relation]] {level: unit}
theorem optional_loop_values :
    ¬HasType loopDecls (s "x") (.named "A") ∧ HasType loopDecls .null (.named "A") := by
  refine ⟨fun h => ?_, ?_⟩
  · have := optional_loop_absent h (.inl rfl)
    simp [s, absent] at this
  · exact .named "A" _ _ rfl (.optNone _ _ rfl)

/-- The checker agrees with the theorem above on the same values. -/
theorem optional_loop_check :
    (check loopDecls [] (.named "A") (s "x") = true ↔ False) ∧
      (check loopDecls [] (.named "A") .null = true ↔ True) := by
  refine ⟨⟨fun h => optional_loop_values.1 ((check_iff _ _ _).1 h), False.elim⟩,
    ⟨fun _ => trivial, fun _ => (check_iff _ _ _).2 optional_loop_values.2⟩⟩

/-! ## JSON Schema projection -/

open JsonSchema in
/-- Validation against the projected schema with the block's `$defs`. -/
def jsOk (D : Decls) (t : Ty) (v : Value) : Bool := validate (defsOf D) [] (proj D t) v

#guard jsOk [] INT (num 42) && !jsOk [] INT (num 15 1) && !jsOk [] INT (num 9223372036854775808)
#guard jsOk [] INT (num (-9223372036854775808)) && jsOk [] INT (num 420 1)
#guard jsOk [] DECIMAL (s "12.50") && jsOk [] DECIMAL (num 3) && !jsOk [] DECIMAL (s "12,50")
#guard jsOk [] DATETIME (s "2026-11-11T10:00:00Z") && !jsOk [] DATETIME (s "2026-02-30T10:00:00Z")
#guard jsOk [] IRI (s "urn:uuid:1234") && !jsOk [] IRI (s "orders/42")
#guard jsOk [] payment (obj [("tag", s "Cash")])
#guard jsOk [] payment (obj [("tag", s "Card"), ("value", obj [("last4", s "4242")])])
#guard !jsOk [] payment (obj [("tag", s "Card")])
#guard !jsOk [] payment (obj [("tag", s "Cash"), ("value", .null)])
#guard !jsOk [] payment (obj [("tag", s "Cheque")])
#guard jsOk [] withNote (obj [("id", s "o-1")]) && !jsOk [] withNote (obj [("note", s "n")])
-- the schema of `type A = optional<A>` is `anyOf [null, $ref A]`: the validator terminates
#guard ["A", "B", "D"].all fun n =>
  jsOk loopDecls (.named n) .null && !jsOk loopDecls (.named n) (s "x")
#guard jsOk treeDecls (.named "Node")
  (obj [("label", s "a"), ("children", .array [obj [("label", s "b"), ("children", .array [])]])])

/-- The schema of `type A = optional<A>` rejects `"x"`, by the agreement theorem. -/
-- @tg: verifies:: [[openspec:formal-model#JSON Schema Projection Agreement#Schema agrees with the checker]] {level: unit}
theorem optional_loop_schema : jsOk loopDecls (.named "A") (s "x") = false := by
  cases h : jsOk loopDecls (.named "A") (s "x")
  · rfl
  · have hD : ∀ d ∈ loopDecls, JsonSchema.tagsOk d.2 = true := by
      intro d hd
      simp only [loopDecls, List.mem_cons, List.not_mem_nil, or_false] at hd
      rcases hd with rfl | rfl | rfl | rfl | rfl <;> simp [JsonSchema.tagsOk, INT]
    have := (JsonSchema.jsonSchema_agrees loopDecls hD (.named "A") (by simp [JsonSchema.tagsOk])
      (s "x") (by simp [s, JsonSchema.jsonValue])).1 h
    exact absurd this optional_loop_values.1

end Vertep.Test.Types
