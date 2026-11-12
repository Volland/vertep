import VertepLean.Rdf.Equiv.StateRegion
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes in the regions of the states

By structural recursion over the state tree: the node shapes hold in the regions of a state and
all its descendants exactly when `SOK` holds of every state of the subtree with its parent.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

/-- The rules of a state that the node shapes in its region check. -/
def SOK (k : Ctx) (ts : List Triple) (Kn : String → Prop) (pp : Option State) (Y : State) : Prop :=
  ((¬(Y.initial ≠ [] ∧ Y.initialElement.isSome) ∧ Y.id ≠ "" ∧
    ∀ x ∈ Y.initial, TR k ts Kn x (k.iri (stateFragment Y.id))) ∧ ElemOK (pp.isSome = true) Y) ∧
  KidsOK k ts Kn (parT k pp) [k.iri "chart"] Y

theorem plus_head_iff {R : Term → Term → Prop} {x v : Term} :
    Plus R x v ↔ ∃ w, R x w ∧ (w = v ∨ Plus R w v) := by
  constructor
  · intro h
    rcases h.cases_head with h | ⟨w, h1, h2⟩
    · exact ⟨v, h, .inl rfl⟩
    · exact ⟨w, h1, .inr h2⟩
  · rintro ⟨w, h1, rfl | h2⟩
    · exact .one h1
    · exact Plus.head h1 h2

