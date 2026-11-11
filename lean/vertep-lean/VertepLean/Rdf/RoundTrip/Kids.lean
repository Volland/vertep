import VertepLean.Rdf.RoundTrip.Loc

/-!
# RDF round trip: children of a node

The children of each node kind as (fragment, emission) lists, their decomposition facts
(`Decomp`), and the links from a node to its children (`KidsL`, `self_kids`).
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

variable (k : Ctx)

/-- A decomposition of the emission `E` of the node at `f` (linked from `owner` by `prop`): its
relevant triples are the link, triples about the node itself, or triples of one of the children
`D` (fragment and emission), whose fragments are distinct children of `f`. -/
structure Decomp (owner : Term) (prop f : String) (E : List Triple)
    (D : List (String × List Triple)) : Prop where
  cov : ∀ t ∈ E, Rel t → t = ⟨owner, vt prop, k.iri f⟩ ∨ t.s = k.iri f ∨ ∃ d ∈ D, t ∈ d.2
  ch : ∀ d ∈ D, Ch f d.1
  loc : ∀ d ∈ D, LocU k (k.iri f) d.1 d.2
  nd : (D.map Prod.fst).Nodup
  sub : ∀ d ∈ D, d.2.Sublist E.tail

variable {k}

theorem Decomp.cap {ts E : List Triple} {owner : Term} {prop f : String}
    {D : List (String × List Triple)} (hD : Decomp k owner prop f E D) (h : Cap k ts (U f) E)
    (hown : Above (k := k) owner f) {d : String × List Triple} (hd : d ∈ D) :
    Cap k ts (U d.1) d.2 :=
  h.kid D (fun t ht hr => by
      rcases hD.cov t ht hr with h | h | h
      · exact .inl (by rw [h])
      · exact .inr (.inl h)
      · exact .inr (.inr h))
    hD.ch hD.loc hD.nd hown hd ((hD.sub d hd).trans (List.tail_sublist E))

theorem Decomp.locL {owner : Term} {prop f : String} {E : List Triple}
    {D : List (String × List Triple)} (hD : Decomp k owner prop f E D) : LocL k owner prop f E := by
  intro t ht hr
  rcases hD.cov t ht hr with h | h | ⟨d, hd, ht'⟩
  · exact .inl h
  · exact .inr ⟨f, h, U_refl f⟩
  · rcases hD.loc d hd t ht' hr with h | ⟨g, hg, hU⟩
    · exact .inr ⟨f, h, U_refl f⟩
    · exact .inr ⟨g, hg, U_up (hD.ch d hd) hU⟩

variable (k)

theorem sublist_flatMap_of_mem {α β : Type} {f : α → List β} {a : α} :
    ∀ {l : List α}, a ∈ l → (f a).Sublist (l.flatMap f)
  | x :: l, h => by
    rw [List.flatMap_cons]
    rcases List.mem_cons.1 h with rfl | h
    · exact List.sublist_append_left _ _
    · exact (sublist_flatMap_of_mem h).trans (List.sublist_append_right _ _)

/-! ## Children lists -/

def extK (f : String) (p : Extension × Nat) : String × List Triple :=
  (f ++ ":x" ++ toString p.2,
    k.node (k.iri f) "extension" (f ++ ":x" ++ toString p.2) ["ExtensionNode"] ++
      ordT (k.iri (f ++ ":x" ++ toString p.2)) p.2 ++
      Ctx.extensionFields (k.iri (f ++ ":x" ++ toString p.2)) p.1)

def extKids (f : String) (xs : List Extension) : List (String × List Triple) :=
  xs.zipIdx.map (extK k f)

theorem extensions_eq (f : String) (xs : List Extension) :
    k.extensions (k.iri f) f xs = (extKids k f xs).flatMap Prod.snd := by
  simp only [Ctx.extensions, extKids, extK, List.flatMap_map]

def tokK (f pre prop : String) (p : String × Nat) : String × List Triple :=
  (pre ++ toString p.2,
    k.node (k.iri f) prop (pre ++ toString p.2) ["Token"] ++ ordT (k.iri (pre ++ toString p.2)) p.2 ++
      str (k.iri (pre ++ toString p.2)) "value" p.1)

