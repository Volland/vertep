import VertepLean.Rdf.Equiv.Region
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes at the leaves

`good_iff` lists the node shapes by focus. Parameters, tokens, extension nodes and content nodes:
the node shapes hold at the node exactly when its local rule does (only parameters have one,
`<param>`: exactly one of `expr` and `location`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

section
variable (G : Graph) (h : NoSub G)
include h

theorem good_iff (x : Term) : Good G x ↔
    ((∃ l ∈ containedClasses, Ty G x l) → Conforms G containedS x) ∧
    (Ty G x "Chart" → Conforms G chartS x) ∧ (Ty G x "StateNode" → Conforms G stateNodeS x) ∧
    (Ty G x "State" → Conforms G stateS x) ∧ (Ty G x "Parallel" → Conforms G parallelS x) ∧
    (Ty G x "Final" → Conforms G finalS x) ∧ (Ty G x "History" → Conforms G historyS' x) ∧
    ((∃ s, (⟨s, vt "initial", x⟩ : Triple) ∈ G) → Conforms G initialRefScopeS x) ∧
    (Ty G x "Initial" → Conforms G initialS x) ∧ (Ty G x "Transition" → Conforms G transitionS x) ∧
    (Ty G x "Token" → Conforms G tokenS x) ∧ (Ty G x "OnEntry" ∨ Ty G x "OnExit" → Conforms G handlerS x) ∧
    (Ty G x "ExecutableContent" → Conforms G execS x) ∧ (Ty G x "Raise" → Conforms G raiseS x) ∧
    (Ty G x "Send" → Conforms G sendS x) ∧ (Ty G x "Cancel" → Conforms G cancelS x) ∧
    (Ty G x "Log" → Conforms G logS x) ∧ (Ty G x "Assign" → Conforms G assignS x) ∧
    (Ty G x "Script" → Conforms G scriptS x) ∧ (Ty G x "If" → Conforms G ifS x) ∧
    (Ty G x "IfBranch" → Conforms G ifBranchS x) ∧ (Ty G x "Foreach" → Conforms G foreachS x) ∧
    (Ty G x "ExtensionContent" → Conforms G extensionContentS x) ∧ (Ty G x "Param" → Conforms G paramS x) ∧
    (Ty G x "Content" → Conforms G contentS x) ∧ (Ty G x "DataItem" → Conforms G dataItemS x) ∧
    (Ty G x "DoneData" → Conforms G doneDataS x) ∧ (Ty G x "Invoke" → Conforms G invokeS x) ∧
    (Ty G x "ExtensionNode" → Conforms G extensionNodeS x) := by
  simp only [ShapesL.Good, nodeShapes, List.forall_mem_cons, List.forall_mem_nil, and_true,
    List.not_mem_nil, false_implies, implies_true,
    foc_containedS G h, foc_chartS G h, foc_stateNodeS G h, foc_stateS G h, foc_parallelS G h,
    foc_finalS G h, foc_historyS' G h, foc_initialRefScopeS, foc_initialS G h, foc_transitionS G h,
    foc_tokenS G h, foc_handlerS G h, foc_execS G h, foc_raiseS G h, foc_sendS G h, foc_cancelS G h,
    foc_logS G h, foc_assignS G h, foc_scriptS G h, foc_ifS G h, foc_ifBranchS G h,
    foc_foreachS G h, foc_extensionContentS G h, foc_paramS G h, foc_contentS G h,
    foc_dataItemS G h, foc_doneDataS G h, foc_invokeS G h, foc_extensionNodeS G h]

end

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

/-- The types of a node, from its captured emission. -/
theorem ty_view {X : String} {E : List Triple} (hcap : Cap k ts (U X) E) (l : String) :
    Ty (Graph.ofList ts) (k.iri X) l ↔ (⟨k.iri X, rdfType, vt l⟩ : Triple) ∈ E := by
  simp only [Ty, rdf_type_eq, mem_ofList']
  exact hcap.lookup (U_refl X) (by simp)

/-- The objects of a node, from its captured emission. -/
theorem obj_view {X : String} {E : List Triple} (hcap : Cap k ts (U X) E) (l : String) (o : Term)
    (hl : l ≠ "historyDefault") (hc : l ≠ "choice") :
    (⟨k.iri X, vt l, o⟩ : Triple) ∈ Graph.ofList ts ↔ (⟨k.iri X, vt l, o⟩ : Triple) ∈ E := by
  rw [mem_ofList']
  exact hcap.lookup (U_refl X) (by simp [hl, hc])

/-- The region of a node without children is the node itself. -/
theorem region_leaf (env : Env k ts P) {owner : Term} {prop X : String} {E : List Triple}
    (hcap : Cap k ts (U X) E) (hown : Above (k := k) owner X)
    (hcov : ∀ t ∈ E, Rel t → t = ⟨owner, vt prop, k.iri X⟩ ∨ t.s = k.iri X) :
    (∀ g, U X g → Good (Graph.ofList ts) (k.iri g)) ↔ Good (Graph.ofList ts) (k.iri X) := by
  have cx : Ctxt k ts owner prop X E [] :=
    ⟨hcap, ⟨fun t ht hr => (hcov t ht hr).elim .inl (fun h => .inr (.inl h)), by simp, by simp,
      by simp, by simp⟩, hown⟩
  rw [region_iff env cx]; simp

end Vertep.Rdf.Equiv
