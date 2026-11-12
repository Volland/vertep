import VertepLean.Config.Basic

/-!
# Tree lemmas for compiled charts

`ancestors`, `isDesc` and their interaction with parents, on any compiled chart and under
`CWF`. Shared by every part of the configuration proof.
-/

namespace Vertep.CChart

variable (cc : CChart)

theorem ancestors_eq_of_parent {i p : Nat} (h : (cc.st i).parent = some p) (hp : p < i) :
    cc.ancestors i = p :: cc.ancestors p := by
  rw [CChart.ancestors]; simp [h, hp]

theorem ancestors_eq_nil_of_none {i : Nat} (h : (cc.st i).parent = none) :
    cc.ancestors i = [] := by
  rw [CChart.ancestors]; simp [h]

theorem ancestors_eq_nil_of_ge {i p : Nat} (h : (cc.st i).parent = some p) (hp : ¬p < i) :
    cc.ancestors i = [] := by
  rw [CChart.ancestors]; simp [h, hp]

theorem mem_ancestors_lt : ∀ (i a : Nat), a ∈ cc.ancestors i → a < i := by
  intro i
  induction i using Nat.strongRecOn with
  | ind i ih =>
    intro a ha
    rw [CChart.ancestors] at ha
    split at ha
    · rename_i p hp
      split at ha
      · rename_i hlt
        rcases List.mem_cons.1 ha with rfl | h
        · exact hlt
        · exact Nat.lt_trans (ih p hlt a h) hlt
      · simp at ha
    · simp at ha

theorem isDesc_iff_mem (a b : Nat) : cc.isDesc a b = true ↔ b ∈ cc.ancestors a := by
  simp [CChart.isDesc]

theorem isDesc_lt {a b : Nat} (h : cc.isDesc a b = true) : b < a :=
  cc.mem_ancestors_lt a b ((cc.isDesc_iff_mem a b).1 h)

theorem isDesc_irrefl (a : Nat) : cc.isDesc a a = false := by
  cases h : cc.isDesc a a
  · rfl
  · exact absurd (cc.isDesc_lt h) (Nat.lt_irrefl a)

/-- One step up: `a` is a proper descendant of `b` iff `b` is `a`'s parent or an ancestor of
it (with the parent below `a`). -/
theorem isDesc_iff_parent (a b : Nat) : cc.isDesc a b = true ↔
    ∃ p, (cc.st a).parent = some p ∧ p < a ∧ (p = b ∨ cc.isDesc p b = true) := by
  rw [isDesc_iff_mem]
  constructor
  · intro h
    rw [CChart.ancestors] at h
    split at h
    · rename_i p hp
      split at h
      · rename_i hlt
        refine ⟨p, hp, hlt, ?_⟩
        rcases List.mem_cons.1 h with rfl | h
        · exact Or.inl rfl
        · exact Or.inr ((cc.isDesc_iff_mem p b).2 h)
      · simp at h
    · simp at h
  · rintro ⟨p, hp, hlt, h⟩
    rw [cc.ancestors_eq_of_parent hp hlt]
    rcases h with rfl | h
    · exact List.mem_cons_self
    · exact List.mem_cons_of_mem _ ((cc.isDesc_iff_mem p b).1 h)

theorem isDesc_of_parent {a p : Nat} (hp : (cc.st a).parent = some p) (hlt : p < a) :
    cc.isDesc a p = true :=
  (cc.isDesc_iff_parent a p).2 ⟨p, hp, hlt, Or.inl rfl⟩

theorem isDesc_trans {a b c : Nat} (h₁ : cc.isDesc a b = true) (h₂ : cc.isDesc b c = true) :
    cc.isDesc a c = true := by
  induction a using Nat.strongRecOn with
  | ind a ih =>
    obtain ⟨p, hp, hlt, h⟩ := (cc.isDesc_iff_parent a b).1 h₁
    refine (cc.isDesc_iff_parent a c).2 ⟨p, hp, hlt, Or.inr ?_⟩
    rcases h with rfl | h
    · exact h₂
    · exact ih p hlt h

