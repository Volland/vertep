import VertepLean.Rdf.Equiv.Global
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# Where the special triples of the projection come from

A triple is `Clean` when its predicate is none of `vt:initial`, `vt:id`, `vt:dataId`,
`vt:choice`, `vt:historyDefault` and it does not type a node `vt:StateNode` or `vt:TargetRef`.
`cls_project`: every triple of the projection is clean, or comes from one of the target lists of
the chart (`tcalls`), is a state's `vt:StateNode` type or `vt:id`, a data item's `vt:dataId`, or a
`vt:historyDefault` triple.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

def special : List String := ["initial", "id", "dataId", "choice", "historyDefault"]

/-- Not a special triple. -/
def Clean (t : Triple) : Prop :=
  (∀ l ∈ special, t.p ≠ vt l) ∧ (t.p = rdfType → t.o ≠ vt "StateNode" ∧ t.o ≠ vt "TargetRef")

/-- The arguments of one call of `Ctx.targets`. -/
structure TCall where
  owner : Term
  prop : String
  list : String
  step : String
  ids : List String
  scope : Term

def TCall.emit (k : Ctx) (C : TCall) : List Triple :=
  k.targets C.owner C.prop C.list C.step C.ids C.scope

/-- The scope of a state's transitions' targets. -/
def scopeT (k : Ctx) (pp : Option State) (Y : State) : Term :=
  match Y.element with
  | .history _ => parT k pp
  | _ => k.iri "chart"

/-- The target lists of a state: its `initial` attribute, its `<initial>` transition and its
transitions. -/
def stateCalls (k : Ctx) (pp : Option State) (Y : State) : List TCall :=
  ⟨k.iri (stateFragment Y.id), "initial", stateFragment Y.id ++ ":ia", "", Y.initial,
    k.iri (stateFragment Y.id)⟩ ::
  ((Y.initialElement.map fun ie => (⟨k.iri (stateFragment Y.id ++ ":init" ++ ":t"), "target",
      stateFragment Y.id ++ ":init" ++ ":t", ":tg", ie.transition.targets,
      k.iri (stateFragment Y.id)⟩ : TCall)).toList ++
   Y.transitions.zipIdx.map fun p => (⟨k.iri (stateFragment Y.id ++ ":t" ++ toString p.2),
      "target", stateFragment Y.id ++ ":t" ++ toString p.2, ":tg", p.1.targets, scopeT k pp Y⟩ : TCall))

/-- Every target list of the chart. -/
def tcalls (k : Ctx) (c : Chart) : List TCall :=
  ⟨k.iri "chart", "initial", "chart:ia", "", c.initial, k.iri "chart"⟩ ::
    c.parented.flatMap fun q => stateCalls k q.1 q.2

/-- Every data item with the fragment of its node. -/
def stateData (Y : State) : List (String × Data) :=
  Y.data.zipIdx.map fun p => (stateFragment Y.id ++ ":d" ++ toString p.2, p.1)

def dataNodes (c : Chart) : List (String × Data) :=
  c.data.zipIdx.map (fun p => ("chart:d" ++ toString p.2, p.1)) ++ c.allStates.flatMap stateData

/-- The special triples, by origin. -/
def Special (k : Ctx) (c : Chart) (t : Triple) : Prop :=
  (∃ C ∈ tcalls k c, t ∈ C.emit k) ∨
  (∃ Y ∈ c.allStates, t = ⟨k.iri (stateFragment Y.id), rdfType, vt "StateNode"⟩ ∨
    t = ⟨k.iri (stateFragment Y.id), vt "id", strLit Y.id⟩) ∨
  (∃ q ∈ dataNodes c, t = ⟨k.iri q.1, vt "dataId", strLit q.2.id⟩) ∨
  t ∈ historyDefaultsList k none c.states

section
variable (k : Ctx)

theorem clean_vt (s o : Term) (l : String) (h : l ∉ special) : Clean ⟨s, vt l, o⟩ := by
  refine ⟨fun l' hl' he => ?_, fun he => absurd he (vt_ne_rdfType l)⟩
  simp only [vt_inj] at he; subst he; exact h hl'

theorem clean_type (s : Term) (c : String) (h1 : c ≠ "StateNode") (h2 : c ≠ "TargetRef") :
    Clean ⟨s, rdfType, vt c⟩ :=
  ⟨fun l _ he => vt_ne_rdfType l he.symm, fun _ => ⟨by simpa using h1, by simpa using h2⟩⟩

syntax "cln" : tactic
macro_rules
  | `(tactic| cln) => `(tactic| first
      | (apply clean_vt; simp [special])
      | (apply clean_type <;> simp))

theorem cl_str (n : Term) (p v : String) (h : p ∉ special) : AllT Clean (str n p v) := by
  simp [str, clean_vt _ _ _ h]
theorem cl_optStr (n : Term) (p : String) (v : Option String) (h : p ∉ special) :
    AllT Clean (optStr n p v) := by
  cases v <;> simp [optStr, str, clean_vt _ _ _ h]
theorem cl_bool (n : Term) (p : String) (b : Bool) (h : p ∉ special) : AllT Clean (bool n p b) := by
  simp [bool, clean_vt _ _ _ h]
