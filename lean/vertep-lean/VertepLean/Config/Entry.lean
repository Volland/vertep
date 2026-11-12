import VertepLean.Config.EntryMain

/-!
# The entry lemma

`entry_legal`: from the situation `EntryPre` describes, `entrySet` succeeds (`entrySet_isSome`)
and the configuration it completes is legal. Each targeted transition enters a legal fragment
below its domain (`trans_spec`: the states entered are pairwise related or compatible and each
one is complete), the domains are pairwise unrelated so the fragments never meet, and the
configuration left by the exit is legal outside the domains.
-/

namespace Vertep.Entry

open CChart

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat}

theorem exactlyOne_congr {cs : List Nat} {P Q : Nat → Prop} (h : ∀ c ∈ cs, P c ↔ Q c)
    (he : ExactlyOne cs P) : ExactlyOne cs Q := by
  obtain ⟨⟨c, hc, hp⟩, hu⟩ := he
  exact ⟨⟨c, hc, (h c hc).1 hp⟩,
    fun c₁ h₁ c₂ h₂ q₁ q₂ => hu c₁ h₁ c₂ h₂ ((h c₁ h₁).2 q₁) ((h c₂ h₂).2 q₂)⟩

theorem below_congr {p : Nat} {S S' : List Nat} (h : ∀ x, x ∈ S ↔ x ∈ S') (hb : cc.Below p S) :
    cc.Below p S' := by
  refine ⟨fun x hx => hb.desc x ((h x).2 hx), fun x hx => hb.noHist x ((h x).2 hx),
    fun x hx q hq => ?_, fun x hx hk => exactlyOne_congr (fun c _ => h c) (hb.compound x ((h x).2 hx) hk),
    fun x hx hk c hc => (h c).1 (hb.parallel x ((h x).2 hx) hk c hc),
    fun hk => exactlyOne_congr (fun c _ => h c) (hb.top hk),
    fun hk c hc => (h c).1 (hb.topPar hk c hc)⟩
  rcases hb.parent x ((h x).2 hx) q hq with h' | h'
  · exact Or.inl h'
  · exact Or.inr ((h q).1 h')

theorem sibling_unrel (hw : CWF cc) {x c₁ c₂ : Nat} (h₁ : (cc.st c₁).parent = some x)
    (h₂ : (cc.st c₂).parent = some x) : cc.isDesc c₁ c₂ = false := by
  cases h : cc.isDesc c₁ c₂
  · rfl
  · exfalso
    have := hw.parent_lt' h₂
    rcases cc.parent_desc_or_eq h h₁ with rfl | h'
    · omega
    · have := cc.isDesc_lt h'; omega

/-- A set below `d` whose states are pairwise related or compatible and each complete is a
legal fragment below `d`. -/
theorem below_of (hw : CWF cc) {E : List Nat} (hk : cc.isCompoundOrRoot d = true)
    (hpc : PC cc d E) (hcond : ∀ w ∈ E, cc.isDesc w d = true → Cond cc E d w)
    (hhit : ∃ z ∈ E, cc.isDesc z d = true) : cc.Below d (E.filter (fun x => cc.isDesc x d)) := by
  have hmem : ∀ x, x ∈ E.filter (fun x => cc.isDesc x d) ↔ x ∈ E ∧ cc.isDesc x d = true := by
    intro x; simp [List.mem_filter]
  have anc : ∀ w, w ∈ E → cc.isDesc w d = true → ∀ a, cc.isDesc w a = true →
      cc.isDesc a d = true → a ∈ E := by
    intro w
    induction w using Nat.strongRecOn with
    | ind w ih =>
      intro hw' hwd a hwa had
      obtain ⟨q, hq, hlt, hqa⟩ := (cc.isDesc_iff_parent w a).1 hwa
      rcases (hcond w hw' hwd).2.1 q hq with rfl | hqE
      · rcases hqa with rfl | h'
        · exact absurd had (by simp [cc.isDesc_irrefl])
        · exact (not_desc_both h' had).elim
      · rcases hqa with rfl | h'
        · exact hqE
        · exact ih q hlt hqE (cc.isDesc_trans h' had) a h' had
  -- the child towards an entered descendant is entered
  have child : ∀ y, (y = d ∨ cc.isDesc y d = true) → ∀ z ∈ E, cc.isDesc z y = true →
      ∃ c ∈ (cc.st y).children, c ∈ E ∧ cc.isDesc c d = true := by
    intro y hy z hz hzy
    have hzd := desc_of_desc_desc_or_eq hzy hy
    obtain ⟨c, hc, hzc⟩ := hw.chain_child_nonhist hzy (hcond z hz hzd).1
    have hcd := desc_of_desc_desc_or_eq (hw.child_isDesc hc) hy
    refine ⟨c, hc, ?_, hcd⟩
    rcases hzc with rfl | hzc
    · exact hz
    · exact anc z hz hzd c hzc hcd
  have uniq : ∀ y, cc.isParallel y = false → ∀ c₁ ∈ (cc.st y).children, ∀ c₂ ∈ (cc.st y).children,
      c₁ ∈ E → cc.isDesc c₁ d = true → c₂ ∈ E → cc.isDesc c₂ d = true → c₁ = c₂ := by
    intro y hy c₁ h₁ c₂ h₂ e₁ d₁ e₂ d₂
    have p₁ := ((hw.children_iff y c₁).1 h₁).2.1
    have p₂ := ((hw.children_iff y c₂).1 h₂).2.1
    rcases hpc c₁ e₁ c₂ e₂ d₁ d₂ with h | h | h | h
    · exact h
    · rw [sibling_unrel hw p₁ p₂] at h; cases h
    · rw [sibling_unrel hw p₂ p₁] at h; cases h
    · rw [sibling_parallel hw p₁ p₂ h] at hy; cases hy
  refine ⟨fun x hx => ((hmem x).1 hx).2, fun x hx => (hcond x ((hmem x).1 hx).1 ((hmem x).1 hx).2).1,
    fun x hx q hq => ?_, fun x hx hk' => ?_, fun x hx hk' c hc => ?_, fun _ => ?_, fun hp => ?_⟩
  · obtain ⟨hxE, hxd⟩ := (hmem x).1 hx
    rcases (hcond x hxE hxd).2.1 q hq with h | h
    · exact Or.inl h
    · rcases cc.parent_desc_or_eq hxd hq with h' | h'
      · exact Or.inl h'
      · exact Or.inr ((hmem q).2 ⟨h, h'⟩)
  · obtain ⟨hxE, hxd⟩ := (hmem x).1 hx
    obtain ⟨z, hz, hzx⟩ := (hcond x hxE hxd).2.2.1 hk'
    obtain ⟨c, hc, hcE, hcd⟩ := child x (Or.inr hxd) z hz hzx
    refine ⟨⟨c, hc, (hmem c).2 ⟨hcE, hcd⟩⟩, fun c₁ h₁ c₂ h₂ e₁ e₂ => ?_⟩
    exact uniq x (compound_not_parallel (CWF.isCompoundOrRoot_of_compound hk')) c₁ h₁ c₂ h₂
      ((hmem c₁).1 e₁).1 ((hmem c₁).1 e₁).2 ((hmem c₂).1 e₂).1 ((hmem c₂).1 e₂).2
  · obtain ⟨hxE, hxd⟩ := (hmem x).1 hx
    obtain ⟨z, hz, hzc⟩ := (hcond x hxE hxd).2.2.2 hk' c hc
    have hcd := cc.isDesc_trans (hw.child_isDesc hc) hxd
    refine (hmem c).2 ⟨?_, hcd⟩
    rcases hzc with rfl | hzc
    · exact hz
    · exact anc z hz (cc.isDesc_trans hzc hcd) c hzc hcd
  · obtain ⟨z, hz, hzd⟩ := hhit
    obtain ⟨c, hc, hcE, hcd⟩ := child d (Or.inl rfl) z hz hzd
    refine ⟨⟨c, hc, (hmem c).2 ⟨hcE, hcd⟩⟩, fun c₁ h₁ c₂ h₂ e₁ e₂ => ?_⟩
    exact uniq d (compound_not_parallel hk) c₁ h₁ c₂ h₂
      ((hmem c₁).1 e₁).1 ((hmem c₁).1 e₁).2 ((hmem c₂).1 e₂).1 ((hmem c₂).1 e₂).2
  · rw [compound_not_parallel hk] at hp; cases hp

/-- **One transition**: its targets and effective targets enter a legal fragment below its
domain, and nothing else. -/
theorem trans_spec (hw : CWF cc) (ho : HistOk cc hist) (hspec : EffSpec cc hist) {t : Nat}
    {cfg : List Nat} (hdom : DomOk cc hist cfg t d) (hne : (cc.tr t).targets ≠ [])
    (acc acc' : EntryAcc) (hn : ∀ z ∈ acc.toEnter, cc.isDesc z d = false)
    (hc : (do let acc ← (cc.tr t).targets.foldlM (fun acc x => cc.addDesc hist d cc.entryFuel x acc) acc
              (cc.effectiveTargets hist t).foldlM
                (fun acc s => cc.addAnc hist d cc.entryFuel s (some d) acc) acc) = some acc') :
    (∀ x ∈ acc.toEnter, x ∈ acc'.toEnter) ∧
    (∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x d = true) ∧
    cc.Below d (acc'.toEnter.filter (fun x => cc.isDesc x d)) := by
  obtain ⟨hSD, hSH, hSA⟩ := contracts (d := d) hw ho cc.entryFuel
  have hId := (idem (cc := cc) (hist := hist) (d := d) cc.entryFuel).1
  obtain ⟨b, hb, hr⟩ := bind_some hc
  have hpc : PC cc d acc.toEnter := by
    intro a ha _ _ had _; rw [hn a ha] at had; cases had
  have hfit : Fits cc d acc.toEnter d := by
    intro w hw' hwd; rw [hn w hw'] at hwd; cases hwd
  have hok : ∀ x ∈ (cc.tr t).targets, TgtOk cc hist d d x := by
    intro x hx
    cases hxh : cc.isHistory x
    · rcases hdom.targets x hx with h | ⟨h, _⟩
      · exact Or.inl ⟨hxh, by rwa [pos_nonhist hxh] at h⟩
      · rw [hxh] at h; cases h
    · obtain ⟨P, hP, _⟩ := hw.history_parent x hxh
      refine Or.inr ⟨hxh, P, hP, ?_⟩
      have heff : ∀ e, EffOfH cc hist x e → cc.isDesc e d = true :=
        fun e he => hdom.eff e (effMem_iff.2 ⟨x, hx, Or.inr ⟨hxh, he⟩⟩)
      rcases hdom.targets x hx with h | ⟨_, h | h⟩
      · rw [pos_hist hxh hP] at h; exact Or.inr (Or.inl h)
      · rw [pos_hist hxh hP] at h; exact Or.inl h
      · rw [pos_hist hxh hP] at h; exact Or.inr (Or.inr ⟨rfl, h, heff⟩)
  obtain ⟨p1, p2, p3, p4⟩ := t1_spec hw ho hSD hSH hId (Or.inl rfl) _ acc b hb hok
    ((hw.targets_legal t).imp (fun h => Or.inr h)) hpc hfit hn
  have heT : ∀ e ∈ cc.effectiveTargets hist t, e ∈ b.toEnter := by
    intro e he
    obtain ⟨x, hx, hxe⟩ := effMem_iff.1 ((hspec t e).1 he)
    exact (p4 x hx).2.1 e hxe
  obtain ⟨a1, a2, a3, a4, a5, _⟩ := af_spec hSA (Or.inl ⟨rfl, Or.inl rfl⟩) _ b acc' hr
    (fun e he => hdom.eff e ((hspec t e).1 he)) p1
    (fun e he a h1 _ => ⟨e, heT e he, h1⟩)
  have hsub : ∀ x ∈ acc.toEnter, x ∈ acc'.toEnter := fun x hx => a2 x (p2 x hx)
  have hloc : ∀ x ∈ acc'.toEnter, x ∈ acc.toEnter ∨ cc.isDesc x d = true := by
    intro x hx
    rcases a3 x hx with h | h
    · by_cases hxa : x ∈ acc.toEnter
      · exact Or.inl hxa
      · exact Or.inr (p3 x h hxa).1
    · exact Or.inr h
  refine ⟨hsub, hloc, below_of hw hdom.kind a1 (fun w hw' hwd => ?_) ?_⟩
  · have hwn : w ∉ acc.toEnter := fun h => by rw [hn w h] at hwd; cases hwd
    by_cases hwb : w ∈ b.toEnter
    · obtain ⟨_, c1, c2, c3, _⟩ := p3 w hwb hwn
      refine ⟨c1, fun q hq => ?_, c34_mono a2 c2⟩
      rcases c3 q hq with h | h | ⟨hqd, x, hx, hxq⟩
      · exact Or.inl h
      · exact Or.inr (a2 q h)
      · -- an effective target below `q`
        refine Or.inr ?_
        cases hxh : cc.isHistory x
        · have he : x ∈ cc.effectiveTargets hist t := (hspec t x).2 (.plain hx hxh)
          exact a4 x he q hxq hqd
        · obtain ⟨P, hP, _⟩ := hw.history_parent x hxh
          obtain ⟨e, he, heP⟩ := effOfH_exists hw ho hxh hP
          have he' : e ∈ cc.effectiveTargets hist t :=
            (hspec t e).2 (effMem_iff.2 ⟨x, hx, Or.inr ⟨hxh, he⟩⟩)
          refine a4 e he' q ?_ hqd
          rcases cc.parent_desc_or_eq hxq hP with rfl | h'
          · exact heP
          · exact cc.isDesc_trans heP h'
    · exact a5 w hw' hwb
  · obtain ⟨e, he⟩ := hw.effMem_exists ho t hne
    have he' := (hspec t e).2 he
    exact ⟨e, a2 e (heT e he'), hdom.eff e he⟩

/-- Two targeted transitions whose domains share a proper descendant are the same. -/
theorem dom_unique {dhist : List (Nat × List Nat)} {cfg ts : List Nat}
    (hpre : EntryPre cc hist dhist cfg ts) {t₁ t₂ d₁ d₂ x : Nat} (h₁ : t₁ ∈ ts) (h₂ : t₂ ∈ ts)
    (n₁ : (cc.tr t₁).targets ≠ []) (n₂ : (cc.tr t₂).targets ≠ [])
    (e₁ : cc.domain dhist t₁ = some d₁) (e₂ : cc.domain dhist t₂ = some d₂)
    (x₁ : cc.isDesc x d₁ = true) (x₂ : cc.isDesc x d₂ = true) : t₁ = t₂ := by
  refine Classical.byContradiction fun hne => ?_
  obtain ⟨s1, s2, s3⟩ := hpre.sep t₁ h₁ t₂ h₂ hne n₁ n₂ d₁ d₂ e₁ e₂
  rcases cc.isDesc_chain x₁ x₂ with h | h | h
  · exact s1 h
  · rw [s2] at h; cases h
  · rw [s3] at h; cases h

/-- The invariant of `entrySet` over the transitions processed so far. -/
def EInv (cc : CChart) (dhist : List (Nat × List Nat)) (pre : List Nat) (E : List Nat) : Prop :=
  (∀ x ∈ E, ∃ t ∈ pre, (cc.tr t).targets ≠ [] ∧ ∃ d, cc.domain dhist t = some d ∧
    cc.isDesc x d = true) ∧
  (∀ t ∈ pre, (cc.tr t).targets ≠ [] → ∀ d, cc.domain dhist t = some d →
    cc.Below d (E.filter (fun x => cc.isDesc x d)))

theorem entrySet_aux (hw : CWF cc) (hspec : EffSpec cc hist) {dhist : List (Nat × List Nat)}
    {cfg ts : List Nat} (hpre : EntryPre cc hist dhist cfg ts) :
    ∀ (l pre : List Nat) (acc acc' : EntryAcc), pre ++ l = ts →
      l.foldlM (fun acc t => do
        let dom := cc.domain dhist t
        let floor := dom.getD 0
        let acc ← (cc.tr t).targets.foldlM
          (fun acc x => cc.addDesc hist floor cc.entryFuel x acc) acc
        (cc.effectiveTargets hist t).foldlM
          (fun acc s => cc.addAnc hist floor cc.entryFuel s dom acc) acc) acc = some acc' →
      EInv cc dhist pre acc.toEnter → EInv cc dhist ts acc'.toEnter := by
  intro l
  induction l with
  | nil =>
    intro pre acc acc' hts h hinv
    rw [foldlM_nil_some h]; simpa [← hts] using hinv
  | cons t l ih =>
    intro pre acc acc' hts h hinv
    obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
    have htin : t ∈ ts := by rw [← hts]; simp
    have hpre_ts : ∀ t' ∈ pre, t' ∈ ts := fun t' ht' => by rw [← hts]; exact List.mem_append_left _ ht'
    have htpre : t ∉ pre := by
      intro ht
      have := hpre.nodup
      rw [← hts] at this
      exact (List.nodup_append.1 this).2.2 t ht t List.mem_cons_self rfl
    refine ih (pre ++ [t]) b acc' (by rw [← hts]; simp) hr ?_
    obtain ⟨i1, i2⟩ := hinv
    by_cases hne : (cc.tr t).targets = []
    · have : b = acc := by
        simp [hne, effectiveTargets_nil hne] at hb; exact hb.symm
      subst this
      refine ⟨fun x hx => ?_, fun t' ht' hne' d' hd' => ?_⟩
      · obtain ⟨t', ht', h'⟩ := i1 x hx
        exact ⟨t', List.mem_append_left _ ht', h'⟩
      · rcases List.mem_append.1 ht' with ht' | ht'
        · exact i2 t' ht' hne' d' hd'
        · simp at ht'; subst ht'; exact absurd hne hne'
    · obtain ⟨d, hd, hdom⟩ := hpre.dom t htin hne
      simp only [hd, Option.getD_some] at hb
      have hn : ∀ z ∈ acc.toEnter, cc.isDesc z d = false := by
        intro z hz
        cases hzd : cc.isDesc z d
        · rfl
        · obtain ⟨t', ht', hne', d', hd', hzd'⟩ := i1 z hz
          have := dom_unique hpre (hpre_ts t' ht') htin hne' hne hd' hd hzd' hzd
          subst this; exact absurd ht' htpre
      obtain ⟨u1, u2, u3⟩ := trans_spec hw hpre.histOk hspec hdom hne acc b hn hb
      refine ⟨fun x hx => ?_, fun t' ht' hne' d' hd' => ?_⟩
      · rcases u2 x hx with h' | h'
        · obtain ⟨t', ht', h''⟩ := i1 x h'
          exact ⟨t', List.mem_append_left _ ht', h''⟩
        · exact ⟨t, by simp, hne, d, hd, h'⟩
      · rcases List.mem_append.1 ht' with htp | htt
        · refine below_congr (fun x => ?_) (i2 t' htp hne' d' hd')
          simp only [List.mem_filter]
          constructor
          · rintro ⟨h1, h2⟩; exact ⟨u1 x h1, h2⟩
          · rintro ⟨h1, h2⟩
            refine ⟨?_, h2⟩
            rcases u2 x h1 with h' | h'
            · exact h'
            · have := dom_unique hpre (hpre_ts t' htp) htin hne' hne hd' hd (by simpa using h2) h'
              subst this; exact absurd htp htpre
        · simp at htt; subst htt
          rw [hd] at hd'; cases hd'
          exact u3

end Vertep.Entry

namespace Vertep

open CChart Entry

/-- **The entry lemma.** From `EntryPre`, `entrySet` succeeds and the states before plus the
states it enters form a legal configuration. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem entry_legal : EntryLegal := by
  intro cc hist dhist cfg ts hw hspec hpre
  have ho := hpre.histOk
  have hsome := entrySet_isSome hw ho (ts := ts) (dhist := dhist) (fun t ht hne => by
    obtain ⟨d, hd, hdom⟩ := hpre.dom t ht hne
    exact ⟨d, hd, fun e he => hdom.eff e ((hspec t e).1 he)⟩)
  obtain ⟨acc, hacc⟩ := Option.isSome_iff_exists.1 hsome
  refine ⟨acc, hacc, fun L hL => ?_⟩
  have hinv := entrySet_aux hw hspec hpre ts [] {} acc rfl (by unfold CChart.entrySet at hacc; exact hacc)
    ⟨fun x hx => by simp at hx, fun t ht => by simp at ht⟩
  obtain ⟨i1, i2⟩ := hinv
  have hdesc0 : ∀ x ∈ cfg, cc.isDesc x 0 = true := fun x hx =>
    hw.isDesc_zero x (Nat.pos_of_ne_zero (hpre.noHist x hx).1) (hpre.noHist x hx).2.1
  have hmemF : ∀ (d x : Nat), x ∈ acc.toEnter.filter (fun x => cc.isDesc x d) ↔
      x ∈ acc.toEnter ∧ cc.isDesc x d = true := by
    intro d x; simp [List.mem_filter]
  -- the children of a compound state (or the root) in `L`
  have key : ∀ y, (y = 0 ∨ y ∈ cfg ∨ y ∈ acc.toEnter) → cc.isCompoundOrRoot y = true →
      ExactlyOne (cc.st y).children (· ∈ L) := by
    intro y hy hk
    by_cases hya : y ∈ acc.toEnter
    · obtain ⟨t, ht, hne, d, hd, hyd⟩ := i1 y hya
      obtain ⟨d', hd', hdom⟩ := hpre.dom t ht hne
      rw [hd] at hd'; cases hd'
      have hb := i2 t ht hne d hd
      have hy0 : y ≠ 0 := CWF.isDesc_ne_zero hyd
      refine exactlyOne_congr (fun c hc => ?_)
        (hb.compound y ((hmemF d y).2 ⟨hya, hyd⟩) (hw.kind_compound_of_ne_zero hy0 hk))
      have hcd := cc.isDesc_trans (hw.child_isDesc hc) hyd
      rw [hmemF, hL c]
      constructor
      · exact fun h => Or.inr h.1
      · rintro (h | h)
        · rw [hdom.empty c h] at hcd; cases hcd
        · exact ⟨h, hcd⟩
    · have hy' : y = 0 ∨ y ∈ cfg := by
        rcases hy with h | h | h
        · exact Or.inl h
        · exact Or.inr h
        · exact absurd h hya
      by_cases hdomy : ∃ t ∈ ts, (cc.tr t).targets ≠ [] ∧ cc.domain dhist t = some y
      · obtain ⟨t, ht, hne, hd⟩ := hdomy
        obtain ⟨d', hd', hdom⟩ := hpre.dom t ht hne
        rw [hd] at hd'; cases hd'
        refine exactlyOne_congr (fun c hc => ?_) ((i2 t ht hne y hd).top hk)
        have hcd := hw.child_isDesc hc
        rw [hmemF, hL c]
        constructor
        · exact fun h => Or.inr h.1
        · rintro (h | h)
          · rw [hdom.empty c h] at hcd; cases hcd
          · exact ⟨h, hcd⟩
      · have hnd : ∀ t ∈ ts, (cc.tr t).targets ≠ [] → cc.domain dhist t ≠ some y :=
          fun t ht hne hd => hdomy ⟨t, ht, hne, hd⟩
        refine exactlyOne_congr (fun c hc => ?_) (hpre.compound y hy' hk hnd)
        rw [hL c]
        constructor
        · exact Or.inl
        · rintro (h | h)
          · exact h
          · exfalso
            obtain ⟨t, ht, hne, d, hd, hcd⟩ := i1 c h
            obtain ⟨d', hd', hdom⟩ := hpre.dom t ht hne
            rw [hd] at hd'; cases hd'
            rcases cc.parent_desc_or_eq hcd ((hw.children_iff y c).1 hc).2.1 with rfl | hyd
            · exact hnd t ht hne hd
            · rcases hy' with rfl | hyc
              · have := cc.isDesc_lt hyd; omega
              · rw [hdom.empty y hyc] at hyd; cases hyd
  refine ⟨fun x hx => ?_, fun x hx => ?_, fun x hx q hq => ?_, fun x hx hk => ?_,
    fun x hx hk c hc => ?_, fun hk => key 0 (Or.inl rfl) hk, fun hp => ?_⟩
  · rcases (hL x).1 hx with h | h
    · exact hdesc0 x h
    · obtain ⟨t, ht, hne, d, hd, hxd⟩ := i1 x h
      obtain ⟨d', hd', hdom⟩ := hpre.dom t ht hne
      rw [hd] at hd'; cases hd'
      rcases hdom.active with rfl | hdc
      · exact hxd
      · exact cc.isDesc_trans hxd (hdesc0 d hdc)
  · rcases (hL x).1 hx with h | h
    · exact (hpre.noHist x h).2.2
    · obtain ⟨t, ht, hne, d, hd, hxd⟩ := i1 x h
      exact (i2 t ht hne d hd).noHist x ((hmemF d x).2 ⟨h, hxd⟩)
  · rcases (hL x).1 hx with h | h
    · rcases hpre.parent x h q hq with h' | h'
      · exact Or.inl h'
      · exact Or.inr ((hL q).2 (Or.inl h'))
    · obtain ⟨t, ht, hne, d, hd, hxd⟩ := i1 x h
      obtain ⟨d', hd', hdom⟩ := hpre.dom t ht hne
      rw [hd] at hd'; cases hd'
      rcases (i2 t ht hne d hd).parent x ((hmemF d x).2 ⟨h, hxd⟩) q hq with rfl | h'
      · rcases hdom.active with h'' | h''
        · exact Or.inl h''
        · exact Or.inr ((hL q).2 (Or.inl h''))
      · exact Or.inr ((hL q).2 (Or.inr ((hmemF d q).1 h').1))
  · rcases (hL x).1 hx with h | h
    · exact key x (Or.inr (Or.inl h)) (CWF.isCompoundOrRoot_of_compound hk)
    · exact key x (Or.inr (Or.inr h)) (CWF.isCompoundOrRoot_of_compound hk)
  · rcases (hL x).1 hx with h | h
    · exact (hL c).2 (Or.inl (hpre.parallel x h hk c hc))
    · obtain ⟨t, ht, hne, d, hd, hxd⟩ := i1 x h
      exact (hL c).2 (Or.inr ((hmemF d c).1
        ((i2 t ht hne d hd).parallel x ((hmemF d x).2 ⟨h, hxd⟩) hk c hc)).1)
  · rw [compound_not_parallel hw.isCompoundOrRoot_zero] at hp; cases hp

end Vertep
