import VertepLean.Rdf.Equiv.TransNodes
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on `<initial>` elements

The node shapes hold in the region of an `<initial>` element of state node `s` exactly when its
transition has no event, no condition and at least one target, its targets are states of the
chart below `s`, and its executable content is well-formed.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

set_option maxHeartbeats 1000000 in
theorem reg_init (env : Env k ts P) {s : Term} {F : String} {ie : Initial}
    (cx : Ctxt k ts s "initialElement" F (initEmission k s F ie) (initKids k s F ie))
    (hc : Contained' k ts (k.iri F) s) (hni : ∀ x, (⟨x, vt "initial", k.iri F⟩ : Triple) ∉ ts)
    (hsT : (⟨s, rdfType, vt "StateNode"⟩ : Triple) ∈ ts)
    (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x) :
    RG k ts F ↔ (ie.transition.events = [] ∧ ie.transition.cond = none ∧ ie.transition.targets ≠ []) ∧
      (∀ x ∈ ie.transition.targets, Kn x ∧ (⟨k.iri (stateFragment x), vt "ancestor", s⟩ : Triple) ∈ ts) ∧
      BlockOk ie.transition.body := by
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hselfF : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨k.iri F, vt l, o⟩ : Triple) ∈ ts ↔ (l = "initialTransition" ∧ o = k.iri (F ++ ":t")) ∨
        (l = "extension" ∧ o ∈ (extKids k F ie.extensions).map (fun d => k.iri d.1))) := by
    intro l o h1 h2
    rw [hov l o h1 h2]
    simp [initEmission, Ctx.node, self_trans k F (F ++ ":t") "initialTransition"
      (ch_lit' F ":t" (by simp) (by simp [NC])), self_ext, Ne.symm hne, h1, h2]
  have hnoF : ∀ o, (⟨k.iri F, vt "initial", o⟩ : Triple) ∉ ts := by
    intro o ho; rw [hselfF _ _ (by decide) (by decide)] at ho; simp at ho
  have hx := rg_exts env cx hc hnoF (xs := ie.extensions) (fun d hd => by simp [initKids, hd])
  -- the transition child
  have hdT : (F ++ ":t", k.transition ie.transition (k.iri F) "initialTransition" (F ++ ":t") 0 s s) ∈
      initKids k s F ie := by simp [initKids]
  have hcapT := cx.kid hdT
  have hchT : Ch F (F ++ ":t") := cx.dec.ch _ hdT
  have hlinkT : (⟨k.iri F, vt "initialTransition", k.iri (F ++ ":t")⟩ : Triple) ∈ ts :=
    (hselfF _ _ (by decide) (by decide)).2 (.inl ⟨rfl, rfl⟩)
  have hcT : Contained' k ts (k.iri (F ++ ":t")) (k.iri F) :=
    Contained'.kid env hc hchT (by simp [containers]) hlinkT
  have cxT : Ctxt k ts (k.iri F) "initialTransition" (F ++ ":t")
      (k.transition ie.transition (k.iri F) "initialTransition" (F ++ ":t") 0 s s)
      (transKids k (F ++ ":t") ie.transition s) :=
    ⟨hcapT, decomp_transition k _ _ _ _ _ _ _, above_of_ch hchT⟩
  have hsrc : ∀ v, ((⟨v, vt "transition", k.iri (F ++ ":t")⟩ : Triple) ∈ ts ∨ ∃ z,
      (⟨z, vt "initialTransition", k.iri (F ++ ":t")⟩ : Triple) ∈ ts ∧
        (⟨v, vt "initialElement", z⟩ : Triple) ∈ ts) ↔ v = s := by
    intro v
    have hup : ∀ z l, l ∈ containers → (⟨z, vt l, k.iri (F ++ ":t")⟩ : Triple) ∈ ts → z = k.iri F :=
      fun z l hl h => (hcT.conts z).1 ⟨l, hl, h⟩
    have hupF : ∀ z l, l ∈ containers → (⟨z, vt l, k.iri F⟩ : Triple) ∈ ts → z = s :=
      fun z l hl h => (hc.conts z).1 ⟨l, hl, h⟩
    have hlinkF : (⟨s, vt "initialElement", k.iri F⟩ : Triple) ∈ ts :=
      cx.cap.sub.subset (by simp [initEmission, Ctx.node])
    constructor
    · rintro (h | ⟨z, h1, h2⟩)
      · have := hup v "transition" (by simp [containers]) h; subst this
        rw [hselfF _ _ (by decide) (by decide)] at h; simp at h
      · have := hup z "initialTransition" (by simp [containers]) h1; subst this
        exact hupF v "initialElement" (by simp [containers]) h2
    · rintro rfl; exact .inr ⟨_, hlinkT, hlinkF⟩
  have hT := reg_transition env cxT hcT (noInit_of hcT hnoF) hsT hsrc Kn hSN
  rw [RG, region_iff env cx]
  have hsplit : (∀ d ∈ initKids k s F ie, ∀ g, U d.1 g → ShapesL.Good (Graph.ofList ts) (k.iri g)) ↔
      RG k ts (F ++ ":t") := by
    simp only [initKids, List.mem_cons, RG] at hx ⊢
    constructor
    · intro h; exact h _ (.inl rfl)
    · intro h d hd; rcases hd with rfl | hd
      · exact h
      · exact hx d hd
  rw [hsplit, hT]
  -- the node itself
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri F) l ↔ l = "Initial" := by
    intro l; rw [ty_view cx.cap]
    simp [initEmission, Ctx.node, self_trans k F (F ++ ":t") "initialTransition"
      (ch_lit' F ":t" (by simp) (by simp [NC])), self_ext, Ne.symm hne, eq_comm]
  have hneT := above_ne (k := k) (above_of_ch hchT)
  have hovT := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcapT l o hl hc'
  have hselfT : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨k.iri (F ++ ":t"), vt l, o⟩ : Triple) ∈ ts ↔
        (⟨k.iri (F ++ ":t"), vt l, o⟩ : Triple) ∈ k.transition ie.transition (k.iri F) "initialTransition"
          (F ++ ":t") 0 s s) := hovT
  have htvT : ∀ (l : String) (o : Term), l ∈ ["event", "cond", "target"] →
      ((⟨k.iri (F ++ ":t"), vt l, o⟩ : Triple) ∈ ts ↔
        (l = "event" ∧ o ∈ (tokKids k (F ++ ":t") (F ++ ":t" ++ ":ev") "event" ie.transition.events).map
          (fun d => k.iri d.1)) ∨
        (l = "cond" ∧ ∃ e, ie.transition.cond = some e ∧ o = .lit (.typed e k.exprDt)) ∨
        (l = "target" ∧ o ∈ (refKids k (k.iri (F ++ ":t")) "target" (F ++ ":t") ":tg"
          ie.transition.targets s).map (fun d => k.iri d.1))) := by
    intro l o hl
    simp only [List.mem_cons, List.not_mem_nil, or_false] at hl
    rw [hovT l o (by rcases hl with rfl | rfl | rfl <;> decide) (by rcases hl with rfl | rfl | rfl <;> decide)]
    rcases hl with rfl | rfl | rfl <;>
    simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit (F ++ ":t") ":ev" j (by simp) (by simp)),
      self_ref k (F ++ ":t") "target" (F ++ ":t") ":tg" (fun j => ch_lit (F ++ ":t") ":tg" j (by simp) (by simp)),
      self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hneT]
  have hxV := objV cx (p := "extension") (c := "ExtensionNode") (D' := extKids k F ie.extensions)
    (fun d hd => by simp [initKids, hd]) (ty_extKids k F _) (by decide) (by decide)
    (fun o ho => by
      have := (hselfF _ o (by decide) (by decide)).1 ((hov _ _ (by decide) (by decide)).2 ho)
      simpa using this)
  have hcl : ∀ (l c : String), (∀ o, (⟨k.iri F, vt l, o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt c⟩ : Triple) ∈ ts) →
      CompHolds (Graph.ofList ts) none (clsP l c) (k.iri F) := by
    intro l c h
    simp only [clsP, pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
      compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList',
      instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
    exact h
  have htyT : (⟨k.iri (F ++ ":t"), rdfType, vt "Transition"⟩ : Triple) ∈ ts :=
    hcapT.sub.subset (by simp [Ctx.transition, Ctx.node])
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  simp only [initialS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, hcl _ _ hxV]
  simp only [pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
    compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, ShapesL.I, valueNode_pred, iri_vi,
    mem_ofList', instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq, holds_node, minC, maxC,
    holds_minCount1, holds_maxCount1, hselfF, ne_eq, String.reduceEq, not_false_eq_true, true_and,
    false_and, or_false, forall_eq, exists_eq, htyT, eventlessTargetedS, nodeS, holds_maxCount0,
    valueNode_seq_cons, valueNode_seq_nil, valueNode_inv, holds_equals]
  have hA : ((∀ (y z : Term), y = k.iri (F ++ ":t") → z = k.iri (F ++ ":t") → y = z) ∧
      (∀ (y : Term), ¬(⟨k.iri (F ++ ":t"), vt "event", y⟩ : Triple) ∈ ts) ∧
      (∀ (y : Term), ¬(⟨k.iri (F ++ ":t"), vt "cond", y⟩ : Triple) ∈ ts) ∧
      ∃ y, (⟨k.iri (F ++ ":t"), vt "target", y⟩ : Triple) ∈ ts) ↔
      (ie.transition.events = [] ∧ ie.transition.cond = none ∧ ¬ie.transition.targets = []) := by
    have hev := fun o => htvT "event" o (by simp)
    have hcd := fun o => htvT "cond" o (by simp)
    have htg := fun o => htvT "target" o (by simp)
    simp only [hev, hcd, htg, String.reduceEq, false_and, or_false, false_or, true_and]
    refine ⟨fun ⟨_, h1, h2, h3⟩ => ⟨?_, ?_, ?_⟩, fun ⟨h1, h2, h3⟩ => ⟨fun y z hy hz => hy.trans hz.symm, ?_, ?_, ?_⟩⟩
    · cases he : ie.transition.events with
      | nil => rfl
      | cons e es => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
          (show (tokK k (F ++ ":t") (F ++ ":t" ++ ":ev") "event" (e, 0)) ∈
            tokKids k (F ++ ":t") (F ++ ":t" ++ ":ev") "event" ie.transition.events by
              simp [tokKids, he])) (h1 _)
    · cases hc' : ie.transition.cond with
      | none => rfl
      | some e => exact absurd ⟨e, hc', rfl⟩ (h2 (.lit (.typed e k.exprDt)))
    · intro he; obtain ⟨y, hy⟩ := h3; simp [refKids, he] at hy
    · intro y hy; simp [tokKids, h1] at hy
    · intro y ⟨e, he, _⟩; simp [h2] at he
    · cases ht : ie.transition.targets with
      | nil => exact absurd ht h3
      | cons x xs => simp [refKids, ht]
  have hB : ∀ (v : Term),
      (∃ z, z = k.iri (F ++ ":t") ∧ ∃ z_1, (⟨z, vt "target", z_1⟩ : Triple) ∈ ts ∧ v = z_1) →
        ∀ (v_1 : Term),
          (∃ z, (⟨z, vt "target", v⟩ : Triple) ∈ ts ∧ ∃ z_1, (⟨z_1, vt "initialTransition", z⟩ : Triple) ∈ ts ∧
            ∃ z, (⟨z, vt "initialElement", z_1⟩ : Triple) ∈ ts ∧ v_1 = z) ↔
          (⟨v, vt "scope", v_1⟩ : Triple) ∈ ts := by
    rintro v ⟨_, rfl, r, hr, rfl⟩ v1
    have hr' := (htvT "target" v (by simp)).1 hr
    simp only [String.reduceEq, false_and, false_or, true_and, refKids, List.map_map, List.mem_map,
      Function.comp_def] at hr'
    obtain ⟨⟨x, j⟩, hxj, rfl⟩ := hr'
    have hd : refK k (k.iri (F ++ ":t")) "target" (F ++ ":t") ":tg" s (x, j) ∈ transKids k (F ++ ":t") ie.transition s := by
      simp only [transKids, List.mem_append, refKids]; exact .inl (.inl (.inr (List.mem_map.2 ⟨_, hxj, rfl⟩)))
    obtain ⟨hcapR, hcR, hownR⟩ := kid_ctx env cxT hcT (kidsL_ref k (F ++ ":t") "target" (F ++ ":t") ":tg"
      ie.transition.targets s) (fun d hd => by simp [transKids, hd]) (by simp [containers])
      (show _ ∈ refKids k (k.iri (F ++ ":t")) "target" (F ++ ":t") ":tg" ie.transition.targets s from
        List.mem_map.2 ⟨_, hxj, rfl⟩)
    have hneR := above_ne hownR
    have hscopeR : ∀ o, (⟨k.iri (refK k (k.iri (F ++ ":t")) "target" (F ++ ":t") ":tg" s (x, j)).1,
        vt "scope", o⟩ : Triple) ∈ ts ↔ o = s := by
      intro o
      rw [obj_view' hcapR _ _ (by decide) (by decide)]
      refine ⟨fun h => ?_, fun h => by rw [h]; simp [refK, Ctx.node]⟩
      have hneR' : ¬ (F ++ ":t" ++ ":tg" ++ toString j = F ++ ":t") := fun e =>
        hneR (by simp only [refK]; rw [e])
      rcases refK_mem (k := k) h with h | h | h | h | h | h <;> simp [Triple.mk.injEq] at h
      all_goals first | exact h | exact h.2 | exact absurd h.1 hneR' | exact absurd h.1.symm hneR' | (exfalso; apply hneR'; simpa [refK] using h.1)
    rw [hscopeR]
    have hupR : ∀ z, (⟨z, vt "target", k.iri (refK k (k.iri (F ++ ":t")) "target" (F ++ ":t") ":tg" s (x, j)).1⟩ : Triple) ∈ ts ↔
        z = k.iri (F ++ ":t") := fun z =>
      ⟨fun h => (hcR.conts z).1 ⟨"target", by simp [containers], h⟩, fun h => h ▸ hr⟩
    have hupT : ∀ z, (⟨z, vt "initialTransition", k.iri (F ++ ":t")⟩ : Triple) ∈ ts ↔ z = k.iri F := fun z =>
      ⟨fun h => (hcT.conts z).1 ⟨"initialTransition", by simp [containers], h⟩, fun h => h ▸ hlinkT⟩
    have hupF : ∀ z, (⟨z, vt "initialElement", k.iri F⟩ : Triple) ∈ ts ↔ z = s := fun z =>
      ⟨fun h => (hc.conts z).1 ⟨"initialElement", by simp [containers], h⟩,
        fun h => h ▸ cx.cap.sub.subset (by simp [initEmission, Ctx.node])⟩
    simp only [hupR, hupT, hupF, exists_eq_left]
    try exact ⟨fun h => h.symm, fun h => h.symm⟩
  constructor
  · rintro ⟨⟨h1, _⟩, h2⟩; exact ⟨hA.1 h1, h2⟩
  · rintro ⟨h1, h2⟩; exact ⟨⟨hA.2 h1, hB⟩, h2⟩

end Vertep.Rdf.Equiv
