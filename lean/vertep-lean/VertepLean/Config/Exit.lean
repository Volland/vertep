import VertepLean.Config.ExitBase

/-!
# The exit lemma

`exit_legal`: from a legal configuration and legal history values, a selected set of
transitions exits exactly the active proper descendants of their domains (pairwise unrelated),
records legal history values, and leaves the situation `EntryPre` describes, with the domains
computed under the history before recording.
-/

namespace Vertep

open CChart

theorem ExactlyOne.congr {cs : List Nat} {P Q : Nat → Prop} (h : ∀ c ∈ cs, P c ↔ Q c)
    (he : ExactlyOne cs P) : ExactlyOne cs Q := by
  obtain ⟨⟨c, hc, hp⟩, hu⟩ := he
  exact ⟨⟨c, hc, (h c hc).1 hp⟩,
    fun c₁ h₁ c₂ h₂ q₁ q₂ => hu c₁ h₁ c₂ h₂ ((h c₁ h₁).2 q₁) ((h c₂ h₂).2 q₂)⟩

theorem lookup_filter_ne (l : List (Nat × List Nat)) (h k : Nat) (hk : k ≠ h) :
    List.lookup k (l.filter (fun e => e.1 != h)) = List.lookup k l := by
  induction l with
  | nil => rfl
  | cons e l ih =>
    obtain ⟨a, b⟩ := e
    by_cases ha : a = h
    · subst ha
      have : (k == a) = false := by simp [hk]
      simp [List.lookup_cons, this, ih]
    · simp [ha, List.lookup_cons, ih]

theorem lookup_cons_filter (l : List (Nat × List Nat)) (h k : Nat) (v : List Nat) :
    List.lookup k ((h, v) :: l.filter (fun e => e.1 != h)) =
      if k = h then some v else List.lookup k l := by
  rw [List.lookup_cons]
  by_cases hk : k = h
  · subst hk; simp
  · have : (k == h) = false := by simp [hk]
    rw [this]; simp only [hk, ite_false]
    exact lookup_filter_ne l h k hk

namespace CWF

variable {cc : CChart}

/-- The value `recordHistory` gives history state `h` of the exited state `s`. -/
def recV (cc : CChart) (config : List Nat) (s h : Nat) : List Nat :=
  if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
  else config.filter (fun x => (cc.st x).parent == some s)

theorem record_lookup (config ex : List Nat) (hist : List (Nat × List Nat)) (k : Nat) :
    histValue (cc.recordPure config ex hist) k = histValue hist k ∨
      ∃ s ∈ ex, k ∈ (cc.st s).histories ∧
        histValue (cc.recordPure config ex hist) k = some (recV cc config s k) := by
  unfold CChart.recordPure CChart.histValue
  have inner : ∀ (s : Nat) (hs : List Nat) (H : List (Nat × List Nat)),
      (∀ h ∈ hs, h ∈ (cc.st s).histories) →
      ∀ k, List.lookup k (hs.foldl (fun hist h =>
        (h, if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
            else config.filter (fun x => (cc.st x).parent == some s)) ::
          hist.filter (fun e => e.1 != h)) H) = List.lookup k H ∨
        (k ∈ (cc.st s).histories ∧ List.lookup k (hs.foldl (fun hist h =>
        (h, if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
            else config.filter (fun x => (cc.st x).parent == some s)) ::
          hist.filter (fun e => e.1 != h)) H) = some (recV cc config s k)) := by
    intro s hs
    induction hs with
    | nil => intro H _ k; exact Or.inl rfl
    | cons h hs ih =>
      intro H hh k
      rw [List.foldl_cons]
      rcases ih _ (fun h' h'' => hh h' (List.mem_cons_of_mem _ h'')) k with e | e
      · rw [e, lookup_cons_filter]
        by_cases hk : k = h
        · subst hk; right; exact ⟨hh k List.mem_cons_self, by simp [recV]⟩
        · left; simp [hk]
      · exact Or.inr e
  have outer : ∀ (l : List Nat) (H : List (Nat × List Nat)), (∀ s ∈ l, s ∈ ex) →
      (∀ k, List.lookup k H = List.lookup k hist ∨ ∃ s ∈ ex, k ∈ (cc.st s).histories ∧
        List.lookup k H = some (recV cc config s k)) →
      ∀ k, List.lookup k (l.foldl (fun hist s => (cc.st s).histories.foldl (fun hist h =>
        (h, if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
            else config.filter (fun x => (cc.st x).parent == some s)) ::
          hist.filter (fun e => e.1 != h)) hist) H) = List.lookup k hist ∨
        ∃ s ∈ ex, k ∈ (cc.st s).histories ∧ List.lookup k (l.foldl (fun hist s =>
          (cc.st s).histories.foldl (fun hist h =>
        (h, if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
            else config.filter (fun x => (cc.st x).parent == some s)) ::
          hist.filter (fun e => e.1 != h)) hist) H) = some (recV cc config s k) := by
    intro l
    induction l with
    | nil => intro H _ hH; exact hH
    | cons s l ih =>
      intro H hl hH
      rw [List.foldl_cons]
      apply ih _ (fun s' h' => hl s' (List.mem_cons_of_mem _ h'))
      intro k
      rcases inner s _ H (fun _ h => h) k with e | e
      · rw [e]; exact hH k
      · exact Or.inr ⟨s, hl s List.mem_cons_self, e⟩
  exact outer ex hist (fun _ h => h) (fun _ => Or.inl rfl) k

