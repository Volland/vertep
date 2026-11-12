import VertepLean.Rdf.Equiv.Nodes
import VertepLean.Rdf.RoundTrip.Chart
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

/-!
# Global facts about the projection

Properties of every triple of `project c base`, proven by induction over the projector:
predicates are `rdf:type` or `vt:` IRIs (so there is no `rdfs:subClassOf`), and the objects of
`vt:initial` are typed `vt:TargetRef`.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-- Every triple of a list satisfies `Q`. -/
def AllT (Q : Triple → Prop) (E : List Triple) : Prop := ∀ t ∈ E, Q t

section AllT
variable {Q : Triple → Prop}
@[simp] theorem allT_nil : AllT Q [] := by simp [AllT]
@[simp] theorem allT_cons {t : Triple} {E : List Triple} : AllT Q (t :: E) ↔ Q t ∧ AllT Q E := by
  simp [AllT]
@[simp] theorem allT_append {a b : List Triple} : AllT Q (a ++ b) ↔ AllT Q a ∧ AllT Q b := by
  simp only [AllT, List.mem_append]
  exact ⟨fun h => ⟨fun t ht => h t (.inl ht), fun t ht => h t (.inr ht)⟩,
    fun ⟨h1, h2⟩ t ht => ht.elim (h1 t) (h2 t)⟩
@[simp] theorem allT_flatMap {α : Type} {l : List α} {f : α → List Triple} :
    AllT Q (l.flatMap f) ↔ ∀ x ∈ l, AllT Q (f x) := by
  simp only [AllT, List.mem_flatMap]
  exact ⟨fun h x hx t ht => h t ⟨x, hx, ht⟩, fun h t ⟨x, hx, ht⟩ => h x hx t ht⟩
@[simp] theorem allT_map {α : Type} {l : List α} {f : α → Triple} :
    AllT Q (l.map f) ↔ ∀ x ∈ l, Q (f x) := by simp [AllT]
end AllT

/-! ## Predicates -/

/-- The predicate is `rdf:type` or a `vt:` IRI. -/
def PF (t : Triple) : Prop := t.p = rdfType ∨ ∃ l, t.p = vt l

section
variable (k : Ctx)

@[simp] theorem pf_mk_vt (s o : Term) (l : String) : PF ⟨s, vt l, o⟩ := .inr ⟨l, rfl⟩
@[simp] theorem pf_mk_type (s o : Term) : PF ⟨s, rdfType, o⟩ := .inl rfl

@[simp] theorem pf_str (n : Term) (p v : String) : AllT PF (str n p v) := by simp [str]
@[simp] theorem pf_optStr (n : Term) (p : String) (v : Option String) : AllT PF (optStr n p v) := by
  cases v <;> simp [optStr, str]
@[simp] theorem pf_bool (n : Term) (p : String) (b : Bool) : AllT PF (bool n p b) := by simp [bool]
@[simp] theorem pf_int (n : Term) (p : String) (v : Nat) : AllT PF (int n p v) := by simp [int]
@[simp] theorem pf_ordT (n : Term) (v : Nat) : AllT PF (ordT n v) := by simp [ordT, int]
@[simp] theorem pf_expr (n : Term) (p v : String) : AllT PF (k.expr n p v) := by simp [Ctx.expr]
@[simp] theorem pf_optExpr (n : Term) (p : String) (v : Option String) : AllT PF (k.optExpr n p v) := by
  cases v <;> simp [Ctx.optExpr, Ctx.expr]
@[simp] theorem pf_node (o : Term) (p f : String) (cs : List String) : AllT PF (k.node o p f cs) := by
  simp [Ctx.node]
@[simp] theorem pf_extensionFields (n : Term) (x : Extension) : AllT PF (Ctx.extensionFields n x) := by
  rcases x with ⟨ns, ln, v, pos⟩; cases v <;> simp [Ctx.extensionFields]
@[simp] theorem pf_extensions (o : Term) (f : String) (xs : List Extension) :
    AllT PF (k.extensions o f xs) := by simp [Ctx.extensions]
@[simp] theorem pf_tokens (o : Term) (p pre : String) (vs : List String) :
    AllT PF (k.tokens o p pre vs) := by simp [Ctx.tokens]
@[simp] theorem pf_params (o : Term) (f : String) (ps : List Param) : AllT PF (k.params o f ps) := by
  simp [Ctx.params]
@[simp] theorem pf_content (o : Term) (f : String) (c : Content) : AllT PF (k.content o f c) := by
  simp [Ctx.content]
