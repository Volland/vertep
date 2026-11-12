import VertepLean.Rdf.Equiv.Tree
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Ancestors in the projection

For a chart with unique state identifiers, the `vt:ancestor` objects of a state node are the
chart node and the nodes of the states whose `descIds` contain the state's identifier; so the
target condition `TR` is: the target is a state of the chart, and the scope is the chart node or
a state above it (`tr_iff`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

section
variable (c : Chart) (base : String) (hnd : c.ids.Nodup)
include hnd

/-- The ancestors of a state, semantically. -/
def AncSem (c : Chart) (base : String) (Y : State) (v : Term) : Prop :=
  v = (ctxOf c base).iri "chart" ∨ ∃ W ∈ c.allStates, v = (ctxOf c base).iri (stateFragment W.id) ∧ Y.id ∈ W.descIds

mutual
theorem anc_rec : ∀ (Y : State) (pp : Option State) (i : Nat) (A' : List Term),
    RecH (ctxOf c base) (project c base) c.parented pp Y i A' →
    (∀ v, v ∈ (ctxOf c base).iri "chart" :: A' ↔ AncSem c base Y v) →
    ∀ q ∈ State.parented pp Y, ∀ v,
      (⟨(ctxOf c base).iri (stateFragment q.2.id), vt "ancestor", v⟩ : Triple) ∈ project c base ↔ AncSem c base q.2 v
  | .mk id ig el ini ie children trs en ex da inv dd xs, pp, i, A', h, hA, q, hq, v => by
    have env := env_project c base
    have hnd' := parented_ids_nodup c hnd
    rw [parented_eq] at hq
    rcases List.mem_cons.1 hq with rfl | hq
    · have hst := h.stctx env hnd'
      rw [state_view hst.cap (above_ne hst.own) _ _ (by decide) (by decide)]
      simp only [String.reduceEq, false_and, false_or, or_false, true_and]
      exact hA v
    · have hk := h.kids env hnd'
      exact anc_recs (some (.mk id ig el ini ie children trs en ex da inv dd xs))
        (A' ++ [(ctxOf c base).iri (stateFragment id)]) children 0 hk (fun Z hZ v => by
          have hqZ : (some (State.mk id ig el ini ie children trs en ex da inv dd xs), Z) ∈ c.parented :=
            (h.kids env hnd' |> fun _ => h.sub _ (by
              rw [parented_eq]; exact List.mem_cons_of_mem _ (mem_parentedList_of hZ (by
                rw [parented_eq]; exact List.mem_cons_self))))
          have hYa : State.mk id ig el ini ie children trs en ex da inv dd xs ∈ c.allStates :=
            parented_allStates (h.sub _ (by rw [parented_eq]; exact List.mem_cons_self))
          simp only [List.cons_append, List.mem_cons, List.mem_append, List.mem_singleton, List.not_mem_nil, or_false, AncSem]
          have := hA v
          simp only [List.mem_cons, AncSem] at this
          constructor
          · rintro (h1 | h1 | h1)
            · exact .inl h1
            · rcases this.1 (.inr h1) with h2 | ⟨W, hW, rfl, hW'⟩
              · exact .inl h2
              · exact .inr ⟨W, hW, rfl, (mem_descIds_iff hnd hqZ hW).2 ⟨_, rfl, .inr hW'⟩⟩
            · exact .inr ⟨_, hYa, h1, (mem_descIds_iff hnd hqZ hYa).2 ⟨_, rfl, .inl rfl⟩⟩
          · rintro (h1 | ⟨W, hW, rfl, hW'⟩)
            · exact .inl h1
            · obtain ⟨P, hP, hPW | hPW⟩ := (mem_descIds_iff hnd hqZ hW).1 hW'
              · cases hP
                right; right
                show _ = (ctxOf c base).iri (stateFragment id)
                rw [show id = W.id from hPW]
              · cases hP
                rcases this.2 (.inr ⟨W, hW, rfl, hPW⟩) with h2 | h2
                · exact .inl h2
                · exact .inr (.inl h2)) q hq v
theorem anc_recs (pp : Option State) (A' : List Term) :
    ∀ (cs : List State) (i : Nat), (∀ p ∈ cs.zipIdx i, RecH (ctxOf c base) (project c base) c.parented pp p.1 p.2 A') →
    (∀ Z ∈ cs, ∀ v, v ∈ (ctxOf c base).iri "chart" :: A' ↔ AncSem c base Z v) →
    ∀ q ∈ State.parentedList pp cs, ∀ v,
      (⟨(ctxOf c base).iri (stateFragment q.2.id), vt "ancestor", v⟩ : Triple) ∈ project c base ↔ AncSem c base q.2 v
  | [], _, _, _, q, hq, _ => by simp [State.parentedList] at hq
  | Z :: cs, i, h, hA, q, hq, v => by
    rw [State.parentedList, List.mem_append] at hq
    rcases hq with hq | hq
    · exact anc_rec Z pp i A' (h (Z, i) (by simp)) (hA Z (by simp)) q hq v
    · exact anc_recs pp A' cs (i + 1) (fun p hp => h p (by simp [hp]))
        (fun Z' hZ' => hA Z' (by simp [hZ'])) q hq v
end

theorem anc_all : ∀ q ∈ c.parented, ∀ v,
    (⟨(ctxOf c base).iri (stateFragment q.2.id), vt "ancestor", v⟩ : Triple) ∈ project c base ↔
      AncSem c base q.2 v := by
  intro q hq v
  refine anc_recs c base hnd none [] c.states 0 (top_rec c base hnd) (fun Z hZ v => ?_) q hq v
  simp only [List.mem_singleton, AncSem]
  have hq' : (none, Z) ∈ c.parented := by
    rw [Chart.parented]; exact mem_parentedList_of hZ (by rw [parented_eq]; exact List.mem_cons_self)
  constructor
  · exact .inl
  · rintro (h | ⟨W, hW, _, hW'⟩)
    · exact h
    · obtain ⟨P, hP, _⟩ := (mem_descIds_iff hnd hq' hW).1 hW'; cases hP

theorem chart_ne_state (x : String) : (ctxOf c base).iri "chart" ≠ (ctxOf c base).iri (stateFragment x) := by
  intro h
  have := congrArg String.toList ((iri_inj _).1 h)
  simp [stateFragment, String.toList_append] at this

theorem tr_chart (x : String) :
    TR (ctxOf c base) (project c base) (· ∈ c.ids) x ((ctxOf c base).iri "chart") ↔ x ∈ c.ids := by
  constructor
  · exact fun h => h.1
  · intro hx
    refine ⟨hx, ?_⟩
    obtain ⟨Z, hZ, rfl⟩ := List.mem_map.1 hx
    obtain ⟨q, hq, rfl⟩ := allStates_parented hZ
    exact (anc_all c base hnd q hq _).2 (.inl rfl)

theorem tr_state (x : String) {Y : State} (hY : Y ∈ c.allStates) :
    TR (ctxOf c base) (project c base) (· ∈ c.ids) x ((ctxOf c base).iri (stateFragment Y.id)) ↔
      x ∈ c.ids ∧ x ∈ Y.descIds := by
  constructor
  · rintro ⟨hx, h⟩
    refine ⟨hx, ?_⟩
    obtain ⟨Z, hZ, rfl⟩ := List.mem_map.1 hx
    obtain ⟨q, hq, rfl⟩ := allStates_parented hZ
    rcases (anc_all c base hnd q hq _).1 h with h1 | ⟨W, hW, h1, h2⟩
    · exact absurd h1.symm (chart_ne_state c base hnd _)
    · have := state_uniq hnd hW hY (stateFragment_inj ((iri_inj _).1 h1)).symm
      rw [← this]; exact h2
  · rintro ⟨hx, hd⟩
    refine ⟨hx, ?_⟩
    obtain ⟨Z, hZ, rfl⟩ := List.mem_map.1 hx
    obtain ⟨q, hq, rfl⟩ := allStates_parented hZ
    exact (anc_all c base hnd q hq _).2 (.inr ⟨Y, hY, rfl, hd⟩)

end

end Vertep.Rdf.Equiv
