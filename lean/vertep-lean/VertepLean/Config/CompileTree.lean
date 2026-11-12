import VertepLean.Config.CompileLoop

/-!
# Compile correctness: the compiled tree

The compiled chart of a well-formed chart, read through `Spec`: state `pos c s` is IR state `s`,
its parent is the position of `s`'s parent (the root for top-level states), and descent in the
compiled chart is descent in the IR (`isDesc_pos_iff`), with ancestors in the same order
(`ancestors_pos`).
-/

namespace Vertep.CompileProof

open Vertep

section
variable {c : Chart} {cc : CChart} (hwf : WellFormed c) (hsp : Spec c cc)

/-- The compiled parent of IR state `s`. -/
def cpar (c : Chart) (s : State) : Nat :=
  match c.parentOf s with
  | some q => pos c q
  | none => 0

theorem pos_pos {s : State} : 0 < pos c s := Nat.succ_pos _

include hwf

theorem parentOf_entry {s : State} (hs : s ∈ c.allStates) : (c.parentOf s, s) ∈ c.parented := by
  obtain ⟨o, ho⟩ := parented_entry hs
  rw [parentOf_eq (wf_nodup hwf) ho]; exact ho

theorem base_pos {s : State} (hs : s ∈ c.allStates) :
    (Compile.states c).getD (pos c s) CState.empty =
    { id := s.id, kind := Compile.kindOf s, parent := some (cpar c s),
      children := ((s.children.filter (fun c => !c.isHistory)).map (·.id)).filterMap
        (Compile.idx c.ids),
      histories := ((s.children.filter (fun c => c.isHistory)).map (·.id)).filterMap
        (Compile.idx c.ids),
      transitions := [], initial := none, onentry := s.onentry, onexit := s.onexit,
      data := s.data, invokes := s.invokes, donedata := s.donedata } := by
  have hnd := wf_nodup hwf
  have hlt := pos_lt hs
  have hk : pos c s - 1 < c.parented.length := by
    rw [length_parented]; have := pos_pos (c := c) (s := s); omega
  have he : pos c s = (pos c s - 1) + 1 := by have := pos_pos (c := c) (s := s); omega
  rw [he, states_getD_succ c _ hk]
  have h2 : (c.parented[pos c s - 1]).2 = s := by
    rw [parented_getElem_snd]; exact getElem_pos hnd hs
  have h1 : (c.parented[pos c s - 1]).1 = c.parentOf s := by
    have hm : ((c.parented[pos c s - 1]).1, (c.parented[pos c s - 1]).2) ∈ c.parented :=
      List.getElem_mem hk
    rw [h2] at hm
    exact (parentOf_eq hnd hm).symm
  rw [h1, h2]
  unfold cpar
  cases hq : c.parentOf s with
  | none => rfl
  | some q => simp only; rw [idx_eq hnd (parentOf_some hnd hs hq).1]; rfl

include hsp

theorem st_kind {s : State} (hs : s ∈ c.allStates) : (cc.st (pos c s)).kind = Compile.kindOf s := by
  rw [strip_kind (hsp.skel _), base_pos hwf hs]

theorem st_parent {s : State} (hs : s ∈ c.allStates) :
    (cc.st (pos c s)).parent = some (cpar c s) := by
  rw [strip_parent (hsp.skel _), base_pos hwf hs]

