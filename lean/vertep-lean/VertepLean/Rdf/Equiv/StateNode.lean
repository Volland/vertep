import VertepLean.Rdf.Equiv.InitNodes
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes at a state node

`state_view` lists the objects of a state node for every predicate other than the derived ones;
`good_state_node` gives the node shapes at a state node in terms of the state's fields, its
children's elements, and target conditions `TR` on its `initial` attribute.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

def histStr : HistoryType → String
  | .shallow => "shallow"
  | .deep => "deep"

/-- The objects of a state node. -/
theorem state_view {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (hcap : Cap k ts (U (stateFragment Y.id)) (core k i pt A Y))
    (hne : pt ≠ k.iri (stateFragment Y.id)) (l : String) (o : Term) (h1 : l ≠ "historyDefault")
    (h2 : l ≠ "choice") :
    (⟨k.iri (stateFragment Y.id), vt l, o⟩ : Triple) ∈ ts ↔
      (l = "historyType" ∧ ∃ h, Y.element = .history h ∧ o = strLit (histStr h)) ∨
      (l = "id" ∧ o = strLit Y.id) ∨ (l = "idGenerated" ∧ o = boolLit Y.idGenerated) ∨
      (l = "order" ∧ o = intLit i) ∨ (l = "ancestor" ∧ o ∈ A) ∨
      (l = "initial" ∧ o ∈ (refKids k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") ""
        Y.initial (k.iri (stateFragment Y.id))).map (fun d => k.iri d.1)) ∨
      (l = "initialElement" ∧ o ∈ (initKidsOpt k (stateFragment Y.id) Y.initialElement).map (fun d => k.iri d.1)) ∨
      (l = "transition" ∧ o ∈ (Y.transitions.zipIdx.map (transK k (stateFragment Y.id)
        (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element))).map (fun d => k.iri d.1)) ∨
      (l = "onentry" ∧ o ∈ (Y.onentry.zipIdx.map (hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry")).map
        (fun d => k.iri d.1)) ∨
      (l = "onexit" ∧ o ∈ (Y.onexit.zipIdx.map (hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit")).map
        (fun d => k.iri d.1)) ∨
      (l = "data" ∧ o ∈ (Y.data.zipIdx.map (dataK k (stateFragment Y.id))).map (fun d => k.iri d.1)) ∨
      (l = "invoke" ∧ o ∈ (Y.invokes.zipIdx.map (invK k (stateFragment Y.id))).map (fun d => k.iri d.1)) ∨
      (l = "donedata" ∧ o ∈ (ddKidsOpt k (stateFragment Y.id) Y.donedata).map (fun d => k.iri d.1)) ∨
      (l = "extension" ∧ o ∈ (extKids k (stateFragment Y.id) Y.extensions).map (fun d => k.iri d.1)) ∨
      (l = "child" ∧ ∃ c ∈ Y.children, o = k.iri (stateFragment c.id)) := by
  have hanc : (∃ a, a ∈ A ∧ "ancestor" = l ∧ a = o) ↔ (l = "ancestor" ∧ o ∈ A) :=
    ⟨fun ⟨a, ha, h1, h2⟩ => ⟨h1.symm, h2 ▸ ha⟩, fun ⟨h1, h2⟩ => ⟨o, h2, h1.symm, rfl⟩⟩
  have hch : (∃ a, a ∈ Y.children ∧ "child" = l ∧ k.iri (stateFragment a.id) = o) ↔
      (l = "child" ∧ ∃ c ∈ Y.children, o = k.iri (stateFragment c.id)) :=
    ⟨fun ⟨a, ha, h1, h2⟩ => ⟨h1.symm, a, ha, h2.symm⟩, fun ⟨h1, a, ha, h2⟩ => ⟨a, ha, h1.symm, h2.symm⟩⟩
  rw [obj_view' hcap l o h1 h2]
  simp only [core, kidsOf, stateKids, List.flatMap_append, List.mem_append]
  simp only [self_refS, self_initS, self_transS,
       self_hookS _ _ ":en" "onentry" "OnEntry" (by simp) (by simp),
       self_hookS _ _ ":ex" "onexit" "OnExit" (by simp) (by simp), self_dataS, self_invS,
       self_ddS, self_extS, rel_mk, rel_type, ne_eq, String.reduceEq, not_false_eq_true,
       and_self, and_true, true_and, h1, h2, vt_inj]
  rcases hel : Y.element with _ | _ | _ | h
  all_goals
    simp only [stateSelf, childLinks, histT, hel, List.mem_append, mem_node, mem_str, mem_bool, mem_ordT,
      List.mem_map, List.mem_cons, List.not_mem_nil, or_false, Triple.mk.injEq, Ne.symm hne, false_and,
      false_or, vt_ne_rdfType, rdfType_ne_vt, vt_inj, true_and, and_true, histStr, reduceCtorEq, exists_false,
      exists_const, and_false]
  all_goals simp only [hanc, hch, or_assoc, List.mem_map]
  all_goals (cases h <;> simp only [histStr, exists_eq_left', StateElement.history.injEq, exists_eq_left,
    hanc, hch, or_assoc, true_and] <;> simp [histStr])

/-- The capture context of a state node. -/
structure StCtx (k : Ctx) (ts : List Triple) (i : Nat) (pt : Term) (A : List Term) (Y : State) : Prop where
  cap : Cap k ts (U (stateFragment Y.id)) (core k i pt A Y)
  own : Above (k := k) pt (stateFragment Y.id)
  cont : Contained' k ts (k.iri (stateFragment Y.id)) pt

theorem StCtx.cx {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y) :
    Ctxt k ts pt "child" (stateFragment Y.id) (core k i pt A Y) (kidsOf k pt A Y) :=
  ⟨h.cap, decomp_core k i pt A Y, h.own⟩

theorem StCtx.ty {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y) (l : String) :
    (⟨k.iri (stateFragment Y.id), rdfType, vt l⟩ : Triple) ∈ ts ↔ l = "StateNode" ∨ l = elementClass Y.element := by
  have hne := above_ne h.own
  rw [h.cap.lookup (U_refl _) (by simp)]
  simp only [core, kidsOf, stateKids, List.flatMap_append, List.mem_append]
  simp only [self_refS, self_initS, self_transS,
       self_hookS _ _ ":en" "onentry" "OnEntry" (by simp) (by simp),
       self_hookS _ _ ":ex" "onexit" "OnExit" (by simp) (by simp), self_dataS, self_invS,
       self_ddS, self_extS, rel_type, rdfType_ne_vt, false_and, or_false]
  rcases hel : Y.element with _ | _ | _ | h <;>
  simp [stateSelf, childLinks, histT, hel, Ne.symm hne, elementClass, eq_comm]

/-- The kids of a state node in one list. -/
theorem StCtx.kidsub {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y)
    {d : String × List Triple} (hd : d ∈ kidsOf k pt A Y) {t : Triple} (ht : t ∈ d.2) : t ∈ ts :=
  kid_sub h.cx hd ht

theorem hcl_of (env : Env k ts P) {x : Term} {l c : String}
    (h : ∀ o, (⟨x, vt l, o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt c⟩ : Triple) ∈ ts) :
    CompHolds (Graph.ofList ts) none (clsP l c) x := by
  simp only [clsP, pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
    compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList',
    instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
  exact h

/-- Objects of a state node along a predicate of one kid list are typed. -/
theorem st_objV {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y)
    {p c : String} {D' : List (String × List Triple)} (hD' : ∀ d ∈ D', d ∈ kidsOf k pt A Y)
    (hT : ∀ d ∈ D', (⟨k.iri d.1, rdfType, vt c⟩ : Triple) ∈ d.2)
    (hmem : ∀ o, (⟨k.iri (stateFragment Y.id), vt p, o⟩ : Triple) ∈ ts → o ∈ D'.map (fun d => k.iri d.1)) :
    ∀ o, (⟨k.iri (stateFragment Y.id), vt p, o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt c⟩ : Triple) ∈ ts := by
  intro o ho
  obtain ⟨d, hd, rfl⟩ := List.mem_map.1 (hmem o ho)
  exact h.kidsub (hD' d hd) (hT d hd)

theorem kidsOf_mem {pt : Term} {A : List Term} {Y : State} {d : String × List Triple} :
    d ∈ kidsOf k pt A Y ↔
      d ∈ refKids k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") "" Y.initial
        (k.iri (stateFragment Y.id)) ∨
      d ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement ∨
      d ∈ Y.transitions.zipIdx.map (transK k (stateFragment Y.id)
        (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) ∨
      d ∈ Y.onentry.zipIdx.map (hookK k (stateFragment Y.id) ":en" "onentry" "OnEntry") ∨
      d ∈ Y.onexit.zipIdx.map (hookK k (stateFragment Y.id) ":ex" "onexit" "OnExit") ∨
      d ∈ Y.data.zipIdx.map (dataK k (stateFragment Y.id)) ∨
      d ∈ Y.invokes.zipIdx.map (invK k (stateFragment Y.id)) ∨
      d ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata ∨
      d ∈ extKids k (stateFragment Y.id) Y.extensions := by
  simp only [kidsOf, stateKids, List.mem_append, or_assoc]

section
variable (k : Ctx)
theorem ty_transK (frag : String) (sc : Term) (trs : List Transition) :
    ∀ d ∈ trs.zipIdx.map (transK k frag sc), (⟨k.iri d.1, rdfType, vt "Transition"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [List.mem_map] at hd; obtain ⟨_, _, rfl⟩ := hd; simp [transK, Ctx.transition, Ctx.node]
theorem ty_hookK (frag seg prop cls : String) (bs : List Block) :
    ∀ d ∈ bs.zipIdx.map (hookK k frag seg prop cls), (⟨k.iri d.1, rdfType, vt cls⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [List.mem_map] at hd; obtain ⟨_, _, rfl⟩ := hd; simp [hookK, hookEmission, Ctx.node]
theorem ty_dataK (frag : String) (ds : List Data) :
    ∀ d ∈ ds.zipIdx.map (dataK k frag), (⟨k.iri d.1, rdfType, vt "DataItem"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [List.mem_map] at hd; obtain ⟨_, _, rfl⟩ := hd; simp [dataK, Ctx.dataItem, Ctx.node]
theorem ty_invK (frag : String) (vs : List Invoke) :
    ∀ d ∈ vs.zipIdx.map (invK k frag), (⟨k.iri d.1, rdfType, vt "Invoke"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [List.mem_map] at hd; obtain ⟨_, _, rfl⟩ := hd; simp [invK, Ctx.invoke, Ctx.node]
theorem ty_initKidsOpt (frag : String) (ie : Option Initial) :
    ∀ d ∈ initKidsOpt k frag ie, (⟨k.iri d.1, rdfType, vt "Initial"⟩ : Triple) ∈ d.2 := by
  intro d hd; cases ie <;> simp [initKidsOpt] at hd; subst hd; simp [initEmission, Ctx.node]
theorem ty_ddKidsOpt (frag : String) (dd : Option DoneData) :
    ∀ d ∈ ddKidsOpt k frag dd, (⟨k.iri d.1, rdfType, vt "DoneData"⟩ : Triple) ∈ d.2 := by
  intro d hd; cases dd <;> simp [ddKidsOpt] at hd; subst hd; simp [Ctx.donedata, Ctx.node]
end

@[simp] theorem refKids_eq_nil {o : Term} {p L st : String} {sc : Term} {l : List String} :
    refKids k o p L st l sc = [] ↔ l = [] := by cases l <;> simp [refKids]
@[simp] theorem initKidsOpt_eq_nil {f : String} {ie : Option Initial} :
    initKidsOpt k f ie = [] ↔ ie = none := by cases ie <;> simp [initKidsOpt]
@[simp] theorem ddKidsOpt_eq_nil {f : String} {dd : Option DoneData} :
    ddKidsOpt k f dd = [] ↔ dd = none := by cases dd <;> simp [ddKidsOpt]

theorem refKids_none {o : Term} {p L st : String} {sc : Term} {l : List String} :
    (∀ (y : Term) (x : String) (j : Nat), (x, j) ∈ l.zipIdx →
      ¬ k.iri (refK k o p L st sc (x, j)).fst = y) ↔ l = [] := by
  cases l with
  | nil => simp
  | cons x l =>
    simp only [reduceCtorEq, iff_false]
    intro hh
    exact hh _ x 0 (by simp) rfl

/-- The target condition: the target names a known state whose node lists the scope as an
ancestor. -/
def TR (k : Ctx) (ts : List Triple) (Kn : String → Prop) (x : String) (sc : Term) : Prop :=
  Kn x ∧ (⟨k.iri (stateFragment x), vt "ancestor", sc⟩ : Triple) ∈ ts

set_option maxHeartbeats 2000000 in
theorem snS_ok (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x)
    (hA : ∀ v, Plus (ValueNode (Graph.ofList ts) (I "child")) (k.iri (stateFragment Y.id)) v ↔ v ∈ A)
    (hchT : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts) :
    Conforms (Graph.ofList ts) stateNodeS (k.iri (stateFragment Y.id)) ↔
      ¬(Y.initial ≠ [] ∧ Y.initialElement.isSome) ∧ Y.id ≠ "" ∧
      ∀ x ∈ Y.initial, TR k ts Kn x (k.iri (stateFragment Y.id)) := by
  have hv := state_view h.cap (above_ne h.own)
  have hmem : ∀ (p : String) (D' : List (String × List Triple)) , p ≠ "historyDefault" → p ≠ "choice" →
      (∀ o, (⟨k.iri (stateFragment Y.id), vt p, o⟩ : Triple) ∈ ts → o ∈ D'.map (fun d => k.iri d.1)) →
      (∀ o, (⟨k.iri (stateFragment Y.id), vt p, o⟩ : Triple) ∈ ts → o ∈ D'.map (fun d => k.iri d.1)) :=
    fun _ _ _ _ x => x
  have c1 := st_objV h (p := "transition") (c := "Transition") (fun d hd => kidsOf_mem.2 (.inr (.inr (.inl hd))))
    (ty_transK k _ _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c2 := st_objV h (p := "onentry") (c := "OnEntry") (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inl hd)))))
    (ty_hookK k _ _ _ _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c3 := st_objV h (p := "onexit") (c := "OnExit") (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inl hd))))))
    (ty_hookK k _ _ _ _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c4 := st_objV h (p := "data") (c := "DataItem")
    (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))
    (ty_dataK k _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c5 := st_objV h (p := "invoke") (c := "Invoke")
    (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd))))))))
    (ty_invK k _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c6 := st_objV h (p := "extension") (c := "ExtensionNode")
    (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inr hd)))))))))
    (ty_extKids k _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have c7 : ∀ o, (⟨k.iri (stateFragment Y.id), vt "child", o⟩ : Triple) ∈ ts →
      (⟨o, rdfType, vt "StateNode"⟩ : Triple) ∈ ts := by
    intro o ho; rw [hv _ _ (by decide) (by decide)] at ho; simp at ho
    obtain ⟨c, hc, rfl⟩ := ho; exact hchT c hc
  have hIE := st_objV h (p := "initialElement") (c := "Initial") (fun d hd => kidsOf_mem.2 (.inr (.inl hd)))
    (ty_initKidsOpt k _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  have hDD := st_objV h (p := "donedata") (c := "DoneData")
    (fun d hd => kidsOf_mem.2 (.inr (.inr (.inr (.inr (.inr (.inr (.inr (.inl hd)))))))))
    (ty_ddKidsOpt k _ _) (fun o ho => by rw [hv _ _ (by decide) (by decide)] at ho; simpa using ho)
  -- initial references
  have hT : (∀ v, (⟨k.iri (stateFragment Y.id), vt "initial", v⟩ : Triple) ∈ ts →
      Conforms (Graph.ofList ts) targetRefS v) ↔ ∀ x ∈ Y.initial, TR k ts Kn x (k.iri (stateFragment Y.id)) := by
    have key : ∀ p ∈ Y.initial.zipIdx, Conforms (Graph.ofList ts) targetRefS
        (k.iri (refK k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") "" (k.iri (stateFragment Y.id)) p).1) ↔
        TR k ts Kn p.1 (k.iri (stateFragment Y.id)) := by
      intro p hp
      have hd : refK k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") "" (k.iri (stateFragment Y.id)) p ∈
          kidsOf k pt A Y := kidsOf_mem.2 (.inl (List.mem_map.2 ⟨p, hp, rfl⟩))
      rcases p with ⟨x, j⟩
      exact targetRef_ok env (h.cx.kid hd) (above_of_ch (h.cx.dec.ch _ hd)) Kn (hSN x)
    simp only [hv _ _ (by decide) (by decide : "initial" ≠ "choice"), refKids, List.map_map, List.mem_map,
      Function.comp_def, String.reduceEq, false_and, false_or, or_false, true_and, forall_exists_index,
      and_imp, forall_apply_eq_imp_iff₂]
    constructor
    · intro hh x hx
      obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hx
      have hm : (Y.initial[i], i) ∈ Y.initial.zipIdx := List.mem_zipIdx_iff_getElem?.2 (by simp)
      exact (key _ hm).1 (hh _ hm)
    · intro hh p hp
      exact (key p hp).2 (hh p.1 (fst_mem_zipIdx hp))
  have hup : ∀ v, ValueNode (Graph.ofList ts) (I "child") (k.iri (stateFragment Y.id)) v ↔ v = pt := by
    intro v
    simp only [ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred, iri_vi, mem_ofList']
    constructor
    · intro hv'; exact (h.cont.conts v).1 ⟨"child", by simp [containers], hv'⟩
    · rintro rfl; exact h.cap.sub.subset (by simp [core, stateSelf, Ctx.node])
  have hty := h.ty
  simp only [stateNodeS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true,
    hcl_of env c1, hcl_of env c2, hcl_of env c3, hcl_of env c4, hcl_of env c5, hcl_of env c6,
    hcl_of env c7, true_and]
  simp only [pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
    compsHold_nil, and_true, holds_class, vn_some, valueNode_oneOrMore, holds_equals, hA,
    holds_maxCount1, hup, holds_node, ShapesL.P, valueNode_pred, iri_vi, mem_ofList']
  rw [hT]
  have hanc : ∀ v, v ∈ A ↔ (⟨k.iri (stateFragment Y.id), vt "ancestor", v⟩ : Triple) ∈ ts := by
    intro v; rw [hv _ _ (by decide) (by decide)]; simp
  simp only [hanc, implies_true, true_and]
  have hupC : CompHolds (Graph.ofList ts) (some (I "child")) (maxC 1) (k.iri (stateFragment Y.id)) := by
    simp only [maxC, holds_maxCount1, vn_some, hup]; intro y z hy hz; exact hy.trans hz.symm
  simp only [hupC, true_and]
  rcases hel : Y.element with _ | _ | _ | hh <;> rcases hie : Y.initialElement with _ | ie <;>
    rcases hdd : Y.donedata with _ | dd <;>
    simp [hv, hty, hel, hie, hdd, instanceOf_iff (Graph.ofList ts) env.noSub, hIE, hDD, orderedS,
      elementClass, initKidsOpt, ddKidsOpt, refKids, refKids_none]

/-! ## Transitions seen from their owner -/

theorem trans_view {owner : Term} {prop X : String} {order : Nat} {source scope : Term} {tr : Transition}
    (hcap : Cap k ts (U X) (k.transition tr owner prop X order source scope)) (hown : Above (k := k) owner X)
    (l : String) (o : Term) (hl : l = "event" ∨ l = "cond" ∨ l = "target") :
    (⟨k.iri X, vt l, o⟩ : Triple) ∈ ts ↔
      (l = "event" ∧ o ∈ (tokKids k X (X ++ ":ev") "event" tr.events).map (fun d => k.iri d.1)) ∨
      (l = "cond" ∧ ∃ e, tr.cond = some e ∧ o = .lit (.typed e k.exprDt)) ∨
      (l = "target" ∧ o ∈ (refKids k (k.iri X) "target" X ":tg" tr.targets scope).map (fun d => k.iri d.1)) := by
  have hne := above_ne hown
  rw [obj_view' hcap l o (by rcases hl with rfl | rfl | rfl <;> decide) (by rcases hl with rfl | rfl | rfl <;> decide)]
  rcases hl with rfl | rfl | rfl <;>
  simp [Ctx.transition, Ctx.node, self_tok _ _ _ _ (fun j => ch_lit X ":ev" j (by simp) (by simp)),
    self_ref k X "target" X ":tg" (fun j => ch_lit X ":tg" j (by simp) (by simp)),
    self_block _ _ _ _ (ch_a _), self_ext, Ne.symm hne]

/-- The scope of the targets of a transition. -/
theorem trans_scope (env : Env k ts P) {owner : Term} {prop X : String} {order : Nat} {source scope : Term}
    {tr : Transition}
    (cx : Ctxt k ts owner prop X (k.transition tr owner prop X order source scope) (transKids k X tr scope)) :
    ∀ R v, (⟨k.iri X, vt "target", R⟩ : Triple) ∈ ts → (⟨R, vt "scope", v⟩ : Triple) ∈ ts → v = scope := by
  intro R v hR hv
  rw [trans_view cx.cap cx.own _ _ (by simp)] at hR
  simp only [String.reduceEq, false_and, false_or, true_and, refKids, List.map_map, List.mem_map,
    Function.comp_def] at hR
  obtain ⟨⟨x, j⟩, hxj, rfl⟩ := hR
  have hd : refK k (k.iri X) "target" X ":tg" scope (x, j) ∈ transKids k X tr scope := by
    simp only [transKids, List.mem_append, refKids]; exact .inl (.inl (.inr (List.mem_map.2 ⟨_, hxj, rfl⟩)))
  have hcapR := cx.kid hd
  have hneR := above_ne (k := k) (above_of_ch (cx.dec.ch _ hd))
  rw [obj_view' hcapR _ _ (by decide) (by decide)] at hv
  have hneR' : ¬ (X ++ ":tg" ++ toString j = X) := fun e => hneR (by simp only [refK]; rw [e])
  rcases refK_mem (k := k) hv with h | h | h | h | h | h <;> simp [Triple.mk.injEq] at h
  all_goals first | exact h | exact h.2 | exact absurd h.1 hneR' | exact absurd h.1.symm hneR' | (exfalso; apply hneR'; simpa [refK] using h.1)

/-- The transitions of a state node and their contexts. -/
theorem st_trans {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y) {T : Term}
    (hT : (⟨k.iri (stateFragment Y.id), vt "transition", T⟩ : Triple) ∈ ts) :
    ∃ tr j, (tr, j) ∈ Y.transitions.zipIdx ∧ T = k.iri (stateFragment Y.id ++ ":t" ++ toString j) ∧
      Ctxt k ts (k.iri (stateFragment Y.id)) "transition" (stateFragment Y.id ++ ":t" ++ toString j)
        (k.transition tr (k.iri (stateFragment Y.id)) "transition" (stateFragment Y.id ++ ":t" ++ toString j) j
          (k.iri (stateFragment Y.id)) (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element))
        (transKids k (stateFragment Y.id ++ ":t" ++ toString j) tr (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) := by
  rw [state_view h.cap (above_ne h.own) _ _ (by decide) (by decide)] at hT
  simp only [String.reduceEq, false_and, false_or, or_false, true_and, List.map_map, List.mem_map,
    Function.comp_def] at hT
  obtain ⟨⟨tr, j⟩, hj, rfl⟩ := hT
  have hd : transK k (stateFragment Y.id) (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element) (tr, j) ∈
      kidsOf k pt A Y := kidsOf_mem.2 (.inr (.inr (.inl (List.mem_map.2 ⟨_, hj, rfl⟩))))
  exact ⟨tr, j, hj, rfl, h.cx.kid hd, decomp_transition k _ _ _ _ _ _ _, above_of_ch (h.cx.dec.ch _ hd)⟩

theorem st_scope (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y) :
    ∀ T R v, (⟨k.iri (stateFragment Y.id), vt "transition", T⟩ : Triple) ∈ ts →
      (⟨T, vt "target", R⟩ : Triple) ∈ ts → (⟨R, vt "scope", v⟩ : Triple) ∈ ts →
      v = scopeOf pt A (k.iri (stateFragment Y.id)) Y.element := by
  intro T R v hT hR hv
  obtain ⟨tr, j, _, rfl, cxT⟩ := st_trans h hT
  exact trans_scope env cxT R v hR hv

def notHistoryOnly' (Y : State) : Prop := Y.children ≠ [] → ∃ c ∈ Y.children, c.isHistory = false

theorem notHO_ok (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y)
    (hchH : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "History"⟩ : Triple) ∈ ts ↔
      c.isHistory = true) :
    CompHolds (Graph.ofList ts) none notHistoryOnly (k.iri (stateFragment Y.id)) ↔ notHistoryOnly' Y := by
  have hv := state_view h.cap (above_ne h.own)
  simp only [notHistoryOnly, holds_or, vn_none, forall_eq, someConforms_cons, someConforms_nil, or_false,
    none0, propS, conforms_mk, compsHold_cons, compsHold_nil, and_true, maxC, holds_maxCount0, vn_some,
    ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hv, ne_eq, not_false_eq_true, holds_qualifiedMin1,
    nodeS, holds_not, cls, holds_class, instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq,
    String.reduceEq, false_and, false_or, true_and, notHistoryOnly', or_false, forall_eq]
  constructor
  · rintro (h1 | ⟨y, ⟨c, hc, rfl⟩, h2⟩) hne
    · cases hch : Y.children with
      | nil => exact absurd hch hne
      | cons c cs => exact absurd ⟨c, by simp [hch], rfl⟩ (h1 _)
    · refine ⟨c, hc, ?_⟩
      rw [hchH c hc] at h2; simpa using h2
  · intro hh
    cases hch : Y.children with
    | nil => left; rintro y ⟨c, hc, _⟩; simp [hch] at hc
    | cons c0 cs =>
      obtain ⟨c, hc, hcH⟩ := hh (by simp [hch])
      right; refine ⟨_, ⟨c, hch ▸ hc, rfl⟩, ?_⟩
      rw [hchH c hc]; simp [hcH]

theorem chartScope_ok (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (hsc : scopeOf pt A (k.iri (stateFragment Y.id)) Y.element = k.iri "chart") :
    CompHolds (Graph.ofList ts) none chartScope (k.iri (stateFragment Y.id)) := by
  simp only [chartScope, holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons,
    compsHold_nil, and_true, cls, holds_class, vn_some, valueNode_seq_cons, valueNode_seq_nil,
    ShapesL.P, valueNode_pred, iri_vi, mem_ofList', instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
  rintro v ⟨T, hT, R, hR, w, hw, hvw⟩
  rw [hvw, st_scope env h T R w hT hR hw, hsc]
  exact env.chart

theorem none_iff_view {i : Nat} {pt : Term} {A : List Term} {Y : State} (h : StCtx k ts i pt A Y) (l : String)
    (h1 : l ≠ "historyDefault") (h2 : l ≠ "choice") :
    CompHolds (Graph.ofList ts) none (pr l [maxC 0]) (k.iri (stateFragment Y.id)) ↔
      ∀ o, (⟨k.iri (stateFragment Y.id), vt l, o⟩ : Triple) ∉ ts := by
  simp [pr, propS, holds_property, maxC, holds_maxCount0]

def ElemOK (PS : Prop) (Y : State) : Prop :=
  match Y.element with
  | .state => notHistoryOnly' Y ∧ Y.donedata = none
  | .parallel => notHistoryOnly' Y ∧ Y.initial = [] ∧ Y.initialElement = none ∧ Y.donedata = none
  | .final => Y.children = [] ∧ Y.transitions = [] ∧ Y.invokes = [] ∧ Y.data = [] ∧ Y.initial = [] ∧
      Y.initialElement = none
  | .history _ => Y.transitions.length ≤ 1 ∧
      (∀ t ∈ Y.transitions, t.events = [] ∧ t.cond = none ∧ t.targets ≠ []) ∧ Y.children = [] ∧
      Y.onentry = [] ∧ Y.onexit = [] ∧ Y.invokes = [] ∧ Y.data = [] ∧ Y.initial = [] ∧
      Y.initialElement = none ∧ Y.donedata = none ∧ PS

set_option maxHeartbeats 2000000 in
theorem elem_ok_nonhist (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (hA0 : A.headD (k.iri (stateFragment Y.id)) = k.iri "chart")
    (hchH : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "History"⟩ : Triple) ∈ ts ↔
      c.isHistory = true) (PS : Prop) (hnh : Y.isHistory = false) :
    ((Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "State" → Conforms (Graph.ofList ts) stateS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Parallel" → Conforms (Graph.ofList ts) parallelS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Final" → Conforms (Graph.ofList ts) finalS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "History" → Conforms (Graph.ofList ts) historyS' (k.iri (stateFragment Y.id)))) ↔
      ElemOK PS Y := by
  have hv := state_view h.cap (above_ne h.own)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) l ↔ l = "StateNode" ∨ l = elementClass Y.element := by
    intro l; simp only [Ty, rdf_type_eq, mem_ofList']; exact h.ty l
  have hsc : Y.element ≠ .history .shallow → Y.element ≠ .history .deep →
      scopeOf pt A (k.iri (stateFragment Y.id)) Y.element = k.iri "chart" := by
    intro h1 h2; rcases hel : Y.element with _ | _ | _ | (_ | _) <;> simp_all [scopeOf]
  have hno := notHO_ok env h hchH
  rcases hel : Y.element with _ | _ | _ | hh
  · have hcs := chartScope_ok env h (hsc (by simp [hel]) (by simp [hel]))
    simp only [hty, hel, elementClass, String.reduceEq, or_false, false_or, false_implies, and_true,
      true_implies, ElemOK]
    simp only [stateS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, hno, hcs]
    simp [pr, propS, holds_property, maxC, holds_maxCount0, hv, hel, ddKidsOpt]
    cases Y.donedata <;> simp
  · have hcs := chartScope_ok env h (hsc (by simp [hel]) (by simp [hel]))
    simp only [hty, hel, elementClass, String.reduceEq, or_false, false_or, false_implies, and_true,
      true_implies, ElemOK, true_and]
    simp only [parallelS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, hno, hcs]
    simp [pr, propS, holds_property, maxC, holds_maxCount0, hv, hel, ddKidsOpt, initKidsOpt, refKids_none,
      kids_empty]
    rcases Y.donedata <;> rcases Y.initialElement <;> simp [kids_empty] <;> exact fun _ => ⟨_, fun _ => rfl⟩
  · simp only [hty, hel, elementClass, String.reduceEq, or_false, false_or, false_implies, and_true,
      true_implies, ElemOK, true_and]
    simp only [finalS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true]
    simp [pr, propS, holds_property, maxC, holds_maxCount0, hv, hel, initKidsOpt, refKids_none,
      kids_empty]
    constructor
    · intro hh
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
      · cases hc : Y.children with
        | nil => rfl
        | cons c cs => exact absurd rfl ((hh _).1 c (by simp [hc]))
      · cases ht : Y.transitions with
        | nil => rfl
        | cons t ts' => exact absurd rfl ((hh _).2.1 t 0 (by simp [ht]))
      · cases ht : Y.invokes with
        | nil => rfl
        | cons t ts' => exact absurd rfl ((hh _).2.2.1 t 0 (by simp [ht]))
      · cases ht : Y.data with
        | nil => rfl
        | cons t ts' => exact absurd rfl ((hh _).2.2.2.1 t 0 (by simp [ht]))
      · cases ht : Y.initial with
        | nil => rfl
        | cons t ts' => exact absurd rfl ((hh _).2.2.2.2.1
            (refK k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") "" (k.iri (stateFragment Y.id)) (t, 0)).1
            (refK k (k.iri (stateFragment Y.id)) "initial" (stateFragment Y.id ++ ":ia") "" (k.iri (stateFragment Y.id)) (t, 0)).2
            (by simp [refKids, ht]))
      · cases ht : Y.initialElement with
        | none => rfl
        | some ie => exact absurd rfl ((hh _).2.2.2.2.2 (stateFragment Y.id ++ ":init")
            (initEmission k (k.iri (stateFragment Y.id)) (stateFragment Y.id ++ ":init") ie) (by simp [ht]))
    · rintro ⟨h1, h2, h3, h4, h5, h6⟩; simp [h1, h2, h3, h4, h5, h6, refKids]
  · simp [State.isHistory, hel] at hnh

theorem zipIdx_le1 {α : Type} {l : List α} {g : Nat → Term} (hg : ∀ a b, g a = g b → a = b) :
    (∀ y z, (∃ a j, (a, j) ∈ l.zipIdx ∧ y = g j) → (∃ a j, (a, j) ∈ l.zipIdx ∧ z = g j) → y = z) ↔
      l.length ≤ 1 := by
  constructor
  · intro h
    match l, h with
    | [], _ => simp
    | [_], _ => simp
    | a :: b :: l, h =>
      have := hg _ _ (h (g 0) (g 1) ⟨a, 0, by simp, rfl⟩ ⟨b, 1, by simp, rfl⟩)
      simp at this
  · intro hl y z ⟨a, j, hj, hy⟩ ⟨b, j', hj', hz⟩
    have h1 := (List.mem_zipIdx hj).2.1
    have h2 := (List.mem_zipIdx hj').2.1
    subst hy; subst hz; congr 1; simp at h1 h2; omega

set_option maxHeartbeats 4000000 in
theorem elem_ok_hist (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (PS : Prop)
    (hparSN : (⟨pt, rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ PS) {hh : HistoryType}
    (hel : Y.element = .history hh) :
    ((Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "State" → Conforms (Graph.ofList ts) stateS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Parallel" → Conforms (Graph.ofList ts) parallelS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Final" → Conforms (Graph.ofList ts) finalS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "History" → Conforms (Graph.ofList ts) historyS' (k.iri (stateFragment Y.id)))) ↔
      ElemOK PS Y := by
  have hv := state_view h.cap (above_ne h.own)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) l ↔ l = "StateNode" ∨ l = elementClass Y.element := by
    intro l; simp only [Ty, rdf_type_eq, mem_ofList']; exact h.ty l
  have hup : ∀ v, (⟨v, vt "child", k.iri (stateFragment Y.id)⟩ : Triple) ∈ ts ↔ v = pt := by
    intro v
    constructor
    · intro hv'; exact (h.cont.conts v).1 ⟨"child", by simp [containers], hv'⟩
    · rintro rfl; exact h.cap.sub.subset (by simp [core, stateSelf, Ctx.node])
  -- the transitions
  have hT : ∀ T, (⟨k.iri (stateFragment Y.id), vt "transition", T⟩ : Triple) ∈ ts →
      ∃ tr j, (tr, j) ∈ Y.transitions.zipIdx ∧ T = k.iri (stateFragment Y.id ++ ":t" ++ toString j) ∧
        (∀ l o, (l = "event" ∨ l = "cond" ∨ l = "target") → ((⟨T, vt l, o⟩ : Triple) ∈ ts ↔
          (l = "event" ∧ o ∈ (tokKids k (stateFragment Y.id ++ ":t" ++ toString j)
            (stateFragment Y.id ++ ":t" ++ toString j ++ ":ev") "event" tr.events).map (fun d => k.iri d.1)) ∨
          (l = "cond" ∧ ∃ e, tr.cond = some e ∧ o = .lit (.typed e k.exprDt)) ∨
          (l = "target" ∧ o ∈ (refKids k (k.iri (stateFragment Y.id ++ ":t" ++ toString j)) "target"
            (stateFragment Y.id ++ ":t" ++ toString j) ":tg" tr.targets
            (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)).map (fun d => k.iri d.1)))) := by
    intro T hTm
    obtain ⟨tr, j, hj, rfl, cxT⟩ := st_trans h hTm
    exact ⟨tr, j, hj, rfl, fun l o hl => trans_view cxT.cap cxT.own l o hl⟩
  have hTmem : ∀ T, (⟨k.iri (stateFragment Y.id), vt "transition", T⟩ : Triple) ∈ ts ↔
      ∃ tr j, (tr, j) ∈ Y.transitions.zipIdx ∧ T = k.iri (stateFragment Y.id ++ ":t" ++ toString j) := by
    intro T; rw [hv _ _ (by decide) (by decide)]
    simp [transK, eq_comm]
  have hcxT : ∀ tr j, (tr, j) ∈ Y.transitions.zipIdx →
      Ctxt k ts (k.iri (stateFragment Y.id)) "transition" (stateFragment Y.id ++ ":t" ++ toString j)
        (k.transition tr (k.iri (stateFragment Y.id)) "transition" (stateFragment Y.id ++ ":t" ++ toString j) j
          (k.iri (stateFragment Y.id)) (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element))
        (transKids k (stateFragment Y.id ++ ":t" ++ toString j) tr (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element)) := by
    intro tr j hj
    have hd : transK k (stateFragment Y.id) (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element) (tr, j) ∈
        kidsOf k pt A Y := kidsOf_mem.2 (.inr (.inr (.inl (List.mem_map.2 ⟨_, hj, rfl⟩))))
    exact ⟨h.cx.kid hd, decomp_transition k _ _ _ _ _ _ _, above_of_ch (h.cx.dec.ch _ hd)⟩
  have heventless : ∀ tr j, (tr, j) ∈ Y.transitions.zipIdx →
      (Conforms (Graph.ofList ts) eventlessTargetedS (k.iri (stateFragment Y.id ++ ":t" ++ toString j)) ↔
        tr.events = [] ∧ tr.cond = none ∧ tr.targets ≠ []) := by
    intro tr j hj
    have hvT := fun l o hl => trans_view (hcxT tr j hj).cap (hcxT tr j hj).own l o hl
    simp only [eventlessTargetedS, nodeS, conforms_mk, compsHold_cons, compsHold_nil, and_true, pr, propS,
      holds_property, vn_none, forall_eq, maxC, minC, holds_maxCount0, holds_minCount1, vn_some,
      ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hvT _ _ (.inl rfl), hvT _ _ (.inr (.inl rfl)),
      hvT _ _ (.inr (.inr rfl)), String.reduceEq, false_and, or_false, false_or, true_and]
    constructor
    · rintro ⟨h1, h2, h3⟩
      refine ⟨?_, ?_, ?_⟩
      · cases he : tr.events with
        | nil => rfl
        | cons e es => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (show tokK k (stateFragment Y.id ++ ":t" ++ toString j) (stateFragment Y.id ++ ":t" ++ toString j ++ ":ev")
              "event" (e, 0) ∈ tokKids k (stateFragment Y.id ++ ":t" ++ toString j)
                (stateFragment Y.id ++ ":t" ++ toString j ++ ":ev") "event" tr.events by simp [tokKids, he])) (h1 _)
      · cases hc : tr.cond with
        | none => rfl
        | some e => exact absurd ⟨e, hc, rfl⟩ (h2 (.lit (.typed e k.exprDt)))
      · intro he; obtain ⟨y, hy⟩ := h3; simp [refKids, he] at hy
    · rintro ⟨h1, h2, h3⟩
      refine ⟨fun y hy => by simp [tokKids, h1] at hy, fun y ⟨e, he, _⟩ => by simp [h2] at he, ?_⟩
      cases ht : tr.targets with
      | nil => exact absurd ht h3
      | cons x xs => simp [refKids, ht]
  -- the scope property, always true
  have hscopeP : ∀ T R, (⟨k.iri (stateFragment Y.id), vt "transition", T⟩ : Triple) ∈ ts →
      (⟨T, vt "target", R⟩ : Triple) ∈ ts → ∀ v,
      ((∃ z, (⟨z, vt "target", R⟩ : Triple) ∈ ts ∧ ∃ z1, (⟨z1, vt "transition", z⟩ : Triple) ∈ ts ∧
        ∃ z2, (⟨z2, vt "child", z1⟩ : Triple) ∈ ts ∧ v = z2) ↔ (⟨R, vt "scope", v⟩ : Triple) ∈ ts) := by
    intro T R hTm hR v
    obtain ⟨tr, j, hj, rfl, cxT⟩ := st_trans h hTm
    have hcT : Contained' k ts (k.iri (stateFragment Y.id ++ ":t" ++ toString j)) (k.iri (stateFragment Y.id)) :=
      Contained'.kid env h.cont (ch_lit _ ":t" j (by simp) (by simp)) (by simp [containers]) hTm
    have hR' := hR
    rw [trans_view cxT.cap cxT.own _ _ (by simp)] at hR'
    simp only [String.reduceEq, false_and, false_or, true_and, refKids, List.map_map, List.mem_map,
      Function.comp_def] at hR'
    obtain ⟨⟨x, jj⟩, hxj, rfl⟩ := hR'
    obtain ⟨hcapR, hcR, _⟩ := kid_ctx env cxT hcT (kidsL_ref k _ "target" _ ":tg" tr.targets _)
      (fun d hd => List.mem_append_left _ (List.mem_append_left _ (List.mem_append_right _ hd)))
      (by simp [containers]) (List.mem_map.2 ⟨_, hxj, rfl⟩)
    have hsc : scopeOf pt A (k.iri (stateFragment Y.id)) Y.element = pt := by simp [scopeOf, hel]
    have e1 : ∀ z, (⟨z, vt "target", k.iri (refK k (k.iri (stateFragment Y.id ++ ":t" ++ toString j)) "target"
        (stateFragment Y.id ++ ":t" ++ toString j) ":tg" (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element) (x, jj)).1⟩ : Triple) ∈ ts ↔
        z = k.iri (stateFragment Y.id ++ ":t" ++ toString j) :=
      fun z => ⟨fun hz => (hcR.conts z).1 ⟨"target", by simp [containers], hz⟩, fun hz => hz ▸ hR⟩
    have e2 : ∀ z, (⟨z, vt "transition", k.iri (stateFragment Y.id ++ ":t" ++ toString j)⟩ : Triple) ∈ ts ↔
        z = k.iri (stateFragment Y.id) :=
      fun z => ⟨fun hz => (hcT.conts z).1 ⟨"transition", by simp [containers], hz⟩, fun hz => hz ▸ hTm⟩
    simp only [e1, e2, hup, exists_eq_left]
    constructor
    · intro hvv
      rw [hvv]
      have := hcapR.sub.subset (show (⟨k.iri (refK k (k.iri (stateFragment Y.id ++ ":t" ++ toString j)) "target"
        (stateFragment Y.id ++ ":t" ++ toString j) ":tg" (scopeOf pt A (k.iri (stateFragment Y.id)) Y.element) (x, jj)).1,
        vt "scope", scopeOf pt A (k.iri (stateFragment Y.id)) Y.element⟩ : Triple) ∈ _ by simp [refK, Ctx.node])
      rwa [hsc] at this
    · intro hv'
      have := trans_scope env cxT _ v hR hv'
      rw [this, hsc]
  have c5 : CompHolds (Graph.ofList ts) none (.property (propS (.seq [ShapesL.P "transition", ShapesL.P "target"])
      [.property (propS (.seq [ShapesL.I "target", ShapesL.I "transition", ShapesL.I "child"]) [.equals (vi "scope")])]))
      (k.iri (stateFragment Y.id)) := by
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil, and_true,
      vn_some, valueNode_seq_cons, valueNode_seq_nil, ShapesL.P, ShapesL.I, valueNode_pred, valueNode_inv,
      iri_vi, mem_ofList', holds_equals]
    rintro R ⟨T, hT, R', hR, hRR⟩ v
    subst hRR
    have := hscopeP T R hT hR v
    rw [← this]
  have c4 : CompHolds (Graph.ofList ts) none (.property (propS (ShapesL.I "child") [cls "StateNode"]))
      (k.iri (stateFragment Y.id)) ↔ PS := by
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil, and_true,
      cls, holds_class, vn_some, ShapesL.I, ShapesL.P, valueNode_inv, valueNode_pred, iri_vi, mem_ofList', hup,
      forall_eq, instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq, hparSN]
  have c2 : CompHolds (Graph.ofList ts) none (pr "transition" [maxC 1, .node eventlessTargetedS])
      (k.iri (stateFragment Y.id)) ↔ Y.transitions.length ≤ 1 ∧
        ∀ t ∈ Y.transitions, t.events = [] ∧ t.cond = none ∧ t.targets ≠ [] := by
    simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      and_true, maxC, holds_maxCount1, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hTmem,
      holds_node]
    rw [zipIdx_le1 (g := fun j => k.iri (stateFragment Y.id ++ ":t" ++ toString j))
      (fun a b hab => frag_inj _ _ _ _ ((iri_inj k).1 hab))]
    apply and_congr_right; intro _
    constructor
    · intro hh t ht
      obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem ht
      have hm : (Y.transitions[j], j) ∈ Y.transitions.zipIdx := List.mem_zipIdx_iff_getElem?.2 (by simp)
      exact (heventless _ _ hm).1 (hh _ ⟨_, j, hm, rfl⟩)
    · rintro hh v ⟨tr, j, hj, rfl⟩
      exact (heventless _ _ hj).2 (hh tr (fst_mem_zipIdx hj))
  have c3 : CompHolds (Graph.ofList ts) none (.property (propS (.alt [ShapesL.P "child", ShapesL.P "onentry", ShapesL.P "onexit",
      ShapesL.P "invoke", ShapesL.P "data", ShapesL.P "initial", ShapesL.P "initialElement", ShapesL.P "donedata"]) [maxC 0]))
      (k.iri (stateFragment Y.id)) ↔ Y.children = [] ∧ Y.onentry = [] ∧ Y.onexit = [] ∧ Y.invokes = [] ∧
        Y.data = [] ∧ Y.initial = [] ∧ Y.initialElement = none ∧ Y.donedata = none := by
    simp only [holds_property, vn_none, forall_eq, conforms_propS, compsHold_cons, compsHold_nil, and_true,
      maxC, holds_maxCount0, vn_some, valueNode_alt, List.mem_cons, List.not_mem_nil, or_false,
      exists_eq_or_imp, exists_false, exists_eq_left, ShapesL.P, valueNode_pred, iri_vi, mem_ofList', hv, ne_eq,
      String.reduceEq, not_false_eq_true, false_and, true_and, false_or, not_or, forall_and]
    constructor
    · rintro ⟨a1, a2, a3, a4, a5, a6, a7, a8⟩
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · cases hc : Y.children with
        | nil => rfl
        | cons c cs => exact absurd ⟨c, by simp [hc], rfl⟩ (a1 _)
      · cases hc : Y.onentry with
        | nil => rfl
        | cons c cs => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (List.mem_map_of_mem (f := hookK k _ _ _ _) (show (c, 0) ∈ Y.onentry.zipIdx by simp [hc]))) (a2 _)
      · cases hc : Y.onexit with
        | nil => rfl
        | cons c cs => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (List.mem_map_of_mem (f := hookK k _ _ _ _) (show (c, 0) ∈ Y.onexit.zipIdx by simp [hc]))) (a3 _)
      · cases hc : Y.invokes with
        | nil => rfl
        | cons c cs => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (List.mem_map_of_mem (f := invK k _) (show (c, 0) ∈ Y.invokes.zipIdx by simp [hc]))) (a4 _)
      · cases hc : Y.data with
        | nil => rfl
        | cons c cs => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (List.mem_map_of_mem (f := dataK k _) (show (c, 0) ∈ Y.data.zipIdx by simp [hc]))) (a5 _)
      · cases hc : Y.initial with
        | nil => rfl
        | cons c cs => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (List.mem_map_of_mem (f := refK k _ _ _ _ _) (show (c, 0) ∈ Y.initial.zipIdx by simp [hc]))) (a6 _)
      · cases hc : Y.initialElement with
        | none => rfl
        | some c => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (show (stateFragment Y.id ++ ":init", initEmission k (k.iri (stateFragment Y.id))
              (stateFragment Y.id ++ ":init") c) ∈ initKidsOpt k (stateFragment Y.id) Y.initialElement by
                simp [initKidsOpt, hc])) (a7 _)
      · cases hc : Y.donedata with
        | none => rfl
        | some c => exact absurd (List.mem_map_of_mem (f := fun d => k.iri d.1)
            (show (stateFragment Y.id ++ ":dd", k.donedata (k.iri (stateFragment Y.id))
              (stateFragment Y.id ++ ":dd") c) ∈ ddKidsOpt k (stateFragment Y.id) Y.donedata by
                simp [ddKidsOpt, hc])) (a8 _)
    · rintro ⟨a1, a2, a3, a4, a5, a6, a7, a8⟩
      simp [a1, a2, a3, a4, a5, a6, a7, a8, refKids, initKidsOpt, ddKidsOpt]
  have c1 : CompHolds (Graph.ofList ts) none (pr "historyType" [minC 1, maxC 1,
      .test (.in_ [strLit "shallow", strLit "deep"])]) (k.iri (stateFragment Y.id)) := by
    simp only [pr, propS, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons, compsHold_nil,
      and_true, minC, maxC, holds_minCount1, holds_maxCount1, holds_test, vn_some, ShapesL.P,
      valueNode_pred, iri_vi, mem_ofList', hv, ne_eq, String.reduceEq, not_false_eq_true, true_and,
      false_and, or_false, hel, StateElement.history.injEq, exists_eq_left']
    cases hh <;> simp [histStr]
  simp only [hty, hel, elementClass, String.reduceEq, or_false, false_or, false_implies, true_and,
    true_implies, ElemOK]
  simp only [historyS', top, conforms_mk, compsHold_cons, compsHold_nil, and_true, c1, c2, c3, c4, c5,
    true_and]
  constructor
  · rintro ⟨⟨a, b⟩, ⟨c, d, e, f, g, h', i', j⟩, l⟩; exact ⟨a, b, c, d, e, f, g, h', i', j, l⟩
  · rintro ⟨a, b, c, d, e, f, g, h', i', j, l⟩; exact ⟨⟨a, b⟩, ⟨c, d, e, f, g, h', i', j⟩, l⟩

/-- **The node shapes at a state node.** -/
theorem good_state_node (env : Env k ts P) {i : Nat} {pt : Term} {A : List Term} {Y : State}
    (h : StCtx k ts i pt A Y) (hA0 : A.headD (k.iri (stateFragment Y.id)) = k.iri "chart")
    (hchH : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "History"⟩ : Triple) ∈ ts ↔
      c.isHistory = true)
    (hchT : ∀ c ∈ Y.children, (⟨k.iri (stateFragment c.id), rdfType, vt "StateNode"⟩ : Triple) ∈ ts)
    (PS : Prop) (hparSN : (⟨pt, rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ PS)
    (Kn : String → Prop)
    (hSN : ∀ x, (⟨k.iri (stateFragment x), rdfType, vt "StateNode"⟩ : Triple) ∈ ts ↔ Kn x)
    (hA : ∀ v, Plus (ValueNode (Graph.ofList ts) (ShapesL.I "child")) (k.iri (stateFragment Y.id)) v ↔ v ∈ A)
    (hni : ∀ s, (⟨s, vt "initial", k.iri (stateFragment Y.id)⟩ : Triple) ∉ ts) :
    ShapesL.Good (Graph.ofList ts) (k.iri (stateFragment Y.id)) ↔
      (¬(Y.initial ≠ [] ∧ Y.initialElement.isSome) ∧ Y.id ≠ "" ∧
        ∀ x ∈ Y.initial, TR k ts Kn x (k.iri (stateFragment Y.id))) ∧ ElemOK PS Y := by
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) l ↔ l = "StateNode" ∨ l = elementClass Y.element := by
    intro l; simp only [Ty, rdf_type_eq, mem_ofList']; exact h.ty l
  have hE : ((Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "State" → Conforms (Graph.ofList ts) stateS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Parallel" → Conforms (Graph.ofList ts) parallelS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "Final" → Conforms (Graph.ofList ts) finalS (k.iri (stateFragment Y.id))) ∧
     (Ty (Graph.ofList ts) (k.iri (stateFragment Y.id)) "History" → Conforms (Graph.ofList ts) historyS' (k.iri (stateFragment Y.id)))) ↔
      ElemOK PS Y := by
    rcases hel : Y.element with _ | _ | _ | hh
    · exact elem_ok_nonhist env h hA0 hchH PS (by simp [State.isHistory, hel])
    · exact elem_ok_nonhist env h hA0 hchH PS (by simp [State.isHistory, hel])
    · exact elem_ok_nonhist env h hA0 hchH PS (by simp [State.isHistory, hel])
    · exact elem_ok_hist env h PS hparSN hel
  rw [good_iff _ env.noSub, ← snS_ok env h Kn hSN hA hchT, ← hE]
  have hc := h.cont.ok env
  rcases hel : Y.element with _ | _ | _ | hh <;>
  simp only [hty, hel, elementClass, containedClasses, mem_ofList', hni, exists_false, false_implies,
    true_implies, String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true, true_or]

end Vertep.Rdf.Equiv
