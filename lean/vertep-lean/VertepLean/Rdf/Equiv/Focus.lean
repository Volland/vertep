import VertepLean.Rdf.Equiv.Shacl

/-!
# Focus nodes of the chart shapes

The shapes split into the node shapes (`nodeShapes`: every shape with a class target, the cycle
component of `vt:HistoryShape` left out, and `vt:InitialRefScopeShape`), whose focus nodes are
nodes of the projection, and the rest (`vt:UniqueStateIdShape` and `vt:UniqueDataIdShape` on
literals, `vt:SelectionShape` on selection nodes, the history cycle component). `Good G x`: every
node shape that has `x` as a focus node holds at `x`. `dataConforms_iff` splits conformance to
the chart shapes along these lines.
-/

namespace Vertep.Rdf.ShapesL

open LeanRDF LeanRDF.Shacl

/-- `vt:HistoryShape` without its cycle component. -/
def historyS' : Shape :=
  top [tcls "History"]
    [pr "historyType" [minC 1, maxC 1, .test (.in_ [strLit "shallow", strLit "deep"])],
     pr "transition" [maxC 1, .node eventlessTargetedS],
     .property (propS (.alt [P "child", P "onentry", P "onexit", P "invoke", P "data", P "initial",
       P "initialElement", P "donedata"]) [maxC 0]),
     .property (propS (I "child") [cls "StateNode"]),
     .property (propS (.seq [P "transition", P "target"])
       [.property (propS (.seq [I "target", I "transition", I "child"]) [.equals (vi "scope")])])]

/-- The cycle component of `vt:HistoryShape`. -/
def cycleC : Component :=
  .property (propS (.seq [.oneOrMore (P "historyDefault"), P "id"]) [.disjoint (vi "id")])

/-- The node shapes. -/
def nodeShapes : List Shape :=
  [containedS, chartS, stateNodeS, stateS, parallelS, finalS, historyS', initialRefScopeS,
   initialS, transitionS, tokenS, handlerS, execS, raiseS, sendS, cancelS, logS, assignS, scriptS,
   ifS, ifBranchS, foreachS, extensionContentS, paramS, contentS, dataItemS, doneDataS, invokeS,
   extensionNodeS]

variable (G : Graph)

/-- Every node shape with focus node `x` holds at `x`. -/
def Good (x : Term) : Prop := ∀ s ∈ nodeShapes, IsFocus G s x → Conforms G s x

/-- `x` has type `vt:l`. -/
def Ty (x : Term) (l : String) : Prop := (⟨x, .iri Vocab.rdf.type, vt l⟩ : Triple) ∈ G

theorem isFocus_top (ts : List Target) (cs : List Component) (x : Term) :
    IsFocus G (top ts cs) x ↔ ∃ tg ∈ ts, TargetHolds G tg x := by
  simp [IsFocus, top, mk, Shape.targets]

theorem targetHolds_tcls (h : NoSub G) (l : String) (x : Term) :
    TargetHolds G (tcls l) x ↔ Ty G x l := by
  simp [tcls, TargetHolds, Ty, instanceOf_iff G h]

