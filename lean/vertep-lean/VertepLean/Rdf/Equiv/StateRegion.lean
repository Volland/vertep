import VertepLean.Rdf.Equiv.StateNode
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes in the region of a state node

`reg_stateCore`: the node shapes hold at a state node and in the regions of its non-state
children (target references, `<initial>`, transitions, handlers, data, invokes, done data and
extensions) exactly when the state's own rules and its children's rules hold.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

theorem forall_zipIdx {α : Type} {l : List α} {Q : α → Prop} :
    (∀ p ∈ l.zipIdx, Q p.1) ↔ ∀ x ∈ l, Q x := by
  constructor
  · intro h x hx
    obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hx
    exact h (l[i], i) (List.mem_zipIdx_iff_getElem?.2 (by simp))
  · intro h p hp; exact h p.1 (fst_mem_zipIdx hp)

theorem nodup_fst_eq {D : List (String × List Triple)} (hnd : (D.map Prod.fst).Nodup)
    {d r : String × List Triple} (hd : d ∈ D) (hr : r ∈ D) (h : d.1 = r.1) : d = r := by
  induction D with
  | nil => simp at hd
  | cons a D ih =>
    simp only [List.map_cons, List.nodup_cons, List.mem_map] at hnd
    simp only [List.mem_cons] at hd hr
    rcases hd with rfl | hd <;> rcases hr with rfl | hr
    · rfl
    · exact absurd ⟨r, hr, h.symm⟩ hnd.1
    · exact absurd ⟨d, hd, h⟩ hnd.1
    · exact ih hnd.2 hd hr

