import VertepLean.Rdf.Equiv.Lcp
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Selection nodes of two targets

A target with path `p` gets the selection chain `chainOf p`: the chart, then the states of its
path. At each entry `x` of the chain the projection writes a selection node with one `vt:choice`:
the next entry, the entry itself at a parallel state, or the target's own reference at the end.
`compat_iff`: the selection nodes of two targets with different references agree exactly when
their paths part below a parallel state, the condition `pairOk_iff` gives for `PairOk`.
-/

namespace Vertep.Rdf.Equiv

open Vertep.Rdf.RT

/-- The selection chain of a target with path `p`. -/
def chainOf (p : List State) : List (String × Bool) := ("chart", false) :: p.map fr

/-- The fragment of the `vt:choice` of entry `i` (`(x, par)`) of `chain`, with reference `r`. -/
def cfS (chain : List (String × Bool)) (i : Nat) (x : String) (par : Bool) (r : String) : String :=
  match chain[i + 1]? with
  | none => r
  | some (c, _) => if par then x else c

/-- The `vt:choice` fragment of state `i` of a path. -/
def val (p : List State) (a : Nat) (r : String) : String :=
  if h : a + 1 < p.length then
    (if p[a].element == .parallel then stateFragment p[a].id else stateFragment (p[a + 1]).id)
  else r

theorem el_beq_par (e : StateElement) : (e == .parallel) = true ↔ e = .parallel := by
  cases e <;> simp <;> rfl

theorem el_beq_par' (e : StateElement) : (e == .parallel) = false ↔ ¬ e = .parallel := by
  cases e <;> simp <;> rfl

theorem sf_ne {x r : String} (h : ':' ∈ r.toList) : stateFragment x ≠ r := by
  intro he; subst he; exact nc_stateFragment x h

theorem chart_ne_sf (x : String) : "chart" ≠ stateFragment x := by
  intro h
  have := congrArg String.toList h
  simp [stateFragment, String.toList_append] at this

theorem cf_zero {p : List State} (hp : 0 < p.length) (r : String) :
    cfS (chainOf p) 0 "chart" false r = stateFragment (p[0]).id := by
  simp only [cfS, chainOf, Nat.zero_add, List.getElem?_cons_succ, List.getElem?_map,
    List.getElem?_eq_getElem hp, Option.map_some, fr, Bool.false_eq_true, ↓reduceIte]

theorem cf_succ {p : List State} {a : Nat} (ha : a < p.length) (r : String) :
    cfS (chainOf p) (a + 1) (stateFragment (p[a]).id) (p[a].element == .parallel) r = val p a r := by
  unfold cfS val chainOf
  simp only [List.getElem?_cons_succ, List.getElem?_map]
  by_cases h : a + 1 < p.length
  · simp only [List.getElem?_eq_getElem h, Option.map_some, fr, h, ↓reduceDIte]
  · simp only [h, ↓reduceDIte]
    rw [List.getElem?_eq_none (by omega)]; rfl

@[simp] theorem chainOf_length (p : List State) : (chainOf p).length = p.length + 1 := by
  simp [chainOf]

@[simp] theorem chainOf_zero (p : List State) : (chainOf p)[0] = ("chart", false) := rfl

