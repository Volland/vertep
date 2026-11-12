import VertepLean.Types.Basic

/-!
# The typing relation and the executable checker

`HasType D v t` is the typing relation of the type-system specification as an inductive
predicate, so it is the *least* relation closed under its rules: a value has a named type only
through a finite derivation. For the guarded but non-contractive `type A = optional<A>` the only
value is therefore `null` (and `undefined`), never `"x"`, although no finite unfolding of `A`
refutes `"x"`.

`check D seen t v` mirrors `Declarations::check` of `crates/vertep-core/src/types/check.rs`
(its verdict: no error). `seen` is Rust's `unfolding`: the named types being expanded at the
current position of the value. It is cleared on descent into a list item, a record field or a
union payload, and a name that repeats at the same position is a mismatch (decision *Type
checking stops on non-contractive recursion*). `check` is total on every declaration block,
validated or not, by the measure (size of the value, declarations not yet expanded at this
position, size of the type).

`check_iff` proves `check D [] t v = true ↔ HasType D v t` for every block `D`. Completeness
cuts loops: between two expansions of the same name at one position only names and present
optionals intervene (a `Chain`), and a chain that repeats a name can be shortened to one that
does not (`Chain.dedup`), which the checker follows to the end.
-/

namespace Vertep.Types

/-- A field or payload is smaller than the object that holds it. -/
theorem sizeOf_present {entries : List (String × Value)} {k : String} {v : Value}
    (h : present entries k = some v) : sizeOf v < sizeOf (Value.object entries) := by
  have hl : entries.lookup k = some v := by
    unfold present at h; split at h <;> simp_all
  have := List.sizeOf_lt_of_mem (lookup_mem' hl)
  simp only [Value.object.sizeOf_spec, Prod.mk.sizeOf_spec] at this ⊢
  omega

/-- The executable checker (Rust `Declarations::check` succeeds). -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]]
-- @tg: implements:: [[decisions#Type checking stops on non-contractive recursion]]
def check (D : Decls) (seen : List String) (t : Ty) (v : Value) : Bool :=
  match t with
  | .value => true
  | .prim p => primOk p v
  | .named n =>
    if _h : seen.contains n then false else
    match _hl : D.lookup n with
    | none => false
    | some u => check D (n :: seen) u v
  | .optional t₀ => absent v || check D seen t₀ v
  | .list t₀ =>
    match v with
    | .array items => items.attach.all fun ⟨x, _hx⟩ => check D [] t₀ x
    | _ => false
  | .record fs =>
    match v with
    | .object entries =>
      entries.all (fun e => isUndef e.2 || fs.any (·.1 == e.1)) &&
      fs.all fun f =>
        match _hp : present entries f.1 with
        | none => optish D f.2
        | some fv => check D [] f.2 fv
    | _ => false
  | .union vs =>
    match v with
    | .object entries =>
      match entries.lookup "tag" with
      | some (.str tag) =>
        entries.all (fun e => e.1 == "tag" || (e.1 == "value" || isUndef e.2)) &&
        match vs.lookup tag with
        | none => false
        | some none => (present entries "value").isNone
        | some (some pt) =>
          match _hp : present entries "value" with
          | none => optish D pt
          | some pv => check D [] pt pv
      | _ => false
    | _ => false
termination_by (sizeOf v, unexpanded D seen, sizeOf t)
decreasing_by
  · exact Prod.Lex.right _ (Prod.Lex.left _ _ (unexpanded_lt _hl _h))
  · exact Prod.Lex.right _ (Prod.Lex.right _ (by simp only [Ty.optional.sizeOf_spec]; omega))
  · exact Prod.Lex.left _ _ (by
      have := List.sizeOf_lt_of_mem _hx
      simp only [Value.array.sizeOf_spec]; omega)
  · exact Prod.Lex.left _ _ (sizeOf_present _hp)
  · exact Prod.Lex.left _ _ (sizeOf_present _hp)


/-- The typing relation: `HasType D v t` when the value `v` has type `t`, named types resolved
in `D`. Records are closed (an entry holding `undefined` is absent, so it is no unknown field), fields of an optional or `value` type may be absent, a union value
is `{"tag": …, "value": …}` with the payload checked against the first variant of that tag,
and `undefined` and `null` are absent. -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]]
inductive HasType (D : Decls) : Value → Ty → Prop where
  | value (v : Value) : HasType D v .value
  | prim (p : Prim) (v : Value) : primOk p v = true → HasType D v (.prim p)
  | named (n : String) (u : Ty) (v : Value) :
      D.lookup n = some u → HasType D v u → HasType D v (.named n)
  | optNone (t : Ty) (v : Value) : absent v = true → HasType D v (.optional t)
  | optSome (t : Ty) (v : Value) : absent v = false → HasType D v t → HasType D v (.optional t)
  | list (t : Ty) (items : List Value) :
      (∀ x ∈ items, HasType D x t) → HasType D (.array items) (.list t)
  | record (fs : List (String × Ty)) (entries : List (String × Value)) :
      (∀ e ∈ entries, isUndef e.2 = true ∨ ∃ f ∈ fs, f.1 = e.1) →
      (∀ f ∈ fs, present entries f.1 = none → optish D f.2 = true) →
      (∀ f ∈ fs, ∀ fv, present entries f.1 = some fv → HasType D fv f.2) →
      HasType D (.object entries) (.record fs)
  | tagOnly (vs : List (String × Option Ty)) (entries : List (String × Value)) (tag : String) :
      entries.lookup "tag" = some (.str tag) → (∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value" ∨ isUndef e.2 = true) →
      vs.lookup tag = some none → present entries "value" = none →
      HasType D (.object entries) (.union vs)
  | payloadAbsent (vs : List (String × Option Ty)) (entries : List (String × Value))
      (tag : String) (pt : Ty) :
      entries.lookup "tag" = some (.str tag) → (∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value" ∨ isUndef e.2 = true) →
      vs.lookup tag = some (some pt) → present entries "value" = none → optish D pt = true →
      HasType D (.object entries) (.union vs)
  | payload (vs : List (String × Option Ty)) (entries : List (String × Value)) (tag : String)
      (pt : Ty) (pv : Value) :
      entries.lookup "tag" = some (.str tag) → (∀ e ∈ entries, e.1 = "tag" ∨ e.1 = "value" ∨ isUndef e.2 = true) →
      vs.lookup tag = some (some pt) → present entries "value" = some pv → HasType D pv pt →
      HasType D (.object entries) (.union vs)

theorem check_named_seen {D : Decls} {seen : List String} {n : String} {v : Value}
    (hs : seen.contains n = true) : check D seen (.named n) v = false := by
  rw [check]; split
  · rfl
  · contradiction

theorem check_named_none {D : Decls} {seen : List String} {n : String} {v : Value}
    (hl : D.lookup n = none) : check D seen (.named n) v = false := by
  rw [check]; split
  · rfl
  · split <;> simp_all

theorem check_named_some {D : Decls} {seen : List String} {n : String} {v : Value} {u : Ty}
    (hs : ¬seen.contains n = true) (hl : D.lookup n = some u) :
    check D seen (.named n) v = check D (n :: seen) u v := by
  rw [check]; split
  · contradiction
  · split <;> simp_all

/-- **Soundness**: whatever the checker accepts has the type, for any set of names under
expansion. -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]]
theorem check_sound (D : Decls) (seen : List String) (t : Ty) (v : Value) :
    check D seen t v = true → HasType D v t := by
  induction seen, t, v using check.induct D with
  | case1 seen v => intro _; exact .value v
  | case2 seen v p => intro h; exact .prim p v (by simpa [check] using h)
  | case3 seen v n hs => intro h; simp [check_named_seen hs] at h
  | case4 seen v n hs hl => intro h; simp [check_named_none hl] at h
  | case5 seen v n hs u hl ih =>
    intro h; rw [check_named_some hs hl] at h; exact .named n u v hl (ih h)
  | case6 seen v t₀ ih =>
    intro h
    rw [check] at h
    cases ha : absent v
    · rw [ha, Bool.false_or] at h; exact .optSome t₀ v ha (ih h)
    · exact .optNone t₀ v ha
  | case7 seen t₀ items ih =>
    intro h
    rw [check] at h
    simp only [List.all_eq_true] at h
    exact .list t₀ items fun x hx => ih x hx (h ⟨x, hx⟩ (List.mem_attach _ _))
  | case8 seen v t₀ hv =>
    intro h; rw [check] at h
    all_goals first | exact hv | simp at h
  | case9 seen fs entries ih =>
    intro h
    rw [check] at h
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, List.any_eq_true, beq_iff_eq] at h
    obtain ⟨hk, hf⟩ := h
    refine .record fs entries hk (fun f hf' hp => ?_) (fun f hf' fv hp => ?_)
    · have := hf f hf'
      split at this
      · exact this
      · simp_all
    · have := hf f hf'
      split at this
      · simp_all
      · rename_i fv' hp'
        rw [hp] at hp'; cases hp'
        exact ih f fv hp this
  | case10 seen v fs hv =>
    intro h; rw [check] at h
    all_goals first | exact hv | simp at h
  | case11 seen vs entries tag htag ih =>
    intro h
    rw [check] at h
    simp only [htag, Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq] at h
    obtain ⟨hk, hm⟩ := h
    split at hm
    · simp at hm
    · rename_i hv
      exact .tagOnly vs entries tag htag hk hv (by simpa using hm)
    · rename_i pt hv
      split at hm
      · rename_i hp
        exact .payloadAbsent vs entries tag pt htag hk hv hp hm
      · rename_i pv hp
        exact .payload vs entries tag pt pv htag hk hv hp (ih pt pv hp hm)
  | case12 seen vs entries htag =>
    intro h; rw [check] at h
    all_goals first | exact htag | simp at h
  | case13 seen v vs hv =>
    intro h; rw [check] at h
    all_goals first | exact hv | simp at h