@[simp] theorem pf_scriptFields (n : Term) (s : Script) : AllT PF (k.scriptFields n s) := by
  simp [Ctx.scriptFields]
@[simp] theorem pf_targets (o : Term) (p l r : String) (ids : List String) (sc : Term) :
    AllT PF (k.targets o p l r ids sc) := by
  simp only [Ctx.targets, allT_flatMap]
  intro x _
  simp only [allT_append, pf_node, pf_ordT, allT_cons, pf_mk_vt, allT_nil, and_self, true_and]
  split
  · simp
  · simp

mutual
theorem pf_exec (o : Term) (p f : String) (i : Nat) : ∀ e, AllT PF (k.exec o p f i e)
  | .mk kind exts => by
    have := pf_execKind (k.iri f) f kind
    simp [Ctx.exec, this]
theorem pf_execKind (n : Term) (f : String) : ∀ kind, AllT PF (k.execKind n f kind)
  | .raise _ => by simp [Ctx.execKind]
  | .send s => by simp [Ctx.execKind]; split <;> simp
  | .cancel .. => by simp [Ctx.execKind]
  | .log .. => by simp [Ctx.execKind]
  | .assign .. => by simp [Ctx.execKind]
  | .script _ => by simp [Ctx.execKind]
  | .if_ bs => by simp only [Ctx.execKind]; exact pf_branches n f 0 bs
  | .foreach _ _ _ body => by
    have := pf_block n "action" (f ++ ":a") 0 body
    simp [Ctx.execKind, this]
  | .extension _ => by simp [Ctx.execKind]
theorem pf_block (o : Term) (p pre : String) : ∀ (i : Nat) (es : List Exec), AllT PF (k.block o p pre i es)
  | _, [] => by simp [Ctx.block]
  | i, e :: es => by
    have h1 := pf_exec o p (pre ++ toString i) i e
    have h2 := pf_block o p pre (i + 1) es
    simp only [Ctx.block, allT_append]; exact ⟨h1, h2⟩
theorem pf_branches (n : Term) (f : String) : ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
    AllT PF (k.branches n f j bs)
  | _, [] => by simp [Ctx.branches]
  | j, (c, body) :: bs => by
    have h1 := pf_block (k.iri (f ++ ":b" ++ toString j)) "action" (f ++ ":b" ++ toString j ++ ":a") 0 body
    have h2 := pf_branches n f (j + 1) bs
    simp [Ctx.branches, h2]; simpa using h1
end

@[simp] theorem pf_transition (tr : Transition) (o : Term) (p f : String) (i : Nat) (src sc : Term) :
    AllT PF (k.transition tr o p f i src sc) := by
  have := pf_block k (k.iri f) "action" (f ++ ":a") 0 tr.body
  simp [Ctx.transition, this]

@[simp] theorem pf_dataItem (o : Term) (f : String) (i : Nat) (d : Data) : AllT PF (k.dataItem o f i d) := by
  simp [Ctx.dataItem]

@[simp] theorem pf_donedata (o : Term) (f : String) (dd : DoneData) : AllT PF (k.donedata o f dd) := by
  simp [Ctx.donedata]; split <;> simp

@[simp] theorem pf_invoke (o : Term) (f : String) (i : Nat) (v : Invoke) : AllT PF (k.invoke o f i v) := by
  have := pf_block k (k.iri f) "finalize" (f ++ ":f") 0 v.finalize
  simp [Ctx.invoke]; split <;> simpa using this

mutual
theorem pf_state (order : Nat) (parent : Term) (ancestors : List Term) :
    ∀ s, AllT PF (k.state order parent ancestors s)
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts => by
    have hc := pf_states 0 (k.iri (stateFragment id)) (ancestors ++ [k.iri (stateFragment id)])
      children
    have hb : ∀ (seg : String) (b : List Exec) (i : Nat),
        AllT PF (k.block (k.iri (stateFragment id ++ seg ++ Nat.repr i)) "action"
          (stateFragment id ++ seg ++ Nat.repr i ++ ":a") 0 b) := fun seg b i => pf_block k _ _ _ 0 b
    rcases initEl with _ | ie <;> rcases dd with _ | d <;> rcases el with _ | _ | _ | h <;>
      simp [Ctx.state, hc, hb]
theorem pf_states (i : Nat) (parent : Term) (ancestors : List Term) :
    ∀ cs, AllT PF (k.states i parent ancestors cs)
  | [] => by simp [Ctx.states]
  | c :: cs => by
    have h1 := pf_state i parent ancestors c
    have h2 := pf_states (i + 1) parent ancestors cs
    simp only [Ctx.states, allT_append]; exact ⟨h1, h2⟩