@[simp] theorem chainOf_succ (p : List State) (a : Nat) (h : a + 1 < (chainOf p).length) :
    (chainOf p)[a + 1] = fr (p[a]'(by simp at h; omega)) := by
  simp [chainOf]

section
variable {c : Chart} (hnd : c.ids.Nodup) {P P' : State} {p p' : List State}
  (he : (P, p) ∈ WP c) (he' : (P', p') ∈ WP c)
include hnd he he'

/-- The selection condition of two targets, by states of their paths. -/
theorem compat_states (r r' : String) :
    (∀ i i' (hi : i < (chainOf p).length) (hi' : i' < (chainOf p').length),
      (chainOf p)[i].1 = (chainOf p')[i'].1 →
      cfS (chainOf p) i (chainOf p)[i].1 (chainOf p)[i].2 r =
        cfS (chainOf p') i' (chainOf p')[i'].1 (chainOf p')[i'].2 r') ↔
    ((p[0]'(wp_len_pos he)).id = (p'[0]'(wp_len_pos he')).id ∧
      ∀ a (ha : a < p.length) (ha' : a < p'.length), p[a].id = p'[a].id → val p a r = val p' a r') := by
  have hp := wp_len_pos he
  have hp' := wp_len_pos he'
  constructor
  · intro H
    refine ⟨?_, fun a ha ha' hid => ?_⟩
    · have := H 0 0 (by simp) (by simp) rfl
      simp only [chainOf_zero] at this
      rw [cf_zero hp, cf_zero hp'] at this
      exact stateFragment_inj this
    · have := H (a + 1) (a + 1) (by simp; omega) (by simp; omega) (by simp [fr, hid])
      simp only [chainOf_succ, fr] at this
      rwa [cf_succ ha, cf_succ ha'] at this
  · rintro ⟨h0, H⟩ i i' hi hi' hx
    rcases i with _ | a <;> rcases i' with _ | a'
    · simp only [chainOf_zero]; rw [cf_zero hp, cf_zero hp', h0]
    · simp only [chainOf_zero, chainOf_succ, fr] at hx; exact absurd hx (chart_ne_sf _)
    · simp only [chainOf_zero, chainOf_succ, fr] at hx; exact absurd hx.symm (chart_ne_sf _)
    · simp only [chainOf_succ, fr] at hx ⊢
      have hid := stateFragment_inj hx
      simp only [chainOf_length] at hi hi'
      obtain ⟨rfl, _⟩ := wpp_prefix hnd he he' (by omega) (by omega) hid
      rw [cf_succ (by omega), cf_succ (by omega)]
      exact H a (by omega) (by omega) hid

/-- **The selection nodes of two targets with different references agree exactly when their
paths part below a parallel state.** -/
theorem compat_iff {r r' : String} (hr : r ≠ r') (hc : ':' ∈ r.toList) (hc' : ':' ∈ r'.toList) :
    (∀ i i' (hi : i < (chainOf p).length) (hi' : i' < (chainOf p').length),
      (chainOf p)[i].1 = (chainOf p')[i'].1 →
      cfS (chainOf p) i (chainOf p)[i].1 (chainOf p)[i].2 r =
        cfS (chainOf p') i' (chainOf p')[i'].1 (chainOf p')[i'].2 r') ↔
    ∃ (h0 : 0 < lcp (p.map State.id) (p'.map State.id)) (h1 : lcp (p.map State.id) (p'.map State.id) < p.length),
      lcp (p.map State.id) (p'.map State.id) < p'.length ∧
      (p[lcp (p.map State.id) (p'.map State.id) - 1]'(by omega)).element = .parallel := by
  have hp := wp_len_pos he
  have hp' := wp_len_pos he'
  have hle := lcp_le_left (p.map State.id) (p'.map State.id)
  have hle' := lcp_le_right (p.map State.id) (p'.map State.id)
  simp only [List.length_map] at hle hle'
  rw [compat_states hnd he he', agree_iff hnd he he' hp hp']
  -- states at agreeing positions are equal
  have heq : ∀ a (ha : a < p.length) (ha' : a < p'.length), a < lcp (p.map State.id) (p'.map State.id) →
      p[a] = p'[a] := fun a ha ha' h =>
    CompileProof.state_inj hnd (wpp_mem hnd he he' ha) (wpp_mem' hnd he he' ha')
      ((agree_iff hnd he he' ha ha').2 h)
  constructor
  · rintro ⟨h0, H⟩
    have hv := H (lcp (p.map State.id) (p'.map State.id) - 1) (by omega) (by omega)
      ((agree_iff hnd he he' (by omega) (by omega)).2 (by omega))
    unfold val at hv
    have hl : lcp (p.map State.id) (p'.map State.id) < p.length := by
      refine Classical.byContradiction fun hn => ?_
      simp only [show ¬ (lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p.length) from by omega, ↓reduceDIte] at hv
      by_cases hn' : lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p'.length
      · simp only [hn', ↓reduceDIte] at hv; split at hv <;> exact sf_ne hc hv.symm
      · simp only [hn', ↓reduceDIte] at hv; exact hr hv
    have hl' : lcp (p.map State.id) (p'.map State.id) < p'.length := by
      refine Classical.byContradiction fun hn => ?_
      simp only [show ¬ (lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p'.length) from by omega, ↓reduceDIte] at hv
      simp only [show lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p.length from by omega, ↓reduceDIte] at hv
      split at hv <;> exact sf_ne hc' hv
    refine ⟨h0, hl, hl', ?_⟩
    simp only [show lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p.length from by omega,
      show lcp (p.map State.id) (p'.map State.id) - 1 + 1 < p'.length from by omega, ↓reduceDIte] at hv
    have he1 := heq (lcp (p.map State.id) (p'.map State.id) - 1) (by omega) (by omega) (by omega)
    refine Classical.byContradiction fun hnp => ?_
    have hb : (p[lcp (p.map State.id) (p'.map State.id) - 1].element == .parallel) = false := by
      exact (el_beq_par' _).2 hnp
    have hb' : (p'[lcp (p.map State.id) (p'.map State.id) - 1].element == .parallel) = false := by
      rw [← he1]; exact hb
    rw [hb, hb'] at hv
    simp only [Bool.false_eq_true, ↓reduceIte] at hv
    have := lcp_lt hnd he he' hl hl'
    apply this
    have h2 := stateFragment_inj hv
    simp only [show lcp (p.map State.id) (p'.map State.id) - 1 + 1 = lcp (p.map State.id) (p'.map State.id)
      by omega] at h2
    exact h2
  · rintro ⟨h0, hl, hl', hpar⟩
    refine ⟨h0, fun a ha ha' hid => ?_⟩
    have hal := (agree_iff hnd he he' ha ha').1 hid
    unfold val
    simp only [show a + 1 < p.length from by omega, show a + 1 < p'.length from by omega, ↓reduceDIte]
    rw [← heq a ha ha' hal]
    by_cases ha1 : a + 1 < lcp (p.map State.id) (p'.map State.id)
    · rw [heq (a + 1) (by omega) (by omega) ha1]
    · have : a = lcp (p.map State.id) (p'.map State.id) - 1 := by omega
      subst this
      simp only [(el_beq_par _).2 hpar, ↓reduceIte]

end

end Vertep.Rdf.Equiv
