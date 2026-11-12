import VertepLean.Rdf.Equiv.Lits

/-!
# Node shapes at one node

Focus nodes of each node shape, `vt:ContainedShape` at a node with exactly one container that
reaches the chart, and the node shapes at a node of each kind, from the node's local view: its
types, its objects for each predicate, and its containers.
-/

namespace Vertep.Rdf.ShapesL

open LeanRDF LeanRDF.Shacl

variable (G : Graph) (h : NoSub G)

theorem isFocus_mk (p : Option Path) (ts : List Target) (cs : List Component) (x : Term) :
    IsFocus G (mk p ts cs) x ↔ ∃ tg ∈ ts, TargetHolds G tg x := by
  simp [IsFocus, mk, Shape.targets]

include h

theorem targetHolds_class (l : String) (x : Term) :
    TargetHolds G (.class_ (vt l)) x ↔ Ty G x l := by
  simp [TargetHolds, Ty, instanceOf_iff G h]

@[simp] theorem foc_containedS (x : Term) :
    IsFocus G containedS x ↔ ∃ l ∈ containedClasses, Ty G x l := by
  simp only [containedS, top, isFocus_mk, List.mem_map]
  constructor
  · rintro ⟨_, ⟨l, hl, rfl⟩, ht⟩; exact ⟨l, hl, (targetHolds_class G h l x).1 ht⟩
  · rintro ⟨l, hl, ht⟩; exact ⟨_, ⟨l, hl, rfl⟩, (targetHolds_class G h l x).2 ht⟩

