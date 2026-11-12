import VertepLean.Rdf.Equiv.Nodup
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Common prefixes of paths

`lcp l m`: the length of the longest common prefix of two lists. On the paths of a chart with
unique state identifiers (`WP`), two paths agree at a position exactly when the position is
below their common prefix length (`agree_iff`), because a path is determined by its last state.
-/

namespace Vertep.Rdf.Equiv

/-- The length of the longest common prefix. -/
def lcp : List String → List String → Nat
  | a :: l, b :: m => if a = b then lcp l m + 1 else 0
  | _, _ => 0

theorem lcp_le_left : ∀ (l m : List String), lcp l m ≤ l.length
  | [], _ => by simp [lcp]
  | _ :: _, [] => by simp [lcp]
  | a :: l, b :: m => by
    simp only [lcp]; split
    · have := lcp_le_left l m; simp; omega
    · simp

theorem lcp_le_right : ∀ (l m : List String), lcp l m ≤ m.length
  | [], _ => by simp [lcp]
  | _ :: _, [] => by simp [lcp]
  | a :: l, b :: m => by
    simp only [lcp]; split
    · have := lcp_le_right l m; simp; omega
    · simp

theorem lt_lcp : ∀ (l m : List String) (i : Nat), i < lcp l m ↔
    i < l.length ∧ ∀ j, j ≤ i → l[j]? = m[j]?
  | [], m, i => by simp [lcp]
  | a :: l, [], i => by
    simp only [lcp, Nat.not_lt_zero, List.length_cons, false_iff, not_and]
    intro _ h; have := h 0 (Nat.zero_le _); simp at this
  | a :: l, b :: m, i => by
    simp only [lcp]
    split
    · rename_i hab; subst hab
      rcases i with _ | i
      · simp
      · rw [show i + 1 < lcp l m + 1 ↔ i < lcp l m by omega, lt_lcp l m i]
        simp only [List.length_cons]
        constructor
        · rintro ⟨h1, h2⟩
          refine ⟨by omega, fun j hj => ?_⟩
          rcases j with _ | j
          · rfl
          · simpa using h2 j (by omega)
        · rintro ⟨h1, h2⟩
          exact ⟨by omega, fun j hj => by simpa using h2 (j + 1) (by omega)⟩
    · rename_i hab
      simp only [Nat.not_lt_zero, false_iff, not_and]
      intro _ h; have := h 0 (Nat.zero_le _); simp at this; exact hab this

theorem lcp_stop : ∀ (l m : List String), lcp l m < l.length → lcp l m < m.length →
    l[lcp l m]? ≠ m[lcp l m]?
  | [], _, h, _ => by simp at h
  | _ :: _, [], _, h => by simp [lcp] at h
  | a :: l, b :: m, h1, h2 => by
    simp only [lcp] at h1 h2 ⊢
    split
    · rename_i hab
      simp only [hab, ↓reduceIte] at h1 h2
      simp only [List.getElem?_cons_succ]
      exact lcp_stop l m (by simp at h1; omega) (by simp at h2; omega)
    · rename_i hab
      simpa using hab