theorem cl_ordT (n : Term) (v : Nat) : AllT Clean (ordT n v) := by simp [ordT, int]; cln
theorem cl_expr (n : Term) (p v : String) (h : p ∉ special) : AllT Clean (k.expr n p v) := by
  simp [Ctx.expr, clean_vt _ _ _ h]
theorem cl_optExpr (n : Term) (p : String) (v : Option String) (h : p ∉ special) :
    AllT Clean (k.optExpr n p v) := by
  cases v <;> simp [Ctx.optExpr, Ctx.expr, clean_vt _ _ _ h]
theorem cl_node (o : Term) (p f : String) (cs : List String) (h : p ∉ special)
    (hc : ∀ c ∈ cs, c ≠ "StateNode" ∧ c ≠ "TargetRef") : AllT Clean (k.node o p f cs) := by
  simp only [Ctx.node, allT_cons, allT_map]
  exact ⟨clean_vt _ _ _ h, fun c hm => clean_type _ _ (hc c hm).1 (hc c hm).2⟩
theorem cl_extensionFields (n : Term) (x : Extension) : AllT Clean (Ctx.extensionFields n x) := by
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp only [Ctx.extensionFields, str, int, List.cons_append, List.nil_append,
    allT_cons, allT_nil, and_true] <;> refine ⟨?_, ?_, ?_, ?_⟩ <;> cln
theorem cl_extensions (o : Term) (f : String) (xs : List Extension) :
    AllT Clean (k.extensions o f xs) := by
  simp only [Ctx.extensions, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), cl_ordT _ _⟩, cl_extensionFields _ _⟩
theorem cl_tokens (o : Term) (prop pre : String) (h : prop ∉ special) (vs : List String) :
    AllT Clean (k.tokens o prop pre vs) := by
  simp only [Ctx.tokens, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨cl_node k _ _ _ _ h (by simp), cl_ordT _ _⟩, cl_str _ _ _ (by simp [special])⟩
theorem cl_params (o : Term) (f : String) (ps : List Param) : AllT Clean (k.params o f ps) := by
  simp only [Ctx.params, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), cl_ordT _ _⟩,
    cl_str _ _ _ (by simp [special])⟩, cl_optExpr k _ _ _ (by simp [special])⟩,
    cl_optExpr k _ _ _ (by simp [special])⟩
theorem cl_content (o : Term) (f : String) (c : Content) : AllT Clean (k.content o f c) := by
  simp only [Ctx.content, allT_append]
  exact ⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), cl_optExpr k _ _ _ (by simp [special])⟩,
    cl_optStr _ _ _ (by simp [special])⟩
theorem cl_scriptFields (n : Term) (sc : Script) : AllT Clean (k.scriptFields n sc) := by
  simp only [Ctx.scriptFields, allT_append]
  exact ⟨cl_optStr _ _ _ (by simp [special]), cl_optExpr k _ _ _ (by simp [special])⟩

mutual
theorem cl_exec (o : Term) (p f : String) (i : Nat) (h : p ∉ special) : ∀ e, AllT Clean (k.exec o p f i e)
  | .mk kind exts => by
    have := cl_execKind (k.iri f) f kind
    simp only [Ctx.exec, allT_append]
    exact ⟨⟨⟨cl_node k _ _ _ _ h (by cases kind <;> simp [execClass]), cl_ordT _ _⟩, this⟩,
      cl_extensions k _ _ _⟩
theorem cl_execKind (n : Term) (f : String) : ∀ kind, AllT Clean (k.execKind n f kind)
  | .raise _ => cl_str _ _ _ (by simp [special])
  | .send s => by
    simp only [Ctx.execKind, allT_append]
    refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨cl_optStr _ _ _ (by simp [special]),
      cl_optExpr k _ _ _ (by simp [special])⟩, cl_optStr _ _ _ (by simp [special])⟩,
      cl_optExpr k _ _ _ (by simp [special])⟩, cl_optStr _ _ _ (by simp [special])⟩,
      cl_optExpr k _ _ _ (by simp [special])⟩, cl_optStr _ _ _ (by simp [special])⟩,
      cl_optExpr k _ _ _ (by simp [special])⟩, cl_optStr _ _ _ (by simp [special])⟩,
      cl_optExpr k _ _ _ (by simp [special])⟩,
      cl_tokens k _ _ _ (by simp [special]) _⟩, cl_params k _ _ _⟩, ?_⟩
    split
    · exact cl_content k _ _ _
    · simp
  | .cancel .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨cl_optStr _ _ _ (by simp [special]), cl_optExpr k _ _ _ (by simp [special])⟩
  | .log .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨cl_optStr _ _ _ (by simp [special]), cl_optExpr k _ _ _ (by simp [special])⟩
  | .assign .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨⟨cl_expr k _ _ _ (by simp [special]), cl_optExpr k _ _ _ (by simp [special])⟩,
      cl_optStr _ _ _ (by simp [special])⟩
  | .script _ => by simp only [Ctx.execKind]; exact cl_scriptFields k _ _
  | .if_ bs => by simp only [Ctx.execKind]; exact cl_branches n f 0 bs
  | .foreach _ _ _ body => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨⟨⟨cl_expr k _ _ _ (by simp [special]), cl_str _ _ _ (by simp [special])⟩,
      cl_optStr _ _ _ (by simp [special])⟩, cl_block n "action" (f ++ ":a") (by simp [special]) 0 body⟩
  | .extension _ => cl_extensionFields _ _
