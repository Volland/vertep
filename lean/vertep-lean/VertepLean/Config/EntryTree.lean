import VertepLean.Config.ExitBase

/-!
# Entry lemma: tree combinatorics

Facts about `Compatible` states used by the entry lemma: compatibility is symmetric and passes
to descendants, siblings below a parallel state are compatible (and siblings that are
compatible have a parallel parent), an ancestor of a state that is related to everything is
related to everything too, distinct values of a recorded history are compatible, and the
ancestors `addAnc` visits are exactly the states between a state and its stop.
-/

namespace Vertep.CChart

/-- Two states may be active together: equal, related, or compatible. -/
def Rel (cc : CChart) (a b : Nat) : Prop :=
  a = b ∨ cc.isDesc a b = true ∨ cc.isDesc b a = true ∨ cc.Compatible a b

end Vertep.CChart

namespace Vertep.Entry

open CChart

variable {cc : CChart}

theorem compat_symm {a b : Nat} (h : cc.Compatible a b) : cc.Compatible b a := by
  obtain ⟨h1, h2, h3, q, hq, ha, hb, hmin⟩ := h
  exact ⟨Ne.symm h1, h3, h2, q, hq, hb, ha, fun q' h1' h2' => hmin q' h2' h1'⟩

theorem rel_symm {a b : Nat} (h : cc.Rel a b) : cc.Rel b a := by
  rcases h with h | h | h | h
  · exact Or.inl h.symm
  · exact Or.inr (Or.inr (Or.inl h))
  · exact Or.inr (Or.inl h)
  · exact Or.inr (Or.inr (Or.inr (compat_symm h)))

theorem rel_refl (a : Nat) : cc.Rel a a := Or.inl rfl

theorem desc_or_eq_trans {a b c : Nat} (h₁ : a = b ∨ cc.isDesc a b = true)
    (h₂ : b = c ∨ cc.isDesc b c = true) : a = c ∨ cc.isDesc a c = true := by
  rcases h₁ with rfl | h₁
  · exact h₂
  · rcases h₂ with rfl | h₂
    · exact Or.inr h₁
    · exact Or.inr (cc.isDesc_trans h₁ h₂)

theorem desc_of_desc_or_eq {a b c : Nat} (h₁ : a = b ∨ cc.isDesc a b = true)
    (h₂ : cc.isDesc b c = true) : cc.isDesc a c = true := by
  rcases h₁ with rfl | h₁
  · exact h₂
  · exact cc.isDesc_trans h₁ h₂

theorem desc_of_desc_desc_or_eq {a b c : Nat} (h₁ : cc.isDesc a b = true)
    (h₂ : b = c ∨ cc.isDesc b c = true) : cc.isDesc a c = true := by
  rcases h₂ with rfl | h₂
  · exact h₁
  · exact cc.isDesc_trans h₁ h₂

theorem atomic_children {b : Nat} (h : cc.isAtomic b = true) : (cc.st b).children = [] := by
  unfold CChart.isAtomic at h
  simp only [Bool.and_eq_true, List.isEmpty_iff] at h
  exact h.1.2

theorem not_desc_both {a b : Nat} (h₁ : cc.isDesc a b = true) (h₂ : cc.isDesc b a = true) :
    False := by
  have := cc.isDesc_lt h₁; have := cc.isDesc_lt h₂; omega

/-- Compatible states have no common descendant-or-self. -/
theorem compat_disjoint {a b w : Nat} (h : cc.Compatible a b) (ha : w = a ∨ cc.isDesc w a = true)
    (hb : w = b ∨ cc.isDesc w b = true) : False := by
  obtain ⟨hab, h1, h2, _⟩ := h
  rcases ha with rfl | ha <;> rcases hb with rfl | hb
  · exact hab rfl
  · rw [hb] at h1; cases h1
  · rw [ha] at h2; cases h2
  · rcases cc.isDesc_chain ha hb with rfl | h | h
    · exact hab rfl
    · rw [h] at h1; cases h1
    · rw [h] at h2; cases h2

