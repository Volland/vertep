import VertepLean.Rdf.RoundTrip.Decode
set_option linter.unusedSimpArgs false

/-!
# RDF round trip: target references and transitions
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

section
variable (k : Ctx)

/-- One target reference of `Ctx.targets` (its triples, selection triples included). -/
def refK (owner : Term) (prop list refStep : String) (scope : Term) (p : String × Nat) :
    String × List Triple :=
  (list ++ refStep ++ toString p.2,
    k.node owner prop (list ++ refStep ++ toString p.2) ["TargetRef"] ++
      ordT (k.iri (list ++ refStep ++ toString p.2)) p.2 ++
      [⟨k.iri (list ++ refStep ++ toString p.2), vt "state", k.iri (stateFragment p.1)⟩,
       ⟨k.iri (list ++ refStep ++ toString p.2), vt "scope", scope⟩] ++
      match k.chains.lookup p.1 with
      | none => []
      | some chain =>
        chain.zipIdx.flatMap fun ((x, parallel), i) =>
          let sel := k.iri (list ++ ":sel:" ++ x)
          let choice := match chain[i + 1]? with
            | none => k.iri (list ++ refStep ++ toString p.2)
            | some (c, _) => if parallel then k.iri x else k.iri c
          [⟨sel, vt "choice", choice⟩])

def refKids (owner : Term) (prop list refStep : String) (ids : List String) (scope : Term) :
    List (String × List Triple) :=
  ids.zipIdx.map (refK k owner prop list refStep scope)

theorem targets_eq (owner : Term) (prop list refStep : String) (ids : List String) (scope : Term) :
    k.targets owner prop list refStep ids scope =
      (refKids k owner prop list refStep ids scope).flatMap Prod.snd := by
  simp only [Ctx.targets, refKids, refK, List.flatMap_map]; rfl

theorem kidsL_ref (f prop list refStep : String) (ids : List String) (scope : Term) :
    KidsL k f prop (refKids k (k.iri f) prop list refStep ids scope) := by
  intro d hd
  simp only [refKids, refK, List.mem_map] at hd
  obtain ⟨⟨x, j⟩, _, rfl⟩ := hd
  refine ⟨fun t ht hr => ?_, by simp⟩
  simp only [List.mem_append, Ctx.node, ordT, int, List.mem_cons, List.mem_map,
    List.not_mem_nil, or_false] at ht
  rcases ht with ((((h | ⟨c, _, h⟩) | h) | h | h) | h)
  · exact .inl h
  · exact .inr ⟨_, by subst h; rfl, U_refl _⟩
  · exact .inr ⟨_, by subst h; rfl, U_refl _⟩
  · exact .inr ⟨_, by subst h; rfl, U_refl _⟩
  · exact .inr ⟨_, by subst h; rfl, U_refl _⟩
  · exfalso
    split at h
    · simp at h
    · simp only [List.mem_flatMap] at h
      obtain ⟨_, _, h⟩ := h
      simp at h
      exact hr.2 (by subst h; rfl)


@[simp] theorem fst_refKids (owner : Term) (prop list refStep : String) (ids : List String)
    (scope : Term) : (refKids k owner prop list refStep ids scope).map Prod.fst =
      ids.zipIdx.map (fun p => list ++ refStep ++ toString p.2) := by
  simp [refKids, refK, List.map_map, Function.comp_def]

def transKids (X : String) (tr : Transition) (scope : Term) : List (String × List Triple) :=
  tokKids k X (X ++ ":ev") "event" tr.events ++
    refKids k (k.iri X) "target" X ":tg" tr.targets scope ++
    blockKids k X ":a" "action" 0 tr.body ++ extKids k X tr.extensions

