import VertepLean.Rdf.RoundTrip.Transition
set_option linter.unusedSimpArgs false

/-!
# RDF round trip: data items, invokes, done data, hooks and `<initial>` elements

Decomposition and decoding of the remaining node kinds a state owns, in the style of
`dec_transition`: given capture (`Cap`) of the node's emission and an owner above it, the decoder
returns the IR the node was projected from. Also `Cap.split`, which passes capture to a part of
an emission whose other relevant triples lie outside the part's region.
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

variable {k : Ctx} {ts : List Triple}

/-- Capture passes to a part `A` of a captured emission `E` when every other relevant triple of
`E` has a subject outside `A`'s region. -/
theorem Cap.split {E A : List Triple} {R RA : String → Prop} (h : Cap k ts R E)
    (hsub : A.Sublist E) (hRA : ∀ g, RA g → R g)
    (hcov : ∀ t ∈ E, Rel t → t ∈ A ∨ ∀ g, t.s = k.iri g → ¬ RA g) : Cap k ts RA A :=
  ⟨hsub.trans h.sub, fun t ht hr g hg hg' => by
    rcases hcov t (h.cap t ht hr g hg (hRA g hg')) hr with h' | h'
    · exact h'
    · exact absurd hg' (h' g hg)⟩

theorem ch_f (X : String) : ∀ j : Nat, Ch X (X ++ ":f" ++ toString j) :=
  fun j => ch_lit X ":f" j (by simp) (by simp)

section
variable (k)

/-! ## Children of the node kinds -/

def invKids (F : String) (v : Invoke) : List (String × List Triple) :=
  tokKids k F (F ++ ":nl") "namelist" v.namelist ++ paramKids k F v.params ++
    contentKids k F v.content ++ blockKids k F ":f" "finalize" 0 v.finalize ++
    extKids k F v.extensions

def ddKids (F : String) (dd : DoneData) : List (String × List Triple) :=
  contentKids k F dd.content ++ paramKids k F dd.params

/-- The emission of an `<initial>` element node at `F`, owned by state node `s`. -/
def initEmission (s : Term) (F : String) (ie : Initial) : List Triple :=
  k.node s "initialElement" F ["Initial"] ++
    k.transition ie.transition (k.iri F) "initialTransition" (F ++ ":t") 0 s s ++
    k.extensions (k.iri F) F ie.extensions

def initKids (s : Term) (F : String) (ie : Initial) : List (String × List Triple) :=
  (F ++ ":t", k.transition ie.transition (k.iri F) "initialTransition" (F ++ ":t") 0 s s) ::
    extKids k F ie.extensions

/-- The emission of an `onentry`/`onexit` handler node at `F`. -/
def hookEmission (s : Term) (prop cls F : String) (i : Nat) (b : List Exec) : List Triple :=
  k.node s prop F [cls] ++ ordT (k.iri F) i ++ k.block (k.iri F) "action" (F ++ ":a") 0 b

theorem decomp_dataItem (owner : Term) (of : String) (i : Nat) (d : Data) :
    Decomp k owner "data" (of ++ ":d" ++ toString i) (k.dataItem owner of i d)
      (extKids k (of ++ ":d" ++ toString i) d.extensions) := by
  constructor
  · intro t ht hr
    simp [Ctx.dataItem, extensions_eq] at ht ⊢
    grind (splits := 40)
  · exact ch_ext k _ _
  · exact (kidsL_ext k _ _).loc
  · exact nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _
  · intro d hd
    simp only [Ctx.dataItem, Ctx.node, List.cons_append, List.tail_cons]
    have := sub_flatMap hd; rw [← extensions_eq] at this; sub_tac

theorem decomp_invoke (owner : Term) (F : String) (i : Nat) (v : Invoke) :
    Decomp k owner "invoke" F (k.invoke owner F i v) (invKids k F v) := by
  constructor
  · intro t ht hr
    rcases hc : v.content with _ | c <;>
    simp [Ctx.invoke, hc, tokens_eq, params_eq, block_eq, extensions_eq, invKids,
      contentKids] at ht ⊢ <;>
    grind (splits := 80)
  · apply ch_of_fst
    simp only [invKids, List.map_append, fst_extKids, fst_tokKids, fst_paramKids,
      fst_contentKids, fst_blockKids, List.forall_mem_append, List.forall_mem_map]
    refine ⟨⟨⟨⟨?_, ?_⟩, ?_⟩, ?_⟩, ?_⟩
    · ch_tac
    · ch_tac
    · cases v.content <;> simp [Ch, String.toList_append]
    · ch_tac
    · ch_tac
  · exact kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsLoc_append k
      (kidsL_tok k _ _ _ _).loc (kidsL_param k _ _).loc) (kidsL_content k _ _).loc)
      (kidsL_block k _ _ _ _ _).loc) (kidsL_ext k _ _).loc
  · simp [invKids, List.nodup_append]; nd_tac
  · intro d hd
    simp only [Ctx.invoke, Ctx.node, List.cons_append, List.tail_cons]
    simp only [invKids, List.mem_append] at hd
    rcases hd with (((hd | hd) | hd) | hd) | hd
    · have := sub_flatMap hd; rw [← tokens_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← params_eq] at this; sub_tac
    · rcases hc : v.content with _ | c
      · simp [contentKids, hc] at hd
      · simp only [contentKids, hc, List.mem_singleton] at hd; subst hd
        sub_tac
    · have := sub_flatMap hd; rw [← block_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← extensions_eq] at this; sub_tac