theorem compat_up {a b y z q' : Nat} (h : cc.Compatible a b) (hy : y = a ∨ cc.isDesc y a = true)
    (hz : z = b ∨ cc.isDesc z b = true) (hyq : cc.isDesc y q' = true)
    (hzq : cc.isDesc z q' = true) : cc.isDesc a q' = true := by
  rcases hy with rfl | hy
  · exact hyq
  · rcases cc.isDesc_chain hy hyq with rfl | h' | h'
    · exact (compat_disjoint h (Or.inr hzq) hz).elim
    · exact h'
    · exact (compat_disjoint h (Or.inr (cc.isDesc_trans hzq h')) hz).elim

/-- Compatibility passes to descendants. -/
theorem compat_mono {a b y z : Nat} (h : cc.Compatible a b) (hy : y = a ∨ cc.isDesc y a = true)
    (hz : z = b ∨ cc.isDesc z b = true) : cc.Compatible y z := by
  have h' := h
  obtain ⟨_, _, _, q, hq, haq, hbq, hmin⟩ := h'
  refine ⟨?_, ?_, ?_, q, hq, desc_of_desc_or_eq hy haq, desc_of_desc_or_eq hz hbq, ?_⟩
  · rintro rfl; exact compat_disjoint h hy hz
  · cases hd : cc.isDesc y z
    · rfl
    · exact (compat_disjoint h hy (Or.inr (desc_of_desc_or_eq (Or.inr hd) (by
        rcases hz with rfl | hz
        · exact (compat_disjoint h hy (Or.inr hd) |>.elim)
        · exact hz)))).elim
  · cases hd : cc.isDesc z y
    · rfl
    · exfalso
      rcases hy with rfl | hy
      · exact compat_disjoint h (Or.inr hd) hz
      · exact compat_disjoint h (Or.inr (cc.isDesc_trans hd hy)) hz
  · intro q' hy' hz'
    exact hmin q' (compat_up h hy hz hy' hz') (compat_up (compat_symm h) hz hy hz' hy')

theorem rel_of_compat_mono {a b y z : Nat} (h : cc.Compatible a b)
    (hy : y = a ∨ cc.isDesc y a = true) (hz : z = b ∨ cc.isDesc z b = true) : cc.Rel y z :=
  Or.inr (Or.inr (Or.inr (compat_mono h hy hz)))

/-- Siblings below a parallel state are compatible. -/
theorem sibling_compat (hw : CWF cc) {A c c' : Nat} (hc : (cc.st c).parent = some A)
    (hc' : (cc.st c').parent = some A) (hne : c ≠ c') (hA : cc.isParallel A = true) :
    cc.Compatible c c' := by
  have hlt := hw.parent_lt' hc
  have hlt' := hw.parent_lt' hc'
  refine ⟨hne, ?_, ?_, A, hA, hw.isDesc_parent hc, hw.isDesc_parent hc', ?_⟩
  · cases hd : cc.isDesc c c'
    · rfl
    · exfalso
      rcases cc.parent_desc_or_eq hd hc with rfl | h
      · omega
      · have := cc.isDesc_lt h; omega
  · cases hd : cc.isDesc c' c
    · rfl
    · exfalso
      rcases cc.parent_desc_or_eq hd hc' with rfl | h
      · omega
      · have := cc.isDesc_lt h; omega
  · intro q' h1 _
    rcases cc.parent_desc_or_eq h1 hc with rfl | h
    · exact Or.inl rfl
    · exact Or.inr h

/-- Compatible siblings have a parallel parent. -/
theorem sibling_parallel (hw : CWF cc) {A c c' : Nat} (hc : (cc.st c).parent = some A)
    (hc' : (cc.st c').parent = some A) (h : cc.Compatible c c') : cc.isParallel A = true := by
  obtain ⟨_, _, _, q, hq, hcq, _, hmin⟩ := h
  rcases hmin A (hw.isDesc_parent hc) (hw.isDesc_parent hc') with rfl | h'
  · exact hq
  · exfalso
    rcases cc.parent_desc_or_eq hcq hc with rfl | h''
    · exact not_desc_both h' h' |>.elim
    · exact not_desc_both h' h''

/-- An ancestor of a state related to every state of a set is related to every state of it. -/
theorem rel_anc {z a w : Nat} (hza : cc.isDesc z a = true) (hzw : cc.Rel z w) : cc.Rel a w := by
  by_cases h1 : a = w
  · exact Or.inl h1
  cases h2 : cc.isDesc a w
  · cases h3 : cc.isDesc w a
    · refine Or.inr (Or.inr (Or.inr ?_))
      rcases hzw with rfl | h | h | h
      · rw [hza] at h3; cases h3
      · rcases cc.isDesc_chain hza h with h' | h' | h'
        · exact absurd h' h1
        · rw [h'] at h2; cases h2
        · rw [h'] at h3; cases h3
      · rw [cc.isDesc_trans h hza] at h3; cases h3
      · obtain ⟨_, hzw1, _, q, hq, hzq, hwq, hmin⟩ := h
        rcases cc.isDesc_chain hza hzq with rfl | h' | h'
        · rw [hwq] at h3; cases h3
        · refine ⟨h1, h2, h3, q, hq, h', hwq, fun q' ha' hw' => hmin q' (cc.isDesc_trans hza ha') hw'⟩
        · rw [cc.isDesc_trans hwq h'] at h3; cases h3
    · exact Or.inr (Or.inr (Or.inl h3))
  · exact Or.inr (Or.inl h2)

/-! ## Lowest common ancestors and history values -/

/-- Two unrelated distinct proper descendants of `r` have a lowest common ancestor. -/
theorem lca_exists : ∀ x y r, cc.isDesc x r = true → cc.isDesc y r = true →
    x ≠ y → cc.isDesc x y = false → cc.isDesc y x = false →
    ∃ q, (q = r ∨ cc.isDesc q r = true) ∧ cc.isDesc x q = true ∧ cc.isDesc y q = true ∧
      ∀ q', cc.isDesc x q' = true → cc.isDesc y q' = true → q' = q ∨ cc.isDesc q q' = true := by
  intro x
  induction x using Nat.strongRecOn with
  | ind x ih =>
    intro y r hx hy hne h1 h2
    obtain ⟨p, hp, hlt, hpr⟩ := (cc.isDesc_iff_parent x r).1 hx
    by_cases hyp : cc.isDesc y p = true
    · refine ⟨p, hpr, cc.isDesc_of_parent hp hlt, hyp, fun q' hq' _ => ?_⟩
      exact (cc.parent_desc_or_eq hq' hp).imp_left Eq.symm
    · have hpr' : cc.isDesc p r = true := by
        rcases hpr with rfl | h
        · exact absurd hy hyp
        · exact h
      have hpy : p ≠ y := by
        rintro rfl; rw [cc.isDesc_of_parent hp hlt] at h1; cases h1
      have hpy1 : cc.isDesc p y = false := by
        cases h : cc.isDesc p y
        · rfl
        · rw [cc.isDesc_trans (cc.isDesc_of_parent hp hlt) h] at h1; cases h1
      have hpy2 : cc.isDesc y p = false := by simpa using hyp
      obtain ⟨q, hqr, hpq, hyq, hmin⟩ := ih p hlt y r hpr' hy hpy hpy1 hpy2
      refine ⟨q, hqr, cc.isDesc_trans (cc.isDesc_of_parent hp hlt) hpq, hyq, fun q' hq' hy' => ?_⟩
      rcases cc.parent_desc_or_eq hq' hp with rfl | h
      · exact absurd hy' hyp
      · exact hmin q' h hy'

/-- A legal fragment is closed under ancestors below its top. -/
theorem below_anc {p : Nat} {S : List Nat} (hb : cc.Below p S) :
    ∀ x, x ∈ S → ∀ a, cc.isDesc x a = true → cc.isDesc a p = true → a ∈ S := by
  intro x
  induction x using Nat.strongRecOn with
  | ind x ih =>
    intro hx a hxa hap
    obtain ⟨q, hq, hlt, hqa⟩ := (cc.isDesc_iff_parent x a).1 hxa
    rcases hb.parent x hx q hq with rfl | hqS
    · rcases hqa with rfl | h
      · exact absurd hap (by simp [cc.isDesc_irrefl])
      · exact (not_desc_both h hap).elim
    · rcases hqa with rfl | h
      · exact hqS
      · exact ih q hlt hqS a h hap

theorem kind_of_child (hw : CWF cc) {q c : Nat} (hc : c ∈ (cc.st q).children) :
    cc.isParallel q = true ∨ cc.isCompoundOrRoot q = true := by
  have hl := hw.leaf q
  have hne : (cc.st q).children ≠ [] := List.ne_nil_of_mem hc
  rw [CWF.isParallel_iff, CWF.isCompoundOrRoot_iff]
  rw [CWF.isHistory_iff] at hl
  revert hl hne
  cases (cc.st q).kind with
  | history ty => cases ty <;> simp_all
  | _ => simp_all

theorem compound_not_parallel {x : Nat} (h : cc.isCompoundOrRoot x = true) :
    cc.isParallel x = false := by
  rw [CWF.isCompoundOrRoot_iff] at h
  unfold CChart.isParallel
  rcases h with h | h <;> rw [h] <;> decide

/-- Two distinct unrelated states of a legal fragment are compatible: their lowest common
ancestor has two children in the fragment, so it is parallel. -/
theorem below_compat (hw : CWF cc) {p : Nat} {S : List Nat} (hb : cc.Below p S) {x y : Nat}
    (hx : x ∈ S) (hy : y ∈ S) (hne : x ≠ y) (h1 : cc.isDesc x y = false)
    (h2 : cc.isDesc y x = false) : cc.Compatible x y := by
  obtain ⟨q, hqp, hxq, hyq, hmin⟩ := lca_exists x y p (hb.desc x hx) (hb.desc y hy) hne h1 h2
  refine ⟨hne, h1, h2, q, ?_, hxq, hyq, hmin⟩
  obtain ⟨cx, hcx, hcx'⟩ := hw.chain_child_nonhist hxq (hb.noHist x hx)
  obtain ⟨cy, hcy, hcy'⟩ := hw.chain_child_nonhist hyq (hb.noHist y hy)
  have hcxd : cc.isDesc cx q = true := hw.child_isDesc hcx
  have hcyd : cc.isDesc cy q = true := hw.child_isDesc hcy
  have memS : ∀ c z, z ∈ S → (c = z ∨ cc.isDesc z c = true) → cc.isDesc c q = true → c ∈ S := by
    intro c z hz hcz hcq
    rcases hcz with rfl | hcz
    · exact hz
    · exact below_anc hb z hz c hcz (desc_of_desc_desc_or_eq hcq hqp)
  have hcxS := memS cx x hx hcx' hcxd
  have hcyS := memS cy y hy hcy' hcyd
  have hne' : cx ≠ cy := by
    rintro rfl
    rcases hcx' with rfl | hcx'
    · rcases hcy' with rfl | hcy'
      · exact hne rfl
      · rw [hcy'] at h2; cases h2
    · rcases hcy' with rfl | hcy'
      · rw [hcx'] at h1; cases h1
      · rcases hmin cx hcx' hcy' with rfl | h
        · exact absurd hcxd (by simp [cc.isDesc_irrefl])
        · exact not_desc_both h hcxd
  rcases kind_of_child hw hcx with h | h
  · exact h
  · exfalso
    have hex : ExactlyOne (cc.st q).children (· ∈ S) := by
      rcases hqp with rfl | hqp
      · exact hb.top h
      · have hqS : q ∈ S := below_anc hb x hx q hxq hqp
        have hq0 : q ≠ 0 := CWF.isDesc_ne_zero hqp
        exact hb.compound q hqS (hw.kind_compound_of_ne_zero hq0 h)
    exact hne' (hex.2 cx hcx cy hcy hcxS hcyS)

/-- Distinct values of a recorded history are compatible. -/
theorem histValue_compat (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist)
    {h : Nat} {v : List Nat} (hv : CChart.histValue hist h = some v) {x y : Nat} (hx : x ∈ v)
    (hy : y ∈ v) (hne : x ≠ y) : cc.Compatible x y := by
  obtain ⟨hh, p, S, hp, hb, hdeep, hshallow⟩ := ho h v hv
  cases hd : cc.isDeepHistory h
  · obtain ⟨hxS, hxp⟩ := (hshallow hd x).1 hx
    obtain ⟨hyS, hyp⟩ := (hshallow hd y).1 hy
    rcases hw.history_parent_kind hh hp with hk | hk
    · exfalso
      have hex := hb.top (CWF.isCompoundOrRoot_of_compound hk)
      have hxc : x ∈ (cc.st p).children :=
        (hw.children_iff p x).2 ⟨cc.isDesc_lt_size (hb.desc x hxS), hxp, hb.noHist x hxS⟩
      have hyc : y ∈ (cc.st p).children :=
        (hw.children_iff p y).2 ⟨cc.isDesc_lt_size (hb.desc y hyS), hyp, hb.noHist y hyS⟩
      exact hne (hex.2 x hxc y hyc hxS hyS)
    · exact sibling_compat hw hxp hyp hne hk
  · obtain ⟨hxS, hxa⟩ := (hdeep hd x).1 hx
    obtain ⟨hyS, hya⟩ := (hdeep hd y).1 hy
    have atom_nd : ∀ a b, a ∈ S → cc.isAtomic b = true → cc.isDesc a b = false := by
      intro a b ha hb'
      cases h' : cc.isDesc a b
      · rfl
      · obtain ⟨c, hc, _⟩ := hw.chain_child_nonhist h' (hb.noHist a ha)
        rw [atomic_children hb'] at hc; simp at hc
    exact below_compat hw hb hxS hyS hne (atom_nd x y hxS hya) (atom_nd y x hyS hxa)

/-! ## The ancestors `addAnc` visits -/

theorem mem_ancestorsBelow : ∀ (y stop a : Nat), cc.isDesc y stop = true →
    (a ∈ cc.ancestorsBelow y (some stop) ↔ cc.isDesc y a = true ∧ cc.isDesc a stop = true) := by
  intro y
  induction y using Nat.strongRecOn with
  | ind y ih =>
    intro stop a hys
    obtain ⟨p, hp, hlt, hps⟩ := (cc.isDesc_iff_parent y stop).1 hys
    have hanc : cc.ancestors y = p :: cc.ancestors p := cc.ancestors_eq_of_parent hp hlt
    simp only [CChart.ancestorsBelow, hanc, List.takeWhile_cons]
    by_cases hpe : p = stop
    · subst hpe
      simp only [bne_self_eq_false, Bool.false_eq_true, ite_false, List.not_mem_nil, false_iff]
      rintro ⟨h1, h2⟩
      rcases cc.parent_desc_or_eq h1 hp with rfl | h
      · exact absurd h2 (by simp [cc.isDesc_irrefl])
      · exact not_desc_both h h2
    · have hpe' : (p != stop) = true := by simpa using hpe
      simp only [hpe', ite_true, List.mem_cons]
      have hps' : cc.isDesc p stop = true := by
        rcases hps with h | h
        · exact absurd h hpe
        · exact h
      have := ih p hlt stop a hps'
      unfold CChart.ancestorsBelow at this
      rw [this]
      constructor
      · rintro (rfl | ⟨h1, h2⟩)
        · exact ⟨cc.isDesc_of_parent hp hlt, hps'⟩
        · exact ⟨cc.isDesc_trans (cc.isDesc_of_parent hp hlt) h1, h2⟩
      · rintro ⟨h1, h2⟩
        rcases cc.parent_desc_or_eq h1 hp with rfl | h
        · exact Or.inl rfl
        · exact Or.inr ⟨h, h2⟩

end Vertep.Entry
