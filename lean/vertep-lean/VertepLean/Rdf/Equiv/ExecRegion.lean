import VertepLean.Rdf.Equiv.ExecNodes
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# The node shapes on executable content, by region

`reg_exec`: the node shapes hold in the region of an executable content element exactly when the
element satisfies `ExecOk`; with `reg_block` and `reg_branches` for nested blocks and `<if>`
branches, by structural recursion.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

variable {k : Ctx} {ts : List Triple} {P : List (Option State × State)}

/-- The node shapes at an `<if>` branch node: a branch without a condition is the last one and
not the first. -/
theorem good_branch_node (env : Env k ts P) {F : String} {c : Option Expr} {body : List Exec}
    {j n : Nat}
    (cx : Ctxt k ts (k.iri F) "branch" (branchK k F ((c, body), j)).1 (branchK k F ((c, body), j)).2
      (blockKids k (branchK k F ((c, body), j)).1 ":a" "action" 0 body))
    (hc : Contained' k ts (k.iri (branchK k F ((c, body), j)).1) (k.iri F))
    (hni : ∀ s, (⟨s, vt "initial", k.iri (branchK k F ((c, body), j)).1⟩ : Triple) ∉ ts)
    (hsib : ∀ o, (∃ v, (⟨k.iri F, vt "branch", v⟩ : Triple) ∈ ts ∧ (⟨v, vt "order", o⟩ : Triple) ∈ ts) ↔
      ∃ j' < n, o = intLit j') :
    ShapesL.Good (Graph.ofList ts) (k.iri (branchK k F ((c, body), j)).1) ↔
      (c.isSome ∨ (0 < j ∧ ∀ j' < n, j' ≤ j)) := by
  have hlink : (⟨k.iri F, vt "branch", k.iri (branchK k F ((c, body), j)).1⟩ : Triple) ∈ ts :=
    cx.cap.sub.subset (by simp [branchK, Ctx.node])
  simp only [branchK] at cx hc hni hlink ⊢
  generalize F ++ ":b" ++ toString j = B at cx hc hni hlink ⊢
  have hne := above_ne cx.own
  have hacV := objV cx (p := "action") (c := "ExecutableContent")
    (D' := blockKids k B ":a" "action" 0 body)
    (fun d hd => hd) (ty_blockKids k _ ":a" "action" 0 body) (by decide) (by decide)
    (fun v hv => by
      simp [Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne] at hv; simpa using hv)
  have hov := fun l o hl hc' => obj_view' (k := k) (ts := ts) cx.cap l o hl hc'
  have hup : ∀ z, (⟨z, vt "branch", k.iri B⟩ : Triple) ∈ ts ↔ z = k.iri F := by
    intro z
    constructor
    · intro h; exact (hc.conts z).1 ⟨"branch", by simp [containers], h⟩
    · rintro rfl; exact hlink
  have hty : ∀ l, Ty (Graph.ofList ts) (k.iri B) l ↔ l = "IfBranch" := by
    intro l; rw [ty_view cx.cap]
    simp [Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne, eq_comm]
  have hordB : ∀ o, (⟨k.iri B, vt "order", o⟩ : Triple) ∈ ts ↔ o = intLit j := by
    intro o; rw [hov _ _ (by decide) (by decide)]
    simp [Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne]
  have hpath : ∀ o, (∃ z, (⟨z, vt "branch", k.iri B⟩ : Triple) ∈ ts ∧ ∃ w,
      (⟨z, vt "branch", w⟩ : Triple) ∈ ts ∧ (⟨w, vt "order", o⟩ : Triple) ∈ ts) ↔ ∃ j' < n, o = intLit j' := by
    intro o; rw [← hsib o]; simp only [hup, exists_eq_left]
  leaf_tac env, hc, hty, hni
  simp only [ifBranchS, top, conforms_mk, compsHold_cons, compsHold_nil, and_true, holds_or,
    holds_node, holds_property, vn_none, forall_eq, someConforms_cons, someConforms_nil, or_false,
    some1, propS, nodeS, conforms_mk, holds_minCount1, vn_some, ShapesL.P, ShapesL.I, valueNode_pred,
    valueNode_inv, valueNode_seq_cons, valueNode_seq_nil, iri_vi, mem_ofList', holds_and,
    allConform_cons, allConform_nil, holds_test, holds_lessThanOrEquals, hordB, forall_eq,
    exists_eq_right, orderedS, pr, dt, minC, maxC, optLitP, lit, clsP, cls, holds_maxCount1,
    holds_class, instanceOf_iff (Graph.ofList ts) env.noSub, rdf_type_eq, hacV, implies_true,
    ok_dt_int, ok_minInclusive, ok_minExclusive, exists_eq, true_and]
  have hP : ∀ v, (∃ z, (⟨z, vt "branch", k.iri B⟩ : Triple) ∈ ts ∧ ∃ z1, (⟨z, vt "branch", z1⟩ : Triple) ∈ ts ∧
      ∃ z2, (⟨z1, vt "order", z2⟩ : Triple) ∈ ts ∧ v = z2) ↔ ∃ j' < n, v = intLit j' := by
    intro v; rw [← hpath v]
    constructor
    · rintro ⟨z, h1, z1, h2, z2, h3, rfl⟩; exact ⟨z, h1, z1, h2, h3⟩
    · rintro ⟨z, h1, z1, h2, h3⟩; exact ⟨z, h1, z1, h2, v, h3, rfl⟩
  simp only [hP]
  rcases c with _ | c <;>
    simp (config := {contextual := true}) [hov, Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne,
      hacV, lessB_int]
  intro _
  exact ⟨fun h j' hj' => h _ _ j' hj' rfl rfl, fun h _ _ x hx _ _ => h x hx⟩

/-! ## Leaf children of a node -/

section
variable (env : Env k ts P) {o : Term} {prop F : String} {E : List Triple}
  {D : List (String × List Triple)} (cx : Ctxt k ts o prop F E D) (hcF : Contained' k ts (k.iri F) o)
  (hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts)
include env cx hcF hnoF

theorem rg_exts {xs : List Extension} (hsub : ∀ d ∈ extKids k F xs, d ∈ D) :
    ∀ d ∈ extKids k F xs, RG k ts d.1 := by
  intro d hd
  obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_ext k F xs) hsub (by simp [containers]) hd
  have hni := noInit_of hc hnoF
  simp only [extKids, List.mem_map] at hd
  obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
  exact good_ext env hcap hown hc hni

theorem rg_toks {pre p : String} (hpre : ∀ j : Nat, Ch F (pre ++ toString j)) (hp : p ∈ containers)
    {vs : List String} (hsub : ∀ d ∈ tokKids k F pre p vs, d ∈ D) :
    ∀ d ∈ tokKids k F pre p vs, RG k ts d.1 := by
  intro d hd
  obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_tok k F pre p vs) hsub hp hd
  have hni := noInit_of hc hnoF
  simp only [tokKids, List.mem_map] at hd
  obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
  exact good_tok env hcap hown hc hni

theorem rg_params {ps : List Param} (hsub : ∀ d ∈ paramKids k F ps, d ∈ D) :
    (∀ d ∈ paramKids k F ps, RG k ts d.1) ↔ ParamsOk ps := by
  have key : ∀ x ∈ ps.zipIdx, RG k ts (paramK k F x).1 ↔ x.1.expr.isSome ≠ x.1.location.isSome := by
    intro x hx
    have hd : paramK k F x ∈ paramKids k F ps := List.mem_map.2 ⟨x, hx, rfl⟩
    obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_param k F ps) hsub (by simp [containers]) hd
    exact good_param env hcap hown hc (noInit_of hc hnoF)
  simp only [paramKids, List.forall_mem_map, ParamsOk]
  constructor
  · intro h p hp
    obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hp
    exact (key (ps[i], i) (List.mem_zipIdx_iff_getElem?.2 (by simp [hi]))).1
      (h _ (List.mem_zipIdx_iff_getElem?.2 (by simp [hi])))
  · intro h x hx
    exact (key x hx).2 (h x.1 (fst_mem_zipIdx hx))