theorem isDesc_of_parent_trans {a p b : Nat} (hp : (cc.st a).parent = some p) (hlt : p < a)
    (h : p = b ∨ cc.isDesc p b = true) : cc.isDesc a b = true :=
  (cc.isDesc_iff_parent a b).2 ⟨p, hp, hlt, h⟩

/-- The ancestors of a state form a chain: two ancestors of `x` are equal or related. -/
theorem isDesc_chain {x a b : Nat} (ha : cc.isDesc x a = true) (hb : cc.isDesc x b = true) :
    a = b ∨ cc.isDesc a b = true ∨ cc.isDesc b a = true := by
  induction x using Nat.strongRecOn with
  | ind x ih =>
    obtain ⟨p, hp, hlt, h1⟩ := (cc.isDesc_iff_parent x a).1 ha
    obtain ⟨p', hp', _, h2⟩ := (cc.isDesc_iff_parent x b).1 hb
    rw [hp] at hp'; cases hp'
    rcases h1 with rfl | h1 <;> rcases h2 with rfl | h2
    · exact Or.inl rfl
    · exact Or.inr (Or.inl h2)
    · exact Or.inr (Or.inr h1)
    · exact ih p hlt h1 h2

/-- Proper descendants of `b` that are ancestors-or-self of `x`, with `x` below `b`, are related
to the parent chain: if `x` is a proper descendant of `b` and `a` is `x`'s parent, then `a = b`
or `a` is a proper descendant of `b`. -/
theorem parent_desc_or_eq {x b p : Nat} (h : cc.isDesc x b = true)
    (hp : (cc.st x).parent = some p) : p = b ∨ cc.isDesc p b = true := by
  obtain ⟨p', hp', _, h⟩ := (cc.isDesc_iff_parent x b).1 h
  rw [hp] at hp'; cases hp'; exact h

theorem isDesc_lt_size {a b : Nat} (h : cc.isDesc a b = true) : a < cc.size := by
  obtain ⟨p, hp, _, _⟩ := (cc.isDesc_iff_parent a b).1 h
  refine Classical.byContradiction fun hge => ?_
  have : cc.st a = CState.empty := by
    unfold CChart.st; rw [Array.getD_eq_getD_getElem?]
    simp [CChart.size] at hge
    simp [Array.getElem?_eq_none (by omega : cc.states.size ≤ a)]
  rw [this] at hp; simp [CState.empty] at hp

end Vertep.CChart

namespace Vertep.CWF

open CChart

variable {cc : CChart}

theorem isDesc_zero (hw : CWF cc) : ∀ a, 0 < a → a < cc.size → cc.isDesc a 0 = true := by
  intro a
  induction a using Nat.strongRecOn with
  | ind a ih =>
    intro ha hs
    obtain ⟨p, hp, hlt⟩ := hw.parent_lt a ha hs
    refine cc.isDesc_of_parent_trans hp hlt ?_
    rcases Nat.eq_zero_or_pos p with rfl | hpos
    · exact Or.inl rfl
    · exact Or.inr (ih p hlt hpos (Nat.lt_trans hlt hs))

theorem parent_lt' (hw : CWF cc) {i p : Nat} (hp : (cc.st i).parent = some p) : p < i := by
  by_cases h0 : i = 0
  · subst h0; rw [hw.root_parent] at hp; cases hp
  by_cases hs : i < cc.size
  · obtain ⟨p', hp', hlt⟩ := hw.parent_lt i (Nat.pos_of_ne_zero h0) hs
    rw [hp] at hp'; cases hp'; exact hlt
  · have : cc.st i = CState.empty := by
      unfold CChart.st; rw [Array.getD_eq_getD_getElem?]
      simp [CChart.size] at hs
      simp [Array.getElem?_eq_none (by omega : cc.states.size ≤ i)]
    rw [this] at hp; simp [CState.empty] at hp