/-! ## Completeness -/

/-- `Chain D v t ns t'`: at the value `v`, expanding the names `ns` in order and entering present
optionals leads from `t` to `t'`, without consuming any of `v`. -/
inductive Chain (D : Decls) (v : Value) : Ty → List String → Ty → Prop where
  | refl (t : Ty) : Chain D v t [] t
  | named (n : String) (u : Ty) (ns : List String) (t' : Ty) :
      D.lookup n = some u → Chain D v u ns t' → Chain D v (.named n) (n :: ns) t'
  | opt (t : Ty) (ns : List String) (t' : Ty) :
      absent v = false → Chain D v t ns t' → Chain D v (.optional t) ns t'

/-- A chain through `n` has a suffix starting at `named n`. -/
theorem Chain.split {D : Decls} {v : Value} {t t' : Ty} {ns : List String}
    (hc : Chain D v t ns t') {n : String} (hn : n ∈ ns) :
    ∃ ns₂, (n :: ns₂) <:+ ns ∧ Chain D v (.named n) (n :: ns₂) t' := by
  induction hc with
  | refl => simp at hn
  | named m u ns t' hl hc ih =>
    by_cases hnm : n = m
    · subst hnm; exact ⟨ns, List.suffix_refl _, .named _ u ns t' hl hc⟩
    · have : n ∈ ns := by simpa [hnm] using hn
      obtain ⟨ns₂, hs, hc₂⟩ := ih this
      exact ⟨ns₂, hs.trans (List.suffix_cons _ _), hc₂⟩
  | opt t ns t' _ _ ih => exact ih hn

/-- Loop cutting: every chain can be shortened to one that expands each name at most once. -/
theorem Chain.dedup {D : Decls} {v : Value} {t t' : Ty} {ns : List String}
    (hc : Chain D v t ns t') : ∃ ns', ns'.Nodup ∧ Chain D v t ns' t' := by
  induction hc with
  | refl t => exact ⟨[], List.nodup_nil, .refl t⟩
  | named n u ns t' hl _ ih =>
    obtain ⟨ns', hnd, hc'⟩ := ih
    by_cases hn : n ∈ ns'
    · obtain ⟨ns₂, hs, hc₂⟩ := hc'.split hn
      exact ⟨n :: ns₂, hnd.sublist hs.sublist, hc₂⟩
    · exact ⟨n :: ns', List.nodup_cons.2 ⟨hn, hnd⟩, .named n u ns' t' hl hc'⟩
  | opt t ns t' ha _ ih =>
    obtain ⟨ns', hnd, hc'⟩ := ih
    exact ⟨ns', hnd, .opt t ns' t' ha hc'⟩

/-- The checker follows a chain without repeated names, avoiding the names already under
expansion, to its end. -/
theorem Chain.check_eq {D : Decls} {v : Value} {t t' : Ty} {ns : List String}
    (hc : Chain D v t ns t') (hnd : ns.Nodup) :
    ∀ seen : List String, (∀ n ∈ ns, ¬seen.contains n = true) →
      (∀ s, check D s t' v = true) → check D seen t v = true := by
  induction hc with
  | refl t => intro seen _ h; exact h seen
  | named n u ns t' hl _ ih =>
    intro seen hd h
    have hn : ¬seen.contains n = true := hd n List.mem_cons_self
    rw [check_named_some hn hl]
    have ⟨hnn, hnd'⟩ := List.nodup_cons.1 hnd
    refine ih hnd' (n :: seen) (fun m hm => ?_) h
    have hms := hd m (List.mem_cons_of_mem _ hm)
    have hmn : m ≠ n := fun e => hnn (e ▸ hm)
    simp only [List.contains_cons, Bool.or_eq_true, not_or]
    exact ⟨by simpa using hmn, hms⟩
  | opt t ns t' ha _ ih =>
    intro seen hd h
    rw [check, ha, Bool.false_or]
    exact ih hnd seen hd h

/-- A chain to a type the checker accepts at any position gives acceptance. -/
theorem check_of_chain {D : Decls} {v : Value} {t : Ty}
    (h : ∃ ns t', Chain D v t ns t' ∧ ∀ s, check D s t' v = true) : check D [] t v = true := by
  obtain ⟨ns, t', hc, hk⟩ := h
  obtain ⟨ns', hnd, hc'⟩ := hc.dedup
  exact hc'.check_eq hnd [] (fun _ _ => by simp) hk

/-- Every derivation of `HasType` is a chain at its position ending in a rule that consumes the
value, whose parts the checker accepts. -/
theorem hasType_chain {D : Decls} {v : Value} {t : Ty} (h : HasType D v t) :
    ∃ ns t', Chain D v t ns t' ∧ ∀ s, check D s t' v = true := by
  induction h with
  | value v => exact ⟨[], _, .refl _, fun s => by rw [check]⟩
  | prim p v hp => exact ⟨[], _, .refl _, fun s => by rw [check]; exact hp⟩
  | named n u v hl _ ih =>
    obtain ⟨ns, t', hc, hk⟩ := ih
    exact ⟨n :: ns, t', .named n u ns t' hl hc, hk⟩
  | optNone t v ha => exact ⟨[], _, .refl _, fun s => by rw [check, ha, Bool.true_or]⟩
  | optSome t v ha _ ih =>
    obtain ⟨ns, t', hc, hk⟩ := ih
    exact ⟨ns, t', .opt t ns t' ha hc, hk⟩
  | list t items _ ih =>
    refine ⟨[], _, .refl _, fun s => ?_⟩
    rw [check]
    simp only [List.all_eq_true]
    intro ⟨x, hx⟩ _
    exact check_of_chain (ih x hx)
  | record fs entries hk hopt _ ih =>
    refine ⟨[], _, .refl _, fun s => ?_⟩
    rw [check]
    simp only [Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, List.any_eq_true, beq_iff_eq]
    refine ⟨hk, fun f hf => ?_⟩
    split
    · rename_i hp; exact hopt f hf hp
    · rename_i fv hp; exact check_of_chain (ih f hf fv hp)
  | tagOnly vs entries tag htag hk hv hp =>
    refine ⟨[], _, .refl _, fun s => ?_⟩
    rw [check]
    simp only [htag, hv, hp, Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq,
      Option.isNone_none, and_true]
    exact hk
  | payloadAbsent vs entries tag pt htag hk hv hp ho =>
    refine ⟨[], _, .refl _, fun s => ?_⟩
    rw [check]
    simp only [htag, hv, Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    refine ⟨hk, ?_⟩
    split
    · exact ho
    · simp_all
  | payload vs entries tag pt pv htag hk hv hp _ ih =>
    refine ⟨[], _, .refl _, fun s => ?_⟩
    rw [check]
    simp only [htag, hv, Bool.and_eq_true, List.all_eq_true, Bool.or_eq_true, beq_iff_eq]
    refine ⟨hk, ?_⟩
    split
    · simp_all
    · rename_i pv' hp'
      rw [hp] at hp'; cases hp'
      exact check_of_chain ih

/-- **Checker correctness** (verification backlog 4.1): on every declaration block, the checker
started with no names under expansion accepts exactly the values of the type. -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]]
theorem check_iff (D : Decls) (t : Ty) (v : Value) : check D [] t v = true ↔ HasType D v t :=
  ⟨check_sound D [] t v, fun h => check_of_chain (hasType_chain h)⟩

end Vertep.Types
