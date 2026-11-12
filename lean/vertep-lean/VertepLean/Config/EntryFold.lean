import VertepLean.Config.EntrySteps

/-!
# Entry lemma: the loops over targets and ancestors

`addAnc` from one call (`sa_step`), a sequence of `addAnc` calls (`af_spec`), a sequence of
`addDesc` calls on pairwise compatible targets (`t1_spec`) and the two together, the shape of
`addDesc` on a compound state, a history value and a history default (`tt_spec`).
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat}

/-! ## `addAnc` -/

theorem sa_step (hw : CWF cc) {n : Nat} (hSD : SD cc hist d n) : SA cc hist d (n + 1) := by
  intro y D G acc acc' hc hyD hg hpc hanch
  rw [CChart.addAnc] at hc
  have hL : ∀ a, a ∈ ancL cc d y (some D) ↔ cc.isDesc y a = true ∧ cc.isDesc a G = true :=
    fun a => mem_ancL hyD hg
  -- the fold over the ancestors
  have key : ∀ (l : List Nat) (acc acc' : EntryAcc), (∀ a ∈ l, a ∈ ancL cc d y (some D)) →
      l.foldlM (fun acc anc =>
        have acc := acc.add anc
        if cc.isParallel anc = true then
          (cc.st anc).children.foldlM (fun acc c =>
            if (acc.toEnter.any fun x => cc.isDesc x c) = true then pure acc
            else cc.addDesc hist d n c acc) acc
        else pure acc) acc = some acc' →
      PC cc d acc.toEnter → Anch cc acc.toEnter G y →
      PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
      (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧
      (∀ a ∈ l, a ∈ acc'.toEnter ∧ (cc.isParallel a = true → ∀ c ∈ (cc.st a).children,
        (∃ z ∈ acc'.toEnter, cc.isDesc z c = true) ∨ DoneD cc hist d acc'.toEnter n c)) ∧
      (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → cc.isHistory w = false ∧ C34 cc acc'.toEnter w ∧
        ∀ q, (cc.st w).parent = some q → q = G ∨ q ∈ acc'.toEnter ∨
          q ∈ ancL cc d y (some D)) := by
    intro l
    induction l with
    | nil =>
      intro acc acc' _ h hpc _
      rw [foldlM_nil_some h]
      exact ⟨hpc, fun x hx => hx, fun x hx => Or.inl hx, fun a ha => by simp at ha,
        fun w hw hn => absurd hw hn⟩
    | cons a l ih =>
      intro acc acc' hl h hpc hanc
      obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
      obtain ⟨hya, haG⟩ := (hL a).1 (hl a List.mem_cons_self)
      have had := in_of_G hg haG
      obtain ⟨z, hz, hza⟩ := hanc a hya haG
      have hpc1 := pc_add (s := a) hpc (fits_anc hpc hz hza (cc.isDesc_trans hza had))
      have ha1 : a ∈ (acc.add a).toEnter := (mem_add acc a a).2 (Or.inr rfl)
      have hsub1 : ∀ x ∈ acc.toEnter, x ∈ (acc.add a).toEnter :=
        fun x hx => (mem_add acc a x).2 (Or.inl hx)
      have hah : cc.isHistory a = false := by
        cases h' : cc.isHistory a
        · rfl
        · rw [hw.not_isDesc_history h'] at hya; cases hya
      -- the step on `a`
      have step : PC cc d b.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ b.toEnter) ∧
          (∀ x ∈ b.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧ a ∈ b.toEnter ∧
          (cc.isParallel a = true → ∀ c ∈ (cc.st a).children,
            Touched cc b.toEnter c ∧
            ((∃ z ∈ b.toEnter, cc.isDesc z c = true) ∨ DoneD cc hist d b.toEnter n c)) ∧
          (∀ w ∈ b.toEnter, w ∉ acc.toEnter → w ≠ a → Cond cc b.toEnter a w) := by
        simp only at hb
        split at hb
        · rename_i hk
          obtain ⟨l1, l2, l3, l4, l5⟩ := loop_spec hw hSD a (cc.st a).children (acc.add a) b
            (fun c hc => hc) hb hk ha1 had hpc1
          refine ⟨l1, fun x hx => l2 x (hsub1 x hx), fun x hx => ?_, l2 a ha1,
            fun _ c hc => l5 c hc, fun w hwb hwn hwa => ?_⟩
          · rcases l3 x hx with h' | h'
            · rcases (mem_add acc a x).1 h' with h'' | rfl
              · exact Or.inl h''
              · exact Or.inr haG
            · exact Or.inr (cc.isDesc_trans h' haG)
          · refine l4 w hwb (fun h' => ?_)
            rcases (mem_add acc a w).1 h' with h'' | h''
            · exact hwn h''
            · exact hwa h''
        · rename_i hk
          simp at hb; subst hb
          refine ⟨hpc1, hsub1, fun x hx => ?_, ha1, fun h' => absurd h' hk, fun w hwb hwn hwa => ?_⟩
          · rcases (mem_add acc a x).1 hx with h'' | rfl
            · exact Or.inl h''
            · exact Or.inr haG
          · rcases (mem_add acc a w).1 hwb with h'' | h''
            · exact absurd h'' hwn
            · exact absurd h'' hwa
      obtain ⟨s1, s2, s3, s4, s5, s6⟩ := step
      obtain ⟨i1, i2, i3, i4, i5⟩ := ih b acc' (fun a' ha' => hl a' (List.mem_cons_of_mem _ ha'))
        hr s1 (anch_mono s2 hanc)
      refine ⟨i1, fun x hx => i2 x (s2 x hx), fun x hx => ?_, fun a' ha' => ?_,
        fun w hw' hwn => ?_⟩
      · rcases i3 x hx with h' | h'
        · exact s3 x h'
        · exact Or.inr h'
      · rcases List.mem_cons.1 ha' with rfl | ha'
        · refine ⟨i2 _ s4, fun hk c hc => ?_⟩
          rcases (s5 hk c hc).2 with ⟨z', hz', h'⟩ | h'
          · exact Or.inl ⟨z', i2 z' hz', h'⟩
          · exact Or.inr (doneD_mono i2 h')
        · exact i4 a' ha'
      · by_cases hwb : w ∈ b.toEnter
        · by_cases hwa : w = a
          · subst hwa
            refine ⟨hah, ⟨fun _ => ⟨z, i2 z (s2 z hz), hza⟩, fun hk c hc =>
              touched_mono i2 (s5 hk c hc).1⟩, fun q hq => ?_⟩
            rcases cc.parent_desc_or_eq haG hq with rfl | hqG
            · exact Or.inl rfl
            · exact Or.inr (Or.inr ((hL q).2 ⟨cc.isDesc_trans hya (hw.isDesc_parent hq), hqG⟩))
          · obtain ⟨c1, c2, c3⟩ := s6 w hwb hwn hwa
            refine ⟨c1, c34_mono i2 c3, fun q hq => ?_⟩
            rcases c2 q hq with rfl | h'
            · exact Or.inr (Or.inl (i2 _ s4))
            · exact Or.inr (Or.inl (i2 q h'))
        · exact i5 w hw' hwb
  obtain ⟨k1, k2, k3, k4, k5⟩ := key _ acc acc' (fun a ha => ha) hc hpc hanch
  refine ⟨k1, k2, k3, fun a h1 h2 => (k4 a ((hL a).2 ⟨h1, h2⟩)).1, fun w hw' hwn => ?_, ?_⟩
  · obtain ⟨c1, c3, c2⟩ := k5 w hw' hwn
    refine ⟨c1, fun q hq => ?_, c3⟩
    rcases c2 q hq with h' | h' | h'
    · exact Or.inl h'
    · exact Or.inr h'
    · exact Or.inr (k4 q h').1
  · rw [DoneA]
    exact k4

/-- A sequence of `addAnc` calls. -/
theorem af_spec {n : Nat} (hSA : SA cc hist d n) {D G : Nat} (hg : Hg cc d D G) :
    ∀ (L : List Nat) (acc acc' : EntryAcc),
      L.foldlM (fun acc y => cc.addAnc hist d n y (some D) acc) acc = some acc' →
      (∀ y ∈ L, cc.isDesc y D = true) → PC cc d acc.toEnter →
      (∀ y ∈ L, Anch cc acc.toEnter G y) →
      PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
      (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧
      (∀ y ∈ L, ∀ a, cc.isDesc y a = true → cc.isDesc a G = true → a ∈ acc'.toEnter) ∧
      (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → Cond cc acc'.toEnter G w) ∧
      (∀ y ∈ L, DoneA cc hist d acc'.toEnter n y (some D)) := by
  intro L
  induction L with
  | nil =>
    intro acc acc' h _ hpc _
    rw [foldlM_nil_some h]
    exact ⟨hpc, fun x hx => hx, fun x hx => Or.inl hx, fun y hy => by simp at hy,
      fun w hw hn => absurd hw hn, fun y hy => by simp at hy⟩
  | cons y L ih =>
    intro acc acc' h hD hpc hanc
    obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
    obtain ⟨s1, s2, s3, s4, s5, s6⟩ := hSA y D G acc b hb (hD y List.mem_cons_self) hg hpc
      (hanc y List.mem_cons_self)
    obtain ⟨i1, i2, i3, i4, i5, i6⟩ := ih b acc' hr (fun y' hy' => hD y' (List.mem_cons_of_mem _ hy'))
      s1 (fun y' hy' => anch_mono s2 (hanc y' (List.mem_cons_of_mem _ hy')))
    refine ⟨i1, fun x hx => i2 x (s2 x hx), fun x hx => ?_, fun y' hy' a h1 h2 => ?_,
      fun w hw hwn => ?_, fun y' hy' => ?_⟩
    · rcases i3 x hx with h' | h'
      · exact s3 x h'
      · exact Or.inr h'
    · rcases List.mem_cons.1 hy' with rfl | hy'
      · exact i2 a (s4 a h1 h2)
      · exact i4 y' hy' a h1 h2
    · by_cases hwb : w ∈ b.toEnter
      · exact cond_mono i2 (s5 w hwb hwn)
      · exact i5 w hw hwb
    · rcases List.mem_cons.1 hy' with rfl | hy'
      · exact doneA_mono i2 s6
      · exact i6 y' hy'

end Vertep.Entry