theorem isDesc_parent (hw : CWF cc) {i p : Nat} (hp : (cc.st i).parent = some p) :
    cc.isDesc i p = true :=
  cc.isDesc_of_parent hp (hw.parent_lt' hp)

theorem isDesc_iff (hw : CWF cc) (a b : Nat) : cc.isDesc a b = true ↔
    ∃ p, (cc.st a).parent = some p ∧ (p = b ∨ cc.isDesc p b = true) := by
  rw [cc.isDesc_iff_parent]
  constructor
  · rintro ⟨p, hp, _, h⟩; exact ⟨p, hp, h⟩
  · rintro ⟨p, hp, h⟩; exact ⟨p, hp, hw.parent_lt' hp, h⟩

theorem child_isDesc (hw : CWF cc) {i c : Nat} (h : c ∈ (cc.st i).children) :
    cc.isDesc c i = true :=
  hw.isDesc_parent ((hw.children_iff i c).1 h).2.1

theorem history_isDesc (hw : CWF cc) {i h : Nat} (hh : h ∈ (cc.st i).histories) :
    cc.isDesc h i = true :=
  hw.isDesc_parent ((hw.histories_iff i h).1 hh).2.1

theorem eq_singleton_of_nodup {l : List Nat} {c : Nat} (hnd : l.Nodup) (hc : c ∈ l)
    (hu : ∀ b ∈ l, b = c) : l = [c] := by
  match l, hnd with
  | [], _ => simp at hc
  | [a], _ => rw [hu a (by simp)]
  | a :: b :: rest, hnd =>
    have ha := hu a (by simp)
    have hb := hu b (by simp)
    subst ha; subst hb
    simp at hnd

theorem filter_length_one_iff (hw : CWF cc) (i : Nat) (S : Nat → Prop) [DecidablePred S] :
    ((cc.st i).children.filter (fun x => decide (S x))).length = 1 ↔
      ExactlyOne (cc.st i).children S := by
  have hnd := hw.children_nodup i
  constructor
  · intro hl
    obtain ⟨c, hc⟩ := List.length_eq_one_iff.1 hl
    have hmem : c ∈ (cc.st i).children.filter (fun x => decide (S x)) := by rw [hc]; simp
    simp only [List.mem_filter, decide_eq_true_eq] at hmem
    refine ⟨⟨c, hmem.1, hmem.2⟩, fun c₁ h₁ c₂ h₂ s₁ s₂ => ?_⟩
    have m₁ : c₁ ∈ (cc.st i).children.filter (fun x => decide (S x)) := by simp [h₁, s₁]
    have m₂ : c₂ ∈ (cc.st i).children.filter (fun x => decide (S x)) := by simp [h₂, s₂]
    rw [hc] at m₁ m₂; simp at m₁ m₂; rw [m₁, m₂]
  · rintro ⟨⟨c, hc, hs⟩, hu⟩
    have : (cc.st i).children.filter (fun x => decide (S x)) = [c] := by
      apply eq_singleton_of_nodup (hnd.filter _) (by simp [hc, hs])
      intro b hb
      simp only [List.mem_filter, decide_eq_true_eq] at hb
      exact hu b hb.1 c hc hb.2 hs
    rw [this]; rfl

/-- A legal configuration is a legal fragment below the root, and conversely. -/
theorem legal_iff_below (hw : CWF cc) (config : List Nat) :
    Legal cc config ↔ cc.Below 0 config := by
  constructor
  · rintro ⟨h1, h2, h3, h4, h5⟩
    refine ⟨fun x hx => hw.isDesc_zero x (Nat.pos_of_ne_zero (h1 x hx).1) (h1 x hx).2.1,
      fun x hx => (h1 x hx).2.2, h2, fun x hx hc => (hw.filter_length_one_iff x _).1 (h4 x hx hc),
      h5, fun _ => (hw.filter_length_one_iff 0 _).1 h3, fun hp => ?_⟩
    unfold CChart.isParallel at hp; rw [hw.root_kind] at hp; exact absurd hp (by decide)
  · intro hb
    refine ⟨fun x hx => ⟨?_, cc.isDesc_lt_size (hb.desc x hx), hb.noHist x hx⟩, hb.parent,
      (hw.filter_length_one_iff 0 _).2 (hb.top (by unfold CChart.isCompoundOrRoot; rw [hw.root_kind]; decide)),
      fun x hx hc => (hw.filter_length_one_iff x _).2 (hb.compound x hx hc), hb.parallel⟩
    intro h0; subst h0
    have := cc.isDesc_lt (hb.desc 0 hx); omega

end Vertep.CWF
