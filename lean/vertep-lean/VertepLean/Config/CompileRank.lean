import VertepLean.Config.CompileLegal

/-!
# Compile correctness: the history rank

Compiled history default edges are IR history default edges (`historyDefaultEdges`). A compiled
cycle gives, by pigeonhole, an IR cycle of length at most the number of states, which
`HistoryDefaultsAcyclic` rules out; so the number of states reachable along history default
edges strictly decreases along an edge and is a rank below the chart size.
-/

set_option linter.unusedSectionVars false

namespace Vertep.CompileProof

open Vertep

/-! ## Generic facts -/

theorem transGen_head {α : Type} {r : α → α → Prop} {a b d : α} (h : r a b)
    (h' : Relation.TransGen r b d) : Relation.TransGen r a d := by
  induction h' with
  | single hb => exact Relation.TransGen.tail (Relation.TransGen.single h) hb
  | tail _ hc ih => exact Relation.TransGen.tail ih hc

theorem transGen_walk {α : Type} {r : α → α → Prop} {a b : α} (h : Relation.TransGen r a b) :
    ∃ m, ∃ f : Nat → α, 0 < m ∧ f 0 = a ∧ f m = b ∧ ∀ i, i < m → r (f i) (f (i + 1)) := by
  induction h with
  | single hb =>
    rename_i b'
    refine ⟨1, fun i => if i = 0 then a else b', by omega, by simp, by simp, fun i hi => ?_⟩
    have : i = 0 := by omega
    subst this; simpa using hb
  | @tail b' c' _ hc ih =>
    obtain ⟨m, f, hm, h0, hmb, hstep⟩ := ih
    refine ⟨m + 1, fun i => if i ≤ m then f i else c', by omega, by simp [h0],
      by simp only [show ¬(m + 1 ≤ m) by omega, ↓reduceIte], fun i hi => ?_⟩
    by_cases him : i < m
    · simp only [Nat.le_of_lt him, ↓reduceIte, show i + 1 ≤ m by omega]
      exact hstep i him
    · have : i = m := by omega
      subst this
      simp only [Nat.le_refl, ↓reduceIte, show ¬(i + 1 ≤ i) by omega, hmb]; exact hc

theorem nodup_map_of_inj {α β : Type} (f : α → β) : ∀ {l : List α}, l.Nodup →
    (∀ x ∈ l, ∀ y ∈ l, f x = f y → x = y) → (l.map f).Nodup
  | [], _, _ => by simp
  | a :: l, hnd, hinj => by
    rw [List.nodup_cons] at hnd
    rw [List.map_cons, List.nodup_cons]
    refine ⟨fun hm => ?_, nodup_map_of_inj f hnd.2 (fun x hx y hy => hinj x (by simp [hx]) y (by simp [hy]))⟩
    obtain ⟨b, hb, he⟩ := List.mem_map.1 hm
    have := hinj a (by simp) b (by simp [hb]) he.symm
    subst this; exact hnd.1 hb

/-- Pigeonhole: `n + 1` values below `n` repeat. -/
theorem pigeonhole (n : Nat) (f : Nat → Nat) (hf : ∀ k, k ≤ n → f k < n) :
    ∃ i j, i < j ∧ j ≤ n ∧ f i = f j := by
  refine Classical.byContradiction fun hne => ?_
  have hinj : ∀ x ∈ List.range (n + 1), ∀ y ∈ List.range (n + 1), f x = f y → x = y := by
    intro x hx y hy he
    simp only [List.mem_range] at hx hy
    refine Classical.byContradiction fun hxy => ?_
    rcases Nat.lt_or_gt_of_ne hxy with h | h
    · exact hne ⟨x, y, h, by omega, he⟩
    · exact hne ⟨y, x, h, by omega, he.symm⟩
  have hnd := nodup_map_of_inj f List.nodup_range hinj
  have hsub : (List.range (n + 1)).map f ⊆ List.range n := by
    intro y hy
    obtain ⟨k, hk, rfl⟩ := List.mem_map.1 hy
    simp only [List.mem_range] at hk ⊢
    exact hf k (by omega)
  have := hnd.length_le_of_subset hsub
  simp only [List.length_map, List.length_range] at this; omega

theorem filter_length_le {α : Type} (p q : α → Bool) : ∀ (l : List α),
    (∀ y ∈ l, p y = true → q y = true) → (l.filter p).length ≤ (l.filter q).length
  | [], _ => by simp
  | a :: l, himp => by
    have ih := filter_length_le p q l (fun z hz => himp z (by simp [hz]))
    simp only [List.filter_cons]
    have ha := himp a (by simp)
    cases hpa : p a <;> cases hqa : q a <;> simp_all <;> omega

theorem filter_length_lt {α : Type} (p q : α → Bool) : ∀ (l : List α),
    (∀ y ∈ l, p y = true → q y = true) → (∃ y ∈ l, q y = true ∧ p y = false) →
      (l.filter p).length < (l.filter q).length
  | [], _, ⟨_, hy, _⟩ => by simp at hy
  | a :: l, himp, ⟨y, hy, hq, hp⟩ => by
    have hle := filter_length_le p q l (fun z hz => himp z (by simp [hz]))
    rcases List.mem_cons.1 hy with rfl | hy
    · simp only [List.filter_cons, hq, hp]; simp; omega
    · have := filter_length_lt p q l (fun z hz => himp z (by simp [hz])) ⟨y, hy, hq, hp⟩
      simp only [List.filter_cons]
      have ha := himp a (by simp)
      cases hpa : p a <;> cases hqa : q a <;> simp_all <;> omega

/-! ## History default edges -/

/-- A compiled history default edge between history states. -/
def HEdge (cc : CChart) (h x : Nat) : Prop :=
  cc.isHistory h = true ∧ ∃ d, cc.historyDefault h = some d ∧ x ∈ (cc.tr d).targets ∧
    cc.isHistory x = true

theorem initialTargets_eq {q : State} (h : q.kind = .compound) :
    initialTargets q = (Compile.resolveInitial q).1 := by
  unfold initialTargets Compile.resolveInitial Compile.firstChild
  rw [h]
  simp only [show (StateKind.compound != StateKind.compound) = false by decide, Bool.false_eq_true,
    ↓reduceIte]
  by_cases hc : (!q.initial.isEmpty) = true
  · simp only [hc, ↓reduceIte]
  · simp only [hc, Bool.false_eq_true, ↓reduceIte]
    cases q.initialElement <;> cases List.find? (fun c => !c.isHistory) q.children <;> rfl

section
variable {c : Chart} {cc : CChart} (hwf : WellFormed c) (hsp : Spec c cc)
include hwf hsp

theorem hedge_ir {h x : Nat} (he : HEdge cc h x) :
    ∃ u ∈ c.allStates, ∃ v ∈ c.allStates, pos c u = h ∧ pos c v = x ∧
      v ∈ historyDefaultEdges c u := by
  have hnd := wf_nodup hwf
  obtain ⟨hh, d, hd, hx, hxh⟩ := he
  have hlt : h < cc.size := by
    by_cases hlt : h < cc.size
    · exact hlt
    · unfold CChart.isHistory at hh; rw [st_ge (by omega)] at hh; cases hh
  have h0 : 0 < h := by
    rcases Nat.eq_zero_or_pos h with rfl | h0
    · rw [isHistory_zero hwf hsp] at hh; cases hh
    · exact h0
  obtain ⟨u, hu, rfl⟩ := exists_pos hwf hsp h0 hlt
  obtain ⟨hx0, hxs⟩ := good_targets_lt hwf hsp d x hx
  obtain ⟨v, hv, rfl⟩ := exists_pos hwf hsp hx0 hxs
  refine ⟨u, hu, v, hv, rfl, rfl, ?_⟩
  have huh : u.isHistory = true := by rw [← isHistory_pos hwf hsp hu]; exact hh
  have hvh : v.isHistory = true := by rw [← isHistory_pos hwf hsp hv]; exact hxh
  obtain ⟨_, _, _, _, htrs, hhist, _⟩ := stateOk_of hwf hsp hu
  have hho := hhist huh
  obtain ⟨q, hq⟩ : ∃ q, c.parentOf u = some q := by
    cases hq : c.parentOf u with
    | none => rw [top_not_history hwf (parentOf_none hnd hu hq)] at huh; cases huh
    | some q => exact ⟨q, rfl⟩
  have hqA := (parentOf_some hnd hu hq).1
  obtain ⟨k, hk, hke, hp⟩ := at_index hwf hsp hu
  have hIH : IsHist (cc.st (k + 1)) := by
    unfold IsHist; rw [← hp, st_kind hwf hsp hu]; exact (kindOf_history' u).2 huh
  -- membership in the IR edges from a target identifier
  have hedge : ∀ L : List String, v.id ∈ L →
      v ∈ (L.filterMap c.find).filter (·.isHistory) := by
    intro L hL
    simp only [List.mem_filter, List.mem_filterMap]
    exact ⟨⟨v.id, hL, find_eq hnd hv⟩, hvh⟩
  unfold historyDefaultEdges
  by_cases hemp : u.transitions = []
  · obtain ⟨t, h1, _, _, h4⟩ := hsp.hist k hk hIH (by rw [hke]; exact hemp)
    rw [← hp] at h1 h4
    unfold CChart.historyDefault at hd
    rw [h1] at hd; simp only [List.head?_cons, Option.some.injEq] at hd; subst hd
    rw [h4] at hx
    unfold synth at hx
    have hpar : (cc.states.getD (pos c u) CState.empty).parent = some (pos c q) := by
      show (cc.st (pos c u)).parent = _
      rw [st_parent hwf hsp hu]; unfold cpar; rw [hq]
    rw [hpar] at hx; simp only [Option.getD_some] at hx
    rw [hemp, hq]; simp only [List.head?_nil]
    split at hx
    · rename_i ti hti
      obtain ⟨kq, hkq, hkqe, hpq⟩ := at_index hwf hsp hqA
      have hini := hsp.init kq hkq
      rw [hkqe, ← hpq] at hini
      by_cases hc : q.kind = .compound
      · obtain ⟨t', ht', _, _, _, htg⟩ := hini.1 hc
        have hti' : (cc.st (pos c q)).initial = some ti := hti
        rw [hti'] at ht'; cases ht'
        have htg' : (cc.trans.getD ti CTrans.empty).targets = _ := htg
        rw [htg'] at hx
        obtain ⟨w, hw, hid, hwp⟩ := resolve_mem hwf hsp hx
        rw [pos_inj hnd hw hv hwp] at hid
        rw [initialTargets_eq hc]
        exact hedge _ hid
      · have hti' : (cc.st (pos c q)).initial = some ti := hti
        rw [hini.2 hc] at hti'; cases hti'
    · have := ((children_iff' hwf hsp _ (pos c v)).1 hx).2.2
      rw [hxh] at this; cases this
  · obtain ⟨tr, htr⟩ : ∃ tr, u.transitions = [tr] := by
      have := hho.1
      match h' : u.transitions, hemp, this with
      | [tr], _, _ => exact ⟨tr, rfl⟩
      | [], hn, _ => exact absurd rfl hn
      | _ :: _ :: _, _, hl => simp at hl
    have hdecl := hsp.decl k hk (fun hc => hemp (by rw [← hke]; exact hc.2))
    rw [← hp, hke] at hdecl
    obtain ⟨hlen, hto⟩ := hdecl
    rw [htr] at hlen
    obtain ⟨t, ht⟩ : ∃ t, (cc.st (pos c u)).transitions = [t] := by
      match h' : (cc.st (pos c u)).transitions, hlen with
      | [t], _ => exact ⟨t, rfl⟩
    unfold CChart.historyDefault at hd
    have hdt : d = t := by rw [ht] at hd; simpa using hd.symm
    rw [hdt] at hx
    obtain ⟨_, _, _, tr', htr', htg⟩ := hto t (by rw [ht]; simp)
    rw [htr, List.mem_singleton] at htr'; subst htr'
    have htg' : (cc.tr t).targets = Compile.resolve c.ids tr'.targets := htg
    rw [htg'] at hx
    obtain ⟨w, hw, hid, hwp⟩ := resolve_mem hwf hsp hx
    rw [pos_inj hnd hw hv hwp] at hid
    rw [htr]; simp only [List.head?_cons]
    exact hedge _ hid

/-- IR reachability along history default edges within the fuel. -/
theorem reach_of_walk (w : Nat → State) : ∀ (L fuel : Nat) (F : List State), L < fuel →
    w 0 ∈ F → (∀ b, b < L → w (b + 1) ∈ historyDefaultEdges c (w b)) →
      w L ∈ historyReach c fuel F := by
  intro L
  induction L generalizing w with
  | zero =>
    intro fuel F hf h0 _
    obtain ⟨f', rfl⟩ : ∃ f', fuel = f' + 1 := ⟨fuel - 1, by omega⟩
    simp only [historyReach]; exact List.mem_append_left _ h0
  | succ L ih =>
    intro fuel F hf h0 hstep
    obtain ⟨f', rfl⟩ : ∃ f', fuel = f' + 1 := ⟨fuel - 1, by omega⟩
    simp only [historyReach]
    refine List.mem_append_right _ ?_
    exact ih (fun b => w (b + 1)) f' _ (by omega)
      (List.mem_flatMap.2 ⟨w 0, h0, hstep 0 (by omega)⟩)
      (fun b hb => hstep (b + 1) (by omega))

theorem hedge_hist {h x : Nat} (he : HEdge cc h x) :
    cc.isHistory h = true ∧ 0 < h ∧ h < cc.size := by
  obtain ⟨hh, _⟩ := he
  refine ⟨hh, ?_, ?_⟩
  · rcases Nat.eq_zero_or_pos h with rfl | h0
    · rw [isHistory_zero hwf hsp] at hh; cases hh
    · exact h0
  · by_cases hlt : h < cc.size
    · exact hlt
    · unfold CChart.isHistory at hh; rw [st_ge (by omega)] at hh; cases hh

/-- No compiled history default cycle. -/
theorem hedge_acyclic (h : Nat) : ¬ Relation.TransGen (HEdge cc) h h := by
  intro hc
  have hnd := wf_nodup hwf
  obtain ⟨m, f, hm, hf0, hfm, hstep⟩ := transGen_walk hc
  have hn := hsp.size
  -- a cycle `f i = f j` with `i < j ≤ n` along the walk
  obtain ⟨i, j, hij, hjn, hjm, hfij⟩ : ∃ i j, i < j ∧ j - i ≤ c.allStates.length ∧ j ≤ m ∧
      f i = f j := by
    by_cases hmn : m ≤ c.allStates.length
    · exact ⟨0, m, hm, by omega, Nat.le_refl _, by rw [hf0, hfm]⟩
    · obtain ⟨i, j, hij, hj, he⟩ := pigeonhole c.allStates.length (fun k => f k - 1) (fun k hk => by
        obtain ⟨_, h0, hs⟩ := hedge_hist hwf hsp (hstep k (by omega))
        omega)
      have h1 := (hedge_hist hwf hsp (hstep i (by omega))).2.1
      have h2 := (hedge_hist hwf hsp (hstep j (by omega))).2.1
      exact ⟨i, j, hij, by omega, by omega, by omega⟩
  -- the states along the cycle
  let st : Nat → State := fun y => c.allStates.getD (y - 1) default
  have hst : ∀ y, 0 < y → y < cc.size → st y ∈ c.allStates ∧ pos c (st y) = y := by
    intro y h0 hy
    have hy' : y - 1 < c.allStates.length := by omega
    have : st y = c.allStates[y - 1] := by
      simp only [st, List.getD_eq_getElem?_getD, List.getElem?_eq_getElem hy', Option.getD_some]
    rw [this]; exact ⟨List.getElem_mem _, by rw [pos_getElem hnd]; omega⟩
  have hedgeIR : ∀ k, k < m → st (f (k + 1)) ∈ historyDefaultEdges c (st (f k)) := by
    intro k hk
    obtain ⟨u, hu, v, hv, hpu, hpv, he⟩ := hedge_ir hwf hsp (hstep k hk)
    obtain ⟨_, h0, hs⟩ := hedge_hist hwf hsp (hstep k hk)
    have hsu := hst (f k) h0 hs
    have hu' : st (f k) = u := pos_inj hnd hsu.1 hu (hsu.2.trans hpu.symm)
    have hx0 : 0 < f (k + 1) := by rw [← hpv]; exact pos_pos
    have hxs : f (k + 1) < cc.size := by rw [← hpv]; exact pos_lt_size hsp hv
    have hsv := hst (f (k + 1)) hx0 hxs
    have hv' : st (f (k + 1)) = v := pos_inj hnd hsv.1 hv (hsv.2.trans hpv.symm)
    rw [hu', hv']; exact he
  obtain ⟨hhi, h0i, hsi⟩ := hedge_hist hwf hsp (hstep i (by omega))
  have hu := hst (f i) h0i hsi
  have hreach := reach_of_walk hwf hsp (fun b => st (f (i + 1 + b))) (j - i - 1)
    c.allStates.length (historyDefaultEdges c (st (f i))) (by omega)
    (by simpa using hedgeIR i (by omega))
    (fun b hb => by
      have := hedgeIR (i + 1 + b) (by omega)
      simpa [Nat.add_assoc] using this)
  have hji : i + 1 + (j - i - 1) = j := by omega
  simp only [hji, ← hfij] at hreach
  have hhist : (st (f i)).isHistory = true := by
    rw [← isHistory_pos hwf hsp hu.1, hu.2]; exact hhi
  exact hwf.2.2.2.2.2.2.2.2.2 (st (f i)) hu.1 hhist (List.mem_map_of_mem hreach)

open Classical in
/-- The rank: the number of states reachable along history default edges. -/
noncomputable def rank (cc : CChart) (h : Nat) : Nat :=
  ((List.range cc.size).filter (fun y => decide (Relation.TransGen (HEdge cc) h y))).length

open Classical in
theorem history_rank' : ∃ r : Nat → Nat, (∀ h, cc.isHistory h = true → r h < cc.size) ∧
    ∀ h d x, cc.isHistory h = true → cc.historyDefault h = some d → x ∈ (cc.tr d).targets →
      cc.isHistory x = true → r x < r h := by
  refine ⟨rank cc, fun h hh => ?_, fun h d x hh hd hx hxh => ?_⟩
  · have hlt : h < cc.size := by
      by_cases hlt : h < cc.size
      · exact hlt
      · unfold CChart.isHistory at hh; rw [st_ge (by omega)] at hh; cases hh
    have := filter_length_lt (fun y => decide (Relation.TransGen (HEdge cc) h y)) (fun _ => true)
      (List.range cc.size) (fun _ _ _ => rfl)
      ⟨h, List.mem_range.2 hlt, rfl, by simpa using hedge_acyclic hwf hsp h⟩
    have h2 : ∀ l : List Nat, (l.filter (fun _ => true)).length = l.length := by
      intro l
      induction l with
      | nil => rfl
      | cons a l ih => simp only [List.filter_cons, ↓reduceIte, List.length_cons, ih]
    have h3 := h2 (List.range cc.size)
    rw [List.length_range] at h3
    unfold rank; omega
  · have he : HEdge cc h x := ⟨hh, d, hd, hx, hxh⟩
    have hxs := (good_targets_lt hwf hsp d x hx).2
    unfold rank
    apply filter_length_lt
    · intro y _ hy
      simp only [decide_eq_true_eq] at hy ⊢
      exact transGen_head he hy
    · exact ⟨x, List.mem_range.2 hxs, by simpa using Relation.TransGen.single he,
        by simpa using hedge_acyclic hwf hsp x⟩

end

end Vertep.CompileProof
