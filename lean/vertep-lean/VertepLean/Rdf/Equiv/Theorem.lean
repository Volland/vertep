import VertepLean.Rdf.Equiv.StateRules
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# The Shapes Equivalence Theorem: the node shapes

`WFNodes c` is `WellFormed c` without the rules the chart shapes state over the whole chart
(unique state identifiers, target sets, unique data identifiers, history default cycles):
`wellFormed_iff` splits `WellFormed` along that line.

* `nodeShapes_iff`: on a chart with unique state identifiers, every node shape (every chart shape
  with a class target, `vt:InitialRefScopeShape`, and `vt:HistoryShape` without its cycle
  component) holds on the projection exactly when `WFNodes c`.
* `uniqueStateId_ok`: `vt:UniqueStateIdShape` holds on every projection.
* `shapes_equiv_nodes`: conformance to the chart shapes is `WFNodes c` together with the three
  remaining shapes on the selection nodes, the history nodes' cycles and the data identifiers.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-- The rules of `WellFormed` the node shapes check. -/
def WFNodes (c : Chart) : Prop :=
  c.states ≠ [] ∧ (∀ s ∈ c.allStates, s.id ≠ "") ∧ (∀ x ∈ c.initial, x ∈ c.ids) ∧
  (∀ s ∈ c.states, s.isHistory = false) ∧ (∀ d ∈ c.allData, DataOk d) ∧
  ∀ p ∈ c.parented, StateOkN c p.1 p.2

/-- `WellFormed` split into the node rules and the rules over the whole chart. -/
theorem wellFormed_iff (c : Chart) :
    WellFormed c ↔ c.ids.Nodup ∧ WFNodes c ∧ TargetSetOk c c.initial ∧ (∀ p ∈ c.parented, TSets c p.2) ∧
      (c.allData.map (·.id)).Nodup ∧ HistoryDefaultsAcyclic c := by
  simp only [WellFormed, WFNodes, stateOk_iff]
  constructor
  · rintro ⟨a, b, cc, d, e, f, g, h, i, j⟩
    exact ⟨cc, ⟨a, b, d, f, h, fun p hp => (i p hp).1⟩, e, fun p hp => (i p hp).2, g, j⟩
  · rintro ⟨cc, ⟨a, b, d, f, h, i⟩, e, i2, g, j⟩
    exact ⟨a, b, cc, d, e, f, g, h, fun p hp => ⟨i p hp, i2 p hp⟩, j⟩

theorem mem_parented_iff {c : Chart} {Z : State} : Z ∈ c.allStates ↔ ∃ q ∈ c.parented, q.2 = Z := by
  constructor
  · exact allStates_parented
  · rintro ⟨q, hq, rfl⟩; exact parented_allStates hq

set_option maxHeartbeats 1000000 in
/-- **The node shapes hold on the projection of a chart with unique state identifiers exactly when
its node rules do.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "node shapes; the full theorem is shapes_equivalence"}
theorem nodeShapes_iff (c : Chart) (base : String) (hnd : c.ids.Nodup) :
    (∀ x, ShapesL.Good (toRDF c base) x) ↔ WFNodes c := by
  rw [goodAll_iff c base hnd]
  simp only [tr_chart c base hnd]
  have hs : (∀ q ∈ c.parented, SOK (ctxOf c base) (project c base) (· ∈ c.ids) q.1 q.2) ↔
      ∀ q ∈ c.parented, q.2.id ≠ "" ∧ (∀ d ∈ q.2.data, DataOk d) ∧ StateOkN c q.1 q.2 ∧
        (q.2.isHistory = true → q.1.isSome = true) :=
    forall₂_congr fun q hq => sok_iff c base hnd hq
  rw [hs]
  have hroot : ∀ q ∈ c.parented, q.1 = none → q.2 ∈ c.states := by
    intro q hq h1
    rcases parentedList_shape none c.states q hq with ⟨_, h2⟩ | ⟨P, _, h2, _⟩
    · exact h2
    · rw [h1] at h2; cases h2
  simp only [WFNodes, Chart.allData, List.mem_append, List.mem_flatMap]
  constructor
  · rintro ⟨⟨⟨h1, h2, h3⟩, h4⟩, h5⟩
    refine ⟨h1, fun s hs => ?_, h2, h3, fun d hd => ?_, fun p hp => (h5 p hp).2.2.1⟩
    · obtain ⟨q, hq, rfl⟩ := allStates_parented hs; exact (h5 q hq).1
    · rcases hd with hd | ⟨s, hs, hd⟩
      · exact h4 d hd
      · obtain ⟨q, hq, rfl⟩ := allStates_parented hs; exact (h5 q hq).2.1 d hd
  · rintro ⟨h1, h2, h3, h4, h5, h6⟩
    refine ⟨⟨⟨h1, h3, h4⟩, fun d hd => h5 d (.inl hd)⟩, fun q hq => ⟨h2 _ (parented_allStates hq),
      fun d hd => h5 d (.inr ⟨q.2, parented_allStates hq, hd⟩), h6 q hq, fun hh => ?_⟩⟩
    rcases hq1 : q.1 with _ | P
    · have := h4 _ (hroot q hq hq1); rw [this] at hh; cases hh
    · rfl

/-- `vt:UniqueStateIdShape` holds on every projection: two state nodes with the same `vt:id` are
the same node. -/
theorem uniqueStateId_ok (c : Chart) (base : String) :
    ∀ x s, (⟨s, .iri (vi "id"), x⟩ : Triple) ∈ toRDF c base → Conforms (toRDF c base) uniqueStateIdS x := by
  intro x _ _
  simp only [uniqueStateIdS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, holds_property,
    vn_none, forall_eq, propS, conforms_mk, maxC, holds_maxCount1, vn_some, ShapesL.I, ShapesL.P,
    valueNode_inv, valueNode_pred, iri_vi, toRDF, mem_ofList']
  intro y z hy hz
  obtain ⟨Y, _, rfl, rfl⟩ := id_triple c base hy
  obtain ⟨Z, _, rfl, hZ⟩ := id_triple c base hz
  rw [strLit_inj] at hZ; rw [hZ]

/-- **Conformance to the chart shapes, for a chart with unique state identifiers**: the node rules
of `WellFormed` together with the shapes over the whole chart (selection nodes, history default
cycles, data identifiers), which `selection_iff`, `cycle_iff` and `dataId_iff` tie to the
remaining rules of `WellFormed`. -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "conformance split by shape; the full theorem is shapes_equivalence"}
theorem shapes_equiv_nodes (c : Chart) (base : String) (hnd : c.ids.Nodup) :
    (Shacl.validate (toRDF c base) chartShapesL).conforms = true ↔
      WFNodes c ∧
      (∀ x, Ty (toRDF c base) x "History" → CompHolds (toRDF c base) none cycleC x) ∧
      (∀ x o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ toRDF c base → Conforms (toRDF c base) selectionS x) ∧
      (∀ x s, (⟨s, .iri (vi "dataId"), x⟩ : Triple) ∈ toRDF c base → Conforms (toRDF c base) uniqueDataIdS x) := by
  rw [validate_conforms, dataConforms_iff _ (noSub c base), nodeShapes_iff c base hnd]
  constructor
  · rintro ⟨h1, h2, h3, _, h5⟩; exact ⟨h1, h2, h3, h5⟩
  · rintro ⟨h1, h2, h3, h5⟩; exact ⟨h1, h2, h3, uniqueStateId_ok c base, h5⟩

end Vertep.Rdf.Equiv