/-- The last element of a reversed list satisfying a predicate that holds exactly below `n`. -/
theorem find_rev {α : Type} (l : List α) (q : α → Bool) (n : Nat) (hn : n ≤ l.length)
    (hq : ∀ m (hm : m < l.length), q l[m] = true ↔ m < n) :
    (n = 0 → l.reverse.find? q = none) ∧ ∀ (h : 0 < n), l.reverse.find? q = some (l[n - 1]'(by omega)) := by
  constructor
  · rintro rfl
    rw [List.find?_eq_none]
    intro x hx hqx
    obtain ⟨m, hm, rfl⟩ := List.getElem_of_mem (List.mem_reverse.1 hx)
    exact absurd ((hq m hm).1 hqx) (Nat.not_lt_zero _)
  · intro h0
    rw [List.find?_eq_some_iff_getElem]
    refine ⟨(hq _ _).2 (by omega), l.length - n, by simp; omega, ?_, ?_⟩
    · rw [List.getElem_reverse]; congr 1; omega
    · intro j hj
      rw [List.getElem_reverse]
      have h2 : ¬ q l[l.length - 1 - j] = true := fun h3 => by
        have := (hq (l.length - 1 - j) (by omega)).1 h3; omega
      simpa using h2

theorem kind_parallel (s : State) : s.kind = .parallel ↔ s.element = .parallel := by
  rcases s with ⟨_, _, el⟩
  cases el <;> simp only [State.kind, State.element] <;> (try split) <;> simp

theorem lcp_comm : ∀ (l m : List String), lcp l m = lcp m l
  | [], [] => rfl
  | [], _ :: _ => rfl
  | _ :: _, [] => rfl
  | a :: l, b :: m => by
    simp only [lcp]
    by_cases h : a = b
    · subst h; simp [lcp_comm l m]
    · simp [h, Ne.symm h]

section
variable {c : Chart} {P : State} {p : List State} (he : (P, p) ∈ WP c)
include he

theorem wp_len_pos : 0 < p.length := by
  have := wp_ne_nil he
  exact List.length_pos_iff.2 this

theorem wp_last_get (hnd : c.ids.Nodup) (h : p.length - 1 < p.length) : p[p.length - 1] = P := by
  have := wp_last hnd he
  rw [List.getLast_eq_getElem] at this
  exact this

end

section
variable {c : Chart} (hnd : c.ids.Nodup) {P P' : State} {p p' : List State}
  (he : (P, p) ∈ WP c) (he' : (P', p') ∈ WP c)
include hnd he he'

/-- **Two paths agree at a position exactly below their common prefix length.** -/
theorem agree_iff {i : Nat} (hi : i < p.length) (hi' : i < p'.length) :
    p[i].id = p'[i].id ↔ i < lcp (p.map State.id) (p'.map State.id) := by
  rw [lt_lcp]
  constructor
  · intro h
    obtain ⟨_, ht⟩ := wp_prefix hnd he he' hi hi' h
    refine ⟨by simpa using hi, fun j hj => ?_⟩
    have h1 : (p.take (i + 1))[j]? = (p'.take (i + 1))[j]? := by rw [ht]
    simp only [List.getElem?_take, show j < i + 1 by omega, ↓reduceIte] at h1
    simp [h1]
  · rintro ⟨_, h⟩
    have := h i (Nat.le_refl _)
    simp only [List.getElem?_map, List.getElem?_eq_getElem hi, List.getElem?_eq_getElem hi',
      Option.map_some, Option.some.injEq] at this
    exact this

theorem lcp_lt (hl : lcp (p.map State.id) (p'.map State.id) < p.length)
    (hl' : lcp (p.map State.id) (p'.map State.id) < p'.length) :
    p[lcp (p.map State.id) (p'.map State.id)].id ≠ p'[lcp (p.map State.id) (p'.map State.id)].id := by
  intro h
  have := (agree_iff hnd he he' hl hl').1 h
  omega

theorem wpp_prefix {i j : Nat} (hi : i < p.length) (hj : j < p'.length) (h : p[i].id = p'[j].id) :
    i = j ∧ p.take (i + 1) = p'.take (j + 1) := wp_prefix hnd he he' hi hj h

theorem wpp_desc : P'.id ∈ P.descIds ↔ ∃ k, ∃ hk : k + 1 < p'.length, p'[k] = P :=
  wp_desc hnd he' (wp_mem he)

theorem wpp_mem {i : Nat} (hi : i < p.length) : p[i] ∈ c.allStates := wp_elems he _ (List.getElem_mem _)

theorem wpp_mem' {i : Nat} (hi : i < p'.length) : p'[i] ∈ c.allStates := wp_elems he' _ (List.getElem_mem _)

/-- `P'` is below `P` exactly when `P`'s path is a proper prefix of `P'`'s. -/
theorem desc_iff : P'.id ∈ P.descIds ↔
    lcp (p.map State.id) (p'.map State.id) = p.length ∧ p.length < p'.length := by
  have hpn := wp_len_pos he
  have hpn' := wp_len_pos he'
  have hle := lcp_le_left (p.map State.id) (p'.map State.id)
  have hle' := lcp_le_right (p.map State.id) (p'.map State.id)
  simp only [List.length_map] at hle hle'
  rw [wpp_desc hnd he he']
  constructor
  · rintro ⟨k, hk, hkP⟩
    have h1 : p[p.length - 1].id = p'[k].id := by rw [hkP, wp_last_get he hnd (by omega)]
    obtain ⟨hk1, _⟩ := wpp_prefix hnd he he' (by omega) (by omega) h1
    have := (agree_iff hnd he he' (i := p.length - 1) (by omega) (by omega)).1 (by
      rw [h1]; congr 2; omega)
    omega
  · rintro ⟨h1, h2⟩
    refine ⟨p.length - 1, by omega, ?_⟩
    have := (agree_iff hnd he he' (i := p.length - 1) (by omega) (by omega)).2 (by omega)
    have h3 := wp_last_get he hnd (by omega)
    have h4 := CompileProof.state_inj hnd (wpp_mem hnd he he' (by omega)) (wpp_mem' hnd he he' (by omega)) this
    rw [← h4, h3]

theorem same_iff : P.id = P'.id ↔
    lcp (p.map State.id) (p'.map State.id) = p.length ∧ p.length = p'.length := by
  have hpn := wp_len_pos he
  have hpn' := wp_len_pos he'
  have hle := lcp_le_left (p.map State.id) (p'.map State.id)
  have hle' := lcp_le_right (p.map State.id) (p'.map State.id)
  simp only [List.length_map] at hle hle'
  have hP := wp_last_get he hnd (by omega)
  have hP' := wp_last_get he' hnd (by omega)
  constructor
  · intro h
    have h1 : p[p.length - 1].id = p'[p'.length - 1].id := by rw [hP, hP', h]
    obtain ⟨hk1, _⟩ := wpp_prefix hnd he he' (by omega) (by omega) h1
    have := (agree_iff hnd he he' (i := p.length - 1) (by omega) (by omega)).1 (by
      rw [h1]; congr 2; omega)
    omega
  · rintro ⟨h1, h2⟩
    have := (agree_iff hnd he he' (i := p.length - 1) (by omega) (by omega)).2 (by omega)
    rw [← hP, ← hP', this]; congr 2; omega

theorem lca_iff (hl : lcp (p.map State.id) (p'.map State.id) < p.length)
    (hl' : lcp (p.map State.id) (p'.map State.id) < p'.length) :
    LcaParallel c P P' ↔ ∃ h0 : 0 < lcp (p.map State.id) (p'.map State.id),
      (p[lcp (p.map State.id) (p'.map State.id) - 1]'(by omega)).element = .parallel := by
  have hanc := wp_ancestors hnd he
  simp only at hanc
  unfold LcaParallel
  rw [hanc]
  have hq : ∀ m (hm : m < p.dropLast.length),
      decide (P'.id ∈ (p.dropLast[m]).descIds) = true ↔ m < lcp (p.map State.id) (p'.map State.id) := by
    intro m hm
    simp only [List.length_dropLast] at hm
    simp only [decide_eq_true_eq, List.getElem_dropLast]
    have hW := wpp_mem hnd he he' (i := m) (by omega)
    have hd := wp_desc hnd he' hW
    simp only at hd
    rw [hd]
    constructor
    · rintro ⟨k, hk, hkm⟩
      have h1 : p[m].id = p'[k].id := by rw [hkm]
      obtain ⟨rfl, _⟩ := wpp_prefix hnd he he' (by omega) (by omega) h1
      exact (agree_iff hnd he he' (by omega) (by omega)).1 h1
    · intro hm'
      refine ⟨m, by omega, ?_⟩
      exact CompileProof.state_inj hnd (wpp_mem' hnd he he' (by omega)) hW
        ((agree_iff hnd he he' (by omega) (by omega)).2 hm').symm
  have hfr := find_rev p.dropLast (fun W => decide (P'.id ∈ W.descIds))
    (lcp (p.map State.id) (p'.map State.id)) (by simp; omega) hq
  by_cases h0 : lcp (p.map State.id) (p'.map State.id) = 0
  · rw [hfr.1 h0]; simp [h0]
  · rw [hfr.2 (by omega)]
    simp only [List.getElem_dropLast, kind_parallel]
    exact ⟨fun h => ⟨by omega, h⟩, fun ⟨_, h⟩ => h⟩

/-- **Two targets may be entered together exactly when their paths part below a parallel
state.** -/
theorem pairOk_iff : PairOk c P P' ↔
    ∃ (h0 : 0 < lcp (p.map State.id) (p'.map State.id)) (h1 : lcp (p.map State.id) (p'.map State.id) < p.length),
      lcp (p.map State.id) (p'.map State.id) < p'.length ∧
      (p[lcp (p.map State.id) (p'.map State.id) - 1]'(by omega)).element = .parallel := by
  have hle := lcp_le_left (p.map State.id) (p'.map State.id)
  have hle' := lcp_le_right (p.map State.id) (p'.map State.id)
  simp only [List.length_map] at hle hle'
  unfold PairOk
  simp only [ne_eq]
  rw [same_iff hnd he he', desc_iff hnd he he', desc_iff hnd he' he, lcp_comm (p'.map State.id)]
  constructor
  · rintro ⟨h1, h2, h3, h4⟩
    have hl : lcp (p.map State.id) (p'.map State.id) < p.length := by omega
    have hl' : lcp (p.map State.id) (p'.map State.id) < p'.length := by omega
    obtain ⟨h0, h5⟩ := (lca_iff hnd he he' hl hl').1 h4
    exact ⟨h0, hl, hl', h5⟩
  · rintro ⟨h0, hl, hl', h5⟩
    exact ⟨by omega, by omega, by omega, (lca_iff hnd he he' hl hl').2 ⟨h0, h5⟩⟩

end

end Vertep.Rdf.Equiv