/-- Every ancestor of an active state is active or the root. -/
theorem active_anc (_hw : CWF cc) {config : List Nat} (hb : cc.Below 0 config) :
    ∀ x ∈ config, ∀ y, cc.isDesc x y = true → y = 0 ∨ y ∈ config := by
  intro x
  induction x using Nat.strongRecOn with
  | ind x ih =>
    intro hx y hy
    obtain ⟨q, hq, hlt, h⟩ := (cc.isDesc_iff_parent x y).1 hy
    rcases h with rfl | h
    · exact hb.parent x hx _ hq
    · rcases hb.parent x hx q hq with rfl | hq'
      · have := cc.isDesc_lt h; omega
      · exact ih q hlt hq' y h

/-- The fragment of a legal configuration below an active state. -/
theorem below_sub (hw : CWF cc) {config : List Nat} (hb : cc.Below 0 config) {s : Nat}
    (hs : s ∈ config) : cc.Below s (config.filter (fun x => cc.isDesc x s)) := by
  have hmem : ∀ x, x ∈ config.filter (fun x => cc.isDesc x s) ↔ x ∈ config ∧ cc.isDesc x s = true := by
    intro x; simp
  have hs0 : s ≠ 0 := isDesc_ne_zero (hb.desc s hs)
  refine ⟨fun x hx => ((hmem x).1 hx).2, fun x hx => hb.noHist x ((hmem x).1 hx).1, ?_, ?_, ?_, ?_, ?_⟩
  · intro x hx q hq
    obtain ⟨hx1, hx2⟩ := (hmem x).1 hx
    rcases cc.parent_desc_or_eq hx2 hq with rfl | h
    · exact Or.inl rfl
    · right
      rcases hb.parent x hx1 q hq with rfl | hq'
      · have := cc.isDesc_lt h; omega
      · exact (hmem q).2 ⟨hq', h⟩
  · intro x hx hc
    obtain ⟨hx1, hx2⟩ := (hmem x).1 hx
    refine ExactlyOne.congr (fun c hcx => ?_) (hb.compound x hx1 hc)
    rw [hmem]
    exact ⟨fun h => ⟨h, cc.isDesc_trans (hw.child_isDesc hcx) hx2⟩, fun h => h.1⟩
  · intro x hx hp c hc
    obtain ⟨hx1, hx2⟩ := (hmem x).1 hx
    exact (hmem c).2 ⟨hb.parallel x hx1 hp c hc, cc.isDesc_trans (hw.child_isDesc hc) hx2⟩
  · intro hc
    refine ExactlyOne.congr (fun c hcx => ?_)
      (hb.compound s hs (hw.kind_compound_of_ne_zero hs0 hc))
    rw [hmem]
    exact ⟨fun h => ⟨h, hw.child_isDesc hcx⟩, fun h => h.1⟩
  · intro hp c hc
    exact (hmem c).2 ⟨hb.parallel s hs hp c hc, hw.child_isDesc hc⟩

