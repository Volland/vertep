import VertepLean.Rdf.Equiv.Special
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# Regions of the projection

The node shapes hold at every node of a fragment's region exactly when they hold at the node and
in the regions of its children (`region_iff`): other fragments of the region carry no triples, and
the node shapes have no focus there. A child of a node with a single container, reachable from the
chart, has its parent as single container (`Contained.kid`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-- What every node lemma needs of the graph: no subclass triples, containment links between a
fragment and its children (or a parent and a child state), objects of `vt:initial` typed, and the
chart node `cn` typed `vt:Chart`. -/
structure Env (k : Ctx) (ts : List Triple) (P : List (Option State × State)) : Prop where
  noSub : NoSub (Graph.ofList ts)
  link : AllT (LinkOK k P) ts
  initTy : ∀ s o, (⟨s, vt "initial", o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt "TargetRef"⟩ : Triple) ∈ ts
  chart : (⟨k.iri "chart", rdfType, vt "Chart"⟩ : Triple) ∈ ts

theorem env_project (c : Chart) (base : String) :
    Env (ctxOf c base) (project c base) c.parented where
  noSub := noSub c base
  link := link_project c base
  initTy := fun _ _ h => initial_typed c base h
  chart := by rw [project_eq]; simp [chartSelf]

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

@[simp] theorem mem_ofList' {t : Triple} : t ∈ Graph.ofList ts ↔ t ∈ ts := Graph.mem_ofList

/-- No relevant triple has subject `k.iri g`. -/
def NoRel (k : Ctx) (ts : List Triple) (g : String) : Prop :=
  ∀ p o, Rel ⟨k.iri g, p, o⟩ → (⟨k.iri g, p, o⟩ : Triple) ∉ ts

theorem good_of_noRel (env : Env k ts P) {g : String} (h : NoRel k ts g) :
    Good (Graph.ofList ts) (k.iri g) := by
  have hty : ∀ l, ¬ Ty (Graph.ofList ts) (k.iri g) l := fun l hl => by
    simp only [Ty, rdf_type_eq, mem_ofList'] at hl
    exact h _ _ (by simp) hl
  have hin : ¬ ∃ s, (⟨s, vt "initial", k.iri g⟩ : Triple) ∈ ts := fun ⟨s, hs⟩ =>
    h _ _ (by simp) (env.initTy s _ hs)
  intro s hs hf
  simp only [nodeShapes, List.mem_cons, List.not_mem_nil, or_false] at hs
  rcases hs with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl |
    rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;>
    simp [foc_containedS _ env.noSub, foc_chartS _ env.noSub, foc_stateNodeS _ env.noSub,
      foc_stateS _ env.noSub, foc_parallelS _ env.noSub, foc_finalS _ env.noSub,
      foc_historyS' _ env.noSub, foc_initialRefScopeS, foc_initialS _ env.noSub,
      foc_transitionS _ env.noSub, foc_tokenS _ env.noSub, foc_handlerS _ env.noSub,
      foc_execS _ env.noSub, foc_raiseS _ env.noSub, foc_sendS _ env.noSub, foc_cancelS _ env.noSub,
      foc_logS _ env.noSub, foc_assignS _ env.noSub, foc_scriptS _ env.noSub, foc_ifS _ env.noSub,
      foc_ifBranchS _ env.noSub, foc_foreachS _ env.noSub, foc_extensionContentS _ env.noSub,
      foc_paramS _ env.noSub, foc_contentS _ env.noSub, foc_dataItemS _ env.noSub,
      foc_doneDataS _ env.noSub, foc_invokeS _ env.noSub, foc_extensionNodeS _ env.noSub, hty] at hf <;>
    exact absurd hf hin

/-- The node shapes hold in a region exactly when they hold at its node and in its children's
regions. -/
theorem region_iff (env : Env k ts P) {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (cx : Ctxt k ts owner prop X E D) :
    (∀ g, U X g → Good (Graph.ofList ts) (k.iri g)) ↔
      Good (Graph.ofList ts) (k.iri X) ∧ ∀ d ∈ D, ∀ g, U d.1 g → Good (Graph.ofList ts) (k.iri g) := by
  constructor
  · intro h
    exact ⟨h X (U_refl X), fun d hd g hg => h g (U_up (cx.dec.ch d hd) hg)⟩
  · rintro ⟨hX, hD⟩ g hg
    by_cases hgX : g = X
    · subst hgX; exact hX
    by_cases hk : ∃ d ∈ D, U d.1 g
    · obtain ⟨d, hd, hU⟩ := hk; exact hD d hd g hU
    apply good_of_noRel env
    intro p o hr ht
    have hE := cx.cap.cap _ ht hr g rfl hg
    rcases cx.dec.cov _ hE hr with he | he | ⟨d, hd, ht'⟩
    · simp only [Triple.mk.injEq] at he
      exact cx.own g hg he.1.symm
    · exact hgX ((iri_inj k).1 he)
    · rcases cx.dec.loc d hd _ ht' hr with he | ⟨g', hg', hU⟩
      · exact hgX ((iri_inj k).1 he)
      · exact hk ⟨d, hd, by rw [(iri_inj k).1 hg']; exact hU⟩

/-! ## Containers of children -/

theorem ch_uniq {g g' f : String} (h : Ch g f) (h' : Ch g' f) : g = g' := by
  obtain ⟨a, ha, hna⟩ := h
  obtain ⟨b, hb, hnb⟩ := h'
  rw [ha] at hb
  have : g.toList ++ [':'] ++ a = g'.toList ++ [':'] ++ b := by simpa using hb
  have hl : (g.toList ++ [':'] ++ a).reverse = (g'.toList ++ [':'] ++ b).reverse := by rw [this]
  simp only [List.reverse_append, List.reverse_cons, List.reverse_nil, List.nil_append,
    List.cons_append] at hl
  have hna' : ':' ∉ a.reverse := by simpa [NC] using hna
  have hnb' : ':' ∉ b.reverse := by simpa [NC] using hnb
  have key : ∀ {x y : List Char} {u v : List Char}, ':' ∉ x → ':' ∉ y →
      x ++ ':' :: u = y ++ ':' :: v → x = y ∧ u = v := by
    intro x y u v hx hy hxy
    induction x generalizing y with
    | nil =>
      cases y with
      | nil => simpa using hxy
      | cons c y => simp at hxy; exact absurd (List.mem_cons_self) (hxy.1 ▸ hy)
    | cons c x ih =>
      cases y with
      | nil => simp at hxy; exact absurd List.mem_cons_self (hxy.1 ▸ hx)
      | cons d y =>
        simp only [List.cons_append, List.cons.injEq] at hxy
        have := ih (fun h => hx (List.mem_cons_of_mem _ h)) (fun h => hy (List.mem_cons_of_mem _ h))
          hxy.2
        exact ⟨by rw [hxy.1, this.1], this.2⟩
  have := (key hna' hnb' hl).2
  exact String.toList_inj.1 (List.reverse_inj.1 this)

theorem not_ch_sf {g x : String} : ¬ Ch g (stateFragment x) := by
  rintro ⟨a, ha, _⟩
  have := nc_stateFragment x
  rw [ha] at this
  exact this (List.mem_append_right _ List.mem_cons_self)

/-- A node with a single container, `owner`, reached from the chart node. -/
structure Contained' (k : Ctx) (ts : List Triple) (x owner : Term) : Prop where
  conts : ∀ s, (∃ l ∈ containers, (⟨s, vt l, x⟩ : Triple) ∈ ts) ↔ s = owner
  up : owner = k.iri "chart" ∨ Plus (ValueNode (Graph.ofList ts) upPath) owner (k.iri "chart")

theorem Contained'.ok (env : Env k ts P) {x owner : Term} (h : Contained' k ts x owner) :
    Conforms (Graph.ofList ts) containedS x := by
  have hup : ∀ y, ValueNode (Graph.ofList ts) upPath x y ↔ y = owner := by
    intro y; rw [vn_up]; simpa using h.conts y
  have h1 : ValueNode (Graph.ofList ts) upPath x owner := (hup owner).2 rfl
  have hp : Plus (ValueNode (Graph.ofList ts) upPath) x (k.iri "chart") := by
    rcases h.up with he | hp
    · rw [he] at h1; exact .one h1
    · exact Plus.head h1 hp
  have hcn : InstanceOf (Graph.ofList ts) (k.iri "chart") (vt "Chart") :=
    (instanceOf_iff _ env.noSub _ _).2 (by simpa using env.chart)
  simp only [containedS, top, conforms_mk, compsHold_cons, compsHold_nil, holds_property, vn_none,
    forall_eq, propS, minC, maxC, holds_minCount1, holds_maxCount1, vn_some,
    holds_qualifiedMin1, valueNode_oneOrMore, and_true, hup, nodeS, cls, holds_class]
  exact ⟨⟨⟨owner, rfl⟩, fun y z hy hz => hy.trans hz.symm⟩, _, hp, hcn⟩

/-- A child `f` of a fragment `X` linked from it by a containment predicate has `X` as its single
container. -/
theorem Contained'.kid (env : Env k ts P) {X f : String} {owner : Term} {l : String}
    (hX : Contained' k ts (k.iri X) owner) (hch : Ch X f) (hl : l ∈ containers)
    (hlink : (⟨k.iri X, vt l, k.iri f⟩ : Triple) ∈ ts) : Contained' k ts (k.iri f) (k.iri X) where
  conts s := by
    constructor
    · rintro ⟨l', hl', ht⟩
      obtain ⟨f', hf', h⟩ := env.link _ ht ⟨l', hl', rfl⟩
      simp only [iri_inj] at hf'; subst hf'
      rcases h with ⟨g, hg, hs⟩ | ⟨q, _, hq, _⟩
      · exact hs.trans (by rw [ch_uniq hg hch])
      · exact absurd (hq ▸ hch) not_ch_sf
    · rintro rfl; exact ⟨l, hl, hlink⟩
  up := by
    have : ValueNode (Graph.ofList ts) upPath (k.iri X) owner := by
      rw [vn_up]; obtain ⟨l', hl', h⟩ := (hX.conts owner).2 rfl; exact ⟨l', hl', by simpa using h⟩
    rcases hX.up with he | hp
    · exact .inr (.one (he ▸ this))
    · exact .inr (Plus.head this hp)

end Vertep.Rdf.Equiv
