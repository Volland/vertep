import VertepLean.Config.EntryTree

/-!
# Entry lemma: fuel adequacy

`entrySet` never runs out of fuel on a chart satisfying `CWF` with legal history values. The
measure of a call is the number of states it may still enter: `addDesc` of a state `s` with the
set `U` of states it may reach needs fuel `2 |U|`, `addAnc` needs `2 |U| + 1`, and every nested
`addDesc` call works in `U` minus its own state. The set of a non-history state is its subtree;
the set of a history state `h` with parent `P` is everything below `P` except the history
children of `P` whose rank is not below that of `h` (`CWF.history_rank`).
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart}

/-- The number of states of the chart in `U`. -/
def cnt (cc : CChart) (U : Nat → Bool) : Nat := ((List.range cc.size).filter U).length

theorem filter_len_mono (l : List Nat) {U V : Nat → Bool} (h : ∀ y, U y = true → V y = true) :
    (l.filter U).length ≤ (l.filter V).length := by
  induction l with
  | nil => simp
  | cons a l ih =>
    simp only [List.filter_cons]
    cases hu : U a <;> cases hv : V a
    · simp; omega
    · simp; omega
    · have := h a hu; rw [hv] at this; cases this
    · simp; omega

theorem filter_len_erase (l : List Nat) (U : Nat → Bool) (s : Nat) (hs : s ∈ l)
    (hU : U s = true) : (l.filter (fun y => U y && y != s)).length + 1 ≤ (l.filter U).length := by
  induction l with
  | nil => simp at hs
  | cons a l ih =>
    simp only [List.filter_cons]
    by_cases has : a = s
    · subst has
      simp only [bne_self_eq_false, Bool.and_false, Bool.false_eq_true, ite_false, hU,
        ite_true, List.length_cons]
      have := filter_len_mono l (U := fun y => U y && y != a) (V := U)
        (fun y hy => by simp only [Bool.and_eq_true] at hy; exact hy.1)
      omega
    · have hs' : s ∈ l := by
        rcases List.mem_cons.1 hs with h | h
        · exact absurd h.symm has
        · exact h
      have hne : (a != s) = true := by simpa using has
      have := ih hs'
      cases hu : U a
      · simp only [Bool.false_and, Bool.false_eq_true, ite_false]; omega
      · simp only [hne, Bool.and_self, ite_true, List.length_cons]; omega

theorem cnt_erase {U : Nat → Bool} {s : Nat} (hs : s < cc.size) (hU : U s = true) :
    cnt cc (fun y => U y && y != s) + 1 ≤ cnt cc U :=
  filter_len_erase _ U s (List.mem_range.2 hs) hU

theorem foldlM_isSome {α : Type} (f : EntryAcc → α → Option EntryAcc) :
    ∀ (l : List α) (init : EntryAcc), (∀ acc x, x ∈ l → (f acc x).isSome) →
      (l.foldlM f init).isSome := by
  intro l
  induction l with
  | nil => intro init _; simp
  | cons a l ih =>
    intro init h
    rw [List.foldlM_cons]
    obtain ⟨b, hb⟩ := Option.isSome_iff_exists.1 (h init a List.mem_cons_self)
    rw [hb]
    exact ih b (fun acc x hx => h acc x (List.mem_cons_of_mem _ hx))

theorem bind_isSome {x : Option EntryAcc} {f : EntryAcc → Option EntryAcc} (hx : x.isSome)
    (hf : ∀ y, (f y).isSome) : (x >>= f).isSome := by
  obtain ⟨b, hb⟩ := Option.isSome_iff_exists.1 hx
  rw [hb]; exact hf b

/-- A proper descendant of `x` never has `x`'s ancestor `P` as parent. -/
theorem below_child_not {x P y : Nat} (hxP : cc.isDesc x P = true)
    (hyx : cc.isDesc y x = true) (hy : (cc.st y).parent = some P) : False := by
  rcases cc.parent_desc_or_eq hyx hy with rfl | h
  · exact absurd hxP (by simp [cc.isDesc_irrefl])
  · exact not_desc_both h hxP

theorem histValue_noHist {hist : List (Nat × List Nat)} (ho : HistOk cc hist) {h : Nat}
    {v : List Nat} (hv : CChart.histValue hist h = some v) {x : Nat} (hx : x ∈ v) :
    cc.isHistory x = false := by
  obtain ⟨_, p, S, _, hb, hdeep, hshallow⟩ := ho h v hv
  cases hd : cc.isDeepHistory h
  · exact hb.noHist x ((hshallow hd x).1 hx).1
  · exact hb.noHist x ((hdeep hd x).1 hx).1