theorem kidsOK_A {Kn : String → Prop} {pt : Term} {A' : List Term} {Y : State} :
    KidsOK k ts Kn pt (k.iri "chart" :: A') Y ↔ KidsOK k ts Kn pt [k.iri "chart"] Y := by
  have : scopeOf pt (k.iri "chart" :: A') (k.iri (stateFragment Y.id)) Y.element =
      scopeOf pt [k.iri "chart"] (k.iri (stateFragment Y.id)) Y.element := by
    cases Y.element <;> rfl
  simp only [KidsOK, this]

/-- What the recursion carries for a state `Y` with parent `pp`. -/
structure RecH (k : Ctx) (ts : List Triple) (P : List (Option State × State)) (pp : Option State)
    (Y : State) (i : Nat) (A' : List Term) : Prop where
  sh : SH k ts i (parT k pp) (k.iri "chart" :: A') Y
  sub : ∀ q ∈ State.parented pp Y, q ∈ P
  anc : ∀ v, (v = parT k pp ∨ Plus (ValueNode (Graph.ofList ts) (ShapesL.I "child")) (parT k pp) v) ↔
    v ∈ k.iri "chart" :: A'
  up : parT k pp = k.iri "chart" ∨ Plus (ValueNode (Graph.ofList ts) upPath) (parT k pp) (k.iri "chart")
  parTy : (⟨parT k pp, rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ pp.isSome = true
  parNo : pp = none → ∀ v, (⟨v, vt "child", parT k pp⟩ : Triple) ∉ ts

theorem RecH.cont (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) {pp : Option State}
    {Y : State} {i : Nat} {A' : List Term} (h : RecH k ts P pp Y i A') :
    Contained' k ts (k.iri (stateFragment Y.id)) (parT k pp) where
  conts s := by
    have hmem : (pp, Y) ∈ P := h.sub _ (by rw [parented_eq]; exact List.mem_cons_self)
    constructor
    · rintro ⟨l, hl, ht⟩
      obtain ⟨f, hf, hh⟩ := env.link _ ht ⟨l, hl, rfl⟩
      simp only [iri_inj] at hf; subst hf
      rcases hh with ⟨g, hg, _⟩ | ⟨q, hq, hqf, hs⟩
      · exact absurd hg not_ch_sf
      · have hid := stateFragment_inj hqf
        have := nodup_fst_eq' hnd hq hmem hid.symm
        exact hs.trans (by rw [this])
    · rintro rfl
      exact ⟨"child", by simp [containers], h.sh.cap.sub.subset (by
        rw [state_eq']; simp [stateSelf, Ctx.node])⟩
  up := h.up
where
  nodup_fst_eq' {P : List (Option State × State)} (hnd : (P.map (fun q => q.2.id)).Nodup)
      {a b : Option State × State} (ha : a ∈ P) (hb : b ∈ P) (h : a.2.id = b.2.id) : a = b := by
    induction P with
    | nil => simp at ha
    | cons x P ih =>
      simp only [List.map_cons, List.nodup_cons, List.mem_map] at hnd
      simp only [List.mem_cons] at ha hb
      rcases ha with rfl | ha <;> rcases hb with rfl | hb
      · rfl
      · exact absurd ⟨b, hb, h.symm⟩ hnd.1
      · exact absurd ⟨a, ha, h⟩ hnd.1
      · exact ih hnd.2 ha hb

theorem RecH.stctx (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) {pp : Option State}
    {Y : State} {i : Nat} {A' : List Term} (h : RecH k ts P pp Y i A') :
    StCtx k ts i (parT k pp) (k.iri "chart" :: A') Y :=
  ⟨h.sh.coreCap, h.sh.ownFrag, h.cont env hnd⟩

/-- One state: the node shapes hold in its own region and its non-state children's regions
exactly when `SOK` holds. -/
theorem rg_state (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x)
    {pp : Option State} {Y : State} {i : Nat} {A' : List Term} (h : RecH k ts P pp Y i A')
    (hch : ∀ c ∈ Y.children, ∀ l, (⟨k.iri (stateFragment c.id), rdfType, vt l⟩ : Triple) ∈ ts ↔
      l = "StateNode" ∨ l = elementClass c.element) :
    RG k ts (stateFragment Y.id) ↔ SOK k ts Kn pp Y := by
  have hst := h.stctx env hnd
  have hR : ∀ w, ValueNode (Graph.ofList ts) (ShapesL.I "child") (k.iri (stateFragment Y.id)) w ↔ w = parT k pp := by
    intro w
    simp only [ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred, iri_vi, mem_ofList']
    constructor
    · intro hw; exact (hst.cont.conts w).1 ⟨"child", by simp [containers], hw⟩
    · rintro rfl; exact h.sh.cap.sub.subset (by rw [state_eq']; simp [stateSelf, Ctx.node])
  have hA : ∀ v, Plus (ValueNode (Graph.ofList ts) (ShapesL.I "child")) (k.iri (stateFragment Y.id)) v ↔
      v ∈ k.iri "chart" :: A' := by
    intro v; rw [plus_head_iff, ← h.anc v]
    simp only [hR, exists_eq_left]
    constructor
    · rintro (h1 | h1)
      · exact .inl h1.symm
      · exact .inr h1
    · rintro (h1 | h1)
      · exact .inl h1.symm
      · exact .inr h1
  have hni : ∀ s, (⟨s, vt "initial", k.iri (stateFragment Y.id)⟩ : Triple) ∉ ts := by
    intro s hs
    have := (hst.ty "TargetRef").1 (env.initTy _ _ hs)
    rcases Y with ⟨_, _, el⟩; cases el <;> simp [elementClass, State.element] at this
  have hchH : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "History"⟩ : Triple) ∈ ts ↔
      c.isHistory = true := by
    intro c hc; rw [hch c hc]
    rcases c with ⟨_, _, el⟩; cases el <;> simp [elementClass, State.isHistory, State.element]
  have hchT : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts :=
    fun c hc => (hch c hc _).2 (.inl rfl)
  rw [RG, region_iff env hst.cx, good_state_node env hst rfl hchH hchT _ h.parTy Kn hSN hA hni]
  have hk := reg_kids env hst Kn hSN
  simp only [RG] at hk
  rw [hk, kidsOK_A, SOK]

theorem mem_parentedList_of {pp : Option State} {cs : List State} {c : State} (hc : c ∈ cs)
    {q : Option State × State} (hq : q ∈ State.parented pp c) : q ∈ State.parentedList pp cs := by
  induction cs with
  | nil => simp at hc
  | cons c' cs ih =>
    rw [State.parentedList, List.mem_append]
    rcases List.mem_cons.1 hc with rfl | hc
    · exact .inl hq
    · exact .inr (ih hc)

/-- The children of a state inherit the recursion's hypotheses. -/
theorem RecH.kids (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) {pp : Option State}
    {Y : State} {i : Nat} {A' : List Term} (h : RecH k ts P pp Y i A') :
    ∀ p ∈ Y.children.zipIdx 0, RecH k ts P (some Y) p.1 p.2 (A' ++ [k.iri (stateFragment Y.id)]) := by
  have hst := h.stctx env hnd
  have hsh := capList (k := k) (ts := ts) (k.iri (stateFragment Y.id))
    (k.iri "chart" :: (A' ++ [k.iri (stateFragment Y.id)])) Y.children 0 h.sh.kidsCap h.sh.kidsNd h.sh.kidsOwn
  have hR : ∀ w, ValueNode (Graph.ofList ts) (ShapesL.I "child") (k.iri (stateFragment Y.id)) w ↔ w = parT k pp := by
    intro w
    simp only [ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred, iri_vi, mem_ofList']
    constructor
    · intro hw; exact (hst.cont.conts w).1 ⟨"child", by simp [containers], hw⟩
    · rintro rfl; exact h.sh.cap.sub.subset (by rw [state_eq']; simp [stateSelf, Ctx.node])
  intro p hp
  refine ⟨hsh p hp, fun q hq => h.sub q ?_, ?_, ?_, ?_, by simp⟩
  · rw [parented_eq]
    exact List.mem_cons_of_mem _ (mem_parentedList_of (fst_mem_zipIdx hp) hq)
  · intro v
    simp only [parT, List.cons_append, List.mem_cons, List.mem_append, List.mem_singleton,
      List.not_mem_nil, or_false]
    rw [plus_head_iff]
    simp only [hR, exists_eq_left]
    have ha := h.anc v
    simp only [List.mem_cons] at ha
    constructor
    · rintro (h1 | h1 | h1)
      · exact .inr (.inr h1)
      · rcases ha.1 (.inl h1.symm) with h2 | h2
        · exact .inl h2
        · exact .inr (.inl h2)
      · rcases ha.1 (.inr h1) with h2 | h2
        · exact .inl h2
        · exact .inr (.inl h2)
    · rintro (h1 | h1 | h1)
      · rcases ha.2 (.inl h1) with h2 | h2
        · exact .inr (.inl h2.symm)
        · exact .inr (.inr h2)
      · rcases ha.2 (.inr h1) with h2 | h2
        · exact .inr (.inl h2.symm)
        · exact .inr (.inr h2)
      · exact .inl h1
  · right
    have h1 : ValueNode (Graph.ofList ts) upPath (k.iri (stateFragment Y.id)) (parT k pp) := by
      rw [vn_up]; exact ⟨"child", by simp [containers], by
        simpa using h.sh.cap.sub.subset (by rw [state_eq']; simp [stateSelf, Ctx.node])⟩
    rcases h.up with he | hp'
    · rw [he] at h1; exact .one h1
    · exact Plus.head h1 hp'
  · simp only [parT, Option.isSome_some, iff_true]; exact (hst.ty _).2 (.inl rfl)

mutual
/-- **The node shapes in a state's subtree.** -/
theorem reg_state (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x) :
    ∀ (Y : State) (pp : Option State) (i : Nat) (A' : List Term), RecH k ts P pp Y i A' →
      ((∀ Z ∈ Y.flatten, RG k ts (stateFragment Z.id)) ↔ ∀ q ∈ State.parented pp Y, SOK k ts Kn q.1 q.2)
  | .mk id idGen el ini ie children trs en ex da inv dd xs, pp, i, A', h => by
    have hk := h.kids env hnd
    have hch : ∀ c ∈ (State.mk id idGen el ini ie children trs en ex da inv dd xs).children, ∀ l,
        (⟨k.iri (stateFragment c.id), rdfType, vt l⟩ : Triple) ∈ ts ↔ l = "StateNode" ∨ l = elementClass c.element := by
      intro c hc l
      obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hc
      have hm : ((State.mk id idGen el ini ie children trs en ex da inv dd xs).children[j], j) ∈
          (State.mk id idGen el ini ie children trs en ex da inv dd xs).children.zipIdx 0 :=
        List.mem_zipIdx_iff_getElem?.2 (by simp)
      exact ((hk _ hm).stctx env hnd).ty l
    have ih := reg_states env hnd Kn hSN (some (.mk id idGen el ini ie children trs en ex da inv dd xs))
      (A' ++ [k.iri (stateFragment id)]) children 0 hk
    rw [flatten_eq, parented_eq, List.forall_mem_cons, List.forall_mem_cons, rg_state env hnd Kn hSN h hch]
    simp only [State.children] at ih ⊢
    rw [ih]
theorem reg_states (env : Env k ts P) (hnd : (P.map (fun q => q.2.id)).Nodup) (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x)
    (pp : Option State) (A' : List Term) :
    ∀ (cs : List State) (i : Nat), (∀ p ∈ cs.zipIdx i, RecH k ts P pp p.1 p.2 A') →
      ((∀ Z ∈ State.flattenList cs, RG k ts (stateFragment Z.id)) ↔
        ∀ q ∈ State.parentedList pp cs, SOK k ts Kn q.1 q.2)
  | [], _, _ => by simp [State.flattenList, State.parentedList]
  | c :: cs, i, h => by
    rw [flattenList_cons, State.parentedList, List.forall_mem_append, List.forall_mem_append,
      reg_state env hnd Kn hSN c pp i A' (h (c, i) (by simp)),
      reg_states env hnd Kn hSN pp A' cs (i + 1) (fun p hp => h p (by simp [hp]))]
end

end Vertep.Rdf.Equiv
