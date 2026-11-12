import VertepLean.Rdf.Equiv.DataIds
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# History default cycles

`hd_iff`: the `vt:historyDefault` triples of the projection are the edges `historyDefaultEdges`
of `WellFormed` between history states (the projection omits default targets that are not
history states, which the edges filter out). `acyclic_iff`: `HistoryDefaultsAcyclic`, which
follows paths up to the number of states, is the absence of cycles of any length (pigeonhole).
`cycle_iff`: the cycle component of `vt:HistoryShape` holds exactly when history defaults are
acyclic.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-! ## `vt:historyDefault` triples -/

mutual
theorem mem_hd (k : Ctx) (pp : Option State) : ∀ (s : State) (u : Triple),
    u ∈ historyDefaults k pp s ↔ ∃ q ∈ State.parented pp s, q.2.isHistory = true ∧
      ∃ t ∈ historyDefaultTargets q.2 q.1, k.histories.lookup t = some true ∧
        u = ⟨k.iri (stateFragment q.2.id), vt "historyDefault", k.iri (stateFragment t)⟩
  | .mk id ig el ini ie cs trs en ex da inv dd xs, u => by
    have key : ∀ own : List Triple, historyDefaults k pp (State.mk id ig el ini ie cs trs en ex da inv dd xs) = own ++ historyDefaultsList k (some (State.mk id ig el ini ie cs trs en ex da inv dd xs)) cs →
        (u ∈ own ↔ (State.mk id ig el ini ie cs trs en ex da inv dd xs).isHistory = true ∧ ∃ t ∈ historyDefaultTargets (State.mk id ig el ini ie cs trs en ex da inv dd xs) pp, k.histories.lookup t = some true ∧
          u = ⟨k.iri (stateFragment (State.mk id ig el ini ie cs trs en ex da inv dd xs).id), vt "historyDefault", k.iri (stateFragment t)⟩) →
        (u ∈ historyDefaults k pp (State.mk id ig el ini ie cs trs en ex da inv dd xs) ↔ ∃ q ∈ State.parented pp (State.mk id ig el ini ie cs trs en ex da inv dd xs), q.2.isHistory = true ∧
          ∃ t ∈ historyDefaultTargets q.2 q.1, k.histories.lookup t = some true ∧
            u = ⟨k.iri (stateFragment q.2.id), vt "historyDefault", k.iri (stateFragment t)⟩) := by
      intro own h1 h2
      rw [h1, parented_eq, List.mem_append, mem_hds k _ cs u, h2]
      simp only [List.mem_cons, exists_eq_or_imp]
      rfl
    cases el with
    | history h =>
      refine key ((historyDefaultTargets (State.mk id ig (.history h) ini ie cs trs en ex da inv dd xs) pp).filterMap
        fun t => if k.histories.lookup t = some true then
          some (⟨k.iri (stateFragment id), vt "historyDefault", k.iri (stateFragment t)⟩ : Triple) else none)
        (by simp [historyDefaults]) ?_
      simp only [List.mem_filterMap]
      constructor
      · rintro ⟨t, ht, he⟩
        split at he
        · rename_i hl; cases he; exact ⟨rfl, t, ht, by simpa using hl, rfl⟩
        · cases he
      · rintro ⟨_, t, ht, hl, rfl⟩
        exact ⟨t, ht, by simp [hl]; rfl⟩
    | state => exact key [] (by simp [historyDefaults]) (by simp [State.isHistory, State.element])
    | parallel => exact key [] (by simp [historyDefaults]) (by simp [State.isHistory, State.element])
    | final => exact key [] (by simp [historyDefaults]) (by simp [State.isHistory, State.element])
theorem mem_hds (k : Ctx) (pp : Option State) : ∀ (cs : List State) (u : Triple),
    u ∈ historyDefaultsList k pp cs ↔ ∃ q ∈ State.parentedList pp cs, q.2.isHistory = true ∧
      ∃ t ∈ historyDefaultTargets q.2 q.1, k.histories.lookup t = some true ∧
        u = ⟨k.iri (stateFragment q.2.id), vt "historyDefault", k.iri (stateFragment t)⟩
  | [], u => by simp [historyDefaultsList, State.parentedList]
  | c :: cs, u => by
    rw [historyDefaultsList, State.parentedList, List.mem_append, mem_hd k pp c u, mem_hds k pp cs u]
    simp only [List.mem_append]
    constructor
    · rintro (⟨q, hq, h⟩ | ⟨q, hq, h⟩)
      · exact ⟨q, .inl hq, h⟩
      · exact ⟨q, .inr hq, h⟩
    · rintro ⟨q, hq | hq, h⟩
      · exact .inl ⟨q, hq, h⟩
      · exact .inr ⟨q, hq, h⟩