theorem st_children {s : State} (hs : s ∈ c.allStates) :
    (cc.st (pos c s)).children =
      ((s.children.filter (fun c => !c.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) := by
  rw [strip_children (hsp.skel _), base_pos hwf hs]

theorem st_histories {s : State} (hs : s ∈ c.allStates) :
    (cc.st (pos c s)).histories =
      ((s.children.filter (fun c => c.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) := by
  rw [strip_histories (hsp.skel _), base_pos hwf hs]

omit hwf in
theorem st_zero_kind : (cc.st 0).kind = .root := by
  rw [strip_kind (hsp.skel _), states_getD_zero]

omit hwf in
theorem st_zero_parent : (cc.st 0).parent = none := by
  rw [strip_parent (hsp.skel _), states_getD_zero]

omit hwf in
theorem st_zero_children : (cc.st 0).children =
    ((c.states.filter (fun s => !s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) := by
  rw [strip_children (hsp.skel _), states_getD_zero]

omit hwf in
theorem st_zero_histories : (cc.st 0).histories =
    ((c.states.filter (fun s => s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) := by
  rw [strip_histories (hsp.skel _), states_getD_zero]

omit hwf hsp in
theorem st_ge {j : Nat} (hj : cc.size ≤ j) : cc.st j = CState.empty := by
  unfold CChart.st; rw [Array.getD_eq_getD_getElem?]
  simp [CChart.size] at hj
  simp [Array.getElem?_eq_none (by omega : cc.states.size ≤ j)]

omit hwf hsp in
theorem tr_ge {t : Nat} (ht : cc.trans.size ≤ t) : cc.tr t = CTrans.empty := by
  unfold CChart.tr; rw [Array.getD_eq_getD_getElem?]
  simp [Array.getElem?_eq_none ht]

/-- Every proper state of the compiled chart is the position of an IR state. -/
theorem exists_pos {j : Nat} (h0 : 0 < j) (hj : j < cc.size) :
    ∃ s ∈ c.allStates, pos c s = j := by
  have hj' : j - 1 < c.allStates.length := by rw [hsp.size] at hj; omega
  refine ⟨c.allStates[j - 1], List.getElem_mem hj', ?_⟩
  rw [pos_getElem (wf_nodup hwf)]; omega

omit hwf hsp in
theorem pos_le {s : State} (hs : s ∈ c.allStates) : pos c s ≤ c.allStates.length := pos_lt hs

omit hwf in
theorem pos_lt_size {s : State} (hs : s ∈ c.allStates) : pos c s < cc.size := by
  rw [hsp.size]; have := pos_lt hs; omega

/-! ## Kinds -/

omit hwf hsp in
theorem kindOf_history' (s : State) :
    (∃ ht, Compile.kindOf s = .history ht) ↔ s.isHistory = true := by
  rcases s with ⟨_, _, el, _, _, cs, _⟩
  cases el <;> simp only [Compile.kindOf, State.kind, State.isHistory, State.element,
    State.children] <;> by_cases h : cs = [] <;> simp [h]

theorem isHistory_pos {s : State} (hs : s ∈ c.allStates) : cc.isHistory (pos c s) = s.isHistory := by
  unfold CChart.isHistory
  rw [st_kind hwf hsp hs]
  have := kindOf_history' s
  cases h : Compile.kindOf s <;> cases h' : s.isHistory <;> simp_all

omit hwf hsp in
theorem kindOf_eq_parallel (s : State) : Compile.kindOf s = .parallel ↔ s.kind = .parallel := by
  unfold Compile.kindOf; split <;> simp_all

omit hwf hsp in
theorem kindOf_eq_compound (s : State) : Compile.kindOf s = .compound ↔ s.kind = .compound := by
  unfold Compile.kindOf; split <;> simp_all

omit hwf hsp in
theorem kindOf_eq_atomic (s : State) : Compile.kindOf s = .atomic ↔ s.kind = .atomic := by
  unfold Compile.kindOf; split <;> simp_all

omit hwf hsp in
theorem kindOf_eq_final (s : State) : Compile.kindOf s = .final ↔ s.kind = .final := by
  unfold Compile.kindOf; split <;> simp_all

omit hwf hsp in
theorem kindOf_ne_root (s : State) : Compile.kindOf s ≠ .root := by
  unfold Compile.kindOf; split <;> simp

omit hwf hsp in
theorem kind_beq (a b : Kind) : (a == b) = true ↔ a = b := by
  cases a <;> cases b <;> (try (rename_i x y; cases x <;> cases y)) <;> (try (rename_i x; cases x)) <;>
    decide

theorem isParallel_pos {s : State} (hs : s ∈ c.allStates) :
    cc.isParallel (pos c s) = true ↔ s.kind = .parallel := by
  unfold CChart.isParallel; rw [st_kind hwf hsp hs, ← kindOf_eq_parallel, kind_beq]

theorem isCompound_pos {s : State} (hs : s ∈ c.allStates) :
    cc.isCompound (pos c s) = true ↔ s.kind = .compound := by
  unfold CChart.isCompound; rw [st_kind hwf hsp hs, ← kindOf_eq_compound, kind_beq]

/-! ## Parents and descent -/

omit hsp in
theorem cpar_lt {s : State} (hs : s ∈ c.allStates) : cpar c s < pos c s := by
  unfold cpar; split
  · rename_i q hq; exact pos_parent_lt (wf_nodup hwf) hs hq
  · exact pos_pos

theorem parent_pos {s : State} (hs : s ∈ c.allStates) :
    (cc.st (pos c s)).parent = some (cpar c s) ∧ cpar c s < pos c s :=
  ⟨st_parent hwf hsp hs, cpar_lt hwf hs⟩

theorem isDesc_pos_zero : ∀ (N : Nat) (s : State), s ∈ c.allStates → pos c s = N →
    cc.isDesc (pos c s) 0 = true := by
  intro N
  induction N using Nat.strongRecOn with
  | ind N ih =>
    intro s hs hN
    obtain ⟨hp, hlt⟩ := parent_pos hwf hsp hs
    refine cc.isDesc_of_parent_trans hp hlt ?_
    unfold cpar at *
    split
    · rename_i q hq
      have hq' := (parentOf_some (wf_nodup hwf) hs hq).1
      exact Or.inr (ih (pos c q) (hN ▸ pos_parent_lt (wf_nodup hwf) hs hq) q hq' rfl)
    · exact Or.inl rfl

theorem isDesc_zero' {s : State} (hs : s ∈ c.allStates) : cc.isDesc (pos c s) 0 = true :=
  isDesc_pos_zero hwf hsp _ s hs rfl

theorem isDesc_of_child {q s : State} (hq : q ∈ c.allStates) (hc : s ∈ q.children) :
    cc.isDesc (pos c s) (pos c q) = true := by
  have hnd := wf_nodup hwf
  have hs := mem_of_child c hq hc
  obtain ⟨hp, hlt⟩ := parent_pos hwf hsp hs
  have : cpar c s = pos c q := by unfold cpar; rw [parentOf_child hnd hq hc]
  rw [this] at hp hlt
  exact cc.isDesc_of_parent hp hlt

set_option linter.unusedSectionVars false in
mutual
theorem desc_isDesc : ∀ (q : State), q ∈ c.allStates → ∀ s ∈ State.flattenList q.children,
    cc.isDesc (pos c s) (pos c q) = true
  | .mk a b el d f cs g h i j k l m, hq, s, hsd => by
    have hc : (State.mk a b el d f cs g h i j k l m).children = cs := rfl
    exact descL_isDesc _ hq cs (fun x hx => hc ▸ hx) s (hc ▸ hsd)
theorem descL_isDesc : ∀ (q : State), q ∈ c.allStates → ∀ (cs : List State),
    (∀ x ∈ cs, x ∈ q.children) → ∀ s ∈ State.flattenList cs, cc.isDesc (pos c s) (pos c q) = true
  | _, _, [], _, s, h => by simp [State.flattenList] at h
  | q, hq, d :: ds, hsub, s, h => by
    rw [State.flattenList, List.mem_append] at h
    have hd : d ∈ q.children := hsub d (by simp)
    rcases h with h | h
    · rw [flatten_eq, List.mem_cons] at h
      rcases h with rfl | h
      · exact isDesc_of_child hwf hsp hq hd
      · exact cc.isDesc_trans (desc_isDesc d (mem_of_child c hq hd) s h)
          (isDesc_of_child hwf hsp hq hd)
    · exact descL_isDesc q hq ds (fun x hx => hsub x (by simp [hx])) s h
end

theorem isDesc_desc : ∀ (N : Nat) (s q : State), s ∈ c.allStates → q ∈ c.allStates →
    pos c s = N → cc.isDesc (pos c s) (pos c q) = true → s ∈ State.flattenList q.children := by
  have hnd := wf_nodup hwf
  intro N
  induction N using Nat.strongRecOn with
  | ind N ih =>
    intro s q hs hq hN hd
    obtain ⟨p, hp, _, h⟩ := (cc.isDesc_iff_parent _ _).1 hd
    rw [st_parent hwf hsp hs] at hp; cases hp
    unfold cpar at h
    split at h
    · rename_i o ho
      obtain ⟨ho', hso⟩ := parentOf_some hnd hs ho
      rcases h with h | h
      · rw [pos_inj hnd ho' hq h] at hso
        exact (children_sub _).subset hso
      · have := ih (pos c o) (hN ▸ pos_parent_lt hnd hs ho) o q ho' hq rfl h
        exact (flattenList_sub_children _ o this).subset ((children_sub _).subset hso)
    · rcases h with h | h
      · exact absurd h.symm (Nat.ne_of_gt (pos_pos))
      · exact absurd (cc.isDesc_lt h) (Nat.not_lt_zero _)

theorem isDesc_pos_iff {s q : State} (hs : s ∈ c.allStates) (hq : q ∈ c.allStates) :
    cc.isDesc (pos c s) (pos c q) = true ↔ s ∈ State.flattenList q.children :=
  ⟨isDesc_desc hwf hsp _ s q hs hq rfl, desc_isDesc hwf hsp q hq s⟩

theorem mem_descIds_iff {s q : State} (hs : s ∈ c.allStates) (hq : q ∈ c.allStates) :
    s.id ∈ q.descIds ↔ cc.isDesc (pos c s) (pos c q) = true := by
  rw [isDesc_pos_iff hwf hsp hs hq]
  unfold State.descIds
  constructor
  · intro h
    obtain ⟨t, ht, he⟩ := List.mem_map.1 h
    rwa [state_inj (wf_nodup hwf) ((desc_sub c hq).subset ht) hs he] at ht
  · intro h; exact List.mem_map_of_mem h

/-- Compiled ancestors are the IR ancestors, then the root. -/
theorem ancestors_pos : ∀ (N : Nat) (s : State), s ∈ c.allStates → pos c s = N →
    ∀ f, pos c s ≤ f → cc.ancestors (pos c s) = (c.ancestorsF f s).map (pos c) ++ [0] := by
  have hnd := wf_nodup hwf
  intro N
  induction N using Nat.strongRecOn with
  | ind N ih =>
    intro s hs hN f hf
    obtain ⟨hp, hlt⟩ := parent_pos hwf hsp hs
    rw [cc.ancestors_eq_of_parent hp hlt]
    obtain ⟨f', rfl⟩ : ∃ f', f = f' + 1 := ⟨f - 1, by have := pos_pos (c := c) (s := s); omega⟩
    simp only [Chart.ancestorsF]
    unfold cpar
    cases hq : c.parentOf s with
    | none =>
      simp only
      rw [cc.ancestors_eq_nil_of_none (st_zero_parent hsp)]
      simp
    | some q =>
      simp only
      have hq' := (parentOf_some hnd hs hq).1
      have hlt' := pos_parent_lt hnd hs hq
      rw [ih (pos c q) (hN ▸ hlt') q hq' rfl f' (by omega)]
      simp

theorem ancestors_pos' {s : State} (hs : s ∈ c.allStates) :
    cc.ancestors (pos c s) = (c.ancestors s).map (pos c) ++ [0] :=
  ancestors_pos hwf hsp _ s hs rfl _ (pos_lt hs)

end

end Vertep.CompileProof
