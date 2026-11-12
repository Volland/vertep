import VertepLean.Rdf.Equiv.OtherNodes
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on target references and transitions

A target reference node carries no node shape of its own beyond containment and, for the members
of an `initial` list, `vt:InitialRefScopeShape`; `vt:TargetRefShape` is checked from the owner of
the list: the target names a state of the chart (`Kn`) whose node lists the scope among its
`vt:ancestor`s.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

/-- The triples of a target reference node. -/
theorem refK_mem {owner : Term} {prop list step : String} {scope : Term} {x : String} {j : Nat}
    {t : Triple} (ht : t ∈ (refK k owner prop list step scope (x, j)).2) :
    t.p = vt "choice" ∨ t = ⟨owner, vt prop, k.iri (list ++ step ++ toString j)⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), rdfType, vt "TargetRef"⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "order", intLit j⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "state", k.iri (stateFragment x)⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "scope", scope⟩ := by
  simp only [refK, List.mem_append, Ctx.node, ordT, int, List.mem_cons, List.mem_map,
    List.not_mem_nil, or_false, List.map_cons, List.map_nil] at ht
  rcases ht with (((h | h) | h) | h | h) | h
  · exact .inr (.inl h)
  · exact .inr (.inr (.inl h))
  · exact .inr (.inr (.inr (.inl h)))
  · exact .inr (.inr (.inr (.inr (.inl h))))
  · exact .inr (.inr (.inr (.inr (.inr h))))
  · left
    split at h
    · simp at h
    · simp only [List.mem_flatMap] at h
      obtain ⟨_, _, h⟩ := h
      simp at h; rw [h]