theorem cl_block (o : Term) (p pre : String) (h : p ∉ special) :
    ∀ (i : Nat) (es : List Exec), AllT Clean (k.block o p pre i es)
  | _, [] => by simp [Ctx.block]
  | i, e :: es => by
    have h1 := cl_exec o p (pre ++ toString i) i h e
    have h2 := cl_block o p pre h (i + 1) es
    simp only [Ctx.block, allT_append]; exact ⟨h1, h2⟩
theorem cl_branches (n : Term) (f : String) : ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
    AllT Clean (k.branches n f j bs)
  | _, [] => by simp [Ctx.branches]
  | j, (c, body) :: bs => by
    have h1 := cl_block (k.iri (f ++ ":b" ++ toString j)) "action" (f ++ ":b" ++ toString j ++ ":a")
      (by simp [special]) 0 body
    have h2 := cl_branches n f (j + 1) bs
    simp only [Ctx.branches, allT_append]
    exact ⟨⟨⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), cl_ordT _ _⟩,
      cl_optExpr k _ _ _ (by simp [special])⟩, h1⟩, h2⟩
end

theorem cl_donedata (o : Term) (f : String) (dd : DoneData) : AllT Clean (k.donedata o f dd) := by
  simp only [Ctx.donedata, allT_append]
  refine ⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), ?_⟩, cl_params k _ _ _⟩
  split
  · exact cl_content k _ _ _
  · simp

theorem cl_invoke (o : Term) (f : String) (i : Nat) (v : Invoke) : AllT Clean (k.invoke o f i v) := by
  simp only [Ctx.invoke, allT_append]
  refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨cl_node k _ _ _ _ (by simp [special]) (by simp), cl_ordT _ _⟩,
    cl_optStr _ _ _ (by simp [special])⟩, cl_optExpr k _ _ _ (by simp [special])⟩,
    cl_optStr _ _ _ (by simp [special])⟩, cl_optExpr k _ _ _ (by simp [special])⟩,
    cl_optStr _ _ _ (by simp [special])⟩, cl_optExpr k _ _ _ (by simp [special])⟩,
    cl_tokens k _ _ _ (by simp [special]) _⟩, cl_bool _ _ _ (by simp [special])⟩,
    cl_params k _ _ _⟩, ?_⟩, cl_block k _ _ _ (by simp [special]) 0 _⟩, cl_extensions k _ _ _⟩
  split
  · exact cl_content k _ _ _
  · simp

/-- A transition is clean but for its target list. -/
theorem cl_transition (tr : Transition) (o : Term) (p f : String) (i : Nat) (h : p ∉ special)
    (src sc : Term) :
    AllT (fun t => Clean t ∨ t ∈ k.targets (k.iri f) "target" f ":tg" tr.targets sc)
      (k.transition tr o p f i src sc) := by
  have mono : ∀ {E : List Triple}, AllT Clean E →
      AllT (fun t => Clean t ∨ t ∈ k.targets (k.iri f) "target" f ":tg" tr.targets sc) E :=
    fun h t ht => .inl (h t ht)
  simp only [Ctx.transition, allT_append, allT_cons, allT_nil, and_true]
  refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨mono (cl_node k _ _ _ _ h (by simp)), mono (cl_ordT _ _)⟩, .inl (by cln)⟩,
    mono (cl_tokens k _ _ _ (by simp [special]) _)⟩, mono (cl_optExpr k _ _ _ (by simp [special]))⟩,
    fun t ht => .inr ht⟩, mono (cl_str _ _ _ (by simp [special]))⟩,
    mono (cl_bool _ _ _ (by simp [special]))⟩, mono (cl_block k _ _ _ (by simp [special]) 0 _)⟩,
    mono (cl_extensions k _ _ _)⟩

/-- A data item is clean but for its `vt:dataId`. -/
theorem cl_dataItem (o : Term) (of : String) (i : Nat) (d : Data) :
    AllT (fun t => Clean t ∨ t = ⟨k.iri (of ++ ":d" ++ toString i), vt "dataId", strLit d.id⟩)
      (k.dataItem o of i d) := by
  have mono : ∀ {E : List Triple}, AllT Clean E →
      AllT (fun t => Clean t ∨ t = ⟨k.iri (of ++ ":d" ++ toString i), vt "dataId", strLit d.id⟩) E :=
    fun h t ht => .inl (h t ht)
  simp only [Ctx.dataItem, allT_append]
  refine ⟨⟨⟨⟨⟨⟨mono (cl_node k _ _ _ _ (by simp [special]) (by simp)), mono (cl_ordT _ _)⟩, ?_⟩,
    mono (cl_optStr _ _ _ (by simp [special]))⟩, mono (cl_optExpr k _ _ _ (by simp [special]))⟩,
    mono (cl_optStr _ _ _ (by simp [special]))⟩, mono (cl_extensions k _ _ _)⟩
  simp [str]

end

/-! ## States and the chart -/

/-- The special triples of a subtree, by origin. -/
def SpecialS (k : Ctx) (P : List (Option State × State)) (t : Triple) : Prop :=
  (∃ q ∈ P, ∃ C ∈ stateCalls k q.1 q.2, t ∈ C.emit k) ∨
  (∃ q ∈ P, t = ⟨k.iri (stateFragment q.2.id), rdfType, vt "StateNode"⟩ ∨
    t = ⟨k.iri (stateFragment q.2.id), vt "id", strLit q.2.id⟩) ∨
  (∃ q ∈ P, ∃ x ∈ stateData q.2, t = ⟨k.iri x.1, vt "dataId", strLit x.2.id⟩)

