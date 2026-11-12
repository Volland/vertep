import VertepLean.Config.EntryTargets

/-!
# Entry lemma: the contracts hold

`tt_spec` puts together the `addDesc` calls and the `addAnc` calls on one list of targets;
`sd_step`, `sh_step` and `sa_step` prove the contracts `SD`, `SH`, `SA` for one more unit of
fuel, and `contracts` for every fuel by induction.
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat} {n : Nat}

theorem pairwise_of_all {R : Nat → Nat → Prop} :
    ∀ {l : List Nat}, (∀ x ∈ l, ∀ y ∈ l, R x y) → List.Pairwise R l
  | [], _ => List.Pairwise.nil
  | a :: _, h => List.pairwise_cons.2 ⟨fun y hy => h a List.mem_cons_self y (List.mem_cons_of_mem _ hy),
      pairwise_of_all (fun x hx y hy => h x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy))⟩

theorem t1_spec (hw : CWF cc) (ho : HistOk cc hist) (hSD : SD cc hist d n) (hSH : SH cc hist d n)
    (hId : ∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc')
    {G : Nat} (hGd : G = d ∨ cc.isDesc G d = true) (T : List Nat) (acc acc' : EntryAcc)
    (hc : T.foldlM (fun acc x => cc.addDesc hist d n x acc) acc = some acc')
    (hok : ∀ x ∈ T, TgtOk cc hist d G x) (hpw : T.Pairwise (TR cc)) (hpc : PC cc d acc.toEnter)
    (hf : Fits cc d acc.toEnter G) (hn : ∀ z ∈ acc.toEnter, cc.isDesc z G = false) :
    T1Inv cc hist d n G T acc.toEnter T acc'.toEnter := by
  have := t1_aux hw ho hSD hSH hId hGd hok hf hn T [] acc acc' hc (fun x hx => hx)
    (fun x hx => by simp at hx) (by simpa using hpw)
    ⟨hpc, fun x hx => hx, fun w hw' hw0 => absurd hw' hw0, fun x hx => by simp at hx⟩
  simpa using this

/-- Entering a list of targets below `D`: the `addDesc` calls, then the `addAnc` calls. -/
theorem tt_spec (hw : CWF cc) (ho : HistOk cc hist) (hSD : SD cc hist d n) (hSH : SH cc hist d n)
    (hSA : SA cc hist d n)
    (hId : ∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc')
    {D G : Nat} (hg : Hg cc d D G) (T : List Nat) (acc acc' : EntryAcc)
    (hc : (do let acc ← T.foldlM (fun acc x => cc.addDesc hist d n x acc) acc
              T.foldlM (fun acc x => cc.addAnc hist d n x (some D) acc) acc) = some acc')
    (hT : ∀ x ∈ T, cc.isDesc x D = true) (hok : ∀ x ∈ T, TgtOk cc hist d G x)
    (hpw : T.Pairwise (TR cc)) (hpc : PC cc d acc.toEnter) (hf : Fits cc d acc.toEnter G)
    (hn : ∀ z ∈ acc.toEnter, cc.isDesc z G = false) :
    PC cc d acc'.toEnter ∧ (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
    (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x G = true) ∧
    (∀ w ∈ acc'.toEnter, w ∉ acc.toEnter → Cond cc acc'.toEnter G w) ∧
    (T ≠ [] → ∃ z ∈ acc'.toEnter, cc.isDesc z G = true) ∧
    (∀ x ∈ T, DoneD cc hist d acc'.toEnter n x ∧ DoneA cc hist d acc'.toEnter n x (some D) ∧
      ∀ e, EffOf cc hist x e → e ∈ acc'.toEnter) := by
  obtain ⟨b, hb, hr⟩ := bind_some hc
  obtain ⟨p1, p2, p3, p4⟩ := t1_spec hw ho hSD hSH hId (hg_in hg) T acc b hb hok hpw hpc hf hn
  obtain ⟨a1, a2, a3, a4, a5, a6⟩ := af_spec hSA hg T b acc' hr hT p1
    (fun y hy => (p4 y hy).2.2.1)
  refine ⟨a1, fun x hx => a2 x (p2 x hx), fun x hx => ?_, fun w hw' hwn => ?_, fun hne => ?_,
    fun x hx => ⟨doneD_mono a2 (p4 x hx).1, a6 x hx, fun e he => a2 e ((p4 x hx).2.1 e he)⟩⟩
  · rcases a3 x hx with h | h
    · by_cases hxa : x ∈ acc.toEnter
      · exact Or.inl hxa
      · exact Or.inr (p3 x h hxa).1
    · exact Or.inr h
  · by_cases hwb : w ∈ b.toEnter
    · obtain ⟨_, c1, c2, c3, _⟩ := p3 w hwb hwn
      refine ⟨c1, fun q hq => ?_, c34_mono a2 c2⟩
      rcases c3 q hq with h | h | ⟨hqG, x, hx, hxq⟩
      · exact Or.inl h
      · exact Or.inr (a2 q h)
      · exact Or.inr (a4 x hx q hxq hqG)
    · exact a5 w hw' hwb
  · obtain ⟨x, hx⟩ := List.exists_mem_of_ne_nil _ hne
    obtain ⟨z, hz, hzG⟩ := (p4 x hx).2.2.2
    exact ⟨z, a2 z hz, hzG⟩

theorem sd_step (hw : CWF cc) (ho : HistOk cc hist) (hSD : SD cc hist d n) (hSH : SH cc hist d n)
    (hSA : SA cc hist d n)
    (hId : ∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc') : SD cc hist d (n + 1) := by
  intro s acc acc' hc hnh hsd hpc hfit hnb
  rw [CChart.addDesc] at hc
  simp only [hnh, Bool.false_eq_true, ↓reduceIte] at hc
  have hpc1 := pc_add hpc hfit
  have hs1 : s ∈ (acc.add s).toEnter := (mem_add acc s s).2 (Or.inr rfl)
  have hsub1 : ∀ x ∈ acc.toEnter, x ∈ (acc.add s).toEnter :=
    fun x hx => (mem_add acc s x).2 (Or.inl hx)
  have hfit1 : Fits cc d (acc.add s).toEnter s := by
    intro w hw' hwd
    rcases (mem_add acc s w).1 hw' with h | rfl
    · exact hfit w h hwd
    · exact rel_refl _
  have hnb1 : ∀ z ∈ (acc.add s).toEnter, cc.isDesc z s = false := by
    intro z hz
    rcases (mem_add acc s z).1 hz with h | rfl
    · exact hnb z h
    · exact cc.isDesc_irrefl _
  split at hc
  · -- compound
    rename_i hk
    have hcr := hw.initial_iff s
    rw [CWF.isCompoundOrRoot_of_compound hk] at hcr
    obtain ⟨i, hi⟩ := Option.isSome_iff_exists.1 hcr
    obtain ⟨_, _, hne, hdesc⟩ := hw.initial_ok s i hi
    simp only [hi] at hc
    have hok : ∀ x ∈ (cc.tr i).targets, TgtOk cc hist d s x := by
      intro x hx
      have hxs := hdesc x hx
      cases hxh : cc.isHistory x
      · exact Or.inl ⟨hxh, hxs⟩
      · obtain ⟨P, hP, _⟩ := hw.history_parent x hxh
        refine Or.inr ⟨hxh, P, hP, ?_⟩
        rcases cc.parent_desc_or_eq hxs hP with h | h
        · exact Or.inl h
        · exact Or.inr (Or.inl h)
    obtain ⟨t1, t2, t3, t4, t5, t6⟩ := tt_spec hw ho hSD hSH hSA hId (Or.inl ⟨rfl, Or.inr hsd⟩)
      _ _ acc' hc hdesc hok ((hw.targets_legal i).imp (fun h => Or.inr h))
      (by rw [toEnter_ite]; exact hpc1) (by rw [toEnter_ite]; exact hfit1)
      (by rw [toEnter_ite]; exact hnb1)
    rw [toEnter_ite] at t2 t3 t4
    refine ⟨t1, fun x hx => t2 x (hsub1 x hx), fun x hx => ?_, t2 s hs1,
      fun w hw' hwn hws => t4 w hw' (fun h => ?_), ⟨fun _ => t5 hne, fun hp => ?_⟩, ?_⟩
    · rcases t3 x hx with h | h
      · rcases (mem_add acc s x).1 h with h' | h'
        · exact Or.inl h'
        · exact Or.inr (Or.inl h')
      · exact Or.inr (Or.inr h)
    · rcases (mem_add acc s w).1 h with h' | h'
      · exact hwn h'
      · exact hws h'
    · rw [compound_not_parallel (CWF.isCompoundOrRoot_of_compound hk)] at hp; cases hp
    · rw [DoneD]
      simp only [hnh, Bool.false_eq_true, ↓reduceIte]
      refine ⟨t2 s hs1, fun _ x hx => ?_, fun hp => ?_⟩
      · simp only [initT, hi] at hx
        exact ⟨(t6 x hx).1, (t6 x hx).2.1⟩
      · rw [compound_not_parallel (CWF.isCompoundOrRoot_of_compound hk)] at hp; cases hp
  · rename_i hk
    have hk' : cc.isCompound s = false := by simpa using hk
    split at hc
    · -- parallel
      rename_i hp
      obtain ⟨l1, l2, l3, l4, l5⟩ := loop_spec hw hSD s (cc.st s).children (acc.add s) acc'
        (fun c hc => hc) hc hp hs1 hsd hpc1
      refine ⟨l1, fun x hx => l2 x (hsub1 x hx), fun x hx => ?_, l2 s hs1,
        fun w hw' hwn hws => l4 w hw' (fun h => ?_), ⟨?_, fun _ c hc => (l5 c hc).1⟩, ?_⟩
      · rcases l3 x hx with h | h
        · rcases (mem_add acc s x).1 h with h' | h'
          · exact Or.inl h'
          · exact Or.inr (Or.inl h')
        · exact Or.inr (Or.inr h)
      · rcases (mem_add acc s w).1 h with h' | h'
        · exact hwn h'
        · exact hws h'
      · intro h; rw [hk'] at h; cases h
      · rw [DoneD]
        simp only [hnh, Bool.false_eq_true, ↓reduceIte]
        refine ⟨l2 s hs1, ?_, fun _ c hc => (l5 c hc).2⟩
        intro h; rw [hk'] at h; cases h
    · -- atomic
      rename_i hp
      have hp' : cc.isParallel s = false := by simpa using hp
      simp at hc; subst hc
      refine ⟨hpc1, hsub1, fun x hx => ?_, hs1, fun w hw' hwn hws => ?_, ⟨?_, ?_⟩, ?_⟩
      · rcases (mem_add acc s x).1 hx with h | h
        · exact Or.inl h
        · exact Or.inr (Or.inl h)
      · rcases (mem_add acc s w).1 hw' with h | h
        · exact absurd h hwn
        · exact absurd h hws
      · intro h; rw [hk'] at h; cases h
      · intro h; rw [hp'] at h; cases h
      · rw [DoneD]
        simp only [hnh, Bool.false_eq_true, ↓reduceIte]
        refine ⟨hs1, ?_, ?_⟩
        · intro h; rw [hk'] at h; cases h
        · intro h; rw [hp'] at h; cases h

theorem sh_step (hw : CWF cc) (ho : HistOk cc hist) (hSD : SD cc hist d n) (hSH : SH cc hist d n)
    (hSA : SA cc hist d n)
    (hId : ∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc') : SH cc hist d (n + 1) := by
  intro h P G acc acc' hc hh hP hg heff hpc hfit hnb
  have hpar : (cc.st h).parent.getD 0 = P := by rw [hP]; rfl
  rw [CChart.addDesc] at hc
  simp only [hh, ↓reduceIte] at hc
  split at hc
  · -- a recorded value
    rename_i v hv
    rw [hpar] at hc
    have hvne : v ≠ [] := by
      obtain ⟨x, hx⟩ := (hw.histValue_ne ho hv).1
      exact List.ne_nil_of_mem hx
    have hpw : v.Pairwise (TR cc) := by
      apply pairwise_of_all
      intro x hx y hy
      by_cases hxy : x = y
      · exact Or.inl hxy
      · refine Or.inr ?_
        rw [pos_nonhist (histValue_noHist ho hv hx), pos_nonhist (histValue_noHist ho hv hy)]
        exact histValue_compat hw ho hv hx hy hxy
    obtain ⟨t1, t2, t3, t4, t5, t6⟩ := tt_spec hw ho hSD hSH hSA hId hg v acc acc' hc
      (fun x hx => (hw.histValue_ne ho hv).2 P hP x hx)
      (fun x hx => Or.inl ⟨histValue_noHist ho hv hx, heff x (Or.inl ⟨v, hv, hx⟩)⟩)
      hpw hpc hfit hnb
    refine ⟨t1, t2, t3, t4, t5 hvne, fun e he => ?_, ?_⟩
    · rcases he with ⟨v', hv', he'⟩ | ⟨hv', _⟩
      · rw [hv] at hv'; cases hv'
        exact (t6 e he').2.2 e (Or.inl ⟨histValue_noHist ho hv he', rfl⟩)
      · rw [hv] at hv'; cases hv'
    · rw [DoneD]
      simp only [hh, ↓reduceIte, hv, hpar]
      exact fun x hx => ⟨(t6 x hx).1, (t6 x hx).2.1⟩
  · rename_i hv
    obtain ⟨d0, hd0, hne, hdesc⟩ := historyDefault_of hw hh
    simp only [hd0] at hc
    rw [hpar] at hc
    have hx2h : ∀ x ∈ (cc.tr d0).targets, cc.isHistory x = true →
        ∀ e, EffOfH cc hist x e → EffOfH cc hist h e :=
      fun x hx hxh e he => Or.inr ⟨hv, d0, hd0, effMem_iff.2 ⟨x, hx, Or.inr ⟨hxh, he⟩⟩⟩
    have hok : ∀ x ∈ (cc.tr d0).targets, TgtOk cc hist d G x := by
      intro x hx
      cases hxh : cc.isHistory x
      · exact Or.inl ⟨hxh, heff x (Or.inr ⟨hv, d0, hd0, EffMem.plain hx hxh⟩)⟩
      · obtain ⟨P2, hP2, _⟩ := hw.history_parent x hxh
        refine Or.inr ⟨hxh, P2, hP2, ?_⟩
        rcases hg with ⟨hGP, _⟩ | ⟨hGd, _⟩
        · rw [hGP]
          rcases cc.parent_desc_or_eq (hdesc x hx P hP) hP2 with h' | h'
          · exact Or.inl h'
          · exact Or.inr (Or.inl h')
        · obtain ⟨e, he, heP2⟩ := effOfH_exists hw ho hxh hP2
          have hed := heff e (hx2h x hx hxh e he)
          rcases cc.isDesc_chain heP2 hed with h' | h' | h'
          · exact Or.inl h'
          · exact Or.inr (Or.inl h')
          · refine Or.inr (Or.inr ⟨hGd, by rw [← hGd]; exact h', fun e' he' => ?_⟩)
            rw [← hGd]; exact heff e' (hx2h x hx hxh e' he')
    obtain ⟨t1, t2, t3, t4, t5, t6⟩ := tt_spec hw ho hSD hSH hSA hId hg _ _ acc' hc
      (fun x hx => hdesc x hx P hP) hok ((hw.targets_legal d0).imp (fun h => Or.inr h))
      hpc hfit hnb
    refine ⟨t1, t2, t3, t4, t5 hne, fun e he => ?_, ?_⟩
    · rcases he with ⟨v', hv', _⟩ | ⟨_, d1, hd1, hm⟩
      · rw [hv] at hv'; cases hv'
      · rw [hd0] at hd1; cases hd1
        obtain ⟨x, hx, hxe⟩ := effMem_iff.1 hm
        exact (t6 x hx).2.2 e hxe
    · rw [DoneD]
      simp only [hh, ↓reduceIte, hv, hd0, hpar]
      exact fun x hx => ⟨(t6 x hx).1, (t6 x hx).2.1⟩

/-- **The contracts of `addDesc` and `addAnc`**, for every fuel. -/
theorem contracts (hw : CWF cc) (ho : HistOk cc hist) :
    ∀ n, SD cc hist d n ∧ SH cc hist d n ∧ SA cc hist d n := by
  intro n
  induction n with
  | zero =>
    refine ⟨fun s acc acc' hc => ?_, fun h P G acc acc' hc => ?_, fun y D G acc acc' hc => ?_⟩
    · simp [CChart.addDesc] at hc
    · simp [CChart.addDesc] at hc
    · simp [CChart.addAnc] at hc
  | succ n ih =>
    obtain ⟨hSD, hSH, hSA⟩ := ih
    have hId := (idem (cc := cc) (hist := hist) (d := d) n).1
    exact ⟨sd_step hw ho hSD hSH hSA hId, sh_step hw ho hSD hSH hSA hId, sa_step hw hSD⟩

end Vertep.Entry