/-- Conformance to the chart shapes, split into the node shapes, the history cycles, the
selection nodes and the two uniqueness shapes. -/
theorem dataConforms_iff (h : NoSub G) :
    DataConforms G chartShapesL ↔
      (∀ x, Good G x) ∧ (∀ x, Ty G x "History" → CompHolds G none cycleC x) ∧
      (∀ x o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ G → Conforms G selectionS x) ∧
      (∀ x s, (⟨s, .iri (vi "id"), x⟩ : Triple) ∈ G → Conforms G uniqueStateIdS x) ∧
      (∀ x s, (⟨s, .iri (vi "dataId"), x⟩ : Triple) ∈ G → Conforms G uniqueDataIdS x) := by
  have hH : ∀ x, Conforms G historyS x ↔ Conforms G historyS' x ∧ CompHolds G none cycleC x := by
    intro x
    simp only [historyS, historyS', cycleC, conforms_top, compsHold_cons, compsHold_nil, and_true]
    constructor
    · rintro ⟨a, b, c, d, e, f⟩; exact ⟨⟨a, b, c, d, f⟩, e⟩
    · rintro ⟨⟨a, b, c, d, f⟩, e⟩; exact ⟨a, b, c, d, e, f⟩
  have hF : ∀ x, IsFocus G historyS x ↔ IsFocus G historyS' x := fun x => by
    simp [historyS, historyS', isFocus_top]
  simp only [DataConforms, chartShapesL, Good, nodeShapes, List.mem_cons, List.not_mem_nil,
    or_false, forall_eq_or_imp, forall_eq, hH, hF]
  have fS : ∀ x, IsFocus G selectionS x ↔ ∃ o, (⟨x, .iri (vi "choice"), o⟩ : Triple) ∈ G := by
    intro x; simp [selectionS, isFocus_top, TargetHolds]
  have fU : ∀ x, IsFocus G uniqueStateIdS x ↔ ∃ s, (⟨s, .iri (vi "id"), x⟩ : Triple) ∈ G := by
    intro x; simp [uniqueStateIdS, isFocus_top, TargetHolds]
  have fD : ∀ x, IsFocus G uniqueDataIdS x ↔ ∃ s, (⟨s, .iri (vi "dataId"), x⟩ : Triple) ∈ G := by
    intro x; simp [uniqueDataIdS, isFocus_top, TargetHolds]
  have fHy : ∀ x, IsFocus G historyS' x ↔ Ty G x "History" := by
    intro x; simp [historyS', isFocus_top, targetHolds_tcls G h]
  simp only [fS, fU, fD, fHy]
  constructor
  · rintro ⟨h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, h13, h14, h15, h16, h17, h18, h19,
      h20, h21, h22, h23, h24, h25, h26, h27, h28, h29, h30, h31, h32⟩
    refine ⟨fun x => ⟨h1 x, h2 x, h3 x, h5 x, h6 x, h7 x, fun hx => (h8 x hx).1, h9 x, h10 x, h11 x,
      h13 x, h14 x, h15 x, h16 x, h17 x, h18 x, h19 x, h20 x, h21 x, h22 x, h23 x, h24 x, h25 x,
      h26 x, h27 x, h28 x, h30 x, h31 x, h32 x⟩, fun x hx => (h8 x hx).2,
      fun x o hx => h12 x ⟨o, hx⟩, fun x s hx => h4 x ⟨s, hx⟩, fun x s hx => h29 x ⟨s, hx⟩⟩
  · rintro ⟨hg, hc, hs, hu, hd⟩
    refine ⟨fun x => (hg x).1, fun x => (hg x).2.1, fun x => (hg x).2.2.1,
      fun x ⟨s, hx⟩ => hu x s hx, fun x => (hg x).2.2.2.1, fun x => (hg x).2.2.2.2.1,
      fun x => (hg x).2.2.2.2.2.1, fun x hx => ⟨(hg x).2.2.2.2.2.2.1 hx, hc x hx⟩,
      fun x => (hg x).2.2.2.2.2.2.2.1, fun x => (hg x).2.2.2.2.2.2.2.2.1,
      fun x => (hg x).2.2.2.2.2.2.2.2.2.1, fun x ⟨o, hx⟩ => hs x o hx, ?_⟩
    have := fun x => (hg x).2.2.2.2.2.2.2.2.2.2
    refine ⟨fun x => (this x).1, fun x => (this x).2.1, fun x => (this x).2.2.1,
      fun x => (this x).2.2.2.1, fun x => (this x).2.2.2.2.1, fun x => (this x).2.2.2.2.2.1,
      fun x => (this x).2.2.2.2.2.2.1, fun x => (this x).2.2.2.2.2.2.2.1,
      fun x => (this x).2.2.2.2.2.2.2.2.1, fun x => (this x).2.2.2.2.2.2.2.2.2.1, ?_⟩
    have := fun x => (this x).2.2.2.2.2.2.2.2.2.2
    refine ⟨fun x => (this x).1, fun x => (this x).2.1, fun x => (this x).2.2.1,
      fun x => (this x).2.2.2.1, fun x => (this x).2.2.2.2.1, fun x => (this x).2.2.2.2.2.1,
      fun x ⟨s, hx⟩ => hd x s hx, fun x => (this x).2.2.2.2.2.2.1,
      fun x => (this x).2.2.2.2.2.2.2.1, fun x => (this x).2.2.2.2.2.2.2.2⟩

end Vertep.Rdf.ShapesL
