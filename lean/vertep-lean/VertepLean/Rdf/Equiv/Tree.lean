import VertepLean.Rdf.Equiv.NodeShapes
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# Tree facts for charts with unique state identifiers

Parent–child pairs (`State.parented`), subtrees (`flatten`) and `descIds`: a state's identifier is
among `W.descIds` exactly when its parent is `W` or a descendant of `W` (`mem_descIds_iff`).
-/

namespace Vertep.Rdf.Equiv

open Vertep.Rdf.RT

mutual
theorem flatten_children_sub : ∀ (s : State), ∀ X ∈ s.flatten, ∀ Y ∈ X.children, Y ∈ s.flatten
  | .mk id ig el ini ie cs trs en ex da inv dd xs, X, hX, Y, hY => by
    rw [flatten_eq] at hX ⊢
    rcases List.mem_cons.1 hX with rfl | hX
    · exact List.mem_cons_of_mem _ (children_sub cs |>.subset hY)
    · exact List.mem_cons_of_mem _ (flattenList_children_sub cs X hX Y hY)
theorem flattenList_children_sub : ∀ (cs : List State), ∀ X ∈ State.flattenList cs, ∀ Y ∈ X.children,
    Y ∈ State.flattenList cs
  | [], X, hX, _, _ => by simp [State.flattenList] at hX
  | c :: cs, X, hX, Y, hY => by
    rw [flattenList_cons, List.mem_append] at hX ⊢
    rcases hX with hX | hX
    · exact .inl (flatten_children_sub c X hX Y hY)
    · exact .inr (flattenList_children_sub cs X hX Y hY)
end

mutual
/-- Every parent–child pair below `pp`: the root pair, or a child of a state of the subtree. -/
theorem parented_shape : ∀ (pp : Option State) (s : State), ∀ q ∈ State.parented pp s,
    (q.1 = pp ∧ q.2 = s) ∨ ∃ P ∈ s.flatten, q.1 = some P ∧ q.2 ∈ P.children
  | pp, .mk id ig el ini ie cs trs en ex da inv dd xs, q, hq => by
    rw [parented_eq] at hq
    rcases List.mem_cons.1 hq with rfl | hq
    · exact .inl ⟨rfl, rfl⟩
    · right
      rcases parentedList_shape _ cs q hq with ⟨h1, h2⟩ | ⟨P, hP, h1, h2⟩
      · exact ⟨_, by rw [flatten_eq]; exact List.mem_cons_self, h1, h2⟩
      · exact ⟨P, by rw [flatten_eq]; exact List.mem_cons_of_mem _ hP, h1, h2⟩
theorem parentedList_shape : ∀ (pp : Option State) (cs : List State), ∀ q ∈ State.parentedList pp cs,
    (q.1 = pp ∧ q.2 ∈ cs) ∨ ∃ P ∈ State.flattenList cs, q.1 = some P ∧ q.2 ∈ P.children
  | _, [], q, hq => by simp [State.parentedList] at hq
  | pp, c :: cs, q, hq => by
    rw [State.parentedList, List.mem_append] at hq
    rcases hq with hq | hq
    · rcases parented_shape pp c q hq with ⟨h1, h2⟩ | ⟨P, hP, h1, h2⟩
      · exact .inl ⟨h1, by simp [h2]⟩
      · exact .inr ⟨P, by rw [flattenList_cons]; exact List.mem_append_left _ hP, h1, h2⟩
    · rcases parentedList_shape pp cs q hq with ⟨h1, h2⟩ | ⟨P, hP, h1, h2⟩
      · exact .inl ⟨h1, List.mem_cons_of_mem _ h2⟩
      · exact .inr ⟨P, by rw [flattenList_cons]; exact List.mem_append_right _ hP, h1, h2⟩
end

mutual
/-- Every state of a subtree below its root has a parent pair in the subtree's pairs. -/
theorem flattenList_parented : ∀ (pp : Option State) (cs : List State), ∀ Z ∈ State.flattenList cs,
    ∃ q ∈ State.parentedList pp cs, q.2 = Z
  | _, [], Z, hZ => by simp [State.flattenList] at hZ
  | pp, c :: cs, Z, hZ => by
    rw [flattenList_cons, List.mem_append] at hZ
    rw [State.parentedList]
    rcases hZ with hZ | hZ
    · obtain ⟨q, hq, rfl⟩ := flatten_parented pp c Z hZ
      exact ⟨q, List.mem_append_left _ hq, rfl⟩
    · obtain ⟨q, hq, rfl⟩ := flattenList_parented pp cs Z hZ
      exact ⟨q, List.mem_append_right _ hq, rfl⟩
theorem flatten_parented : ∀ (pp : Option State) (s : State), ∀ Z ∈ s.flatten,
    ∃ q ∈ State.parented pp s, q.2 = Z
  | pp, .mk id ig el ini ie cs trs en ex da inv dd xs, Z, hZ => by
    rw [flatten_eq] at hZ
    rw [parented_eq]
    rcases List.mem_cons.1 hZ with rfl | hZ
    · exact ⟨_, List.mem_cons_self, rfl⟩
    · obtain ⟨q, hq, rfl⟩ := flattenList_parented _ cs Z hZ
      exact ⟨q, List.mem_cons_of_mem _ hq, rfl⟩
end

section
variable {c : Chart} (hnd : c.ids.Nodup)
include hnd