theorem specialS_mono {k : Ctx} {P P' : List (Option State × State)} (h : ∀ q ∈ P, q ∈ P')
    {t : Triple} : SpecialS k P t → SpecialS k P' t := by
  rintro (⟨q, hq, h1⟩ | ⟨q, hq, h1⟩ | ⟨q, hq, h1⟩)
  · exact .inl ⟨q, h q hq, h1⟩
  · exact .inr (.inl ⟨q, h q hq, h1⟩)
  · exact .inr (.inr ⟨q, h q hq, h1⟩)

section
variable (k : Ctx)

theorem or_clean_vt {R : Prop} {s o : Term} {l : String} (h : l ∉ special) :
    (Clean ⟨s, vt l, o⟩ ∨ R) ↔ True := iff_true_intro (.inl (clean_vt _ _ _ h))
theorem or_clean_type {R : Prop} {s : Term} {c : String} (h1 : c ≠ "StateNode") (h2 : c ≠ "TargetRef") :
    (Clean ⟨s, rdfType, vt c⟩ ∨ R) ↔ True := iff_true_intro (.inl (clean_type _ _ h1 h2))

theorem lift {R : Triple → Prop} {E : List Triple} (h : AllT Clean E) :
    AllT (fun t => Clean t ∨ R t) E := fun t ht => .inl (h t ht)