theorem historyDefault_of (hw : CWF cc) {h : Nat} (hh : cc.isHistory h = true) :
    ∃ d, cc.historyDefault h = some d ∧ (cc.tr d).targets ≠ [] ∧
      ∀ x ∈ (cc.tr d).targets, ∀ p, (cc.st h).parent = some p → cc.isDesc x p = true := by
  obtain ⟨d, hd, hne, hdesc⟩ := hw.history_default h hh
  exact ⟨d, by simp [CChart.historyDefault, hd], hne, hdesc⟩

section Fuel

variable {hist : List (Nat × List Nat)} {fl : Nat} (r : Nat → Nat)

/-- The states an `addDesc` call may reach lie in `U`. -/
def RD (cc : CChart) (r : Nat → Nat) (s : Nat) (U : Nat → Bool) : Prop :=
  s < cc.size ∧ U s = true ∧
  (cc.isHistory s = false → ∀ y, cc.isDesc y s = true → U y = true) ∧
  (cc.isHistory s = true → ∀ P, (cc.st s).parent = some P → ∀ y, cc.isDesc y P = true →
    (cc.isHistory y = true → (cc.st y).parent = some P → r y < r s) → U y = true)

/-- The states an `addAnc` call stopping at `stop` may reach lie in `U`. -/
def AncOk (cc : CChart) (stop : Nat) (U : Nat → Bool) : Prop :=
  ∀ y, cc.isDesc y stop = true → (cc.isHistory y = true ∧ (cc.st y).parent = some stop → False) →
    U y = true