theorem decomp_transition (tr : Transition) (owner : Term) (prop X : String) (order : Nat)
    (source scope : Term) :
    Decomp k owner prop X (k.transition tr owner prop X order source scope) (transKids k X tr scope) := by
  constructor
  · intro t ht hr
    simp [Ctx.transition, tokens_eq, targets_eq, block_eq, extensions_eq, transKids] at ht ⊢
    grind (splits := 80)
  · apply ch_of_fst
    simp only [transKids, List.map_append, fst_extKids, fst_tokKids, fst_refKids, fst_blockKids,
      List.forall_mem_append, List.forall_mem_map]
    ch_tac
  · exact kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsL_tok k _ _ _ _).loc
      (kidsL_ref k _ _ _ _ _ _).loc) (kidsL_block k _ _ _ _ _).loc) (kidsL_ext k _ _).loc
  · simp [transKids, List.nodup_append]; nd_tac
  · intro d hd
    simp only [Ctx.transition, Ctx.node, List.cons_append, List.tail_cons]
    simp only [transKids, List.mem_append] at hd
    rcases hd with ((hd | hd) | hd) | hd
    · have := sub_flatMap hd; rw [← tokens_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← targets_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← block_eq] at this; sub_tac
    · have := sub_flatMap hd; rw [← extensions_eq] at this; sub_tac

theorem self_ref (f prop list refStep : String) (hs : ∀ j : Nat, Ch f (list ++ refStep ++ toString j))
    (ids : List String) (scope : Term) {P o : Term} (hr : Rel ⟨k.iri f, P, o⟩) :
    ⟨k.iri f, P, o⟩ ∈ k.targets (k.iri f) prop list refStep ids scope ↔
      P = vt prop ∧ o ∈ (refKids k (k.iri f) prop list refStep ids scope).map (fun d => k.iri d.1) := by
  rw [targets_eq]
  refine self_kids k (kidsL_ref k f prop list refStep ids scope) ?_ hr
  apply ch_of_fst; simp only [fst_refKids, List.forall_mem_map]; intro p _; exact hs p.2


theorem mem_refK {owner : Term} {prop list refStep : String} {scope : Term} {p : String × Nat}
    {t : Triple} (hc : t.p ≠ vt "choice") :
    t ∈ (refK k owner prop list refStep scope p).2 ↔
      t ∈ k.node owner prop (list ++ refStep ++ toString p.2) ["TargetRef"] ++
        ordT (k.iri (list ++ refStep ++ toString p.2)) p.2 ++
        [⟨k.iri (list ++ refStep ++ toString p.2), vt "state", k.iri (stateFragment p.1)⟩,
         ⟨k.iri (list ++ refStep ++ toString p.2), vt "scope", scope⟩] := by
  simp only [refK, List.mem_append]
  constructor
  · rintro (h | h)
    · exact h
    · exfalso
      split at h
      · simp at h
      · simp only [List.mem_flatMap] at h
        obtain ⟨_, _, h⟩ := h
        simp at h
        exact hc (by subst h; rfl)
  · intro h; exact .inl h

end

variable {k : Ctx} {ts : List Triple}

