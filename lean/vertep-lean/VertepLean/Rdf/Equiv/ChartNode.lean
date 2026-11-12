import VertepLean.Rdf.Equiv.States
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes in the chart node's region

The chart node, its `initial` references, its data items, its `<script>` and its extensions.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

section
variable (c : Chart) (base : String)

theorem chart_cx : Ctxt (ctxOf c base) (project c base) noOwner "chart" "chart"
    (chartCore (ctxOf c base) c) (chartKids (ctxOf c base) c) :=
  ⟨chart_coreCap c base, decomp_chart _ c, above_noOwner _ "chart"⟩

theorem chart_view (l : String) (o : Term) (h1 : l ≠ "historyDefault") (h2 : l ≠ "choice") :
    (⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ project c base ↔
      (⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ chartCore (ctxOf c base) c :=
  (chart_coreCap c base).lookup (U_refl _) (by simp [h1, h2])

/-- A child of the chart node has the chart node as its single container. -/
theorem chart_kid_cont {f l : String} (hch : Ch "chart" f) (hl : l ∈ containers)
    (hlink : (⟨(ctxOf c base).iri "chart", vt l, (ctxOf c base).iri f⟩ : Triple) ∈ project c base) :
    Contained' (ctxOf c base) (project c base) ((ctxOf c base).iri f) ((ctxOf c base).iri "chart") where
  conts s := by
    constructor
    · rintro ⟨l', hl', ht⟩
      obtain ⟨f', hf', h⟩ := link_project c base _ ht ⟨l', hl', rfl⟩
      simp only [iri_inj] at hf'; subst hf'
      rcases h with ⟨g, hg, hs⟩ | ⟨q, _, hq, _⟩
      · exact hs.trans (by rw [ch_uniq hg hch])
      · exact absurd (hq ▸ hch) not_ch_sf
    · rintro rfl; exact ⟨l, hl, hlink⟩
  up := .inl rfl

theorem chart_kid_ctx {p : String} {D' : List (String × List Triple)}
    (hL : KidsL (ctxOf c base) "chart" p D') (hD' : ∀ d ∈ D', d ∈ chartKids (ctxOf c base) c)
    (hp : p ∈ containers) {d : String × List Triple} (hd : d ∈ D') :
    Cap (ctxOf c base) (project c base) (U d.1) d.2 ∧
      Contained' (ctxOf c base) (project c base) ((ctxOf c base).iri d.1) ((ctxOf c base).iri "chart") ∧
      Above (k := ctxOf c base) ((ctxOf c base).iri "chart") d.1 := by
  have cx := chart_cx c base
  have hcap := cx.kid (hD' d hd)
  have hch := cx.dec.ch d (hD' d hd)
  exact ⟨hcap, chart_kid_cont c base hch hp (hcap.sub.subset (hL d hd).2), above_of_ch hch⟩

end

section
variable (c : Chart) (base : String)

/-- Nothing links to the chart node. -/
theorem chart_uncontained : ∀ s l, l ∈ containers →
    (⟨s, vt l, (ctxOf c base).iri "chart"⟩ : Triple) ∉ project c base := by
  intro s l hl ht
  obtain ⟨f, hf, h⟩ := link_project c base _ ht ⟨l, hl, rfl⟩
  simp only [iri_inj] at hf; subst hf
  rcases h with ⟨g, ⟨a, ha, _⟩, _⟩ | ⟨q, _, hq, _⟩
  · have : ':' ∈ "chart".toList := by rw [ha]; simp
    simp at this
  · have := congrArg String.toList hq
    simp [stateFragment, String.toList_append] at this

set_option maxHeartbeats 4000000 in
theorem reg_chart (Kn : String → Prop)
    (hSN : ∀ x, (⟨(ctxOf c base).iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ project c base ↔ Kn x)
    (htopT : ∀ s ∈ c.states, ∀ l, (⟨(ctxOf c base).iri (stateFragment s.id), rdfType, vt l⟩ : Triple) ∈
      project c base ↔ l = "StateNode" ∨ l = elementClass s.element) :
    RG (ctxOf c base) (project c base) "chart" ↔
      (c.states ≠ [] ∧ (∀ x ∈ c.initial, TR (ctxOf c base) (project c base) Kn x ((ctxOf c base).iri "chart")) ∧
        (∀ s ∈ c.states, s.isHistory = false)) ∧ ∀ d ∈ c.data, DataOk d := by
  have env := env_project c base
  have cx := chart_cx c base
  have hv := chart_view c base
  have hnd := cx.dec.nd
  -- kids other than the initial references are not objects of vt:initial
  have hnoinit : ∀ d ∈ chartKids (ctxOf c base) c, d ∉ chartRefs (ctxOf c base) c →
      Contained' (ctxOf c base) (project c base) ((ctxOf c base).iri d.1) ((ctxOf c base).iri "chart") → ∀ s, (⟨s, vt "initial", (ctxOf c base).iri d.1⟩ : Triple) ∉ project c base := by
    intro d hd hnr hc s hs
    have := (hc.conts s).1 ⟨"initial", by simp [containers], hs⟩
    subst this
    rw [hv _ _ (by decide) (by decide)] at hs
    simp only [chartCore, chartRefs, List.mem_append] at hs
    simp only [self_chartRefs, self_dataS, self_scriptS, self_extS, rel_mk, ne_eq, String.reduceEq,
      not_false_eq_true, and_self, and_true, true_and, vt_inj, false_and, or_false] at hs
    simp only [chartSelf, childLinks, List.mem_append, List.mem_cons, List.mem_map, mem_optStr, mem_str,
      mem_bool, Triple.mk.injEq, vt_inj, String.reduceEq, false_and, and_false, false_or, or_false,
      exists_false, vt_ne_rdfType, List.not_mem_nil] at hs
    obtain ⟨a, ha, hae⟩ := hs
    have : a = d := nodup_fst_eq hnd (List.mem_append_left _ (List.mem_append_left _
      (List.mem_append_left _ ha))) hd ((iri_inj (ctxOf c base)).1 hae)
    exact hnr (this ▸ ha)
  have hE : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ project c base ↔
        (⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ chartCore (ctxOf c base) c) := hv
  have chT : ∀ (p cl : String) (D' : List (String × List Triple)), (∀ d ∈ D', d ∈ chartKids (ctxOf c base) c) →
      (∀ d ∈ D', (⟨(ctxOf c base).iri d.1, rdfType, vt cl⟩ : Triple) ∈ d.2) → p ≠ "historyDefault" → p ≠ "choice" →
      (∀ o, (⟨(ctxOf c base).iri "chart", vt p, o⟩ : Triple) ∈ chartCore (ctxOf c base) c →
        o ∈ D'.map (fun d => (ctxOf c base).iri d.1)) →
      CompHolds (Graph.ofList (project c base)) none (clsP p cl) ((ctxOf c base).iri "chart") := by
    intro p cl D' hD' hT h1 h2 hm
    apply hcl_of env
    intro o ho
    rw [hv _ _ h1 h2] at ho
    obtain ⟨d, hd, rfl⟩ := List.mem_map.1 (hm o ho)
    exact kid_sub cx (hD' d hd) (hT d hd)
  have hself : ∀ (P : Term) (o : Term), Rel ⟨(ctxOf c base).iri "chart", P, o⟩ →
      ((⟨(ctxOf c base).iri "chart", P, o⟩ : Triple) ∈ chartCore (ctxOf c base) c ↔
        (⟨(ctxOf c base).iri "chart", P, o⟩ : Triple) ∈ chartSelf (ctxOf c base) c ∨
        (⟨(ctxOf c base).iri "chart", P, o⟩ : Triple) ∈ childLinks (ctxOf c base) ((ctxOf c base).iri "chart") c.states ∨
        (P = vt "initial" ∧ o ∈ (chartRefs (ctxOf c base) c).map (fun d => (ctxOf c base).iri d.1)) ∨
        (P = vt "data" ∧ o ∈ (c.data.zipIdx.map (dataK (ctxOf c base) "chart")).map (fun d => (ctxOf c base).iri d.1)) ∨
        (P = vt "script" ∧ o ∈ (scriptKids (ctxOf c base) c.script).map (fun d => (ctxOf c base).iri d.1)) ∨
        (P = vt "extension" ∧ o ∈ (extKids (ctxOf c base) "chart" c.extensions).map (fun d => (ctxOf c base).iri d.1))) := by
    intro P o hr
    simp only [chartCore, chartRefs, List.mem_append]
    rw [self_chartRefs _ _ _ hr, self_dataS _ _ _ hr, self_scriptS _ _ hr, self_extS _ _ _ hr]
    constructor
    · rintro (((((h | h) | h) | h) | h) | h)
      · exact .inl h
      · exact .inr (.inr (.inl h))
      · exact .inr (.inr (.inr (.inl h)))
      · exact .inr (.inr (.inr (.inr (.inl h))))
      · exact .inr (.inl h)
      · exact .inr (.inr (.inr (.inr (.inr h))))
    · rintro (h | h | h | h | h | h)
      · exact .inl (.inl (.inl (.inl (.inl h))))
      · exact .inl (.inr h)
      · exact .inl (.inl (.inl (.inl (.inr h))))
      · exact .inl (.inl (.inl (.inr h)))
      · exact .inl (.inl (.inr h))
      · exact .inr h
  -- the children of the chart node
  have cnr : ∀ d, (d ∈ c.data.zipIdx.map (dataK (ctxOf c base) "chart") ∨ d ∈ scriptKids (ctxOf c base) c.script ∨
      d ∈ extKids (ctxOf c base) "chart" c.extensions) → d ∉ chartRefs (ctxOf c base) c := by
    intro d hd hr
    have hnd' := hnd
    simp only [chartKids, List.append_assoc, List.map_append, List.nodup_append] at hnd'
    apply hnd'.2.2 d.1 (List.mem_map_of_mem hr) d.1 _ rfl
    simp only [List.mem_append]
    rcases hd with hd | hd | hd
    · exact .inl (List.mem_map_of_mem hd)
    · exact .inr (.inl (List.mem_map_of_mem hd))
    · exact .inr (.inr (List.mem_map_of_mem hd))
  have kR : ∀ d ∈ chartRefs (ctxOf c base) c, RG (ctxOf c base) (project c base) d.1 := by
    intro d hd
    obtain ⟨hcap, hc, hown⟩ := chart_kid_ctx c base (kidsL_ref _ "chart" "initial" "chart:ia" "" c.initial _)
      (fun d hd => by simp [chartKids, chartRefs, hd]) (by simp [containers]) hd
    simp only [chartRefs, refKids, List.mem_map] at hd
    obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
    exact rg_ref env hcap hown hc (fun s hs => ⟨(hc.conts s).1 ⟨"initial", by simp [containers], hs⟩, rfl⟩)
  have kD : (∀ d ∈ c.data.zipIdx.map (dataK (ctxOf c base) "chart"), RG (ctxOf c base) (project c base) d.1) ↔
      ∀ d ∈ c.data, DataOk d := by
    rw [List.forall_mem_map, ← forall_zipIdx (Q := fun d => DataOk d)]
    apply forall₂_congr; intro p hp
    have hd := List.mem_map_of_mem (f := dataK (ctxOf c base) "chart") hp
    have hdk : dataK (ctxOf c base) "chart" p ∈ chartKids (ctxOf c base) c := by simp [chartKids, hd]
    obtain ⟨hcap, hc, hown⟩ := chart_kid_ctx c base (kidsL_dataK _ "chart" c.data)
      (fun d hd => by simp [chartKids, hd]) (by simp [containers]) hd
    have hni := hnoinit _ hdk (cnr _ (.inl hd)) hc
    rcases p with ⟨d, j⟩
    simp only [dataK] at hcap hc hown hni ⊢
    exact reg_data env ⟨hcap, decomp_dataItem _ _ _ _ _, hown⟩ hc hni
  have kX : ∀ d ∈ extKids (ctxOf c base) "chart" c.extensions, RG (ctxOf c base) (project c base) d.1 := by
    intro d hd
    have hdk : d ∈ chartKids (ctxOf c base) c := by simp [chartKids, hd]
    obtain ⟨hcap, hc, hown⟩ := chart_kid_ctx c base (kidsL_ext _ "chart" c.extensions)
      (fun d hd => by simp [chartKids, hd]) (by simp [containers]) hd
    have hni := hnoinit _ hdk (cnr _ (.inr (.inr hd))) hc
    simp only [extKids, List.mem_map] at hd
    obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
    exact good_ext env hcap hown hc hni
  have kS : ∀ d ∈ scriptKids (ctxOf c base) c.script, RG (ctxOf c base) (project c base) d.1 := by
    intro d hd
    have hdk : d ∈ chartKids (ctxOf c base) c := by simp [chartKids, hd]
    obtain ⟨hcap, hc, hown⟩ := chart_kid_ctx c base (kidsL_script _ c.script)
      (fun d hd => by simp [chartKids, hd]) (by simp [containers]) hd
    have hni := hnoinit _ hdk (cnr _ (.inr (.inl hd))) hc
    rcases hsc : c.script with _ | sc
    · simp [scriptKids, hsc] at hd
    simp only [scriptKids, hsc, List.mem_singleton] at hd
    subst hd
    have hne := above_ne hown
    simp only at hcap hc hni hne hown ⊢
    rw [RG, region_leaf env (prop := "script") hcap hown (fun t ht _ => by
      simp only [List.mem_append, mem_node, mem_scriptFields, List.mem_singleton] at ht
      rcases ht with (h | ⟨_, _, h⟩) | ⟨_, _, h⟩ | ⟨_, _, h⟩ <;> subst h <;> simp)]
    have hty : ∀ l, Ty (Graph.ofList (project c base)) ((ctxOf c base).iri "chart:scr") l ↔ l = "Script" := by
      intro l; rw [ty_view hcap]; simp [Ctx.node, Ne.symm hne, eq_comm, Ctx.scriptFields]
    have hov := fun l o hl hc' => obj_view' (k := ctxOf c base) (ts := project c base) hcap l o hl hc'
    leaf_tac env, hc, hty, hni
    rcases sc with ⟨_ | a, _ | b⟩ <;> simp [scriptS, hov, Ctx.node, Ctx.scriptFields, hne, Ne.symm hne]
  -- the chart node
  have hV : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ project c base ↔
        (⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ chartSelf (ctxOf c base) c ∨
        (⟨(ctxOf c base).iri "chart", vt l, o⟩ : Triple) ∈ childLinks (ctxOf c base) ((ctxOf c base).iri "chart") c.states ∨
        (l = "initial" ∧ o ∈ (chartRefs (ctxOf c base) c).map (fun d => (ctxOf c base).iri d.1)) ∨
        (l = "data" ∧ o ∈ (c.data.zipIdx.map (dataK (ctxOf c base) "chart")).map (fun d => (ctxOf c base).iri d.1)) ∨
        (l = "script" ∧ o ∈ (scriptKids (ctxOf c base) c.script).map (fun d => (ctxOf c base).iri d.1)) ∨
        (l = "extension" ∧ o ∈ (extKids (ctxOf c base) "chart" c.extensions).map (fun d => (ctxOf c base).iri d.1))) := by
    intro l o h1 h2
    rw [hv l o h1 h2, hself _ _ (by simp [h1, h2])]
    simp only [vt_inj]
  have hchild : ∀ o, (⟨(ctxOf c base).iri "chart", vt "child", o⟩ : Triple) ∈ project c base ↔
      ∃ s ∈ c.states, o = (ctxOf c base).iri (stateFragment s.id) := by
    intro o; rw [hV _ _ (by decide) (by decide)]
    simp [chartSelf, childLinks, eq_comm]
  have hty : ∀ l, Ty (Graph.ofList (project c base)) ((ctxOf c base).iri "chart") l ↔ l = "Chart" := by
    intro l
    simp only [Ty, rdf_type_eq, mem_ofList']
    rw [(chart_coreCap c base).lookup (U_refl _) (by simp), hself _ _ (by simp)]
    simp [chartSelf, childLinks, eq_comm]
  have hni : ∀ s, (⟨s, vt "initial", (ctxOf c base).iri "chart"⟩ : Triple) ∉ project c base :=
    fun s => chart_uncontained c base s "initial" (by simp [containers])
  have c1 : CompHolds (Graph.ofList (project c base)) none
      (.property (propS (.alt [ShapesL.I "child", ShapesL.I "action", ShapesL.I "extension"]) [maxC 0]))
      ((ctxOf c base).iri "chart") := by
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil, and_true,
      maxC, holds_maxCount0, vn_some, valueNode_alt, List.mem_cons, List.not_mem_nil, or_false,
      exists_eq_or_imp, exists_eq_left, ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred, iri_vi,
      mem_ofList', not_or]
    intro y
    exact ⟨chart_uncontained c base y "child" (by simp [containers]),
      chart_uncontained c base y "action" (by simp [containers]),
      chart_uncontained c base y "extension" (by simp [containers])⟩
  have c7 : CompHolds (Graph.ofList (project c base)) none (pr "child" [cls "StateNode", minC 1])
      ((ctxOf c base).iri "chart") ↔ c.states ≠ [] := by
    simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      and_true, cls, holds_class, minC, holds_minCount1, vn_some, ShapesL.P, valueNode_pred, iri_vi,
      mem_ofList', hchild, instanceOf_iff (Graph.ofList (project c base)) env.noSub, rdf_type_eq]
    constructor
    · rintro ⟨_, ⟨y, ⟨s, hs, rfl⟩⟩⟩ he; simp [he] at hs
    · intro hne
      refine ⟨fun v ⟨s, hs, hv'⟩ => hv' ▸ (htopT s hs _).2 (.inl rfl), ?_⟩
      cases hcs : c.states with
      | nil => exact absurd hcs hne
      | cons s ss => exact ⟨_, s, by simp, rfl⟩
  have c12 : CompHolds (Graph.ofList (project c base)) none
      (pr "child" [.qualifiedMax (nodeS [cls "History"]) 0 []]) ((ctxOf c base).iri "chart") ↔
      ∀ s ∈ c.states, s.isHistory = false := by
    simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      and_true, holds_qualifiedMax0, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hchild,
      nodeS, cls, holds_class, instanceOf_iff (Graph.ofList (project c base)) env.noSub, rdf_type_eq]
    constructor
    · intro hh s hs
      have := hh _ ⟨s, hs, rfl⟩
      rw [htopT s hs] at this
      rcases s with ⟨_, _, el⟩; cases el <;> simp_all [elementClass, State.isHistory, State.element]
    · rintro hh v ⟨s, hs, rfl⟩
      rw [htopT s hs]
      have := hh s hs
      rcases s with ⟨_, _, el⟩; cases el <;> simp_all [elementClass, State.isHistory, State.element]
  have c8 : CompHolds (Graph.ofList (project c base)) none (pr "initial" [.node targetRefS])
      ((ctxOf c base).iri "chart") ↔ ∀ x ∈ c.initial, TR (ctxOf c base) (project c base) Kn x ((ctxOf c base).iri "chart") := by
    have key : ∀ p ∈ c.initial.zipIdx, Conforms (Graph.ofList (project c base)) targetRefS
        ((ctxOf c base).iri (refK (ctxOf c base) ((ctxOf c base).iri "chart") "initial" "chart:ia" "" ((ctxOf c base).iri "chart") p).1) ↔
        TR (ctxOf c base) (project c base) Kn p.1 ((ctxOf c base).iri "chart") := by
      intro p hp
      have hd : refK (ctxOf c base) ((ctxOf c base).iri "chart") "initial" "chart:ia" "" ((ctxOf c base).iri "chart") p ∈
          chartKids (ctxOf c base) c := by
        simp only [chartKids, chartRefs, refKids, List.mem_append]
        exact .inl (.inl (.inl (List.mem_map_of_mem hp)))
      rcases p with ⟨x, j⟩
      exact targetRef_ok env (cx.kid hd) (above_of_ch (cx.dec.ch _ hd)) Kn (hSN x)
    have hiv : ∀ v, (⟨(ctxOf c base).iri "chart", vt "initial", v⟩ : Triple) ∈ project c base ↔
        v ∈ (chartRefs (ctxOf c base) c).map (fun d => (ctxOf c base).iri d.1) := by
      intro v; rw [hV _ _ (by decide) (by decide)]
      simp [chartSelf, childLinks]
    simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      and_true, holds_node, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hiv,
      List.forall_mem_map, chartRefs, refKids]
    rw [← forall_zipIdx (Q := fun x => TR (ctxOf c base) (project c base) Kn x ((ctxOf c base).iri "chart"))]
    exact forall₂_congr key
  have hgood : ShapesL.Good (Graph.ofList (project c base)) ((ctxOf c base).iri "chart") ↔
      c.states ≠ [] ∧ (∀ x ∈ c.initial, TR (ctxOf c base) (project c base) Kn x ((ctxOf c base).iri "chart")) ∧
        (∀ s ∈ c.states, s.isHistory = false) := by
    rw [good_iff _ env.noSub]
    simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
      String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
      true_and, and_true, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true, true_or]
    have cD := chT "data" "DataItem" _ (fun d hd => List.mem_append_left _ (List.mem_append_left _
      (List.mem_append_right _ hd))) (ty_dataK _ "chart" c.data)
      (by decide) (by decide) (fun o ho => by
        rw [hself _ _ (by simp)] at ho; simpa [chartSelf, childLinks] using ho)
    have cX := chT "extension" "ExtensionNode" _ (fun d hd => List.mem_append_right _ hd) (ty_extKids _ "chart" _)
      (by decide) (by decide) (fun o ho => by
        rw [hself _ _ (by simp)] at ho; simpa [chartSelf, childLinks] using ho)
    have cS : CompHolds (Graph.ofList (project c base)) none (pr "script" [cls "Script", maxC 1])
        ((ctxOf c base).iri "chart") := by
      have hsv : ∀ o, (⟨(ctxOf c base).iri "chart", vt "script", o⟩ : Triple) ∈ project c base ↔
          o ∈ (scriptKids (ctxOf c base) c.script).map (fun d => (ctxOf c base).iri d.1) := by
        intro o; rw [hV _ _ (by decide) (by decide)]; simp [chartSelf, childLinks]
      simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
        and_true, cls, holds_class, maxC, holds_maxCount1, vn_some, ShapesL.P, valueNode_pred, iri_vi,
        mem_ofList', hsv, instanceOf_iff (Graph.ofList (project c base)) env.noSub, rdf_type_eq]
      refine ⟨fun v hv' => ?_, fun y z hy hz => ?_⟩
      · obtain ⟨d, hd, rfl⟩ := List.mem_map.1 hv'
        have hdk : d ∈ chartKids (ctxOf c base) c := by simp [chartKids, hd]
        rcases hsc : c.script with _ | sc <;> simp [scriptKids, hsc] at hd
        subst hd; exact kid_sub cx hdk (by simp [Ctx.node])
      · rcases hsc : c.script with _ | sc <;> simp [scriptKids, hsc] at hy hz; rw [hy, hz]
    simp only [chartS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, c1, c7, c8, c12, cD,
      cX, cS, true_and]
    rcases hb : c.binding <;>
    simp [optStrP, strP, boolP, pr, propS, dt, minC, maxC, hV, chartSelf, childLinks, hb] <;>
    rcases hn : c.name <;> simp [hn]
  rw [RG, region_iff env cx, hgood]
  have hk : (∀ d ∈ chartKids (ctxOf c base) c, ∀ g, U d.1 g → ShapesL.Good (Graph.ofList (project c base))
      ((ctxOf c base).iri g)) ↔ ∀ d ∈ c.data, DataOk d := by
    rw [← kD]; simp only [RG] at kR kS kX ⊢
    simp only [chartKids, List.mem_append]
    constructor
    · intro hh d hd; exact hh d (.inl (.inl (.inr hd)))
    · intro hh d hd
      rcases hd with ((hd | hd) | hd) | hd
      · exact kR d hd
      · exact hh d hd
      · exact kS d hd
      · exact kX d hd
  rw [hk]

end

end Vertep.Rdf.Equiv