theorem rg_content {c : Option Content} (hsub : ∀ d ∈ contentKids k F c, d ∈ D) :
    ∀ d ∈ contentKids k F c, RG k ts d.1 := by
  intro d hd
  obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_content k F c) hsub (by simp [containers]) hd
  cases c with
  | none => simp [contentKids] at hd
  | some c =>
    simp only [contentKids, List.mem_singleton] at hd; subst hd
    exact good_content env hcap hown hc (noInit_of hc hnoF)

end

/-! ## Branch conditions -/

theorem branch_conds (bs : List (Option Expr × List Exec)) :
    (∀ p ∈ bs.zipIdx, p.1.1.isSome = true ∨ (0 < p.2 ∧ ∀ j' < bs.length, j' ≤ p.2)) ↔
      ((∀ b ∈ bs.dropLast, b.1.isSome = true) ∧ (∀ b, bs.head? = some b → b.1.isSome = true)) := by
  constructor
  · intro h
    refine ⟨fun b hb => ?_, fun b hb => ?_⟩
    · obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hb
      have hi' : i < bs.length - 1 := by simpa using hi
      have hmem : (bs[i], i) ∈ bs.zipIdx := List.mem_zipIdx_iff_getElem?.2 (by simp)
      rcases h _ hmem with h1 | ⟨_, h2⟩
      · simpa [List.getElem_dropLast] using h1
      · have := h2 (bs.length - 1) (by omega); omega
    · cases bs with
      | nil => simp at hb
      | cons b0 bs =>
        simp only [List.head?_cons, Option.some.injEq] at hb; subst hb
        rcases h (b0, 0) (by simp) with h1 | ⟨h2, _⟩
        · exact h1
        · exact absurd h2 (Nat.lt_irrefl 0)
  · rintro ⟨h1, h2⟩ ⟨b, i⟩ hp
    have hm := List.mem_zipIdx_iff_getElem?.1 hp
    simp only [List.getElem?_eq_some_iff] at hm
    obtain ⟨hi, rfl⟩ := hm
    simp only
    by_cases h0 : i = 0
    · subst h0; left
      exact h2 _ (by cases bs <;> simp_all)
    by_cases hl : i < bs.length - 1
    · left
      exact h1 _ (List.mem_iff_getElem.2 ⟨i, by simp; omega, by simp [List.getElem_dropLast]⟩)
    · right; exact ⟨by omega, fun j' hj' => by omega⟩

