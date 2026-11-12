import VertepLean.Rdf.Equiv.ExecRegion
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on handlers, data, done data and invokes

The node shapes hold in the region of an `onentry`/`onexit` handler, a `<data>` item, a
`<donedata>` or an `<invoke>` exactly when its well-formedness rules do.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

theorem reg_hook (env : Env k ts P) {s : Term} {prop cls F : String} {i : Nat} {b : List Exec}
    (hcls : cls = "OnEntry" ∨ cls = "OnExit")
    (cx : Ctxt k ts s prop F (hookEmission k s prop cls F i b) (blockKids k F ":a" "action" 0 b))
    (hc : Contained' k ts (k.iri F) s) (hni : ∀ x, (⟨x, vt "initial", k.iri F⟩ : Triple) ∉ ts) :
    RG k ts F ↔ BlockOk b := by
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts := by
    intro v hv; rw [hov _ _ (by decide) (by decide)] at hv
    simp [hookEmission, Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne] at hv
  have hb := reg_block env cx hc hnoF ":a" "action" (by simp [containers]) 0 b (fun d hd => hd)
  simp only [RG] at hb
  rw [RG, region_iff env cx, hb]
  have hacV := objV cx (p := "action") (c := "ExecutableContent") (D' := blockKids k F ":a" "action" 0 b)
    (fun d hd => hd) (ty_blockKids k _ ":a" "action" 0 b) (by decide) (by decide)
    (fun v hv => by
      simp [hookEmission, Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne] at hv; simpa using hv)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri F) l ↔ l = cls := by
    intro l; rw [ty_view cx.cap]
    simp [hookEmission, Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne, eq_comm]
  suffices ShapesL.Good (Graph.ofList ts) (k.iri F) by simp [this]
  rw [good_iff _ env.noSub]
  rcases hcls with rfl | rfl <;>
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or] <;>
  simp (config := {contextual := true}) [handlerS, orderedS, hov, hookEmission, Ctx.node,
    self_block _ _ _ _ (ch_a _), Ne.symm hne, hacV, instanceOf_iff (Graph.ofList ts) env.noSub]

