import VertepLean.Rdf.RoundTrip.Cap
import VertepLean.WellFormed

/-!
# RDF round trip: locality of the projection

Every relevant triple a node's emission contains is its link from its owner or has a subject at
or below the node's fragment (`LocL`, `LocU`); for states, below the fragments of its subtree.
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

variable (k : Ctx)

@[simp] theorem mem_str {t : Triple} {n : Term} {p v : String} :
    t ∈ str n p v ↔ t = ⟨n, vt p, strLit v⟩ := by simp [str]
@[simp] theorem mem_optStr {t : Triple} {n : Term} {p : String} {v : Option String} :
    t ∈ optStr n p v ↔ ∃ x, v = some x ∧ t = ⟨n, vt p, strLit x⟩ := by
  cases v <;> simp [optStr, str]
@[simp] theorem mem_bool {t : Triple} {n : Term} {p : String} {b : Bool} :
    t ∈ bool n p b ↔ t = ⟨n, vt p, boolLit b⟩ := by simp [bool]
@[simp] theorem mem_int {t : Triple} {n : Term} {p : String} {v : Nat} :
    t ∈ int n p v ↔ t = ⟨n, vt p, intLit v⟩ := by simp [int]
@[simp] theorem mem_ordT {t : Triple} {n : Term} {v : Nat} :
    t ∈ ordT n v ↔ t = ⟨n, vt "order", intLit v⟩ := by simp [ordT, int]
@[simp] theorem mem_expr {t : Triple} {n : Term} {p v : String} :
    t ∈ k.expr n p v ↔ t = ⟨n, vt p, .lit (.typed v k.exprDt)⟩ := by simp [Ctx.expr]
@[simp] theorem mem_optExpr {t : Triple} {n : Term} {p : String} {v : Option String} :
    t ∈ k.optExpr n p v ↔ ∃ x, v = some x ∧ t = ⟨n, vt p, .lit (.typed x k.exprDt)⟩ := by
  cases v <;> simp [Ctx.optExpr, Ctx.expr]
@[simp] theorem mem_node {t : Triple} {owner : Term} {prop frag : String} {cs : List String} :
    t ∈ k.node owner prop frag cs ↔
      t = ⟨owner, vt prop, k.iri frag⟩ ∨ ∃ c ∈ cs, t = ⟨k.iri frag, rdfType, vt c⟩ := by
  simp [Ctx.node, eq_comm]
@[simp] theorem mem_extensionFields {t : Triple} {n : Term} {x : Extension} :
    t ∈ Ctx.extensionFields n x ↔ t = ⟨n, vt "namespace", strLit x.ns⟩ ∨
      t = ⟨n, vt "localName", strLit x.localName⟩ ∨
      (∃ v, x.value = .attribute v ∧ t = ⟨n, vt "attributeValue", strLit v⟩) ∨
      (∃ v, x.value = .element v ∧ t = ⟨n, vt "elementXml", strLit v⟩) ∨
      t = ⟨n, vt "position", intLit x.position⟩ := by
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp [Ctx.extensionFields]
@[simp] theorem mem_scriptFields {t : Triple} {n : Term} {s : Script} :
    t ∈ k.scriptFields n s ↔ (∃ x, s.src = some x ∧ t = ⟨n, vt "src", strLit x⟩) ∨
      (∃ x, s.body = some x ∧ t = ⟨n, vt "scriptBody", .lit (.typed x k.exprDt)⟩) := by
  simp [Ctx.scriptFields]

theorem U_app {f s : String} (h : s.toList.head? = some ':') (n : String) : U f (f ++ s ++ n) := by
  refine ⟨s.toList ++ n.toList, by simp [String.toList_append], ?_⟩
  cases hs : s.toList with
  | nil => simp [hs] at h
  | cons c l => simp [hs] at h; simp [h]

theorem U_app1 {f s : String} (h : s.toList.head? = some ':') : U f (f ++ s) := by
  refine ⟨s.toList, by simp [String.toList_append], ?_⟩
  cases hs : s.toList with
  | nil => simp [hs] at h
  | cons c l => simp [hs] at h; simp [h]

theorem locL_tokens (owner : Term) (prop pre : String) (f : String) (vs : List String)
    (hpre : ∀ j : Nat, U f (pre ++ toString j)) :
    LocU k owner f (k.tokens owner prop pre vs) := by
  intro t ht _
  simp only [Ctx.tokens, List.mem_flatMap] at ht
  obtain ⟨⟨v, j⟩, _, ht⟩ := ht
  simp [Ctx.node, ordT, int, str] at ht
  grind (splits := 60) [U_refl]


