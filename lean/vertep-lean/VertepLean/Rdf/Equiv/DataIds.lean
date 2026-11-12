import VertepLean.Rdf.Equiv.Paths
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Data identifiers

`dataId_iff`: `vt:UniqueDataIdShape` holds on the projection of a chart with unique state
identifiers exactly when its data identifiers are unique. Data item nodes have distinct
fragments (`dataNodes_fst`), so two items with one identifier give a literal with two subjects.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-! ## Fragments -/

/-- Two fragments that are a colon-free head and a tail starting with a colon split alike. -/
theorem frag_split {a b x y : String} (ha : NC a.toList) (hb : NC b.toList)
    (hx : x.toList.head? = some ':') (hy : y.toList.head? = some ':') (h : a ++ x = b ++ y) :
    a = b ∧ x = y := by
  have h' := congrArg String.toList h
  simp only [String.toList_append] at h'
  have hab := nc_split ha hb (.inr hx) (.inr hy) h'
  refine ⟨String.toList_inj.1 hab, ?_⟩
  rw [hab] at h'
  exact String.toList_inj.1 (List.append_cancel_left h')

theorem nc_chart : NC "chart".toList := by simp [NC]

theorem head_colon (s : String) : (":" ++ s).toList.head? = some ':' := by
  simp [String.toList_append]

theorem toString_inj {m n : Nat} (h : toString m = toString n) : m = n := by
  have := congrArg String.toList h
  simp only [toList_toString] at this
  exact digits_inj.1 this

/-- If `L.map f` has no duplicates, `L.map g` has none exactly when `g` separates what `f` does. -/
theorem nodup_map_iff {α β γ : Type} (f : α → β) (g : α → γ) :
    ∀ (L : List α), (L.map f).Nodup →
      ((L.map g).Nodup ↔ ∀ a ∈ L, ∀ b ∈ L, g a = g b → f a = f b)
  | [], _ => by simp
  | a :: L, h => by
    simp only [List.map_cons, List.nodup_cons, List.mem_map, List.mem_cons] at h ⊢
    rw [nodup_map_iff f g L h.2]
    constructor
    · rintro ⟨h1, h2⟩ x hx y hy hxy
      rcases hx with rfl | hx <;> rcases hy with rfl | hy
      · rfl
      · exact absurd ⟨y, hy, hxy.symm⟩ h1
      · exact absurd ⟨x, hx, hxy⟩ h1
      · exact h2 x hx y hy hxy
    · intro H
      refine ⟨fun ⟨y, hy, hyg⟩ => h.1 ⟨y, hy, (H a (.inl rfl) y (.inr hy) hyg.symm).symm⟩,
        fun x hx y hy => H x (.inr hx) y (.inr hy)⟩

theorem nodup_flatMap_of {α β : Type} (f : α → List β) : ∀ (L : List α), (∀ a ∈ L, (f a).Nodup) →
    L.Pairwise (fun a b => ∀ x ∈ f a, x ∉ f b) → (L.flatMap f).Nodup
  | [], _, _ => by simp
  | a :: L, h1, h2 => by
    rw [List.flatMap_cons, List.nodup_append]
    rw [List.pairwise_cons] at h2
    refine ⟨h1 a (by simp), nodup_flatMap_of f L (fun b hb => h1 b (by simp [hb])) h2.2, ?_⟩
    intro x hx y hy hxy
    subst hxy
    obtain ⟨b, hb, hy⟩ := List.mem_flatMap.1 hy
    exact h2.1 b hb x hx hy

theorem map_fst_zipIdx {α : Type} : ∀ (L : List α) (n : Nat), (L.zipIdx n).map Prod.fst = L
  | [], _ => rfl
  | a :: L, n => by simp [List.zipIdx_cons, map_fst_zipIdx L (n + 1)]

theorem nodup_zipIdx_snd {α β : Type} (h : Nat → β) (hinj : ∀ m n, h m = h n → m = n) :
    ∀ (L : List α) (n : Nat), ((L.zipIdx n).map (fun p => h p.2)).Nodup
  | [], _ => by simp
  | a :: L, n => by
    simp only [List.zipIdx_cons, List.map_cons, List.nodup_cons, List.mem_map]
    refine ⟨fun ⟨p, hp, he⟩ => ?_, nodup_zipIdx_snd h hinj L (n + 1)⟩
    have := hinj _ _ he
    have h2 := List.mem_zipIdx hp
    omega


/-! ## Data identifiers -/

section
variable (c : Chart) (base : String)

theorem dataNode_mem {q : String × Data} (hq : q ∈ dataNodes c) :
    (⟨(ctxOf c base).iri q.1, vt "dataId", strLit q.2.id⟩ : Triple) ∈ project c base := by
  rw [project_eq]
  simp only [dataNodes, List.mem_append, List.mem_map, List.mem_flatMap] at hq
  rcases hq with ⟨⟨d, i⟩, hdi, rfl⟩ | ⟨Y, hY, hq⟩
  · refine List.mem_append_left _ (List.mem_append_left _ (List.mem_append_left _
      (List.mem_append_left _ (List.mem_append_right _ ?_))))
    refine List.mem_flatMap.2 ⟨dataK (ctxOf c base) "chart" (d, i), List.mem_map.2 ⟨_, hdi, rfl⟩, ?_⟩
    simp [dataK, Ctx.dataItem, Ctx.node, str]
  · obtain ⟨q', hq', rfl⟩ := allStates_parented hY
    obtain ⟨i', A'', h⟩ := parentedList_sub (ctxOf c base) none 0 [] c.states q' hq'
    simp only [stateData, List.mem_map] at hq
    obtain ⟨⟨d, j⟩, hdj, rfl⟩ := hq
    refine List.mem_append_left _ (List.mem_append_left _ (List.mem_append_right _ ?_))
    have := h _ (mem_state_of_kid (ctxOf c base) i' _ _ q'.2
      (d := dataK (ctxOf c base) (stateFragment q'.2.id) (d, j)) ?_ (u := ⟨(ctxOf c base).iri
        (stateFragment q'.2.id ++ ":d" ++ toString j), vt "dataId", strLit d.id⟩) ?_)
    · simpa [parT] using this
    · simp only [kidsOf, stateKids, List.mem_append]
      exact .inl (.inl (.inl (.inr (List.mem_map.2 ⟨_, hdj, rfl⟩))))
    · simp [dataK, Ctx.dataItem, Ctx.node, str]

theorem dataNodes_snd : (dataNodes c).map Prod.snd = c.allData := by
  simp only [dataNodes, Chart.allData, List.map_append, List.map_map, Function.comp_def,
    List.map_flatMap, stateData]
  congr 1
  · exact map_fst_zipIdx c.data 0
  · congr 1; funext Y; exact map_fst_zipIdx Y.data 0

theorem frag_d_inj {a b : String} {m n : Nat} (ha : NC a.toList) (hb : NC b.toList)
    (h : a ++ ":d" ++ toString m = b ++ ":d" ++ toString n) : a = b ∧ m = n := by
  rw [String.append_assoc, String.append_assoc] at h
  have := frag_split ha hb (by simp [String.toList_append]) (by simp [String.toList_append]) h
  refine ⟨this.1, toString_inj ?_⟩
  have h2 := congrArg String.toList this.2
  simp only [String.toList_append, List.append_cancel_left_eq] at h2
  exact String.toList_inj.1 h2

theorem dataNodes_fst (hnd : c.ids.Nodup) : ((dataNodes c).map Prod.fst).Nodup := by
  simp only [dataNodes, List.map_append, List.map_map, Function.comp_def, List.map_flatMap,
    stateData, List.nodup_append]
  have hS : c.allStates.Pairwise (fun a b => a.id ≠ b.id) := List.pairwise_map.1 hnd
  refine ⟨nodup_zipIdx_snd (fun i => "chart:d" ++ toString i) (fun m n h =>
      (frag_d_inj nc_chart nc_chart h).2) c.data 0, ?_, ?_⟩
  · refine nodup_flatMap_of _ _ (fun Y _ => nodup_zipIdx_snd (fun i => stateFragment Y.id ++ ":d" ++ toString i)
      (fun m n h => (frag_d_inj (nc_stateFragment _) (nc_stateFragment _) h).2) Y.data 0) ?_
    refine hS.imp fun {Y Z} hYZ x hx hx' => ?_
    simp only [List.mem_map] at hx hx'
    obtain ⟨p, _, rfl⟩ := hx
    obtain ⟨p', _, he⟩ := hx'
    exact hYZ (stateFragment_inj (frag_d_inj (nc_stateFragment _) (nc_stateFragment _) he).1).symm
  · intro x hx y hy hxy
    subst hxy
    simp only [List.mem_map, List.mem_flatMap] at hx hy
    obtain ⟨p, _, rfl⟩ := hx
    obtain ⟨Y, _, p', _, he⟩ := hy
    have := (frag_d_inj (nc_stateFragment _) nc_chart he).1
    have := congrArg String.toList this
    simp [stateFragment, String.toList_append] at this

/-- **`vt:UniqueDataIdShape` holds exactly when data identifiers are unique.** -/
-- @tg: implements:: [[openspec:formal-model#Shapes Equivalence Theorem]] {confidence: partial, note: "unique data identifiers; the full theorem is shapes_equivalence"}
theorem dataId_iff (hnd : c.ids.Nodup) :
    (∀ x s, (⟨s, .iri (vi "dataId"), x⟩ : Triple) ∈ toRDF c base →
      Conforms (toRDF c base) uniqueDataIdS x) ↔ (c.allData.map (·.id)).Nodup := by
  simp only [uniqueDataIdS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, holds_property,
    vn_none, forall_eq, propS, conforms_mk, maxC, holds_maxCount1, vn_some, ShapesL.I, ShapesL.P,
    valueNode_inv, valueNode_pred, iri_vi, toRDF, mem_ofList']
  have hmap : c.allData.map (·.id) = (dataNodes c).map (fun q => q.2.id) := by
    rw [← dataNodes_snd c, List.map_map]; rfl
  rw [hmap, nodup_map_iff Prod.fst (fun q => q.2.id) _ (dataNodes_fst c hnd)]
  constructor
  · intro H a ha b hb hab
    have h1 := dataNode_mem c base ha
    have h2 := dataNode_mem c base hb
    rw [hab] at h1
    exact (iri_inj _).1 (H _ _ h1 _ _ h1 h2)
  · intro H x s _ y z hy hz
    obtain ⟨a, ha, rfl, rfl⟩ := dataId_triple c base hy
    obtain ⟨b, hb, rfl, hab⟩ := dataId_triple c base hz
    rw [H a ha b hb (strLit_inj.1 hab)]

end

end Vertep.Rdf.Equiv
