import VertepLean.Rdf.Equiv.Leaves
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on executable content

For parameters, tokens, extension nodes, content nodes and executable content elements: the node
shapes at the node, from its captured emission, its single container and its not being an object
of `vt:initial`.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

/-- `RG X`: the node shapes hold at every node of the region of `X`. -/
def RG (k : Ctx) (ts : List Triple) (X : String) : Prop :=
  ∀ g, U X g → ShapesL.Good (Graph.ofList ts) (k.iri g)

theorem obj_view' {X : String} {E : List Triple} (hcap : Cap k ts (U X) E) (l : String) (o : Term)
    (hl : l ≠ "historyDefault") (hc : l ≠ "choice") :
    (⟨k.iri X, vt l, o⟩ : Triple) ∈ ts ↔ (⟨k.iri X, vt l, o⟩ : Triple) ∈ E :=
  hcap.lookup (U_refl X) (by simp [hl, hc])

theorem noInit_of {x owner : Term} (hc : Contained' k ts x owner)
    (hno : ∀ o, (⟨owner, vt "initial", o⟩ : Triple) ∉ ts) :
    ∀ s, (⟨s, vt "initial", x⟩ : Triple) ∉ ts := fun s hs => by
  have := (hc.conts s).1 ⟨"initial", by simp [containers], hs⟩
  subst this; exact hno _ hs

/-- The context of a child in a list of children linked by `p` from the node `X`. -/
theorem kid_ctx (env : Env k ts P) {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (cx : Ctxt k ts owner prop X E D)
    (hcX : Contained' k ts (k.iri X) owner) {p : String} {D' : List (String × List Triple)}
    (hL : KidsL k X p D') (hD' : ∀ d ∈ D', d ∈ D) (hp : p ∈ containers)
    {d : String × List Triple} (hd : d ∈ D') :
    Cap k ts (U d.1) d.2 ∧ Contained' k ts (k.iri d.1) (k.iri X) ∧ Above (k := k) (k.iri X) d.1 := by
  have hcap := cx.kid (hD' d hd)
  have hch := cx.dec.ch d (hD' d hd)
  exact ⟨hcap, Contained'.kid env hcX hch hp (hcap.sub.subset (hL d hd).2), above_of_ch hch⟩

/-! ## Leaves -/

syntax "leaf_tac" term "," term "," term "," term : tactic
macro_rules
  | `(tactic| leaf_tac $env, $hc, $hty, $hni) => `(tactic|
    (rw [good_iff _ ($env).noSub]
     simp only [$hty:term, containedClasses, mem_ofList', $hni:term, exists_false, false_implies,
       true_implies, String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or,
       implies_true, true_and, and_true, ($hc).ok $env, List.not_mem_nil, exists_eq_left, or_self,
       iff_true]))

theorem good_tok (env : Env k ts P) {f pre prop : String} {v : String} {j : Nat}
    (hcap : Cap k ts (U (tokK k f pre prop (v, j)).1) (tokK k f pre prop (v, j)).2)
    (hown : Above (k := k) (k.iri f) (tokK k f pre prop (v, j)).1)
    (hc : Contained' k ts (k.iri (tokK k f pre prop (v, j)).1) (k.iri f))
    (hni : ∀ s, (⟨s, vt "initial", k.iri (tokK k f pre prop (v, j)).1⟩ : Triple) ∉ ts) :
    RG k ts (tokK k f pre prop (v, j)).1 := by
  have hne := above_ne hown
  simp only [tokK] at hcap hc hni hne hown ⊢
  generalize pre ++ toString j = X at hcap hc hni hne hown ⊢
  rw [RG, region_leaf env (prop := prop) hcap hown (fun t ht _ => by
    simp [Ctx.node, ordT, int, str] at ht; rcases ht with h | h | h | h <;> simp [h])]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "Token" := by
    intro l; rw [ty_view hcap]; simp [Ctx.node, Ne.symm hne, eq_comm]
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcap l o hl hc'
  leaf_tac env, hc, hty, hni
  simp [tokenS, orderedS, hov, Ctx.node, str, ordT, int, hne, Ne.symm hne]

theorem good_ext (env : Env k ts P) {f : String} {x : Extension} {j : Nat}
    (hcap : Cap k ts (U (extK k f (x, j)).1) (extK k f (x, j)).2)
    (hown : Above (k := k) (k.iri f) (extK k f (x, j)).1)
    (hc : Contained' k ts (k.iri (extK k f (x, j)).1) (k.iri f))
    (hni : ∀ s, (⟨s, vt "initial", k.iri (extK k f (x, j)).1⟩ : Triple) ∉ ts) :
    RG k ts (extK k f (x, j)).1 := by
  have hne := above_ne hown
  simp only [extK] at hcap hc hni hne hown ⊢
  generalize f ++ ":x" ++ toString j = X at hcap hc hni hne hown ⊢
  rw [RG, region_leaf env (prop := "extension") hcap hown (fun t ht _ => by
    rcases x with ⟨ns, ln, v, pos⟩
    cases v <;> simp [Ctx.node, ordT, int, str, Ctx.extensionFields] at ht <;>
      rcases ht with h | h | h | h | h | h | h <;> simp [h])]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "ExtensionNode" := by
    intro l; rw [ty_view hcap]
    rcases x with ⟨ns, ln, v, pos⟩
    cases v <;> simp [Ctx.node, Ctx.extensionFields, Ne.symm hne, eq_comm]
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcap l o hl hc'
  leaf_tac env, hc, hty, hni
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp [extensionNodeS, extensionFieldsS, orderedS, hov, Ctx.node, str, ordT, int,
    Ctx.extensionFields, hne, Ne.symm hne]

theorem good_param (env : Env k ts P) {f : String} {p : Param} {j : Nat}
    (hcap : Cap k ts (U (paramK k f (p, j)).1) (paramK k f (p, j)).2)
    (hown : Above (k := k) (k.iri f) (paramK k f (p, j)).1)
    (hc : Contained' k ts (k.iri (paramK k f (p, j)).1) (k.iri f))
    (hni : ∀ s, (⟨s, vt "initial", k.iri (paramK k f (p, j)).1⟩ : Triple) ∉ ts) :
    RG k ts (paramK k f (p, j)).1 ↔ p.expr.isSome ≠ p.location.isSome := by
  have hne := above_ne hown
  simp only [paramK] at hcap hc hni hne hown ⊢
  generalize f ++ ":p" ++ toString j = X at hcap hc hni hne hown ⊢
  rw [RG, region_leaf env (prop := "param") hcap hown (fun t ht _ => by
    rcases p with ⟨n, _ | e, _ | l⟩ <;> simp [Ctx.node, ordT, int, str, Ctx.optExpr, Ctx.expr] at ht <;>
      grind)]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "Param" := by
    intro l; rw [ty_view hcap]; simp [Ctx.node, Ne.symm hne, eq_comm]
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcap l o hl hc'
  leaf_tac env, hc, hty, hni
  simp [paramS, orderedS, hov, Ctx.node, Ctx.optExpr, Ctx.expr, str, ordT, int, hne, Ne.symm hne]
  rcases p with ⟨n, _ | e, _ | l⟩ <;> simp

theorem good_content (env : Env k ts P) {f : String} {c : Content}
    (hcap : Cap k ts (U (f ++ ":c")) (k.content (k.iri f) f c))
    (hown : Above (k := k) (k.iri f) (f ++ ":c"))
    (hc : Contained' k ts (k.iri (f ++ ":c")) (k.iri f))
    (hni : ∀ s, (⟨s, vt "initial", k.iri (f ++ ":c")⟩ : Triple) ∉ ts) :
    RG k ts (f ++ ":c") := by
  have hne := above_ne hown
  simp only [Ctx.content] at hcap
  generalize f ++ ":c" = X at hcap hc hni hne hown ⊢
  rw [RG, region_leaf env (prop := "content") hcap hown (fun t ht _ => by
    rcases c with ⟨_ | e, _ | b⟩ <;> simp [Ctx.node, optStr, str, Ctx.optExpr, Ctx.expr] at ht <;>
      grind)]
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "Content" := by
    intro l; rw [ty_view hcap]; rcases c with ⟨_ | e, _ | b⟩ <;>
      simp [Ctx.node, Ne.symm hne, eq_comm, optStr, str, Ctx.optExpr, Ctx.expr]
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) hcap l o hl hc'
  leaf_tac env, hc, hty, hni
  rcases c with ⟨_ | e, _ | b⟩ <;>
    simp [contentS, hov, Ctx.node, Ctx.optExpr, Ctx.expr, optStr, str, hne, Ne.symm hne]

/-! ## Types of children -/

section
variable (k : Ctx)
theorem ty_extKids (f : String) (xs : List Extension) :
    ∀ d ∈ extKids k f xs, (⟨k.iri d.1, rdfType, vt "ExtensionNode"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [extKids, extK, List.mem_map] at hd
  obtain ⟨_, _, rfl⟩ := hd; simp [Ctx.node]
theorem ty_tokKids (f pre prop : String) (vs : List String) :
    ∀ d ∈ tokKids k f pre prop vs, (⟨k.iri d.1, rdfType, vt "Token"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [tokKids, tokK, List.mem_map] at hd
  obtain ⟨_, _, rfl⟩ := hd; simp [Ctx.node]
theorem ty_paramKids (f : String) (ps : List Param) :
    ∀ d ∈ paramKids k f ps, (⟨k.iri d.1, rdfType, vt "Param"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [paramKids, paramK, List.mem_map] at hd
  obtain ⟨_, _, rfl⟩ := hd; simp [Ctx.node]
theorem ty_contentKids (f : String) (c : Option Content) :
    ∀ d ∈ contentKids k f c, (⟨k.iri d.1, rdfType, vt "Content"⟩ : Triple) ∈ d.2 := by
  intro d hd; cases c <;> simp [contentKids] at hd; subst hd; simp [Ctx.content, Ctx.node]
theorem ty_blockKids (f s prop : String) (i : Nat) (es : List Exec) :
    ∀ d ∈ blockKids k f s prop i es, (⟨k.iri d.1, rdfType, vt "ExecutableContent"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [blockKids, blockK, List.mem_map] at hd
  obtain ⟨⟨⟨kind, _⟩, _⟩, _, rfl⟩ := hd; simp [Ctx.exec, Ctx.node]
theorem ty_branchKids (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    ∀ d ∈ branchKids k f j bs, (⟨k.iri d.1, rdfType, vt "IfBranch"⟩ : Triple) ∈ d.2 := by
  intro d hd; simp only [branchKids, branchK, List.mem_map] at hd
  obtain ⟨_, _, rfl⟩ := hd; simp [Ctx.node]
end

theorem kid_sub {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (cx : Ctxt k ts owner prop X E D) {d : String × List Triple}
    (hd : d ∈ D) {t : Triple} (ht : t ∈ d.2) : t ∈ ts :=
  cx.cap.sub.subset (List.mem_of_mem_tail ((cx.dec.sub d hd).subset ht))

/-! ## Executable content -/

theorem forall_some_ne {α : Type} {a : Option α} {f : α → Term} :
    (∀ (y : Term) (x : α), a = some x → ¬ y = f x) ↔ a = none := by
  cases a with
  | none => simp
  | some v =>
    constructor
    · intro h; exact absurd rfl (h (f v) v rfl)
    · intro h; cases h

theorem kids_empty {D : List (String × List Triple)} :
    (∀ (y : Term) (x : String) (x1 : List Triple), (x, x1) ∈ D → ¬ k.iri x = y) ↔ D = [] := by
  constructor
  · intro h
    cases D with
    | nil => rfl
    | cons d D => exact absurd rfl (h (k.iri d.1) d.1 d.2 List.mem_cons_self)
  · rintro rfl; simp

@[simp] theorem paramKids_eq_nil {f : String} {ps : List Param} : paramKids k f ps = [] ↔ ps = [] := by
  cases ps <;> simp [paramKids]
@[simp] theorem tokKids_eq_nil {f pre prop : String} {vs : List String} :
    tokKids k f pre prop vs = [] ↔ vs = [] := by
  cases vs <;> simp [tokKids]

theorem or_none_iff {α β : Type} {a : Option α} {b : Option β} :
    (a = none ∨ b = none) ↔ (a.isSome = true → b = none) := by
  cases a <;> simp

theorem objV {owner : Term} {prop X : String} {E : List Triple} {D : List (String × List Triple)}
    (cx : Ctxt k ts owner prop X E D) {p c : String} {D' : List (String × List Triple)}
    (hD' : ∀ d ∈ D', d ∈ D) (hT : ∀ d ∈ D', (⟨k.iri d.1, rdfType, vt c⟩ : Triple) ∈ d.2)
    (hp1 : p ≠ "historyDefault") (hp2 : p ≠ "choice")
    (hmem : ∀ v, (⟨k.iri X, vt p, v⟩ : Triple) ∈ E → v ∈ D'.map (fun d => k.iri d.1)) :
    ∀ v, (⟨k.iri X, vt p, v⟩ : Triple) ∈ ts → (⟨v, rdfType, vt c⟩ : Triple) ∈ ts := by
  intro v hv
  rw [obj_view' cx.cap p v hp1 hp2] at hv
  obtain ⟨d, hd, rfl⟩ := List.mem_map.1 (hmem v hv)
  exact kid_sub cx (hD' d hd) (hT d hd)

/-- The rules of an executable content element checked at its own node. -/
def execLocal : ExecKind → Prop
  | .send s => ¬(s.event.isSome ∧ s.eventexpr.isSome) ∧ ¬(s.target.isSome ∧ s.targetexpr.isSome) ∧
      ¬(s.type.isSome ∧ s.typeexpr.isSome) ∧ ¬(s.id.isSome ∧ s.idlocation.isSome) ∧
      ¬(s.delay.isSome ∧ s.delayexpr.isSome) ∧ ¬(s.content.isSome ∧ (s.params ≠ [] ∨ s.namelist ≠ []))
  | .cancel a b => a.isSome ≠ b.isSome
  | .assign _ e c => ¬(e.isSome ∧ c.isSome)
  | .foreach _ item _ _ => item ≠ ""
  | _ => True

set_option maxHeartbeats 1000000 in
theorem good_exec_node (env : Env k ts P) {owner : Term} {prop X : String} {order : Nat}
    {kind : ExecKind} {exts : List Extension}
    (cx : Ctxt k ts owner prop X (k.exec owner prop X order (.mk kind exts))
      (execKids k X (.mk kind exts)))
    (hc : Contained' k ts (k.iri X) owner)
    (hni : ∀ s, (⟨s, vt "initial", k.iri X⟩ : Triple) ∉ ts) :
    ShapesL.Good (Graph.ofList ts) (k.iri X) ↔ execLocal kind := by
  have hne := above_ne cx.own
  have hkT : ∀ d ∈ execKids k X (.mk kind exts), ∀ t ∈ d.2, t ∈ ts := fun d hd t ht => kid_sub cx hd ht
  have hxT : ∀ d ∈ extKids k X exts, (⟨k.iri d.1, rdfType, vt "ExtensionNode"⟩ : Triple) ∈ ts :=
    fun d hd => hkT d (by simp [execKids, hd]) _ (ty_extKids k X exts d hd)
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri X) l ↔ l = "ExecutableContent" ∨ l = execClass kind := by
    intro l; rw [ty_view cx.cap]
    cases kind <;> (try rename_i sd; try rcases sd with ⟨_, _, _, _, _, _, _, _, _, _, _, _, ct⟩) <;>
      (try rcases ct with _ | ct) <;>
      simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_a _), self_branch, Ne.symm hne, eq_comm, execClass]
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have htyT : ∀ l, (⟨k.iri X, rdfType, vt l⟩ : Triple) ∈ ts ↔ l = "ExecutableContent" ∨ l = execClass kind := by
    intro l; rw [← hty l]; simp [Ty]
  have hxT' : ∀ (x : String) (y : List Triple), (x, y) ∈ extKids k X exts →
      (⟨k.iri x, rdfType, vt "ExtensionNode"⟩ : Triple) ∈ ts := fun x y h => hxT (x, y) h
  have hextV : ∀ v, (⟨k.iri X, vt "extension", v⟩ : Triple) ∈ ts →
      (⟨v, rdfType, vt "ExtensionNode"⟩ : Triple) ∈ ts := by
    intro v hv
    rw [hov _ _ (by decide) (by decide)] at hv
    have : (⟨k.iri X, vt "extension", v⟩ : Triple) ∈ k.extensions (k.iri X) X exts := by
      cases kind <;> (try rename_i sd; try rcases sd with ⟨_, _, _, _, _, _, _, _, _, _, _, _, ct⟩) <;>
        (try rcases ct with _ | ct) <;>
        simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
          self_block _ _ _ _ (ch_a _), self_branch, Ne.symm hne] at hv <;>
        simp [self_ext, hv]
    rw [self_ext k X exts (by simp)] at this
    obtain ⟨-, hm⟩ := this
    obtain ⟨d, hd, rfl⟩ := List.mem_map.1 hm
    exact hxT d hd
  have hordV : ∀ v, (⟨k.iri X, vt "order", v⟩ : Triple) ∈ ts ↔ v = intLit order := by
    intro v
    rw [hov _ _ (by decide) (by decide)]
    cases kind <;> (try rename_i sd; try rcases sd with ⟨_, _, _, _, _, _, _, _, _, _, _, _, ct⟩) <;>
      (try rcases ct with _ | ct) <;>
      simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
        self_block _ _ _ _ (ch_a _), self_branch, Ne.symm hne]
  have hexS : Conforms (Graph.ofList ts) execS (k.iri X) := by
    simp only [execS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, holds_xone,
      holds_node, holds_property, vn_none, forall_eq, conforms_mk, nodeS, cls, holds_class,
      instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq, mem_ofList', htyT, orderedS, pr, propS, ShapesL.P, vn_some,
      valueNode_pred, iri_vi, hordV, clsP, dt, minC, maxC, holds_test, holds_minCount1,
      holds_maxCount1, forall_eq, exists_eq, ok_dt_int, ok_minInclusive, implies_true, true_and]
    refine ⟨?_, fun y z hy hz => hy.trans hz.symm, fun v hv => hextV v hv⟩
    cases kind <;> simp [execClass, instanceOf_iff (Graph.ofList ts) env.noSub, htyT]
  have hself : ∀ l o, l ≠ "historyDefault" → l ≠ "choice" →
      ((⟨k.iri X, vt l, o⟩ : Triple) ∈ ts ↔ (⟨k.iri X, vt l, o⟩ : Triple) ∈ k.exec owner prop X order (.mk kind exts)) := hov
  rw [good_iff _ env.noSub]
  cases kind <;>
    simp only [hty, containedClasses, mem_ofList', hni, exists_false, false_implies, true_implies,
      String.reduceEq, List.mem_cons, exists_eq_or_imp, or_false, false_or, implies_true,
      true_and, and_true, hc.ok env, List.not_mem_nil, exists_eq_left, or_self, iff_true, execClass,
      or_true, true_or, hexS]
  case raise ev =>
    simp [raiseS, hov, Ctx.exec, Ctx.execKind, self_ext, Ne.symm hne, execLocal]
  case cancel a b =>
    rcases a with _ | a <;> rcases b with _ | b <;>
      simp [cancelS, hov, Ctx.exec, Ctx.execKind, self_ext, Ne.symm hne, execLocal]
  case log a b =>
    rcases a with _ | a <;> rcases b with _ | b <;>
      simp [logS, hov, Ctx.exec, Ctx.execKind, self_ext, Ne.symm hne, execLocal]
  case assign l a b =>
    rcases a with _ | a <;> rcases b with _ | b <;>
      simp [assignS, hov, Ctx.exec, Ctx.execKind, self_ext, Ne.symm hne, execLocal]
  case script sc =>
    rcases sc with ⟨_ | a, _ | b⟩ <;>
      simp [scriptS, hov, Ctx.exec, Ctx.execKind, self_ext, Ne.symm hne, execLocal, Ctx.scriptFields]
  case extension x =>
    rcases x with ⟨ns, ln, v, pos⟩
    cases v <;> simp [extensionContentS, extensionFieldsS, hov, Ctx.exec, Ctx.execKind, self_ext,
      Ne.symm hne, execLocal]
  case if_ bs =>
    have hbrV := objV cx (p := "branch") (c := "IfBranch") (D' := branchKids k X 0 bs)
      (fun d hd => by simp [execKids, execKindKids, hd]) (ty_branchKids k X 0 bs) (by decide) (by decide)
      (fun v hv => by
        simp [Ctx.exec, Ctx.execKind, self_ext, self_branch, Ne.symm hne] at hv; simpa using hv)
    simp (config := {contextual := true}) [ifS, hbrV, instanceOf_iff (Graph.ofList ts) env.noSub, execLocal]
  case foreach arr item idx body =>
    have hacV := objV cx (p := "action") (c := "ExecutableContent") (D' := blockKids k X ":a" "action" 0 body)
      (fun d hd => by simp [execKids, execKindKids, hd]) (ty_blockKids k X ":a" "action" 0 body)
      (by decide) (by decide)
      (fun v hv => by
        simp [Ctx.exec, Ctx.execKind, self_ext, self_block _ _ _ _ (ch_a _), Ne.symm hne] at hv; simpa using hv)
    rcases idx with _ | idx <;>
    simp (config := {contextual := true}) [foreachS, hacV, instanceOf_iff (Graph.ofList ts) env.noSub,
      execLocal, hov, Ctx.exec, Ctx.execKind, self_ext, self_block _ _ _ _ (ch_a _), Ne.symm hne]
  case send sd =>
    rcases sd with ⟨a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, nl, ps, ct⟩
    have hnlV := objV cx (p := "namelist") (c := "Token") (D' := tokKids k X (X ++ ":nl") "namelist" nl)
      (fun d hd => by simp [execKids, execKindKids, hd]) (ty_tokKids k X _ _ nl) (by decide) (by decide)
      (fun v hv => by
        rcases ct with _ | ct <;>
        simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
          Ne.symm hne] at hv <;> simpa using hv)
    have hpaV := objV cx (p := "param") (c := "Param") (D' := paramKids k X ps)
      (fun d hd => by simp [execKids, execKindKids, hd]) (ty_paramKids k X ps) (by decide) (by decide)
      (fun v hv => by
        rcases ct with _ | ct <;>
        simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
          Ne.symm hne] at hv <;> simpa using hv)
    have hcoV := objV cx (p := "content") (c := "Content") (D' := contentKids k X ct)
      (fun d hd => by simp [execKids, execKindKids, hd]) (ty_contentKids k X ct) (by decide) (by decide)
      (fun v hv => by
        rcases ct with _ | ct <;>
        simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
          Ne.symm hne, contentKids] at hv <;> simpa [contentKids] using hv)
    rcases ct with _ | ct <;>
      simp (config := {contextual := true}) [sendS, hnlV, hpaV, hcoV,
        instanceOf_iff (Graph.ofList ts) env.noSub, execLocal, hov, Ctx.exec, Ctx.execKind, self_ext,
        self_tok _ _ _ _ (ch_nl _), self_param, self_content, Ne.symm hne, forall_some_ne, or_none_iff, forall_and, kids_empty]

end Vertep.Rdf.Equiv