/-! ## The recursion -/

theorem exec_noInit {owner : Term} {prop X : String} {order : Nat} {e : Exec}
    (cx : Ctxt k ts owner prop X (k.exec owner prop X order e) (execKids k X e)) :
    ∀ v, (⟨k.iri X, vt "initial", v⟩ : Triple) ∉ ts := by
  intro v hv
  have hne := above_ne cx.own
  rw [obj_view' cx.cap _ _ (by decide) (by decide)] at hv
  rcases e with ⟨kind, exts⟩
  cases kind <;> (try rename_i sd; try rcases sd with ⟨_, _, _, _, _, _, _, _, _, _, _, _, ct⟩) <;>
    (try rcases ct with _ | ct) <;>
    simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
      self_block _ _ _ _ (ch_a _), self_branch, Ne.symm hne] at hv

theorem branch_noInit {F : String} {c : Option Expr} {body : List Exec} {j : Nat}
    (cx : Ctxt k ts (k.iri F) "branch" (branchK k F ((c, body), j)).1 (branchK k F ((c, body), j)).2
      (blockKids k (branchK k F ((c, body), j)).1 ":a" "action" 0 body)) :
    ∀ v, (⟨k.iri (branchK k F ((c, body), j)).1, vt "initial", v⟩ : Triple) ∉ ts := by
  intro v hv
  have hne := above_ne cx.own
  rw [obj_view' cx.cap _ _ (by decide) (by decide)] at hv
  simp [branchK, Ctx.node, self_block _ _ _ _ (ch_a _), Ne.symm hne] at hv

/-- The orders of the branches of an `<if>`. -/
theorem if_sib (env : Env k ts P) {owner : Term} {prop X : String} {order : Nat}
    {bs : List (Option Expr × List Exec)} {exts : List Extension}
    (cx : Ctxt k ts owner prop X (k.exec owner prop X order (.mk (.if_ bs) exts))
      (execKids k X (.mk (.if_ bs) exts))) :
    ∀ o, (∃ v, (⟨k.iri X, vt "branch", v⟩ : Triple) ∈ ts ∧ (⟨v, vt "order", o⟩ : Triple) ∈ ts) ↔
      ∃ j' < bs.length, o = intLit j' := by
  have hne := above_ne cx.own
  have hbr : ∀ v, (⟨k.iri X, vt "branch", v⟩ : Triple) ∈ ts ↔
      v ∈ (branchKids k X 0 bs).map (fun d => k.iri d.1) := by
    intro v; rw [obj_view' cx.cap _ _ (by decide) (by decide)]
    simp [Ctx.exec, Ctx.execKind, self_ext, self_branch, Ne.symm hne]
  have hord : ∀ d ∈ (bs.zipIdx 0), ∀ o, (⟨k.iri (branchK k X d).1, vt "order", o⟩ : Triple) ∈ ts ↔
      o = intLit d.2 := by
    intro d hd o
    have hdk : branchK k X d ∈ execKids k X (.mk (.if_ bs) exts) := by
      simp only [execKids, execKindKids, branchKids]
      exact List.mem_append_left _ (List.mem_map.2 ⟨d, hd, rfl⟩)
    have hcap := cx.kid hdk
    have hneB := above_ne (k := k) (above_of_ch (cx.dec.ch _ hdk))
    rw [obj_view' hcap _ _ (by decide) (by decide)]
    rcases d with ⟨⟨c, body⟩, j⟩
    simp only [branchK] at hneB ⊢
    rcases c <;> simp [Ctx.node, self_block _ _ _ _ (ch_a _), hneB, Ne.symm hneB]
  intro o
  simp only [hbr, branchKids, List.map_map, List.mem_map, Function.comp_def]
  constructor
  · rintro ⟨v, ⟨d, hd, rfl⟩, ho⟩
    rw [hord d hd] at ho
    exact ⟨d.2, by have := List.mem_zipIdx hd; omega, ho⟩
  · rintro ⟨j, hj, rfl⟩
    refine ⟨_, ⟨(bs[j], j), List.mem_zipIdx_iff_getElem?.2 (by simp), rfl⟩, ?_⟩
    exact (hord _ (List.mem_zipIdx_iff_getElem?.2 (by simp)) _).2 rfl

mutual
theorem reg_exec (env : Env k ts P) (owner : Term) (prop X : String) (order : Nat) :
    ∀ (e : Exec), Ctxt k ts owner prop X (k.exec owner prop X order e) (execKids k X e) →
      Contained' k ts (k.iri X) owner → (∀ s, (⟨s, vt "initial", k.iri X⟩ : Triple) ∉ ts) →
      (RG k ts X ↔ ExecOk e)
  | .mk kind exts, cx, hc, hni => by
    have hnoX := exec_noInit cx
    have hx := rg_exts env cx hc hnoX (xs := exts) (fun d hd => by simp [execKids, hd])
    rw [RG, region_iff env cx, good_exec_node env cx hc hni]
    have hsplit : (∀ d ∈ execKids k X (.mk kind exts), ∀ g, U d.1 g →
        ShapesL.Good (Graph.ofList ts) (k.iri g)) ↔ ∀ d ∈ execKindKids k X kind, RG k ts d.1 := by
      simp only [execKids, List.mem_append]
      constructor
      · intro h d hd; exact h d (.inl hd)
      · intro h d hd
        rcases hd with hd | hd
        · exact h d hd
        · exact hx d hd
    rw [hsplit]
    cases kind with
    | raise _ => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | cancel a b => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | log _ _ => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | assign _ a b => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | script _ => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | extension _ => simp [execKindKids, ExecOk, ExecKindOk, execLocal]
    | send sd =>
      have hp := rg_params env cx hc hnoX (ps := sd.params)
        (fun d hd => by simp [execKids, execKindKids, hd])
      have ht := rg_toks env cx hc hnoX (pre := X ++ ":nl") (p := "namelist") (ch_nl X)
        (by simp [containers]) (vs := sd.namelist) (fun d hd => by simp [execKids, execKindKids, hd])
      have hco := rg_content env cx hc hnoX (c := sd.content)
        (fun d hd => by simp [execKids, execKindKids, hd])
      have hsp : (∀ d ∈ execKindKids k X (.send sd), RG k ts d.1) ↔ ParamsOk sd.params := by
        simp only [execKindKids, List.mem_append]
        rw [← hp]
        constructor
        · intro h d hd; exact h d (.inl (.inr hd))
        · intro h d hd
          rcases hd with (hd | hd) | hd
          · exact ht d hd
          · exact h d hd
          · exact hco d hd
      rw [hsp]
      simp only [ExecOk, ExecKindOk, execLocal, SendOk]
      constructor
      · rintro ⟨⟨a, b, c, d, e, f⟩, g⟩; exact ⟨a, b, c, d, e, f, g⟩
      · rintro ⟨a, b, c, d, e, f, g⟩; exact ⟨⟨a, b, c, d, e, f⟩, g⟩
    | if_ bs =>
      have hsib := if_sib env cx
      have hb := reg_branches env cx hc hnoX bs.length hsib 0 bs
        (fun d hd => by simp [execKids, execKindKids, hd])
      simp only [execKindKids]
      rw [hb, branch_conds]
      simp [ExecOk, ExecKindOk, execLocal, and_assoc]
    | foreach arr item idx body =>
      have hb := reg_block env cx hc hnoX ":a" "action" (by simp [containers]) 0 body
        (fun d hd => by simp [execKids, execKindKids, hd])
      simp only [execKindKids]
      rw [hb]
      simp [ExecOk, ExecKindOk, execLocal]
theorem reg_block (env : Env k ts P) {oF : Term} {propF F : String} {EF : List Triple}
    {DF : List (String × List Triple)} (cx : Ctxt k ts oF propF F EF DF)
    (hcF : Contained' k ts (k.iri F) oF) (hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts)
    (s p : String) (hp : p ∈ containers) :
    ∀ (i : Nat) (es : List Exec), (∀ d ∈ blockKids k F s p i es, d ∈ DF) →
      ((∀ d ∈ blockKids k F s p i es, RG k ts d.1) ↔ BlockOk es)
  | _, [], _ => by simp [blockKids, BlockOk]
  | i, e :: es, hsub => by
    have hd0 : blockK k F s p (e, i) ∈ blockKids k F s p i (e :: es) := by simp [blockKids]
    obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_block k F s p i (e :: es)) hsub hp hd0
    have cxe : Ctxt k ts (k.iri F) p (F ++ s ++ toString i)
        (k.exec (k.iri F) p (F ++ s ++ toString i) i e) (execKids k (F ++ s ++ toString i) e) :=
      ⟨hcap, decomp_exec k _ _ _ _ e, hown⟩
    have ih1 := reg_exec env (k.iri F) p (F ++ s ++ toString i) i e cxe hc (noInit_of hc hnoF)
    have ih2 := reg_block env cx hcF hnoF s p hp (i + 1) es (fun d hd => hsub d (by
      simp only [blockKids, List.zipIdx_cons, List.map_cons, List.mem_cons]; exact .inr hd))
    have heq : blockKids k F s p i (e :: es) = blockK k F s p (e, i) :: blockKids k F s p (i + 1) es := by
      simp [blockKids]
    rw [heq, List.forall_mem_cons, ih2, BlockOk]
    simp only [blockK] at ih1 ⊢
    rw [ih1]
theorem reg_branches (env : Env k ts P) {oF : Term} {propF F : String} {EF : List Triple}
    {DF : List (String × List Triple)} (cx : Ctxt k ts oF propF F EF DF)
    (hcF : Contained' k ts (k.iri F) oF) (hnoF : ∀ v, (⟨k.iri F, vt "initial", v⟩ : Triple) ∉ ts)
    (n : Nat) (hsib : ∀ o, (∃ v, (⟨k.iri F, vt "branch", v⟩ : Triple) ∈ ts ∧
      (⟨v, vt "order", o⟩ : Triple) ∈ ts) ↔ ∃ j' < n, o = intLit j') :
    ∀ (j : Nat) (bs : List (Option Expr × List Exec)), (∀ d ∈ branchKids k F j bs, d ∈ DF) →
      ((∀ d ∈ branchKids k F j bs, RG k ts d.1) ↔
        (∀ p ∈ bs.zipIdx j, p.1.1.isSome = true ∨ (0 < p.2 ∧ ∀ j' < n, j' ≤ p.2)) ∧ BranchesOk bs)
  | _, [], _ => by simp [branchKids, BranchesOk]
  | j, (c, body) :: bs, hsub => by
    have hd0 : branchK k F ((c, body), j) ∈ branchKids k F j ((c, body) :: bs) := by simp [branchKids]
    obtain ⟨hcap, hc, hown⟩ := kid_ctx env cx hcF (kidsL_branch k F j ((c, body) :: bs)) hsub
      (by simp [containers]) hd0
    have cxb : Ctxt k ts (k.iri F) "branch" (branchK k F ((c, body), j)).1 (branchK k F ((c, body), j)).2
        (blockKids k (branchK k F ((c, body), j)).1 ":a" "action" 0 body) :=
      ⟨hcap, decomp_branch k F ((c, body), j), hown⟩
    have hnoB := branch_noInit cxb
    have hg := good_branch_node env (n := n) cxb hc (noInit_of hc hnoF) hsib
    have hbody := reg_block env cxb hc hnoB ":a" "action" (by simp [containers]) 0 body (fun d hd => hd)
    have ih := reg_branches env cx hcF hnoF n hsib (j + 1) bs (fun d hd => hsub d (by
      simp only [branchKids, List.zipIdx_cons, List.map_cons, List.mem_cons]; exact .inr hd))
    have heq : branchKids k F j ((c, body) :: bs) = branchK k F ((c, body), j) :: branchKids k F (j + 1) bs := by
      simp [branchKids]
    simp only [RG] at hbody
    rw [heq, List.forall_mem_cons, ih, RG, region_iff env cxb, hg, hbody]
    simp only [List.zipIdx_cons, List.forall_mem_cons, BranchesOk]
    constructor
    · rintro ⟨⟨a, b⟩, c', d⟩; exact ⟨⟨a, c'⟩, b, d⟩
    · rintro ⟨⟨a, c'⟩, b, d⟩; exact ⟨⟨a, b⟩, c', d⟩
end

end Vertep.Rdf.Equiv