theorem decomp_donedata (owner : Term) (F : String) (dd : DoneData) :
    Decomp k owner "donedata" F (k.donedata owner F dd) (ddKids k F dd) := by
  constructor
  · intro t ht hr
    rcases hc : dd.content with _ | c <;>
    simp [Ctx.donedata, hc, params_eq, ddKids, contentKids] at ht ⊢ <;>
    grind (splits := 40)
  · apply ch_of_fst
    simp only [ddKids, List.map_append, fst_paramKids, fst_contentKids, List.forall_mem_append,
      List.forall_mem_map]
    refine ⟨?_, ?_⟩
    · cases dd.content <;> simp [Ch, String.toList_append]
    · ch_tac
  · exact kidsLoc_append k (kidsL_content k _ _).loc (kidsL_param k _ _).loc
  · simp [ddKids, List.nodup_append]; nd_tac
  · intro d hd
    simp only [Ctx.donedata, Ctx.node, List.cons_append, List.tail_cons]
    simp only [ddKids, List.mem_append] at hd
    rcases hd with hd | hd
    · rcases hc : dd.content with _ | c
      · simp [contentKids, hc] at hd
      · simp only [contentKids, hc, List.mem_singleton] at hd; subst hd
        sub_tac
    · have := sub_flatMap hd; rw [← params_eq] at this; sub_tac

theorem decomp_hook (s : Term) (prop cls F : String) (i : Nat) (b : List Exec) :
    Decomp k s prop F (hookEmission k s prop cls F i b) (blockKids k F ":a" "action" 0 b) := by
  constructor
  · intro t ht hr
    simp [hookEmission, block_eq] at ht ⊢
    grind (splits := 40)
  · apply ch_of_fst; simp only [fst_blockKids, List.forall_mem_map]; ch_tac
  · exact (kidsL_block k _ _ _ _ _).loc
  · exact nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _
  · intro d hd
    simp only [hookEmission, Ctx.node, List.cons_append, List.tail_cons]
    have := sub_flatMap hd; rw [← block_eq] at this; sub_tac