theorem dec_block {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (s bprop : String)
    (hs : ∀ j : Nat, Ch X (X ++ s ++ toString j)) (es : List Exec)
    (hsub : ∀ p ∈ es.zipIdx 0, blockK k X s bprop p ∈ D)
    (hself : Lk ts (k.iri X) bprop ((blockKids k X s bprop 0 es).map (fun d => k.iri d.1)))
    (fuel : Nat) (hf : ts.length ≤ fuel) :
    Decode.block (Index.of ts) k.exprDt fuel (k.iri X) bprop = .ok es := by
  have hmem : ∀ p ∈ es.zipIdx 0, Cap k ts (U (blockK k X s bprop p).1) (blockK k X s bprop p).2 ∧
      (blockK k X s bprop p).2.length ≤ fuel := fun p hp => by
    have h := c.kid (hsub p hp)
    exact ⟨h, Nat.le_trans h.sub.length_le hf⟩
  unfold Decode.block
  rw [ordered_kids es (blockK k X s bprop) hself (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
    (fun p hp => ord_exec (hmem p hp).1 (above_ne (above_of_ch (hs p.2))))]
  simp only [bind, Except.bind]
  exact dec_blockMembers fuel X s bprop hs es 0 hmem

theorem dec_targets {owner : Term} {prop X : String} {E : List Triple}
    {D : List (String × List Triple)} (c : Ctxt k ts owner prop X E D) (tprop list refStep : String)
    (hs : ∀ j : Nat, Ch X (list ++ refStep ++ toString j)) (ids : List String) (scope : Term)
    (hsub : ∀ p ∈ ids.zipIdx, refK k (k.iri X) tprop list refStep scope p ∈ D)
    (hself : Lk ts (k.iri X) tprop
      ((refKids k (k.iri X) tprop list refStep ids scope).map (fun d => k.iri d.1)))
    (hid : ∀ x ∈ ids, Lk ts (k.iri (stateFragment x)) "id" [strLit x]) :
    Decode.targetIds (Index.of ts) (k.iri X) tprop = .ok ids := by
  have hcap : ∀ p ∈ ids.zipIdx, Cap k ts (U (list ++ refStep ++ toString p.2))
      (refK k (k.iri X) tprop list refStep scope p).2 := fun p hp => c.kid (hsub p hp)
  have hlk : ∀ p ∈ ids.zipIdx, ∀ (P : String) (o : Term), P ≠ "choice" → P ≠ "historyDefault" →
      (⟨k.iri (list ++ refStep ++ toString p.2), vt P, o⟩ ∈ ts ↔
        ⟨k.iri (list ++ refStep ++ toString p.2), vt P, o⟩ ∈
          k.node (k.iri X) tprop (list ++ refStep ++ toString p.2) ["TargetRef"] ++
          ordT (k.iri (list ++ refStep ++ toString p.2)) p.2 ++
          [⟨k.iri (list ++ refStep ++ toString p.2), vt "state", k.iri (stateFragment p.1)⟩,
           ⟨k.iri (list ++ refStep ++ toString p.2), vt "scope", scope⟩]) := by
    intro p hp P o h1 h2
    rw [(hcap p hp).lookup (U_refl _) (by simp [h1, h2]), mem_refK k (by simpa using h1)]
  have hoX : ∀ j : Nat, k.iri X ≠ k.iri (list ++ refStep ++ toString j) := fun j =>
    above_ne (above_of_ch (hs j))
  have hXne : ∀ j : Nat, list ++ refStep ++ Nat.repr j ≠ X := fun j h =>
    hoX j (by rw [show list ++ refStep ++ toString j = X from h])
  unfold Decode.targetIds
  rw [ordered_kids ids _ hself (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
    (fun p hp => by
      intro o
      show ⟨k.iri (list ++ refStep ++ toString p.2), _, _⟩ ∈ ts ↔ _
      rw [hlk p hp "order" o (by decide) (by decide)]; simp [hXne, intLit])]
  simp only [bind, Except.bind]
  refine (mapM_kids (k := k) _ _ id ids 0 ?_).trans (by simp)
  intro p hp
  have hp' : p.1 ∈ ids := (List.mem_zipIdx hp).2.2 ▸ List.getElem_mem _
  have h1 : Decode.oneNode (Index.of ts) (k.iri (list ++ refStep ++ toString p.2)) "state" =
      .ok (k.iri (stateFragment p.1)) :=
    oneNode_eq ⟨_, rfl⟩ (by intro o; rw [hlk p hp "state" o (by decide) (by decide)]; simp [hXne])
  simp only [refK, h1, bind, Except.bind, string_eq (by simpa [litOf, strLit] using hid _ hp'), id]


theorem ch_seg (X s : String) (h : s.toList.head? = some ':') (hn : NC s.toList.tail) :
    ∀ j : Nat, Ch X (X ++ s ++ toString j) := fun j => ch_lit X s j h hn

syntax "lkt" term "," term : tactic
macro_rules
  | `(tactic| lkt $h, $ho) => `(tactic|
    (intro o
     rw [($h).lookup (U_refl _) (by simp)]
     simp [Ctx.transition, self_ext, self_tok _ _ _ _ (ch_seg _ ":ev" (by simp) (by simp)),
       self_ref _ _ _ _ _ (ch_seg _ ":tg" (by simp) (by simp)),
       self_block _ _ _ _ (ch_a _), blockKids, refKids, tokKids, strLit, intLit, boolLit, ($ho),
       (Ne.symm $ho)]))

/-- **Decoding inverts projecting a transition**, given the `vt:id` of each target state. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "transitions decoded from any graph capturing their projection"}
theorem dec_transition (tr : Transition) (owner : Term) (prop X : String) (order : Nat)
    (source scope : Term) (hcap : Cap k ts (U X) (k.transition tr owner prop X order source scope))
    (hown : Above (k := k) owner X)
    (hid : ∀ x ∈ tr.targets, Lk ts (k.iri (stateFragment x)) "id" [strLit x])
    (fuel : Nat) (hf : ts.length ≤ fuel) :
    Decode.transition (Index.of ts) k.exprDt fuel (k.iri X) = .ok tr := by
  have hD := decomp_transition k tr owner prop X order source scope
  have cx : Ctxt k ts owner prop X _ _ := ⟨hcap, hD, hown⟩
  have hoX := above_ne hown
  have h1 : Decode.string (Index.of ts) (k.iri X) "transitionType" =
      .ok (match tr.type with | .external => "external" | .internal => "internal") := by
    cases hty : tr.type <;> exact string_eq (by
      intro o; rw [hcap.lookup (U_refl _) (by simp)]
      simp [Ctx.transition, hty, self_ext, self_tok _ _ _ _ (ch_seg _ ":ev" (by simp) (by simp)),
        self_ref _ _ _ _ _ (ch_seg _ ":tg" (by simp) (by simp)), self_block _ _ _ _ (ch_a _),
        strLit, hoX, Ne.symm hoX])
  rcases tr with ⟨evs, cond, tgs, ty, tyx, body, exts⟩
  have h2 : Decode.tokens (Index.of ts) (k.iri X) "event" = .ok evs :=
    dec_tokens cx (X ++ ":ev") "event" (fun a b h => frag_inj _ _ _ _ h)
      (fun j => by simp [← String.toList_inj, String.toList_append]) evs
      (fun d hd => by simp only [transKids, List.mem_append]; exact .inl (.inl (.inl hd)))
      (by lkt hcap, hoX)
  have h3 : Decode.optExpr (Index.of ts) k.exprDt (k.iri X) "cond" = .ok cond :=
    optExpr_eq (by lkt hcap, hoX)
  have h4 : Decode.targetIds (Index.of ts) (k.iri X) "target" = .ok tgs :=
    dec_targets cx "target" X ":tg" (ch_seg _ ":tg" (by simp) (by simp)) tgs scope
      (fun p hp => by simp only [transKids, List.mem_append]; exact .inl (.inl (.inr (List.mem_map_of_mem hp))))
      (by lkt hcap, hoX) hid
  have h5 : Decode.boolean (Index.of ts) (k.iri X) "transitionTypeExplicit" = .ok tyx :=
    boolean_eq (by lkt hcap, hoX)
  have h6 : Decode.block (Index.of ts) k.exprDt fuel (k.iri X) "action" = .ok body :=
    dec_block cx ":a" "action" (ch_a _) body
      (fun p hp => by simp only [transKids, List.mem_append]; exact .inl (.inr (List.mem_map_of_mem hp)))
      (by lkt hcap, hoX) fuel hf
  have h7 : Decode.extensions (Index.of ts) (k.iri X) = .ok exts :=
    dec_extensions cx exts (fun d hd => by simp only [transKids, List.mem_append]; exact .inr hd)
      (by lkt hcap, hoX)
  cases ty <;> simp [Decode.transition, h1, h2, h3, h4, h5, h6, h7, bind, Except.bind, pure,
    Except.pure]

end RT
end Vertep.Rdf
