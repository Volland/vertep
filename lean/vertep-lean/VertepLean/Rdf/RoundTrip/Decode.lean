import VertepLean.Rdf.RoundTrip.Kids
set_option linter.unusedSimpArgs false

/-!
# RDF round trip: decoding executable content and collections

Given capture (`Cap`) of a node's emission, the decoder returns the IR the node was projected
from: extensions, tokens, parameters, content, and executable content with its nested blocks and
`if` branches (`dec_exec`, by structural recursion with the fuel bounded by emission length).
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

variable {k : Ctx} {ts : List Triple}

/-- Decoding a collection: the members are the children, in order. -/
theorem ordered_kids {s : Term} {prop : String} {α : Type} (xs : List α)
    (K : α × Nat → String × List Triple)
    (hself : Lk ts s prop ((xs.zipIdx.map K).map (fun d => k.iri d.1)))
    (hnd : ((xs.zipIdx.map K).map Prod.fst).Nodup)
    (hord : ∀ p ∈ xs.zipIdx, Lk ts (k.iri (K p).1) "order" [intLit p.2]) :
    Decode.ordered (Index.of ts) s prop = .ok ((xs.zipIdx.map K).map (fun d => k.iri d.1)) := by
  apply ordered_eq hself
  · have hnd' : ((xs.zipIdx.map K).map (fun d => d.1)).Nodup := hnd
    rw [List.Nodup, List.pairwise_map] at hnd' ⊢
    exact hnd'.imp (fun h he => h ((iri_inj k).1 he))
  · intro m hm
    simp only [List.mem_map] at hm
    obtain ⟨d, _, rfl⟩ := hm
    exact ⟨_, rfl⟩
  · intro j hj
    simp only [List.length_map, List.length_zipIdx] at hj
    have hp : (xs[j], j) ∈ xs.zipIdx := by
      rw [List.mem_zipIdx_iff_getElem?]; simp [hj]
    simpa [List.getElem_map, List.getElem_zipIdx] using hord _ hp

theorem mapM_kids {α β : Type} (dec : Term → D β) (K : α × Nat → String × List Triple)
    (val : α → β) :
    ∀ (xs : List α) (i : Nat), (∀ p ∈ xs.zipIdx i, dec (k.iri (K p).1) = .ok (val p.1)) →
      (((xs.zipIdx i).map K).map (fun d => k.iri d.1)).mapM dec = .ok (xs.map val)
  | [], _, _ => rfl
  | x :: xs, i, h => by
    have h1 := h (x, i) (by simp)
    have h2 := mapM_kids dec K val xs (i + 1) (fun p hp => h p (by simp [hp]))
    simp only [List.zipIdx_cons, List.map_cons, List.mapM_cons, h1, h2]
    rfl


syntax "lk_tac" term : tactic
macro_rules
  | `(tactic| lk_tac $h) => `(tactic| (intro o; rw [($h).lookup (U_refl _) (by simp)]; simp [strLit, intLit, boolLit]))

theorem dec_ext {b f : String} {i : Nat} {x : Extension}
    (h : Cap k ts (U f) (k.node (k.iri b) "extension" f ["ExtensionNode"] ++ ordT (k.iri f) i ++
      Ctx.extensionFields (k.iri f) x)) :
    Decode.extensionFields (Index.of ts) (k.iri f) = .ok x ∧ Lk ts (k.iri f) "order" [intLit i] := by
  refine ⟨?_, by lk_tac h⟩
  rcases x with ⟨ns, ln, v, pos⟩
  have h1 : Decode.string (Index.of ts) (k.iri f) "namespace" = .ok ns := string_eq (by lk_tac h)
  have h2 : Decode.string (Index.of ts) (k.iri f) "localName" = .ok ln := string_eq (by lk_tac h)
  have h3 : Decode.integer (Index.of ts) (k.iri f) "position" = .ok pos := integer_eq (by lk_tac h)
  rcases v with a | a
  ·
    have h4 : Decode.optString (Index.of ts) (k.iri f) "attributeValue" = .ok (some a) :=
      optString_eq (by lk_tac h)
    have h5 : Decode.optString (Index.of ts) (k.iri f) "elementXml" = .ok none :=
      optString_eq (by lk_tac h)
    simp only [Decode.extensionFields, h1, h2, h3, h4, h5, bind, Except.bind, pure, Except.pure]
  ·
    have h4 : Decode.optString (Index.of ts) (k.iri f) "attributeValue" = .ok none :=
      optString_eq (by lk_tac h)
    have h5 : Decode.optString (Index.of ts) (k.iri f) "elementXml" = .ok (some a) :=
      optString_eq (by lk_tac h)
    simp only [Decode.extensionFields, h1, h2, h3, h4, h5, bind, Except.bind, pure, Except.pure]