theorem colon_head (s : String) : (":" ++ s).toList.head? = some ':' := by
  simp [String.toList_append]

theorem locU_extensions (f : String) (xs : List Extension) :
    LocU k (k.iri f) f (k.extensions (k.iri f) f xs) := by
  intro t ht _
  simp only [Ctx.extensions, List.mem_flatMap] at ht
  obtain ⟨⟨x, j⟩, _, ht⟩ := ht
  have hU : U f (f ++ ":x" ++ toString j) := U_app (by simp) _
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp [Ctx.node, ordT, int, str, Ctx.extensionFields] at ht <;> grind (splits := 60) [U_refl]

theorem locU_params (f : String) (ps : List Param) :
    LocU k (k.iri f) f (k.params (k.iri f) f ps) := by
  intro t ht _
  simp only [Ctx.params, List.mem_flatMap] at ht
  obtain ⟨⟨x, j⟩, _, ht⟩ := ht
  have hU : U f (f ++ ":p" ++ toString j) := U_app (by simp) _
  rcases x with ⟨nm, e, l⟩
  cases e <;> cases l <;> simp [Ctx.node, ordT, int, str, Ctx.optExpr, Ctx.expr] at ht <;>
    grind (splits := 60) [U_refl]

theorem locL_content (f : String) (c : Content) :
    LocL k (k.iri f) "content" (f ++ ":c") (k.content (k.iri f) f c) := by
  intro t ht _
  rcases c with ⟨e, b⟩
  cases e <;> cases b <;> simp [Ctx.content, Ctx.node, str, optStr, Ctx.optExpr, Ctx.expr] at ht <;>
    grind (splits := 60) [U_refl]

theorem locU_content (f : String) (c : Content) :
    LocU k (k.iri f) f (k.content (k.iri f) f c) := by
  intro t ht hr
  have hU : U f (f ++ ":c") := U_app1 (by simp)
  rcases locL_content k f c t ht hr with rfl | ⟨g, hg, hg'⟩
  · exact .inl rfl
  · exact .inr ⟨g, hg, U_trans hU hg'⟩

theorem locU_targets (owner : Term) (o prop list refStep : String) (ids : List String)
    (scope : Term) (hr : ∀ j : Nat, U o (list ++ refStep ++ toString j)) :
    LocU k owner o (k.targets owner prop list refStep ids scope) := by
  intro t ht hrel
  simp only [Ctx.targets, List.mem_flatMap] at ht
  obtain ⟨⟨x, j⟩, _, ht⟩ := ht
  have hU := hr j
  simp only [List.mem_append, Ctx.node, ordT, int, List.mem_cons, List.mem_map,
    List.not_mem_nil, or_false] at ht
  rcases ht with ((((h | ⟨c, _, h⟩) | h) | h | h) | h)
  · exact .inl (by subst h; rfl)
  · exact .inr ⟨_, by subst h; rfl, hU⟩
  · exact .inr ⟨_, by subst h; rfl, hU⟩
  · exact .inr ⟨_, by subst h; rfl, hU⟩
  · exact .inr ⟨_, by subst h; rfl, hU⟩
  · exfalso
    split at h
    · simp at h
    · simp only [List.mem_flatMap] at h
      obtain ⟨_, _, h⟩ := h
      simp at h
      exact hrel.2 (by subst h; rfl)


mutual
theorem locL_exec (owner : Term) (prop frag : String) (order : Nat) :
    ∀ e, LocL k owner prop frag (k.exec owner prop frag order e)
  | .mk kind exts => by
    have h1 := locU_execKind frag kind
    have h2 := locU_extensions k frag exts
    intro t ht hr
    simp only [Ctx.exec, List.mem_append, mem_node, mem_ordT] at ht
    unfold LocU at h1 h2
    grind (splits := 60) [U_refl]