theorem decomp_init (s : Term) (F : String) (ie : Initial) :
    Decomp k s "initialElement" F (initEmission k s F ie) (initKids k s F ie) := by
  constructor
  · intro t ht hr
    simp [initEmission, initKids, extensions_eq] at ht ⊢
    grind (splits := 40)
  · intro d hd
    simp only [initKids, List.mem_cons] at hd
    rcases hd with rfl | hd
    · simp [Ch, String.toList_append]
    · exact ch_ext k _ _ d hd
  · intro d hd
    simp only [initKids, List.mem_cons] at hd
    rcases hd with rfl | hd
    · exact (locL_transition k _ _ _ _ _ _ _).locU
    · exact kidsLoc_ext k _ _ d hd
  · simp only [initKids, List.map_cons, List.nodup_cons, fst_extKids]
    refine ⟨?_, nodup_zipIdx_map' _ (fun q r h => frag_inj _ _ _ _ h) _ _⟩
    simp [← String.toList_inj, String.toList_append]
  · intro d hd
    simp only [initEmission, Ctx.node, List.cons_append, List.tail_cons]
    simp only [initKids, List.mem_cons] at hd
    rcases hd with rfl | hd
    · sub_tac
    · have := sub_flatMap hd; rw [← extensions_eq] at this; sub_tac