def tokKids (f pre prop : String) (vs : List String) : List (String × List Triple) :=
  vs.zipIdx.map (tokK k f pre prop)

theorem tokens_eq (f pre prop : String) (vs : List String) :
    k.tokens (k.iri f) prop pre vs = (tokKids k f pre prop vs).flatMap Prod.snd := by
  simp only [Ctx.tokens, tokKids, tokK, List.flatMap_map]

def paramK (f : String) (p : Param × Nat) : String × List Triple :=
  (f ++ ":p" ++ toString p.2,
    k.node (k.iri f) "param" (f ++ ":p" ++ toString p.2) ["Param"] ++
      ordT (k.iri (f ++ ":p" ++ toString p.2)) p.2 ++
      str (k.iri (f ++ ":p" ++ toString p.2)) "paramName" p.1.name ++
      k.optExpr (k.iri (f ++ ":p" ++ toString p.2)) "expr" p.1.expr ++
      k.optExpr (k.iri (f ++ ":p" ++ toString p.2)) "location" p.1.location)

def paramKids (f : String) (ps : List Param) : List (String × List Triple) :=
  ps.zipIdx.map (paramK k f)

theorem params_eq (f : String) (ps : List Param) :
    k.params (k.iri f) f ps = (paramKids k f ps).flatMap Prod.snd := by
  simp only [Ctx.params, paramKids, paramK, List.flatMap_map]

def blockK (f s prop : String) (p : Exec × Nat) : String × List Triple :=
  (f ++ s ++ toString p.2, k.exec (k.iri f) prop (f ++ s ++ toString p.2) p.2 p.1)

def blockKids (f s prop : String) (i : Nat) (es : List Exec) : List (String × List Triple) :=
  (es.zipIdx i).map (blockK k f s prop)

theorem block_eq (f s prop : String) :
    ∀ (i : Nat) (es : List Exec),
      k.block (k.iri f) prop (f ++ s) i es = (blockKids k f s prop i es).flatMap Prod.snd
  | _, [] => by simp [Ctx.block, blockKids]
  | i, e :: es => by
    rw [Ctx.block, block_eq f s prop (i + 1) es]
    simp [blockKids, blockK, List.flatMap_map, String.append_assoc]

def branchK (f : String) (p : (Option Expr × List Exec) × Nat) : String × List Triple :=
  (f ++ ":b" ++ toString p.2,
    k.node (k.iri f) "branch" (f ++ ":b" ++ toString p.2) ["IfBranch"] ++
      ordT (k.iri (f ++ ":b" ++ toString p.2)) p.2 ++
      k.optExpr (k.iri (f ++ ":b" ++ toString p.2)) "cond" p.1.1 ++
      k.block (k.iri (f ++ ":b" ++ toString p.2)) "action" (f ++ ":b" ++ toString p.2 ++ ":a") 0
        p.1.2)