/-- Recorded history values are legal. -/
theorem record_histOk (hw : CWF cc) {config ex : List Nat} {hist : List (Nat × List Nat)}
    (hb : cc.Below 0 config) (hex : ∀ s ∈ ex, s ∈ config) (ho : HistOk cc hist) :
    HistOk cc (cc.recordPure config ex hist) := by
  intro h v hv
  rcases record_lookup config ex hist h with e | ⟨s, hs, hh, e⟩
  · rw [e] at hv; exact ho h v hv
  · rw [e] at hv; cases hv
    obtain ⟨_, hp, hhist⟩ := (hw.histories_iff s h).1 hh
    refine ⟨hhist, s, _, hp, hw.below_sub hb (hex s hs), ?_, ?_⟩
    · intro hd x
      simp only [recV, hd, ite_true, List.mem_filter, Bool.and_eq_true]
      exact ⟨fun ⟨a, b, c⟩ => ⟨⟨a, c⟩, b⟩, fun ⟨⟨a, c⟩, b⟩ => ⟨a, b, c⟩⟩
    · intro hd x
      simp only [recV, hd, Bool.false_eq_true, ite_false, List.mem_filter, beq_iff_eq]
      constructor
      · rintro ⟨h1, h2⟩; exact ⟨⟨h1, hw.isDesc_parent h2⟩, h2⟩
      · rintro ⟨⟨h1, _⟩, h2⟩; exact ⟨h1, h2⟩

theorem mem_exitSetOf {config : List Nat} {hist : List (Nat × List Nat)} {t s : Nat} :
    s ∈ cc.exitSetOf config hist t ↔ (cc.tr t).targets ≠ [] ∧
      ∃ d, cc.domain hist t = some d ∧ s ∈ config ∧ cc.isDesc s d = true := by
  unfold CChart.exitSetOf
  by_cases ht : (cc.tr t).targets = []
  · simp [ht]
  · have : (cc.tr t).targets.isEmpty = false := by simpa using ht
    rw [this]
    simp only [Bool.false_eq_true, ite_false]
    cases hd : cc.domain hist t <;> simp [ht]

theorem mem_exitSet {config : List Nat} {hist : List (Nat × List Nat)} {ts : List Nat}
    {s : Nat} : s ∈ cc.exitSet config hist ts ↔
      s ∈ config ∧ ∃ t ∈ ts, s ∈ cc.exitSetOf config hist t := by
  unfold CChart.exitSet; simp

/-- The domain of a selected targeted transition: compound or the root, active, containing its
effective targets. -/
theorem sel_domain (hw : CWF cc) {config : List Nat} {hist : List (Nat × List Nat)}
    {ts : List Nat} (hb : cc.Below 0 config) (ho : HistOk cc hist)
    (hs : Selected cc config hist ts) {t : Nat} (ht : t ∈ ts) (hne : (cc.tr t).targets ≠ []) :
    ∃ d, cc.domain hist t = some d ∧ cc.isCompoundOrRoot d = true ∧ (d = 0 ∨ d ∈ config) ∧
      ∀ e, EffMem cc hist t e → cc.isDesc e d = true := by
  obtain ⟨a, ha, _, hc⟩ := hs.enabled t ht
  unfold CChart.candidates at hc
  rw [List.mem_flatMap] at hc
  obtain ⟨src, hsrc, htr⟩ := hc
  have hsource := hw.trans_source src t htr
  have hdef := hw.trans_not_default src t htr
  have hsa : src = 0 ∨ src ∈ config := by
    rcases List.mem_cons.1 hsrc with rfl | h
    · exact Or.inr ha
    · exact hw.active_anc hb a ha src ((cc.isDesc_iff_mem a src).2 h)
  obtain ⟨d, hd, hk, hda, heff⟩ := hw.domain_spec ho hdef hne
  refine ⟨d, hd, hk, ?_, heff⟩
  rw [hsource] at hda
  rcases hda with rfl | rfl | h
  · exact Or.inl rfl
  · exact hsa
  · rcases hsa with rfl | hsa
    · have := cc.isDesc_lt h; omega
    · exact hw.active_anc hb src hsa d h