/-- The state's children other than its `initial` references are not objects of `vt:initial`. -/
theorem kid_noinit {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y)
    {d : String × List Triple} (hd : d ∈ kidsOf k pt A Y)
    (hnr : d ∉ refKids k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") ""
      Y.initial (k.iri (stateFragment Y.id)))
    (hc : Contained' k ts (k.iri d.1) (k.iri (stateFragment Y.id))) :
    ∀ s, (⟨s, vt "initial", k.iri d.1⟩ : Triple) ∉ ts := by
  intro s hs
  have := (hc.conts s).1 ⟨"initial", by simp [containers], hs⟩
  subst this
  rw [state_view h.cap (above_ne h.own) _ _ (by decide) (by decide)] at hs
  simp only [String.reduceEq, false_and, false_or, or_false, true_and, List.mem_map] at hs
  obtain ⟨r, hr, he⟩ := hs
  have := nodup_fst_eq (decomp_core k i pt A Y).nd (kidsOf_mem.2 (.inl hr)) hd ((iri_inj k).1 he)
  exact hnr (this ▸ hr)

/-- The rules of a state's non-state children. -/
def KidsOK (k : Ctx) (ts : List Triple) (Kn : String → Prop) (pt : Term) (A : List Term) (Y : State) : Prop :=
  (∀ ie, Y.initialElement = some ie →
    (ie.transition.events = [] ∧ ie.transition.cond = none ∧ ie.transition.targets ≠ []) ∧
    (∀ x ∈ ie.transition.targets, TR k ts Kn x (k.iri (stateFragment Y.id))) ∧ BlockOk ie.transition.body) ∧
  (∀ t ∈ Y.transitions, (∀ x ∈ t.targets, TR k ts Kn x (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) ∧
    BlockOk t.body) ∧
  (∀ b ∈ Y.onentry, BlockOk b) ∧ (∀ b ∈ Y.onexit, BlockOk b) ∧ (∀ d ∈ Y.data, DataOk d) ∧
  (∀ v ∈ Y.invokes, InvokeOk v) ∧
  (∀ dd, Y.donedata = some dd → ¬(dd.content.isSome ∧ dd.params ≠ []) ∧ ParamsOk dd.params)

set_option maxHeartbeats 4000000 in
theorem reg_kids (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x) :
    (∀ d ∈ kidsOf k pt A Y, RG k ts d.1) ↔ KidsOK k ts Kn pt A Y := by
  have cx := h.cx
  have hv := state_view h.cap (above_ne h.own)
  have hsT : (⟨k.iri (stateFragment Y.id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts := (h.ty _).2 (.inl rfl)
  have hnoIT : ∀ o, (⟨k.iri (stateFragment Y.id), vt "initialTransition", o⟩ : Triple) ∉ ts := by
    intro o ho; rw [hv _ _ (by decide) (by decide)] at ho; simp at ho
  -- generic context of a kid
  have gk : ∀ {p : String} {D' : List (String × List Triple)}, KidsL k (stateFragment Y.id) p D' →
      (∀ d ∈ D', d ∈ kidsOf k pt A Y) → p ∈ containers → ∀ d ∈ D',
      Cap k ts (U d.1) d.2 ∧ Contained' k ts (k.iri d.1) (k.iri (stateFragment Y.id)) ∧
        Above (k := k) (k.iri (stateFragment Y.id)) d.1 :=
    fun hL hD' hp d hd => kid_ctx env cx h.cont hL hD' hp hd
  have nr : ∀ d, (d ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement ∨
      d ∈ Y.transitions.zipIdx.map (transK k (stateFragment Y.id)
        (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) ∨
      d ∈ Y.onentry.zipIdx.map (hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry") ∨
      d ∈ Y.onexit.zipIdx.map (hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit") ∨
      d ∈ Y.data.zipIdx.map (dataK k (stateFragment Y.id)) ∨
      d ∈ Y.invokes.zipIdx.map (invK k (stateFragment Y.id)) ∨
      d ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata ∨
      d ∈ extKids k (stateFragment Y.id) Y.extensions) →
      d ∉ refKids k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") ""
        Y.initial (k.iri (stateFragment Y.id)) := by
    intro d hd hr
    have hnd := (decomp_core k i pt A Y).nd
    simp only [kidsOf, stateKids, List.append_assoc, List.map_append, List.nodup_append] at hnd
    have := hnd.2.2 d.1 (List.mem_map_of_mem hr) d.1
    apply this _ rfl
    simp only [List.mem_append]
    rcases hd with hd | hd | hd | hd | hd | hd | hd | hd
    · exact .inl (List.mem_map_of_mem (f := Prod.fst) hd)
    · exact .inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd))
    · exact .inr (.inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd)))
    · exact .inr (.inr (.inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd))))
    · exact .inr (.inr (.inr (.inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd)))))
    · exact .inr (.inr (.inr (.inr (.inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd))))))
    · exact .inr (.inr (.inr (.inr (.inr (.inr (.inl (List.mem_map_of_mem (f := Prod.fst) hd)))))))
    · exact .inr (.inr (.inr (.inr (.inr (.inr (.inr (List.mem_map_of_mem (f := Prod.fst) hd)))))))
  -- refs
  have h1 : ∀ d ∈ refKids k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") ""
      Y.initial (k.iri (stateFragment Y.id)), RG k ts d.1 := by
    intro d hd
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_ref k _ "initial" _ "" _ _) (fun d hd => kidsOf_mem.2 (.inl hd))
      (by simp [containers]) d hd
    simp only [refKids, List.mem_map] at hd
    obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
    exact rg_ref env hcap hown hc (fun s hs => ⟨(hc.conts s).1 ⟨"initial", by simp [containers], hs⟩, rfl⟩)
  -- <initial>
  have h2 : (∀ d ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement, RG k ts d.1) ↔
      ∀ ie, Y.initialElement = some ie →
        (ie.transition.events = [] ∧ ie.transition.cond = none ∧ ie.transition.targets ≠ []) ∧
        (∀ x ∈ ie.transition.targets, TR k ts Kn x (k.iri (stateFragment Y.id))) ∧ BlockOk ie.transition.body := by
    rcases hie : Y.initialElement with _ | ie
    · simp [initKidsOpt]
    · have hd : (stateFragment Y.id ++ ":init", initEmission k (k.iri (stateFragment Y.id))
          (stateFragment Y.id ++ ":init") ie) ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement := by
        simp [initKidsOpt, hie]
      obtain ⟨hcap, hc, hown⟩ := gk (kidsL_initKidsOpt k _ _) (fun d hd => kidsOf_mem.2 (.inr (.inl hd)))
        (by simp [containers]) _ hd
      have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inl hd))) (nr _ (.inl hd)) hc
      have := reg_init env ⟨hcap, decomp_init k _ _ ie, hown⟩ hc hni hsT Kn hSN
      simp only [initKidsOpt, List.mem_singleton, forall_eq, Option.some.injEq, forall_eq', this]
      rfl
  -- transitions
  have h3 : (∀ d ∈ Y.transitions.zipIdx.map (transK k (stateFragment Y.id)
      (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)), RG k ts d.1) ↔
      ∀ t ∈ Y.transitions, (∀ x ∈ t.targets, TR k ts Kn x (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) ∧
        BlockOk t.body := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun t : Transition => (∀ x ∈ t.targets,
      TR k ts Kn x (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) ∧ BlockOk t.body)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := transK k (stateFragment Y.id)
      (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) hp
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_transK k _ _ _) (fun d hd => kidsOf_mem.2 (.inr (.inr (.inl hd))))
      (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inl hd)))) (nr _ (.inr (.inl hd))) hc
    rcases p with ⟨t, j⟩
    simp only [transK] at hcap hc hown hni ⊢
    have hlinkT : (⟨k.iri (stateFragment Y.id), vt "transition", k.iri (stateFragment Y.id ++ ":t" ++ toString j)⟩ :
        Triple) ∈ ts := hcap.sub.subset (by simp [Ctx.transition, Ctx.node])
    have hsrc : ∀ v, ((⟨v, vt "transition", k.iri (stateFragment Y.id ++ ":t" ++ toString j)⟩ : Triple) ∈ ts ∨ ∃ z,
        (⟨z, vt "initialTransition", k.iri (stateFragment Y.id ++ ":t" ++ toString j)⟩ : Triple) ∈ ts ∧
          (⟨v, vt "initialElement", z⟩ : Triple) ∈ ts) ↔ v = k.iri (stateFragment Y.id) := by
      intro v
      constructor
      · rintro (hh | ⟨z, h1, _⟩)
        · exact (hc.conts v).1 ⟨"transition", by simp [containers], hh⟩
        · have := (hc.conts z).1 ⟨"initialTransition", by simp [containers], h1⟩
          subst this; exact absurd h1 (hnoIT _)
      · intro hvv; rw [hvv]; exact .inl hlinkT
    exact reg_transition env ⟨hcap, decomp_transition k _ _ _ _ _ _ _, hown⟩ hc hni hsT hsrc Kn hSN
  -- handlers
  have h4 : (∀ d ∈ Y.onentry.zipIdx.map (hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry"), RG k ts d.1) ↔
      ∀ b ∈ Y.onentry, BlockOk b := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun b => BlockOk b)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry") hp
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_hookK k _ _ _ _ _) (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inl hd)))))
      (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inl hd))))) (nr _ (.inr (.inr (.inl hd)))) hc
    rcases p with ⟨b, j⟩
    simp only [hookK] at hcap hc hown hni ⊢
    exact reg_hook env (.inl rfl) ⟨hcap, decomp_hook k _ _ _ _ _ _, hown⟩ hc hni
  have h5 : (∀ d ∈ Y.onexit.zipIdx.map (hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit"), RG k ts d.1) ↔
      ∀ b ∈ Y.onexit, BlockOk b := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun b => BlockOk b)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit") hp
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_hookK k _ _ _ _ _)
      (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inl hd)))))) (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inl hd))))))
      (nr _ (.inr (.inr (.inr (.inl hd))))) hc
    rcases p with ⟨b, j⟩
    simp only [hookK] at hcap hc hown hni ⊢
    exact reg_hook env (.inr rfl) ⟨hcap, decomp_hook k _ _ _ _ _ _, hown⟩ hc hni
  have h6 : (∀ d ∈ Y.data.zipIdx.map (dataK k (stateFragment Y.id)), RG k ts d.1) ↔ ∀ d ∈ Y.data, DataOk d := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun d => DataOk d)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := dataK k (stateFragment Y.id)) hp
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_dataK k _ _)
      (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inl hd))))))) (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))
      (nr _ (.inr (.inr (.inr (.inr (.inl hd)))))) hc
    rcases p with ⟨d, j⟩
    simp only [dataK] at hcap hc hown hni ⊢
    exact reg_data env ⟨hcap, decomp_dataItem k _ _ _ _, hown⟩ hc hni
  have h7 : (∀ d ∈ Y.invokes.zipIdx.map (invK k (stateFragment Y.id)), RG k ts d.1) ↔ ∀ v ∈ Y.invokes, InvokeOk v := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun v => InvokeOk v)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := invK k (stateFragment Y.id)) hp
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_invK k _ _)
      (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))) (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd))))))))
      (nr _ (.inr (.inr (.inr (.inr (.inr (.inl hd))))))) hc
    rcases p with ⟨v, j⟩
    simp only [invK] at hcap hc hown hni ⊢
    exact reg_invoke env ⟨hcap, decomp_invoke k _ _ _ _, hown⟩ hc hni
  have h8 : (∀ d ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata, RG k ts d.1) ↔
      ∀ dd, Y.donedata = some dd → ¬(dd.content.isSome ∧ dd.params ≠ []) ∧ ParamsOk dd.params := by
    rcases hdd : Y.donedata with _ | dd
    · simp [ddKidsOpt]
    · have hd : (stateFragment Y.id ++ ":dd", k.donedata (k.iri (stateFragment Y.id))
          (stateFragment Y.id ++ ":dd") dd) ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata := by
        simp [ddKidsOpt, hdd]
      obtain ⟨hcap, hc, hown⟩ := gk (kidsL_ddKidsOpt k _ _)
        (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd))))))))) (by simp [containers]) _ hd
      have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))))
        (nr _ (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))) hc
      have := reg_donedata env ⟨hcap, decomp_donedata k _ _ dd, hown⟩ hc hni
      simp only [ddKidsOpt, List.mem_singleton, forall_eq, Option.some.injEq, forall_eq', this]
      try rfl
  have hx : ∀ d ∈ extKids k (stateFragment Y.id) Y.extensions, RG k ts d.1 := by
    intro d hd
    obtain ⟨hcap, hc, hown⟩ := gk (kidsL_ext k _ _)
      (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inr hd))))))))) (by simp [containers]) _ hd
    have hni := kid_noinit h (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inr hd)))))))))
      (nr _ (.inr (.inr (.inr (.inr (.inr (.inr (.inr hd)))))))) hc
    simp only [extKids, List.mem_map] at hd
    obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
    exact good_ext env hcap hown hc hni
  have e : (∀ d ∈ kidsOf k pt A Y, RG k ts d.1) ↔
      (∀ d ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement, RG k ts d.1) ∧
      (∀ d ∈ Y.transitions.zipIdx.map (transK k (stateFragment Y.id)
        (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)), RG k ts d.1) ∧
      (∀ d ∈ Y.onentry.zipIdx.map (hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry"), RG k ts d.1) ∧
      (∀ d ∈ Y.onexit.zipIdx.map (hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit"), RG k ts d.1) ∧
      (∀ d ∈ Y.data.zipIdx.map (dataK k (stateFragment Y.id)), RG k ts d.1) ∧
      (∀ d ∈ Y.invokes.zipIdx.map (invK k (stateFragment Y.id)), RG k ts d.1) ∧
      (∀ d ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata, RG k ts d.1) := by
    constructor
    · intro hall
      exact ⟨fun d hd => hall d (kidsOf_mem.2 (.inr (.inl hd))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inl hd)))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inr (.inl hd))))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inl hd)))))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inl hd))))))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))),
        fun d hd => hall d (kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))))⟩
    · rintro ⟨a2, a3, a4, a5, a6, a7, a8⟩ d hd
      rcases kidsOf_mem.1 hd with hd | hd | hd | hd | hd | hd | hd | hd | hd
      · exact h1 d hd
      · exact a2 d hd
      · exact a3 d hd
      · exact a4 d hd
      · exact a5 d hd
      · exact a6 d hd
      · exact a7 d hd
      · exact a8 d hd
      · exact hx d hd
  rw [e, h2, h3, h4, h5, h6, h7, h8]
  rfl

end Vertep.Rdf.Equiv