def branchKids (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    List (String × List Triple) :=
  (bs.zipIdx j).map (branchK k f)

theorem branches_eq (f : String) :
    ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
      k.branches (k.iri f) f j bs = (branchKids k f j bs).flatMap Prod.snd
  | _, [] => by simp [Ctx.branches, branchKids]
  | j, (c, body) :: bs => by
    rw [Ctx.branches, branches_eq f (j + 1) bs]
    simp [branchKids, branchK, List.flatMap_map]

def contentKids (f : String) : Option Content → List (String × List Triple)
  | some c => [(f ++ ":c", k.content (k.iri f) f c)]
  | none => []

def execKindKids (X : String) : ExecKind → List (String × List Triple)
  | .send s => tokKids k X (X ++ ":nl") "namelist" s.namelist ++ paramKids k X s.params ++
      contentKids k X s.content
  | .if_ bs => branchKids k X 0 bs
  | .foreach _ _ _ body => blockKids k X ":a" "action" 0 body
  | _ => []

def execKids (X : String) : Exec → List (String × List Triple)
  | .mk kind exts => execKindKids k X kind ++ extKids k X exts


/-! ## Facts about children lists -/

theorem nodup_zipIdx_map {α : Type} (F : Nat → String) (hF : ∀ a b, F a = F b → a = b) :
    ∀ (xs : List α) (i : Nat), ((xs.zipIdx i).map (fun p => F p.2)).Nodup
  | [], _ => by simp
  | x :: xs, i => by
    rw [List.zipIdx_cons, List.map_cons, List.nodup_cons]
    refine ⟨?_, nodup_zipIdx_map F hF xs (i + 1)⟩
    simp only [List.mem_map, not_exists, not_and]
    intro p hp he
    have := List.mem_zipIdx (x := p.1) (i := p.2) hp
    have := hF _ _ he
    omega

/-- Each child of a children list is local to its fragment and linked from `k.iri f`. -/
def KidsLoc (f : String) (D : List (String × List Triple)) : Prop :=
  ∀ d ∈ D, LocU k (k.iri f) d.1 d.2

theorem kidsLoc_append {f : String} {A B : List (String × List Triple)}
    (ha : KidsLoc k f A) (hb : KidsLoc k f B) : KidsLoc k f (A ++ B) := by
  intro d hd; rcases List.mem_append.1 hd with h | h
  · exact ha d h
  · exact hb d h

theorem kidsLoc_ext (f : String) (xs : List Extension) : KidsLoc k f (extKids k f xs) := by
  intro d hd t ht hr
  simp only [extKids, extK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp at ht <;> grind (splits := 60) [U_refl]

theorem kidsLoc_tok (f pre prop : String) (vs : List String) :
    KidsLoc k f (tokKids k f pre prop vs) := by
  intro d hd t ht hr
  simp only [tokKids, tokK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  simp at ht; grind (splits := 60) [U_refl]

theorem kidsLoc_param (f : String) (ps : List Param) : KidsLoc k f (paramKids k f ps) := by
  intro d hd t ht hr
  simp only [paramKids, paramK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  simp at ht; grind (splits := 60) [U_refl]

theorem kidsLoc_content (f : String) (c : Option Content) : KidsLoc k f (contentKids k f c) := by
  intro d hd
  cases c with
  | none => simp [contentKids] at hd
  | some c =>
    simp only [contentKids, List.mem_singleton] at hd; subst hd
    intro t ht hr
    rcases locL_content k f c t ht hr with rfl | h
    · exact .inl rfl
    · exact .inr h

theorem kidsLoc_block (f s prop : String) (i : Nat) (es : List Exec) :
    KidsLoc k f (blockKids k f s prop i es) := by
  intro d hd t ht hr
  simp only [blockKids, blockK, List.mem_map] at hd
  obtain ⟨⟨e, j⟩, _, rfl⟩ := hd
  rcases locL_exec k _ _ _ _ e t ht hr with rfl | h
  · exact .inl rfl
  · exact .inr h

theorem kidsLoc_branch (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    KidsLoc k f (branchKids k f j bs) := by
  intro d hd t ht hr
  simp only [branchKids, branchK, List.mem_map] at hd
  obtain ⟨⟨⟨c, body⟩, i⟩, _, rfl⟩ := hd
  have h1 := locU_block k (f ++ ":b" ++ toString i) ":a" "action" (by simp) 0 body
  unfold LocU at h1
  simp at ht; grind (splits := 60) [U_refl]


theorem sub_flatMap {d : String × List Triple} {D : List (String × List Triple)} (h : d ∈ D) :
    d.2.Sublist (D.flatMap Prod.snd) := sublist_flatMap_of_mem (f := Prod.snd) h

syntax "sub_tac" : tactic
macro_rules
  | `(tactic| sub_tac) => `(tactic| first
      | assumption
      | exact List.Sublist.refl _
      | (apply List.sublist_append_of_sublist_left; sub_tac)
      | (apply List.sublist_append_of_sublist_right; sub_tac))

theorem frag_inj (f s : String) (a b : Nat) (h : f ++ s ++ toString a = f ++ s ++ toString b) :
    a = b := by
  simpa [← String.toList_inj, String.toList_append, digits_inj] using h

theorem nodup_zipIdx_map' {α : Type} (G : α × Nat → String)
    (hG : ∀ p q : α × Nat, G p = G q → p.2 = q.2) :
    ∀ (xs : List α) (i : Nat), ((xs.zipIdx i).map G).Nodup
  | [], _ => by simp
  | x :: xs, i => by
    rw [List.zipIdx_cons, List.map_cons, List.nodup_cons]
    refine ⟨?_, nodup_zipIdx_map' G hG xs (i + 1)⟩
    simp only [List.mem_map, not_exists, not_and]
    intro p hp he
    have := List.mem_zipIdx (x := p.1) (i := p.2) hp
    have := hG _ _ he
    simp at this
    omega

@[simp] theorem toList_repr (n : Nat) : (Nat.repr n).toList = Nat.toDigits 10 n := by
  simp [Nat.repr]

syntax "frag_tac" : tactic
macro_rules
  | `(tactic| frag_tac) => `(tactic| (simp (config := {contextual := true})
      [← String.toList_inj, String.toList_append, digits_inj]))

syntax "ch_tac" : tactic
macro_rules
  | `(tactic| ch_tac) => `(tactic| (simp (config := {contextual := true})
      [Ch, String.toList_append]))

theorem nodup_option_toList {α : Type} (o : Option α) : o.toList.Nodup := by
  cases o <;> simp

syntax "nd_tac" : tactic
macro_rules
  | `(tactic| nd_tac) => `(tactic| (and_intros <;> first
      | exact nodup_zipIdx_map' _ (fun p q h => by
          simpa [← String.toList_inj, String.toList_append, digits_inj] using h) _ _
      | exact nodup_option_toList _
      | (intros; (try intro); subst_vars; simp_all [← String.toList_inj, String.toList_append, digits_inj])))

theorem ch_of_fst {f : String} {D : List (String × List Triple)}
    (h : ∀ x ∈ D.map Prod.fst, Ch f x) : ∀ d ∈ D, Ch f d.1 :=
  fun d hd => h d.1 (List.mem_map_of_mem hd)

@[simp] theorem fst_extKids (f : String) (xs : List Extension) :
    (extKids k f xs).map Prod.fst = xs.zipIdx.map (fun p => f ++ ":x" ++ toString p.2) := by
  simp [extKids, extK, List.map_map, Function.comp_def]
@[simp] theorem fst_tokKids (f pre prop : String) (vs : List String) :
    (tokKids k f pre prop vs).map Prod.fst = vs.zipIdx.map (fun p => pre ++ toString p.2) := by
  simp [tokKids, tokK, List.map_map, Function.comp_def]
@[simp] theorem fst_paramKids (f : String) (ps : List Param) :
    (paramKids k f ps).map Prod.fst = ps.zipIdx.map (fun p => f ++ ":p" ++ toString p.2) := by
  simp [paramKids, paramK, List.map_map, Function.comp_def]
@[simp] theorem fst_blockKids (f s prop : String) (i : Nat) (es : List Exec) :
    (blockKids k f s prop i es).map Prod.fst = (es.zipIdx i).map (fun p => f ++ s ++ toString p.2) := by
  simp [blockKids, blockK, List.map_map, Function.comp_def]
@[simp] theorem fst_branchKids (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    (branchKids k f j bs).map Prod.fst = (bs.zipIdx j).map (fun p => f ++ ":b" ++ toString p.2) := by
  simp [branchKids, branchK, List.map_map, Function.comp_def]
@[simp] theorem fst_contentKids (f : String) (c : Option Content) :
    (contentKids k f c).map Prod.fst = (c.map (fun _ => f ++ ":c")).toList := by
  cases c <;> simp [contentKids]

theorem decomp_exec (owner : Term) (prop X : String) (order : Nat) (e : Exec) :
    Decomp k owner prop X (k.exec owner prop X order e) (execKids k X e) := by
  rcases e with ⟨kind, exts⟩
  constructor
  · intro t ht hr
    cases kind <;>
      simp [Ctx.exec, Ctx.execKind, extensions_eq, tokens_eq, params_eq, branches_eq, block_eq,
        execKids, execKindKids, contentKids] at ht ⊢ <;>
      grind (splits := 80)
  · apply ch_of_fst
    cases kind <;> simp only [execKids, execKindKids, List.map_append, fst_extKids, fst_tokKids,
      fst_paramKids, fst_contentKids, fst_branchKids, fst_blockKids, List.forall_mem_append,
      List.forall_mem_map, List.nil_append] <;> ch_tac
  · cases kind <;> simp only [execKids, execKindKids] <;>
      first
      | exact kidsLoc_ext k X _
      | exact kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsLoc_tok k _ _ _ _)
          (kidsLoc_param k _ _)) (kidsLoc_content k _ _)) (kidsLoc_ext k X _)
      | exact kidsLoc_append k (kidsLoc_branch k _ _ _) (kidsLoc_ext k X _)
      | exact kidsLoc_append k (kidsLoc_block k _ _ _ _ _) (kidsLoc_ext k X _)
      | (simp only [List.nil_append]; exact kidsLoc_ext k X _)
  · cases kind <;> simp [execKids, execKindKids, List.nodup_append] <;> nd_tac
  · intro d hd
    simp only [Ctx.exec, Ctx.node, List.cons_append, List.tail_cons]
    simp only [execKids, List.mem_append] at hd
    rcases hd with hd | hd
    · cases kind <;> simp only [execKindKids, List.mem_append, List.not_mem_nil] at hd
      case send s =>
        simp only [Ctx.execKind]
        rcases hd with (hd | hd) | hd
        · have := sub_flatMap hd; rw [← tokens_eq] at this; sub_tac
        · have := sub_flatMap hd; rw [← params_eq] at this; sub_tac
        · rcases hc : s.content with _ | c
          · simp [contentKids, hc] at hd
          · simp only [contentKids, hc, List.mem_singleton] at hd; subst hd
            sub_tac
      case if_ bs =>
        simp only [Ctx.execKind]
        have := sub_flatMap hd; rw [← branches_eq] at this; sub_tac
      case foreach body =>
        simp only [Ctx.execKind]
        have := sub_flatMap hd; rw [← block_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← extensions_eq] at this; sub_tac


/-! ## Links from a node to its children -/

/-- Each child is linked from `k.iri f` by `prop` and local to its fragment. -/
def KidsL (f prop : String) (D : List (String × List Triple)) : Prop :=
  ∀ d ∈ D, LocL k (k.iri f) prop d.1 d.2 ∧ ⟨k.iri f, vt prop, k.iri d.1⟩ ∈ d.2

/-- The triples of the children with subject `k.iri f` are exactly the links. -/
theorem self_kids {f prop : String} {D : List (String × List Triple)} (hL : KidsL k f prop D)
    (hch : ∀ d ∈ D, Ch f d.1) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ D.flatMap Prod.snd ↔ P = vt prop ∧ o ∈ D.map (fun d => k.iri d.1) := by
  constructor
  · intro h
    rw [List.mem_flatMap] at h
    obtain ⟨d, hd, ht⟩ := h
    rcases (hL d hd).1 _ ht hr with h | ⟨g, hg, hU⟩
    · simp only [Triple.mk.injEq] at h
      exact ⟨h.2.1, List.mem_map.2 ⟨d, hd, h.2.2.symm⟩⟩
    · simp only [iri_inj] at hg; subst hg
      exact absurd hU (not_U_of_Ch (hch d hd))
  · rintro ⟨rfl, ho⟩
    obtain ⟨d, hd, rfl⟩ := List.mem_map.1 ho
    exact List.mem_flatMap.2 ⟨d, hd, (hL d hd).2⟩

theorem kidsL_ext (f : String) (xs : List Extension) : KidsL k f "extension" (extKids k f xs) := by
  intro d hd
  simp only [extKids, extK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  refine ⟨fun t ht hr => ?_, by simp⟩
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp at ht <;> grind (splits := 60) [U_refl]

theorem kidsL_tok (f pre prop : String) (vs : List String) :
    KidsL k f prop (tokKids k f pre prop vs) := by
  intro d hd
  simp only [tokKids, tokK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  refine ⟨fun t ht hr => ?_, by simp⟩
  simp at ht; grind (splits := 60) [U_refl]

theorem kidsL_param (f : String) (ps : List Param) : KidsL k f "param" (paramKids k f ps) := by
  intro d hd
  simp only [paramKids, paramK, List.mem_map] at hd
  obtain ⟨⟨x, i⟩, _, rfl⟩ := hd
  refine ⟨fun t ht hr => ?_, by simp⟩
  simp at ht; grind (splits := 60) [U_refl]

theorem kidsL_content (f : String) (c : Option Content) :
    KidsL k f "content" (contentKids k f c) := by
  intro d hd
  cases c with
  | none => simp [contentKids] at hd
  | some c =>
    simp only [contentKids, List.mem_singleton] at hd; subst hd
    exact ⟨locL_content k f c, by simp [Ctx.content]⟩

theorem kidsL_block (f s prop : String) (i : Nat) (es : List Exec) :
    KidsL k f prop (blockKids k f s prop i es) := by
  intro d hd
  simp only [blockKids, blockK, List.mem_map] at hd
  obtain ⟨⟨e, j⟩, _, rfl⟩ := hd
  refine ⟨locL_exec k _ _ _ _ e, ?_⟩
  rcases e with ⟨kind, exts⟩
  simp [Ctx.exec]

theorem kidsL_branch (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    KidsL k f "branch" (branchKids k f j bs) := by
  intro d hd
  simp only [branchKids, branchK, List.mem_map] at hd
  obtain ⟨⟨⟨c, body⟩, i⟩, _, rfl⟩ := hd
  have h1 := locU_block k (f ++ ":b" ++ toString i) ":a" "action" (by simp) 0 body
  unfold LocU at h1
  refine ⟨fun t ht hr => ?_, by simp⟩
  simp at ht; grind (splits := 60) [U_refl]

theorem KidsL.loc {f prop : String} {D : List (String × List Triple)} (h : KidsL k f prop D) :
    KidsLoc k f D := fun d hd => (h d hd).1.locU

theorem ch_ext (f : String) (xs : List Extension) : ∀ d ∈ extKids k f xs, Ch f d.1 := by
  apply ch_of_fst; simp only [fst_extKids, List.forall_mem_map]; ch_tac
theorem ch_tok (f pre prop : String) (hpre : ∀ j : Nat, Ch f (pre ++ toString j)) (vs : List String) :
    ∀ d ∈ tokKids k f pre prop vs, Ch f d.1 := by
  apply ch_of_fst; simp only [fst_tokKids, List.forall_mem_map]; intro p _; exact hpre p.2
theorem ch_param (f : String) (ps : List Param) : ∀ d ∈ paramKids k f ps, Ch f d.1 := by
  apply ch_of_fst; simp only [fst_paramKids, List.forall_mem_map]; ch_tac
theorem ch_content (f : String) (c : Option Content) : ∀ d ∈ contentKids k f c, Ch f d.1 := by
  apply ch_of_fst; cases c <;> simp [Ch, String.toList_append]
theorem ch_block (f s prop : String) (hs : ∀ j : Nat, Ch f (f ++ s ++ toString j)) (i : Nat)
    (es : List Exec) : ∀ d ∈ blockKids k f s prop i es, Ch f d.1 := by
  apply ch_of_fst; simp only [fst_blockKids, List.forall_mem_map]; intro p _; exact hs p.2
theorem ch_branch (f : String) (j : Nat) (bs : List (Option Expr × List Exec)) :
    ∀ d ∈ branchKids k f j bs, Ch f d.1 := by
  apply ch_of_fst; simp only [fst_branchKids, List.forall_mem_map]; ch_tac

end RT
end Vertep.Rdf