theorem locU_execKind (frag : String) :
    ∀ kind, LocU k (k.iri frag) frag (k.execKind (k.iri frag) frag kind)
  | .raise ev => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .send s => by
    have h1 := locL_tokens k (k.iri frag) "namelist" (frag ++ ":nl") frag s.namelist
      (fun j => U_app (by simp) _)
    have h2 := locU_params k frag s.params
    intro t ht hr
    unfold LocU at h1 h2
    rcases hc : s.content with _ | c
    · simp [Ctx.execKind, hc] at ht; grind (splits := 60) [U_refl]
    · have h3 := locU_content k frag c
      unfold LocU at h3
      simp [Ctx.execKind, hc] at ht; grind (splits := 60) [U_refl]
  | .cancel a b => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .log a b => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .assign a b c => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .script s => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .if_ bs => by
    have h := locU_branches frag 0 bs
    intro t ht hr; simp only [Ctx.execKind] at ht; exact h t ht hr
  | .foreach arr item idx body => by
    have h := locU_block frag ":a" "action" (by simp) 0 body
    intro t ht hr
    unfold LocU at h
    simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
  | .extension x => by
    intro t ht _; simp [Ctx.execKind] at ht; grind (splits := 60) [U_refl]
theorem locU_block (f s prop : String) (hs : s.toList.head? = some ':') :
    ∀ (i : Nat) (es : List Exec), LocU k (k.iri f) f (k.block (k.iri f) prop (f ++ s) i es)
  | _, [] => by intro t ht; simp [Ctx.block] at ht
  | i, e :: es => by
    have h1 := locL_exec (k.iri f) prop (f ++ s ++ toString i) i e
    have h2 := locU_block f s prop hs (i + 1) es
    have hU := U_app (f := f) hs (toString i)
    intro t ht hr
    simp only [Ctx.block, List.mem_append] at ht
    rcases ht with ht | ht
    · rcases h1 t ht hr with rfl | ⟨g, hg, hg'⟩
      · exact .inl rfl
      · exact .inr ⟨g, hg, U_trans hU hg'⟩
    · exact h2 t ht hr
theorem locU_branches (frag : String) :
    ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
      LocU k (k.iri frag) frag (k.branches (k.iri frag) frag j bs)
  | _, [] => by intro t ht; simp [Ctx.branches] at ht
  | j, (c, body) :: bs => by
    have h1 := locU_block (frag ++ ":b" ++ toString j) ":a" "action" (by simp) 0 body
    have h2 := locU_branches frag (j + 1) bs
    have hU : U frag (frag ++ ":b" ++ toString j) := U_app (by simp) _
    intro t ht hr
    unfold LocU at h1 h2
    simp [Ctx.branches] at ht
    grind (splits := 60) [U_refl, U_trans]
end


theorem locL_transition (tr : Transition) (owner : Term) (prop frag : String) (order : Nat)
    (source scope : Term) :
    LocL k owner prop frag (k.transition tr owner prop frag order source scope) := by
  have h1 := locL_tokens k (k.iri frag) "event" (frag ++ ":ev") frag tr.events
    (fun j => U_app (by simp) _)
  have h2 := locU_targets k (k.iri frag) frag "target" frag ":tg" tr.targets scope
    (fun j => U_app (by simp) _)
  have h3 := locU_block k frag ":a" "action" (by simp) 0 tr.body
  have h4 := locU_extensions k frag tr.extensions
  unfold LocU at h1 h2 h3 h4
  intro t ht hr
  simp [Ctx.transition] at ht
  grind (splits := 60) [U_refl]

theorem locL_dataItem (owner : Term) (ownerFrag : String) (i : Nat) (d : Data) :
    LocL k owner "data" (ownerFrag ++ ":d" ++ toString i) (k.dataItem owner ownerFrag i d) := by
  have h4 := locU_extensions k (ownerFrag ++ ":d" ++ toString i) d.extensions
  unfold LocU at h4
  intro t ht hr
  simp [Ctx.dataItem] at ht
  grind (splits := 60) [U_refl]

theorem locL_donedata (owner : Term) (frag : String) (dd : DoneData) :
    LocL k owner "donedata" frag (k.donedata owner frag dd) := by
  have h2 := locU_params k frag dd.params
  unfold LocU at h2
  intro t ht hr
  rcases hc : dd.content with _ | c
  · simp [Ctx.donedata, hc] at ht; grind (splits := 60) [U_refl]
  · have h3 := locU_content k frag c
    unfold LocU at h3
    simp [Ctx.donedata, hc] at ht; grind (splits := 60) [U_refl]