end

mutual
theorem pf_hd (parent : Option State) : ∀ s, AllT PF (historyDefaults k parent s)
  | .mk id ig el ini ie children trs en ex da inv dd xs => by
    have h := pf_hds (some (.mk id ig el ini ie children trs en ex da inv dd xs)) children
    simp only [historyDefaults, allT_append]
    refine ⟨?_, h⟩
    cases el <;> simp [AllT]
    intro t _ _ _ ht; rw [← ht]; exact .inr ⟨_, rfl⟩
theorem pf_hds (parent : Option State) : ∀ cs, AllT PF (historyDefaultsList k parent cs)
  | [] => by simp [historyDefaultsList]
  | c :: cs => by
    simp only [historyDefaultsList, allT_append]; exact ⟨pf_hd parent c, pf_hds parent cs⟩
end

end

theorem pf_project (c : Chart) (base : String) : AllT PF (project c base) := by
  unfold project
  rcases indexStates [("chart", false)] c.states with ⟨a, b⟩
  simp only [allT_append]
  refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨by simp, by simp⟩, by simp⟩, by simp⟩, by simp⟩, by simp⟩, by simp⟩, ?_⟩, ?_⟩,
    pf_states _ _ _ _ _⟩, by simp⟩, pf_hds _ _ _⟩
  · simp
  · cases c.script <;> simp

/-- The projection has no `rdfs:subClassOf` triples. -/
theorem subClassOf_eq : Vocab.rdfs.subClassOf = ⟨"http://www.w3.org/2000/01/rdf-schema#subClassOf"⟩ := rfl

/-- The projection has no `rdfs:subClassOf` triples. -/
theorem noSub (c : Chart) (base : String) : NoSub (toRDF c base) := by
  intro s o h
  rw [toRDF, Graph.mem_ofList] at h
  rcases pf_project c base _ h with h | ⟨l, h⟩
  · simp only [subClassOf_eq, rdfType, Term.iri.injEq, IRI.mk.injEq] at h
    have := congrArg String.toList h
    simp at this
  · simp only [vt, Term.iri.injEq, subClassOf_eq, IRI.mk.injEq] at h
    have := congrArg (fun s => s.toList.take 5) h
    simp [vtNs, String.toList_append] at this


/-! ## Containment links -/

/-- The parent node of a state: the chart node or the parent state's node. -/
def parT (k : Ctx) : Option State → Term
  | none => k.iri "chart"
  | some p => k.iri (stateFragment p.id)

/-- A containment triple links a node to a child fragment (`Ch`), or a parent to a child state
given by a parent–child pair of `P`. -/
def LinkOK (k : Ctx) (P : List (Option State × State)) (t : Triple) : Prop :=
  (∃ l ∈ containers, t.p = vt l) → ∃ f, t.o = k.iri f ∧
    ((∃ g, Ch g f ∧ t.s = k.iri g) ∨ ∃ q ∈ P, f = stateFragment q.2.id ∧ t.s = parT k q.1)

theorem ch_lit' (f s : String) (h : s.toList.head? = some ':') (hn : NC s.toList.tail) :
    Ch f (f ++ s) := by
  cases hs : s.toList with
  | nil => simp [hs] at h
  | cons c l =>
    simp [hs] at h hn; subst h
    exact ⟨l, by simp [String.toList_append, hs], hn⟩

section
variable (k : Ctx) (P : List (Option State × State))

theorem linkOK_of_ne {t : Triple} (h : ∀ l ∈ containers, t.p ≠ vt l) : LinkOK k P t :=
  fun ⟨l, hl, he⟩ => absurd he (h l hl)

theorem linkOK_mono {P P' : List (Option State × State)} (hP : ∀ q ∈ P, q ∈ P') {t : Triple}
    (h : LinkOK k P t) : LinkOK k P' t := by
  intro hc
  obtain ⟨f, hf, h⟩ := h hc
  exact ⟨f, hf, h.imp_right fun ⟨q, hq, h⟩ => ⟨q, hP q hq, h⟩⟩

@[simp] theorem link_type (s o : Term) : LinkOK k P ⟨s, rdfType, o⟩ :=
  linkOK_of_ne k P (fun l _ => (vt_ne_rdfType l).symm)

