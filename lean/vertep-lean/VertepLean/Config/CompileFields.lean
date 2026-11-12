import VertepLean.Config.CompileTree

/-!
# Compile correctness: the structural fields of `CWF`

Each field of `CWF` except `history_rank`, from `Spec` and `WellFormed`.
-/

set_option linter.unusedSectionVars false

namespace Vertep.CompileProof

open Vertep

theorem filterMap_nodup {α β : Type} (f : α → Option β) :
    ∀ {l : List α}, l.Nodup → (∀ a ∈ l, ∀ b ∈ l, ∀ x, f a = some x → f b = some x → a = b) →
      (l.filterMap f).Nodup
  | [], _, _ => by simp
  | a :: l, hnd, hinj => by
    rw [List.nodup_cons] at hnd
    have ih := filterMap_nodup f hnd.2 (fun x hx y hy => hinj x (by simp [hx]) y (by simp [hy]))
    rw [List.filterMap_cons]
    split
    · exact ih
    · rename_i x hx
      refine List.nodup_cons.2 ⟨fun hm => ?_, ih⟩
      obtain ⟨b, hb, hbx⟩ := List.mem_filterMap.1 hm
      have := hinj a (by simp) b (by simp [hb]) x hx hbx
      subst this; exact hnd.1 hb

theorem State.kind_compound {s : State} (h : s.kind = .compound) :
    s.element = .state ∧ s.children ≠ [] := by
  unfold State.kind at h
  split at h
  · rename_i he; split at h
    · cases h
    · rename_i hc; exact ⟨he, by simpa using hc⟩
  all_goals cases h

theorem State.kind_atomic {s : State} (h : s.kind = .atomic) : s.children = [] := by
  unfold State.kind at h
  split at h
  · split at h
    · rename_i hc; simpa using hc
    · cases h
  all_goals cases h

theorem State.kind_final {s : State} (h : s.kind = .final) : s.element = .final := by
  unfold State.kind at h
  split at h
  · split at h <;> cases h
  all_goals first | assumption | cases h

theorem State.kind_parallel {s : State} (h : s.kind = .parallel) : s.element = .parallel := by
  unfold State.kind at h
  split at h
  · split at h <;> cases h
  all_goals first | assumption | cases h

theorem State.isHistory_of_element {s : State} (h : s.isHistory = true) :
    ∃ ht, s.element = .history ht := by
  unfold State.isHistory at h
  split at h
  · exact ⟨_, by assumption⟩
  · cases h

section
variable {c : Chart} {cc : CChart} (hwf : WellFormed c) (hsp : Spec c cc)
include hwf hsp

theorem at_index {s : State} (hs : s ∈ c.allStates) :
    ∃ k, ∃ hk : k < c.allStates.length, c.allStates[k] = s ∧ pos c s = k + 1 := by
  have hp := pos_lt hs
  have h0 := pos_pos (c := c) (s := s)
  refine ⟨pos c s - 1, by omega, getElem_pos (wf_nodup hwf) hs, by omega⟩

theorem stateOk_of {s : State} (hs : s ∈ c.allStates) : StateOk c (c.parentOf s) s :=
  wf_stateOk hwf (parentOf_entry hwf hs)

omit hsp in
theorem top_not_history {s : State} (hs : s ∈ c.states) : s.isHistory = false :=
  hwf.2.2.2.2.2.1 s hs

/-! ## Children and histories -/

omit hsp in
theorem cpar_eq_zero_iff {t : State} (ht : t ∈ c.allStates) : cpar c t = 0 ↔ t ∈ c.states := by
  have hnd := wf_nodup hwf
  unfold cpar
  constructor
  · intro h
    cases hq : c.parentOf t with
    | none => exact parentOf_none hnd ht hq
    | some q => rw [hq] at h; exact absurd h (Nat.ne_of_gt pos_pos)
  · intro h; rw [parentOf_top hnd h]

