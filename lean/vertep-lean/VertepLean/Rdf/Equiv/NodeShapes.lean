import VertepLean.Rdf.Equiv.ChartNode
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on the whole projection

`goodAll_iff`: for a chart with unique state identifiers, the node shapes hold at every node of
its projection exactly when the chart node's rules (`reg_chart`) and every state's rules
(`SOK`) hold, with `Kn` the identifiers of the chart's states.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

section
variable (c : Chart) (base : String)

theorem good_of_noTy {x : Term}
    (h : ∀ C, (⟨x, rdfType, C⟩ : Triple) ∉ project c base) :
    ShapesL.Good (toRDF c base) x := by
  have env := env_project c base
  intro s hs hf
  have hty : ∀ l, ¬ Ty (toRDF c base) x l := fun l hl => by
    simp only [Ty, rdf_type_eq, toRDF, mem_ofList'] at hl; exact h _ hl
  have hin : ¬ ∃ s, (⟨s, vt "initial", x⟩ : Triple) ∈ toRDF c base := fun ⟨s, hs⟩ =>
    h _ (env.initTy s _ (by simpa [toRDF] using hs))
  simp only [nodeShapes, List.mem_cons, List.not_mem_nil, or_false] at hs
  rcases hs with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl |
    rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;>
    simp [foc_containedS _ (noSub c base), foc_chartS _ (noSub c base), foc_stateNodeS _ (noSub c base),
      foc_stateS _ (noSub c base), foc_parallelS _ (noSub c base), foc_finalS _ (noSub c base),
      foc_historyS' _ (noSub c base), foc_initialRefScopeS, foc_initialS _ (noSub c base),
      foc_transitionS _ (noSub c base), foc_tokenS _ (noSub c base), foc_handlerS _ (noSub c base),
      foc_execS _ (noSub c base), foc_raiseS _ (noSub c base), foc_sendS _ (noSub c base),
      foc_cancelS _ (noSub c base), foc_logS _ (noSub c base), foc_assignS _ (noSub c base),
      foc_scriptS _ (noSub c base), foc_ifS _ (noSub c base), foc_ifBranchS _ (noSub c base),
      foc_foreachS _ (noSub c base), foc_extensionContentS _ (noSub c base), foc_paramS _ (noSub c base),
      foc_contentS _ (noSub c base), foc_dataItemS _ (noSub c base), foc_doneDataS _ (noSub c base),
      foc_invokeS _ (noSub c base), foc_extensionNodeS _ (noSub c base), hty] at hf <;>
    exact absurd hf hin

/-- Every typed node lies in the chart node's region or in a state's region. -/
theorem typed_region {x C : Term} (h : (⟨x, rdfType, C⟩ : Triple) ∈ project c base) :
    ∃ g, x = (ctxOf c base).iri g ∧ (U "chart" g ∨ ∃ Z ∈ c.allStates, U (stateFragment Z.id) g) := by
  have h' := h
  rw [project_eq] at h'
  simp only [List.mem_append] at h'
  have hr : Rel ⟨x, rdfType, C⟩ := by simp
  have hcore : ∀ t ∈ chartCore (ctxOf c base) c, t.p = rdfType → ∃ g, t.s = (ctxOf c base).iri g ∧ U "chart" g := by
    intro t ht hp
    rcases (decomp_chart (ctxOf c base) c).locL t ht (by rcases t with ⟨_, _, _⟩; simp at hp; subst hp; simp) with
      he | hg
    · rw [he] at hp; simp at hp
    · exact hg
  rcases h' with (((((ht | ht) | ht) | ht) | ht) | ht) | ht
  · obtain ⟨g, hg, hU⟩ := hcore _ (by simp only [chartCore, List.mem_append]; exact .inl (.inl (.inl (.inl (.inl ht))))) rfl
    exact ⟨g, hg, .inl hU⟩
  · obtain ⟨g, hg, hU⟩ := hcore _ (by simp only [chartCore, List.mem_append]; exact .inl (.inl (.inl (.inl (.inr ht))))) rfl
    exact ⟨g, hg, .inl hU⟩
  · obtain ⟨g, hg, hU⟩ := hcore _ (by simp only [chartCore, List.mem_append]; exact .inl (.inl (.inl (.inr ht)))) rfl
    exact ⟨g, hg, .inl hU⟩
  · obtain ⟨g, hg, hU⟩ := hcore _ (by simp only [chartCore, List.mem_append]; exact .inl (.inl (.inr ht))) rfl
    exact ⟨g, hg, .inl hU⟩
  · rcases states_cov _ _ _ 0 _ _ ht hr with he | ⟨g, hg, Z, hZ, hU⟩
    · simp [childLinks] at he
    · exact ⟨g, hg, .inr ⟨Z, hZ, hU⟩⟩
  · obtain ⟨g, hg, hU⟩ := hcore _ (by simp only [chartCore, List.mem_append]; exact .inr ht) rfl
    exact ⟨g, hg, .inl hU⟩
  · exact absurd (hd_ps _ _ _ _ ht) (by simp)
end

section
variable (c : Chart) (base : String)

theorem parented_ids_nodup (hnd : c.ids.Nodup) : (c.parented.map (fun q => q.2.id)).Nodup := by
  have : c.parented.map (fun q => q.2.id) = c.ids := by
    rw [show (fun q : Option State × State => q.2.id) = State.id ∘ Prod.snd from rfl, ← List.map_map,
      Chart.parented, parentedList_snd]; rfl
  rw [this]; exact hnd

theorem state_typed {Z : State} (hZ : Z ∈ c.allStates) :
    (⟨(ctxOf c base).iri (stateFragment Z.id), rdfType, vt "StateNode"⟩ : Triple) ∈ project c base := by
  have : Z ∈ c.parented.map Prod.snd := by rwa [Chart.parented, parentedList_snd]
  obtain ⟨q, hq, rfl⟩ := List.mem_map.1 this
  obtain ⟨i', A'', h⟩ := parentedList_sub (ctxOf c base) none 0 [] c.states q hq
  have := h ⟨(ctxOf c base).iri (stateFragment q.2.id), rdfType, vt "StateNode"⟩
    (by rw [state_eq']; simp [stateSelf, Ctx.node])
  rw [project_eq]; simp only [List.mem_append]; exact .inl (.inl (.inr (by simpa [parT] using this)))

theorem hSN_ids (x : String) :
    (⟨(ctxOf c base).iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ project c base ↔ x ∈ c.ids := by
  constructor
  · intro h
    obtain ⟨Y, hY, he⟩ := stateNode_typed c base h
    have := stateFragment_inj ((iri_inj _).1 he)
    exact this ▸ List.mem_map_of_mem hY
  · intro h
    obtain ⟨Z, hZ, rfl⟩ := List.mem_map.1 h
    exact state_typed c base hZ

theorem chart_not_SN : (⟨(ctxOf c base).iri "chart", rdfType, vt "StateNode"⟩ : Triple) ∉ project c base := by
  intro h
  obtain ⟨Y, _, he⟩ := stateNode_typed c base h
  have := congrArg String.toList ((iri_inj _).1 he)
  simp [stateFragment, String.toList_append] at this

theorem top_rec (hnd : c.ids.Nodup) :
    ∀ p ∈ c.states.zipIdx 0, RecH (ctxOf c base) (project c base) c.parented none p.1 p.2 [] := by
  have hsh := capList (k := ctxOf c base) ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"]
    c.states 0 (chart_statesCap c base) hnd (fun g hR he => by
      rw [iri_inj] at he; subst he; exact not_RSs_chart hR)
  intro p hp
  refine ⟨hsh p hp, fun q hq => ?_, ?_, .inl rfl, ?_, fun _ v hv => ?_⟩
  · rw [Chart.parented]; exact mem_parentedList_of (fst_mem_zipIdx hp) hq
  · intro v
    simp only [parT, List.mem_singleton]
    constructor
    · rintro (h | h)
      · exact h
      · rcases h.cases_head with h' | ⟨w, h', _⟩ <;>
        exact absurd (by simpa [ShapesL.I, toRDF] using h') (chart_uncontained c base _ "child" (by simp [containers]))
    · intro h; exact .inl h
  · simp only [parT, Option.isSome_none, Bool.false_eq_true, iff_false]; exact chart_not_SN c base
  · exact chart_uncontained c base v "child" (by simp [containers]) hv

end

set_option maxHeartbeats 1000000 in
/-- **The node shapes on the projection of a chart with unique state identifiers.** -/
theorem goodAll_iff (c : Chart) (base : String) (hnd : c.ids.Nodup) :
    (∀ x, ShapesL.Good (toRDF c base) x) ↔
      ((c.states ≠ [] ∧ (∀ x ∈ c.initial, TR (ctxOf c base) (project c base) (· ∈ c.ids) x ((ctxOf c base).iri "chart")) ∧
        (∀ s ∈ c.states, s.isHistory = false)) ∧ ∀ d ∈ c.data, DataOk d) ∧
      ∀ q ∈ c.parented, SOK (ctxOf c base) (project c base) (· ∈ c.ids) q.1 q.2 := by
  have env := env_project c base
  have hnd' := parented_ids_nodup c hnd
  have htop := top_rec c base hnd
  have htopT : ∀ s ∈ c.states, ∀ l, (⟨(ctxOf c base).iri (stateFragment s.id), rdfType, vt l⟩ : Triple) ∈
      project c base ↔ l = "StateNode" ∨ l = elementClass s.element := by
    intro s hs l
    obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hs
    have hm : (c.states[j], j) ∈ c.states.zipIdx 0 := List.mem_zipIdx_iff_getElem?.2 (by simp)
    exact ((htop _ hm).stctx env hnd').ty l
  have hs : (∀ Z ∈ c.allStates, RG (ctxOf c base) (project c base) (stateFragment Z.id)) ↔
      ∀ q ∈ c.parented, SOK (ctxOf c base) (project c base) (· ∈ c.ids) q.1 q.2 :=
    reg_states env hnd' (· ∈ c.ids) (hSN_ids c base) none [] c.states 0 htop
  refine Iff.trans ?_ (and_congr (reg_chart c base (· ∈ c.ids) (hSN_ids c base) htopT) hs)
  constructor
  · intro h
    exact ⟨fun g _ => h _, fun Z _ g _ => h _⟩
  · rintro ⟨hc, hz⟩ x
    by_cases hx : ∃ C, (⟨x, rdfType, C⟩ : Triple) ∈ project c base
    · obtain ⟨C, hC⟩ := hx
      obtain ⟨g, rfl, hg | ⟨Z, hZ, hU⟩⟩ := typed_region c base hC
      · exact hc g hg
      · exact hz Z hZ g hU
    · exact good_of_noTy c base (fun C hC => hx ⟨C, hC⟩)

end Vertep.Rdf.Equiv
