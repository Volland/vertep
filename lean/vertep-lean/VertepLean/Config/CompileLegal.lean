import VertepLean.Config.CompileFields

/-!
# Compile correctness: legal target sets and history defaults

IR target sets that pass `TargetSetOk` (pairwise `PairOk` with `LcaParallel`) become legal
compiled target sets (pairwise `Compatible` positions); the regions of a parallel state are
pairwise compatible; every history state has one default transition with targets below its
parent.
-/

set_option linter.unusedSectionVars false

namespace Vertep.CompileProof

open Vertep

section
variable {c : Chart} {cc : CChart} (hwf : WellFormed c) (hsp : Spec c cc)
include hwf hsp

theorem compatible_siblings {p a b : Nat} (hpar : cc.isParallel p = true)
    (ha : a ∈ (cc.st p).children) (hb : b ∈ (cc.st p).children) (hne : a ≠ b) :
    cc.Compatible a b := by
  have hpa := ((children_iff' hwf hsp p a).1 ha).2.1
  have hpb := ((children_iff' hwf hsp p b).1 hb).2.1
  have hlta := parent_lt_of hwf hsp hpa
  have hltb := parent_lt_of hwf hsp hpb
  have nd : ∀ {x y : Nat}, (cc.st x).parent = some p → (cc.st y).parent = some p →
      cc.isDesc x y = false := by
    intro x y hx hy
    cases h : cc.isDesc x y
    · rfl
    · rcases cc.parent_desc_or_eq h hx with rfl | h'
      · have := parent_lt_of hwf hsp hy; omega
      · have := cc.isDesc_lt h'; have := parent_lt_of hwf hsp hy; omega
  exact ⟨hne, nd hpa hpb, nd hpb hpa, p, hpar, cc.isDesc_of_parent hpa hlta,
    cc.isDesc_of_parent hpb hltb, fun q' h1 _ => (cc.parent_desc_or_eq h1 hpa).imp_left Eq.symm⟩

theorem pos_nonhist {x : Nat} (h : cc.isHistory x = false) : cc.pos x = x := by
  unfold CChart.pos; rw [h]; rfl

theorem legal_children {p : Nat} (hpar : cc.isParallel p = true) :
    cc.TargetsLegal (cc.st p).children := by
  unfold CChart.TargetsLegal
  refine (children_nodup' hwf hsp p).imp_of_mem (fun {a b} ha hb hne => ?_)
  rw [pos_nonhist hwf hsp ((children_iff' hwf hsp p a).1 ha).2.2,
    pos_nonhist hwf hsp ((children_iff' hwf hsp p b).1 hb).2.2]
  exact compatible_siblings hwf hsp hpar ha hb hne

omit hsp in
theorem ancestorsF_mem : ∀ (f : Nat) (s : State), s ∈ c.allStates → ∀ x ∈ c.ancestorsF f s,
    x ∈ c.allStates
  | 0, _, _, x, hx => by simp [Chart.ancestorsF] at hx
  | f + 1, s, hs, x, hx => by
    simp only [Chart.ancestorsF] at hx
    split at hx
    · rename_i q hq
      have hq' := (parentOf_some (wf_nodup hwf) hs hq).1
      rcases List.mem_cons.1 hx with rfl | hx
      · exact hq'
      · exact ancestorsF_mem f q hq' x hx
    · simp at hx

omit hwf hsp in
theorem ancestors_pairwise (cc : CChart) : ∀ i, (cc.ancestors i).Pairwise (fun a b => b < a) := by
  intro i
  induction i using Nat.strongRecOn with
  | ind i ih =>
    rw [CChart.ancestors]
    split
    · rename_i p _
      split
      · rename_i hlt
        exact List.Pairwise.cons (fun b hb => cc.mem_ancestors_lt p b hb) (ih p hlt)
      · exact List.Pairwise.nil
    · exact List.Pairwise.nil

theorem tp_pos {u : State} (hu : u ∈ c.allStates) :
    ∃ w ∈ c.allStates, targetPosition c u.id = some w ∧ cc.pos (pos c u) = pos c w := by
  have hnd := wf_nodup hwf
  unfold targetPosition
  rw [find_eq hnd hu]
  simp only
  cases hh : u.isHistory
  · refine ⟨u, hu, by simp, ?_⟩
    rw [pos_nonhist hwf hsp (by rw [isHistory_pos hwf hsp hu, hh])]
  · cases hq : c.parentOf u with
    | none => rw [top_not_history hwf (parentOf_none hnd hu hq)] at hh; cases hh
    | some q =>
      refine ⟨q, (parentOf_some hnd hu hq).1, by simp, ?_⟩
      unfold CChart.pos
      rw [isHistory_pos hwf hsp hu, hh, st_parent hwf hsp hu]
      unfold cpar; rw [hq]; rfl

theorem pairOk_compatible {w w' : State} (hw : w ∈ c.allStates) (hw' : w' ∈ c.allStates)
    (h : PairOk c w w') : cc.Compatible (pos c w) (pos c w') := by
  have hnd := wf_nodup hwf
  obtain ⟨hne, hd1, hd2, hl⟩ := h
  have nd : ∀ {x y : State}, x ∈ c.allStates → y ∈ c.allStates → x.id ∉ y.descIds →
      cc.isDesc (pos c x) (pos c y) = false := by
    intro x y hx hy hn
    cases h : cc.isDesc (pos c x) (pos c y)
    · rfl
    · exact absurd ((mem_descIds_iff hwf hsp hx hy).2 h) hn
  refine ⟨fun he => hne (by rw [pos_inj hnd hw hw' he]), nd hw hw' hd1, nd hw' hw hd2, ?_⟩
  unfold LcaParallel at hl
  split at hl
  · rename_i p hfind
    obtain ⟨hpred, as, bs, hsplit, hbefore⟩ := List.find?_eq_some_iff_append.1 hfind
    have hpA : p ∈ c.allStates :=
      ancestorsF_mem hwf _ w hw p (by unfold Chart.ancestors at hsplit; rw [hsplit]; simp)
    have hanc := ancestors_pos' hwf hsp hw
    have hwp : cc.isDesc (pos c w) (pos c p) = true := by
      rw [cc.isDesc_iff_mem, hanc, hsplit]; simp
    refine ⟨pos c p, (isParallel_pos hwf hsp hpA).2 hl, hwp,
      (mem_descIds_iff hwf hsp hw' hpA).1 (of_decide_eq_true hpred), fun q' h1 h2 => ?_⟩
    rcases cc.isDesc_chain h1 hwp with h | h | h
    · exact Or.inl h
    · exfalso
      have hq'0 : 0 < q' := Nat.lt_of_le_of_lt (Nat.zero_le _)
        (Nat.lt_trans (pos_pos (c := c) (s := p)) (cc.isDesc_lt h))
      obtain ⟨v, hv, rfl⟩ := exists_pos hwf hsp hq'0 (cc.isDesc_lt_size h)
      have hmem : pos c v ∈ (as ++ p :: bs).map (pos c) := by
        have := (cc.isDesc_iff_mem _ _).1 h1
        rw [hanc, hsplit, List.mem_append] at this
        rcases this with h' | h'
        · exact h'
        · simp only [List.mem_singleton] at h'; exact absurd h' (Nat.ne_of_gt pos_pos)
      obtain ⟨v', hv', hvv⟩ := List.mem_map.1 hmem
      have hv'A : v' ∈ c.allStates :=
        ancestorsF_mem hwf _ w hw v' (by unfold Chart.ancestors at hsplit; rw [hsplit]; exact hv')
      rw [pos_inj hnd hv'A hv hvv] at hv'
      have hpv : w'.id ∈ v.descIds := (mem_descIds_iff hwf hsp hw' hv).2 h2
      rcases List.mem_append.1 hv' with hva | hvb
      · have := hbefore v hva
        simp [hpv] at this
      · rcases List.mem_cons.1 hvb with rfl | hvb
        · rw [cc.isDesc_irrefl] at h; cases h
        · have hpw := ancestors_pairwise cc (pos c w)
          rw [hanc, hsplit] at hpw
          have hsub : ((p :: bs).map (pos c)).Sublist ((as ++ p :: bs).map (pos c) ++ [0]) :=
            ((List.sublist_append_right as (p :: bs)).map _).trans (List.sublist_append_left _ _)
          have := (List.pairwise_cons.1 (hpw.sublist hsub)).1 (pos c v) (List.mem_map_of_mem hvb)
          have := cc.isDesc_lt h
          omega
    · exact Or.inr h
  · exact absurd hl id

theorem good1_legal {tg : List Nat} (h : Good1 c tg) : cc.TargetsLegal tg := by
  have hnd := wf_nodup hwf
  obtain ⟨L, rfl, _, hT⟩ := h
  unfold CChart.TargetsLegal Compile.resolve
  rw [List.pairwise_filterMap]
  unfold TargetSetOk at hT
  rw [List.pairwise_filterMap] at hT
  refine hT.imp_of_mem (fun {a a'} _ _ H => ?_)
  intro b hb b' hb'
  obtain ⟨u, hu, rfl, rfl⟩ := idx_some hnd hb
  obtain ⟨u', hu', rfl, rfl⟩ := idx_some hnd hb'
  obtain ⟨w, hw, htp, hpw⟩ := tp_pos hwf hsp hu
  obtain ⟨w', hw', htp', hpw'⟩ := tp_pos hwf hsp hu'
  rw [hpw, hpw']
  exact pairOk_compatible hwf hsp hw hw' (H w htp w' htp')

theorem targets_legal' (t : Nat) : cc.TargetsLegal (cc.tr t).targets := by
  rcases hsp.good t with h | h
  · exact good1_legal hwf hsp h
  · obtain ⟨p, hp, hpar, _⟩ := good2_parallel hwf hsp h
    rw [hp]; exact legal_children hwf hsp hpar

theorem history_default' (h : Nat) (hh : cc.isHistory h = true) :
    ∃ d, (cc.st h).transitions = [d] ∧ (cc.tr d).targets ≠ [] ∧
      ∀ x ∈ (cc.tr d).targets, ∀ p, (cc.st h).parent = some p → cc.isDesc x p = true := by
  have hnd := wf_nodup hwf
  have h0 : 0 < h := by
    rcases Nat.eq_zero_or_pos h with rfl | h0
    · rw [isHistory_zero hwf hsp] at hh; cases hh
    · exact h0
  have hlt : h < cc.size := by
    by_cases hlt : h < cc.size
    · exact hlt
    · unfold CChart.isHistory at hh; rw [st_ge (by omega)] at hh; cases hh
  obtain ⟨s, hs, rfl⟩ := exists_pos hwf hsp h0 hlt
  have hsh : s.isHistory = true := by rw [← isHistory_pos hwf hsp hs]; exact hh
  obtain ⟨_, _, _, _, htrs, hhist, _⟩ := stateOk_of hwf hsp hs
  have hho := hhist hsh
  obtain ⟨q, hq⟩ : ∃ q, c.parentOf s = some q := by
    cases hq : c.parentOf s with
    | none => rw [top_not_history hwf (parentOf_none hnd hs hq)] at hsh; cases hsh
    | some q => exact ⟨q, rfl⟩
  obtain ⟨hqA, hsq⟩ := parentOf_some hnd hs hq
  have hpar : (cc.st (pos c s)).parent = some (pos c q) := by
    rw [st_parent hwf hsp hs]; unfold cpar; rw [hq]
  obtain ⟨k, hk, he, hp⟩ := at_index hwf hsp hs
  have hIH : IsHist (cc.st (k + 1)) := by
    unfold IsHist; rw [← hp, st_kind hwf hsp hs]; exact (kindOf_history' s).2 hsh
  by_cases hemp : s.transitions = []
  · obtain ⟨t, h1, _, _, h4⟩ := hsp.hist k hk hIH (by rw [he]; exact hemp)
    rw [← hp] at h1 h4
    refine ⟨t, h1, ?_⟩
    rw [h4]
    unfold synth
    have hpar' : (cc.states.getD (pos c s) CState.empty).parent = some (pos c q) := hpar
    rw [hpar']
    simp only [Option.getD_some]
    split
    · rename_i ti hti
      obtain ⟨_, _, hne, hd⟩ := initial_ok' hwf hsp (pos c q) ti hti
      refine ⟨hne, fun x hx p hp' => ?_⟩
      rw [hpar] at hp'; cases hp'; exact hd x hx
    · rename_i hnone
      obtain ⟨p', hp', _, hne, _⟩ := good2_parallel hwf hsp
        (tg := (cc.states.getD (pos c q) CState.empty).children)
        ⟨pos c s, pos c q, rfl, by rw [hpar']; rfl,
          by rw [← hp] at hIH; exact hIH, hnone⟩
      refine ⟨by rw [hp']; exact hne, fun x hx p hp'' => ?_⟩
      rw [hpar] at hp''; cases hp''
      have := ((children_iff' hwf hsp _ x).1 hx).2.1
      exact cc.isDesc_of_parent this (parent_lt_of hwf hsp this)
  · obtain ⟨tr, htr⟩ : ∃ tr, s.transitions = [tr] := by
      have := hho.1
      match h' : s.transitions, hemp, this with
      | [tr], _, _ => exact ⟨tr, rfl⟩
      | [], hn, _ => exact absurd rfl hn
      | _ :: _ :: _, _, hl => simp at hl
    have hd := hsp.decl k hk (fun hc => hemp (by rw [← he]; exact hc.2))
    rw [← hp, he] at hd
    obtain ⟨hlen, hto⟩ := hd
    rw [htr] at hlen
    obtain ⟨t, ht⟩ : ∃ t, (cc.st (pos c s)).transitions = [t] := by
      match h' : (cc.st (pos c s)).transitions, hlen with
      | [t], _ => exact ⟨t, rfl⟩
    refine ⟨t, ht, ?_⟩
    obtain ⟨_, _, _, tr', htr', htg⟩ := hto t (by rw [ht]; simp)
    rw [htr, List.mem_singleton] at htr'; subst htr'
    have htg' : (cc.tr t).targets = Compile.resolve c.ids tr'.targets := htg
    obtain ⟨_, _, hne, hdesc⟩ := hho.2.1 tr' (by rw [htr]; rfl)
    have hids := (htrs tr' (by rw [htr]; simp)).1
    rw [htg']
    refine ⟨resolve_ne_nil hwf hsp hne hids, fun x hx p hp' => ?_⟩
    rw [hpar] at hp'; cases hp'
    obtain ⟨u, hu, hid, rfl⟩ := resolve_mem hwf hsp hx
    exact (mem_descIds_iff hwf hsp hu hqA).1 (hdesc q hq u.id hid (hids _ hid))

end

end Vertep.CompileProof
