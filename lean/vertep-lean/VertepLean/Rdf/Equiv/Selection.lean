import VertepLean.Rdf.Equiv.Compat
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Selection nodes and legal target sets

`vt:SelectionShape` (one `vt:choice` per selection node) holds on the projection of a chart with
unique state identifiers exactly when every target list of the chart is a legal target set
(`selection_iff`). A selection node names its list and a chain entry (`sel_inj`), the lists of
the chart have distinct fragments (`tcall_inj`), and within a list two targets' choices agree
exactly when `PairOk` holds of their positions (`compat_iff`, `pairOk_iff`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-! ## Choice triples of a target list -/

theorem choice_eq (k : Ctx) (m : List (String × Bool)) (i : Nat) (x : String) (par : Bool) (r : String) :
    (match m[i + 1]? with
      | none => k.iri r
      | some (c, _) => if par then k.iri x else k.iri c) = k.iri (cfS m i x par r) := by
  unfold cfS
  rcases m[i + 1]? with _ | ⟨c, _⟩
  · rfl
  · cases par <;> rfl

theorem mem_choice (k : Ctx) {owner : Term} {prop L step : String} {ids : List String} {sc : Term}
    {x y : Term} (hp : prop ≠ "choice") :
    (⟨x, vt "choice", y⟩ : Triple) ∈ k.targets owner prop L step ids sc ↔
      ∃ t j, (t, j) ∈ ids.zipIdx ∧ ∃ m, k.chains.lookup t = some m ∧ ∃ i, ∃ hi : i < m.length,
        x = k.iri (L ++ ":sel:" ++ m[i].1) ∧ y = k.iri (cfS m i m[i].1 m[i].2 (L ++ step ++ toString j)) := by
  simp only [Ctx.targets, List.mem_flatMap]
  constructor
  · rintro ⟨⟨t, j⟩, hj, hu⟩
    simp only [Ctx.node, ordT, int, List.cons_append, List.singleton_append, List.map_cons,
      List.map_nil, List.mem_cons, Triple.mk.injEq, List.mem_append, List.nil_append] at hu
    rcases hu with h | h | h | h | h | h
    · simp only [vt_inj] at h; exact absurd h.2.1.symm hp
    · simp at h
    · simp at h
    · simp at h
    · simp at h
    · split at h
      · simp at h
      · rename_i m hm
        simp only [List.mem_flatMap] at h
        obtain ⟨⟨⟨x', par⟩, i⟩, hi, h⟩ := h
        simp only [List.mem_singleton, Triple.mk.injEq, true_and] at h
        obtain ⟨rfl, rfl⟩ := h
        have hi' := List.mem_zipIdx_iff_getElem?.1 hi
        simp only at hi'
        obtain ⟨hlt, hget⟩ := List.getElem?_eq_some_iff.1 hi'
        refine ⟨t, j, hj, m, hm, i, hlt, by rw [hget], ?_⟩
        rw [hget]; unfold cfS
        rcases m[i + 1]? with _ | ⟨c', _⟩
        · rfl
        · cases par <;> rfl
  · rintro ⟨t, j, hj, m, hm, i, hi, rfl, rfl⟩
    refine ⟨(t, j), hj, ?_⟩
    simp only [Ctx.node, ordT, int, List.cons_append, List.singleton_append, List.map_cons,
      List.map_nil, List.mem_cons, List.mem_append, List.nil_append]
    right; right; right; right; right
    rw [hm]
    simp only [List.mem_flatMap]
    refine ⟨(m[i], i), List.mem_zipIdx_iff_getElem?.2 (by simp [hi]), ?_⟩
    simp only [List.mem_singleton, Triple.mk.injEq, true_and]
    unfold cfS
    rcases m[i + 1]? with _ | ⟨c', _⟩
    · rfl
    · cases m[i].2 <;> rfl

/-! ## Selection node fragments -/

theorem nc_reverse {l : List Char} (h : NC l) : NC l.reverse := fun hm => h (List.mem_reverse.1 hm)

theorem sel_inj {L L' a a' : String} (ha : NC a.toList) (ha' : NC a'.toList)
    (h : L ++ ":sel:" ++ a = L' ++ ":sel:" ++ a') : L = L' ∧ a = a' := by
  have h1 := congrArg (fun s : String => s.toList.reverse) h
  simp only [String.toList_append, List.reverse_append] at h1
  have hs : (":sel:" : String).toList.reverse = ':' :: "les".toList ++ [':'] := by decide
  rw [hs] at h1
  have h1' : a.toList.reverse ++ (':' :: ("les".toList ++ [':'] ++ L.toList.reverse)) =
      a'.toList.reverse ++ (':' :: ("les".toList ++ [':'] ++ L'.toList.reverse)) := by
    simpa using h1
  have h2 := nc_split (nc_reverse ha) (nc_reverse ha') (.inr rfl) (.inr rfl) h1'
  have h3 : a = a' := String.toList_inj.1 (List.reverse_inj.1 h2)
  subst h3
  refine ⟨?_, rfl⟩
  have h4 := List.append_cancel_left h1
  have h5 := List.append_cancel_left h4
  exact String.toList_inj.1 (List.reverse_inj.1 h5)

theorem nc_chainOf (p : List State) : ∀ i (hi : i < (chainOf p).length), NC (chainOf p)[i].1.toList := by
  intro i hi
  rcases i with _ | a
  · simp [NC]
  · simp only [chainOf_succ, fr]; exact nc_stateFragment _

/-! ## Chains and target positions -/

section
variable {c : Chart} (hnd : c.ids.Nodup)
include hnd

theorem entry_cases {T : State} {pT : List State} (he : (T, pT) ∈ WP c) :
    (entryOf (("chart", false) :: pT.map fr) T = [] ∧ targetPosition c T.id = none) ∨
    ∃ P p, (P, p) ∈ WP c ∧ targetPosition c T.id = some P ∧
      entryOf (("chart", false) :: pT.map fr) T = chainOf p := by
  have hT := wp_mem he
  have hpos := wp_len_pos he
  unfold targetPosition
  rw [CompileProof.find_eq hnd hT]
  simp only
  rcases T with ⟨id, ig, el, ini, ie, ch, trs, en, ex, da, inv, dd, xs⟩
  cases el with
  | history h =>
    simp only [entryOf, State.element, State.isHistory, ↓reduceIte, List.length_cons, List.length_map]
    by_cases hl : pT.length = 1
    · left
      refine ⟨by simp [hl], ?_⟩
      exact wp_parent_none hnd he hl
    · right
      have hp := wp_parent_some hnd he (k := pT.length - 2) (by dsimp only; omega)
      have hQ := wp_closed he (pT.length - 2) (by dsimp only; omega)
      dsimp only at hp hQ
      refine ⟨_, _, hQ, hp, ?_⟩
      simp only [show 2 < pT.length + 1 from by omega, ↓reduceIte, chainOf]
      rw [List.dropLast_cons_of_ne_nil (by simpa using List.ne_nil_of_length_pos hpos), ← List.map_dropLast,
        List.dropLast_eq_take, show pT.length - 1 = pT.length - 2 + 1 from by omega]
  | state => exact .inr ⟨_, _, he, rfl, rfl⟩
  | parallel => exact .inr ⟨_, _, he, rfl, rfl⟩
  | final => exact .inr ⟨_, _, he, rfl, rfl⟩

theorem lookup_some (base : String) {t : String} {m : List (String × Bool)}
    (h : (ctxOf c base).chains.lookup t = some m) :
    m = [] ∨ ∃ P p, (P, p) ∈ WP c ∧ targetPosition c t = some P ∧ m = chainOf p := by
  by_cases ht : t ∈ c.ids
  · obtain ⟨T, hT, rfl⟩ := List.mem_map.1 ht
    obtain ⟨pT, he⟩ := wp_of_mem hT
    have hl := chains_lookup hnd base he
    simp only at hl
    rw [hl] at h; cases h
    rcases entry_cases hnd he with ⟨h1, _⟩ | ⟨P, p, h1, h2, h3⟩
    · exact .inl h1
    · exact .inr ⟨P, p, h1, h2, h3⟩
  · rw [chains_lookup_none c base ht] at h; cases h

theorem lookup_pos (base : String) {t : String} {P : State} (h : targetPosition c t = some P) :
    ∃ p, (P, p) ∈ WP c ∧ (ctxOf c base).chains.lookup t = some (chainOf p) := by
  have ht : ∃ T, c.find t = some T := by
    unfold targetPosition at h
    split at h
    · exact ⟨_, by assumption⟩
    · cases h
  obtain ⟨T, hf⟩ := ht
  obtain ⟨hT, rfl⟩ := (find_iff hnd).1 hf
  obtain ⟨pT, he⟩ := wp_of_mem hT
  have hl := chains_lookup hnd base he
  simp only at hl
  rcases entry_cases hnd he with ⟨_, h2⟩ | ⟨P', p, h1, h2, h3⟩
  · rw [h2] at h; cases h
  · rw [h2] at h; cases h
    exact ⟨p, h1, by rw [hl, h3]⟩

end

/-! ## The target lists of a chart -/

section
variable (k : Ctx) (c : Chart)

/-- The calls of one state, by suffix of the list fragment. -/
def SC (pp : Option State) (Y : State) (s : String) (C : TCall) : Prop :=
  (s = ":ia" ∧ C = ⟨k.iri (stateFragment Y.id), "initial", stateFragment Y.id ++ ":ia", "", Y.initial,
    k.iri (stateFragment Y.id)⟩) ∨
  (s = ":init" ++ ":t" ∧ ∃ ie, Y.initialElement = some ie ∧ C = ⟨k.iri (stateFragment Y.id ++ ":init" ++ ":t"),
    "target", stateFragment Y.id ++ ":init" ++ ":t", ":tg", ie.transition.targets, k.iri (stateFragment Y.id)⟩) ∨
  (∃ t j, (t, j) ∈ Y.transitions.zipIdx ∧ s = ":t" ++ toString j ∧
    C = ⟨k.iri (stateFragment Y.id ++ ":t" ++ toString j), "target", stateFragment Y.id ++ ":t" ++ toString j,
      ":tg", t.targets, scopeT k pp Y⟩)

theorem stateCalls_shape {pp : Option State} {Y : State} {C : TCall} (h : C ∈ stateCalls k pp Y) :
    ∃ s, C.list = stateFragment Y.id ++ s ∧ s.toList.head? = some ':' ∧ SC k pp Y s C := by
  simp only [stateCalls, List.mem_cons, List.mem_append, Option.mem_toList, Option.mem_def,
    Option.map_eq_some_iff, List.mem_map] at h
  rcases h with rfl | ⟨ie, hie, rfl⟩ | ⟨⟨t, j⟩, hj, rfl⟩
  · exact ⟨":ia", rfl, rfl, .inl ⟨rfl, rfl⟩⟩
  · exact ⟨":init" ++ ":t", by simp [String.append_assoc], by decide, .inr (.inl ⟨rfl, ie, hie, rfl⟩)⟩
  · exact ⟨":t" ++ toString j, by simp [String.append_assoc], by simp [String.toList_append],
      .inr (.inr ⟨t, j, hj, rfl, rfl⟩)⟩

theorem sc_uniq {pp : Option State} {Y : State} {s : String} {C C' : TCall}
    (h : SC k pp Y s C) (h' : SC k pp Y s C') : C = C' := by
  have n1 : ∀ j : Nat, ":ia" ≠ ":t" ++ toString j := fun j h => by
    have := congrArg String.toList h; simp [String.toList_append] at this
  have n2 : ∀ j : Nat, ":init" ++ ":t" ≠ ":t" ++ toString j := fun j h => by
    have := congrArg String.toList h; simp [String.toList_append] at this
  have n3 : ":ia" ≠ ":init" ++ ":t" := by decide
  rcases h with ⟨rfl, rfl⟩ | ⟨rfl, ie, hie, rfl⟩ | ⟨t, j, hj, rfl, rfl⟩ <;>
    rcases h' with ⟨h1, rfl⟩ | ⟨h1, ie', hie', rfl⟩ | ⟨t', j', hj', h1, rfl⟩
  all_goals first
    | rfl
    | exact absurd h1 n3 | exact absurd h1.symm n3
    | exact absurd h1 (n1 _) | exact absurd h1.symm (n1 _)
    | exact absurd h1 (n2 _) | exact absurd h1.symm (n2 _)
    | skip
  · rw [hie] at hie'; cases hie'; rfl
  · have := congrArg String.toList h1
    simp only [String.toList_append, List.append_cancel_left_eq, toList_toString] at this
    have hjj := digits_inj.1 this
    subst hjj
    have e1 := List.mem_zipIdx_iff_getElem?.1 hj
    have e2 := List.mem_zipIdx_iff_getElem?.1 hj'
    simp only at e1 e2
    rw [e1] at e2; cases e2; rfl

theorem tcall_shape {C : TCall} (h : C ∈ tcalls k c) :
    (C.list = "chart" ++ ":ia" ∧ C = ⟨k.iri "chart", "initial", "chart:ia", "", c.initial, k.iri "chart"⟩) ∨
    ∃ q ∈ c.parented, ∃ s, C.list = stateFragment q.2.id ++ s ∧ s.toList.head? = some ':' ∧ SC k q.1 q.2 s C := by
  simp only [tcalls, List.mem_cons, List.mem_flatMap] at h
  rcases h with rfl | ⟨q, hq, h⟩
  · exact .inl ⟨rfl, rfl⟩
  · exact .inr ⟨q, hq, stateCalls_shape k h⟩

theorem tcall_colon {C : TCall} (h : C ∈ tcalls k c) : ':' ∈ C.list.toList := by
  rcases tcall_shape k c h with ⟨h1, _⟩ | ⟨q, _, s, h1, h2, _⟩
  · rw [h1]; decide
  · rw [h1, String.toList_append]
    refine List.mem_append_right _ ?_
    cases hs : s.toList with
    | nil => rw [hs] at h2; cases h2
    | cons a l => rw [hs] at h2; simp at h2; subst h2; exact List.mem_cons_self

theorem tcall_inj (hnd : c.ids.Nodup) {C C' : TCall} (h : C ∈ tcalls k c) (h' : C' ∈ tcalls k c)
    (hl : C.list = C'.list) : C = C' := by
  have hcs : ∀ (x s : String), s.toList.head? = some ':' → "chart" ++ ":ia" ≠ stateFragment x ++ s := by
    intro x s hs he
    have := (frag_split nc_chart (nc_stateFragment x) (by decide) hs he).1
    exact chart_ne_sf x this
  rcases tcall_shape k c h with ⟨h1, rfl⟩ | ⟨q, hq, s, h1, h2, h3⟩ <;>
    rcases tcall_shape k c h' with ⟨h1', rfl⟩ | ⟨q', hq', s', h1', h2', h3'⟩
  · rfl
  · exact absurd (h1.symm.trans (hl.trans h1')) (hcs _ _ h2')
  · exact absurd (h1'.symm.trans (hl.symm.trans h1)) (hcs _ _ h2)
  · have := frag_split (nc_stateFragment _) (nc_stateFragment _) h2 h2' (h1.symm.trans (hl.trans h1'))
    obtain ⟨e1, rfl⟩ := this
    have := parented_uniq hnd hq hq' (stateFragment_inj e1)
    subst this
    exact sc_uniq k h3 h3'

end

/-! ## One target list -/

/-- The choices of a target list agree at every selection node. -/
def Agree (k : Ctx) (L step : String) (ids : List String) : Prop :=
  ∀ t j t' j' m m' i i', (t, j) ∈ ids.zipIdx → (t', j') ∈ ids.zipIdx →
    k.chains.lookup t = some m → k.chains.lookup t' = some m' →
    ∀ (hi : i < m.length) (hi' : i' < m'.length), m[i].1 = m'[i'].1 →
      cfS m i m[i].1 m[i].2 (L ++ step ++ toString j) = cfS m' i' m'[i'].1 m'[i'].2 (L ++ step ++ toString j')

theorem targetSetOk_iff (c : Chart) (ids : List String) :
    TargetSetOk c ids ↔ ∀ j j' (hj : j < ids.length) (hj' : j' < ids.length), j < j' →
      ∀ P P', targetPosition c ids[j] = some P → targetPosition c ids[j'] = some P' → PairOk c P P' := by
  unfold TargetSetOk
  rw [List.pairwise_filterMap, List.pairwise_iff_getElem]
  constructor
  · intro h j j' hj hj' hlt P P' hP hP'; exact h j j' hj hj' hlt P hP P' hP'
  · intro h j j' hj hj' hlt P hP P' hP'; exact h j j' hj hj' hlt P P' hP hP'

theorem rf_ne {L step : String} {j j' : Nat} (h : j ≠ j') : L ++ step ++ toString j ≠ L ++ step ++ toString j' := by
  intro he
  have := congrArg String.toList he
  simp only [String.toList_append, List.append_cancel_left_eq, toList_toString] at this
  exact h (digits_inj.1 this)

theorem rf_colon {L step : String} (hc : ':' ∈ L.toList) (j : Nat) : ':' ∈ (L ++ step ++ toString j).toList := by
  simp only [String.toList_append, List.mem_append]; exact .inl (.inl hc)

/-- **A target list's selection nodes agree exactly when it is a legal target set.** -/
theorem agree_iff_tso {c : Chart} (hnd : c.ids.Nodup) (base : String) {L step : String} (ids : List String)
    (hc : ':' ∈ L.toList) : Agree (ctxOf c base) L step ids ↔ TargetSetOk c ids := by
  rw [targetSetOk_iff]
  constructor
  · intro H j j' hj hj' hlt P P' hP hP'
    obtain ⟨p, he, hl⟩ := lookup_pos hnd base hP
    obtain ⟨p', he', hl'⟩ := lookup_pos hnd base hP'
    have := (compat_iff hnd he he' (rf_ne (by omega)) (rf_colon hc j) (rf_colon hc j')).1 (fun i i' hi hi' hx =>
      H ids[j] j ids[j'] j' _ _ i i' (List.mem_zipIdx_iff_getElem?.2 (by simp))
        (List.mem_zipIdx_iff_getElem?.2 (by simp)) hl hl' hi hi' hx)
    exact (pairOk_iff hnd he he').2 this
  · intro H t j t' j' m m' i i' hj hj' hm hm' hi hi' hx
    have ej := List.mem_zipIdx_iff_getElem?.1 hj
    have ej' := List.mem_zipIdx_iff_getElem?.1 hj'
    simp only at ej ej'
    obtain ⟨hjl, rfl⟩ := List.getElem?_eq_some_iff.1 ej
    obtain ⟨hjl', rfl⟩ := List.getElem?_eq_some_iff.1 ej'
    rcases lookup_some hnd base hm with rfl | ⟨P, p, he, hP, rfl⟩
    · simp at hi
    rcases lookup_some hnd base hm' with rfl | ⟨P', p', he', hP', rfl⟩
    · simp at hi'
    rcases Nat.lt_trichotomy j j' with hlt | rfl | hlt
    · have := (pairOk_iff hnd he he').1 (H j j' hjl hjl' hlt P P' hP hP')
      exact (compat_iff hnd he he' (rf_ne (by omega)) (rf_colon hc j) (rf_colon hc j')).2 this i i' hi hi' hx
    · rw [hP] at hP'; cases hP'
      have := wp_uniq hnd he he' rfl
      simp only [Prod.mk.injEq, true_and] at this
      subst this
      exact (compat_states hnd he he _ _).2 ⟨rfl, fun _ _ _ _ => rfl⟩ i i' hi hi' hx
    · have := (pairOk_iff hnd he' he).1 (H j' j hjl' hjl hlt P' P hP' hP)
      exact ((compat_iff hnd he' he (rf_ne (by omega)) (rf_colon hc j') (rf_colon hc j)).2 this i' i hi' hi
        hx.symm).symm

/-! ## The selection shape -/

section
variable (c : Chart) (base : String) (hnd : c.ids.Nodup)
include hnd

theorem selection_tcalls :
    (∀ x o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ toRDF c base → Conforms (toRDF c base) selectionS x) ↔
      ∀ C ∈ tcalls (ctxOf c base) c, TargetSetOk c C.ids := by
  simp only [selectionS, conforms_top, compsHold_cons, compsHold_nil, and_true, pr, holds_property,
    vn_none, forall_eq, conforms_propS, maxC, holds_maxCount1, vn_some, ShapesL.P, valueNode_pred,
    iri_vi, toRDF, mem_ofList']
  have hp : ∀ C ∈ tcalls (ctxOf c base) c, C.prop ≠ "choice" := fun C hC => by
    rcases tcalls_prop hC with h | h <;> rw [h] <;> decide
  constructor
  · intro H C hC
    rw [← agree_iff_tso hnd base C.ids (tcall_colon _ c hC)]
    intro t j t' j' m m' i i' hj hj' hm hm' hi hi' hx
    have h1 := tcalls_sub c base C hC _ ((mem_choice (ctxOf c base) (hp C hC)).2
      ⟨t, j, hj, m, hm, i, hi, rfl, rfl⟩)
    have h2 := tcalls_sub c base C hC _ ((mem_choice (ctxOf c base) (hp C hC)).2
      ⟨t', j', hj', m', hm', i', hi', rfl, rfl⟩)
    have h2' : (⟨(ctxOf c base).iri (C.list ++ ":sel:" ++ m[i].1), vt "choice",
        (ctxOf c base).iri (cfS m' i' m'[i'].1 m'[i'].2 (C.list ++ C.step ++ toString j'))⟩ : Triple) ∈
        project c base := by rw [hx]; exact h2
    exact (iri_inj _).1 (H _ _ h1 _ _ h1 h2')
  · intro H x o _ y z hy hz
    obtain ⟨C, hC, hy'⟩ := choice_triple c base hy
    obtain ⟨C', hC', hz'⟩ := choice_triple c base hz
    obtain ⟨t, j, hj, m, hm, i, hi, hx, rfl⟩ := (mem_choice (ctxOf c base) (hp C hC)).1 hy'
    obtain ⟨t', j', hj', m', hm', i', hi', hx', rfl⟩ := (mem_choice (ctxOf c base) (hp C' hC')).1 hz'
    have hnc : ∀ {t : String} {m : List (String × Bool)} (hm : (ctxOf c base).chains.lookup t = some m)
        (i : Nat) (hi : i < m.length), NC m[i].1.toList := by
      intro t m hm i hi
      rcases lookup_some hnd base hm with rfl | ⟨_, p, _, _, rfl⟩
      · simp at hi
      · exact nc_chainOf p i hi
    obtain ⟨hL, he⟩ := sel_inj (hnc hm i hi) (hnc hm' i' hi') ((iri_inj _).1 (hx.symm.trans hx'))
    have := tcall_inj _ c hnd hC hC' hL
    subst this
    have hA := (agree_iff_tso hnd base (step := C.step) C.ids (tcall_colon _ c hC)).2 (H C hC)
    rw [hA t j t' j' m m' i i' hj hj' hm hm' hi hi' he]

/-- **`vt:SelectionShape` holds on the projection of a chart with unique state identifiers
exactly when every target list of the chart is a legal target set.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "legal target sets; the full theorem is shapes_equivalence"}
theorem selection_iff :
    (∀ x o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ toRDF c base → Conforms (toRDF c base) selectionS x) ↔
      (TargetSetOk c c.initial ∧ ∀ q ∈ c.parented, TSets c q.2) := by
  rw [selection_tcalls c base hnd]
  simp only [tcalls, List.mem_cons, forall_eq_or_imp, List.mem_flatMap, forall_exists_index, and_imp]
  apply and_congr Iff.rfl
  have hq : ∀ q : Option State × State, (∀ C ∈ stateCalls (ctxOf c base) q.1 q.2, TargetSetOk c C.ids) ↔
      TSets c q.2 := by
    intro q
    simp only [stateCalls, List.mem_cons, List.mem_append, Option.mem_toList, Option.mem_def,
      Option.map_eq_some_iff, List.mem_map, forall_eq_or_imp, TSets]
    apply and_congr Iff.rfl
    constructor
    · intro h
      refine ⟨fun ie hie => h _ (.inl ⟨ie, hie, rfl⟩), fun t ht => ?_⟩
      obtain ⟨j, hjl, rfl⟩ := List.getElem_of_mem ht
      exact h _ (.inr ⟨(q.2.transitions[j], j), List.mem_zipIdx_iff_getElem?.2 (by simp), rfl⟩)
    · rintro ⟨h1, h2⟩ C (⟨ie, hie, rfl⟩ | ⟨⟨t, j⟩, hj, rfl⟩)
      · exact h1 ie hie
      · have := List.mem_zipIdx_iff_getElem?.1 hj
        simp only at this
        exact h2 t (List.mem_of_getElem? this)
  constructor
  · intro h q hq'
    exact (hq q).1 fun C hC => h C q hq' hC
  · intro h C q hq' hC
    exact (hq q).2 (h q hq') C hC

end

end Vertep.Rdf.Equiv