/-- The capture context of a node at `X`: its emission is captured and decomposed. -/
structure Ctxt (k : Ctx) (ts : List Triple) (owner : Term) (prop X : String) (E : List Triple)
    (D : List (String × List Triple)) : Prop where
  cap : Cap k ts (U X) E
  dec : Decomp k owner prop X E D
  own : Above (k := k) owner X

theorem Ctxt.kid {owner : Term} {prop X : String} {E : List Triple} {D : List (String × List Triple)}
    (c : Ctxt k ts owner prop X E D) {d : String × List Triple} (hd : d ∈ D) :
    Cap k ts (U d.1) d.2 := c.dec.cap c.cap c.own hd

theorem nd_kids {α : Type} (K : α × Nat → String × List Triple)
    (hK : ∀ p q, (K p).1 = (K q).1 → p.2 = q.2) (xs : List α) (i : Nat) :
    (((xs.zipIdx i).map K).map Prod.fst).Nodup := by
  rw [List.map_map]
  exact nodup_zipIdx_map' _ (fun p q h => hK p q h) xs i

theorem dec_extensions {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (xs : List Extension)
    (hsub : ∀ d ∈ extKids k X xs, d ∈ D)
    (hself : Lk ts (k.iri X) "extension" ((extKids k X xs).map (fun d => k.iri d.1))) :
    Decode.extensions (Index.of ts) (k.iri X) = .ok xs := by
  have hcap : ∀ p ∈ xs.zipIdx, _ := fun p hp => dec_ext (c.kid (hsub _ (List.mem_map_of_mem hp)))
  unfold Decode.extensions
  rw [ordered_kids xs (extK k X) hself
    (nd_kids _ (fun p q h => frag_inj _ _ _ _ h) xs 0) (fun p hp => (hcap p hp).2)]
  simp only [bind, Except.bind]
  have := mapM_kids (k := k) (Decode.extensionFields (Index.of ts)) (extK k X) id xs 0
    (fun p hp => (hcap p hp).1)
  simpa using this


theorem dec_tokens {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (pre p : String)
    (hpre : ∀ a b : Nat, pre ++ toString a = pre ++ toString b → a = b)
    (hne : ∀ j : Nat, pre ++ Nat.repr j ≠ X) (vs : List String)
    (hsub : ∀ d ∈ tokKids k X pre p vs, d ∈ D)
    (hself : Lk ts (k.iri X) p ((tokKids k X pre p vs).map (fun d => k.iri d.1))) :
    Decode.tokens (Index.of ts) (k.iri X) p = .ok vs := by
  have hcap : ∀ q ∈ vs.zipIdx, Decode.string (Index.of ts) (k.iri (tokK k X pre p q).1) "value" =
      .ok q.1 ∧ Lk ts (k.iri (tokK k X pre p q).1) "order" [intLit q.2] := fun q hq => by
    have h := c.kid (hsub _ (List.mem_map_of_mem hq))
    simp only [tokK] at h ⊢
    have hn := hne q.2
    exact ⟨string_eq (by intro o; rw [h.lookup (U_refl _) (by simp)]; simp [strLit, hn]),
      by intro o; rw [h.lookup (U_refl _) (by simp)]; simp [intLit, hn]⟩
  unfold Decode.tokens
  rw [ordered_kids vs (tokK k X pre p) hself (nd_kids _ (fun q r h => hpre _ _ h) vs 0)
    (fun q hq => (hcap q hq).2)]
  simp only [bind, Except.bind]
  have := mapM_kids (k := k) (fun t => Decode.string (Index.of ts) t "value") (tokK k X pre p) id
    vs 0 (fun q hq => (hcap q hq).1)
  simpa using this

theorem dec_params {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (ps : List Param)
    (hsub : ∀ d ∈ paramKids k X ps, d ∈ D)
    (hself : Lk ts (k.iri X) "param" ((paramKids k X ps).map (fun d => k.iri d.1))) :
    Decode.params (Index.of ts) k.exprDt (k.iri X) = .ok ps := by
  have hcap : ∀ q ∈ ps.zipIdx, Cap k ts (U (paramK k X q).1) (paramK k X q).2 :=
    fun q hq => c.kid (hsub _ (List.mem_map_of_mem hq))
  unfold Decode.params
  rw [ordered_kids ps (paramK k X) hself (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) ps 0)
    (fun q hq => by have h := hcap q hq; simp only [paramK] at h ⊢; lk_tac h)]
  simp only [bind, Except.bind]
  refine (mapM_kids (k := k) _ (paramK k X) id ps 0 ?_).trans (by simp)
  intro q hq
  have h := hcap q hq
  simp only [paramK] at h ⊢
  have h1 : Decode.string (Index.of ts) (k.iri (X ++ ":p" ++ toString q.2)) "paramName" =
    .ok q.1.name := string_eq (by lk_tac h)
  have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri (X ++ ":p" ++ toString q.2)) "expr" =
    .ok q.1.expr := optExpr_eq (by lk_tac h)
  have h3 : Decode.optExpr (Index.of ts) k.exprDt (k.iri (X ++ ":p" ++ toString q.2)) "location" =
    .ok q.1.location := optExpr_eq (by lk_tac h)
  simp only [h1, h2, h3, pure, Except.pure, id]

theorem dec_content {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (ct : Option Content)
    (hsub : ∀ d ∈ contentKids k X ct, d ∈ D)
    (hself : Lk ts (k.iri X) "content" ((contentKids k X ct).map (fun d => k.iri d.1))) :
    Decode.content (Index.of ts) k.exprDt (k.iri X) = .ok ct := by
  cases ct with
  | none =>
    simp only [contentKids, List.map_nil] at hself
    simp [Decode.content, optNode_none hself, bind, Except.bind, pure, Except.pure]
  | some ct =>
    simp only [contentKids, List.map_cons, List.map_nil] at hself hsub
    have h := c.kid (hsub _ (List.mem_singleton_self _))
    simp only at h
    have h1 : Decode.optExpr (Index.of ts) k.exprDt (k.iri (X ++ ":c")) "expr" = .ok ct.expr :=
      optExpr_eq (by intro o; rw [h.lookup (U_refl _) (by simp)]; simp [Ctx.content])
    have h2 : Decode.optString (Index.of ts) (k.iri (X ++ ":c")) "inlineContent" = .ok ct.body :=
      optString_eq (by intro o; rw [h.lookup (U_refl _) (by simp)]; simp [Ctx.content, strLit])
    simp only [Decode.content, optNode_some (v := k.iri (X ++ ":c")) ⟨_, rfl⟩ hself, h1, h2, bind, Except.bind, pure,
      Except.pure]


/-! ## Self lookups through children collections -/

section
variable (k)

theorem self_ext (f : String) (xs : List Extension) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.extensions (k.iri f) f xs ↔
      P = vt "extension" ∧ o ∈ (extKids k f xs).map (fun d => k.iri d.1) := by
  rw [extensions_eq]; exact self_kids k (kidsL_ext k f xs) (ch_ext k f xs) hr

theorem self_tok (f s prop : String) (hs : ∀ j : Nat, Ch f (f ++ s ++ toString j))
    (vs : List String) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.tokens (k.iri f) prop (f ++ s) vs ↔
      P = vt prop ∧ o ∈ (tokKids k f (f ++ s) prop vs).map (fun d => k.iri d.1) := by
  rw [tokens_eq]; exact self_kids k (kidsL_tok k _ _ _ vs) (ch_tok k _ _ _ hs vs) hr

theorem self_param (f : String) (ps : List Param) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.params (k.iri f) f ps ↔
      P = vt "param" ∧ o ∈ (paramKids k f ps).map (fun d => k.iri d.1) := by
  rw [params_eq]; exact self_kids k (kidsL_param k f ps) (ch_param k f ps) hr

theorem self_content (f : String) (c : Content) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.content (k.iri f) f c ↔ P = vt "content" ∧ o = k.iri (f ++ ":c") := by
  have := self_kids k (kidsL_content k f (some c)) (ch_content k f (some c)) hr
  simpa [contentKids] using this

theorem self_block (f s prop : String) (hs : ∀ j : Nat, Ch f (f ++ s ++ toString j)) (i : Nat)
    (es : List Exec) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.block (k.iri f) prop (f ++ s) i es ↔
      P = vt prop ∧ o ∈ (blockKids k f s prop i es).map (fun d => k.iri d.1) := by
  rw [block_eq]; exact self_kids k (kidsL_block k f s prop i es) (ch_block k f s prop hs i es) hr

theorem self_branch (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) {P o : Term}
    (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.branches (k.iri f) f j bs ↔
      P = vt "branch" ∧ o ∈ (branchKids k f j bs).map (fun d => k.iri d.1) := by
  rw [branches_eq]; exact self_kids k (kidsL_branch k f j bs) (ch_branch k f j bs) hr

theorem ch_lit (f s : String) (j : Nat) (h : s.toList.head? = some ':')
    (hn : NC s.toList.tail) : Ch f (f ++ s ++ toString j) := by
  cases hs : s.toList with
  | nil => simp [hs] at h
  | cons c l =>
    simp [hs] at h hn; subst h
    exact ⟨l ++ Nat.toDigits 10 j, by simp [String.toList_append, hs], by simp [hn]⟩

end


section
variable (k)

theorem decomp_branch (X : String) (p : (Option Expr × List Exec) × Nat) :
    Decomp k (k.iri X) "branch" (branchK k X p).1 (branchK k X p).2
      (blockKids k (branchK k X p).1 ":a" "action" 0 p.1.2) := by
  rcases p with ⟨⟨c, body⟩, j⟩
  simp only [branchK]
  constructor
  · intro t ht hr
    simp [block_eq] at ht ⊢
    grind (splits := 40)
  · apply ch_of_fst; simp only [fst_blockKids, List.forall_mem_map]; ch_tac
  · exact (kidsL_block k _ _ _ _ _).loc
  · exact nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _
  · intro d hd
    simp only [Ctx.node, List.cons_append, List.tail_cons]
    have := sub_flatMap hd; rw [← block_eq] at this; sub_tac

end


theorem above_ne {owner : Term} {X : String} (h : Above (k := k) owner X) : owner ≠ k.iri X :=
  h X (U_refl X)

theorem len_kid {E : List Triple} {D : List (String × List Triple)} {owner : Term}
    {prop X : String} (hD : Decomp k owner prop X E D) {d : String × List Triple} (hd : d ∈ D)
    {fuel : Nat} (h : E.length ≤ fuel + 1) : d.2.length ≤ fuel := by
  have := (hD.sub d hd).length_le
  simp at this; omega

theorem hasType_of {X : String} {E : List Triple} (h : Cap k ts (U X) E) (cls : String)
    {b : Bool} (hb : ⟨k.iri X, rdfType, vt cls⟩ ∈ E ↔ b = true) :
    Decode.hasType (Index.of ts) (k.iri X) cls = b := by
  rw [hasType_eq, Bool.eq_iff_iff, decide_eq_true_iff, h.lookup (U_refl _) (by simp), hb]

theorem ch_nl (X : String) : ∀ j : Nat, Ch X (X ++ ":nl" ++ toString j) :=
  fun j => ch_lit X ":nl" j (by simp) (by simp)

theorem ch_a (X : String) : ∀ j : Nat, Ch X (X ++ ":a" ++ toString j) :=
  fun j => ch_lit X ":a" j (by simp) (by simp)

syntax "lkx" term "," term : tactic
macro_rules
  | `(tactic| lkx $h, $ho) => `(tactic|
    (intro o
     rw [($h).lookup (U_refl _) (by simp)]
     simp [Ctx.exec, Ctx.execKind, self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
       self_block _ _ _ _ (ch_a _), self_branch, contentKids, blockKids, branchKids, strLit, intLit, boolLit, ($ho),
       (Ne.symm $ho)]))

theorem bind_ok_eq {α β ε : Type} (x : Except ε α) (a : α) (f : α → Except ε β) (h : x = .ok a) :
    x >>= f = f a := by rw [h]; rfl

theorem ord_exec {owner : Term} {prop f : String} {order : Nat} {e : Exec}
    (h : Cap k ts (U f) (k.exec owner prop f order e)) (ho : owner ≠ k.iri f) :
    Lk ts (k.iri f) "order" [intLit order] := by
  rcases e with ⟨kind, exts⟩
  cases kind <;> (try rename_i sd; try rcases sd with ⟨_, _, _, _, _, _, _, _, _, _, _, _, ct⟩) <;>
    (try rcases ct with _ | ct) <;> lkx h, ho

mutual
/-- **Decoding inverts projecting executable content.** If a graph captures the emission of an
executable content element (and its nested blocks), `Decode.exec` returns that element. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "executable content decoded from any graph capturing its projection"}
theorem dec_exec (fuel : Nat) (owner : Term) (prop X : String) (order : Nat) :
    ∀ (e : Exec), Cap k ts (U X) (k.exec owner prop X order e) → Above (k := k) owner X →
      (k.exec owner prop X order e).length ≤ fuel →
      Decode.exec (Index.of ts) k.exprDt fuel (k.iri X) = .ok e
  | .mk kind exts, hcap, hown, hfuel => by
    have hD := decomp_exec k owner prop X order (.mk kind exts)
    have cx : Ctxt k ts owner prop X _ _ := ⟨hcap, hD, hown⟩
    have hoX := above_ne hown
    obtain _ | fuel := fuel
    · simp [Ctx.exec, Ctx.node] at hfuel
    have hsx : ∀ d ∈ extKids k X exts, d ∈ execKids k X (.mk kind exts) := fun d hd => by
      simp only [execKids, List.mem_append]; exact .inr hd
    cases kind with
    | raise ev =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Raise") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.string (Index.of ts) (k.iri X) "eventName" = .ok ev :=
        string_eq (by lkx hcap, hoX)
      simp [Decode.exec, hT, h1, hext, bind, Except.bind, pure, Except.pure]
    | cancel a b =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Cancel") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.optString (Index.of ts) (k.iri X) "sendId" = .ok a :=
        optString_eq (by lkx hcap, hoX)
      have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "sendidexpr" = .ok b :=
        optExpr_eq (by lkx hcap, hoX)
      simp [Decode.exec, hT, h1, h2, hext, bind, Except.bind, pure, Except.pure]
    | log a b =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Log") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.optString (Index.of ts) (k.iri X) "label" = .ok a :=
        optString_eq (by lkx hcap, hoX)
      have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "expr" = .ok b :=
        optExpr_eq (by lkx hcap, hoX)
      simp [Decode.exec, hT, h1, h2, hext, bind, Except.bind, pure, Except.pure]
    | assign a b c =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Assign") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.expr (Index.of ts) k.exprDt (k.iri X) "location" = .ok a :=
        expr_eq (by lkx hcap, hoX)
      have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "expr" = .ok b :=
        optExpr_eq (by lkx hcap, hoX)
      have h3 : Decode.optString (Index.of ts) (k.iri X) "inlineContent" = .ok c :=
        optString_eq (by lkx hcap, hoX)
      simp [Decode.exec, hT, h1, h2, h3, hext, bind, Except.bind, pure, Except.pure]
    | script sc =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Script") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.optString (Index.of ts) (k.iri X) "src" = .ok sc.src :=
        optString_eq (by lkx hcap, hoX)
      have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "scriptBody" = .ok sc.body :=
        optExpr_eq (by lkx hcap, hoX)
      simp [Decode.exec, Decode.script, hT, h1, h2, hext, bind, Except.bind, pure, Except.pure]
    | extension x =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "ExtensionContent") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have h1 : Decode.extensionFields (Index.of ts) (k.iri X) = .ok x := by
        rcases x with ⟨ns, ln, v, pos⟩
        have a1 : Decode.string (Index.of ts) (k.iri X) "namespace" = .ok ns := string_eq (by lkx hcap, hoX)
        have a2 : Decode.string (Index.of ts) (k.iri X) "localName" = .ok ln := string_eq (by lkx hcap, hoX)
        have a3 : Decode.integer (Index.of ts) (k.iri X) "position" = .ok pos :=
          integer_eq (by lkx hcap, hoX)
        rcases v with a | a
        · have a4 : Decode.optString (Index.of ts) (k.iri X) "attributeValue" = .ok (some a) :=
            optString_eq (by lkx hcap, hoX)
          have a5 : Decode.optString (Index.of ts) (k.iri X) "elementXml" = .ok none :=
            optString_eq (by lkx hcap, hoX)
          simp only [Decode.extensionFields, a1, a2, a3, a4, a5, bind, Except.bind, pure, Except.pure]
        · have a4 : Decode.optString (Index.of ts) (k.iri X) "attributeValue" = .ok none :=
            optString_eq (by lkx hcap, hoX)
          have a5 : Decode.optString (Index.of ts) (k.iri X) "elementXml" = .ok (some a) :=
            optString_eq (by lkx hcap, hoX)
          simp only [Decode.extensionFields, a1, a2, a3, a4, a5, bind, Except.bind, pure, Except.pure]
      simp [Decode.exec, hT, h1, hext, bind, Except.bind, pure, Except.pure]
    | send sd =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Send") := fun cls => by
        apply hasType_of hcap cls
        rcases hc : sd.content <;> simp [Ctx.exec, Ctx.execKind, self_ext, execClass, hc,
          self_tok _ _ _ _ (ch_nl _), self_param, self_content]
      rcases sd with ⟨a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, nl, ps, ct⟩
      have hext := dec_extensions cx exts hsx (by rcases ct <;> lkx hcap, hoX)
      have h1 : Decode.optString (Index.of ts) (k.iri X) "eventName" = .ok a1 :=
        optString_eq (by rcases ct <;> lkx hcap, hoX)
      have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "eventexpr" = .ok a2 :=
        optExpr_eq (by rcases ct <;> lkx hcap, hoX)
      have h3 : Decode.optString (Index.of ts) (k.iri X) "sendTarget" = .ok a3 :=
        optString_eq (by rcases ct <;> lkx hcap, hoX)
      have h4 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "targetexpr" = .ok a4 :=
        optExpr_eq (by rcases ct <;> lkx hcap, hoX)
      have h5 : Decode.optString (Index.of ts) (k.iri X) "type" = .ok a5 :=
        optString_eq (by rcases ct <;> lkx hcap, hoX)
      have h6 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "typeexpr" = .ok a6 :=
        optExpr_eq (by rcases ct <;> lkx hcap, hoX)
      have h7 : Decode.optString (Index.of ts) (k.iri X) "sendId" = .ok a7 :=
        optString_eq (by rcases ct <;> lkx hcap, hoX)
      have h8 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "idlocation" = .ok a8 :=
        optExpr_eq (by rcases ct <;> lkx hcap, hoX)
      have h9 : Decode.optString (Index.of ts) (k.iri X) "delay" = .ok a9 :=
        optString_eq (by rcases ct <;> lkx hcap, hoX)
      have h10 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "delayexpr" = .ok a10 :=
        optExpr_eq (by rcases ct <;> lkx hcap, hoX)
      have h11 : Decode.tokens (Index.of ts) (k.iri X) "namelist" = .ok nl :=
        dec_tokens cx (X ++ ":nl") "namelist" (fun a b h => frag_inj _ _ _ _ h)
          (fun j => by simp [← String.toList_inj, String.toList_append]) nl
          (fun d hd => by simp only [execKids, execKindKids, List.mem_append]; exact .inl (.inl (.inl hd)))
          (by rcases ct <;> lkx hcap, hoX)
      have h12 : Decode.params (Index.of ts) k.exprDt (k.iri X) = .ok ps :=
        dec_params cx ps
          (fun d hd => by simp only [execKids, execKindKids, List.mem_append]; exact .inl (.inl (.inr hd)))
          (by rcases ct <;> lkx hcap, hoX)
      have h13 : Decode.content (Index.of ts) k.exprDt (k.iri X) = .ok ct :=
        dec_content cx ct
          (fun d hd => by simp only [execKids, execKindKids, List.mem_append]; exact .inl (.inr hd))
          (by rcases ct <;> lkx hcap, hoX)
      simp [Decode.exec, hT, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, h13, hext, bind,
        Except.bind, pure, Except.pure]
    | if_ bs =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "If") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass, self_branch])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have hkid : ∀ p ∈ bs.zipIdx 0, branchK k X p ∈ execKids k X (.mk (.if_ bs) exts) :=
        fun p hp => List.mem_append.2 (.inl (List.mem_map_of_mem hp))
      have ho : Decode.ordered (Index.of ts) (k.iri X) "branch" =
          .ok (((bs.zipIdx 0).map (branchK k X)).map (fun d => k.iri d.1)) :=
        ordered_kids bs (branchK k X) (by lkx hcap, hoX) (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
          (fun p hp => by
            have h := cx.kid (hkid p hp)
            have hoB := above_ne (k := k) (above_of_ch (ch_lit X ":b" p.2 (by simp) (by simp)))
            simp only [branchK] at h ⊢
            lkx h, hoB)
      have hcaps : ∀ p ∈ bs.zipIdx 0, Cap k ts (U (branchK k X p).1) (branchK k X p).2 ∧
          (branchK k X p).2.length ≤ fuel + 1 :=
        fun p hp => ⟨cx.kid (hkid p hp), Nat.le_succ_of_le (len_kid hD (hkid p hp) hfuel)⟩
      simp only [Decode.exec, hT, hext]
      simp only [Bool.or_eq_true, beq_iff_eq, String.reduceEq, or_self, or_false, false_or,
        or_true, true_or, ite_true, ite_false, ↓reduceIte]
      rw [bind_ok_eq _ _ _ ho]
      rw [bind_ok_eq _ _ _ (dec_branchMembers fuel X _ ?hF bs 0 hcaps)]
      · rfl
      · intro b; rfl
    | foreach arr item idx body =>
      have hT : ∀ cls, Decode.hasType (Index.of ts) (k.iri X) cls =
          (cls == "ExecutableContent" || cls == "Foreach") := fun cls =>
        hasType_of hcap cls (by simp [Ctx.exec, Ctx.execKind, self_ext, execClass,
          self_block _ _ _ _ (ch_a _)])
      have hext := dec_extensions cx exts hsx (by lkx hcap, hoX)
      have hkid : ∀ p ∈ body.zipIdx 0, blockK k X ":a" "action" p ∈ execKids k X (.mk (.foreach arr item idx body) exts) :=
        fun p hp => List.mem_append.2 (.inl (List.mem_map_of_mem hp))
      have hmem : ∀ p ∈ body.zipIdx 0, Cap k ts (U (blockK k X ":a" "action" p).1)
          (blockK k X ":a" "action" p).2 ∧ (blockK k X ":a" "action" p).2.length ≤ fuel :=
        fun p hp => ⟨cx.kid (hkid p hp), len_kid hD (hkid p hp) hfuel⟩
      have ho : Decode.ordered (Index.of ts) (k.iri X) "action" =
          .ok (((body.zipIdx 0).map (blockK k X ":a" "action")).map (fun d => k.iri d.1)) :=
        ordered_kids body _ (by lkx hcap, hoX) (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
          (fun p hp => ord_exec (hmem p hp).1 (above_ne (above_of_ch (ch_a _ p.2))))
      have hb := dec_blockMembers fuel X ":a" "action" (ch_a _) body 0 hmem
      have h1 : Decode.expr (Index.of ts) k.exprDt (k.iri X) "array" = .ok arr :=
        expr_eq (by lkx hcap, hoX)
      have h2 : Decode.string (Index.of ts) (k.iri X) "item" = .ok item :=
        string_eq (by lkx hcap, hoX)
      have h3 : Decode.optString (Index.of ts) (k.iri X) "index" = .ok idx :=
        optString_eq (by lkx hcap, hoX)
      simp only [Decode.exec, hT, hext]
      simp only [Bool.or_eq_true, beq_iff_eq, String.reduceEq, or_self, or_false, false_or,
        or_true, true_or, ite_true, ite_false, ↓reduceIte]
      simp only [ho, h1, h2, h3, bind, Except.bind, hb]
      rfl
theorem dec_blockMembers (fuel : Nat) (X s prop : String) (hs : ∀ j : Nat, Ch X (X ++ s ++ toString j)) :
    ∀ (es : List Exec) (i : Nat),
      (∀ p ∈ es.zipIdx i, Cap k ts (U (blockK k X s prop p).1) (blockK k X s prop p).2 ∧
        (blockK k X s prop p).2.length ≤ fuel) →
      (((es.zipIdx i).map (blockK k X s prop)).map (fun d => k.iri d.1)).mapM
        (Decode.exec (Index.of ts) k.exprDt fuel) = .ok es
  | [], _, _ => rfl
  | e :: es, i, h => by
    have h1 := h (e, i) (by simp)
    have he := dec_exec fuel (k.iri X) prop (X ++ s ++ toString i) i e h1.1
      (above_of_ch (hs i)) h1.2
    have hes := dec_blockMembers fuel X s prop hs es (i + 1) (fun p hp => h p (by simp [hp]))
    simp only [List.zipIdx_cons, List.map_cons, List.mapM_cons]
    simp only [blockK] at he ⊢
    rw [he, hes]; rfl
theorem dec_branchMembers (fuel : Nat) (X : String) (F : Term → D (Option Expr × List Exec))
    (hF : ∀ b, F b = (do
      let c ← Decode.optExpr (Index.of ts) k.exprDt b "cond"
      let l ← (do
        let a ← Decode.ordered (Index.of ts) b "action"
        List.mapM (Decode.exec (Index.of ts) k.exprDt fuel) a)
      pure (c, l))) :
    ∀ (bs : List (Option Expr × List Exec)) (i : Nat),
      (∀ p ∈ bs.zipIdx i, Cap k ts (U (branchK k X p).1) (branchK k X p).2 ∧
        (branchK k X p).2.length ≤ fuel + 1) →
      (((bs.zipIdx i).map (branchK k X)).map (fun d => k.iri d.1)).mapM F = .ok bs
  | [], _, _ => rfl
  | (c, body) :: bs, i, h => by
    have h1 := h ((c, body), i) (by simp)
    have hD := decomp_branch k X ((c, body), i)
    have hown : Above (k := k) (k.iri X) (branchK k X ((c, body), i)).1 :=
      above_of_ch (ch_lit X ":b" i (by simp) (by simp))
    have cx : Ctxt k ts _ _ _ _ _ := ⟨h1.1, hD, hown⟩
    have hoB := above_ne hown
    simp only [branchK] at h1 hoB
    have hc : Decode.optExpr (Index.of ts) k.exprDt (k.iri (X ++ ":b" ++ toString i)) "cond" = .ok c :=
      optExpr_eq (by lkx h1.1, hoB)
    have hmem : ∀ p ∈ body.zipIdx 0, Cap k ts (U (blockK k (X ++ ":b" ++ toString i) ":a" "action" p).1)
        (blockK k (X ++ ":b" ++ toString i) ":a" "action" p).2 ∧
        (blockK k (X ++ ":b" ++ toString i) ":a" "action" p).2.length ≤ fuel := fun p hp =>
      ⟨cx.kid (List.mem_map_of_mem hp), len_kid hD (List.mem_map_of_mem hp) h1.2⟩
    have ho : Decode.ordered (Index.of ts) (k.iri (X ++ ":b" ++ toString i)) "action" =
        .ok ((blockKids k (X ++ ":b" ++ toString i) ":a" "action" 0 body).map (fun d => k.iri d.1)) :=
      ordered_kids body _ (by lkx h1.1, hoB) (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
        (fun p hp => ord_exec (hmem p hp).1 (above_ne (above_of_ch (ch_a _ p.2))))
    have hb := dec_blockMembers fuel (X ++ ":b" ++ toString i) ":a" "action" (ch_a _) body 0 hmem
    have hes := dec_branchMembers fuel X F hF bs (i + 1) (fun p hp => h p (by simp [hp]))
    simp only [List.zipIdx_cons, List.map_cons, List.mapM_cons, hes]
    simp only [branchK]
    rw [hF, hc]
    simp only [blockKids] at ho
    simp only [bind, Except.bind, ho, hb]
    rfl
end

end RT
end Vertep.Rdf
