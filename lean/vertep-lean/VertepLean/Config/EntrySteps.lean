import VertepLean.Config.EntryCore

/-!
# Entry lemma: one step at a time

Lemmas for single additions (an ancestor of an entered state, a child of a parallel state, a
target inside a compatible region keep `PC`), the effective targets of a history state, and the
contracts of `addDesc` on a non-history state (`SD`), on a history state (`SH`) and of
`addAnc` (`SA`), with the loops over parallel children and over `addAnc` calls.
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat}

/-! ## Single additions -/

theorem pc_extend {E E' : List Nat} {s : Nat} (hpc : PC cc d E) (hf : Fits cc d E s)
    (hm : ∀ x, x ∈ E' → x ∈ E ∨ x = s) : PC cc d E' := by
  intro a ha b hb had hbd
  rcases hm a ha with ha | rfl <;> rcases hm b hb with hb | rfl
  · exact hpc a ha b hb had hbd
  · exact rel_symm (hf a ha had)
  · exact hf b hb hbd
  · exact rel_refl _

theorem pc_add {acc : EntryAcc} {s : Nat} (hpc : PC cc d acc.toEnter) (hf : Fits cc d acc.toEnter s) :
    PC cc d (acc.add s).toEnter :=
  pc_extend hpc hf (fun x hx => (mem_add acc s x).1 hx)

theorem fits_anc {E : List Nat} {a z : Nat} (hpc : PC cc d E) (hz : z ∈ E)
    (hza : cc.isDesc z a = true) (hzd : cc.isDesc z d = true) : Fits cc d E a :=
  fun w hw hwd => rel_anc hza (hpc z hz w hw hzd hwd)

/-- An untouched child of an entered parallel state fits. -/
theorem fits_par_child (hw : CWF cc) {E : List Nat} {A c : Nat} (hpc : PC cc d E) (hA : A ∈ E)
    (hAd : cc.isDesc A d = true) (hpar : cc.isParallel A = true) (hc : c ∈ (cc.st A).children)
    (hun : ∀ z ∈ E, cc.isDesc z c = false) : Fits cc d E c := by
  intro w hwE hwd
  have hcA := hw.child_isDesc hc
  have hcp := ((hw.children_iff A c).1 hc).2.1
  rcases hpc A hA w hwE hAd hwd with rfl | h | h | h
  · exact Or.inr (Or.inl hcA)
  · exact Or.inr (Or.inl (cc.isDesc_trans hcA h))
  · obtain ⟨c', hc', hwc'⟩ := hw.chain_child w A h
    by_cases hcc : c' = c
    · subst hcc
      rcases hwc' with rfl | h'
      · exact rel_refl _
      · rw [hun w hwE] at h'; cases h'
    · exact rel_of_compat_mono (sibling_compat hw hcp hc' (Ne.symm hcc) hpar) (Or.inl rfl)
        (hwc'.imp_left Eq.symm)
  · exact rel_of_compat_mono h (Or.inr hcA) (Or.inl rfl)

/-- Fitting at `G` with nothing below it passes down to states below `G`. -/
theorem fits_down {E : List Nat} {G r : Nat} (hf : Fits cc d E G)
    (hn : ∀ z ∈ E, cc.isDesc z G = false) (hr : r = G ∨ cc.isDesc r G = true) :
    Fits cc d E r ∧ ∀ z ∈ E, cc.isDesc z r = false := by
  refine ⟨fun w hw hwd => ?_, fun z hz => ?_⟩
  · rcases hf w hw hwd with rfl | h | h | h
    · rcases hr with rfl | hr
      · exact rel_refl _
      · exact Or.inr (Or.inl hr)
    · exact Or.inr (Or.inl (desc_of_desc_or_eq hr h))
    · rw [hn w hw] at h; cases h
    · exact rel_of_compat_mono h hr (Or.inl rfl)
  · cases h : cc.isDesc z r
    · rfl
    · have := desc_of_desc_desc_or_eq h hr
      rw [hn z hz] at this; cases this

theorem hg_in {D G : Nat} (hg : Hg cc d D G) : G = d ∨ cc.isDesc G d = true := by
  rcases hg with ⟨rfl, h⟩ | ⟨rfl, _⟩
  · exact h
  · exact Or.inl rfl

theorem hg_iff {D G : Nat} (hg : Hg cc d D G) (a : Nat) :
    cc.isDesc a D = true ∧ cc.isDesc a d = true ↔ cc.isDesc a G = true := by
  rcases hg with ⟨rfl, h⟩ | ⟨rfl, h⟩
  · exact ⟨fun h' => h'.1, fun h' => ⟨h', desc_of_desc_desc_or_eq h' h⟩⟩
  · exact ⟨fun h' => h'.2, fun h' => ⟨cc.isDesc_trans h' h, h'⟩⟩

theorem mem_ancL {y D G a : Nat} (hyD : cc.isDesc y D = true) (hg : Hg cc d D G) :
    a ∈ ancL cc d y (some D) ↔ cc.isDesc y a = true ∧ cc.isDesc a G = true := by
  unfold ancL
  rw [List.mem_filter, mem_ancestorsBelow y D a hyD, ← hg_iff hg a]
  constructor
  · rintro ⟨⟨h1, h2⟩, h3⟩
    simp only [Bool.and_eq_true] at h3
    exact ⟨h1, h2, h3.2⟩
  · rintro ⟨h1, h2, h3⟩
    refine ⟨⟨h1, h2⟩, ?_⟩
    simp only [Bool.and_eq_true, bne_iff_ne, ne_eq]
    exact ⟨CWF.isDesc_ne_zero h3, h3⟩

theorem in_of_G {D G a : Nat} (hg : Hg cc d D G) (h : cc.isDesc a G = true) :
    cc.isDesc a d = true :=
  desc_of_desc_desc_or_eq h (hg_in hg)

/-! ## Effective targets of one target -/

theorem effOfH_below (hw : CWF cc) (ho : HistOk cc hist) {h e P : Nat}
    (hh : cc.isHistory h = true) (he : EffOfH cc hist h e) (hP : (cc.st h).parent = some P) :
    cc.isDesc e P = true := by
  rcases he with ⟨v, hv, hev⟩ | ⟨_, d0, hd0, hm⟩
  · exact (hw.histValue_ne ho hv).2 P hP e hev
  · obtain ⟨d1, hd1, _, hdesc⟩ := historyDefault_of hw hh
    rw [hd0] at hd1; cases hd1
    exact hw.effMem_below ho P d0 e hm (fun y hy => hdesc y hy P hP)

theorem effOfH_exists (hw : CWF cc) (ho : HistOk cc hist) {h P : Nat}
    (hh : cc.isHistory h = true) (hP : (cc.st h).parent = some P) :
    ∃ e, EffOfH cc hist h e ∧ cc.isDesc e P = true := by
  cases hv : CChart.histValue hist h with
  | some v =>
    obtain ⟨x, hx⟩ := (hw.histValue_ne ho hv).1
    exact ⟨x, Or.inl ⟨v, hv, hx⟩, (hw.histValue_ne ho hv).2 P hP x hx⟩
  | none =>
    obtain ⟨d0, hd0, hne, hdesc⟩ := historyDefault_of hw hh
    obtain ⟨e, he⟩ := hw.effMem_exists ho d0 hne
    exact ⟨e, Or.inr ⟨hv, d0, hd0, he⟩,
      hw.effMem_below ho P d0 e he (fun y hy => hdesc y hy P hP)⟩

theorem effMem_iff {t e : Nat} :
    EffMem cc hist t e ↔ ∃ x ∈ (cc.tr t).targets, EffOf cc hist x e := by
  constructor
  · intro h
    cases h with
    | plain hx hh => exact ⟨_, hx, Or.inl ⟨hh, rfl⟩⟩
    | value hx hh hv hxv => exact ⟨_, hx, Or.inr ⟨hh, Or.inl ⟨_, hv, hxv⟩⟩⟩
    | dflt hx hh hv hd hm => exact ⟨_, hx, Or.inr ⟨hh, Or.inr ⟨hv, _, hd, hm⟩⟩⟩
  · rintro ⟨x, hx, ⟨hh, rfl⟩ | ⟨hh, ⟨v, hv, hev⟩ | ⟨hv, d0, hd0, hm⟩⟩⟩
    · exact .plain hx hh
    · exact .value hx hh hv hev
    · exact .dflt hx hh hv hd0 hm

/-! ## The contracts -/

section Contracts

variable (cc : CChart) (hist : List (Nat × List Nat)) (d : Nat)

/-- `addDesc` on a non-history state `s` below the floor, from a state where `s` fits and
nothing is entered below it, enters `s` and completes it below. -/
def SD (n : Nat) : Prop :=
  ∀ s (acc acc' : EntryAcc), cc.addDesc hist d n s acc = some acc' → cc.isHistory s = false →
    cc.isDesc s d = true → PC cc d acc.toEnter → Fits cc d acc.toEnter s →
    (∀ z ∈ acc.toEnter, cc.isDesc z s = false) →
    PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
    (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ x = s ∨ cc.isDesc x s = true) ∧ s ∈ acc'.toEnter ∧
    (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → w ≠ s → Cond cc acc'.toEnter s w) ∧
    C34 cc acc'.toEnter s ∧ DoneD cc hist d acc'.toEnter n s

/-- `addDesc` on a history state `h` with parent `P`, entering below the root `G`, restores a
complete fragment below `G` containing its effective targets. -/
def SH (n : Nat) : Prop :=
  ∀ h P G (acc acc' : EntryAcc), cc.addDesc hist d n h acc = some acc' →
    cc.isHistory h = true → (cc.st h).parent = some P → Hg cc d P G →
    (∀ e, EffOfH cc hist h e → cc.isDesc e G = true) → PC cc d acc.toEnter →
    Fits cc d acc.toEnter G → (∀ z ∈ acc.toEnter, cc.isDesc z G = false) →
    PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
    (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧
    (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → Cond cc acc'.toEnter G w) ∧
    (∃ z ∈ acc'.toEnter, cc.isDesc z G = true) ∧
    (∀ e, EffOfH cc hist h e → e ∈ acc'.toEnter) ∧ DoneD cc hist d acc'.toEnter n h

/-- Anchored: every ancestor of `y` below `G` has an entered descendant. -/
def Anch (E : List Nat) (G y : Nat) : Prop :=
  ∀ a, cc.isDesc y a = true → cc.isDesc a G = true → ∃ z ∈ E, cc.isDesc z a = true

/-- `addAnc` on an anchored state `y` below `D` enters its ancestors below the root `G` and
completes the parallel ones. -/
def SA (n : Nat) : Prop :=
  ∀ y D G (acc acc' : EntryAcc), cc.addAnc hist d n y (some D) acc = some acc' →
    cc.isDesc y D = true → Hg cc d D G → PC cc d acc.toEnter → Anch cc acc.toEnter G y →
    PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
    (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧
    (∀ a, cc.isDesc y a = true → cc.isDesc a G = true → a ∈ acc'.toEnter) ∧
    (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → Cond cc acc'.toEnter G w) ∧
    DoneA cc hist d acc'.toEnter n y (some D)

end Contracts

theorem anch_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {G y : Nat} (ha : Anch cc E G y) :
    Anch cc E' G y := by
  intro a h1 h2
  obtain ⟨z, hz, h'⟩ := ha a h1 h2
  exact ⟨z, h z hz, h'⟩

/-! ## The loop over the children of a parallel state -/

theorem loop_spec (hw : CWF cc) {n : Nat} (hSD : SD cc hist d n) (A : Nat) :
    ∀ (cs : List Nat) (acc acc' : EntryAcc), (∀ c ∈ cs, c ∈ (cc.st A).children) →
      cs.foldlM (fun acc c => if (acc.toEnter.any fun x => cc.isDesc x c) = true then pure acc
        else cc.addDesc hist d n c acc) acc = some acc' →
      cc.isParallel A = true → A ∈ acc.toEnter → cc.isDesc A d = true → PC cc d acc.toEnter →
      PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
      (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x A = true) ∧
      (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → Cond cc acc'.toEnter A w) ∧
      (∀ c ∈ cs, Touched cc acc'.toEnter c ∧
        ((∃ z ∈ acc'.toEnter, cc.isDesc z c = true) ∨ DoneD cc hist d acc'.toEnter n c)) := by
  intro cs
  induction cs with
  | nil =>
    intro acc acc' _ h _ _ _ hpc
    rw [foldlM_nil_some h]
    exact ⟨hpc, fun x hx => hx, fun x hx => Or.inl hx, fun w hw hn => absurd hw hn,
      fun c hc => by simp at hc⟩
  | cons c cs ih =>
    intro acc acc' hcs h hpar hA hAd hpc
    obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
    have hcc := hcs c List.mem_cons_self
    have hcA := hw.child_isDesc hcc
    obtain ⟨_, hcp, hch⟩ := (hw.children_iff A c).1 hcc
    -- the step on `c`
    have step : PC cc d b.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ b.toEnter) ∧
        (∀ x ∈ b.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x A = true) ∧
        (∀ w ∈ b.toEnter, w ∉ acc.toEnter → Cond cc b.toEnter A w) ∧ Touched cc b.toEnter c ∧
        ((∃ z ∈ b.toEnter, cc.isDesc z c = true) ∨ DoneD cc hist d b.toEnter n c) := by
      split at hb
      · rename_i hany
        simp at hb; subst hb
        obtain ⟨z, hz, hzc⟩ := List.any_eq_true.1 hany
        exact ⟨hpc, fun x hx => hx, fun x hx => Or.inl hx, fun w hw hn => absurd hw hn,
          ⟨z, hz, Or.inr hzc⟩, Or.inl ⟨z, hz, hzc⟩⟩
      · rename_i hany
        have hun : ∀ z ∈ acc.toEnter, cc.isDesc z c = false := by
          intro z hz
          cases h' : cc.isDesc z c
          · rfl
          · exact absurd (List.any_eq_true.2 ⟨z, hz, h'⟩) hany
        obtain ⟨h1, h2, h3, h4, h5, h6, h7⟩ := hSD c acc b hb hch (cc.isDesc_trans hcA hAd) hpc
          (fits_par_child hw hpc hA hAd hpar hcc hun) hun
        refine ⟨h1, h2, fun x hx => ?_, fun w hwb hwn => ?_, ⟨c, h4, Or.inl rfl⟩, Or.inr h7⟩
        · rcases h3 x hx with h | rfl | h
          · exact Or.inl h
          · exact Or.inr hcA
          · exact Or.inr (cc.isDesc_trans h hcA)
        · by_cases hwc : w = c
          · subst hwc
            exact ⟨hch, fun q hq => by rw [hcp] at hq; cases hq; exact Or.inl rfl, h6⟩
          · obtain ⟨hh, hq, h34⟩ := h5 w hwb hwn hwc
            refine ⟨hh, fun q hq' => ?_, h34⟩
            rcases hq q hq' with rfl | hq''
            · exact Or.inr h4
            · exact Or.inr hq''
    obtain ⟨s1, s2, s3, s4, s5, s6⟩ := step
    obtain ⟨i1, i2, i3, i4, i5⟩ := ih b acc' (fun c' hc' => hcs c' (List.mem_cons_of_mem _ hc'))
      hr hpar (s2 A hA) hAd s1
    refine ⟨i1, fun x hx => i2 x (s2 x hx), fun x hx => ?_, fun w hw' hwn => ?_, ?_⟩
    · rcases i3 x hx with h | h
      · exact s3 x h
      · exact Or.inr h
    · by_cases hwb : w ∈ b.toEnter
      · exact cond_mono i2 (s4 w hwb hwn)
      · exact i4 w hw' hwb
    · intro c' hc'
      rcases List.mem_cons.1 hc' with rfl | hc'
      · refine ⟨touched_mono i2 s5, ?_⟩
        rcases s6 with ⟨z, hz, h'⟩ | h'
        · exact Or.inl ⟨z, i2 z hz, h'⟩
        · exact Or.inr (doneD_mono i2 h')
      · exact i5 c' hc'

end Vertep.Entry