theorem link_vt (s o : Term) (l : String) (h : l ∉ containers) : LinkOK k P ⟨s, vt l, o⟩ :=
  linkOK_of_ne k P (fun l' hl' he => h (by simp only [vt_inj] at he; exact he ▸ hl'))

syntax "lnk" : tactic
macro_rules
  | `(tactic| lnk) => `(tactic| (apply link_vt; simp [containers]))

theorem link_str (n : Term) (p v : String) (h : p ∉ containers) : AllT (LinkOK k P) (str n p v) := by
  simp [str, link_vt k P _ _ _ h]
theorem link_optStr (n : Term) (p : String) (v : Option String) (h : p ∉ containers) :
    AllT (LinkOK k P) (optStr n p v) := by
  cases v <;> simp [optStr, str, link_vt k P _ _ _ h]
theorem link_bool (n : Term) (p : String) (b : Bool) (h : p ∉ containers) :
    AllT (LinkOK k P) (bool n p b) := by simp [bool, link_vt k P _ _ _ h]
theorem link_ordT (n : Term) (v : Nat) : AllT (LinkOK k P) (ordT n v) := by
  simp [ordT, int]; lnk
theorem link_expr (n : Term) (p v : String) (h : p ∉ containers) : AllT (LinkOK k P) (k.expr n p v) := by
  simp [Ctx.expr, link_vt k P _ _ _ h]
theorem link_optExpr (n : Term) (p : String) (v : Option String) (h : p ∉ containers) :
    AllT (LinkOK k P) (k.optExpr n p v) := by
  cases v <;> simp [Ctx.optExpr, Ctx.expr, link_vt k P _ _ _ h]
theorem link_node (o p f : String) (cs : List String) (h : Ch o f) :
    AllT (LinkOK k P) (k.node (k.iri o) p f cs) := by
  simp only [Ctx.node, allT_cons, allT_map, link_type, implies_true, and_true]
  exact fun _ => ⟨f, rfl, .inl ⟨o, h, rfl⟩⟩
theorem link_extensionFields (n : Term) (x : Extension) : AllT (LinkOK k P) (Ctx.extensionFields n x) := by
  rcases x with ⟨ns, ln, v, pos⟩
  cases v <;> simp only [Ctx.extensionFields, str, int, List.cons_append, List.nil_append,
    allT_cons, allT_nil, and_true] <;> refine ⟨?_, ?_, ?_, ?_⟩ <;> lnk
theorem link_extensions (f : String) (xs : List Extension) :
    AllT (LinkOK k P) (k.extensions (k.iri f) f xs) := by
  simp only [Ctx.extensions, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨link_node k P _ _ _ _ (ch_lit f ":x" _ (by simp) (by simp)), link_ordT k P _ _⟩,
    link_extensionFields k P _ _⟩
theorem link_tokens (f prop s : String) (hs : ∀ j : Nat, Ch f (f ++ s ++ toString j))
    (vs : List String) : AllT (LinkOK k P) (k.tokens (k.iri f) prop (f ++ s) vs) := by
  simp only [Ctx.tokens, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨link_node k P _ _ _ _ (hs _), link_ordT k P _ _⟩, link_str k P _ _ _ (by simp [containers])⟩
theorem link_params (f : String) (ps : List Param) : AllT (LinkOK k P) (k.params (k.iri f) f ps) := by
  simp only [Ctx.params, allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨⟨⟨link_node k P _ _ _ _ (ch_lit f ":p" _ (by simp) (by simp)), link_ordT k P _ _⟩,
    link_str k P _ _ _ (by simp [containers])⟩, link_optExpr k P _ _ _ (by simp [containers])⟩,
    link_optExpr k P _ _ _ (by simp [containers])⟩
theorem link_content (f : String) (c : Content) : AllT (LinkOK k P) (k.content (k.iri f) f c) := by
  simp only [Ctx.content, allT_append]
  exact ⟨⟨link_node k P _ _ _ _ (ch_lit' f ":c" (by simp) (by simp)),
    link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩
theorem link_scriptFields (n : Term) (sc : Script) : AllT (LinkOK k P) (k.scriptFields n sc) := by
  simp only [Ctx.scriptFields, allT_append]
  exact ⟨link_optStr k P _ _ _ (by simp [containers]), link_optExpr k P _ _ _ (by simp [containers])⟩
theorem link_targets (o prop list r : String) (hr : ∀ j : Nat, Ch o (list ++ r ++ toString j))
    (ids : List String) (sc : Term) :
    AllT (LinkOK k P) (k.targets (k.iri o) prop list r ids sc) := by
  simp only [Ctx.targets, allT_flatMap]
  intro x _
  simp only [allT_append, allT_cons, allT_nil, and_true]
  refine ⟨⟨⟨link_node k P _ _ _ _ (hr _), link_ordT k P _ _⟩, by lnk, by lnk⟩, ?_⟩
  split
  · simp
  · simp only [allT_flatMap, allT_cons, allT_nil, and_true]
    intro _ _; lnk

mutual
theorem link_exec (o p f : String) (i : Nat) (h : Ch o f) : ∀ e, AllT (LinkOK k P) (k.exec (k.iri o) p f i e)
  | .mk kind exts => by
    have := link_execKind f kind
    simp only [Ctx.exec, allT_append]
    exact ⟨⟨⟨link_node k P _ _ _ _ h, link_ordT k P _ _⟩, this⟩, link_extensions k P _ _⟩
theorem link_execKind (f : String) : ∀ kind, AllT (LinkOK k P) (k.execKind (k.iri f) f kind)
  | .raise _ => link_str k P _ _ _ (by simp [containers])
  | .send s => by
    simp only [Ctx.execKind, allT_append]
    refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨link_optStr k P _ _ _ (by simp [containers]),
      link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
      link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
      link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
      link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
      link_optExpr k P _ _ _ (by simp [containers])⟩,
      link_tokens k P f "namelist" ":nl" (ch_nl f) _⟩, link_params k P _ _⟩, ?_⟩
    split
    · exact link_content k P _ _
    · simp
  | .cancel .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨link_optStr k P _ _ _ (by simp [containers]), link_optExpr k P _ _ _ (by simp [containers])⟩
  | .log .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨link_optStr k P _ _ _ (by simp [containers]), link_optExpr k P _ _ _ (by simp [containers])⟩
  | .assign .. => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨⟨link_expr k P _ _ _ (by simp [containers]), link_optExpr k P _ _ _ (by simp [containers])⟩,
      link_optStr k P _ _ _ (by simp [containers])⟩
  | .script _ => by simp only [Ctx.execKind]; exact link_scriptFields k P _ _
  | .if_ bs => by simp only [Ctx.execKind]; exact link_branches f 0 bs
  | .foreach _ _ _ body => by
    simp only [Ctx.execKind, allT_append]
    exact ⟨⟨⟨link_expr k P _ _ _ (by simp [containers]), link_str k P _ _ _ (by simp [containers])⟩,
      link_optStr k P _ _ _ (by simp [containers])⟩, link_block f ":a" "action" (ch_a f) 0 body⟩
  | .extension _ => link_extensionFields k P _ _
theorem link_block (f s p : String) (hs : ∀ j : Nat, Ch f (f ++ s ++ toString j)) :
    ∀ (i : Nat) (es : List Exec), AllT (LinkOK k P) (k.block (k.iri f) p (f ++ s) i es)
  | _, [] => by simp [Ctx.block]
  | i, e :: es => by
    have h1 := link_exec f p (f ++ s ++ toString i) i (hs i) e
    have h2 := link_block f s p hs (i + 1) es
    simp only [Ctx.block, allT_append]; exact ⟨h1, h2⟩
theorem link_branches (f : String) : ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
    AllT (LinkOK k P) (k.branches (k.iri f) f j bs)
  | _, [] => by simp [Ctx.branches]
  | j, (c, body) :: bs => by
    have h1 := link_block (f ++ ":b" ++ toString j) ":a" "action" (ch_a _) 0 body
    have h2 := link_branches f (j + 1) bs
    simp only [Ctx.branches, allT_append]
    exact ⟨⟨⟨⟨link_node k P _ _ _ _ (ch_lit f ":b" _ (by simp) (by simp)), link_ordT k P _ _⟩,
      link_optExpr k P _ _ _ (by simp [containers])⟩, h1⟩, h2⟩
end

theorem link_transition (tr : Transition) (o p f : String) (i : Nat) (h : Ch o f) (src sc : Term) :
    AllT (LinkOK k P) (k.transition tr (k.iri o) p f i src sc) := by
  simp only [Ctx.transition, allT_append, allT_cons, allT_nil, and_true]
  exact ⟨⟨⟨⟨⟨⟨⟨⟨⟨link_node k P _ _ _ _ h, link_ordT k P _ _⟩, by lnk⟩,
    link_tokens k P f "event" ":ev" (fun j => ch_lit f ":ev" j (by simp) (by simp)) _⟩,
    link_optExpr k P _ _ _ (by simp [containers])⟩,
    link_targets k P f "target" f ":tg" (fun j => ch_lit f ":tg" j (by simp) (by simp)) _ _⟩,
    link_str k P _ _ _ (by simp [containers])⟩, link_bool k P _ _ _ (by simp [containers])⟩,
    link_block k P f ":a" "action" (ch_a f) 0 _⟩, link_extensions k P _ _⟩

theorem link_dataItem (o : String) (i : Nat) (d : Data) :
    AllT (LinkOK k P) (k.dataItem (k.iri o) o i d) := by
  simp only [Ctx.dataItem, allT_append]
  exact ⟨⟨⟨⟨⟨⟨link_node k P _ _ _ _ (ch_lit o ":d" _ (by simp) (by simp)), link_ordT k P _ _⟩,
    link_str k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
    link_optExpr k P _ _ _ (by simp [containers])⟩, link_optStr k P _ _ _ (by simp [containers])⟩,
    link_extensions k P _ _⟩

theorem link_donedata (o f : String) (h : Ch o f) (dd : DoneData) :
    AllT (LinkOK k P) (k.donedata (k.iri o) f dd) := by
  simp only [Ctx.donedata, allT_append]
  refine ⟨⟨link_node k P _ _ _ _ h, ?_⟩, link_params k P _ _⟩
  split
  · exact link_content k P _ _
  · simp

theorem link_invoke (o f : String) (h : Ch o f) (i : Nat) (v : Invoke) :
    AllT (LinkOK k P) (k.invoke (k.iri o) f i v) := by
  simp only [Ctx.invoke, allT_append]
  refine ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨link_node k P _ _ _ _ h, link_ordT k P _ _⟩,
    link_optStr k P _ _ _ (by simp [containers])⟩, link_optExpr k P _ _ _ (by simp [containers])⟩,
    link_optStr k P _ _ _ (by simp [containers])⟩, link_optExpr k P _ _ _ (by simp [containers])⟩,
    link_optStr k P _ _ _ (by simp [containers])⟩, link_optExpr k P _ _ _ (by simp [containers])⟩,
    link_tokens k P f "namelist" ":nl" (ch_nl f) _⟩, link_bool k P _ _ _ (by simp [containers])⟩,
    link_params k P _ _⟩, ?_⟩, link_block k P f ":f" "finalize" (ch_f f) 0 _⟩, link_extensions k P _ _⟩
  split
  · exact link_content k P _ _
  · simp

end

theorem parented_eq (pp : Option State) (s : State) :
    State.parented pp s = (pp, s) :: State.parentedList (some s) s.children := by
  rcases s with ⟨id, idGen, el, initial, initEl, children, trs, onentry, onexit, data, invokes, dd, exts⟩
  rw [State.parented]; rfl

section
variable (k : Ctx)

theorem link_hooks (frag seg prop cls : String) (hs : seg.toList.head? = some ':')
    (hn : NC seg.toList.tail) (P : List (Option State × State)) (bs : List Block) :
    AllT (LinkOK k P) (bs.zipIdx.flatMap fun (b, i) =>
      k.node (k.iri frag) prop (frag ++ seg ++ toString i) [cls] ++
        ordT (k.iri (frag ++ seg ++ toString i)) i ++
        k.block (k.iri (frag ++ seg ++ toString i)) "action" (frag ++ seg ++ toString i ++ ":a") 0 b) := by
  simp only [allT_flatMap, allT_append]
  intro x _
  exact ⟨⟨link_node k P _ _ _ _ (ch_lit frag seg _ hs hn), link_ordT k P _ _⟩,
    link_block k P _ ":a" "action" (ch_a _) 0 _⟩

theorem link_st_ia (frag : String) (P : List (Option State × State)) (ids : List String) (sc : Term) :
    AllT (LinkOK k P) (k.targets (k.iri frag) "initial" (frag ++ ":ia") "" ids sc) :=
  link_targets k P _ _ _ _ (ch_ia _) _ _
theorem link_st_tr (frag : String) (P : List (Option State × State)) (t : Transition) (i : Nat) (src sc : Term) :
    AllT (LinkOK k P) (k.transition t (k.iri frag) "transition" (frag ++ ":t" ++ toString i) i src sc) :=
  link_transition k P _ _ _ _ _ (ch_lit _ ":t" _ (by simp) (by simp)) _ _
theorem link_st_init (frag : String) (P : List (Option State × State)) (ie : Initial) (s : Term) :
    AllT (LinkOK k P) (k.node (k.iri frag) "initialElement" (frag ++ ":init") ["Initial"] ++
          k.transition ie.transition (k.iri (frag ++ ":init")) "initialTransition"
            (frag ++ ":init" ++ ":t") 0 s s ++
          k.extensions (k.iri (frag ++ ":init")) (frag ++ ":init") ie.extensions) := by
  simp only [allT_append]
  exact ⟨⟨link_node k _ _ _ _ _ (ch_lit' _ ":init" (by simp) (by simp [NC])),
    link_transition k _ _ _ _ _ _ (ch_lit' _ ":t" (by simp) (by simp [NC])) _ _⟩,
    link_extensions k _ _ _⟩
theorem link_st_data (frag : String) (P : List (Option State × State)) (d : Data) (i : Nat) :
    AllT (LinkOK k P) (k.dataItem (k.iri frag) frag i d) := link_dataItem k P _ _ _
theorem link_st_inv (frag : String) (P : List (Option State × State)) (v : Invoke) (i : Nat) :
    AllT (LinkOK k P) (k.invoke (k.iri frag) (frag ++ ":inv" ++ toString i) i v) :=
  link_invoke k P _ _ (ch_lit _ ":inv" _ (by simp) (by simp)) _ _
theorem link_st_dd (frag : String) (P : List (Option State × State)) (d : DoneData) :
    AllT (LinkOK k P) (k.donedata (k.iri frag) (frag ++ ":dd") d) :=
  link_donedata k _ _ _ (ch_lit' _ ":dd" (by simp) (by simp [NC])) d
theorem link_st_ext (frag : String) (P : List (Option State × State)) (xs : List Extension) :
    AllT (LinkOK k P) (k.extensions (k.iri frag) frag xs) := link_extensions k P _ _

theorem link_vt' {P : List (Option State × State)} {s o : Term} {l : String} (h : l ∉ containers) :
    LinkOK k P ⟨s, vt l, o⟩ := link_vt k P _ _ _ h
theorem link_en (f : String) (P : List (Option State × State)) (l : String) (j : Nat) :
    LinkOK k P ⟨k.iri f, vt l, k.iri (f ++ ":en" ++ toString j)⟩ :=
  fun _ => ⟨_, rfl, .inl ⟨f, ch_lit f ":en" j (by simp) (by simp [NC]), rfl⟩⟩
theorem link_ex (f : String) (P : List (Option State × State)) (l : String) (j : Nat) :
    LinkOK k P ⟨k.iri f, vt l, k.iri (f ++ ":ex" ++ toString j)⟩ :=
  fun _ => ⟨_, rfl, .inl ⟨f, ch_lit f ":ex" j (by simp) (by simp [NC]), rfl⟩⟩
theorem link_ie (f : String) (P : List (Option State × State)) (l : String) :
    LinkOK k P ⟨k.iri f, vt l, k.iri (f ++ ":init")⟩ :=
  fun _ => ⟨_, rfl, .inl ⟨f, ch_lit' f ":init" (by simp) (by simp [NC]), rfl⟩⟩
theorem link_ie_tr (f : String) (P : List (Option State × State)) (t : Transition) (s s' : Term) :
    AllT (LinkOK k P) (k.transition t (k.iri (f ++ ":init")) "initialTransition" (f ++ ":init" ++ ":t") 0 s s') :=
  link_transition k _ _ _ _ _ _ (ch_lit' _ ":t" (by simp) (by simp [NC])) _ _
theorem link_act (g : String) (P : List (Option State × State)) (b : List Exec) :
    AllT (LinkOK k P) (k.block (k.iri g) "action" (g ++ ":a") 0 b) := link_block k P g ":a" "action" (ch_a g) 0 b

mutual
theorem link_state (pp : Option State) (i : Nat) (A : List Term) :
    ∀ s, AllT (LinkOK k (State.parented pp s)) (k.state i (parT k pp) A s)
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts => by
    have hc := link_states (some (.mk id idGen el initial initEl children trs onentry onexit data
      invokes dd exts)) 0 (A ++ [k.iri (stateFragment id)]) children
    have hP : ∀ q ∈ State.parentedList (some (.mk id idGen el initial initEl children trs onentry
        onexit data invokes dd exts)) children,
        q ∈ State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data
          invokes dd exts) := fun q hq => by
      rw [parented_eq]; exact List.mem_cons_of_mem _ hq
    have hc' : AllT (LinkOK k _) _ := fun t ht => linkOK_mono k hP (hc t ht)
    simp only [parT, State.id] at hc'
    have hmem : (pp, State.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts) ∈
        State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts) := by
      rw [parented_eq]; exact List.mem_cons_self
    generalize State.parented pp (.mk id idGen el initial initEl children trs onentry onexit data
      invokes dd exts) = P at hc' hmem ⊢
    have hlink : LinkOK k P ⟨parT k pp, vt "child", k.iri (stateFragment id)⟩ :=
      fun _ => ⟨_, rfl, .inr ⟨_, hmem, rfl, rfl⟩⟩
    have l1 : ∀ o, LinkOK k P ⟨k.iri (stateFragment id), vt "id", o⟩ := fun _ => link_vt' k (by simp [containers])
    have l2 : ∀ o, LinkOK k P ⟨k.iri (stateFragment id), vt "idGenerated", o⟩ := fun _ => link_vt' k (by simp [containers])
    have l3 : ∀ o, LinkOK k P ⟨k.iri (stateFragment id), vt "historyType", o⟩ := fun _ => link_vt' k (by simp [containers])
    have l4 : ∀ o, LinkOK k P ⟨k.iri (stateFragment id), vt "ancestor", o⟩ := fun _ => link_vt' k (by simp [containers])
    rcases initEl with _ | ie <;> rcases dd with _ | d <;> rcases el with _ | _ | _ | h <;>
      simp only [Ctx.state, Ctx.node, allT_append, allT_cons, allT_nil, allT_map, allT_flatMap,
        Prod.forall, hlink, hc', link_ordT, link_st_ia, link_st_tr, link_st_data, link_st_inv,
        link_st_ext, link_st_init, link_st_dd, str, bool, l1, l2, l3, l4, link_en, link_ex, link_act, link_ie, link_ie_tr,
        link_type, implies_true, and_true, true_and, and_self, List.map_cons, List.map_nil]

theorem link_states (pp : Option State) (i : Nat) (A : List Term) :
    ∀ cs, AllT (LinkOK k (State.parentedList pp cs)) (k.states i (parT k pp) A cs)
  | [] => by simp [Ctx.states]
  | c :: cs => by
    have h1 := link_state pp i A c
    have h2 := link_states pp (i + 1) A cs
    simp only [Ctx.states, allT_append]
    refine ⟨fun t ht => linkOK_mono k (fun q hq => ?_) (h1 t ht),
      fun t ht => linkOK_mono k (fun q hq => ?_) (h2 t ht)⟩
    · rw [State.parentedList]; exact List.mem_append_left _ hq
    · rw [State.parentedList]; exact List.mem_append_right _ hq
end

theorem link_hds (P : List (Option State × State)) (parent : Option State) (cs : List State) :
    AllT (LinkOK k P) (historyDefaultsList k parent cs) := by
  intro t ht
  have := hd_ps k parent cs t ht
  rcases t with ⟨s, p, o⟩
  simp only at this; subst this
  exact link_vt k P _ _ _ (by simp [containers])

end

/-- Every containment triple of the projection links a node to a child fragment or a parent to
a child state. -/
theorem link_project (c : Chart) (base : String) :
    AllT (LinkOK (ctxOf c base) c.parented) (project c base) := by
  rw [project_eq]
  have hs := link_states (ctxOf c base) none 0 [(ctxOf c base).iri "chart"] c.states
  simp only [parT] at hs
  simp only [allT_append]
  refine ⟨⟨⟨⟨⟨⟨?_, ?_⟩, ?_⟩, ?_⟩, hs⟩, ?_⟩, link_hds _ _ _ _⟩
  · simp only [chartSelf, allT_append, allT_cons, allT_nil, link_type, and_true, true_and]
    exact ⟨⟨⟨⟨link_optStr _ _ _ _ _ (by simp [containers]), link_str _ _ _ _ _ (by simp [containers])⟩,
      link_bool _ _ _ _ _ (by simp [containers])⟩, link_str _ _ _ _ _ (by simp [containers])⟩,
      link_bool _ _ _ _ _ (by simp [containers])⟩
  · simp only [chartRefs, ← targets_eq]
    exact link_targets _ _ "chart" _ _ _ ch_chart_ia _ _
  · simp only [allT_flatMap, List.mem_map]
    rintro _ ⟨x, _, rfl⟩
    exact link_dataItem _ _ "chart" _ _
  · cases c.script with
    | none => simp [scriptKids]
    | some sc =>
      simp only [scriptKids, List.flatMap_cons, List.flatMap_nil, List.append_nil, allT_append]
      exact ⟨link_node _ _ _ _ _ _ ch_scr, link_scriptFields _ _ _ _⟩
  · simp only [← extensions_eq]; exact link_extensions _ _ _ _

end Vertep.Rdf.Equiv
