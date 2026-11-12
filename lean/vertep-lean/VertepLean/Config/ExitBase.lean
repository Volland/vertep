import VertepLean.Config.ExitEff

/-!
# Exit lemma: facts about domains, history values and effective targets

Lemmas behind `exit_legal`: the domain of a targeted transition contains its effective targets
and is compound or the root; effective targets are nonempty and lie below the parents of the
history states they come from; recorded history values are nonempty. Also `start_entry`.
-/

namespace Vertep.CWF

open CChart

variable {cc : CChart}

theorem isCompound_iff (x : Nat) : cc.isCompound x = true ↔ (cc.st x).kind = .compound := by
  unfold CChart.isCompound
  cases (cc.st x).kind <;> (try rename_i h; cases h) <;> decide

theorem isParallel_iff (x : Nat) : cc.isParallel x = true ↔ (cc.st x).kind = .parallel := by
  unfold CChart.isParallel
  cases (cc.st x).kind <;> (try rename_i h; cases h) <;> decide

theorem isCompoundOrRoot_iff (x : Nat) : cc.isCompoundOrRoot x = true ↔
    (cc.st x).kind = .compound ∨ (cc.st x).kind = .root := by
  unfold CChart.isCompoundOrRoot
  cases (cc.st x).kind <;> (try rename_i h; cases h) <;> decide

theorem isHistory_iff (x : Nat) : cc.isHistory x = true ↔
    (cc.st x).kind = .history .shallow ∨ (cc.st x).kind = .history .deep := by
  unfold CChart.isHistory
  cases (cc.st x).kind <;> (try rename_i h; cases h) <;> decide

theorem kind_compound_of_ne_zero (hw : CWF cc) {x : Nat} (hx : x ≠ 0)
    (h : cc.isCompoundOrRoot x = true) : cc.isCompound x = true := by
  rw [isCompoundOrRoot_iff] at h
  rw [isCompound_iff]
  have := hw.kind_ne_root x hx
  rcases h with h | h
  · exact h
  · exact absurd h this

theorem isCompoundOrRoot_zero (hw : CWF cc) : cc.isCompoundOrRoot 0 = true := by
  unfold CChart.isCompoundOrRoot; rw [hw.root_kind]; decide

theorem isCompoundOrRoot_of_compound {x : Nat} (h : cc.isCompound x = true) :
    cc.isCompoundOrRoot x = true := by
  rw [isCompound_iff] at h; rw [isCompoundOrRoot_iff]; exact Or.inl h

/-- The child of `p` on the way down to a proper descendant `x`. -/
theorem chain_child (_hw : CWF cc) : ∀ x p, cc.isDesc x p = true →
    ∃ c, (cc.st c).parent = some p ∧ (c = x ∨ cc.isDesc x c = true) := by
  intro x
  induction x using Nat.strongRecOn with
  | ind x ih =>
    intro p h
    obtain ⟨q, hq, hlt, h'⟩ := (cc.isDesc_iff_parent x p).1 h
    rcases h' with rfl | h'
    · exact ⟨x, hq, Or.inl rfl⟩
    · obtain ⟨c, hc, hc'⟩ := ih q hlt p h'
      refine ⟨c, hc, Or.inr ?_⟩
      rcases hc' with rfl | hc'
      · exact cc.isDesc_of_parent hq hlt
      · exact cc.isDesc_trans (cc.isDesc_of_parent hq hlt) hc'