theorem locL_invoke (owner : Term) (frag : String) (order : Nat) (v : Invoke) :
    LocL k owner "invoke" frag (k.invoke owner frag order v) := by
  have h1 := locL_tokens k (k.iri frag) "namelist" (frag ++ ":nl") frag v.namelist
    (fun j => U_app (by simp) _)
  have h2 := locU_params k frag v.params
  have h3 := locU_block k frag ":f" "finalize" (by simp) 0 v.finalize
  have h4 := locU_extensions k frag v.extensions
  unfold LocU at h1 h2 h3 h4
  intro t ht hr
  rcases hc : v.content with _ | c
  · simp [Ctx.invoke, hc] at ht; grind (splits := 60) [U_refl]
  · have h5 := locU_content k frag c
    unfold LocU at h5
    simp [Ctx.invoke, hc] at ht; grind (splits := 60) [U_refl]

theorem locU_of_locL {owner : Term} {prop f o : String} {E : List Triple}
    (h : LocL k owner prop f E) (_ho : owner = k.iri o) (hU : U o f) : LocU k owner o E := by
  intro t ht hr
  rcases h t ht hr with rfl | ⟨g, hg, hg'⟩
  · exact .inl rfl
  · exact .inr ⟨g, hg, U_trans hU hg'⟩

/-- The region of a state: its subtree's fragments and everything below them. -/
def RS (s : State) (g : String) : Prop := ∃ Y ∈ s.flatten, U (stateFragment Y.id) g

/-- The region of sibling states. -/
def RSs (cs : List State) (g : String) : Prop := ∃ Y ∈ State.flattenList cs, U (stateFragment Y.id) g

mutual
theorem locS_state (order : Nat) (parent : Term) (ancestors : List Term) :
    ∀ s, ∀ t ∈ k.state order parent ancestors s, Rel t →
      t = ⟨parent, vt "child", k.iri (stateFragment s.id)⟩ ∨ ∃ g, t.s = k.iri g ∧ RS s g
  | .mk id idGen el initial initEl children ts onentry onexit data invokes dd exts => by
    have hc := locS_states 0 (k.iri (stateFragment id)) (ancestors ++ [k.iri (stateFragment id)])
      children
    suffices h : ∀ t ∈ k.state order parent ancestors
        (.mk id idGen el initial initEl children ts onentry onexit data invokes dd exts), Rel t →
        t = ⟨parent, vt "child", k.iri (stateFragment id)⟩ ∨
        (∃ g, t.s = k.iri g ∧ U (stateFragment id) g) ∨
        (∃ c ∈ children, t = ⟨k.iri (stateFragment id), vt "child", k.iri (stateFragment c.id)⟩) ∨
        ∃ g, t.s = k.iri g ∧ RSs children g by
      intro t ht hr
      have hself : State.mk id idGen el initial initEl children ts onentry onexit data invokes dd
          exts ∈ (State.mk id idGen el initial initEl children ts onentry onexit data invokes dd
          exts).flatten := by
        rw [State.flatten]; exact List.mem_cons_self
      have hkid : ∀ Y ∈ State.flattenList children, Y ∈ (State.mk id idGen el initial initEl
          children ts onentry onexit data invokes dd exts).flatten := by
        intro Y hY; rw [State.flatten]; exact List.mem_cons_of_mem _ hY
      rcases h t ht hr with h | ⟨g, hg, hU⟩ | ⟨c, _, h⟩ | ⟨g, hg, Y, hY, hU⟩
      · exact .inl h
      · exact .inr ⟨g, hg, _, hself, hU⟩
      · exact .inr ⟨stateFragment id, by rw [h], _, hself, U_refl _⟩
      · exact .inr ⟨g, hg, Y, hkid Y hY, hU⟩
    intro t ht hr
    simp only [Ctx.state] at ht
    generalize stateFragment id = frag at ht hc ⊢
    have htr : ∀ (tr : Transition) (i : Nat) (sc : Term) (x : Triple),
        x ∈ k.transition tr (k.iri frag) "transition" (frag ++ ":t" ++ toString i) i (k.iri frag) sc →
        Rel x → x.s = k.iri frag ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun tr i sc =>
      locU_of_locL k (locL_transition k tr _ _ _ i _ sc) rfl (U_app (by simp) _)
    have hhook : ∀ (seg : String) (hs : seg.toList.head? = some ':') (b : List Exec) (i : Nat)
        (x : Triple), x ∈ k.block (k.iri (frag ++ seg ++ toString i)) "action"
          (frag ++ seg ++ toString i ++ ":a") 0 b →
        Rel x → x.s = k.iri (frag ++ seg ++ toString i) ∨ ∃ g, x.s = k.iri g ∧ U frag g :=
      fun seg hs b i x hx hr => by
        rcases locU_block k (frag ++ seg ++ toString i) ":a" "action" (by simp) 0 b x hx hr with
          h | ⟨g, hg, hU⟩
        · exact .inl h
        · exact .inr ⟨g, hg, U_trans (U_app hs _) hU⟩
    have hen := hhook ":en" (by simp)
    have hex := hhook ":ex" (by simp)
    have hUen : ∀ i : Nat, U frag (frag ++ ":en" ++ toString i) := fun i => U_app (by simp) _
    have hUex : ∀ i : Nat, U frag (frag ++ ":ex" ++ toString i) := fun i => U_app (by simp) _
    have hda : ∀ (i : Nat) (d : Data) (x : Triple), x ∈ k.dataItem (k.iri frag) frag i d →
        Rel x → x.s = k.iri frag ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun i d =>
      locU_of_locL k (locL_dataItem k _ frag i d) rfl (U_app (by simp) _)
    have hin : ∀ (i : Nat) (v : Invoke) (x : Triple),
        x ∈ k.invoke (k.iri frag) (frag ++ ":inv" ++ toString i) i v →
        Rel x → x.s = k.iri frag ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun i v =>
      locU_of_locL k (locL_invoke k _ _ i v) rfl (U_app (by simp) _)
    have hdd : ∀ (d : DoneData) (x : Triple), x ∈ k.donedata (k.iri frag) (frag ++ ":dd") d →
        Rel x → x.s = k.iri frag ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun d =>
      locU_of_locL k (locL_donedata k _ _ d) rfl (U_app1 (by simp))
    have hx := locU_extensions k frag exts
    have hta := locU_targets k (k.iri frag) frag "initial" (frag ++ ":ia") "" initial (k.iri frag)
      (fun j => by simp [U, String.toList_append])
    have hit : ∀ (tr : Transition) (x : Triple),
        x ∈ k.transition tr (k.iri (frag ++ ":init")) "initialTransition" (frag ++ ":init" ++ ":t")
          0 (k.iri frag) (k.iri frag) →
        Rel x → x.s = k.iri (frag ++ ":init") ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun tr x hx hr => by
      rcases locL_transition k tr _ _ _ 0 _ _ x hx hr with h | ⟨g, hg, hU⟩
      · exact .inl (by rw [h])
      · exact .inr ⟨g, hg, U_trans (U_app (by simp) _) hU⟩
    have hix : ∀ (xs : List Extension) (x : Triple),
        x ∈ k.extensions (k.iri (frag ++ ":init")) (frag ++ ":init") xs →
        Rel x → x.s = k.iri (frag ++ ":init") ∨ ∃ g, x.s = k.iri g ∧ U frag g := fun xs x hx hr => by
      rcases locU_extensions k (frag ++ ":init") xs x hx hr with h | ⟨g, hg, hU⟩
      · exact .inl h
      · exact .inr ⟨g, hg, U_trans (U_app1 (by simp)) hU⟩
    have hUi : U frag (frag ++ ":init") := U_app1 (by simp)
    unfold LocU at hx hta
    rcases initEl with _ | ie <;> rcases dd with _ | d <;> rcases el with _ | _ | _ | h <;>
      simp at ht <;>
      grind (splits := 100) [U_refl, U_trans]