theorem rg_ref (env : Env k ts P) {O prop list step : String} {scope : Term} {x : String} {j : Nat}
    (hcap : Cap k ts (U (refK k (k.iri O) prop list step scope (x, j)).1)
      (refK k (k.iri O) prop list step scope (x, j)).2)
    (hown : Above (k := k) (k.iri O) (refK k (k.iri O) prop list step scope (x, j)).1)
    (hc : Contained' k ts (k.iri (refK k (k.iri O) prop list step scope (x, j)).1) (k.iri O))
    (hinit : ∀ s, (⟨s, vt "initial", k.iri (refK k (k.iri O) prop list step scope (x, j)).1⟩ : Triple) ∈ ts →
      s = k.iri O ∧ scope = k.iri O) :
    RG k ts (refK k (k.iri O) prop list step scope (x, j)).1 := by
  have hne := above_ne hown
  have hmem := fun t (ht : t ∈ (refK k (k.iri O) prop list step scope (x, j)).2) => refK_mem (k := k) ht
  have htyE : (⟨k.iri (list ++ step ++ toString j), rdfType, vt "TargetRef"⟩ : Triple) ∈
      (refK k (k.iri O) prop list step scope (x, j)).2 := by simp [refK, Ctx.node]
  have hscE : (⟨k.iri (list ++ step ++ toString j), vt "scope", scope⟩ : Triple) ∈
      (refK k (k.iri O) prop list step scope (x, j)).2 := by simp [refK, Ctx.node]
  have hR : (refK k (k.iri O) prop list step scope (x, j)).1 = list ++ step ++ toString j := rfl
  rw [hR] at hcap hc hinit hne hown ⊢
  generalize (refK k (k.iri O) prop list step scope (x, j)).2 = E at hcap hmem htyE hscE
  generalize list ++ step ++ toString j = R at hcap hc hinit hne hown hmem htyE hscE ⊢
  rw [RG, region_leaf env (prop := prop) hcap hown (fun t ht hr => by
    rcases hmem t ht with h | h | h | h | h | h
    · exact absurd h hr.2
    · exact .inl h
    all_goals exact .inr (by rw [h]))]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri R) l ↔ l = "TargetRef" := by
    intro l; rw [ty_view hcap]
    constructor
    · intro h
      rcases hmem _ h with h | h | h | h | h | h <;> simp [Triple.mk.injEq] at h
      exact h
    · rintro rfl; exact htyE
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcap l o hl hc'
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  rintro ⟨s, hs⟩
  obtain ⟨rfl, rfl⟩ := hinit s hs
  simp only [initialRefScopeS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, holds_property,
    vn_none, forall_eq, propS, conforms_mk, holds_equals, vn_some, ShapesL.I, ShapesL.P, valueNode_inv,
    valueNode_pred, iri_vi, mem_ofList']
  intro v
  rw [hov _ _ (by decide) (by decide)]
  constructor
  · intro h
    have := (hc.conts v).1 ⟨"initial", by simp [containers], h⟩
    subst this; exact hscE
  · intro h
    rcases hmem _ h with h | h | h | h | h | h <;> simp [Triple.mk.injEq] at h
    · exact absurd (congrArg k.iri h.1.symm) hne
    · rw [h]; exact hs

/-- `vt:TargetRefShape` at a target reference node: its target is a known state that lists the
scope among its ancestors. -/
theorem targetRef_ok (env : Env k ts P) {O prop list step : String} {scope : Term} {x : String} {j : Nat}
    (hcap : Cap k ts (U (refK k (k.iri O) prop list step scope (x, j)).1)
      (refK k (k.iri O) prop list step scope (x, j)).2)
    (hown : Above (k := k) (k.iri O) (refK k (k.iri O) prop list step scope (x, j)).1)
    (Kn : String → Prop)
    (hSN : (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x) :
    Conforms (Graph.ofList ts) targetRefS (k.iri (refK k (k.iri O) prop list step scope (x, j)).1) ↔
      Kn x ∧ (⟨k.iri (stateFragment x), vt "ancestor", scope⟩ : Triple) ∈ ts := by
  have hne := above_ne hown
  have hmem := fun t (ht : t ∈ (refK k (k.iri O) prop list step scope (x, j)).2) => refK_mem (k := k) ht
  have hE : ∀ t, t = ⟨k.iri (list ++ step ++ toString j), rdfType, vt "TargetRef"⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "order", intLit j⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "state", k.iri (stateFragment x)⟩ ∨
      t = ⟨k.iri (list ++ step ++ toString j), vt "scope", scope⟩ →
      t ∈ (refK k (k.iri O) prop list step scope (x, j)).2 := by
    intro t ht; rcases ht with rfl | rfl | rfl | rfl <;> simp [refK, Ctx.node]
  have hR : (refK k (k.iri O) prop list step scope (x, j)).1 = list ++ step ++ toString j := rfl
  rw [hR] at hcap hne hown ⊢
  generalize (refK k (k.iri O) prop list step scope (x, j)).2 = E at hcap hmem hE
  generalize list ++ step ++ toString j = R at hcap hne hown hmem hE ⊢
  have hv : ∀ l o, l ≠ "choice" → l ≠ "historyDefault" → ((⟨k.iri R, vt l, o⟩ : Triple) ∈ ts ↔
      (l = "order" ∧ o = intLit j) ∨ (l = "state" ∧ o = k.iri (stateFragment x)) ∨
      (l = "scope" ∧ o = scope)) := by
    intro l o h1 h2
    rw [obj_view' hcap l o h2 h1]
    constructor
    · intro h
      rcases hmem _ h with h | h | h | h | h | h <;> simp [Triple.mk.injEq] at h
      · exact absurd h h1
      · exact absurd (congrArg k.iri h.1.symm) hne
      · exact .inl h
      · exact .inr (.inl h)
      · exact .inr (.inr h)
    · rintro (⟨rfl, rfl⟩ | ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩) <;> exact hE _ (by simp)
  have hty : (⟨k.iri R, rdfType, vt "TargetRef"⟩ : Triple) ∈ ts :=
    (hcap.lookup (U_refl R) (by simp)).2 (hE _ (.inl rfl))
  simp only [targetRefS, nodeS, conforms_mk, compsHold_cons, compsHold_nil, and_true, cls, holds_class,
    vn_none, forall_eq, instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq, mem_ofList', hty, true_and,
    holds_not, propS, vn_some, ShapesL.P, valueNode_seq_cons, valueNode_seq_nil, valueNode_pred, iri_vi,
    holds_disjoint, holds_node, orderedS, pr, dt, minC, maxC, holds_property, holds_test,
    holds_minCount1, holds_maxCount1]
  simp only [hv, ne_eq, String.reduceEq, not_false_eq_true, false_and, false_or, or_false, true_and,
    exists_eq_left, forall_eq, ok_dt_int, ok_minInclusive, exists_eq, implies_true, true_and, hSN,
    and_true]
  constructor
  · rintro ⟨h1, _, ⟨h2, _⟩, _⟩
    refine ⟨h2, Classical.byContradiction fun hn => h1 (fun v ⟨z, hz, he⟩ hs => by
      subst he; subst hs; exact hn hz)⟩
  · rintro ⟨h2, h3⟩
    exact ⟨fun h => h scope ⟨scope, h3, rfl⟩ rfl, fun y z hy hz => hy.trans hz.symm,
      ⟨h2, fun y z hy hz => hy.trans hz.symm⟩, fun y z hy hz => hy.trans hz.symm⟩

set_option maxHeartbeats 1000000 in
theorem reg_transition (env : Env k ts P) {tr : Transition} {owner : Term} {prop X : String}
    {order : Nat} {source scope : Term}
    (cx : Ctxt k ts owner prop X (k.transition tr owner prop X order source scope) (transKids k X tr scope))
    (hc : Contained' k ts (k.iri X) owner) (hni : ∀ s, (⟨s, vt "initial", k.iri X⟩ : Triple) ∉ ts)
    (hsrcT : (⟨source, rdfType, vt "StateNode"⟩ : Triple) ∈ ts)
    (hsrc : ∀ v, ((⟨v, vt "transition", k.iri X⟩ : Triple) ∈ ts ∨ ∃ z,
      (⟨z, vt "initialTransition", k.iri X⟩ : Triple) ∈ ts ∧ (⟨v, vt "initialElement", z⟩ : Triple) ∈ ts) ↔
        v = source)
    (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x) :
    RG k ts X ↔ (∀ x ∈ tr.targets, Kn x ∧
      (⟨k.iri (stateFragment x), vt "ancestor", scope⟩ : Triple) ∈ ts) ∧ BlockOk tr.body := by
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hself : ∀ (P : Term) (o : Term), Rel ⟨k.iri X, P, o⟩ →
      ((⟨k.iri X, P, o⟩ : Triple) ∈ k.transition tr owner prop X order source scope ↔
        (⟨k.iri X, P, o⟩ : Triple) ∈ k.transition tr owner prop X order source scope) := fun _ _ _ => Iff.rfl
  have hnoX : ∀ o, (⟨k.iri X, vt "initial", o⟩ : Triple) ∉ ts := by
    intro o ho; rw [hov _ _ (by decide) (by decide)] at ho
    simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
      self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
      self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne] at ho
  -- children
  have hev := rg_toks env cx hc hnoX (pre := X ++ ":ev") (p := "event")
    (fun j => ch_lit X ":ev" j (by simp) (by simp)) (by simp [containers]) (vs := tr.events)
    (fun d hd => by simp [transKids, hd])
  have hx := rg_exts env cx hc hnoX (xs := tr.extensions) (fun d hd => by simp [transKids, hd])
  have hb := reg_block env cx hc hnoX ":a" "action" (by simp [containers]) 0 tr.body
    (fun d hd => by simp [transKids, hd])
  have hr : ∀ d ∈ refKids k (k.iri X) "target" X ":tg" tr.targets scope, RG k ts d.1 := by
    intro d hd
    obtain ⟨hcap, hcd, hown⟩ := kid_ctx env cx hc (kidsL_ref k X "target" X ":tg" tr.targets scope)
      (fun d hd => by simp [transKids, hd]) (by simp [containers]) hd
    simp only [refKids, List.mem_map] at hd
    obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
    exact rg_ref env hcap hown hcd (fun s hs => absurd (((hcd.conts s).1
      ⟨"initial", by simp [containers], hs⟩) ▸ hs) (hnoX _))
  rw [RG, region_iff env cx]
  have hsplit : (∀ d ∈ transKids k X tr scope, ∀ g, U d.1 g → ShapesL.Good (Graph.ofList ts) (k.iri g)) ↔
      BlockOk tr.body := by
    rw [← hb]; simp only [transKids, List.mem_append, RG] at hev hx hr ⊢
    constructor
    · intro h d hd; exact h d (.inl (.inr hd))
    · intro h d hd
      rcases hd with ((hd | hd) | hd) | hd
      · exact hev d hd
      · exact hr d hd
      · exact h d hd
      · exact hx d hd
  rw [hsplit]
  -- the node itself
  have hevV := objV cx (p := "event") (c := "Token") (D' := tokKids k X (X ++ ":ev") "event" tr.events)
    (fun d hd => by simp [transKids, hd]) (ty_tokKids k X _ _ _) (by decide) (by decide)
    (fun o ho => by
      simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
        self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
        self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne] at ho; simpa using ho)
  have hacV := objV cx (p := "action") (c := "ExecutableContent") (D' := blockKids k X ":a" "action" 0 tr.body)
    (fun d hd => by simp [transKids, hd]) (ty_blockKids k X _ _ _ _) (by decide) (by decide)
    (fun o ho => by
      simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
        self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
        self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne] at ho; simpa using ho)
  have hxV := objV cx (p := "extension") (c := "ExtensionNode") (D' := extKids k X tr.extensions)
    (fun d hd => by simp [transKids, hd]) (ty_extKids k X _) (by decide) (by decide)
    (fun o ho => by
      simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
        self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
        self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne] at ho; simpa using ho)
  have htgV : ∀ o, (⟨k.iri X, vt "target", o⟩ : Triple) ∈ ts ↔
      o ∈ (refKids k (k.iri X) "target" X ":tg" tr.targets scope).map (fun d => k.iri d.1) := by
    intro o; rw [hov _ _ (by decide) (by decide)]
    simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
      self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
      self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne]
  have hT : (∀ v, (⟨k.iri X, vt "target", v⟩ : Triple) ∈ ts → Conforms (Graph.ofList ts) targetRefS v) ↔
      ∀ x ∈ tr.targets, Kn x ∧ (⟨k.iri (stateFragment x), vt "ancestor", scope⟩ : Triple) ∈ ts := by
    have key : ∀ p ∈ tr.targets.zipIdx, Conforms (Graph.ofList ts) targetRefS
        (k.iri (refK k (k.iri X) "target" X ":tg" scope p).1) ↔
        Kn p.1 ∧ (⟨k.iri (stateFragment p.1), vt "ancestor", scope⟩ : Triple) ∈ ts := by
      intro p hp
      have hd : refK k (k.iri X) "target" X ":tg" scope p ∈ transKids k X tr scope := by
        simp only [transKids, List.mem_append, refKids]; exact .inl (.inl (.inr (List.mem_map.2 ⟨p, hp, rfl⟩)))
      rcases p with ⟨x, j⟩
      exact targetRef_ok env (cx.kid hd) (above_of_ch (cx.dec.ch _ hd)) Kn (hSN x)
    simp only [htgV, refKids, List.map_map, List.mem_map, Function.comp_def, forall_exists_index, and_imp,
      forall_apply_eq_imp_iff₂]
    constructor
    · intro h x hx
      obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hx
      have hm : (tr.targets[i], i) ∈ tr.targets.zipIdx := List.mem_zipIdx_iff_getElem?.2 (by simp)
      exact (key _ hm).1 (h _ hm)
    · intro h p hp
      exact (key p hp).2 (h p.1 (fst_mem_zipIdx hp))
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "Transition" := by
    intro l; rw [ty_view cx.cap]
    simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
      self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
      self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne, eq_comm]
  have hcl : ∀ (l c : String), (∀ o, (⟨k.iri X, vt l, o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt c⟩ : Triple) ∈ ts) →
      CompHolds (Graph.ofList ts) none (clsP l c) (k.iri X) := by
    intro l c h
    simp only [clsP, pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
      compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList',
      instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
    exact h
  have htg : CompHolds (Graph.ofList ts) none (pr "target" [.node targetRefS]) (k.iri X) ↔
      ∀ x ∈ tr.targets, Kn x ∧ (⟨k.iri (stateFragment x), vt "ancestor", scope⟩ : Triple) ∈ ts := by
    rw [← hT]
    simp [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      holds_node, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList']
  have hsrcC : CompHolds (Graph.ofList ts) none (.property (propS (.alt [I "transition",
      .seq [I "initialTransition", I "initialElement"]]) [.equals (vi "source")])) (k.iri X) := by
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil,
      and_true, holds_equals, vn_some, valueNode_alt, List.mem_cons, List.not_mem_nil, or_false,
      exists_eq_or_imp, exists_eq_left, ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred,
      valueNode_seq_cons, valueNode_seq_nil, iri_vi, mem_ofList']
    intro v
    rw [hov _ _ (by decide) (by decide)]
    have h1 : (∃ z, (⟨z, vt "initialTransition", k.iri X⟩ : Triple) ∈ ts ∧
        ∃ z_1, (⟨z_1, vt "initialElement", z⟩ : Triple) ∈ ts ∧ v = z_1) ↔
        ∃ z, (⟨z, vt "initialTransition", k.iri X⟩ : Triple) ∈ ts ∧ (⟨v, vt "initialElement", z⟩ : Triple) ∈ ts :=
      ⟨fun ⟨z, h1, z1, h2, he⟩ => ⟨z, h1, he ▸ h2⟩, fun ⟨z, h1, h2⟩ => ⟨z, h1, v, h2, rfl⟩⟩
    rw [h1, hsrc v]
    simp [hne, Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
      self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
      self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne, eq_comm]
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  simp only [transitionS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true,
    hcl _ _ hevV, hcl _ _ hacV, hcl _ _ hxV, true_and, htg, hsrcC]
  rcases hty' : tr.type <;> rcases hcd : tr.cond <;>
  simp [orderedS, hov, Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
    self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
    self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne, hty', hcd, instanceOf_iff (Graph.ofList ts) env.noSub,
    hsrcT]

end Vertep.Rdf.Equiv