theorem rd_nonhist {x s : Nat} {U : Nat → Bool} (hx : cc.isHistory x = false)
    (hU : ∀ y, (y = x ∨ cc.isDesc y x = true) → U y = true) (hxs : x < cc.size)
    (hs : x ≠ s) (hs' : ∀ y, cc.isDesc y x = true → y ≠ s) :
    RD cc r x (fun y => U y && y != s) := by
  refine ⟨hxs, ?_, fun _ y hy => ?_, fun h => by rw [hx] at h; cases h⟩
  · simp [hU x (Or.inl rfl), hs]
  · simp [hU y (Or.inr hy), hs' y hy]

theorem fuel_adequate (hw : CWF cc) (ho : HistOk cc hist)
    (hr : ∀ h d x, cc.isHistory h = true → cc.historyDefault h = some d →
      x ∈ (cc.tr d).targets → cc.isHistory x = true → r x < r h) :
    ∀ n, (∀ s U acc, RD cc r s U → 2 * cnt cc U ≤ n → (cc.addDesc hist fl n s acc).isSome) ∧
      (∀ y stop U acc, cc.isDesc y stop = true → AncOk cc stop U → 2 * cnt cc U + 1 ≤ n →
        (cc.addAnc hist fl n y (some stop) acc).isSome) := by
  intro n
  induction n with
  | zero =>
    refine ⟨fun s U acc hrd hn => ?_, fun y stop U acc _ _ hn => by omega⟩
    have := cnt_erase (cc := cc) hrd.1 hrd.2.1
    omega
  | succ n ih =>
    obtain ⟨ihD, ihA⟩ := ih
    -- the loop over the children of a parallel state
    have loop : ∀ (A : Nat) (U : Nat → Bool) (acc : EntryAcc), 2 * cnt cc U ≤ n →
        (∀ c ∈ (cc.st A).children, RD cc r c U) →
        ((cc.st A).children.foldlM (fun acc c =>
          if (acc.toEnter.any fun x => cc.isDesc x c) = true then pure acc
          else cc.addDesc hist fl n c acc) acc).isSome := by
      intro A U acc hn hc
      apply foldlM_isSome
      intro acc c hcm
      split
      · simp
      · exact ihD c U acc (hc c hcm) hn
    have child_rd : ∀ (A c : Nat) (U : Nat → Bool), c ∈ (cc.st A).children →
        (∀ y, cc.isDesc y A = true → (cc.isHistory y = true ∧ (cc.st y).parent = some A → False)
          → U y = true) → RD cc r c U := by
      intro A c U hc hU
      obtain ⟨hcs, hcp, hch⟩ := (hw.children_iff A c).1 hc
      have hcA := hw.isDesc_parent hcp
      refine ⟨hcs, hU c hcA (fun h => by rw [hch] at h; cases h.1), fun _ y hy => ?_,
        fun h => by rw [hch] at h; cases h⟩
      exact hU y (cc.isDesc_trans hy hcA) (fun h => below_child_not hcA hy h.2)
    refine ⟨fun s U acc hrd hn => ?_, fun y stop U acc hys hanc hn => ?_⟩
    · obtain ⟨hs, hUs, hnh, hh⟩ := hrd
      have hcnt := cnt_erase (cc := cc) hs hUs
      rw [CChart.addDesc]
      split
      · rename_i hhs
        obtain ⟨P, hP, _⟩ := hw.history_parent s hhs
        have hpar : (cc.st s).parent.getD 0 = P := by rw [hP]; rfl
        have hU := hh hhs P hP
        have hanc' : AncOk cc P (fun y => U y && y != s) := by
          intro y hy hn'
          have : U y = true := hU y hy (fun h1 h2 => (hn' ⟨h1, h2⟩).elim)
          have hne : y ≠ s := by rintro rfl; exact hn' ⟨hhs, hP⟩
          simp [this, hne]
        -- a non-history state below `P`
        have nh_rd : ∀ x, cc.isDesc x P = true → cc.isHistory x = false →
            RD cc r x (fun y => U y && y != s) := by
          intro x hxP hxh
          refine rd_nonhist r hxh (fun y hy => ?_) (cc.isDesc_lt_size hxP) ?_ ?_
          · rcases hy with rfl | hy
            · exact hU y hxP (fun h => by rw [hxh] at h; cases h)
            · exact hU y (cc.isDesc_trans hy hxP) (fun _ h => (below_child_not hxP hy h).elim)
          · rintro rfl; rw [hhs] at hxh; cases hxh
          · rintro y hy rfl; exact below_child_not hxP hy hP
        split
        · rename_i v hv
          simp only [hpar]
          apply bind_isSome
          · apply foldlM_isSome
            intro acc x hx
            exact ihD x (fun y => U y && y != s) acc (nh_rd x ((hw.histValue_ne ho hv).2 P hP x hx)
              (histValue_noHist ho hv hx)) (by omega)
          · intro acc
            apply foldlM_isSome
            intro acc x hx
            exact ihA x P (fun y => U y && y != s) acc ((hw.histValue_ne ho hv).2 P hP x hx) hanc' (by omega)
        · rename_i hv
          split
          · rename_i d0 hd0
            simp only [hpar]
            obtain ⟨d1, hd1, _, hdesc⟩ := historyDefault_of hw hhs
            rw [hd0] at hd1; cases hd1
            apply bind_isSome
            · apply foldlM_isSome
              intro acc x hx
              have hxP := hdesc x hx P hP
              refine ihD x (fun y => U y && y != s) acc ?_ (by omega)
              cases hxh : cc.isHistory x
              · exact nh_rd x hxP hxh
              · have hrx := hr s d0 x hhs hd0 hx hxh
                refine ⟨cc.isDesc_lt_size hxP, ?_, ?_, ?_⟩
                · have : U x = true := hU x hxP (fun _ _ => hrx)
                  have hne : x ≠ s := by rintro rfl; omega
                  simp [this, hne]
                · intro h; rw [hxh] at h; cases h
                · intro _ P2 hP2 y hy hcond
                  have hyP : cc.isDesc y P = true := by
                    rcases cc.parent_desc_or_eq hxP hP2 with rfl | h
                    · exact hy
                    · exact cc.isDesc_trans hy h
                  have hcond' : cc.isHistory y = true → (cc.st y).parent = some P → r y < r s := by
                    intro h1 h2
                    rcases cc.parent_desc_or_eq hxP hP2 with rfl | h
                    · have := hcond h1 h2; omega
                    · exact (below_child_not h hy h2).elim
                  have : U y = true := hU y hyP hcond'
                  have hne : y ≠ s := by
                    rintro rfl; have := hcond' hhs hP; omega
                  simp [this, hne]
            · intro acc
              apply foldlM_isSome
              intro acc x hx
              exact ihA x P (fun y => U y && y != s) acc (hdesc x hx P hP) hanc' (by omega)
          · simp
      · rename_i hhs
        have hhs' : cc.isHistory s = false := by simpa using hhs
        have hU := hnh hhs'
        have below_rd : ∀ x, cc.isDesc x s = true → RD cc r x (fun y => U y && y != s) := by
          intro x hxs
          have hne : ∀ y, cc.isDesc y s = true → y ≠ s := by
            rintro y hy rfl; exact absurd hy (by simp [cc.isDesc_irrefl])
          refine ⟨cc.isDesc_lt_size hxs, by simp [hU x hxs, hne x hxs], fun _ y hy => ?_,
            fun _ P2 hP2 y hy _ => ?_⟩
          · have := cc.isDesc_trans hy hxs; simp [hU y this, hne y this]
          · have : cc.isDesc y s = true := by
              rcases cc.parent_desc_or_eq hxs hP2 with rfl | h
              · exact hy
              · exact cc.isDesc_trans hy h
            simp [hU y this, hne y this]
        split
        · apply bind_isSome
          · apply foldlM_isSome
            intro acc x hx
            refine ihD x (fun y => U y && y != s) acc (below_rd x ?_) (by omega)
            split at hx
            · rename_i i hi
              exact ((hw.initial_ok s i hi).2.2.2) x hx
            · simp at hx
          · intro acc
            apply foldlM_isSome
            intro acc x hx
            refine ihA x s (fun y => U y && y != s) acc ?_ (fun y hy _ => ?_) (by omega)
            · split at hx
              · rename_i i hi
                exact ((hw.initial_ok s i hi).2.2.2) x hx
              · simp at hx
            · have hne : y ≠ s := by
                rintro rfl; exact absurd hy (by simp [cc.isDesc_irrefl])
              simp [hU y hy, hne]
        · split
          · exact loop s (fun y => U y && y != s) _ (by omega) (fun c hc => below_rd c (hw.child_isDesc hc))
          · simp
    · rw [CChart.addAnc]
      apply foldlM_isSome
      intro acc a ha
      have ha' := (List.mem_filter.1 ha).1
      have hast : cc.isDesc a stop = true := ((mem_ancestorsBelow y stop a hys).1 ha').2
      simp only
      split
      · refine loop a U _ (by omega) (fun c hc => child_rd a c U hc (fun z hz hn' => ?_))
        refine hanc z (cc.isDesc_trans hz hast) (fun h => ?_)
        exact below_child_not hast hz h.2
      · simp

end Fuel

theorem effectiveTargets_nil {hist : List (Nat × List Nat)} {t : Nat}
    (h : (cc.tr t).targets = []) : cc.effectiveTargets hist t = [] := by
  simp [CChart.effectiveTargets, CChart.effTargetsF, h]

/-- **Fuel adequacy of `entrySet`.** -/
theorem entrySet_isSome (hw : CWF cc) {hist dhist : List (Nat × List Nat)} (ho : HistOk cc hist)
    {ts : List Nat}
    (hdom : ∀ t ∈ ts, (cc.tr t).targets ≠ [] →
      ∃ d, cc.domain dhist t = some d ∧ ∀ e ∈ cc.effectiveTargets hist t, cc.isDesc e d = true) :
    (cc.entrySet hist dhist ts).isSome := by
  obtain ⟨r, _, hr⟩ := hw.history_rank
  have hall : cnt cc (fun _ => true) = cc.size := by
    have : ∀ l : List Nat, (l.filter (fun _ => true)).length = l.length := by
      intro l; induction l <;> simp_all
    simp only [cnt, this, List.length_range]
  unfold CChart.entrySet
  apply foldlM_isSome
  intro acc t ht
  apply bind_isSome
  · apply foldlM_isSome
    intro acc x hx
    obtain ⟨_, hxs⟩ := hw.targets_lt t x hx
    refine ((fuel_adequate r hw ho hr _).1 x _ acc ⟨hxs, rfl, fun _ _ _ => rfl,
      fun _ _ _ _ _ _ => rfl⟩ ?_)
    rw [hall]; unfold CChart.entryFuel; omega
  · intro acc
    by_cases hne : (cc.tr t).targets = []
    · rw [effectiveTargets_nil hne]; simp
    · obtain ⟨d, hd, he⟩ := hdom t ht hne
      apply foldlM_isSome
      intro acc e hem
      rw [hd]
      refine (fuel_adequate r hw ho hr _).2 e d _ acc (he e hem) (fun _ _ _ => rfl) ?_
      rw [hall]; unfold CChart.entryFuel; omega

end Vertep.Entry
