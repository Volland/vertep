import VertepLean.Rdf.Equiv.Cycle
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Conformance forces unique state identifiers

States with one identifier share a node. In a conforming projection a state node has one
container (`[^vt:child] maxCount 1`) and one `vt:order`, and the chart node has none; so the
chain of containers from a state node up to the chart node is a function of the node
(`chain_func`). Two states with one identifier then have the same chain, which a tree only
allows for one state (`nodup_of_props`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

theorem compsHold_mem {G : Graph} {p : Option Path} {x : Term} :
    ∀ {cs : List Component}, CompsHold G p cs x → ∀ c ∈ cs, CompHolds G p c x
  | [], _, c, hc => by simp at hc
  | d :: ds, h, c, hc => by
    rw [compsHold_cons] at h
    rcases List.mem_cons.1 hc with rfl | hc
    · exact h.1
    · exact compsHold_mem h.2 c hc

section
variable (k : Ctx) (ts : List Triple) (C : Term)

/-- The containers of a node, up to the chart node `C`. -/
def Chain : List Term → Prop
  | [] => False
  | [x] => x = C
  | x :: y :: l => (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ ts ∧ (⟨y, vt "child", x⟩ : Triple) ∈ ts ∧
      Chain (y :: l)

variable (P1 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ ts → ∀ p q,
    (⟨p, vt "child", x⟩ : Triple) ∈ ts → (⟨q, vt "child", x⟩ : Triple) ∈ ts → p = q)
  (P2 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ ts → ∀ o o',
    (⟨x, vt "order", o⟩ : Triple) ∈ ts → (⟨x, vt "order", o'⟩ : Triple) ∈ ts → o = o')
  (P3 : ∀ p, (⟨p, vt "child", C⟩ : Triple) ∉ ts)

include P1 P3 in
theorem chain_func : ∀ (L L' : List Term), Chain ts C L → Chain ts C L' → L.head? = L'.head? → L = L'
  | [], _, h, _, _ => h.elim
  | _, [], _, h, _ => h.elim
  | [x], [x'], h, h', _ => by simp only [Chain] at h h'; rw [h, h']
  | [x], x' :: y' :: l', h, h', he => by
    simp only [Chain, List.head?_cons, Option.some.injEq] at h h' he
    subst he; subst h; exact absurd h'.2.1 (P3 _)
  | x :: y :: l, [x'], h, h', he => by
    simp only [Chain, List.head?_cons, Option.some.injEq] at h h' he
    subst he; subst h'; exact absurd h.2.1 (P3 _)
  | x :: y :: l, x' :: y' :: l', h, h', he => by
    simp only [Chain, List.head?_cons, Option.some.injEq] at h h' he
    subst he
    have hy := P1 x h.1 y y' h.2.1 h'.2.1
    subst hy
    rw [chain_func (y :: l) (y :: l') h.2.2 h'.2.2 rfl]

end

section
variable (k : Ctx) (ts : List Triple) (C : Term)

/-- The node of a state. -/
abbrev N (s : State) : Term := k.iri (stateFragment s.id)

mutual
theorem occ_state (i : Nat) (p : Term) (A : List Term) : ∀ (s : State),
    (∀ u ∈ k.state i p A s, u ∈ ts) → ∀ (Cp : List Term), Chain ts C Cp → Cp.head? = some p →
      ∀ Z ∈ s.flatten, ∃ D : List Term, D ≠ [] ∧ D.getLast? = some (N k s) ∧ D.head? = some (N k Z) ∧
        Chain ts C (D ++ Cp)
  | .mk id ig el ini ie cs trs en ex da inv dd xs, hsub, Cp, hC, hp, Z, hZ => by
    have hst : ∀ u ∈ k.state i p A (.mk id ig el ini ie cs trs en ex da inv dd xs), u ∈ ts := hsub
    rw [state_eq'] at hsub
    have hty : (⟨k.iri (stateFragment id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts :=
      hsub _ (by simp [stateSelf, Ctx.node, State.id])
    have hch : (⟨p, vt "child", k.iri (stateFragment id)⟩ : Triple) ∈ ts :=
      hsub _ (by simp [stateSelf, Ctx.node, State.id])
    have hC' : Chain ts C (k.iri (stateFragment id) :: Cp) := by
      match Cp, hC, hp with
      | q :: l, hC, hp =>
        simp only [List.head?_cons, Option.some.injEq] at hp; subst hp
        exact ⟨hty, hch, hC⟩
    rw [flatten_eq] at hZ
    rcases List.mem_cons.1 hZ with rfl | hZ
    · exact ⟨[k.iri (stateFragment id)], by simp, rfl, rfl, hC'⟩
    · obtain ⟨D, c', _, _, _, hD, hl, hh, hc⟩ := occ_states 0 (k.iri (stateFragment id))
        (A ++ [k.iri (stateFragment id)]) cs (fun u hu => hsub u (List.mem_append_right _ hu)) _ hC' rfl Z hZ
      refine ⟨D ++ [k.iri (stateFragment id)], by simp, by simp [N, State.id], ?_, by simpa using hc⟩
      cases D with
      | nil => exact absurd rfl hD
      | cons a l => simpa using hh
theorem occ_states (i : Nat) (p : Term) (A : List Term) : ∀ (cs : List State),
    (∀ u ∈ k.states i p A cs, u ∈ ts) → ∀ (Cp : List Term), Chain ts C Cp → Cp.head? = some p →
      ∀ Z ∈ State.flattenList cs, ∃ D : List Term, ∃ c' : State, ∃ j, i ≤ j ∧
        (⟨N k c', vt "order", intLit j⟩ : Triple) ∈ ts ∧
        D ≠ [] ∧ D.getLast? = some (N k c') ∧ D.head? = some (N k Z) ∧ Chain ts C (D ++ Cp)
  | [], _, _, _, _, Z, hZ => by simp [State.flattenList] at hZ
  | c :: cs, hsub, Cp, hC, hp, Z, hZ => by
    rw [Ctx.states] at hsub
    rw [flattenList_cons, List.mem_append] at hZ
    rcases hZ with hZ | hZ
    · obtain ⟨D, h1, h2, h3, h4⟩ := occ_state i p A c (fun u hu => hsub u (List.mem_append_left _ hu))
        Cp hC hp Z hZ
      refine ⟨D, c, i, Nat.le_refl _, ?_, h1, h2, h3, h4⟩
      have := fun u hu => hsub u (List.mem_append_left _ hu)
      rw [state_eq'] at this
      exact this _ (by simp [stateSelf, ordT, int, N])
    · obtain ⟨D, c', j, hj, ho, h1, h2, h3, h4⟩ := occ_states (i + 1) p A cs
        (fun u hu => hsub u (List.mem_append_right _ hu)) Cp hC hp Z hZ
      exact ⟨D, c', j, by omega, ho, h1, h2, h3, h4⟩
end

variable (P1 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ ts → ∀ p q,
    (⟨p, vt "child", x⟩ : Triple) ∈ ts → (⟨q, vt "child", x⟩ : Triple) ∈ ts → p = q)
  (P2 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ ts → ∀ o o',
    (⟨x, vt "order", o⟩ : Triple) ∈ ts → (⟨x, vt "order", o'⟩ : Triple) ∈ ts → o = o')
  (P3 : ∀ p, (⟨p, vt "child", C⟩ : Triple) ∉ ts)
include P1 P2 P3

theorem N_inj {a b : State} (h : N k a = N k b) : a.id = b.id := stateFragment_inj ((iri_inj _).1 h)

mutual
theorem nd_state (i : Nat) (p : Term) (A : List Term) : ∀ (s : State),
    (∀ u ∈ k.state i p A s, u ∈ ts) → ∀ (Cp : List Term), Chain ts C Cp → Cp.head? = some p →
      (s.flatten.map State.id).Nodup
  | .mk id ig el ini ie cs trs en ex da inv dd xs, hsub, Cp, hC, hp => by
    rw [state_eq'] at hsub
    have hty : (⟨k.iri (stateFragment id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts :=
      hsub _ (by simp [stateSelf, Ctx.node, State.id])
    have hch : (⟨p, vt "child", k.iri (stateFragment id)⟩ : Triple) ∈ ts :=
      hsub _ (by simp [stateSelf, Ctx.node, State.id])
    have hC' : Chain ts C (k.iri (stateFragment id) :: Cp) := by
      match Cp, hC, hp with
      | q :: l, hC, hp =>
        simp only [List.head?_cons, Option.some.injEq] at hp; subst hp
        exact ⟨hty, hch, hC⟩
    have hsub' : ∀ u ∈ k.states 0 (k.iri (stateFragment id)) (A ++ [k.iri (stateFragment id)]) cs, u ∈ ts :=
      fun u hu => hsub u (List.mem_append_right _ hu)
    rw [flatten_eq, List.map_cons, List.nodup_cons]
    refine ⟨fun hm => ?_, nd_states 0 _ _ cs hsub' _ hC' rfl⟩
    obtain ⟨Z, hZ, hZid⟩ := List.mem_map.1 hm
    obtain ⟨D, c', _, _, _, hD, _, hh, hc⟩ := occ_states k ts C 0 _ _ cs hsub' _ hC' rfl Z hZ
    have := chain_func ts C P1 P3 _ _ hc hC' (by
      cases D with
      | nil => exact absurd rfl hD
      | cons a l =>
        simp only [List.cons_append, List.head?_cons] at hh ⊢
        rw [hh]; simp only [N, Option.some.injEq]; rw [hZid]; rfl)
    have := congrArg List.length this
    simp only [List.length_append, List.length_cons] at this
    have : D.length ≠ 0 := fun h => hD (List.length_eq_zero_iff.1 h)
    omega
theorem nd_states (i : Nat) (p : Term) (A : List Term) : ∀ (cs : List State),
    (∀ u ∈ k.states i p A cs, u ∈ ts) → ∀ (Cp : List Term), Chain ts C Cp → Cp.head? = some p →
      ((State.flattenList cs).map State.id).Nodup
  | [], _, _, _, _ => by simp [State.flattenList]
  | c :: cs, hsub, Cp, hC, hp => by
    rw [Ctx.states] at hsub
    have h1 : ∀ u ∈ k.state i p A c, u ∈ ts := fun u hu => hsub u (List.mem_append_left _ hu)
    have h2 : ∀ u ∈ k.states (i + 1) p A cs, u ∈ ts := fun u hu => hsub u (List.mem_append_right _ hu)
    rw [flattenList_cons, List.map_append, List.nodup_append]
    refine ⟨nd_state i p A c h1 Cp hC hp, nd_states (i + 1) p A cs h2 Cp hC hp, ?_⟩
    intro x hx y hy hxy
    subst hxy
    obtain ⟨Z1, hZ1, rfl⟩ := List.mem_map.1 hx
    obtain ⟨Z2, hZ2, hZid⟩ := List.mem_map.1 hy
    obtain ⟨D1, hD1, hl1, hh1, hc1⟩ := occ_state k ts C i p A c h1 Cp hC hp Z1 hZ1
    obtain ⟨D2, c', j, hj, ho, hD2, hl2, hh2, hc2⟩ := occ_states k ts C (i + 1) p A cs h2 Cp hC hp Z2 hZ2
    have he := chain_func ts C P1 P3 _ _ hc1 hc2 (by
      cases D1 with
      | nil => exact absurd rfl hD1
      | cons a l =>
        cases D2 with
        | nil => exact absurd rfl hD2
        | cons b m =>
          simp only [List.cons_append, List.head?_cons] at hh1 hh2 ⊢
          rw [hh1, hh2]; simp [N, hZid])
    have hD := List.append_cancel_right he
    subst hD
    rw [hl1] at hl2
    have hcc : N k c = N k c' := Option.some.inj hl2
    have hoc : (⟨N k c, vt "order", intLit i⟩ : Triple) ∈ ts := by
      have := h1; rw [state_eq'] at this
      exact this _ (by simp [stateSelf, ordT, int, N])
    have htc : (⟨N k c, rdfType, vt "StateNode"⟩ : Triple) ∈ ts := by
      have := h1; rw [state_eq'] at this
      exact this _ (by simp [stateSelf, Ctx.node, N])
    rw [← hcc] at ho
    have := P2 _ htc _ _ hoc ho
    rw [intLit_inj] at this
    omega
end

end

/-- **Conformance to the chart shapes forces unique state identifiers.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "unique state identifiers; the full theorem is shapes_equivalence"}
theorem conforms_nodup (c : Chart) (base : String)
    (h : (Shacl.validate (toRDF c base) chartShapesL).conforms = true) : c.ids.Nodup := by
  rw [validate_conforms, dataConforms_iff _ (noSub c base)] at h
  have hg := h.1
  have hG : ∀ t, t ∈ toRDF c base ↔ t ∈ project c base := fun t => by simp [toRDF]
  have hsn : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ project c base →
      Conforms (toRDF c base) stateNodeS x := fun x hx =>
    ((good_iff _ (noSub c base) x).1 (hg x)).2.2.1 (by simpa [Ty, hG] using hx)
  have P1 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ project c base → ∀ p q,
      (⟨p, vt "child", x⟩ : Triple) ∈ project c base → (⟨q, vt "child", x⟩ : Triple) ∈ project c base →
      p = q := by
    intro x hx p q hp hq
    have h0 := hsn x hx
    rw [stateNodeS, conforms_top] at h0
    have := compsHold_mem h0 (.property (propS (I "child") [maxC 1])) (by simp)
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil,
      and_true, maxC, holds_maxCount1, vn_some, ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred,
      iri_vi, hG] at this
    exact this p q hp hq
  have P2 : ∀ x, (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ project c base → ∀ o o',
      (⟨x, vt "order", o⟩ : Triple) ∈ project c base → (⟨x, vt "order", o'⟩ : Triple) ∈ project c base →
      o = o' := by
    intro x hx o o' ho ho'
    have h0 := hsn x hx
    rw [stateNodeS, conforms_top] at h0
    have := compsHold_mem h0 (.node orderedS) (by simp)
    simp only [holds_node, vn_none, forall_eq, orderedS, conforms_nodeS, compsHold_cons, compsHold_nil,
      and_true, pr, holds_property, conforms_propS, maxC, holds_maxCount1, vn_some, ShapesL.P,
      valueNode_pred, iri_vi, hG] at this
    exact this.2.2.1 o o' ho ho'
  have hch : (⟨(ctxOf c base).iri "chart", rdfType, vt "Chart"⟩ : Triple) ∈ project c base := by
    rw [project_eq]; simp [chartSelf]
  have P3 : ∀ p, (⟨p, vt "child", (ctxOf c base).iri "chart"⟩ : Triple) ∉ project c base := by
    intro p hp
    have hc := ((good_iff _ (noSub c base) _).1 (hg _)).2.1 (by simpa [Ty, hG] using hch)
    rw [chartS, conforms_top] at hc
    have := compsHold_mem hc
      (.property (propS (.alt [I "child", I "action", I "extension"]) [maxC 0])) (by simp)
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil,
      and_true, maxC, holds_maxCount0, vn_some, valueNode_alt, ShapesL.I, ShapesL.P, valueNode_inv,
      valueNode_pred, iri_vi, hG] at this
    exact this p ⟨_, List.mem_cons_self, by simpa [hG] using hp⟩
  have hsub : ∀ u ∈ (ctxOf c base).states 0 ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"]
      c.states, u ∈ project c base := by
    intro u hu; rw [project_eq]; simp only [List.mem_append]; exact .inl (.inl (.inr hu))
  exact nd_states (ctxOf c base) (project c base) _ P1 P2 P3 0 _ _ c.states hsub
    [(ctxOf c base).iri "chart"] rfl rfl

end Vertep.Rdf.Equiv