theorem state_uniq {X Z : State} (hX : X ∈ c.allStates) (hZ : Z ∈ c.allStates) (h : X.id = Z.id) : X = Z := by
  have hnd' : (c.allStates.map State.id).Nodup := hnd
  clear hnd
  generalize c.allStates = L at hX hZ hnd'
  induction L with
  | nil => simp at hX
  | cons a l ih =>
    simp only [List.map_cons, List.nodup_cons, List.mem_map] at hnd'
    simp only [List.mem_cons] at hX hZ
    rcases hX with rfl | hX <;> rcases hZ with rfl | hZ
    · rfl
    · exact absurd ⟨Z, hZ, h.symm⟩ hnd'.1
    · exact absurd ⟨X, hX, h⟩ hnd'.1
    · exact ih hX hZ hnd'.2

theorem parented_uniq {q q' : Option State × State} (hq : q ∈ c.parented) (hq' : q' ∈ c.parented)
    (h : q.2.id = q'.2.id) : q = q' :=
  RecH.cont.nodup_fst_eq' (parented_ids_nodup c hnd) hq hq' h

end

theorem parented_allStates {c : Chart} {q : Option State × State} (hq : q ∈ c.parented) :
    q.2 ∈ c.allStates := mem_parented_allStates hq

theorem parent_allStates {c : Chart} {q : Option State × State} (hq : q ∈ c.parented) {P : State}
    (hP : q.1 = some P) : P ∈ c.allStates := by
  rcases parentedList_shape none c.states q hq with ⟨h1, _⟩ | ⟨P', hP', h1, _⟩
  · rw [hP] at h1; cases h1
  · rw [hP] at h1; cases h1; exact hP'

mutual
theorem parented_closed : ∀ (pp : Option State) (s : State), ∀ q ∈ State.parented pp s,
    ∀ r ∈ State.parentedList (some q.2) q.2.children, r ∈ State.parented pp s
  | pp, .mk id ig el ini ie cs trs en ex da inv dd xs, q, hq, r, hr => by
    rw [parented_eq] at hq ⊢
    rcases List.mem_cons.1 hq with rfl | hq
    · exact List.mem_cons_of_mem _ hr
    · exact List.mem_cons_of_mem _ (parentedList_closed _ cs q hq r hr)
theorem parentedList_closed : ∀ (pp : Option State) (cs : List State), ∀ q ∈ State.parentedList pp cs,
    ∀ r ∈ State.parentedList (some q.2) q.2.children, r ∈ State.parentedList pp cs
  | _, [], q, hq, _, _ => by simp [State.parentedList] at hq
  | pp, c :: cs, q, hq, r, hr => by
    rw [State.parentedList, List.mem_append] at hq ⊢
    rcases hq with hq | hq
    · exact .inl (parented_closed pp c q hq r hr)
    · exact .inr (parentedList_closed pp cs q hq r hr)
end

theorem allStates_parented {c : Chart} {W : State} (hW : W ∈ c.allStates) :
    ∃ q ∈ c.parented, q.2 = W := flattenList_parented none c.states W hW

/-- **A state's identifier is below `W` exactly when its parent is `W` or below `W`.** -/
theorem mem_descIds_iff {c : Chart} (hnd : c.ids.Nodup) {q : Option State × State} (hq : q ∈ c.parented)
    {W : State} (hW : W ∈ c.allStates) :
    q.2.id ∈ W.descIds ↔ ∃ P, q.1 = some P ∧ (P.id = W.id ∨ P.id ∈ W.descIds) := by
  obtain ⟨qW, hqW, rfl⟩ := allStates_parented hW
  have hsub : ∀ r ∈ State.parentedList (some qW.2) qW.2.children, r ∈ c.parented :=
    fun r hr => parentedList_closed none c.states qW hqW r hr
  simp only [State.descIds, List.mem_map]
  constructor
  · rintro ⟨Z, hZ, hZid⟩
    obtain ⟨q', hq', rfl⟩ := flattenList_parented (some qW.2) qW.2.children Z hZ
    have := parented_uniq hnd (hsub q' hq') hq hZid
    subst this
    rcases parentedList_shape _ _ q' hq' with ⟨h1, _⟩ | ⟨P, hP, h1, _⟩
    · exact ⟨qW.2, h1, .inl rfl⟩
    · exact ⟨P, h1, .inr ⟨P, hP, rfl⟩⟩
  · rintro ⟨P, hP, hPW | ⟨Z, hZ, hZid⟩⟩
    · have hPa := parent_allStates hq hP
      have hPq := state_uniq hnd hPa (parented_allStates hqW) hPW
      have hX : q.2 ∈ P.children := by
        rcases parentedList_shape none c.states q hq with ⟨h1, _⟩ | ⟨P', _, h1, h2⟩
        · rw [hP] at h1; cases h1
        · rw [hP] at h1; cases h1; exact h2
      rw [hPq] at hX
      exact ⟨q.2, children_sub _ |>.subset hX, rfl⟩
    · have hZa : Z ∈ c.allStates := by
        obtain ⟨q'', hq'', rfl⟩ := flattenList_parented (some qW.2) qW.2.children Z hZ
        exact parented_allStates (hsub q'' hq'')
      have hPa := parent_allStates hq hP
      have hZP := state_uniq hnd hZa hPa hZid
      have hX : q.2 ∈ Z.children := by
        rcases parentedList_shape none c.states q hq with ⟨h1, _⟩ | ⟨P', _, h1, h2⟩
        · rw [hP] at h1; cases h1
        · rw [hP] at h1; cases h1; rw [hZP]; exact h2
      exact ⟨q.2, flattenList_children_sub _ Z hZ _ hX, rfl⟩

end Vertep.Rdf.Equiv