/-- A state with a proper descendant has a child or a history child. -/
theorem has_kid (hw : CWF cc) {x c : Nat} (h : cc.isDesc x c = true) :
    (cc.st c).children ≠ [] ∨ (cc.st c).histories ≠ [] := by
  obtain ⟨y, hy, _⟩ := hw.chain_child x c h
  have hys : y < cc.size := by
    rcases ‹y = x ∨ cc.isDesc x y = true› with rfl | h'
    · exact cc.isDesc_lt_size h
    · exact Nat.lt_trans (cc.isDesc_lt h') (cc.isDesc_lt_size h)
  cases hh : cc.isHistory y
  · left; exact List.ne_nil_of_mem ((hw.children_iff c y).2 ⟨hys, hy, hh⟩)
  · right; exact List.ne_nil_of_mem ((hw.histories_iff c y).2 ⟨hys, hy, hh⟩)

/-- History states have no proper descendants. -/
theorem not_isDesc_history (hw : CWF cc) {x h : Nat} (hh : cc.isHistory h = true) :
    cc.isDesc x h = false := by
  cases hd : cc.isDesc x h
  · rfl
  · have := hw.leaf h (Or.inr (Or.inr hh))
    rcases hw.has_kid hd with h1 | h1
    · exact absurd this.1 h1
    · exact absurd this.2 h1

/-- The child of `p` on the way down to a non-history proper descendant is not a history. -/
theorem chain_child_nonhist (hw : CWF cc) {x p : Nat} (h : cc.isDesc x p = true)
    (hx : cc.isHistory x = false) : ∃ c ∈ (cc.st p).children, c = x ∨ cc.isDesc x c = true := by
  obtain ⟨c, hc, hc'⟩ := hw.chain_child x p h
  have hcs : c < cc.size := by
    rcases hc' with rfl | h'
    · exact cc.isDesc_lt_size h
    · exact Nat.lt_trans (cc.isDesc_lt h') (cc.isDesc_lt_size h)
  have hch : cc.isHistory c = false := by
    rcases hc' with rfl | h'
    · exact hx
    · cases hhc : cc.isHistory c
      · rfl
      · rw [hw.not_isDesc_history hhc] at h'; cases h'
  exact ⟨c, (hw.children_iff p c).2 ⟨hcs, hc, hch⟩, hc'⟩

/-- The parent of a history state has a (non-history) child. -/
theorem history_parent_child (hw : CWF cc) : ∀ h, cc.isHistory h = true → ∀ p,
    (cc.st h).parent = some p → (cc.st p).children ≠ [] := by
  obtain ⟨r, hlt, hr⟩ := hw.history_rank
  suffices H : ∀ n h, r h < n → cc.isHistory h = true → ∀ p, (cc.st h).parent = some p →
      ∃ x, cc.isHistory x = false ∧ cc.isDesc x p = true by
    intro h hh p hp
    obtain ⟨x, hx, hd⟩ := H (r h + 1) h (Nat.lt_succ_self _) hh p hp
    obtain ⟨c, hc, _⟩ := hw.chain_child_nonhist hd hx
    exact List.ne_nil_of_mem hc
  intro n
  induction n with
  | zero => intro h hlt'; omega
  | succ n ih =>
    intro h hrn hh p hp
    obtain ⟨d, hd, hne, hdesc⟩ := hw.history_default h hh
    obtain ⟨y, hy⟩ := List.exists_mem_of_ne_nil _ hne
    have hyp := hdesc y hy p hp
    cases hyh : cc.isHistory y
    · exact ⟨y, hyh, hyp⟩
    · obtain ⟨p', hp', _⟩ := hw.history_parent y hyh
      have hdef : cc.historyDefault h = some d := by simp [CChart.historyDefault, hd]
      have := hr h d y hh hdef hy hyh
      obtain ⟨x, hx, hxp'⟩ := ih y (by omega) hyh p' hp'
      refine ⟨x, hx, ?_⟩
      rcases cc.parent_desc_or_eq hyp hp' with rfl | h'
      · exact hxp'
      · exact cc.isDesc_trans hxp' h'

theorem isDesc_ne_zero {x p : Nat} (h : cc.isDesc x p = true) : x ≠ 0 := by
  have := cc.isDesc_lt h; omega

/-- Every state of a legal fragment has an atomic descendant-or-self in the fragment. -/
theorem below_atomic (hw : CWF cc) {p : Nat} {S : List Nat} (hb : cc.Below p S) :
    ∀ x ∈ S, ∃ a ∈ S, cc.isAtomic a = true ∧ (a = x ∨ cc.isDesc a x = true) := by
  suffices H : ∀ n x, cc.size - x < n → x ∈ S →
      ∃ a ∈ S, cc.isAtomic a = true ∧ (a = x ∨ cc.isDesc a x = true) from
    fun x hx => H (cc.size - x + 1) x (by omega) hx
  intro n
  induction n with
  | zero => intro x h; omega
  | succ n ih =>
    intro x hn hx
    have hx0 : x ≠ 0 := isDesc_ne_zero (hb.desc x hx)
    have hxs : x < cc.size := cc.isDesc_lt_size (hb.desc x hx)
    have hxh := hb.noHist x hx
    by_cases hc : (cc.st x).children = []
    · refine ⟨x, hx, ?_, Or.inl rfl⟩
      simp [CChart.isAtomic, hx0, hc, hxh]
    · -- x is compound or parallel
      have hk : cc.isCompound x = true ∨ cc.isParallel x = true := by
        have hnr := hw.kind_ne_root x hx0
        have hl := hw.leaf x
        rw [isCompound_iff, isParallel_iff]
        have hxh' : ¬((cc.st x).kind = .history .shallow ∨ (cc.st x).kind = .history .deep) := by
          rw [← isHistory_iff]; simp [hxh]
        rw [isHistory_iff] at hl
        revert hnr hl hxh'
        cases (cc.st x).kind with
        | history ty => cases ty <;> simp_all
        | _ => simp_all
      have : ∃ c ∈ (cc.st x).children, c ∈ S := by
        rcases hk with hk | hk
        · exact (hb.compound x hx hk).1
        · obtain ⟨c, hc'⟩ := List.exists_mem_of_ne_nil _ hc
          exact ⟨c, hc', hb.parallel x hx hk c hc'⟩
      obtain ⟨c, hcx, hcS⟩ := this
      have hcd := hw.child_isDesc hcx
      have := cc.isDesc_lt hcd
      obtain ⟨a, ha, hat, hac⟩ := ih c (by omega) hcS
      refine ⟨a, ha, hat, Or.inr ?_⟩
      rcases hac with rfl | hac
      · exact hcd
      · exact cc.isDesc_trans hac hcd

/-- The kind of the parent of a history state: compound or parallel. -/
theorem history_parent_kind (hw : CWF cc) {h p : Nat} (hh : cc.isHistory h = true)
    (hp : (cc.st h).parent = some p) : cc.isCompound p = true ∨ cc.isParallel p = true := by
  obtain ⟨p', hp', hp0⟩ := hw.history_parent h hh
  rw [hp] at hp'; cases hp'
  have hhs : h < cc.size := by
    have := hw.isDesc_parent hp
    exact cc.isDesc_lt_size this
  have hmem : h ∈ (cc.st p).histories := (hw.histories_iff p h).2 ⟨hhs, hp, hh⟩
  have hnr := hw.kind_ne_root p hp0
  have hl := hw.leaf p
  have hne : (cc.st p).histories ≠ [] := List.ne_nil_of_mem hmem
  rw [isCompound_iff, isParallel_iff]
  rw [isHistory_iff] at hl
  revert hnr hl hne
  cases (cc.st p).kind with
  | history ty => cases ty <;> simp_all
  | _ => simp_all

/-- Recorded history values are nonempty and lie below the history's parent. -/
theorem histValue_ne (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist)
    {h : Nat} {v : List Nat} (hv : CChart.histValue hist h = some v) :
    (∃ x, x ∈ v) ∧ ∀ p, (cc.st h).parent = some p → ∀ x ∈ v, cc.isDesc x p = true := by
  obtain ⟨hh, p, S, hp, hb, hdeep, hshallow⟩ := ho h v hv
  constructor
  · have hchild : ∃ c ∈ (cc.st p).children, c ∈ S := by
      rcases hw.history_parent_kind hh hp with hk | hk
      · exact (hb.top (isCompoundOrRoot_of_compound hk)).1
      · obtain ⟨c, hc⟩ := List.exists_mem_of_ne_nil _ (hw.history_parent_child h hh p hp)
        exact ⟨c, hc, hb.topPar hk c hc⟩
    obtain ⟨c, hc, hcS⟩ := hchild
    cases hd : cc.isDeepHistory h
    · exact ⟨c, (hshallow hd c).2 ⟨hcS, ((hw.children_iff p c).1 hc).2.1⟩⟩
    · obtain ⟨a, ha, hat, _⟩ := hw.below_atomic hb c hcS
      exact ⟨a, (hdeep hd a).2 ⟨ha, hat⟩⟩
  · intro p' hp' x hx
    rw [hp] at hp'; cases hp'
    cases hd : cc.isDeepHistory h
    · exact hb.desc x ((hshallow hd x).1 hx).1
    · exact hb.desc x ((hdeep hd x).1 hx).1

/-- Effective targets of a transition whose targets lie below `p` lie below `p`. -/
theorem effMem_below (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist)
    (p : Nat) : ∀ u e, EffMem cc hist u e → (∀ y ∈ (cc.tr u).targets, cc.isDesc y p = true) →
      cc.isDesc e p = true := by
  intro u e hm
  induction hm with
  | plain hx _ => intro ht; exact ht _ hx
  | @value t h v x hx hh hv hxv =>
    intro ht
    obtain ⟨p', hp', _⟩ := hw.history_parent h hh
    have h1 := (hw.histValue_ne ho hv).2 p' hp' x hxv
    rcases cc.parent_desc_or_eq (ht h hx) hp' with rfl | h2
    · exact h1
    · exact cc.isDesc_trans h1 h2
  | @dflt t h d x hx hh _ hd _ ih =>
    intro ht
    obtain ⟨d', hd', _, hdesc⟩ := hw.history_default h hh
    have : d = d' := by simp [CChart.historyDefault, hd'] at hd; exact hd.symm
    subst this
    obtain ⟨p', hp', _⟩ := hw.history_parent h hh
    apply ih
    intro y hy
    have h1 := hdesc y hy p' hp'
    rcases cc.parent_desc_or_eq (ht h hx) hp' with rfl | h2
    · exact h1
    · exact cc.isDesc_trans h1 h2

/-- Effective targets are proper states. -/
theorem effMem_pos (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist) :
    ∀ u e, EffMem cc hist u e → 0 < e ∧ e < cc.size := by
  intro u e hm
  induction hm with
  | plain hx _ => exact hw.targets_lt _ _ hx
  | @value t h v x hx hh hv hxv =>
    obtain ⟨p', hp', _⟩ := hw.history_parent h hh
    have h1 := (hw.histValue_ne ho hv).2 p' hp' x hxv
    exact ⟨Nat.pos_of_ne_zero (isDesc_ne_zero h1), cc.isDesc_lt_size h1⟩
  | dflt _ _ _ _ _ ih => exact ih

/-- A transition with targets has an effective target. -/
theorem effMem_exists (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist) :
    ∀ u, (cc.tr u).targets ≠ [] → ∃ e, EffMem cc hist u e := by
  obtain ⟨r, hlt, hr⟩ := hw.history_rank
  suffices H : ∀ n u, (cc.tr u).targets ≠ [] →
      (∀ h ∈ (cc.tr u).targets, cc.isHistory h = true → r h < n) → ∃ e, EffMem cc hist u e by
    intro u hu
    exact H cc.size u hu (fun h _ hh => hlt h hh)
  intro n
  induction n with
  | zero =>
    intro u hu hn
    obtain ⟨y, hy⟩ := List.exists_mem_of_ne_nil _ hu
    cases hyh : cc.isHistory y
    · exact ⟨y, .plain hy hyh⟩
    · have := hn y hy hyh; omega
  | succ n ih =>
    intro u hu hn
    obtain ⟨y, hy⟩ := List.exists_mem_of_ne_nil _ hu
    cases hyh : cc.isHistory y
    · exact ⟨y, .plain hy hyh⟩
    · cases hv : CChart.histValue hist y with
      | some v =>
        obtain ⟨x, hx⟩ := (hw.histValue_ne ho hv).1
        exact ⟨x, .value hy hyh hv hx⟩
      | none =>
        obtain ⟨d, hd, hne, _⟩ := hw.history_default y hyh
        have hdef : cc.historyDefault y = some d := by simp [CChart.historyDefault, hd]
        obtain ⟨e, he⟩ := ih d hne (fun h hh hhh => by
          have := hr y d h hyh hdef hh hhh
          have := hn y hy hyh
          omega)
        exact ⟨e, .dflt hy hyh hv hdef he⟩

/-- A history target has an effective target below its parent. -/
theorem effMem_history (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist)
    {u h p : Nat} (hu : h ∈ (cc.tr u).targets) (hh : cc.isHistory h = true)
    (hp : (cc.st h).parent = some p) : ∃ e, EffMem cc hist u e ∧ cc.isDesc e p = true := by
  cases hv : CChart.histValue hist h with
  | some v =>
    obtain ⟨x, hx⟩ := (hw.histValue_ne ho hv).1
    exact ⟨x, .value hu hh hv hx, (hw.histValue_ne ho hv).2 p hp x hx⟩
  | none =>
    obtain ⟨d, hd, hne, hdesc⟩ := hw.history_default h hh
    have hdef : cc.historyDefault h = some d := by simp [CChart.historyDefault, hd]
    obtain ⟨e, he⟩ := hw.effMem_exists ho d hne
    exact ⟨e, .dflt hu hh hv hdef he, hw.effMem_below ho p d e he (fun y hy => hdesc y hy p hp)⟩

/-! ## Domains -/

theorem domain_of_not_default {hist : List (Nat × List Nat)} {t : Nat}
    (hdef : (cc.tr t).defaultEntry = false) (ht : (cc.tr t).targets ≠ [])
    (he : cc.effectiveTargets hist t ≠ []) :
    cc.domain hist t = some (if (cc.tr t).internal && cc.isCompound (cc.tr t).source &&
        (cc.effectiveTargets hist t).all (fun s => cc.isDesc s (cc.tr t).source)
      then (cc.tr t).source
      else cc.findLCCA (cc.tr t).source (cc.effectiveTargets hist t)) := by
  unfold CChart.domain
  have ht' : (cc.tr t).targets.isEmpty = false := by simpa using ht
  have he' : (cc.effectiveTargets hist t).isEmpty = false := by simpa using he
  simp only [hdef, ht', he', Bool.false_eq_true, ite_false]
  split <;> simp_all

/-- The domain of a declared targeted transition: some state, compound or the root, an
ancestor-or-self of the source, containing every effective target. -/
theorem domain_spec (hw : CWF cc) {hist : List (Nat × List Nat)} (ho : HistOk cc hist)
    {t : Nat} (hdef : (cc.tr t).defaultEntry = false) (ht : (cc.tr t).targets ≠ []) :
    ∃ d, cc.domain hist t = some d ∧ cc.isCompoundOrRoot d = true ∧
      (d = 0 ∨ d = (cc.tr t).source ∨ cc.isDesc (cc.tr t).source d = true) ∧
      ∀ e, EffMem cc hist t e → cc.isDesc e d = true := by
  have hspec := eff_adequate cc hist hw ho
  obtain ⟨e0, he0⟩ := hw.effMem_exists ho t ht
  have hne : cc.effectiveTargets hist t ≠ [] :=
    List.ne_nil_of_mem ((hspec t e0).2 he0)
  rw [domain_of_not_default hdef ht hne]
  refine ⟨_, rfl, ?_⟩
  split
  · rename_i hc
    simp only [Bool.and_eq_true, List.all_eq_true] at hc
    refine ⟨isCompoundOrRoot_of_compound hc.1.2, Or.inr (Or.inl rfl), fun e he => ?_⟩
    exact hc.2 e ((hspec t e).2 he)
  · unfold CChart.findLCCA
    cases hf : (cc.ancestors (cc.tr t).source).find?
        (fun a => cc.isCompoundOrRoot a && (cc.effectiveTargets hist t).all
          (fun s => cc.isDesc s a)) with
    | none =>
      refine ⟨hw.isCompoundOrRoot_zero, Or.inl rfl, fun e he => ?_⟩
      obtain ⟨h1, h2⟩ := hw.effMem_pos ho t e he
      exact hw.isDesc_zero e h1 h2
    | some a =>
      have hp := List.find?_some hf
      have hm := List.mem_of_find?_eq_some hf
      simp only [Option.getD_some, Bool.and_eq_true, List.all_eq_true] at hp ⊢
      refine ⟨hp.1, Or.inr (Or.inr ((cc.isDesc_iff_mem _ _).2 hm)), fun e he => ?_⟩
      exact hp.2 e ((hspec t e).2 he)

end Vertep.CWF

namespace Vertep

open CChart

/-- **The entry at start.** The root's default entry from the empty configuration satisfies
`EntryPre`. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem start_entry : StartEntry := by
  intro cc t hw ht
  obtain ⟨hdef, hsrc, hne, _⟩ := hw.initial_ok 0 t ht
  have hdom : cc.domain [] t = some 0 := by
    unfold CChart.domain; simp [hdef, hsrc]
  have hho : HistOk cc [] := by
    intro h v hv; simp [CChart.histValue] at hv
  refine ⟨by simp, hho, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · intro t' ht' _
    simp at ht'; subst ht'
    refine ⟨0, hdom, ⟨Or.inl rfl, hw.isCompoundOrRoot_zero, by simp, ?_, ?_⟩⟩
    · intro e he
      obtain ⟨h1, h2⟩ := hw.effMem_pos hho t' e he
      exact hw.isDesc_zero e h1 h2
    · intro x hx
      left
      obtain ⟨hx0, hxs⟩ := hw.targets_lt t' x hx
      unfold CChart.pos
      split
      · rename_i hh
        obtain ⟨p, hp, hp0⟩ := hw.history_parent x hh
        rw [hp]
        exact hw.isDesc_zero p (Nat.pos_of_ne_zero hp0) (Nat.lt_trans (hw.parent_lt' hp) hxs)
      · exact hw.isDesc_zero x hx0 hxs
  · intro t₁ h₁ t₂ h₂ hne'
    simp at h₁ h₂; subst h₁; subst h₂; exact absurd rfl hne'
  · intro x hx; simp at hx
  · intro x hx; simp at hx
  · intro x hx; simp at hx
  · intro x hx _ hnd
    rcases hx with rfl | hx
    · exact absurd hdom (hnd t (by simp) hne)
    · simp at hx

end Vertep