theorem reg_data (env : Env k ts P) {owner : Term} {of : String} {i : Nat} {d : Data}
    (cx : Ctxt k ts owner "data" (of ++ ":d" ++ toString i) (k.dataItem owner of i d)
      (extKids k (of ++ ":d" ++ toString i) d.extensions))
    (hc : Contained' k ts (k.iri (of ++ ":d" ++ toString i)) owner)
    (hni : ∀ x, (⟨x, vt "initial", k.iri (of ++ ":d" ++ toString i)⟩ : Triple) ∉ ts) :
    RG k ts (of ++ ":d" ++ toString i) ↔ DataOk d := by
  simp only [Ctx.dataItem] at cx
  generalize of ++ ":d" ++ toString i = F at cx hc hni ⊢
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts := by
    intro v hv; rw [hov _ _ (by decide) (by decide)] at hv
    simp [Ctx.dataItem, Ctx.node, self_ext, Ne.symm hne] at hv
  have hx := rg_exts env cx hc hnoF (xs := d.extensions) (fun d hd => hd)
  simp only [RG] at hx
  rw [RG, region_iff env cx, and_iff_left hx]
  have hxV := objV cx (p := "extension") (c := "ExtensionNode")
    (D' := extKids k F d.extensions)
    (fun d hd => hd) (ty_extKids k _ _) (by decide) (by decide)
    (fun v hv => by simp [Ctx.dataItem, Ctx.node, self_ext, Ne.symm hne] at hv; simpa using hv)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri F) l ↔ l = "DataItem" := by
    intro l; rw [ty_view cx.cap]
    simp [Ctx.dataItem, Ctx.node, self_ext, Ne.symm hne, eq_comm]
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  have hcls : CompHolds (Graph.ofList ts) none (clsP "extension" "ExtensionNode")
      (k.iri F) := by
    simp only [clsP, pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
      compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList',
      instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
    exact hxV
  simp only [dataItemS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, hcls]
  rcases d with ⟨id, _ | src, _ | ex, _ | ct, xs⟩ <;>
  simp [orderedS, hov, Ctx.dataItem, Ctx.node, self_ext, Ne.symm hne, DataOk]
  exact ⟨_, fun h => absurd rfl h⟩

theorem reg_donedata (env : Env k ts P) {owner : Term} {F : String} {dd : DoneData}
    (cx : Ctxt k ts owner "donedata" F (k.donedata owner F dd) (ddKids k F dd))
    (hc : Contained' k ts (k.iri F) owner) (hni : ∀ x, (⟨x, vt "initial", k.iri F⟩ : Triple) ∉ ts) :
    RG k ts F ↔ (¬(dd.content.isSome ∧ dd.params ≠ []) ∧ ParamsOk dd.params) := by
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts := by
    intro v hv; rw [hov _ _ (by decide) (by decide)] at hv
    rcases hcd : dd.content with _ | c <;>
    simp [Ctx.donedata, Ctx.node, self_param, self_content, hcd, Ne.symm hne] at hv
  have hp := rg_params env cx hc hnoF (ps := dd.params) (fun d hd => by simp [ddKids, hd])
  have hco := rg_content env cx hc hnoF (c := dd.content) (fun d hd => by simp [ddKids, hd])
  rw [RG, region_iff env cx]
  have hsplit : (∀ d ∈ ddKids k F dd, ∀ g, U d.1 g → ShapesL.Good (Graph.ofList ts) (k.iri g)) ↔
      ParamsOk dd.params := by
    rw [← hp]; simp only [ddKids, List.mem_append]
    constructor
    · intro h d hd; exact h d (.inr hd)
    · intro h d hd; rcases hd with hd | hd
      · exact hco d hd
      · exact h d hd
  rw [hsplit]
  have hpaV := objV cx (p := "param") (c := "Param") (D' := paramKids k F dd.params)
    (fun d hd => by simp [ddKids, hd]) (ty_paramKids k F _) (by decide) (by decide)
    (fun v hv => by
      rcases hcd : dd.content with _ | c <;>
      simp [Ctx.donedata, Ctx.node, self_param, self_content, hcd, Ne.symm hne] at hv <;> simpa using hv)
  have hcoV := objV cx (p := "content") (c := "Content") (D' := contentKids k F dd.content)
    (fun d hd => by simp [ddKids, hd]) (ty_contentKids k F _) (by decide) (by decide)
    (fun v hv => by
      rcases hcd : dd.content with _ | c <;>
      simp [Ctx.donedata, Ctx.node, self_param, self_content, hcd, Ne.symm hne, contentKids] at hv <;>
      simpa [contentKids, hcd] using hv)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri F) l ↔ l = "DoneData" := by
    intro l; rw [ty_view cx.cap]
    rcases hcd : dd.content with _ | c <;>
    simp [Ctx.donedata, Ctx.node, self_param, self_content, hcd, Ne.symm hne, eq_comm]
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  rcases dd with ⟨_ | c, ps⟩ <;>
  simp (config := {contextual := true}) [doneDataS, hov, Ctx.donedata, Ctx.node, self_param,
    self_content, Ne.symm hne, hpaV, hcoV, instanceOf_iff (Graph.ofList ts) env.noSub, forall_and,
    kids_empty]

set_option maxHeartbeats 1000000 in
theorem reg_invoke (env : Env k ts P) {owner : Term} {F : String} {i : Nat} {v : Invoke}
    (cx : Ctxt k ts owner "invoke" F (k.invoke owner F i v) (invKids k F v))
    (hc : Contained' k ts (k.iri F) owner) (hni : ∀ x, (⟨x, vt "initial", k.iri F⟩ : Triple) ∉ ts) :
    RG k ts F ↔ InvokeOk v := by
  have hne := above_ne cx.own
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hE : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨k.iri F, vt l, o⟩ : Triple) ∈ ts ↔ (⟨k.iri F, vt l, o⟩ : Triple) ∈ k.invoke owner F i v) := hov
  have hnoF : ∀ o, (⟨k.iri F, vt "initial", o⟩ : Triple) ∉ ts := by
    intro o ho; rw [hov _ _ (by decide) (by decide)] at ho
    rcases hcv : v.content with _ | c <;>
    simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
      self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne] at ho
  have hp := rg_params env cx hc hnoF (ps := v.params) (fun d hd => by simp [invKids, hd])
  have ht := rg_toks env cx hc hnoF (pre := F ++ ":nl") (p := "namelist") (ch_nl F)
    (by simp [containers]) (vs := v.namelist) (fun d hd => by simp [invKids, hd])
  have hco := rg_content env cx hc hnoF (c := v.content) (fun d hd => by simp [invKids, hd])
  have hx := rg_exts env cx hc hnoF (xs := v.extensions) (fun d hd => by simp [invKids, hd])
  have hb := reg_block env cx hc hnoF ":f" "finalize" (by simp [containers]) 0 v.finalize
    (fun d hd => by simp [invKids, hd])
  rw [RG, region_iff env cx]
  have hsplit : (∀ d ∈ invKids k F v, ∀ g, U d.1 g → ShapesL.Good (Graph.ofList ts) (k.iri g)) ↔
      ParamsOk v.params ∧ BlockOk v.finalize := by
    rw [← hp, ← hb]; simp only [invKids, List.mem_append, RG]
    constructor
    · intro h; exact ⟨fun d hd => h d (.inl (.inl (.inl (.inr hd)))), fun d hd => h d (.inl (.inr hd))⟩
    · rintro ⟨h1, h2⟩ d hd
      rcases hd with (((hd | hd) | hd) | hd) | hd
      · exact ht d hd
      · exact h1 d hd
      · exact hco d hd
      · exact h2 d hd
      · exact hx d hd
  rw [hsplit]
  have hnlV := objV cx (p := "namelist") (c := "Token") (D' := tokKids k F (F ++ ":nl") "namelist" v.namelist)
    (fun d hd => by simp [invKids, hd]) (ty_tokKids k F _ _ _) (by decide) (by decide)
    (fun o ho => by
      rcases hcv : v.content with _ | c <;>
      simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne] at ho <;> simpa using ho)
  have hpaV := objV cx (p := "param") (c := "Param") (D' := paramKids k F v.params)
    (fun d hd => by simp [invKids, hd]) (ty_paramKids k F _) (by decide) (by decide)
    (fun o ho => by
      rcases hcv : v.content with _ | c <;>
      simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne] at ho <;> simpa using ho)
  have hcoV := objV cx (p := "content") (c := "Content") (D' := contentKids k F v.content)
    (fun d hd => by simp [invKids, hd]) (ty_contentKids k F _) (by decide) (by decide)
    (fun o ho => by
      rcases hcv : v.content with _ | c <;>
      simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne, contentKids] at ho <;> simpa [contentKids, hcv] using ho)
  have hfiV := objV cx (p := "finalize") (c := "ExecutableContent") (D' := blockKids k F ":f" "finalize" 0 v.finalize)
    (fun d hd => by simp [invKids, hd]) (ty_blockKids k F _ _ _ _) (by decide) (by decide)
    (fun o ho => by
      rcases hcv : v.content with _ | c <;>
      simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne] at ho <;> simpa using ho)
  have hxV := objV cx (p := "extension") (c := "ExtensionNode") (D' := extKids k F v.extensions)
    (fun d hd => by simp [invKids, hd]) (ty_extKids k F _) (by decide) (by decide)
    (fun o ho => by
      rcases hcv : v.content with _ | c <;>
      simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne] at ho <;> simpa using ho)
  -- the `vt:expr` of the content node
  have hcx : ∀ c, v.content = some c → ∀ o, (⟨k.iri (F ++ ":c"), vt "expr", o⟩ : Triple) ∈ ts ↔
      ∃ e, c.expr = some e ∧ o = .lit (.typed e k.exprDt) := by
    intro c hcv o
    have hd : (F ++ ":c", k.content (k.iri F) F c) ∈ invKids k F v := by simp [invKids, contentKids, hcv]
    have hcap := cx.kid hd
    have hneC := above_ne (k := k) (above_of_ch (cx.dec.ch _ hd))
    rw [obj_view' hcap _ _ (by decide) (by decide)]
    simp only at hneC ⊢
    rcases c with ⟨_ | e, _ | b⟩ <;> simp [Ctx.content, Ctx.node, Ne.symm hneC]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri F) l ↔ l = "Invoke" := by
    intro l; rw [ty_view cx.cap]
    rcases hcv : v.content with _ | c <;>
    simp [Ctx.invoke, Ctx.node, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
      self_block _ _ _ _ (ch_f _), hcv, Ne.symm hne, eq_comm]
  rw [good_iff _ env.noSub]
  simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
    String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
    true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, or_true,
    true_or]
  have hcl : ∀ (l c : String), (∀ o, (⟨k.iri F, vt l, o⟩ : Triple) ∈ ts → (⟨o, rdfType, vt c⟩ : Triple) ∈ ts) →
      CompHolds (Graph.ofList ts) none (clsP l c) (k.iri F) := by
    intro l c h
    simp only [clsP, pr, propS, cls, holds_property, vn_none, forall_eq, conforms_mk, compsHold_cons,
      compsHold_nil, and_true, holds_class, vn_some, ShapesL.P, valueNode_pred, iri_vi, mem_ofList',
      instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq]
    exact h
  simp only [invokeS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true,
    hcl _ _ hnlV, hcl _ _ hpaV, hcl _ _ hfiV, hcl _ _ hxV, true_and]
  rcases hcv : v with ⟨a1, a2, a3, a4, a5, a6, nl, af, ps, _ | c, fin, xs⟩ <;>
    simp only [hcv] at hE hcoV hcx <;>
    simp (config := {contextual := true}) [orderedS, hE, Ctx.invoke, Ctx.node, self_ext,
      self_tok _ _ _ _ (ch_nl _), self_param, self_content, self_block _ _ _ _ (ch_f _),
      Ne.symm hne, hcoV, hcx, forall_some_ne, or_none_iff, forall_and, kids_empty,
      instanceOf_iff (Graph.ofList ts) env.noSub, InvokeOk, Decidable.or_iff_not_imp_left, and_assoc, Option.isSome_iff_ne_none]

end Vertep.Rdf.Equiv