theorem locS_states (i : Nat) (parent : Term) (ancestors : List Term) :
    ∀ cs, ∀ t ∈ k.states i parent ancestors cs, Rel t →
      (∃ c ∈ cs, t = ⟨parent, vt "child", k.iri (stateFragment c.id)⟩) ∨ ∃ g, t.s = k.iri g ∧ RSs cs g
  | [] => by intro t ht; simp [Ctx.states] at ht
  | c :: cs => by
    have h1 := locS_state (order := i) parent ancestors c
    have h2 := locS_states (i + 1) parent ancestors cs
    intro t ht hr
    simp only [Ctx.states, List.mem_append] at ht
    rcases ht with ht | ht
    · rcases h1 t ht hr with h | ⟨g, hg, Y, hY, hU⟩
      · exact .inl ⟨c, by simp, h⟩
      · exact .inr ⟨g, hg, Y, by simp [State.flattenList]; exact .inl hY, hU⟩
    · rcases h2 t ht hr with ⟨c', hc', h⟩ | ⟨g, hg, Y, hY, hU⟩
      · exact .inl ⟨c', by simp [hc'], h⟩
      · exact .inr ⟨g, hg, Y, by simp [State.flattenList]; exact .inr hY, hU⟩
end

end RT
end Vertep.Rdf