@[simp] theorem foc_chartS (x : Term) : IsFocus G chartS x ↔ Ty G x "Chart" := by
  simp [chartS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_stateNodeS (x : Term) : IsFocus G stateNodeS x ↔ Ty G x "StateNode" := by
  simp [stateNodeS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_stateS (x : Term) : IsFocus G stateS x ↔ Ty G x "State" := by
  simp [stateS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_parallelS (x : Term) : IsFocus G parallelS x ↔ Ty G x "Parallel" := by
  simp [parallelS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_finalS (x : Term) : IsFocus G finalS x ↔ Ty G x "Final" := by
  simp [finalS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_historyS' (x : Term) : IsFocus G historyS' x ↔ Ty G x "History" := by
  simp [historyS', isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_initialS (x : Term) : IsFocus G initialS x ↔ Ty G x "Initial" := by
  simp [initialS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_transitionS (x : Term) : IsFocus G transitionS x ↔ Ty G x "Transition" := by
  simp [transitionS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_tokenS (x : Term) : IsFocus G tokenS x ↔ Ty G x "Token" := by
  simp [tokenS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_handlerS (x : Term) : IsFocus G handlerS x ↔ Ty G x "OnEntry" ∨ Ty G x "OnExit" := by
  simp [handlerS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_execS (x : Term) : IsFocus G execS x ↔ Ty G x "ExecutableContent" := by
  simp [execS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_raiseS (x : Term) : IsFocus G raiseS x ↔ Ty G x "Raise" := by
  simp [raiseS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_sendS (x : Term) : IsFocus G sendS x ↔ Ty G x "Send" := by
  simp [sendS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_cancelS (x : Term) : IsFocus G cancelS x ↔ Ty G x "Cancel" := by
  simp [cancelS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_logS (x : Term) : IsFocus G logS x ↔ Ty G x "Log" := by
  simp [logS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_assignS (x : Term) : IsFocus G assignS x ↔ Ty G x "Assign" := by
  simp [assignS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_scriptS (x : Term) : IsFocus G scriptS x ↔ Ty G x "Script" := by
  simp [scriptS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_ifS (x : Term) : IsFocus G ifS x ↔ Ty G x "If" := by
  simp [ifS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_ifBranchS (x : Term) : IsFocus G ifBranchS x ↔ Ty G x "IfBranch" := by
  simp [ifBranchS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_foreachS (x : Term) : IsFocus G foreachS x ↔ Ty G x "Foreach" := by
  simp [foreachS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_extensionContentS (x : Term) : IsFocus G extensionContentS x ↔ Ty G x "ExtensionContent" := by
  simp [extensionContentS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_paramS (x : Term) : IsFocus G paramS x ↔ Ty G x "Param" := by
  simp [paramS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_contentS (x : Term) : IsFocus G contentS x ↔ Ty G x "Content" := by
  simp [contentS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_dataItemS (x : Term) : IsFocus G dataItemS x ↔ Ty G x "DataItem" := by
  simp [dataItemS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_doneDataS (x : Term) : IsFocus G doneDataS x ↔ Ty G x "DoneData" := by
  simp [doneDataS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_invokeS (x : Term) : IsFocus G invokeS x ↔ Ty G x "Invoke" := by
  simp [invokeS, isFocus_mk, targetHolds_class G h]

@[simp] theorem foc_extensionNodeS (x : Term) : IsFocus G extensionNodeS x ↔ Ty G x "ExtensionNode" := by
  simp [extensionNodeS, isFocus_mk, targetHolds_class G h]

omit h in
@[simp] theorem foc_initialRefScopeS (x : Term) :
    IsFocus G initialRefScopeS x ↔ ∃ s, (⟨s, vt "initial", x⟩ : Triple) ∈ G := by
  simp [initialRefScopeS, isFocus_mk, TargetHolds]

omit h

theorem vn_up (x y : Term) :
    ValueNode G upPath x y ↔ ∃ l ∈ containers, (⟨y, vt l, x⟩ : Triple) ∈ G := by
  simp only [upPath, valueNode_alt, List.mem_map]
  constructor
  · rintro ⟨_, ⟨l, hl, rfl⟩, hv⟩; exact ⟨l, hl, by simpa using hv⟩
  · rintro ⟨l, hl, hv⟩; exact ⟨_, ⟨l, hl, rfl⟩, by simpa using hv⟩

/-- A node with exactly one container, `owner`, from which the chart node `cn` is reached. -/
structure Contained (x owner cn : Term) (prop : String) : Prop where
  prop_mem : prop ∈ containers
  conts : ∀ s l, l ∈ containers → ((⟨s, vt l, x⟩ : Triple) ∈ G ↔ s = owner ∧ l = prop)
  up : owner = cn ∨ Plus (ValueNode G upPath) owner cn
  chart : Ty G cn "Chart"

include h in
theorem contained_ok {x owner cn : Term} {prop : String} (hc : Contained G x owner cn prop) :
    Conforms G containedS x := by
  have hup : ∀ y, ValueNode G upPath x y ↔ y = owner := by
    intro y
    rw [vn_up]
    constructor
    · rintro ⟨l, hl, hy⟩; exact ((hc.conts y l hl).1 hy).1
    · intro hy; rw [hy]; exact ⟨prop, hc.prop_mem, (hc.conts owner prop hc.prop_mem).2 ⟨rfl, rfl⟩⟩
  have h1 : ValueNode G upPath x owner := (hup owner).2 rfl
  have hp : Plus (ValueNode G upPath) x cn := by
    rcases hc.up with he | hp
    · rw [he] at h1; exact .one h1
    · exact Plus.head h1 hp
  have hcn : InstanceOf G cn (vt "Chart") := (instanceOf_iff G h _ _).2 (by simpa [Ty] using hc.chart)
  simp only [containedS, top, conforms_mk, compsHold_cons, compsHold_nil, holds_property, vn_none,
    forall_eq, propS, minC, maxC, holds_minCount1, holds_maxCount1, vn_some,
    holds_qualifiedMin1, valueNode_oneOrMore, and_true, hup, nodeS, cls, holds_class]
  exact ⟨⟨⟨owner, rfl⟩, fun y z hy hz => hy.trans hz.symm⟩, cn, hp, hcn⟩

end Vertep.Rdf.ShapesL