end

section
variable {c : Chart} (hnd : c.ids.Nodup)
include hnd

theorem find_iff {t : String} {b : State} : c.find t = some b ↔ b ∈ c.allStates ∧ b.id = t := by
  constructor
  · intro h
    unfold Chart.find at h
    exact ⟨List.mem_of_find?_eq_some h, by simpa using List.find?_some h⟩
  · rintro ⟨hb, rfl⟩; exact CompileProof.find_eq hnd hb

theorem mem_hdEdges {a b : State} :
    b ∈ historyDefaultEdges c a ↔ b.isHistory = true ∧ b ∈ c.allStates ∧
      b.id ∈ (match a.transitions.head?, c.parentOf a with
        | some t, _ => t.targets
        | none, some p => initialTargets p
        | none, none => []) := by
  unfold historyDefaultEdges
  simp only [List.mem_filter, List.mem_filterMap, find_iff hnd]
  constructor
  · rintro ⟨⟨t, ht, hb, rfl⟩, hh⟩; exact ⟨hh, hb, ht⟩
  · rintro ⟨hh, hb, ht⟩; exact ⟨⟨_, ht, hb, rfl⟩, hh⟩

/-- The projection's history default targets name the same history states as `WellFormed`'s. -/
theorem hdTargets_iff {a : State} (ha : a ∈ c.allStates) (t : String) :
    (t ∈ historyDefaultTargets a (c.parentOf a) ∧ ∃ T ∈ c.allStates, T.id = t ∧ T.isHistory = true) ↔
    (t ∈ (match a.transitions.head?, c.parentOf a with
        | some t, _ => t.targets
        | none, some p => initialTargets p
        | none, none => []) ∧ ∃ T ∈ c.allStates, T.id = t ∧ T.isHistory = true) := by
  unfold historyDefaultTargets
  rcases hh : a.transitions.head? with _ | tr
  · rcases hp : c.parentOf a with _ | p
    · simp
    · simp only
      have hpa := (CompileProof.parentOf_some hnd ha hp).1
      unfold initialTargets
      by_cases hk : p.kind = .compound
      · have hb : (p.kind == .compound) = true := by rw [hk]; rfl
        have hb' : (p.kind != .compound) = false := by rw [hk]; rfl
        simp only [hb, hb', ↓reduceIte, Bool.false_eq_true]
        by_cases hi : (!p.initial.isEmpty) = true
        · simp only [hi, ↓reduceIte]
        · simp only [hi, Bool.false_eq_true, ↓reduceIte]
          rcases p.initialElement with _ | ie
          · simp only
            rcases hf : p.children.find? (fun c => !c.isHistory) with _ | ch
            · simp [hf]
            · simp only [hf, List.not_mem_nil, false_and, false_iff, not_and, List.mem_singleton]
              rintro rfl ⟨T, hT, hid, hTh⟩
              have hch := List.mem_of_find?_eq_some hf
              have hch' := List.find?_some hf
              have := CompileProof.state_inj hnd hT (CompileProof.mem_of_child c hpa hch) hid
              subst this
              simp [hTh] at hch'
          · rfl
      · have hb : (p.kind == .compound) = false := by
          rcases hkk : p.kind <;> rw [hkk] at hk <;> first | rfl | exact absurd rfl hk
        have hb' : (p.kind != .compound) = true := by
          rcases hkk : p.kind <;> rw [hkk] at hk <;> first | rfl | exact absurd rfl hk
        simp [hb, hb']
  · rfl

theorem hd_iff (base : String) {x y : Term} :
    (⟨x, vt "historyDefault", y⟩ : Triple) ∈ project c base ↔
      ∃ a ∈ c.allStates, a.isHistory = true ∧ x = (ctxOf c base).iri (stateFragment a.id) ∧
        ∃ b ∈ historyDefaultEdges c a, y = (ctxOf c base).iri (stateFragment b.id) := by
  have hmem : (⟨x, vt "historyDefault", y⟩ : Triple) ∈ project c base ↔
      (⟨x, vt "historyDefault", y⟩ : Triple) ∈ historyDefaultsList (ctxOf c base) none c.states := by
    refine ⟨hd_triple c base, fun h => ?_⟩
    rw [project_eq]; exact List.mem_append_right _ h
  rw [hmem, mem_hds]
  simp only [Triple.mk.injEq, true_and]
  constructor
  · rintro ⟨q, hq, hh, t, ht, hl, rfl, rfl⟩
    have hqa := parented_allStates hq
    have hp := CompileProof.parentOf_eq hnd (show (q.1, q.2) ∈ c.parented from hq)
    obtain ⟨T, hT, rfl, hTh⟩ := (histories_lookup hnd base t).1 hl
    rw [← hp] at ht
    have := (hdTargets_iff hnd hqa T.id).1 ⟨ht, T, hT, rfl, hTh⟩
    exact ⟨q.2, hqa, hh, rfl, T, (mem_hdEdges hnd).2 ⟨hTh, hT, this.1⟩, rfl⟩
  · rintro ⟨a, ha, hh, rfl, b, hb, rfl⟩
    obtain ⟨q, hq, rfl⟩ := allStates_parented ha
    have hp := CompileProof.parentOf_eq hnd (show (q.1, q.2) ∈ c.parented from hq)
    obtain ⟨hbh, hba, hbt⟩ := (mem_hdEdges hnd).1 hb
    have := (hdTargets_iff hnd ha b.id).2 ⟨hbt, b, hba, rfl, hbh⟩
    rw [hp] at this
    exact ⟨q, hq, hh, b.id, this.1, (histories_lookup hnd base _).2 ⟨b, hba, rfl, hbh⟩, rfl, rfl⟩

end

section
variable (c : Chart) (base : String)

theorem id_mem {Z : State} (hZ : Z ∈ c.allStates) :
    (⟨(ctxOf c base).iri (stateFragment Z.id), vt "id", strLit Z.id⟩ : Triple) ∈ project c base := by
  obtain ⟨q, hq, rfl⟩ := allStates_parented hZ
  obtain ⟨i', A'', h⟩ := parentedList_sub (ctxOf c base) none 0 [] c.states q hq
  have := h ⟨(ctxOf c base).iri (stateFragment q.2.id), vt "id", strLit q.2.id⟩
    (by rw [state_eq']; simp [stateSelf, Ctx.node, str])
  rw [project_eq]; simp only [List.mem_append]; exact .inl (.inl (.inr (by simpa [parT] using this)))

theorem el_mem {Z : State} (hZ : Z ∈ c.allStates) :
    (⟨(ctxOf c base).iri (stateFragment Z.id), rdfType, vt (elementClass Z.element)⟩ : Triple) ∈
      project c base := by
  obtain ⟨q, hq, rfl⟩ := allStates_parented hZ
  obtain ⟨i', A'', h⟩ := parentedList_sub (ctxOf c base) none 0 [] c.states q hq
  have := h ⟨(ctxOf c base).iri (stateFragment q.2.id), rdfType, vt (elementClass q.2.element)⟩
    (by rw [state_eq']; simp [stateSelf, Ctx.node])
  rw [project_eq]; simp only [List.mem_append]; exact .inl (.inl (.inr (by simpa [parT] using this)))

end

/-! ## Walks along history default edges -/

section
variable {c : Chart}

/-- One history default edge between states of the chart. -/
def HE (c : Chart) (a b : State) : Prop :=
  a ∈ c.allStates ∧ a.isHistory = true ∧ b ∈ historyDefaultEdges c a

theorem reach_walk : ∀ (n : Nat) (F : List State) (Z : State), Z ∈ historyReach c n F →
    ∃ L, L < n ∧ ∃ w : Nat → State, w 0 ∈ F ∧ (∀ b, b < L → w (b + 1) ∈ historyDefaultEdges c (w b)) ∧
      w L = Z
  | 0, F, Z, h => by simp [historyReach] at h
  | n + 1, F, Z, h => by
    simp only [historyReach, List.mem_append] at h
    rcases h with h | h
    · exact ⟨0, by omega, fun _ => Z, h, fun b hb => by omega, rfl⟩
    · obtain ⟨L, hL, w, h0, hs, hw⟩ := reach_walk n _ Z h
      obtain ⟨a, ha, h0'⟩ := List.mem_flatMap.1 h0
      refine ⟨L + 1, by omega, fun b => if b = 0 then a else w (b - 1), by simpa using ha, ?_, by simpa using hw⟩
      intro b hb
      rcases b with _ | b
      · simpa using h0'
      · simpa using hs b (by omega)

theorem reach_of_walk' (w : Nat → State) : ∀ (L fuel : Nat) (F : List State), L < fuel →
    w 0 ∈ F → (∀ b, b < L → w (b + 1) ∈ historyDefaultEdges c (w b)) →
      w L ∈ historyReach c fuel F := by
  intro L
  induction L generalizing w with
  | zero =>
    intro fuel F hf h0 _
    obtain ⟨f', rfl⟩ : ∃ f', fuel = f' + 1 := ⟨fuel - 1, by omega⟩
    simp only [historyReach]; exact List.mem_append_left _ h0
  | succ L ih =>
    intro fuel F hf h0 hstep
    obtain ⟨f', rfl⟩ : ∃ f', fuel = f' + 1 := ⟨fuel - 1, by omega⟩
    simp only [historyReach]
    refine List.mem_append_right _ ?_
    exact ih (fun b => w (b + 1)) f' _ (by omega)
      (List.mem_flatMap.2 ⟨w 0, h0, hstep 0 (by omega)⟩)
      (fun b hb => hstep (b + 1) (by omega))

variable (hnd : c.ids.Nodup)
include hnd

theorem edge_mem {a b : State} (h : b ∈ historyDefaultEdges c a) : b ∈ c.allStates ∧ b.isHistory = true :=
  ⟨((mem_hdEdges hnd).1 h).2.1, ((mem_hdEdges hnd).1 h).1⟩

theorem he_first {a b : State} (h : Relation.TransGen (HE c) a b) : a.isHistory = true := by
  induction h with
  | single h => exact h.2.1
  | tail _ _ ih => exact ih

theorem he_last {a b : State} (h : Relation.TransGen (HE c) a b) : b ∈ c.allStates := by
  induction h with
  | single h => exact (edge_mem hnd h.2.2).1
  | tail _ h _ => exact (edge_mem hnd h.2.2).1

/-- **`HistoryDefaultsAcyclic` is the absence of cycles along history default edges.** -/
theorem acyclic_iff : HistoryDefaultsAcyclic c ↔
    ∀ a ∈ c.allStates, a.isHistory = true → ¬ Relation.TransGen (HE c) a a := by
  constructor
  · intro hac a ha hh hc
    obtain ⟨m, f, hm, hf0, hfm, hstep⟩ := CompileProof.transGen_walk hc
    have hfa : ∀ k, k ≤ m → f k ∈ c.allStates := by
      intro k hk
      rcases k with _ | k
      · rw [hf0]; exact ha
      · exact (edge_mem hnd (hstep k (by omega)).2.2).1
    obtain ⟨i, j, hij, hjn, hjm, hfij⟩ : ∃ i j, i < j ∧ j - i ≤ c.allStates.length ∧ j ≤ m ∧
        f i = f j := by
      by_cases hmn : m ≤ c.allStates.length
      · exact ⟨0, m, hm, by omega, Nat.le_refl _, by rw [hf0, hfm]⟩
      · obtain ⟨i, j, hij, hj, he⟩ := CompileProof.pigeonhole c.allStates.length
          (fun k => CompileProof.pos c (f k) - 1) (fun k hk => by
            have := CompileProof.pos_lt (hfa k (by omega))
            have : 0 < CompileProof.pos c (f k) := Nat.succ_pos _
            omega)
        refine ⟨i, j, hij, by omega, by omega, ?_⟩
        have h1 : 0 < CompileProof.pos c (f i) := Nat.succ_pos _
        have h2 : 0 < CompileProof.pos c (f j) := Nat.succ_pos _
        exact CompileProof.pos_inj hnd (hfa i (by omega)) (hfa j (by omega)) (by omega)
    have hhi := (hstep i (by omega)).2.1
    have hreach := reach_of_walk' (c := c) (fun b => f (i + 1 + b)) (j - i - 1) c.allStates.length
      (historyDefaultEdges c (f i)) (by omega) (by simpa using (hstep i (by omega)).2.2)
      (fun b hb => by
        have := (hstep (i + 1 + b) (by omega)).2.2
        simpa [Nat.add_assoc] using this)
    have hji : i + 1 + (j - i - 1) = j := by omega
    simp only [hji, ← hfij] at hreach
    exact hac (f i) (hfa i (by omega)) hhi (List.mem_map_of_mem hreach)
  · intro hno h hh hhist hmem
    obtain ⟨Z, hZ, hZid⟩ := List.mem_map.1 hmem
    obtain ⟨L, hL, w, h0, hs, hw⟩ := reach_walk _ _ Z hZ
    have hwm : ∀ b, b ≤ L → w b ∈ c.allStates ∧ (w b).isHistory = true := by
      intro b hb
      rcases b with _ | b
      · exact edge_mem hnd h0
      · exact edge_mem hnd (hs b (by omega))
    have hZh : Z = h := CompileProof.state_inj hnd (hw ▸ (hwm L (Nat.le_refl _)).1) hh hZid
    have : ∀ b, b ≤ L → Relation.TransGen (HE c) h (w b) := by
      intro b
      induction b with
      | zero => intro _; exact .single ⟨hh, hhist, h0⟩
      | succ b ih =>
        intro hb
        exact .tail (ih (by omega)) ⟨(hwm b (by omega)).1, (hwm b (by omega)).2, hs b (by omega)⟩
    have := this L (Nat.le_refl _)
    rw [hw, hZh] at this
    exact hno h hh hhist this

end

/-! ## The cycle component -/

section
variable (c : Chart) (base : String) (hnd : c.ids.Nodup)
include hnd

theorem plus_hd {a : State} (ha : a ∈ c.allStates) {z : Term}
    (h : Plus (fun x y => (⟨x, vt "historyDefault", y⟩ : Triple) ∈ project c base)
      ((ctxOf c base).iri (stateFragment a.id)) z) :
    ∃ b, z = (ctxOf c base).iri (stateFragment b.id) ∧ Relation.TransGen (HE c) a b := by
  induction h with
  | one h =>
    obtain ⟨a', ha', hh, hx, b, hb, rfl⟩ := (hd_iff hnd base).1 h
    have := CompileProof.state_inj hnd ha ha' (stateFragment_inj ((iri_inj _).1 hx))
    subst this
    exact ⟨b, rfl, .single ⟨ha, hh, hb⟩⟩
  | tail _ h ih =>
    obtain ⟨b, rfl, hab⟩ := ih
    obtain ⟨a', ha', hh, hx, b', hb', rfl⟩ := (hd_iff hnd base).1 h
    have := CompileProof.state_inj hnd (he_last hnd hab) ha' (stateFragment_inj ((iri_inj _).1 hx))
    subst this
    exact ⟨b', rfl, .tail hab ⟨ha', hh, hb'⟩⟩

theorem hd_plus {a b : State} (h : Relation.TransGen (HE c) a b) :
    Plus (fun x y => (⟨x, vt "historyDefault", y⟩ : Triple) ∈ project c base)
      ((ctxOf c base).iri (stateFragment a.id)) ((ctxOf c base).iri (stateFragment b.id)) := by
  induction h with
  | single h => exact .one ((hd_iff hnd base).2 ⟨_, h.1, h.2.1, rfl, _, h.2.2, rfl⟩)
  | tail _ h ih => exact .tail ih ((hd_iff hnd base).2 ⟨_, h.1, h.2.1, rfl, _, h.2.2, rfl⟩)

/-- **The cycle component of `vt:HistoryShape` holds at every history node exactly when history
defaults are acyclic.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "history default cycles; the full theorem is shapes_equivalence"}
theorem cycle_iff :
    (∀ x, Ty (toRDF c base) x "History" → CompHolds (toRDF c base) none cycleC x) ↔
      HistoryDefaultsAcyclic c := by
  rw [acyclic_iff hnd]
  have hR : ∀ x y, ValueNode (Graph.ofList (project c base)) (.pred (vi "historyDefault")) x y ↔
      (⟨x, vt "historyDefault", y⟩ : Triple) ∈ project c base := by
    intro x y; simp
  simp only [cycleC, holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons,
    compsHold_nil, and_true, holds_disjoint, vn_some, valueNode_seq_cons, valueNode_seq_nil,
    valueNode_oneOrMore, ShapesL.P, valueNode_pred, iri_vi, toRDF, mem_ofList', Ty, rdf_type_eq]
  constructor
  · intro H a ha hh hc
    have hty : (⟨(ctxOf c base).iri (stateFragment a.id), rdfType, vt "History"⟩ : Triple) ∈
        project c base := by
      have := el_mem c base ha
      rcases a with ⟨_, _, el⟩; cases el <;> simp_all [elementClass, State.isHistory, State.element]
    exact H _ hty (strLit a.id) ⟨_, (Plus.congr hR).2 (hd_plus c base hnd hc), _, id_mem c base ha, rfl⟩ (id_mem c base ha)
  · rintro H x _ v ⟨z, hp, z', hz, rfl⟩ hx
    obtain ⟨a, ha, rfl, rfl⟩ := id_triple c base hx
    obtain ⟨b, rfl, hab⟩ := plus_hd c base hnd ha ((Plus.congr hR).1 hp)
    obtain ⟨Y, hY, hYe, hv⟩ := id_triple c base hz
    have h1 := CompileProof.state_inj hnd (he_last hnd hab) hY (stateFragment_inj ((iri_inj _).1 hYe))
    subst h1
    have h2 := CompileProof.state_inj hnd ha (he_last hnd hab) (strLit_inj.1 hv)
    subst h2
    exact H a ha (he_first hnd hab) hab

end

end Vertep.Rdf.Equiv
