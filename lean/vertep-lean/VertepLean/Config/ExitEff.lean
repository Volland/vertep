import VertepLean.Config.Tree

/-!
# Effective targets: fuel adequacy

`effectiveTargets` computes exactly the effective targets `EffMem` on charts with acyclic
history defaults (`CWF.history_rank`): its fuel, the chart size plus one, is never exhausted.
-/

namespace Vertep.CChart

theorem mem_pushUnique (acc : List Nat) (y x : Nat) :
    x ∈ CChart.pushUnique acc y ↔ x ∈ acc ∨ x = y := by
  unfold CChart.pushUnique
  split
  · rename_i h
    constructor
    · exact Or.inl
    · rintro (h' | rfl)
      · exact h'
      · simpa using h
  · simp

theorem mem_foldl_pushUnique (l acc : List Nat) (x : Nat) :
    x ∈ l.foldl CChart.pushUnique acc ↔ x ∈ acc ∨ x ∈ l := by
  induction l generalizing acc with
  | nil => simp
  | cons y l ih =>
    rw [List.foldl_cons, ih, mem_pushUnique]
    simp [or_assoc]

variable (cc : CChart)

/-- The states one target contributes to the effective targets (with fuel `n` for history
defaults). -/
def effContrib (hist : List (Nat × List Nat)) (n s : Nat) : List Nat :=
  if cc.isHistory s then
    match histValue hist s with
    | some v => v
    | none => match cc.historyDefault s with
      | some d => cc.effTargetsF hist n d
      | none => []
  else [s]

theorem mem_foldl_gen {f : List Nat → Nat → List Nat} {g : Nat → List Nat}
    (hf : ∀ acc s x, x ∈ f acc s ↔ x ∈ acc ∨ x ∈ g s) :
    ∀ (l acc : List Nat) (x : Nat), x ∈ l.foldl f acc ↔ x ∈ acc ∨ ∃ s ∈ l, x ∈ g s := by
  intro l
  induction l with
  | nil => simp
  | cons y l ih =>
    intro acc x
    rw [List.foldl_cons, ih, hf]
    simp only [List.mem_cons, exists_eq_or_imp]
    constructor
    · rintro ((h | h) | h)
      · exact Or.inl h
      · exact Or.inr (Or.inl h)
      · exact Or.inr (Or.inr h)
    · rintro (h | h | h)
      · exact Or.inl (Or.inl h)
      · exact Or.inl (Or.inr h)
      · exact Or.inr h

theorem mem_effTargetsF (hist : List (Nat × List Nat)) (n t x : Nat) :
    x ∈ cc.effTargetsF hist (n + 1) t ↔ ∃ s ∈ (cc.tr t).targets, x ∈ cc.effContrib hist n s := by
  rw [CChart.effTargetsF]
  rw [mem_foldl_gen (g := cc.effContrib hist n)]
  · simp
  · intro acc s y
    unfold effContrib
    split
    · rename_i hh
      cases hv : histValue hist s with
      | some v => simp [mem_foldl_pushUnique]
      | none =>
        cases hd : cc.historyDefault s with
        | some d => simp [mem_foldl_pushUnique]
        | none => simp
    · rw [mem_pushUnique]; simp

theorem effTargetsF_sound (hist : List (Nat × List Nat)) :
    ∀ n t x, x ∈ cc.effTargetsF hist n t → EffMem cc hist t x := by
  intro n
  induction n with
  | zero => intro t x h; simp [CChart.effTargetsF] at h
  | succ n ih =>
    intro t x h
    obtain ⟨s, hs, hx⟩ := (cc.mem_effTargetsF hist n t x).1 h
    unfold effContrib at hx
    split at hx
    · rename_i hh
      cases hv : histValue hist s with
      | some v =>
        rw [hv] at hx
        exact .value hs hh hv hx
      | none =>
        rw [hv] at hx
        cases hd : cc.historyDefault s with
        | some d =>
          rw [hd] at hx
          exact .dflt hs hh hv hd (ih d x hx)
        | none => rw [hd] at hx; simp at hx
    · rename_i hh
      simp at hx; subst hx
      exact .plain hs (by simpa using hh)

theorem effTargetsF_complete (hist : List (Nat × List Nat)) (r : Nat → Nat)
    (hr : ∀ h d x, cc.isHistory h = true → cc.historyDefault h = some d →
      x ∈ (cc.tr d).targets → cc.isHistory x = true → r x < r h) :
    ∀ t x, EffMem cc hist t x → ∀ n, (∀ h ∈ (cc.tr t).targets, cc.isHistory h = true → r h < n) →
      x ∈ cc.effTargetsF hist (n + 1) t := by
  intro t x hm
  induction hm with
  | plain hx hh =>
    intro n _
    rw [mem_effTargetsF]
    exact ⟨_, hx, by simp [effContrib, hh]⟩
  | value hx hh hv hxv =>
    intro n _
    rw [mem_effTargetsF]
    exact ⟨_, hx, by simp [effContrib, hh, hv, hxv]⟩
  | @dflt t h d x hx hh hv hd _ ih =>
    intro n hn
    have hrh := hn h hx hh
    obtain ⟨m, rfl⟩ : ∃ m, n = m + 1 := ⟨n - 1, by omega⟩
    rw [mem_effTargetsF]
    refine ⟨h, hx, ?_⟩
    simp only [effContrib, hh, ite_true, hv, hd]
    apply ih m
    intro h' hh' hhist
    have := hr h d h' hh hd hh' hhist
    omega

end Vertep.CChart

namespace Vertep

/-- **Fuel adequacy of `effectiveTargets`.** On a chart satisfying `CWF`, `effectiveTargets`
computes exactly the effective targets. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem eff_adequate : EffAdequate := by
  intro cc hist hw _ t x
  obtain ⟨r, hlt, hr⟩ := hw.history_rank
  constructor
  · exact cc.effTargetsF_sound hist _ t x
  · intro hm
    exact cc.effTargetsF_complete hist r hr t x hm cc.size
      (fun h _ hh => hlt h hh)

end Vertep
