import VertepLean.Config.EntryFold

/-!
# Entry lemma: entering a list of targets

`t1_spec`: `addDesc` on each of a list of targets whose positions are pairwise compatible (a
target listed twice is entered once: the second call changes nothing, `idem`), each below the
root `G` or a history state whose parent lies at or above `G`. `tt_spec` adds the `addAnc`
calls on the same targets: the result is complete below `G`.
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat}

section Defs

variable (cc : CChart) (hist : List (Nat × List Nat)) (d : Nat)

/-- A target entered below the root `G`: a non-history state below `G`, or a history state
whose parent is `G`, below `G`, or above `G` when `G` is the floor and its effective targets lie
below the floor. -/
def TgtOk (G x : Nat) : Prop :=
  (cc.isHistory x = false ∧ cc.isDesc x G = true) ∨
  (cc.isHistory x = true ∧ ∃ P, (cc.st x).parent = some P ∧
    (P = G ∨ cc.isDesc P G = true ∨
      (G = d ∧ cc.isDesc d P = true ∧ ∀ e, EffOfH cc hist x e → cc.isDesc e d = true)))

/-- The root below which entering a target adds states. -/
def RootOf (G x r : Nat) : Prop :=
  (cc.isHistory x = false ∧ r = x) ∨
  (cc.isHistory x = true ∧ ∃ P, (cc.st x).parent = some P ∧
    ((r = P ∧ (P = G ∨ cc.isDesc P G = true)) ∨ (r = G ∧ G = d ∧ cc.isDesc d P = true)))

/-- Two targets of a legal target set. -/
def TR (x y : Nat) : Prop := x = y ∨ cc.Compatible (cc.pos x) (cc.pos y)

/-- What entering a target has achieved. -/
def PerX (n G : Nat) (E : List Nat) (x : Nat) : Prop :=
  DoneD cc hist d E n x ∧ (∀ e, EffOf cc hist x e → e ∈ E) ∧ Anch cc E G x ∧
    ∃ z ∈ E, cc.isDesc z G = true

/-- The invariant of entering targets one by one: `pre` are entered, `E0` was there before. -/
def T1Inv (n G : Nat) (T E0 pre E : List Nat) : Prop :=
  PC cc d E ∧ (∀ x ∈ E0, x ∈ E) ∧
  (∀ w ∈ E, w ∉ E0 → cc.isDesc w G = true ∧ cc.isHistory w = false ∧ C34 cc E w ∧
    (∀ q, (cc.st w).parent = some q → q = G ∨ q ∈ E ∨
      (cc.isDesc q G = true ∧ ∃ x ∈ T, cc.isDesc x q = true)) ∧
    ∃ x ∈ pre, ∃ r, RootOf cc d G x r ∧ (w = r ∨ cc.isDesc w r = true)) ∧
  (∀ x ∈ pre, PerX cc hist d n G E x)

end Defs

theorem pos_hist {x P : Nat} (hh : cc.isHistory x = true) (hP : (cc.st x).parent = some P) :
    cc.pos x = P := by
  simp [CChart.pos, hh, hP]

theorem pos_nonhist {x : Nat} (hh : cc.isHistory x = false) : cc.pos x = x := by
  simp [CChart.pos, hh]

theorem root_below {G x r : Nat} (hr : RootOf cc d G x r) (hok : TgtOk cc hist d G x) :
    r = G ∨ cc.isDesc r G = true := by
  rcases hr with ⟨hh, rfl⟩ | ⟨hh, P, hP, ⟨rfl, h⟩ | ⟨rfl, _⟩⟩
  · rcases hok with ⟨_, h⟩ | ⟨hh', _⟩
    · exact Or.inr h
    · rw [hh] at hh'; cases hh'
  · exact h
  · exact Or.inl rfl