/-- An active compound state or the root has an active proper descendant. -/
theorem active_child (hw : CWF cc) {config : List Nat} (hb : cc.Below 0 config) {d : Nat}
    (hk : cc.isCompoundOrRoot d = true) (ha : d = 0 ∨ d ∈ config) :
    ∃ c ∈ config, cc.isDesc c d = true := by
  rcases ha with rfl | ha
  · obtain ⟨⟨c, hc, hcS⟩, _⟩ := hb.top hk
    exact ⟨c, hcS, hw.child_isDesc hc⟩
  · have hd0 : d ≠ 0 := isDesc_ne_zero (hb.desc d ha)
    obtain ⟨⟨c, hc, hcS⟩, _⟩ := hb.compound d ha (hw.kind_compound_of_ne_zero hd0 hk)
    exact ⟨c, hcS, hw.child_isDesc hc⟩

/-- Distinct selected targeted transitions have unrelated domains. -/
theorem sel_sep (hw : CWF cc) {config : List Nat} {hist : List (Nat × List Nat)}
    {ts : List Nat} (hb : cc.Below 0 config) (ho : HistOk cc hist)
    (hs : Selected cc config hist ts) {t₁ t₂ : Nat} (h₁ : t₁ ∈ ts) (h₂ : t₂ ∈ ts)
    (hne : t₁ ≠ t₂) (n₁ : (cc.tr t₁).targets ≠ []) (n₂ : (cc.tr t₂).targets ≠ [])
    {d₁ d₂ : Nat} (e₁ : cc.domain hist t₁ = some d₁) (e₂ : cc.domain hist t₂ = some d₂) :
    d₁ ≠ d₂ ∧ cc.isDesc d₁ d₂ = false ∧ cc.isDesc d₂ d₁ = false := by
  obtain ⟨d₁', e₁', k₁, a₁, _⟩ := hw.sel_domain hb ho hs h₁ n₁
  obtain ⟨d₂', e₂', k₂, a₂, _⟩ := hw.sel_domain hb ho hs h₂ n₂
  rw [e₁] at e₁'; cases e₁'
  rw [e₂] at e₂'; cases e₂'
  obtain ⟨c₁, hc₁, hcd₁⟩ := hw.active_child hb k₁ a₁
  obtain ⟨c₂, hc₂, hcd₂⟩ := hw.active_child hb k₂ a₂
  have x₁ : c₁ ∈ cc.exitSetOf config hist t₁ := mem_exitSetOf.2 ⟨n₁, d₁, e₁, hc₁, hcd₁⟩
  have x₂ : c₂ ∈ cc.exitSetOf config hist t₂ := mem_exitSetOf.2 ⟨n₂, d₂, e₂, hc₂, hcd₂⟩
  have dis := hs.disjoint t₁ h₁ t₂ h₂ hne
  refine ⟨?_, ?_, ?_⟩
  · rintro rfl
    exact dis c₁ x₁ (mem_exitSetOf.2 ⟨n₂, _, e₂, hc₁, hcd₁⟩)
  · cases h : cc.isDesc d₁ d₂
    · rfl
    · exact absurd (mem_exitSetOf.2 ⟨n₂, _, e₂, hc₁, cc.isDesc_trans hcd₁ h⟩) (dis c₁ x₁)
  · cases h : cc.isDesc d₂ d₁
    · rfl
    · exact absurd x₂ (fun x => dis _ (mem_exitSetOf.2 ⟨n₁, _, e₁, hc₂,
        cc.isDesc_trans hcd₂ h⟩) x)

end CWF

open CWF