/-- The triples about `k.iri f` in the emission of a child at `F` are exactly its link. -/
theorem self_locL {f F prop : String} {E : List Triple} (hL : LocL k (k.iri f) prop F E)
    (hch : Ch f F) (hlink : ⟨k.iri f, vt prop, k.iri F⟩ ∈ E) {P o : Term}
    (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ E ↔ P = vt prop ∧ o = k.iri F := by
  constructor
  · intro h
    rcases hL _ h hr with h | ⟨g, hg, hU⟩
    · simp only [Triple.mk.injEq] at h; exact ⟨h.2.1, h.2.2⟩
    · simp only [iri_inj] at hg; subst hg
      exact absurd hU (not_U_of_Ch hch)
  · rintro ⟨rfl, rfl⟩; exact hlink

theorem self_trans (f F prop : String) (hch : Ch f F) (tr : Transition) (order : Nat)
    (source scope : Term) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.transition tr (k.iri f) prop F order source scope ↔
      P = vt prop ∧ o = k.iri F :=
  self_locL k (locL_transition k tr _ _ _ _ _ _) hch (by simp [Ctx.transition]) hr

end

/-! ## Decoding -/

syntax "lkn" term "," term : tactic
macro_rules
  | `(tactic| lkn $h, $ho) => `(tactic|
    (intro o
     rw [($h).lookup (U_refl _) (by simp)]
     simp [Ctx.dataItem, Ctx.invoke, Ctx.donedata, hookEmission, initEmission, Ctx.transition,
       self_ext, self_tok _ _ _ _ (ch_nl _), self_param, self_content,
       self_block _ _ _ _ (ch_f _), self_block _ _ _ _ (ch_a _), contentKids, blockKids,
       tokKids, paramKids, extKids, strLit, intLit, boolLit, ($ho), (Ne.symm $ho)]))

theorem dec_dataItem {owner : Term} {of : String} {i : Nat} {d : Data}
    (hcap : Cap k ts (U (of ++ ":d" ++ toString i)) (k.dataItem owner of i d))
    (hown : Above (k := k) owner (of ++ ":d" ++ toString i)) :
    Decode.dataItem (Index.of ts) k.exprDt (k.iri (of ++ ":d" ++ toString i)) = .ok d ∧
      Lk ts (k.iri (of ++ ":d" ++ toString i)) "order" [intLit i] := by
  have cx : Ctxt k ts owner "data" _ _ _ := ⟨hcap, decomp_dataItem k owner of i d, hown⟩
  have hoX := above_ne hown
  refine ⟨?_, by lkn hcap, hoX⟩
  rcases d with ⟨id, src, ex, ct, exts⟩
  have h1 : Decode.string (Index.of ts) (k.iri (of ++ ":d" ++ toString i)) "dataId" = .ok id :=
    string_eq (by lkn hcap, hoX)
  have h2 : Decode.optString (Index.of ts) (k.iri (of ++ ":d" ++ toString i)) "src" = .ok src :=
    optString_eq (by lkn hcap, hoX)
  have h3 : Decode.optExpr (Index.of ts) k.exprDt (k.iri (of ++ ":d" ++ toString i)) "expr" =
      .ok ex := optExpr_eq (by lkn hcap, hoX)
  have h4 : Decode.optString (Index.of ts) (k.iri (of ++ ":d" ++ toString i)) "inlineContent" =
      .ok ct := optString_eq (by lkn hcap, hoX)
  have h5 : Decode.extensions (Index.of ts) (k.iri (of ++ ":d" ++ toString i)) = .ok exts :=
    dec_extensions cx exts (fun d hd => hd) (by lkn hcap, hoX)
  simp only [Decode.dataItem, h1, h2, h3, h4, h5, bind, Except.bind, pure, Except.pure]

theorem dec_invoke {owner : Term} {F : String} {i : Nat} {v : Invoke}
    (hcap : Cap k ts (U F) (k.invoke owner F i v)) (hown : Above (k := k) owner F)
    (fuel : Nat) (hf : ts.length ≤ fuel) :
    Decode.invoke (Index.of ts) k.exprDt fuel (k.iri F) = .ok v ∧
      Lk ts (k.iri F) "order" [intLit i] := by
  have cx : Ctxt k ts owner "invoke" _ _ _ := ⟨hcap, decomp_invoke k owner F i v, hown⟩
  have hoX := above_ne hown
  rcases v with ⟨a1, a2, a3, a4, a5, a6, nl, af, ps, ct, fin, exts⟩
  refine ⟨?_, by rcases ct <;> lkn hcap, hoX⟩
  have h1 : Decode.optString (Index.of ts) (k.iri F) "type" = .ok a1 :=
    optString_eq (by rcases ct <;> lkn hcap, hoX)
  have h2 : Decode.optExpr (Index.of ts) k.exprDt (k.iri F) "typeexpr" = .ok a2 :=
    optExpr_eq (by rcases ct <;> lkn hcap, hoX)
  have h3 : Decode.optString (Index.of ts) (k.iri F) "src" = .ok a3 :=
    optString_eq (by rcases ct <;> lkn hcap, hoX)
  have h4 : Decode.optExpr (Index.of ts) k.exprDt (k.iri F) "srcexpr" = .ok a4 :=
    optExpr_eq (by rcases ct <;> lkn hcap, hoX)
  have h5 : Decode.optString (Index.of ts) (k.iri F) "invokeId" = .ok a5 :=
    optString_eq (by rcases ct <;> lkn hcap, hoX)
  have h6 : Decode.optExpr (Index.of ts) k.exprDt (k.iri F) "idlocation" = .ok a6 :=
    optExpr_eq (by rcases ct <;> lkn hcap, hoX)
  have h7 : Decode.tokens (Index.of ts) (k.iri F) "namelist" = .ok nl :=
    dec_tokens cx (F ++ ":nl") "namelist" (fun a b h => frag_inj _ _ _ _ h)
      (fun j => by simp [← String.toList_inj, String.toList_append]) nl
      (fun d hd => by simp only [invKids, List.mem_append]; exact .inl (.inl (.inl (.inl hd))))
      (by rcases ct <;> lkn hcap, hoX)
  have h8 : Decode.boolean (Index.of ts) (k.iri F) "autoforward" = .ok af :=
    boolean_eq (by rcases ct <;> lkn hcap, hoX)
  have h9 : Decode.params (Index.of ts) k.exprDt (k.iri F) = .ok ps :=
    dec_params cx ps
      (fun d hd => by simp only [invKids, List.mem_append]; exact .inl (.inl (.inl (.inr hd))))
      (by rcases ct <;> lkn hcap, hoX)
  have h10 : Decode.content (Index.of ts) k.exprDt (k.iri F) = .ok ct :=
    dec_content cx ct
      (fun d hd => by simp only [invKids, List.mem_append]; exact .inl (.inl (.inr hd)))
      (by rcases ct <;> lkn hcap, hoX)
  have h11 : Decode.block (Index.of ts) k.exprDt fuel (k.iri F) "finalize" = .ok fin :=
    dec_block cx ":f" "finalize" (ch_f _) fin
      (fun p hp => by simp only [invKids, List.mem_append]; exact .inl (.inr (List.mem_map_of_mem hp)))
      (by rcases ct <;> lkn hcap, hoX) fuel hf
  have h12 : Decode.extensions (Index.of ts) (k.iri F) = .ok exts :=
    dec_extensions cx exts (fun d hd => by simp only [invKids, List.mem_append]; exact .inr hd)
      (by rcases ct <;> lkn hcap, hoX)
  simp [Decode.invoke, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, bind, Except.bind,
    pure, Except.pure]

theorem dec_donedata {owner : Term} {F : String} {dd : DoneData}
    (hcap : Cap k ts (U F) (k.donedata owner F dd)) (hown : Above (k := k) owner F) :
    Decode.content (Index.of ts) k.exprDt (k.iri F) = .ok dd.content ∧
      Decode.params (Index.of ts) k.exprDt (k.iri F) = .ok dd.params := by
  have cx : Ctxt k ts owner "donedata" _ _ _ := ⟨hcap, decomp_donedata k owner F dd, hown⟩
  have hoX := above_ne hown
  rcases dd with ⟨ct, ps⟩
  refine ⟨?_, ?_⟩
  · exact dec_content cx ct
      (fun d hd => by simp only [ddKids, List.mem_append]; exact .inl hd)
      (by rcases ct <;> lkn hcap, hoX)
  · exact dec_params cx ps
      (fun d hd => by simp only [ddKids, List.mem_append]; exact .inr hd)
      (by rcases ct <;> lkn hcap, hoX)

theorem dec_hook {s : Term} {prop cls F : String} {i : Nat} {b : List Exec}
    (hcap : Cap k ts (U F) (hookEmission k s prop cls F i b)) (hown : Above (k := k) s F)
    (fuel : Nat) (hf : ts.length ≤ fuel) :
    Decode.block (Index.of ts) k.exprDt fuel (k.iri F) "action" = .ok b ∧
      Lk ts (k.iri F) "order" [intLit i] := by
  have cx : Ctxt k ts s prop _ _ _ := ⟨hcap, decomp_hook k s prop cls F i b, hown⟩
  have hoX := above_ne hown
  exact ⟨dec_block cx ":a" "action" (ch_a _) b (fun p hp => List.mem_map_of_mem hp)
    (by lkn hcap, hoX) fuel hf, by lkn hcap, hoX⟩

theorem dec_init {s : Term} {F : String} {ie : Initial}
    (hcap : Cap k ts (U F) (initEmission k s F ie)) (hown : Above (k := k) s F)
    (hid : ∀ x ∈ ie.transition.targets, Lk ts (k.iri (stateFragment x)) "id" [strLit x])
    (fuel : Nat) (hf : ts.length ≤ fuel) :
    Decode.oneNode (Index.of ts) (k.iri F) "initialTransition" = .ok (k.iri (F ++ ":t")) ∧
      Decode.transition (Index.of ts) k.exprDt fuel (k.iri (F ++ ":t")) = .ok ie.transition ∧
      Decode.extensions (Index.of ts) (k.iri F) = .ok ie.extensions := by
  have cx : Ctxt k ts s "initialElement" _ _ _ := ⟨hcap, decomp_init k s F ie, hown⟩
  have hoX := above_ne hown
  have hT : Ch F (F ++ ":t") := by simp [Ch, String.toList_append]
  have hoT : k.iri F ≠ k.iri (F ++ ":t") := above_ne (above_of_ch hT)
  refine ⟨oneNode_eq ⟨_, rfl⟩ (by
      intro o
      rw [hcap.lookup (U_refl _) (by simp)]
      simp [initEmission, self_trans k F _ _ hT, self_ext, hoX, Ne.symm hoX]),
    dec_transition _ _ _ _ _ _ _ (cx.kid (List.mem_cons_self ..)) (above_of_ch hT) hid fuel hf,
    dec_extensions cx _ (fun d hd => List.mem_cons_of_mem _ hd) ?_⟩
  intro o
  rw [hcap.lookup (U_refl _) (by simp)]
  simp [initEmission, self_trans k F _ _ hT, self_ext, hoX, Ne.symm hoX]

end RT
end Vertep.Rdf