theorem fresh_root {G x y r : Nat} (hx : TgtOk cc hist d G x) (hy : TgtOk cc hist d G y)
    (hc : cc.Compatible (cc.pos x) (cc.pos y)) (hr : RootOf cc d G x r) : r = cc.pos x := by
  rcases hr with ⟨hh, rfl⟩ | ⟨hh, P, hP, ⟨rfl, _⟩ | ⟨rfl, hGd, hdP⟩⟩
  · exact (pos_nonhist hh).symm
  · exact (pos_hist hh hP).symm
  · exfalso
    rw [pos_hist hh hP] at hc
    obtain ⟨hne, h1, h2, _⟩ := hc
    subst hGd
    rcases hy with ⟨hyh, hyG⟩ | ⟨hyh, P', hP', h' | h' | ⟨_, h', _⟩⟩
    · rw [pos_nonhist hyh, cc.isDesc_trans hyG hdP] at h2; cases h2
    · rw [pos_hist hyh hP', h', hdP] at h2; cases h2
    · rw [pos_hist hyh hP', cc.isDesc_trans h' hdP] at h2; cases h2
    · rw [pos_hist hyh hP'] at hne h1 h2
      rcases cc.isDesc_chain hdP h' with h'' | h'' | h''
      · exact hne h''
      · rw [h''] at h1; cases h1
      · rw [h''] at h2; cases h2

variable {n : Nat}

theorem t1inv_extend {G : Nat} {T E0 pre pre' E E' : List Nat}
    (h : T1Inv cc hist d n G T E0 pre E) (hsub : ∀ x ∈ E, x ∈ E') (hpc : PC cc d E')
    (hpre : ∀ x ∈ pre, x ∈ pre')
    (hnew : ∀ w ∈ E', w ∉ E → w ∉ E0 → cc.isDesc w G = true ∧ cc.isHistory w = false ∧
      C34 cc E' w ∧ (∀ q, (cc.st w).parent = some q → q = G ∨ q ∈ E' ∨
        (cc.isDesc q G = true ∧ ∃ x ∈ T, cc.isDesc x q = true)) ∧
      ∃ x ∈ pre', ∃ r, RootOf cc d G x r ∧ (w = r ∨ cc.isDesc w r = true))
    (hx : ∀ x ∈ pre', x ∉ pre → PerX cc hist d n G E' x) : T1Inv cc hist d n G T E0 pre' E' := by
  obtain ⟨_, h2, h3, h4⟩ := h
  refine ⟨hpc, fun x hx' => hsub x (h2 x hx'), fun w hw hw0 => ?_, fun x hx' => ?_⟩
  · by_cases hwE : w ∈ E
    · obtain ⟨a1, a2, a3, a4, x, hx', r, hr, hwr⟩ := h3 w hwE hw0
      refine ⟨a1, a2, c34_mono hsub a3, fun q hq => ?_, x, hpre x hx', r, hr, hwr⟩
      rcases a4 q hq with h' | h' | h'
      · exact Or.inl h'
      · exact Or.inr (Or.inl (hsub q h'))
      · exact Or.inr (Or.inr h')
    · exact hnew w hw hwE hw0
  · by_cases hxp : x ∈ pre
    · obtain ⟨b1, b2, b3, z, hz, hzG⟩ := h4 x hxp
      exact ⟨doneD_mono hsub b1, fun e he => hsub e (b2 e he), anch_mono hsub b3, z, hsub z hz, hzG⟩
    · exact hx x hx' hxp

theorem fresh_fits {G y ry : Nat} {T E0 pre E : List Nat} (h : T1Inv cc hist d n G T E0 pre E)
    (hpreT : ∀ x ∈ pre, x ∈ T) (hok : ∀ x ∈ T, TgtOk cc hist d G x) (hy : TgtOk cc hist d G y)
    (hcomp : ∀ x ∈ pre, cc.Compatible (cc.pos x) (cc.pos y)) (hry : RootOf cc d G y ry)
    (hf0 : Fits cc d E0 G) (hn0 : ∀ z ∈ E0, cc.isDesc z G = false) :
    Fits cc d E ry ∧ ∀ z ∈ E, cc.isDesc z ry = false := by
  obtain ⟨_, _, h3, _⟩ := h
  obtain ⟨f0, n0⟩ := fits_down hf0 hn0 (root_below hry hy)
  -- a new state lies below the position of a target compatible with `y`
  have at_pos : ∀ w ∈ E, w ∉ E0 → ∃ x ∈ pre, ry = cc.pos y ∧
      cc.Compatible (cc.pos x) (cc.pos y) ∧ (w = cc.pos x ∨ cc.isDesc w (cc.pos x) = true) := by
    intro w hw hw0
    obtain ⟨_, _, _, _, x, hx, r, hr, hwr⟩ := h3 w hw hw0
    have hc := hcomp x hx
    have hrx := fresh_root (hok x (hpreT x hx)) hy hc hr
    have hry' := fresh_root hy (hok x (hpreT x hx)) (compat_symm hc) hry
    subst hrx
    exact ⟨x, hx, hry', hc, hwr⟩
  refine ⟨fun w hw hwd => ?_, fun z hz => ?_⟩
  · by_cases hw0 : w ∈ E0
    · exact f0 w hw0 hwd
    · obtain ⟨x, _, hry', hc, hwr⟩ := at_pos w hw hw0
      rw [hry']
      exact rel_of_compat_mono (compat_symm hc) (Or.inl rfl) hwr
  · by_cases hz0 : z ∈ E0
    · exact n0 z hz0
    · obtain ⟨x, _, hry', hc, hzr⟩ := at_pos z hz hz0
      rw [hry']
      exact (compat_mono hc hzr (Or.inl rfl)).2.1

theorem t1_aux (hw : CWF cc) (ho : HistOk cc hist) (hSD : SD cc hist d n) (hSH : SH cc hist d n)
    (hId : ∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc')
    {G : Nat} (hGd : G = d ∨ cc.isDesc G d = true) {T E0 : List Nat}
    (hok : ∀ x ∈ T, TgtOk cc hist d G x) (hf0 : Fits cc d E0 G)
    (hn0 : ∀ z ∈ E0, cc.isDesc z G = false) :
    ∀ (l pre : List Nat) (acc acc' : EntryAcc),
      l.foldlM (fun acc x => cc.addDesc hist d n x acc) acc = some acc' →
      (∀ x ∈ l, x ∈ T) → (∀ x ∈ pre, x ∈ T) → (pre ++ l).Pairwise (TR cc) →
      T1Inv cc hist d n G T E0 pre acc.toEnter → T1Inv cc hist d n G T E0 (pre ++ l) acc'.toEnter := by
  intro l
  induction l with
  | nil =>
    intro pre acc acc' h _ _ _ hinv
    rw [foldlM_nil_some h]; simpa using hinv
  | cons y l ih =>
    intro pre acc acc' h hl hpre hpw hinv
    obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
    have hyT := hl y List.mem_cons_self
    have hy := hok y hyT
    suffices hb' : T1Inv cc hist d n G T E0 (pre ++ [y]) b.toEnter by
      have := ih (pre ++ [y]) b acc' hr (fun x hx => hl x (List.mem_cons_of_mem _ hx))
        (fun x hx => by
          rcases List.mem_append.1 hx with hx | hx
          · exact hpre x hx
          · simp at hx; subst hx; exact hyT) (by simpa using hpw) hb'
      simpa using this
    have hpre' : ∀ x ∈ pre, x ∈ pre ++ [y] := fun x hx => List.mem_append_left _ hx
    by_cases hyp : y ∈ pre
    · -- entered before: nothing changes
      have hsame := hId y acc b (hinv.2.2.2 y hyp).1 hb
      refine t1inv_extend hinv (same_sub hsame) ?_ hpre' (fun w hw hwn => absurd ((hsame w).1 hw) hwn)
        (fun x hx hxp => ?_)
      · intro a ha c hc had hcd
        exact hinv.1 a ((hsame a).1 ha) c ((hsame c).1 hc) had hcd
      · rcases List.mem_append.1 hx with hx | hx
        · exact absurd hx hxp
        · simp at hx; subst hx; exact absurd hyp hxp
    · -- a fresh target: compatible with every target entered before
      have hcomp : ∀ x ∈ pre, cc.Compatible (cc.pos x) (cc.pos y) := by
        intro x hx
        have := (List.pairwise_append.1 hpw).2.2 x hx y List.mem_cons_self
        rcases this with rfl | h'
        · exact absurd hx hyp
        · exact h'
      have hyy : y ∈ pre ++ [y] := by simp
      have only_y : ∀ x ∈ pre ++ [y], x ∉ pre → x = y := by
        intro x hx hxp
        rcases List.mem_append.1 hx with hx | hx
        · exact absurd hx hxp
        · simpa using hx
      rcases hy with ⟨hyh, hyG⟩ | ⟨hyh, P, hP, hPG⟩
      · -- a non-history target
        have hry : RootOf cc d G y y := Or.inl ⟨hyh, rfl⟩
        obtain ⟨hfy, hny⟩ := fresh_fits hinv hpre hok (Or.inl ⟨hyh, hyG⟩) hcomp hry hf0 hn0
        obtain ⟨s1, s2, s3, s4, s5, s6, s7⟩ := hSD y acc b hb hyh
          (desc_of_desc_desc_or_eq hyG hGd) hinv.1 hfy hny
        refine t1inv_extend hinv s2 s1 hpre' (fun w hw' hwn _ => ?_) (fun x hx hxp => ?_)
        · have hloc : w = y ∨ cc.isDesc w y = true := by
            rcases s3 w hw' with h' | h' | h'
            · exact absurd h' hwn
            · exact Or.inl h'
            · exact Or.inr h'
          refine ⟨desc_of_desc_or_eq hloc hyG, ?_, ?_, fun q hq => ?_, y, hyy, y, hry, hloc⟩
          · by_cases hwy : w = y
            · subst hwy; exact hyh
            · exact (s5 w hw' hwn hwy).1
          · by_cases hwy : w = y
            · subst hwy; exact s6
            · exact (s5 w hw' hwn hwy).2.2
          · by_cases hwy : w = y
            · subst hwy
              rcases cc.parent_desc_or_eq hyG hq with rfl | hqG
              · exact Or.inl rfl
              · exact Or.inr (Or.inr ⟨hqG, w, hyT, hw.isDesc_parent hq⟩)
            · rcases (s5 w hw' hwn hwy).2.1 q hq with rfl | h'
              · exact Or.inr (Or.inl s4)
              · exact Or.inr (Or.inl h')
        · rw [only_y x hx hxp]
          refine ⟨s7, fun e he => ?_, fun a h1 _ => ⟨y, s4, h1⟩, y, s4, hyG⟩
          rcases he with ⟨_, rfl⟩ | ⟨hh, _⟩
          · exact s4
          · rw [hyh] at hh; cases hh
      · -- a history target
        have hPd : P = G ∨ cc.isDesc P G = true → P = d ∨ cc.isDesc P d = true :=
          fun h' => desc_or_eq_trans h' hGd
        rcases hPG with h' | h' | ⟨hGd', hdP, heff⟩
        all_goals first
          | (have hPG' : P = G ∨ cc.isDesc P G = true := by first | exact Or.inl h' | exact Or.inr h'
             have hry : RootOf cc d G y P := Or.inr ⟨hyh, P, hP, Or.inl ⟨rfl, hPG'⟩⟩
             have hyok : TgtOk cc hist d G y := Or.inr ⟨hyh, P, hP, by
               first | exact Or.inl h' | exact Or.inr (Or.inl h')⟩
             obtain ⟨hfy, hny⟩ := fresh_fits hinv hpre hok hyok hcomp hry hf0 hn0
             obtain ⟨s1, s2, s3, s4, s5, s6, s7⟩ := hSH y P P acc b hb hyh hP
               (Or.inl ⟨rfl, hPd hPG'⟩) (fun e he => effOfH_below hw ho hyh he hP) hinv.1 hfy hny
             refine t1inv_extend hinv s2 s1 hpre' (fun w hw' hwn _ => ?_) (fun x hx hxp => ?_)
             · have hwP : cc.isDesc w P = true := by
                 rcases s3 w hw' with h'' | h''
                 · exact absurd h'' hwn
                 · exact h''
               obtain ⟨c1, c2, c3⟩ := s4 w hw' hwn
               refine ⟨desc_of_desc_desc_or_eq hwP hPG', c1, c3, fun q hq => ?_, y, hyy, P, hry,
                 Or.inr hwP⟩
               rcases c2 q hq with rfl | h''
               · rcases hPG' with rfl | hqG
                 · exact Or.inl rfl
                 · exact Or.inr (Or.inr ⟨hqG, y, hyT, hw.isDesc_parent hP⟩)
               · exact Or.inr (Or.inl h'')
             · rw [only_y x hx hxp]
               obtain ⟨z, hz, hzP⟩ := s5
               refine ⟨s7, fun e he => ?_, fun a h1 _ => ⟨z, hz, ?_⟩, z, hz,
                 desc_of_desc_desc_or_eq hzP hPG'⟩
               · rcases he with ⟨hh, _⟩ | ⟨_, he⟩
                 · rw [hyh] at hh; cases hh
                 · exact s6 e he
               · rcases cc.parent_desc_or_eq h1 hP with rfl | h''
                 · exact hzP
                 · exact cc.isDesc_trans hzP h'')
          | (subst hGd'
             have hry : RootOf cc G G y G := Or.inr ⟨hyh, P, hP, Or.inr ⟨rfl, rfl, hdP⟩⟩
             have hyok : TgtOk cc hist G G y := Or.inr ⟨hyh, P, hP, Or.inr (Or.inr ⟨rfl, hdP, heff⟩)⟩
             obtain ⟨hfy, hny⟩ := fresh_fits hinv hpre hok hyok hcomp hry hf0 hn0
             obtain ⟨s1, s2, s3, s4, s5, s6, s7⟩ := hSH y P G acc b hb hyh hP
               (Or.inr ⟨rfl, hdP⟩) heff hinv.1 hfy hny
             refine t1inv_extend hinv s2 s1 hpre' (fun w hw' hwn _ => ?_) (fun x hx hxp => ?_)
             · have hwG : cc.isDesc w G = true := by
                 rcases s3 w hw' with h'' | h''
                 · exact absurd h'' hwn
                 · exact h''
               obtain ⟨c1, c2, c3⟩ := s4 w hw' hwn
               refine ⟨hwG, c1, c3, fun q hq => ?_, y, hyy, G, hry, Or.inr hwG⟩
               rcases c2 q hq with h'' | h''
               · exact Or.inl h''
               · exact Or.inr (Or.inl h'')
             · rw [only_y x hx hxp]
               refine ⟨s7, fun e he => ?_, fun a h1 h2 => ?_, s5⟩
               · rcases he with ⟨hh, _⟩ | ⟨_, he⟩
                 · rw [hyh] at hh; cases hh
                 · exact s6 e he
               · exfalso
                 rcases cc.parent_desc_or_eq h1 hP with rfl | h''
                 · exact not_desc_both h2 hdP
                 · exact not_desc_both (cc.isDesc_trans h2 hdP) h'')

end Vertep.Entry