mutual
theorem cls_state (pp : Option State) (i : Nat) (A' : List Term) :
    ∀ s, AllT (fun t => Clean t ∨ SpecialS k (State.parented pp s) t)
      (k.state i (parT k pp) (k.iri "chart" :: A') s)
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts => by
    have hc := cls_states (some (.mk id idGen el initial initEl children trs onentry onexit data
      invokes dd exts)) 0 (A' ++ [k.iri (stateFragment id)]) children
    have hP : ∀ q ∈ State.parentedList (some (.mk id idGen el initial initEl children trs onentry
        onexit data invokes dd exts)) children,
        q ∈ State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data
          invokes dd exts) := fun q hq => by
      rw [parented_eq]; exact List.mem_cons_of_mem _ hq
    have hc' : AllT (fun t => Clean t ∨ SpecialS k (State.parented pp (.mk id idGen el initial
        initEl children trs onentry onexit data invokes dd exts)) t) _ :=
      fun t ht => (hc t ht).imp_right (specialS_mono hP)
    simp only [parT, State.id, List.cons_append] at hc'
    have hmem : (pp, State.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts) ∈
        State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts) := by
      rw [parented_eq]; exact List.mem_cons_self
    have hcalls : ∀ C ∈ stateCalls k pp (.mk id idGen el initial initEl children trs
        onentry onexit data invokes dd exts), ∀ t ∈ C.emit k, SpecialS k (State.parented pp
        (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts)) t :=
      fun C hC t ht => .inl ⟨_, hmem, C, hC, ht⟩
    have hdata : ∀ x ∈ stateData (.mk id idGen el initial initEl children trs onentry
        onexit data invokes dd exts), SpecialS k (State.parented pp (.mk id idGen el initial
        initEl children trs onentry onexit data invokes dd exts)) ⟨k.iri x.1, vt "dataId", strLit x.2.id⟩ :=
      fun x hx => .inr (.inr ⟨_, hmem, x, hx, rfl⟩)
    have hsn : SpecialS k (State.parented pp (.mk id idGen el initial initEl children trs onentry
        onexit data invokes dd exts)) ⟨k.iri (stateFragment id), rdfType, vt "StateNode"⟩ :=
      .inr (.inl ⟨_, hmem, .inl rfl⟩)
    have hid : SpecialS k (State.parented pp (.mk id idGen el initial initEl children trs onentry
        onexit data invokes dd exts)) ⟨k.iri (stateFragment id), vt "id", strLit id⟩ :=
      .inr (.inl ⟨_, hmem, .inr rfl⟩)
    simp only [stateCalls, State.id, State.initial, State.initialElement, State.transitions,
      List.mem_cons, List.mem_append, List.mem_map, Option.mem_toList, Option.map_eq_some_iff,
      forall_eq_or_imp, TCall.emit] at hcalls
    simp only [stateData, State.data, State.id, List.mem_map] at hdata
    generalize State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data
      invokes dd exts) = P at hc' hcalls hdata hsn hid ⊢
    obtain ⟨hia, hrest⟩ := hcalls
    have htr : ∀ (tr : Transition) (j : Nat), (tr, j) ∈ trs.zipIdx → ∀ (p : String) (hp : p ∉ special) (src : Term),
        AllT (fun t => Clean t ∨ SpecialS k P t)
          (k.transition tr (k.iri (stateFragment id)) p (stateFragment id ++ ":t" ++ toString j) j src
            (scopeT k pp (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts))) := by
      intro tr j hj p hp src t ht
      rcases cl_transition k tr _ p _ j hp src _ t ht with h | h
      · exact .inl h
      · exact .inr (hrest _ (.inr ⟨(tr, j), hj, rfl⟩) t h)
    have hdi : ∀ (d : Data) (j : Nat), (d, j) ∈ data.zipIdx →
        AllT (fun t => Clean t ∨ SpecialS k P t) (k.dataItem (k.iri (stateFragment id)) (stateFragment id) j d) := by
      intro d j hj t ht
      rcases cl_dataItem k _ _ j d t ht with h | rfl
      · exact .inl h
      · exact .inr (hdata _ ⟨(d, j), hj, rfl⟩)
    have hie : ∀ ie : Initial, initEl = some ie →
        AllT (fun t => Clean t ∨ SpecialS k P t)
          (k.node (k.iri (stateFragment id)) "initialElement" (stateFragment id ++ ":init") ["Initial"] ++
            k.transition ie.transition (k.iri (stateFragment id ++ ":init")) "initialTransition"
              (stateFragment id ++ ":init" ++ ":t") 0 (k.iri (stateFragment id)) (k.iri (stateFragment id)) ++
            k.extensions (k.iri (stateFragment id ++ ":init")) (stateFragment id ++ ":init") ie.extensions) := by
      intro ie hie
      simp only [allT_append]
      refine ⟨⟨lift (cl_node k _ _ _ _ (by simp [special]) (by simp)), fun t ht => ?_⟩,
        lift (cl_extensions k _ _ _)⟩
      rcases cl_transition k _ _ _ _ 0 (by simp [special]) _ _ t ht with h | h
      · exact .inl h
      · exact .inr (hrest _ (.inl ⟨ie, hie, rfl⟩) t h)
    have hel : ∀ x, x ∈ [elementClass el] → x ≠ "StateNode" ∧ x ≠ "TargetRef" := by
      intro x hx; simp at hx; subst hx; cases el <;> simp [elementClass]
    have hself : ∀ (o : Term) (l : String), l ∉ special →
        Clean ⟨k.iri (stateFragment id), vt l, o⟩ := fun o l h => clean_vt _ _ _ h
    have hia' : AllT (fun t => Clean t ∨ SpecialS k P t) (k.targets (k.iri (stateFragment id)) "initial"
        (stateFragment id ++ ":ia") "" initial (k.iri (stateFragment id))) := fun t ht => .inr (hia t ht)
    have hblk : ∀ (g : String) (b : List Exec), AllT (fun t => Clean t ∨ SpecialS k P t)
        (k.block (k.iri g) "action" (g ++ ":a") 0 b) := fun g b => lift (cl_block k _ _ _ (by simp [special]) 0 b)
    have hinv : ∀ (v : Invoke) (j : Nat), AllT (fun t => Clean t ∨ SpecialS k P t)
        (k.invoke (k.iri (stateFragment id)) (stateFragment id ++ ":inv" ++ toString j) j v) :=
      fun v j => lift (cl_invoke k _ _ _ _)
    have hdd : ∀ d : DoneData, AllT (fun t => Clean t ∨ SpecialS k P t)
        (k.donedata (k.iri (stateFragment id)) (stateFragment id ++ ":dd") d) := fun d => lift (cl_donedata k _ _ _)
    have hext : AllT (fun t => Clean t ∨ SpecialS k P t)
        (k.extensions (k.iri (stateFragment id)) (stateFragment id) exts) := lift (cl_extensions k _ _ _)
    have htr' := fun tr j hj => htr tr j hj "transition" (by simp [special]) (k.iri (stateFragment id))
    rcases initEl with _ | ie <;> rcases dd with _ | d <;> rcases el with _ | _ | _ | h <;>
      simp only [Ctx.state, Ctx.node, allT_append, allT_cons, allT_nil, allT_map, allT_flatMap,
        Prod.forall, str, bool, ordT, int, hc', List.headD_cons, scopeT, State.element] at hie htr' ⊢ <;>
      simp [or_clean_vt, or_clean_type, special, hsn, hid, hia', hblk, hinv, hdd, hext, htr', hdi, hie,
        elementClass, hc'] <;>
      exact ⟨⟨fun a b hab => htr' a b hab, fun a b hab => hdi a b hab⟩, fun a b _ => hinv a b⟩

theorem cls_states (pp : Option State) (i : Nat) (A' : List Term) :
    ∀ cs, AllT (fun t => Clean t ∨ SpecialS k (State.parentedList pp cs) t)
      (k.states i (parT k pp) (k.iri "chart" :: A') cs)
  | [] => by simp [Ctx.states]
  | c :: cs => by
    have h1 := cls_state pp i A' c
    have h2 := cls_states pp (i + 1) A' cs
    simp only [Ctx.states, allT_append]
    refine ⟨fun t ht => (h1 t ht).imp_right (specialS_mono fun q hq => ?_),
      fun t ht => (h2 t ht).imp_right (specialS_mono fun q hq => ?_)⟩
    · rw [State.parentedList]; exact List.mem_append_left _ hq
    · rw [State.parentedList]; exact List.mem_append_right _ hq
end

end

theorem mem_parented_allStates {c : Chart} {q : Option State × State} (h : q ∈ c.parented) :
    q.2 ∈ c.allStates := by
  have : q.2 ∈ c.parented.map Prod.snd := List.mem_map_of_mem h
  rwa [Chart.parented, parentedList_snd] at this

/-- **Where special triples come from.** -/
theorem cls_project (c : Chart) (base : String) :
    ∀ t ∈ project c base, Clean t ∨ Special (ctxOf c base) c t := by
  intro t ht
  rw [project_eq] at ht
  simp only [List.mem_append] at ht
  rcases ht with (((((ht | ht) | ht) | ht) | ht) | ht) | ht
  · left
    simp only [chartSelf, List.mem_append, List.mem_singleton, mem_optStr, mem_str, mem_bool] at ht
    rcases ht with ((((h | ⟨_, _, h⟩) | h) | h) | h) | h <;> subst h <;> first
      | exact clean_type _ _ (by decide) (by decide)
      | exact clean_vt _ _ _ (by simp [special])
  · right; left
    simp only [chartRefs, ← targets_eq] at ht
    exact ⟨_, List.mem_cons_self, ht⟩
  · obtain ⟨d, hd, ht⟩ := List.mem_flatMap.1 ht
    simp only [List.mem_map] at hd
    obtain ⟨⟨x, j⟩, hx, rfl⟩ := hd
    rcases cl_dataItem _ _ "chart" j x t ht with h | rfl
    · exact .inl h
    · exact .inr (.inr (.inr (.inl ⟨("chart:d" ++ toString j, x),
        List.mem_append_left _ (List.mem_map.2 ⟨(x, j), hx, rfl⟩), rfl⟩)))
  · left
    cases hs : c.script with
    | none => simp [hs, scriptKids] at ht
    | some sc =>
      simp only [hs, scriptKids, List.flatMap_cons, List.flatMap_nil, List.append_nil,
        List.mem_append] at ht
      rcases ht with ht | ht
      · exact cl_node _ _ _ _ _ (by simp [special]) (by simp) t ht
      · exact cl_scriptFields _ _ _ t ht
  · have := cls_states (ctxOf c base) none 0 [] c.states t (by simpa [parT] using ht)
    rcases this with h | (⟨q, hq, C, hC, h⟩ | ⟨q, hq, h⟩ | ⟨q, hq, x, hx, h⟩)
    · exact .inl h
    · exact .inr (.inl ⟨C, List.mem_cons_of_mem _ (List.mem_flatMap.2 ⟨q, hq, hC⟩), h⟩)
    · exact .inr (.inr (.inl ⟨q.2, mem_parented_allStates hq, h⟩))
    · exact .inr (.inr (.inr (.inl ⟨x, List.mem_append_right _
        (List.mem_flatMap.2 ⟨q.2, mem_parented_allStates hq, hx⟩), h⟩)))
  · left
    simp only [← extensions_eq] at ht
    exact cl_extensions _ _ _ _ t ht
  · exact .inr (.inr (.inr (.inr ht)))

/-! ## The target lists are part of the projection -/

section
variable (k : Ctx)

theorem mem_state_of_kid (i : Nat) (parent : Term) (A : List Term) (Y : State) {d : String × List Triple}
    (hd : d ∈ kidsOf k parent A Y) {u : Triple} (hu : u ∈ d.2) : u ∈ k.state i parent A Y := by
  rw [state_eq']
  exact List.mem_append_left _ (List.mem_append_right _ (List.mem_flatMap.2 ⟨d, hd, hu⟩))

theorem stateCalls_sub (pp : Option State) (i : Nat) (A' : List Term) (Y : State) :
    ∀ C ∈ stateCalls k pp Y, ∀ u ∈ C.emit k, u ∈ k.state i (parT k pp) (k.iri "chart" :: A') Y := by
  intro C hC u hu
  simp only [stateCalls, List.mem_cons, List.mem_append, Option.mem_toList, Option.mem_def,
    Option.map_eq_some_iff, List.mem_map] at hC
  rcases hC with rfl | (⟨ie, hie, rfl⟩ | ⟨⟨tr, j⟩, hj, rfl⟩)
  · simp only [TCall.emit, targets_eq] at hu
    obtain ⟨d, hd, hu⟩ := List.mem_flatMap.1 hu
    exact mem_state_of_kid k i _ _ Y (d := d) (by simp [kidsOf, stateKids, hd]) hu
  · simp only [TCall.emit] at hu
    refine mem_state_of_kid k i _ _ Y (d := (stateFragment Y.id ++ ":init",
      initEmission k (k.iri (stateFragment Y.id)) (stateFragment Y.id ++ ":init") ie))
      (by simp [kidsOf, stateKids, initKidsOpt, hie]) ?_
    simp only [initEmission, Ctx.transition, List.mem_append, hu, true_or, or_true]
  · simp only [TCall.emit] at hu
    refine mem_state_of_kid k i _ _ Y (d := transK k (stateFragment Y.id)
      (scopeOf (parT k pp) (k.iri "chart" :: A') (k.iri (stateFragment Y.id)) Y.element) (tr, j))
      (by simp only [kidsOf, stateKids, List.mem_append]; exact .inl (.inl (.inl (.inl (.inl (.inl
        (.inr (List.mem_map.2 ⟨(tr, j), hj, rfl⟩)))))))) ?_
    have hsc : scopeOf (parT k pp) (k.iri "chart" :: A') (k.iri (stateFragment Y.id)) Y.element =
        scopeT k pp Y := by
      rcases Y with ⟨_, _, el, _⟩; cases el <;> rfl
    simp only [transK, hsc, Ctx.transition, List.mem_append, hu, true_or, or_true]

mutual
theorem parented_sub (pp : Option State) (i : Nat) (A' : List Term) :
    ∀ s, ∀ q ∈ State.parented pp s, ∃ i' A'', ∀ u ∈ k.state i' (parT k q.1) (k.iri "chart" :: A'') q.2,
      u ∈ k.state i (parT k pp) (k.iri "chart" :: A') s
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts, q, hq => by
    rw [parented_eq] at hq
    rcases List.mem_cons.1 hq with rfl | hq
    · exact ⟨i, A', fun u hu => hu⟩
    · obtain ⟨i', A'', h⟩ := parentedList_sub (some (.mk id idGen el initial initEl children trs
        onentry onexit data invokes dd exts)) 0 (A' ++ [k.iri (stateFragment id)]) children q hq
      refine ⟨i', A'', fun u hu => ?_⟩
      have := h u hu
      simp only [parT, State.id, List.cons_append] at this
      rw [state_eq']
      exact List.mem_append_right _ this
theorem parentedList_sub (pp : Option State) (i : Nat) (A' : List Term) :
    ∀ cs, ∀ q ∈ State.parentedList pp cs, ∃ i' A'', ∀ u ∈ k.state i' (parT k q.1) (k.iri "chart" :: A'') q.2,
      u ∈ k.states i (parT k pp) (k.iri "chart" :: A') cs
  | [], q, hq => by simp [State.parentedList] at hq
  | c :: cs, q, hq => by
    rw [State.parentedList, List.mem_append] at hq
    rcases hq with hq | hq
    · obtain ⟨i', A'', h⟩ := parented_sub pp i A' c q hq
      exact ⟨i', A'', fun u hu => by rw [Ctx.states]; exact List.mem_append_left _ (h u hu)⟩
    · obtain ⟨i', A'', h⟩ := parentedList_sub pp (i + 1) A' cs q hq
      exact ⟨i', A'', fun u hu => by rw [Ctx.states]; exact List.mem_append_right _ (h u hu)⟩
end

end

/-- Every target list's emission is part of the projection. -/
theorem tcalls_sub (c : Chart) (base : String) :
    ∀ C ∈ tcalls (ctxOf c base) c, ∀ u ∈ C.emit (ctxOf c base), u ∈ project c base := by
  intro C hC u hu
  rw [project_eq]
  simp only [tcalls, List.mem_cons, List.mem_flatMap] at hC
  rcases hC with rfl | ⟨q, hq, hC⟩
  · simp only [TCall.emit] at hu
    simp only [List.mem_append, chartRefs, ← targets_eq, hu, true_or, or_true]
  · obtain ⟨i', A'', h⟩ := parentedList_sub (ctxOf c base) none 0 [] c.states q hq
    have := h u (stateCalls_sub _ _ _ _ _ C hC u hu)
    simp only [parT] at this
    simp only [List.mem_append, this, true_or, or_true]

/-! ## Consequences -/

section
variable (k : Ctx)

/-- The triples of a target list, by predicate. -/
theorem targets_cases {o : Term} {p l r : String} {ids : List String} {sc : Term} :
    ∀ u ∈ k.targets o p l r ids sc,
      (u.p = vt p ∧ (⟨u.o, rdfType, vt "TargetRef"⟩ : Triple) ∈ k.targets o p l r ids sc) ∨
      (u.p = rdfType ∧ u.o = vt "TargetRef") ∨ u.p = vt "order" ∨ u.p = vt "state" ∨
      u.p = vt "scope" ∨ u.p = vt "choice" := by
  intro u hu
  have hu' := hu
  simp only [Ctx.targets, List.mem_flatMap] at hu'
  obtain ⟨x, hx, h⟩ := hu'
  simp only [Ctx.node, ordT, int, List.cons_append, List.singleton_append, List.map_cons,
    List.map_nil, List.mem_cons, List.mem_append, List.nil_append] at h
  rcases h with h | h | h | h | h | h
  · subst h; refine .inl ⟨rfl, ?_⟩
    simp only [Ctx.targets, List.mem_flatMap]
    exact ⟨x, hx, by simp [Ctx.node]⟩
  · subst h; exact .inr (.inl ⟨rfl, rfl⟩)
  · subst h; exact .inr (.inr (.inl rfl))
  · subst h; exact .inr (.inr (.inr (.inl rfl)))
  · subst h; exact .inr (.inr (.inr (.inr (.inl rfl))))
  · refine .inr (.inr (.inr (.inr (.inr ?_))))
    split at h
    · simp at h
    · simp only [List.mem_flatMap] at h
      obtain ⟨_, _, h⟩ := h
      simp at h; rw [h]

end

theorem tcalls_prop {k : Ctx} {c : Chart} {C : TCall} (h : C ∈ tcalls k c) :
    C.prop = "initial" ∨ C.prop = "target" := by
  simp only [tcalls, List.mem_cons, List.mem_flatMap] at h
  rcases h with rfl | ⟨q, _, h⟩
  · exact .inl rfl
  · simp only [stateCalls, List.mem_cons, List.mem_append, Option.mem_toList, Option.mem_def,
      Option.map_eq_some_iff, List.mem_map] at h
    rcases h with rfl | ⟨_, _, rfl⟩ | ⟨_, _, rfl⟩
    · exact .inl rfl
    · exact .inr rfl
    · exact .inr rfl

section
variable (c : Chart) (base : String)

theorem hd_p' {t : Triple} (h : t ∈ historyDefaultsList (ctxOf c base) none c.states) :
    t.p = vt "historyDefault" := hd_ps _ _ _ t h

/-- The objects of `vt:initial` are typed `vt:TargetRef`. -/
theorem initial_typed {s o : Term} (h : (⟨s, vt "initial", o⟩ : Triple) ∈ project c base) :
    (⟨o, rdfType, vt "TargetRef"⟩ : Triple) ∈ project c base := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, _, hY | hY⟩ | ⟨q, _, hq⟩ | hh)
  · exact absurd rfl (hc.1 "initial" (by simp [special]))
  · rcases targets_cases _ _ hu with ⟨_, ht⟩ | ⟨h1, _⟩ | h1 | h1 | h1 | h1
    · exact tcalls_sub c base C hC _ ht
    all_goals simp at h1
  all_goals first
    | (simp at hY) | (simp at hq)
    | (have := hd_p' c base hh; simp at this)

/-- Only state nodes are typed `vt:StateNode`. -/
theorem stateNode_typed {x : Term} (h : (⟨x, rdfType, vt "StateNode"⟩ : Triple) ∈ project c base) :
    ∃ Y ∈ c.allStates, x = (ctxOf c base).iri (stateFragment Y.id) := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, hY, hY' | hY'⟩ | ⟨q, _, hq⟩ | hh)
  · exact absurd rfl (hc.2 rfl).1
  · rcases targets_cases _ _ hu with ⟨h1, _⟩ | ⟨_, h1⟩ | h1 | h1 | h1 | h1 <;> simp at h1
  · simp only [Triple.mk.injEq] at hY'; exact ⟨Y, hY, hY'.1⟩
  · simp at hY'
  · simp at hq
  · have := hd_p' c base hh; simp at this

/-- The `vt:id` triples are those of the states. -/
theorem id_triple {s o : Term} (h : (⟨s, vt "id", o⟩ : Triple) ∈ project c base) :
    ∃ Y ∈ c.allStates, s = (ctxOf c base).iri (stateFragment Y.id) ∧ o = strLit Y.id := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, hY, hY' | hY'⟩ | ⟨q, _, hq⟩ | hh)
  · exact absurd rfl (hc.1 "id" (by simp [special]))
  · rcases targets_cases _ _ hu with ⟨h1, _⟩ | ⟨h1, _⟩ | h1 | h1 | h1 | h1 <;> simp at h1
    rcases tcalls_prop hC with h2 | h2 <;> simp [TCall.emit, h2] at h1
  · simp at hY'
  · simp only [Triple.mk.injEq] at hY'; exact ⟨Y, hY, hY'.1, hY'.2.2⟩
  · simp at hq
  · have := hd_p' c base hh; simp at this

/-- The `vt:dataId` triples are those of the data items. -/
theorem dataId_triple {s o : Term} (h : (⟨s, vt "dataId", o⟩ : Triple) ∈ project c base) :
    ∃ q ∈ dataNodes c, s = (ctxOf c base).iri q.1 ∧ o = strLit q.2.id := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, hY, hY' | hY'⟩ | ⟨q, hq, hq'⟩ | hh)
  · exact absurd rfl (hc.1 "dataId" (by simp [special]))
  · rcases targets_cases _ _ hu with ⟨h1, _⟩ | ⟨h1, _⟩ | h1 | h1 | h1 | h1 <;> simp at h1
    rcases tcalls_prop hC with h2 | h2 <;> simp [TCall.emit, h2] at h1
  · simp at hY'
  · simp at hY'
  · simp only [Triple.mk.injEq] at hq'; exact ⟨q, hq, hq'.1, hq'.2.2⟩
  · have := hd_p' c base hh; simp at this

/-- The `vt:choice` triples are those of the target lists. -/
theorem choice_triple {s o : Term} (h : (⟨s, vt "choice", o⟩ : Triple) ∈ project c base) :
    ∃ C ∈ tcalls (ctxOf c base) c, (⟨s, vt "choice", o⟩ : Triple) ∈ C.emit (ctxOf c base) := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, hY, hY' | hY'⟩ | ⟨q, hq, hq'⟩ | hh)
  · exact absurd rfl (hc.1 "choice" (by simp [special]))
  · exact ⟨C, hC, hu⟩
  · simp at hY'
  · simp at hY'
  · simp at hq'
  · have := hd_p' c base hh; simp at this

/-- The `vt:historyDefault` triples are those of `historyDefaultsList`. -/
theorem hd_triple {s o : Term} (h : (⟨s, vt "historyDefault", o⟩ : Triple) ∈ project c base) :
    (⟨s, vt "historyDefault", o⟩ : Triple) ∈ historyDefaultsList (ctxOf c base) none c.states := by
  rcases cls_project c base _ h with hc | (⟨C, hC, hu⟩ | ⟨Y, hY, hY' | hY'⟩ | ⟨q, hq, hq'⟩ | hh)
  · exact absurd rfl (hc.1 "historyDefault" (by simp [special]))
  · rcases targets_cases _ _ hu with ⟨h1, _⟩ | ⟨h1, _⟩ | h1 | h1 | h1 | h1 <;> simp at h1
    rcases tcalls_prop hC with h2 | h2 <;> simp [TCall.emit, h2] at h1
  · simp at hY'
  · simp at hY'
  · simp at hq'
  · exact hh

end

end Vertep.Rdf.Equiv