/-- **The exit lemma.** -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem exit_legal : ExitLegal := by
  intro cc config hist ts hw hb _ ho hs
  simp only
  -- notation
  have hexc : ∀ s, s ∈ cc.exitSet config hist ts ↔ s ∈ config ∧ ∃ t ∈ ts,
      (cc.tr t).targets ≠ [] ∧ ∃ d, cc.domain hist t = some d ∧ cc.isDesc s d = true := by
    intro s
    rw [mem_exitSet]
    constructor
    · rintro ⟨h1, t, ht, h2⟩
      obtain ⟨n, d, e, _, h3⟩ := mem_exitSetOf.1 h2
      exact ⟨h1, t, ht, n, d, e, h3⟩
    · rintro ⟨h1, t, ht, n, d, e, h3⟩
      exact ⟨h1, t, ht, mem_exitSetOf.2 ⟨n, d, e, h1, h3⟩⟩
  generalize hex : cc.exitSet config hist ts = ex at hexc ⊢
  have hcfg : ∀ x, x ∈ config.filter (fun s => !ex.contains s) ↔ x ∈ config ∧ x ∉ ex := by
    intro x; simp
  generalize hcf : config.filter (fun s => !ex.contains s) = cfg at hcfg ⊢
  have hho : HistOk cc (cc.recordPure config ex hist) :=
    hw.record_histOk hb (fun s h => ((hexc s).1 h).1) ho
  refine ⟨hho, ⟨hs.nodup, hho, ?_, ?_, ?_, ?_, ?_, ?_⟩⟩
  · -- dom
    intro t ht hne
    obtain ⟨d, hd, hk, ha, heff⟩ := hw.sel_domain hb ho hs ht hne
    refine ⟨d, hd, ⟨?_, hk, ?_, ?_, ?_⟩⟩
    · -- the domain stays active
      rcases ha with rfl | ha
      · exact Or.inl rfl
      · right
        refine (hcfg d).2 ⟨ha, fun hdx => ?_⟩
        obtain ⟨_, t', ht', n', d', e', hdd⟩ := (hexc d).1 hdx
        by_cases htt : t = t'
        · subst htt; rw [hd] at e'; cases e'
          rw [cc.isDesc_irrefl] at hdd; cases hdd
        · have := (hw.sel_sep hb ho hs ht ht' htt hne n' hd e').2.1
          rw [this] at hdd; cases hdd
    · -- nothing active below the domain
      intro x hx
      cases h : cc.isDesc x d
      · rfl
      · obtain ⟨hx1, hx2⟩ := (hcfg x).1 hx
        exact absurd ((hexc x).2 ⟨hx1, t, ht, hne, d, hd, h⟩) hx2
    · -- effective targets under the recorded history lie below the domain
      suffices H : ∀ u x, EffMem cc (cc.recordPure config ex hist) u x →
          (∀ e, EffMem cc hist u e → EffMem cc hist t e) → cc.isDesc x d = true from
        fun x hx => H t x hx (fun _ h => h)
      intro u x hm
      induction hm with
      | plain hx hh => intro hu; exact heff _ (hu _ (.plain hx hh))
      | @value u h v x hx hh hv hxv =>
        intro hu
        rcases record_lookup config ex hist h with e | ⟨s, hsx, hhs, e⟩
        · rw [e] at hv; exact heff _ (hu _ (.value hx hh hv hxv))
        · rw [e] at hv; cases hv
          have hps := ((hw.histories_iff s h).1 hhs).2.1
          obtain ⟨e0, he0, he0s⟩ := hw.effMem_history ho hx hh hps
          have he0d := heff e0 (hu e0 he0)
          obtain ⟨_, t', ht', n', d', e', hsd'⟩ := (hexc s).1 hsx
          have hxs : cc.isDesc x s = true := by
            unfold recV at hxv
            split at hxv
            · simp at hxv; exact hxv.2.2
            · simp at hxv; exact hw.isDesc_parent hxv.2
          by_cases htt : t = t'
          · subst htt; rw [hd] at e'; cases e'
            exact cc.isDesc_trans hxs hsd'
          · have hsep := hw.sel_sep hb ho hs ht ht' htt hne n' hd e'
            have he0d' := cc.isDesc_trans he0s hsd'
            rcases cc.isDesc_chain he0d he0d' with h1 | h1 | h1
            · exact absurd h1 hsep.1
            · rw [hsep.2.1] at h1; cases h1
            · rw [hsep.2.2] at h1; cases h1
      | @dflt u h dd x hx hh hv hdd _ ih =>
        intro hu
        have hv' : histValue hist h = none := by
          rcases record_lookup config ex hist h with e | ⟨s, _, _, e⟩
          · rw [← e]; exact hv
          · rw [e] at hv; cases hv
        exact ih (fun e he => hu e (.dflt hx hh hv' hdd he))
    · -- target positions
      intro y hy
      cases hyh : cc.isHistory y
      · left
        have : cc.pos y = y := by simp [CChart.pos, hyh]
        rw [this]; exact heff y (.plain hy hyh)
      · obtain ⟨p, hp, _⟩ := hw.history_parent y hyh
        have hpos : cc.pos y = p := by simp [CChart.pos, hyh, hp]
        rw [hpos]
        obtain ⟨e0, he0, he0p⟩ := hw.effMem_history ho hy hyh hp
        have he0d := heff e0 he0
        rcases cc.isDesc_chain he0p he0d with h1 | h1 | h1
        · exact Or.inr ⟨rfl, Or.inl h1⟩
        · exact Or.inl h1
        · exact Or.inr ⟨rfl, Or.inr h1⟩
  · -- sep
    intro t₁ h₁ t₂ h₂ hne n₁ n₂ d₁ d₂ e₁ e₂
    exact hw.sel_sep hb ho hs h₁ h₂ hne n₁ n₂ e₁ e₂
  · -- noHist
    intro x hx
    have hx1 := ((hcfg x).1 hx).1
    exact ⟨isDesc_ne_zero (hb.desc x hx1), cc.isDesc_lt_size (hb.desc x hx1), hb.noHist x hx1⟩
  · -- parent
    intro x hx q hq
    obtain ⟨hx1, hx2⟩ := (hcfg x).1 hx
    rcases hb.parent x hx1 q hq with rfl | hq1
    · exact Or.inl rfl
    · right
      refine (hcfg q).2 ⟨hq1, fun hqx => ?_⟩
      obtain ⟨_, t, ht, n, d, e, hqd⟩ := (hexc q).1 hqx
      exact hx2 ((hexc x).2 ⟨hx1, t, ht, n, d, e,
        cc.isDesc_trans (hw.isDesc_parent hq) hqd⟩)
  · -- parallel
    intro x hx hp c hc
    obtain ⟨hx1, hx2⟩ := (hcfg x).1 hx
    have hc1 := hb.parallel x hx1 hp c hc
    refine (hcfg c).2 ⟨hc1, fun hcx => ?_⟩
    obtain ⟨_, t, ht, n, d, e, hcd⟩ := (hexc c).1 hcx
    have hpc := ((hw.children_iff x c).1 hc).2.1
    rcases cc.parent_desc_or_eq hcd hpc with rfl | h
    · obtain ⟨d', e', hk, _, _⟩ := hw.sel_domain hb ho hs ht n
      rw [e] at e'; cases e'
      rw [isCompoundOrRoot_iff] at hk
      rw [isParallel_iff] at hp
      rcases hk with hk | hk <;> rw [hk] at hp <;> cases hp
    · exact hx2 ((hexc x).2 ⟨hx1, t, ht, n, d, e, h⟩)
  · -- compound
    intro x hx hk hnd
    have hone : ExactlyOne (cc.st x).children (· ∈ config) := by
      rcases hx with rfl | hx
      · exact hb.top hk
      · have hx1 := ((hcfg x).1 hx).1
        exact hb.compound x hx1 (hw.kind_compound_of_ne_zero (isDesc_ne_zero (hb.desc x hx1)) hk)
    obtain ⟨⟨c, hc, hcS⟩, hu⟩ := hone
    refine ⟨⟨c, hc, (hcfg c).2 ⟨hcS, fun hcx => ?_⟩⟩,
      fun c₁ h₁ c₂ h₂ s₁ s₂ => hu c₁ h₁ c₂ h₂ ((hcfg c₁).1 s₁).1 ((hcfg c₂).1 s₂).1⟩
    obtain ⟨_, t, ht, n, d, e, hcd⟩ := (hexc c).1 hcx
    have hpc := ((hw.children_iff x c).1 hc).2.1
    rcases cc.parent_desc_or_eq hcd hpc with rfl | h
    · exact hnd t ht n e
    · rcases hx with rfl | hx
      · have := cc.isDesc_lt h; omega
      · exact ((hcfg x).1 hx).2 ((hexc x).2 ⟨((hcfg x).1 hx).1, t, ht, n, _, e, h⟩)

end Vertep