omit hsp in
theorem cpar_eq_pos_iff {t q : State} (ht : t ∈ c.allStates) (hq : q ∈ c.allStates) :
    cpar c t = pos c q ↔ t ∈ q.children := by
  have hnd := wf_nodup hwf
  unfold cpar
  constructor
  · intro h
    cases hq' : c.parentOf t with
    | none => rw [hq'] at h; exact absurd h.symm (Nat.ne_of_gt pos_pos)
    | some q' =>
      rw [hq'] at h
      obtain ⟨hq'm, hc⟩ := parentOf_some hnd ht hq'
      rw [← pos_inj hnd hq'm hq h]; exact hc
  · intro h; rw [parentOf_child hnd hq h]

theorem mem_kids {cs : List State} (P : Nat)
    (hP : ∀ t ∈ c.allStates, t ∈ cs ↔ cpar c t = P) (hcs : ∀ t ∈ cs, t ∈ c.allStates)
    (p : State → Bool) (x : Nat) :
    x ∈ ((cs.filter p).map State.id).filterMap (Compile.idx c.ids) ↔
      x < cc.size ∧ (cc.st x).parent = some P ∧ ∃ t ∈ c.allStates, pos c t = x ∧ p t = true := by
  have hnd := wf_nodup hwf
  simp only [List.mem_filterMap, List.mem_map, List.mem_filter]
  constructor
  · rintro ⟨_, ⟨t, ⟨htc, hpt⟩, rfl⟩, hx⟩
    have ht := hcs t htc
    rw [idx_eq hnd ht] at hx; cases hx
    exact ⟨pos_lt_size hsp ht, by rw [st_parent hwf hsp ht, (hP t ht).1 htc], t, ht, rfl, hpt⟩
  · rintro ⟨_, hpar, t, ht, rfl, hpt⟩
    rw [st_parent hwf hsp ht] at hpar; cases hpar
    exact ⟨_, ⟨t, ⟨(hP t ht).2 rfl, hpt⟩, rfl⟩, idx_eq hnd ht⟩

theorem kids_nodup {cs : List State} (hsub : cs.Sublist c.allStates) (p : State → Bool) :
    (((cs.filter p).map State.id).filterMap (Compile.idx c.ids)).Nodup := by
  have hnd := wf_nodup hwf
  refine filterMap_nodup _ (hnd.sublist (((List.filter_sublist).trans hsub).map _)) ?_
  intro a _ b _ x ha hb
  obtain ⟨s, hs, rfl, hsx⟩ := idx_some hnd ha
  obtain ⟨s', hs', rfl, hsx'⟩ := idx_some hnd hb
  rw [pos_inj hnd hs hs' (hsx.trans hsx'.symm)]

/-- The kids of compiled state `i` filtered by `p`, described by an IR list `cs`. -/
theorem kids_desc (i : Nat) :
    (i = 0 ∧ (cc.st i).children =
        ((c.states.filter (fun s => !s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) ∧
      (cc.st i).histories =
        ((c.states.filter (fun s => s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids)) ∨
    (∃ q ∈ c.allStates, i = pos c q ∧ (cc.st i).children =
        ((q.children.filter (fun s => !s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids) ∧
      (cc.st i).histories =
        ((q.children.filter (fun s => s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids)) ∨
    (cc.size ≤ i ∧ cc.st i = CState.empty) := by
  rcases Nat.eq_zero_or_pos i with rfl | h0
  · exact Or.inl ⟨rfl, st_zero_children hsp, st_zero_histories hsp⟩
  · by_cases hi : i < cc.size
    · obtain ⟨q, hq, rfl⟩ := exists_pos hwf hsp h0 hi
      exact Or.inr (Or.inl ⟨q, hq, rfl, st_children hwf hsp hq, st_histories hwf hsp hq⟩)
    · exact Or.inr (Or.inr ⟨by omega, st_ge (by omega)⟩)

theorem kids_iff (i x : Nat) (p : State → Bool) (L : List Nat)
    (h0 : i = 0 → L = ((c.states.filter p).map (·.id)).filterMap (Compile.idx c.ids))
    (hq : ∀ q ∈ c.allStates, i = pos c q →
      L = ((q.children.filter p).map (·.id)).filterMap (Compile.idx c.ids))
    (hge : cc.size ≤ i → L = []) :
    x ∈ L ↔ x < cc.size ∧ (cc.st x).parent = some i ∧ ∃ t ∈ c.allStates, pos c t = x ∧ p t = true := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · rw [h0 rfl]
    exact mem_kids hwf hsp 0 (fun t ht => (cpar_eq_zero_iff hwf ht).symm)
      (fun t ht => (states_sub c).subset ht) p x
  · by_cases hi : i < cc.size
    · obtain ⟨q, hqm, rfl⟩ := exists_pos hwf hsp hpos hi
      rw [hq q hqm rfl]
      exact mem_kids hwf hsp (pos c q) (fun t ht => (cpar_eq_pos_iff hwf ht hqm).symm)
        (fun t ht => mem_of_child c hqm ht) p x
    · rw [hge (by omega)]
      simp only [List.not_mem_nil, false_iff]
      rintro ⟨_, hpar, t, ht, rfl, _⟩
      rw [st_parent hwf hsp ht] at hpar; cases hpar
      have := cpar_lt hwf ht; have := pos_lt_size hsp ht; omega

theorem isHistory_zero : cc.isHistory 0 = false := by
  unfold CChart.isHistory; rw [st_zero_kind hsp]

theorem parent_zero_iff (x : Nat) : (cc.st x).parent = none ↔ x = 0 ∨ cc.size ≤ x := by
  constructor
  · intro h
    rcases Nat.eq_zero_or_pos x with rfl | hpos
    · exact Or.inl rfl
    · by_cases hx : x < cc.size
      · obtain ⟨t, ht, rfl⟩ := exists_pos hwf hsp hpos hx
        rw [st_parent hwf hsp ht] at h; cases h
      · exact Or.inr (by omega)
  · rintro (rfl | h)
    · exact st_zero_parent hsp
    · rw [st_ge h]; rfl

theorem pos_of_parent {x i : Nat} (h : (cc.st x).parent = some i) (hx : x < cc.size) :
    ∃ t ∈ c.allStates, pos c t = x := by
  rcases Nat.eq_zero_or_pos x with rfl | hpos
  · rw [st_zero_parent hsp] at h; cases h
  · exact exists_pos hwf hsp hpos hx

theorem children_iff' (i x : Nat) : x ∈ (cc.st i).children ↔
    x < cc.size ∧ (cc.st x).parent = some i ∧ cc.isHistory x = false := by
  rw [kids_iff hwf hsp i x (fun s => !s.isHistory) (cc.st i).children
    (fun h => by subst h; exact st_zero_children hsp)
    (fun q hq h => by subst h; exact st_children hwf hsp hq)
    (fun h => by rw [st_ge h]; rfl)]
  constructor
  · rintro ⟨h1, h2, t, ht, rfl, h3⟩
    refine ⟨h1, h2, ?_⟩
    rw [isHistory_pos hwf hsp ht]; simpa using h3
  · rintro ⟨h1, h2, h3⟩
    obtain ⟨t, ht, rfl⟩ := pos_of_parent hwf hsp h2 h1
    refine ⟨h1, h2, t, ht, rfl, ?_⟩
    rw [isHistory_pos hwf hsp ht] at h3; simp [h3]

theorem histories_iff' (i x : Nat) : x ∈ (cc.st i).histories ↔
    x < cc.size ∧ (cc.st x).parent = some i ∧ cc.isHistory x = true := by
  rw [kids_iff hwf hsp i x (fun s => s.isHistory) (cc.st i).histories
    (fun h => by subst h; exact st_zero_histories hsp)
    (fun q hq h => by subst h; exact st_histories hwf hsp hq)
    (fun h => by rw [st_ge h]; rfl)]
  constructor
  · rintro ⟨h1, h2, t, ht, rfl, h3⟩
    exact ⟨h1, h2, by rw [isHistory_pos hwf hsp ht]; exact h3⟩
  · rintro ⟨h1, h2, h3⟩
    obtain ⟨t, ht, rfl⟩ := pos_of_parent hwf hsp h2 h1
    refine ⟨h1, h2, t, ht, rfl, ?_⟩
    rw [isHistory_pos hwf hsp ht] at h3; exact h3

theorem children_sub_all {q : State} (hq : q ∈ c.allStates) : q.children.Sublist c.allStates :=
  (children_sub _).trans (desc_sub c hq)

theorem children_nodup' (i : Nat) : (cc.st i).children.Nodup := by
  rcases kids_desc hwf hsp i with ⟨_, h, _⟩ | ⟨q, hq, _, h, _⟩ | ⟨_, h⟩
  · rw [h]; exact kids_nodup hwf hsp (states_sub c) _
  · rw [h]; exact kids_nodup hwf hsp (children_sub_all hwf hsp hq) _
  · rw [h]; simp [CState.empty]

theorem histories_nodup' (i : Nat) : (cc.st i).histories.Nodup := by
  rcases kids_desc hwf hsp i with ⟨_, _, h⟩ | ⟨q, hq, _, _, h⟩ | ⟨_, h⟩
  · rw [h]; exact kids_nodup hwf hsp (states_sub c) _
  · rw [h]; exact kids_nodup hwf hsp (children_sub_all hwf hsp hq) _
  · rw [h]; simp [CState.empty]

theorem root_children' : (cc.st 0).children ≠ [] := by
  obtain ⟨s, hs⟩ := List.exists_mem_of_ne_nil _ hwf.1
  have hsA := (states_sub c).subset hs
  intro h
  have := (children_iff' hwf hsp 0 (pos c s)).2
    ⟨pos_lt_size hsp hsA, by rw [st_parent hwf hsp hsA, (cpar_eq_zero_iff hwf hsA).2 hs],
      by rw [isHistory_pos hwf hsp hsA, top_not_history hwf hs]⟩
  rw [h] at this; simp at this

theorem nonempty_children_of {q : State} (hq : q ∈ c.allStates)
    (hk : q.kind = .parallel ∨ q.kind = .compound) (hne : q.children ≠ []) :
    (cc.st (pos c q)).children ≠ [] := by
  obtain ⟨_, _, _, _, _, _, _, hany, _⟩ := stateOk_of hwf hsp hq
  obtain ⟨t, ht, htn⟩ := List.any_eq_true.1 (hany hk hne)
  have htA := mem_of_child c hq ht
  intro h
  have := (children_iff' hwf hsp (pos c q) (pos c t)).2
    ⟨pos_lt_size hsp htA, by rw [st_parent hwf hsp htA, (cpar_eq_pos_iff hwf htA hq).2 ht],
      by rw [isHistory_pos hwf hsp htA]; simpa using htn⟩
  rw [h] at this; simp at this

theorem compound_children' (i : Nat) (h : cc.isCompound i = true) : (cc.st i).children ≠ [] := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · unfold CChart.isCompound at h; rw [st_zero_kind hsp] at h; cases h
  · by_cases hi : i < cc.size
    · obtain ⟨q, hq, rfl⟩ := exists_pos hwf hsp hpos hi
      have hc := (isCompound_pos hwf hsp hq).1 h
      exact nonempty_children_of hwf hsp hq (Or.inr hc) (State.kind_compound hc).2
    · unfold CChart.isCompound at h; rw [st_ge (by omega)] at h; cases h

theorem leaf' (i : Nat) (h : (cc.st i).kind = .atomic ∨ (cc.st i).kind = .final ∨
    cc.isHistory i = true) : (cc.st i).children = [] ∧ (cc.st i).histories = [] := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · rw [st_zero_kind hsp, isHistory_zero hwf hsp] at h; simp at h
  · by_cases hi : i < cc.size
    · obtain ⟨q, hq, rfl⟩ := exists_pos hwf hsp hpos hi
      have hnil : q.children = [] := by
        obtain ⟨_, _, _, _, _, hhist, hfin, _⟩ := stateOk_of hwf hsp hq
        rw [st_kind hwf hsp hq, isHistory_pos hwf hsp hq, kindOf_eq_atomic, kindOf_eq_final] at h
        rcases h with h | h | h
        · exact State.kind_atomic h
        · have := (hfin (State.kind_final h)).1; simpa using this
        · have := (hhist h).2.2.1; simpa using this
      rw [st_children hwf hsp hq, st_histories hwf hsp hq, hnil]; simp
    · rw [st_ge (by omega)]; exact ⟨rfl, rfl⟩

theorem history_parent' (h : Nat) (hh : cc.isHistory h = true) :
    ∃ p, (cc.st h).parent = some p ∧ p ≠ 0 := by
  rcases Nat.eq_zero_or_pos h with rfl | hpos
  · rw [isHistory_zero hwf hsp] at hh; cases hh
  · by_cases hi : h < cc.size
    · obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp hpos hi
      refine ⟨cpar c s, st_parent hwf hsp hs, fun h0 => ?_⟩
      rw [isHistory_pos hwf hsp hs] at hh
      rw [top_not_history hwf ((cpar_eq_zero_iff hwf hs).1 h0)] at hh; cases hh
    · unfold CChart.isHistory at hh; rw [st_ge (by omega)] at hh; cases hh

theorem parent_lt'' (i : Nat) (h0 : 0 < i) (hi : i < cc.size) :
    ∃ p, (cc.st i).parent = some p ∧ p < i := by
  obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp h0 hi
  exact ⟨_, st_parent hwf hsp hs, cpar_lt hwf hs⟩

theorem kind_ne_root' (i : Nat) (h : i ≠ 0) : (cc.st i).kind ≠ .root := by
  by_cases hi : i < cc.size
  · obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp (Nat.pos_of_ne_zero h) hi
    rw [st_kind hwf hsp hs]; exact kindOf_ne_root s
  · rw [st_ge (by omega)]; simp [CState.empty]


/-! ## Transitions -/

theorem trans_info (i t : Nat) (ht : t ∈ (cc.st i).transitions) :
    (cc.tr t).source = i ∧ (cc.tr t).defaultEntry = false := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · rw [hsp.root_trans] at ht; cases ht
  · by_cases hi : i < cc.size
    · obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp hpos hi
      obtain ⟨k, hk, he, hp⟩ := at_index hwf hsp hs
      rw [hp] at ht ⊢
      by_cases hh : IsHist (cc.st (k + 1)) ∧ c.allStates[k].transitions = []
      · obtain ⟨t', h1, h2, h3, _⟩ := hsp.hist k hk hh.1 hh.2
        rw [h1, List.mem_singleton] at ht; subst ht; exact ⟨h2, h3⟩
      · obtain ⟨_, h2, h3, _⟩ := (hsp.decl k hk hh).2 t ht
        exact ⟨h2, h3⟩
    · rw [st_ge (by omega)] at ht; cases ht

theorem resolve_mem {L : List String} {x : Nat} (h : x ∈ Compile.resolve c.ids L) :
    ∃ u ∈ c.allStates, u.id ∈ L ∧ pos c u = x := by
  obtain ⟨a, ha, hx⟩ := List.mem_filterMap.1 h
  obtain ⟨u, hu, rfl, hp⟩ := idx_some (wf_nodup hwf) hx
  exact ⟨u, hu, ha, hp⟩

theorem resolve_ne_nil {L : List String} (hL : L ≠ []) (hids : ∀ a ∈ L, a ∈ c.ids) :
    Compile.resolve c.ids L ≠ [] := by
  obtain ⟨a, ha⟩ := List.exists_mem_of_ne_nil _ hL
  obtain ⟨u, hu, rfl, hidx⟩ := idx_isSome (wf_nodup hwf) (hids a ha)
  intro h
  have : pos c u ∈ Compile.resolve c.ids L := List.mem_filterMap.2 ⟨_, ha, hidx⟩
  rw [h] at this; simp at this

omit hwf hsp in
theorem firstChild_eq {cs : List State} (h : ∃ t ∈ cs, t.isHistory = false) :
    ∃ t ∈ cs, t.isHistory = false ∧ Compile.firstChild cs = [t.id] := by
  unfold Compile.firstChild
  split
  · rename_i t ht
    exact ⟨t, List.mem_of_find?_eq_some ht, by simpa using List.find?_some ht, rfl⟩
  · rename_i hn
    obtain ⟨t, ht, hh⟩ := h
    rw [List.find?_eq_none] at hn
    exact absurd (by simp [hh]) (hn t ht)

theorem rootTargets_ok : rootTargets c ≠ [] ∧ ∀ x ∈ rootTargets c, 0 < x ∧ x < cc.size ∧
    cc.isDesc x 0 = true := by
  have hall : ∀ x ∈ rootTargets c, ∃ u ∈ c.allStates, pos c u = x := by
    intro x hx
    obtain ⟨u, hu, _, hp⟩ := resolve_mem hwf hsp hx
    exact ⟨u, hu, hp⟩
  refine ⟨?_, fun x hx => ?_⟩
  · unfold rootTargets
    split
    · obtain ⟨s0, hs0⟩ := List.exists_mem_of_ne_nil _ hwf.1
      obtain ⟨t, ht, _, he⟩ := firstChild_eq ⟨s0, hs0, top_not_history hwf hs0⟩
      rw [he]
      exact resolve_ne_nil hwf hsp (by simp)
        (by simp only [List.mem_singleton, forall_eq]; exact List.mem_map_of_mem ((states_sub c).subset ht))
    · rename_i hne
      exact resolve_ne_nil hwf hsp (by simpa using hne) hwf.2.2.2.1
  · obtain ⟨u, hu, rfl⟩ := hall x hx
    exact ⟨pos_pos, pos_lt_size hsp hu, isDesc_zero' hwf hsp hu⟩

theorem initTargets_ok {s : State} (hs : s ∈ c.allStates) (hc : s.kind = .compound) :
    Compile.resolve c.ids (Compile.resolveInitial s).1 ≠ [] ∧
    ∀ x ∈ Compile.resolve c.ids (Compile.resolveInitial s).1, cc.isDesc x (pos c s) = true := by
  obtain ⟨_, _, hia, hie, _, _, _, hany, _⟩ := stateOk_of hwf hsp hs
  unfold Compile.resolveInitial
  split
  · rename_i hne
    refine ⟨resolve_ne_nil hwf hsp (by simpa using hne) (fun a ha => (hia.1 a ha).1), fun x hx => ?_⟩
    obtain ⟨u, hu, hid, rfl⟩ := resolve_mem hwf hsp hx
    exact (mem_descIds_iff hwf hsp hu hs).1 ((hia.1 _ hid).2)
  · split
    · rename_i ie hie'
      obtain ⟨_, _, hne, hok, _⟩ := hie ie hie'
      refine ⟨resolve_ne_nil hwf hsp hne (fun a ha => (hok.1 a ha).1), fun x hx => ?_⟩
      obtain ⟨u, hu, hid, rfl⟩ := resolve_mem hwf hsp hx
      exact (mem_descIds_iff hwf hsp hu hs).1 ((hok.1 _ hid).2)
    · obtain ⟨t, ht, htn⟩ := List.any_eq_true.1 (hany (Or.inr hc) (State.kind_compound hc).2)
      obtain ⟨t', ht', _, he⟩ := firstChild_eq ⟨t, ht, by simpa using htn⟩
      simp only [he]
      have ht'A := mem_of_child c hs ht'
      refine ⟨resolve_ne_nil hwf hsp (by simp)
        (by simp only [List.mem_singleton, forall_eq]; exact List.mem_map_of_mem ht'A), fun x hx => ?_⟩
      obtain ⟨u, hu, hid, rfl⟩ := resolve_mem hwf hsp hx
      simp only [List.mem_singleton] at hid
      rw [state_inj (wf_nodup hwf) hu ht'A hid]
      exact isDesc_of_child hwf hsp hs ht'

theorem isCompoundOrRoot_pos {s : State} (hs : s ∈ c.allStates) :
    cc.isCompoundOrRoot (pos c s) = cc.isCompound (pos c s) := by
  unfold CChart.isCompoundOrRoot CChart.isCompound
  rw [st_kind hwf hsp hs]
  have := kindOf_ne_root s
  cases h : Compile.kindOf s <;> first | exact absurd h this | rfl

theorem initial_iff' (i : Nat) : (cc.st i).initial.isSome = cc.isCompoundOrRoot i := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · rw [hsp.root_init]; unfold CChart.isCompoundOrRoot; rw [st_zero_kind hsp]; rfl
  · by_cases hi : i < cc.size
    · obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp hpos hi
      rw [isCompoundOrRoot_pos hwf hsp hs]
      obtain ⟨k, hk, he, hp⟩ := at_index hwf hsp hs
      have hini := hsp.init k hk
      rw [he, ← hp] at hini
      by_cases hc : s.kind = .compound
      · obtain ⟨t, ht, _⟩ := hini.1 hc
        rw [ht, (isCompound_pos hwf hsp hs).2 hc]; rfl
      · rw [hini.2 hc]
        cases h : cc.isCompound (pos c s)
        · rfl
        · exact absurd ((isCompound_pos hwf hsp hs).1 h) hc
    · rw [st_ge (by omega)]; unfold CChart.isCompoundOrRoot; rw [st_ge (by omega)]; rfl

theorem initial_ok' (i t : Nat) (h : (cc.st i).initial = some t) :
    (cc.tr t).defaultEntry = true ∧ (cc.tr t).source = i ∧ (cc.tr t).targets ≠ [] ∧
      ∀ x ∈ (cc.tr t).targets, cc.isDesc x i = true := by
  rcases Nat.eq_zero_or_pos i with rfl | hpos
  · rw [hsp.root_init] at h; cases h
    obtain ⟨h1, h2, h3⟩ := hsp.root_tr
    obtain ⟨hne, hall⟩ := rootTargets_ok hwf hsp
    rw [h3]
    exact ⟨h2, h1, hne, fun x hx => (hall x hx).2.2⟩
  · by_cases hi : i < cc.size
    · obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp hpos hi
      obtain ⟨k, hk, he, hp⟩ := at_index hwf hsp hs
      have hini := hsp.init k hk
      rw [he, ← hp] at hini
      by_cases hc : s.kind = .compound
      · obtain ⟨t', ht', _, h1, h2, h3⟩ := hini.1 hc
        rw [h] at ht'; cases ht'
        have h1' : (cc.tr t).source = pos c s := h1
        have h2' : (cc.tr t).defaultEntry = true := h2
        have h3' : (cc.tr t).targets = _ := h3
        rw [h3']
        exact ⟨h2', h1', initTargets_ok hwf hsp hs hc⟩
      · rw [hini.2 hc] at h; cases h
    · rw [st_ge (by omega)] at h; cases h

/-! ## Targets -/

theorem parent_lt_of {x p : Nat} (h : (cc.st x).parent = some p) : p < x := by
  rcases Nat.eq_zero_or_pos x with rfl | hpos
  · rw [st_zero_parent hsp] at h; cases h
  · by_cases hx : x < cc.size
    · obtain ⟨p', hp', hlt⟩ := parent_lt'' hwf hsp x hpos hx
      rw [h] at hp'; cases hp'; exact hlt
    · rw [st_ge (by omega)] at h; cases h

theorem good2_parallel {tg : List Nat} (h : Good2 cc.states tg) :
    ∃ p, tg = (cc.st p).children ∧ cc.isParallel p = true ∧ (cc.st p).children ≠ [] ∧
      ∃ hh, (cc.st hh).parent = some p ∧ cc.isHistory hh = true := by
  obtain ⟨hh, p, rfl, hpar, ⟨ty, hkind⟩, hinit⟩ := h
  have hhist : cc.isHistory hh = true := by
    show (match (cc.st hh).kind with | .history _ => true | _ => false) = true
    rw [show (cc.st hh).kind = _ from hkind]
  obtain ⟨p', hp', hp0⟩ := history_parent' hwf hsp hh hhist
  have hpar' : (cc.st hh).parent.getD 0 = p := hpar
  rw [hp'] at hpar'; simp only [Option.getD_some] at hpar'; subst hpar'
  have hhs : hh < cc.size := by
    by_cases h : hh < cc.size
    · exact h
    · unfold CChart.isHistory at hhist; rw [st_ge (by omega)] at hhist; cases hhist
  have hmem : hh ∈ (cc.st p').histories := (histories_iff' hwf hsp p' hh).2 ⟨hhs, hp', hhist⟩
  have hcr : cc.isCompoundOrRoot p' = false := by
    have := initial_iff' hwf hsp p'
    have hi : (cc.st p').initial = none := hinit
    rw [hi] at this; exact this.symm
  have hpar_ : cc.isParallel p' = true := by
    have hnl : ¬((cc.st p').kind = .atomic ∨ (cc.st p').kind = .final ∨ cc.isHistory p' = true) := by
      intro hl
      have := (leaf' hwf hsp p' hl).2
      rw [this] at hmem; cases hmem
    have hnr := kind_ne_root' hwf hsp p' hp0
    unfold CChart.isCompoundOrRoot at hcr
    unfold CChart.isParallel
    unfold CChart.isHistory at hnl
    cases hk : (cc.st p').kind <;> rw [hk] at hcr hnl hnr <;>
      first | exact absurd (Or.inr (Or.inr rfl)) hnl | (revert hcr hnl hnr; decide)
  refine ⟨p', rfl, hpar_, ?_, hh, hp', hhist⟩
  have hp'size : p' < cc.size := Nat.lt_trans (parent_lt_of hwf hsp hp') hhs
  obtain ⟨q, hq, hqp⟩ := exists_pos hwf hsp (Nat.pos_of_ne_zero hp0) hp'size
  obtain ⟨s, hs, hsp'⟩ := pos_of_parent hwf hsp hp' hhs
  rw [← hsp', st_parent hwf hsp hs] at hp'
  have hcp : cpar c s = pos c q := by rw [hqp]; exact Option.some.inj hp'
  have hsq := (cpar_eq_pos_iff hwf hs hq).1 hcp
  rw [← hqp] at hpar_ ⊢
  refine nonempty_children_of hwf hsp hq (Or.inl ((isParallel_pos hwf hsp hq).1 hpar_)) ?_
  intro h; rw [h] at hsq; cases hsq

theorem good_targets_lt (t : Nat) : ∀ x ∈ (cc.tr t).targets, 0 < x ∧ x < cc.size := by
  intro x hx
  rcases hsp.good t with ⟨L, hL, _, _⟩ | h
  · rw [hL] at hx
    obtain ⟨u, hu, _, rfl⟩ := resolve_mem hwf hsp hx
    exact ⟨pos_pos, pos_lt_size hsp hu⟩
  · obtain ⟨p, hp, _⟩ := good2_parallel hwf hsp h
    rw [hp] at hx
    obtain ⟨h1, h2, _⟩ := (children_iff' hwf hsp p x).1 hx
    refine ⟨Nat.pos_of_ne_zero fun h0 => ?_, h1⟩
    subst h0; rw [st_zero_parent hsp] at h2; cases h2

end

end Vertep.CompileProof
