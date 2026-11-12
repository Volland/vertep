import VertepLean.Rdf.RoundTrip.State
set_option linter.unusedSimpArgs false

/-!
# RDF round trip: the chart node and the theorem

The projection types exactly one node `vt:Chart` (`NT`, by structural recursion over the
emitters), the derived `vt:historyDefault` triples are never read, the chart node's emission
decomposes like a state's, and capture of every top-level state follows from the projection
being the whole graph. With the state decoder (`dec_state`) this gives the RDF Round Trip
Theorem `fromRDF_project`: `fromRDF (project c base) = .ok (c, base)` for well-formed charts.
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

/-! ## Only the chart node is a `vt:Chart` -/

/-- No triple of `E` types a node `vt:Chart`. -/
def NT (E : List Triple) : Prop := ∀ t ∈ E, t.p = rdfType → t.o ≠ vt "Chart"

@[simp] theorem NT_nil : NT [] := by simp [NT]
@[simp] theorem NT_cons {t : Triple} {E : List Triple} :
    NT (t :: E) ↔ (t.p = rdfType → t.o ≠ vt "Chart") ∧ NT E := by simp [NT]
@[simp] theorem NT_append {a b : List Triple} : NT (a ++ b) ↔ NT a ∧ NT b := by
  simp only [NT, List.mem_append]
  constructor
  · intro h; exact ⟨fun t ht => h t (.inl ht), fun t ht => h t (.inr ht)⟩
  · rintro ⟨h1, h2⟩ t (ht | ht)
    · exact h1 t ht
    · exact h2 t ht
@[simp] theorem NT_flatMap {α : Type} {l : List α} {f : α → List Triple} :
    NT (l.flatMap f) ↔ ∀ x ∈ l, NT (f x) := by
  simp only [NT, List.mem_flatMap]
  constructor
  · intro h x hx t ht; exact h t ⟨x, hx, ht⟩
  · rintro h t ⟨x, hx, ht⟩; exact h x hx t ht
@[simp] theorem NT_map {α : Type} {l : List α} {f : α → Triple} :
    NT (l.map f) ↔ ∀ x ∈ l, (f x).p = rdfType → (f x).o ≠ vt "Chart" := by
  simp [NT]

section
variable (k : Ctx)

@[simp] theorem NT_str (n : Term) (p v : String) : NT (str n p v) := by simp [str]
@[simp] theorem NT_optStr (n : Term) (p : String) (v : Option String) : NT (optStr n p v) := by
  cases v <;> simp [optStr, str]
@[simp] theorem NT_bool (n : Term) (p : String) (b : Bool) : NT (bool n p b) := by simp [bool]
@[simp] theorem NT_int (n : Term) (p : String) (v : Nat) : NT (int n p v) := by simp [int]
@[simp] theorem NT_ordT (n : Term) (v : Nat) : NT (ordT n v) := by simp [ordT, int]
@[simp] theorem NT_expr (n : Term) (p v : String) : NT (k.expr n p v) := by simp [Ctx.expr]
@[simp] theorem NT_optExpr (n : Term) (p : String) (v : Option String) : NT (k.optExpr n p v) := by
  cases v <;> simp [Ctx.optExpr, Ctx.expr]
@[simp] theorem NT_node (o : Term) (p f : String) (cs : List String) :
    NT (k.node o p f cs) ↔ "Chart" ∉ cs := by
  simp only [Ctx.node, NT_cons, NT_map]
  simp only [vt_ne_rdfType, false_implies, true_and, ne_eq, vt_inj, forall_const]
  constructor
  · intro h hm; exact h _ hm rfl
  · intro h x hx he; subst he; exact h hx
@[simp] theorem NT_extensionFields (n : Term) (x : Extension) : NT (Ctx.extensionFields n x) := by
  rcases x with ⟨ns, ln, v, pos⟩; cases v <;> simp [Ctx.extensionFields]
@[simp] theorem NT_extensions (o : Term) (f : String) (xs : List Extension) :
    NT (k.extensions o f xs) := by simp [Ctx.extensions]
@[simp] theorem NT_tokens (o : Term) (p pre : String) (vs : List String) :
    NT (k.tokens o p pre vs) := by simp [Ctx.tokens]
@[simp] theorem NT_params (o : Term) (f : String) (ps : List Param) : NT (k.params o f ps) := by
  simp [Ctx.params]
@[simp] theorem NT_content (o : Term) (f : String) (c : Content) : NT (k.content o f c) := by
  simp [Ctx.content]
@[simp] theorem NT_scriptFields (n : Term) (s : Script) : NT (k.scriptFields n s) := by
  simp [Ctx.scriptFields]
@[simp] theorem NT_targets (o : Term) (p l r : String) (ids : List String) (sc : Term) :
    NT (k.targets o p l r ids sc) := by
  simp only [Ctx.targets, NT_flatMap]
  intro x _
  simp only [NT_append, NT_node, NT_ordT, NT_cons, NT_nil]
  refine ⟨⟨⟨by simp, trivial⟩, by simp, by simp, trivial⟩, ?_⟩
  split
  · simp
  · simp

theorem execClass_ne (kind : ExecKind) : execClass kind ≠ "Chart" := by
  cases kind <;> simp [execClass]

theorem elementClass_ne (el : StateElement) : elementClass el ≠ "Chart" := by
  cases el <;> simp [elementClass]

mutual
theorem NT_exec (o : Term) (p f : String) (i : Nat) : ∀ e, NT (k.exec o p f i e)
  | .mk kind exts => by
    have := NT_execKind (k.iri f) f kind
    simp [Ctx.exec, this, Ne.symm (execClass_ne kind)]
theorem NT_execKind (n : Term) (f : String) : ∀ kind, NT (k.execKind n f kind)
  | .raise _ => by simp [Ctx.execKind]
  | .send s => by simp [Ctx.execKind]; split <;> simp
  | .cancel .. => by simp [Ctx.execKind]
  | .log .. => by simp [Ctx.execKind]
  | .assign .. => by simp [Ctx.execKind]
  | .script _ => by simp [Ctx.execKind]
  | .if_ bs => by simp only [Ctx.execKind]; exact NT_branches n f 0 bs
  | .foreach _ _ _ body => by
    have := NT_block n "action" (f ++ ":a") 0 body
    simp [Ctx.execKind, this]
  | .extension _ => by simp [Ctx.execKind]
theorem NT_block (o : Term) (p pre : String) : ∀ (i : Nat) (es : List Exec), NT (k.block o p pre i es)
  | _, [] => by simp [Ctx.block]
  | i, e :: es => by
    have h1 := NT_exec o p (pre ++ toString i) i e
    have h2 := NT_block o p pre (i + 1) es
    simp only [Ctx.block, NT_append]; exact ⟨h1, h2⟩
theorem NT_branches (n : Term) (f : String) : ∀ (j : Nat) (bs : List (Option Expr × List Exec)),
    NT (k.branches n f j bs)
  | _, [] => by simp [Ctx.branches]
  | j, (c, body) :: bs => by
    have h1 := NT_block (k.iri (f ++ ":b" ++ toString j)) "action" (f ++ ":b" ++ toString j ++ ":a") 0 body
    have h2 := NT_branches n f (j + 1) bs
    simp [Ctx.branches, h2]; simpa using h1
end

@[simp] theorem NT_transition (tr : Transition) (o : Term) (p f : String) (i : Nat) (src sc : Term) :
    NT (k.transition tr o p f i src sc) := by
  have := NT_block k (k.iri f) "action" (f ++ ":a") 0 tr.body
  simp [Ctx.transition, this]

@[simp] theorem NT_dataItem (o : Term) (f : String) (i : Nat) (d : Data) : NT (k.dataItem o f i d) := by
  simp [Ctx.dataItem]

@[simp] theorem NT_donedata (o : Term) (f : String) (dd : DoneData) : NT (k.donedata o f dd) := by
  simp [Ctx.donedata]; split <;> simp

@[simp] theorem NT_invoke (o : Term) (f : String) (i : Nat) (v : Invoke) : NT (k.invoke o f i v) := by
  have := NT_block k (k.iri f) "finalize" (f ++ ":f") 0 v.finalize
  simp [Ctx.invoke]; split <;> simpa using this

mutual
theorem NT_state (order : Nat) (parent : Term) (ancestors : List Term) :
    ∀ s, NT (k.state order parent ancestors s)
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts => by
    have hc := NT_states 0 (k.iri (stateFragment id)) (ancestors ++ [k.iri (stateFragment id)])
      children
    have hb : ∀ (seg : String) (b : List Exec) (i : Nat),
        NT (k.block (k.iri (stateFragment id ++ seg ++ Nat.repr i)) "action"
          (stateFragment id ++ seg ++ Nat.repr i ++ ":a") 0 b) := fun seg b i => NT_block k _ _ _ 0 b
    rcases initEl with _ | ie <;> rcases dd with _ | d <;> rcases el with _ | _ | _ | h <;>
      simp [Ctx.state, hc, hb, elementClass, Ne.symm (elementClass_ne _)]
theorem NT_states (i : Nat) (parent : Term) (ancestors : List Term) :
    ∀ cs, NT (k.states i parent ancestors cs)
  | [] => by simp [Ctx.states]
  | c :: cs => by
    have h1 := NT_state i parent ancestors c
    have h2 := NT_states (i + 1) parent ancestors cs
    simp only [Ctx.states, NT_append]; exact ⟨h1, h2⟩
end

mutual
/-- The derived `vt:historyDefault` triples. -/
theorem hd_p (parent : Option State) : ∀ s, ∀ t ∈ historyDefaults k parent s, t.p = vt "historyDefault"
  | .mk id ig el ini ie children trs en ex da inv dd xs => by
    have h := hd_ps (some (.mk id ig el ini ie children trs en ex da inv dd xs)) children
    intro t ht
    simp only [historyDefaults, List.mem_append] at ht
    rcases ht with ht | ht
    · cases el <;> simp at ht
      obtain ⟨_, _, _, rfl⟩ := ht
      rfl
    · exact h t ht
theorem hd_ps (parent : Option State) : ∀ cs, ∀ t ∈ historyDefaultsList k parent cs, t.p = vt "historyDefault"
  | [] => by simp [historyDefaultsList]
  | c :: cs => by
    have h1 := hd_p parent c
    have h2 := hd_ps parent cs
    intro t ht
    simp only [historyDefaultsList, List.mem_append] at ht
    rcases ht with ht | ht
    · exact h1 t ht
    · exact h2 t ht
end

end

/-! ## The chart node -/

/-- The context `project` builds for a chart. -/
def ctxOf (c : Chart) (base : String) : Ctx :=
  { base, exprDt := exprDatatype c.datamodel,
    chains := (indexStates [("chart", false)] c.states).1,
    histories := (indexStates [("chart", false)] c.states).2 }

section
variable (k : Ctx)

/-- The triples about the chart node itself. -/
def chartSelf (c : Chart) : List Triple :=
  [⟨k.iri "chart", rdfType, vt "Chart"⟩] ++ optStr (k.iri "chart") "name" c.name ++
    str (k.iri "chart") "datamodel" c.datamodel ++
    bool (k.iri "chart") "datamodelExplicit" c.datamodelExplicit ++
    str (k.iri "chart") "binding" (match c.binding with
      | .early => "early"
      | .late => "late") ++
    bool (k.iri "chart") "bindingExplicit" c.bindingExplicit

def scriptKids : Option Script → List (String × List Triple)
  | some s => [("chart:scr", k.node (k.iri "chart") "script" "chart:scr" ["Script"] ++
      k.scriptFields (k.iri "chart:scr") s)]
  | none => []

def chartRefs (c : Chart) : List (String × List Triple) :=
  refKids k (k.iri "chart") "initial" "chart:ia" "" c.initial (k.iri "chart")

def chartKids (c : Chart) : List (String × List Triple) :=
  chartRefs k c ++ c.data.zipIdx.map (dataK k "chart") ++ scriptKids k c.script ++
    extKids k "chart" c.extensions

/-- The chart node's own triples, its kids and its links to the top-level states. -/
def chartCore (c : Chart) : List Triple :=
  chartSelf k c ++ (chartRefs k c).flatMap Prod.snd ++
    (c.data.zipIdx.map (dataK k "chart")).flatMap Prod.snd ++
    (scriptKids k c.script).flatMap Prod.snd ++ childLinks k (k.iri "chart") c.states ++
    (extKids k "chart" c.extensions).flatMap Prod.snd

end

theorem project_eq (c : Chart) (base : String) :
    project c base =
      chartSelf (ctxOf c base) c ++ (chartRefs (ctxOf c base) c).flatMap Prod.snd ++
        (c.data.zipIdx.map (dataK (ctxOf c base) "chart")).flatMap Prod.snd ++
        (scriptKids (ctxOf c base) c.script).flatMap Prod.snd ++
        (ctxOf c base).states 0 ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"] c.states ++
        (extKids (ctxOf c base) "chart" c.extensions).flatMap Prod.snd ++
        historyDefaultsList (ctxOf c base) none c.states := by
  unfold project
  rcases hX : indexStates [("chart", false)] c.states with ⟨a, b⟩
  simp only [ctxOf, hX]
  rcases hb : c.binding with _ | _ <;> rcases c.script with _ | sc <;>
    simp [chartSelf, chartRefs, scriptKids, ← targets_eq, ← extensions_eq, dataK,
      List.flatMap_map, Function.comp_def, hb]

/-- A literal owner for the chart node, which has none. -/
def noOwner : Term := .lit (.typed "" xsdString)

theorem above_noOwner (k : Ctx) (f : String) : Above (k := k) noOwner f := by
  intro g _ h; simp [noOwner, Ctx.iri] at h

theorem ch_chart_ia : ∀ j : Nat, Ch "chart" ("chart:ia" ++ "" ++ toString j) := fun j =>
  ⟨"ia".toList ++ Nat.toDigits 10 j, by simp [String.toList_append], by simp⟩

section
variable (k : Ctx)

theorem kidsL_script (sc : Option Script) : KidsL k "chart" "script" (scriptKids k sc) := by
  intro d hd
  cases sc with
  | none => simp [scriptKids] at hd
  | some sc =>
    simp only [scriptKids, List.mem_singleton] at hd; subst hd
    refine ⟨fun t ht hr => ?_, by simp⟩
    simp only [List.mem_append, mem_node, mem_scriptFields, List.mem_singleton] at ht
    rcases ht with (h | ⟨_, _, h⟩) | ⟨_, _, h⟩ | ⟨_, _, h⟩ <;> subst h <;> simp

theorem ch_scr : Ch "chart" "chart:scr" := ⟨"scr".toList, by simp [String.toList_append], by simp [NC]⟩

theorem decomp_chart (c : Chart) :
    Decomp k noOwner "chart" "chart" (chartCore k c) (chartKids k c) := by
  constructor
  · intro t ht hr
    simp only [chartCore, List.mem_append] at ht
    rcases ht with (((((ht | ht) | ht) | ht) | ht) | ht)
    · refine .inr (.inl ?_)
      simp only [chartSelf, List.mem_append, List.mem_singleton, mem_optStr, mem_str, mem_bool] at ht
      rcases ht with ((((h | ⟨_, _, h⟩) | h) | h) | h) | h <;> subst h <;> rfl
    · obtain ⟨d, hd, ht⟩ := List.mem_flatMap.1 ht
      exact .inr (.inr ⟨d, by simp [chartKids, hd], ht⟩)
    · obtain ⟨d, hd, ht⟩ := List.mem_flatMap.1 ht
      exact .inr (.inr ⟨d, by simp [chartKids, hd], ht⟩)
    · obtain ⟨d, hd, ht⟩ := List.mem_flatMap.1 ht
      exact .inr (.inr ⟨d, by simp [chartKids, hd], ht⟩)
    · simp only [childLinks, List.mem_map] at ht
      obtain ⟨_, _, rfl⟩ := ht
      exact .inr (.inl rfl)
    · obtain ⟨d, hd, ht⟩ := List.mem_flatMap.1 ht
      exact .inr (.inr ⟨d, by simp [chartKids, hd], ht⟩)
  · apply ch_of_fst
    simp only [chartKids, chartRefs, List.map_append, fst_refKids, fst_dataK, fst_extKids,
      List.forall_mem_append, List.forall_mem_map]
    refine ⟨⟨⟨fun p _ => ch_chart_ia p.2, fun p _ => ch_lit "chart" ":d" p.2 (by simp) (by simp)⟩, ?_⟩,
      fun p _ => ch_lit "chart" ":x" p.2 (by simp) (by simp)⟩
    cases c.script <;> simp [scriptKids, ch_scr]
  · simp only [chartKids, chartRefs]
    exact kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsL_ref k _ _ _ _ _ _).loc
      (kidsL_dataK k _ _).loc) (kidsL_script k _).loc) (kidsL_ext k _ _).loc
  · simp only [chartKids, chartRefs, List.map_append, fst_refKids, fst_dataK, fst_extKids]
    cases hs : c.script <;> simp [scriptKids, List.nodup_append] <;> nd2_tac
    all_goals (simp only [tl_eq_comm]; intros; (try intro); simp_all)
  · intro d hd
    simp only [chartCore, chartSelf, List.singleton_append, List.tail_cons]
    simp only [chartKids, List.mem_append] at hd
    rcases hd with ((hd | hd) | hd) | hd <;> have := sub_flatMap hd <;> sub_tac

end

variable {k : Ctx} {ts : List Triple}

theorem cap_all (ts : List Triple) (R : String → Prop) : Cap k ts R ts :=
  ⟨List.Sublist.refl _, fun _ ht _ _ _ _ => ht⟩

theorem not_rel_hd {t : Triple} {parent : Option State} {cs : List State}
    (ht : t ∈ historyDefaultsList k parent cs) : ¬ Rel t := fun hr =>
  hr.1 (hd_ps k parent cs t ht)

theorem not_RSs_chart {cs : List State} : ¬ RSs cs "chart" := by
  rintro ⟨Y, _, hU⟩
  exact chart_not_sf Y.id "chart" (U_refl _) hU

theorem chart_coreCap (c : Chart) (base : String) :
    Cap (ctxOf c base) (project c base) (U "chart") (chartCore (ctxOf c base) c) := by
  refine Cap.split (cap_all _ (fun _ => True)) ?_ (fun _ _ => trivial) ?_
  · rw [project_eq, chartCore]
    exact (((List.Sublist.refl _).append (childLinks_sub _ _ _ 0 _)).append
      (List.Sublist.refl _)).trans (List.sublist_append_left _ _)
  · intro t ht hr
    rw [project_eq] at ht
    simp only [List.mem_append] at ht
    rcases ht with (((((ht | ht) | ht) | ht) | ht) | ht) | ht
    · exact .inl (by simp [chartCore, ht])
    · exact .inl (by simp [chartCore, ht])
    · exact .inl (by simp [chartCore, ht])
    · exact .inl (by simp [chartCore, ht])
    · rcases states_cov _ _ _ 0 _ t ht hr with h | ⟨g', hg', hR⟩
      · exact .inl (by simp [chartCore, h])
      · refine .inr fun g hg hU => ?_
        rw [hg', iri_inj] at hg; subst hg
        obtain ⟨Y, _, hY⟩ := hR
        exact chart_not_sf Y.id _ hU hY
    · exact .inl (by simp [chartCore, ht])
    · exact absurd hr (not_rel_hd ht)

theorem chart_statesCap (c : Chart) (base : String) :
    Cap (ctxOf c base) (project c base) (RSs c.states)
      ((ctxOf c base).states 0 ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"] c.states) := by
  refine Cap.split (cap_all _ (fun _ => True)) ?_ (fun _ _ => trivial) ?_
  · rw [project_eq]
    exact ((List.sublist_append_right _ _).trans (List.sublist_append_left _ _)).trans
      (List.sublist_append_left _ _)
  · intro t ht hr
    rw [project_eq] at ht
    simp only [List.mem_append] at ht
    have hloc : t ∈ chartCore (ctxOf c base) c → ∀ g, t.s = (ctxOf c base).iri g → ¬ RSs c.states g :=
      fun hc g hg hR => by
        rcases (decomp_chart _ c).locL t hc hr with rfl | ⟨g', hg', hU⟩
        · simp [noOwner, Ctx.iri] at hg
        · rw [hg', iri_inj] at hg; subst hg
          obtain ⟨Y, _, hY⟩ := hR
          exact chart_not_sf Y.id _ hU hY
    rcases ht with (((((ht | ht) | ht) | ht) | ht) | ht) | ht
    · exact .inr (hloc (by simp [chartCore, ht]))
    · exact .inr (hloc (by simp [chartCore, ht]))
    · exact .inr (hloc (by simp [chartCore, ht]))
    · exact .inr (hloc (by simp [chartCore, ht]))
    · exact .inl ht
    · exact .inr (hloc (by simp [chartCore, ht]))
    · exact absurd hr (not_rel_hd ht)

/-! ## Identifiers of every state -/

mutual
theorem ids_state : ∀ (s : State) (order : Nat) (P : Term) (A : List Term),
    SH k ts order P A s → ∀ Y ∈ s.flatten, IdL k ts Y.id
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts, order, P, A,
      h, Y, hY => by
    rw [flatten_eq] at hY
    rcases List.mem_cons.1 hY with rfl | hY
    · exact h.id_lk
    · exact ids_states children 0 _ _ (capList _ _ _ 0 h.kidsCap h.kidsNd h.kidsOwn) Y hY
theorem ids_states : ∀ (cs : List State) (i : Nat) (P : Term) (A : List Term),
    (∀ p ∈ cs.zipIdx i, SH k ts p.2 P A p.1) → ∀ Y ∈ State.flattenList cs, IdL k ts Y.id
  | [], _, _, _, _, Y, hY => by simp [State.flattenList] at hY
  | c :: cs, i, P, A, h, Y, hY => by
    rw [flattenList_cons, List.mem_append] at hY
    rcases hY with hY | hY
    · exact ids_state c i P A (h (c, i) (by simp)) Y hY
    · exact ids_states cs (i + 1) P A (fun p hp => h p (by simp [hp])) Y hY
end

/-! ## Well-formedness gives the targets' identifiers -/

mutual
theorem parented_snd (p : Option State) : ∀ s : State, (State.parented p s).map Prod.snd = s.flatten
  | s@(.mk _ _ _ _ _ cs ..) => by
    rw [State.parented, State.flatten, List.map_cons, parentedList_snd]
theorem parentedList_snd (p : Option State) :
    ∀ cs : List State, (State.parentedList p cs).map Prod.snd = State.flattenList cs
  | [] => by simp [State.parentedList, State.flattenList]
  | c :: cs => by
    rw [State.parentedList, State.flattenList, List.map_append, parented_snd, parentedList_snd]
end

theorem tgt_of_wf {c : Chart} (hwf : WellFormed c) :
    ∀ Y ∈ c.allStates, TgtL (· ∈ c.ids) Y := by
  intro Y hY
  have : Y ∈ c.parented.map Prod.snd := by
    rw [Chart.parented, parentedList_snd]; exact hY
  obtain ⟨⟨q, Y'⟩, hq, rfl⟩ := List.mem_map.1 this
  obtain ⟨_, _, hI, hIE, hTr, _⟩ := hwf.2.2.2.2.2.2.2.2.1 _ hq
  exact ⟨fun x hx => (hI.1 x hx).1, fun ie he x hx => ((hIE ie he).2.2.2.1.1 x hx).1,
    fun t ht x hx => (hTr t ht).1 x hx⟩

/-! ## Lookups about the chart node -/

section
variable (k : Ctx)

theorem self_chartRefs (ids : List String) (sc : Term) {P o : Term} (hr : Rel ⟨k.iri "chart", P, o⟩) :
    ⟨k.iri "chart", P, o⟩ ∈ (refKids k (k.iri "chart") "initial" "chart:ia" "" ids sc).flatMap Prod.snd ↔
      P = vt "initial" ∧
        o ∈ (refKids k (k.iri "chart") "initial" "chart:ia" "" ids sc).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_ref k "chart" "initial" "chart:ia" "" ids sc)
    (ch_of_fst (by simp only [fst_refKids, List.forall_mem_map]; intro p _; exact ch_chart_ia p.2)) hr

theorem self_scriptS (sc : Option Script) {P o : Term} (hr : Rel ⟨k.iri "chart", P, o⟩) :
    ⟨k.iri "chart", P, o⟩ ∈ (scriptKids k sc).flatMap Prod.snd ↔
      P = vt "script" ∧ o ∈ (scriptKids k sc).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_script k sc) (ch_of_fst (by cases sc <;> simp [scriptKids, ch_scr])) hr

end

syntax "lkc" term : tactic
macro_rules
  | `(tactic| lkc $h) => `(tactic|
    (intro o
     rw [($h).lookup (U_refl _) (by simp)]
     simp only [chartCore, chartRefs, List.mem_append]
     simp only [self_chartRefs, self_dataS, self_scriptS, self_extS, rel_mk, rel_type, ne_eq,
       String.reduceEq, not_false_eq_true, and_self, and_true, true_and]
     simp [chartSelf, childLinks, strLit, intLit, boolLit]))

theorem project_head (c : Chart) (base : String) :
    project c base = ⟨(ctxOf c base).iri "chart", rdfType, vt "Chart"⟩ ::
      (project c base).tail := by
  rw [project_eq]; rfl

theorem NT_tail (c : Chart) (base : String) : NT (project c base).tail := by
  rw [project_eq]
  have h1 := NT_states (ctxOf c base) 0 ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"] c.states
  have h2 : NT (historyDefaultsList (ctxOf c base) none c.states) := fun t ht hp => by
    rw [hd_ps _ _ _ t ht] at hp; exact absurd hp (vt_ne_rdfType _)
  have h3 := NT_targets (ctxOf c base) ((ctxOf c base).iri "chart") "initial" "chart:ia" ""
    c.initial ((ctxOf c base).iri "chart")
  rw [targets_eq] at h3
  have h4 := NT_extensions (ctxOf c base) ((ctxOf c base).iri "chart") "chart" c.extensions
  rw [extensions_eq] at h4
  rcases c.script with _ | sc <;>
    simp [chartSelf, chartRefs, scriptKids, dataK, h1, h2, h3, h4, List.tail_append] <;>
    (intros; subst_vars; exact NT_dataItem _ _ _ _ _)

theorem charts_eq (c : Chart) (base : String) :
    (((project c base).filter (fun t => t.p == rdfType && t.o == vt "Chart")).map (·.s)).eraseDups =
      [(ctxOf c base).iri "chart"] := by
  have hn := NT_tail c base
  have hf : ((project c base).tail.filter (fun t => t.p == rdfType && t.o == vt "Chart")) = [] := by
    rw [List.filter_eq_nil_iff]
    intro t ht h
    simp only [Bool.and_eq_true, beq_iff_eq] at h
    exact hn t ht h.1 h.2
  rw [project_head, List.filter_cons_of_pos (by simp), hf]
  rfl

theorem baseOf (base : String) : baseOfChartNode (base ++ "#" ++ "chart") = some base := by
  unfold baseOfChartNode
  have h1 : (base ++ "#" ++ "chart").endsWith "#chart" = true := by
    rw [String.endsWith_eq_endsWith_toSlice, String.Slice.endsWith_string_iff]
    simp only [String.copy_toSlice, String.toList_append]
    exact ⟨base.toList, by simp⟩
  simp only [h1, ↓reduceIte]
  congr 1
  apply String.toList_inj.1
  simp [String.toList_append, String.length]

theorem iri_ctxOf (c : Chart) (base f : String) :
    (ctxOf c base).iri f = Term.iri ⟨base ++ "#" ++ f⟩ := rfl

theorem exprDt_ctxOf (c : Chart) (base : String) :
    (ctxOf c base).exprDt = exprDatatype c.datamodel := rfl

theorem top_nodup {k : Ctx} {cs : List State} (hnd : ((State.flattenList cs).map State.id).Nodup) :
    (cs.map (fun c => k.iri (stateFragment c.id))).Nodup := by
  have h1 : (cs.map State.id).Nodup := hnd.sublist ((children_sub _).map _)
  have : cs.map (fun c => k.iri (stateFragment c.id)) =
      (cs.map State.id).map (fun x => k.iri (stateFragment x)) := by
    simp [List.map_map, Function.comp_def]
  rw [this, List.Nodup, List.pairwise_map]
  exact h1.imp (fun hne he => hne (stateFragment_inj ((iri_inj k).1 he)))

theorem Chart.eta (c : Chart) :
    ({ name := c.name, datamodel := c.datamodel, datamodelExplicit := c.datamodelExplicit,
       binding := c.binding, bindingExplicit := c.bindingExplicit, initial := c.initial,
       data := c.data, script := c.script, states := c.states, extensions := c.extensions } : Chart) = c := by
  rcases c; rfl

/-- **Decoding inverts projecting a well-formed chart**. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "decoding half of fromRDF_project"}
theorem decode_project (c : Chart) (hwf : WellFormed c) (base : String) :
    decode (project c base) = .ok (c, base) := by
  have hcC := chart_coreCap c base
  have cx : Ctxt (ctxOf c base) (project c base) noOwner "chart" "chart" _ _ :=
    ⟨hcC, decomp_chart _ c, above_noOwner _ "chart"⟩
  have hSH := capList (k := ctxOf c base) ((ctxOf c base).iri "chart") [(ctxOf c base).iri "chart"]
    c.states 0 (chart_statesCap c base) hwf.2.2.1 (fun g hR he => by
      rw [iri_inj] at he; subst he; exact not_RSs_chart hR)
  have hids : ∀ x ∈ c.ids, IdL (ctxOf c base) (project c base) x := by
    intro x hx
    obtain ⟨Y, hY, rfl⟩ := List.mem_map.1 hx
    exact ids_states c.states 0 _ _ hSH Y hY
  have hT : ∀ Y ∈ State.flattenList c.states, TgtL (IdL (ctxOf c base) (project c base)) Y := by
    intro Y hY
    obtain ⟨h1, h2, h3⟩ := tgt_of_wf hwf Y hY
    exact ⟨fun x hx => hids x (h1 x hx), fun ie he x hx => hids x (h2 ie he x hx),
      fun t ht x hx => hids x (h3 t ht x hx)⟩
  have ho : ∀ f, (ctxOf c base).iri "chart" ≠ (ctxOf c base).iri ("chart" ++ f) → True := fun _ _ => trivial
  clear ho
  have h_dm : Decode.string (Index.of (project c base)) ((ctxOf c base).iri "chart") "datamodel" =
      .ok c.datamodel := string_eq (by lkc hcC)
  have h_name : Decode.optString (Index.of (project c base)) ((ctxOf c base).iri "chart") "name" =
      .ok c.name := optString_eq (by lkc hcC)
  have h_dmx : Decode.boolean (Index.of (project c base)) ((ctxOf c base).iri "chart")
      "datamodelExplicit" = .ok c.datamodelExplicit := boolean_eq (by lkc hcC)
  have h_bx : Decode.boolean (Index.of (project c base)) ((ctxOf c base).iri "chart")
      "bindingExplicit" = .ok c.bindingExplicit := boolean_eq (by lkc hcC)
  have h_bind : Decode.string (Index.of (project c base)) ((ctxOf c base).iri "chart") "binding" =
      .ok (match c.binding with
        | .early => "early"
        | .late => "late") := string_eq (by lkc hcC)
  have h_ini : Decode.targetIds (Index.of (project c base)) ((ctxOf c base).iri "chart") "initial" =
      .ok c.initial :=
    dec_targets cx "initial" "chart:ia" "" ch_chart_ia c.initial ((ctxOf c base).iri "chart")
      (fun p hp => by simp [chartKids, chartRefs, refKids, List.mem_map_of_mem hp])
      (by lkc hcC) (fun x hx => hids x (hwf.2.2.2.1 x hx))
  have h_ext : Decode.extensions (Index.of (project c base)) ((ctxOf c base).iri "chart") =
      .ok c.extensions :=
    dec_extensions cx c.extensions (fun d hd => by simp [chartKids, hd]) (by lkc hcC)
  have hDA : ∀ p ∈ c.data.zipIdx, _ := fun p hp =>
    dec_dataItem (cx.kid (d := dataK (ctxOf c base) "chart" p)
      (by simp [chartKids, List.mem_map_of_mem hp]))
      (above_of_ch (ch_lit "chart" ":d" p.2 (by simp) (by simp)))
  have h_dao := ordered_kids (k := ctxOf c base) (ts := project c base)
    (s := (ctxOf c base).iri "chart") (prop := "data") c.data (dataK (ctxOf c base) "chart")
    (by lkc hcC) (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _) (fun p hp => (hDA p hp).2)
  have h_dam := mapM_kids (k := ctxOf c base)
    (Decode.dataItem (Index.of (project c base)) (ctxOf c base).exprDt)
    (dataK (ctxOf c base) "chart") id c.data 0 (fun p hp => (hDA p hp).1)
  have hSC0 : c.script = none →
      Decode.optNode (Index.of (project c base)) ((ctxOf c base).iri "chart") "script" = .ok none :=
    fun he => optNode_none (by lkc hcC; simp [he, scriptKids])
  have hSC1 : ∀ sc, c.script = some sc →
      Decode.optNode (Index.of (project c base)) ((ctxOf c base).iri "chart") "script" =
        .ok (some ((ctxOf c base).iri "chart:scr")) ∧
      Decode.script (Index.of (project c base)) (ctxOf c base).exprDt
        ((ctxOf c base).iri "chart:scr") = .ok sc := by
    intro sc he
    have hk := cx.kid (d := ("chart:scr", (ctxOf c base).node ((ctxOf c base).iri "chart") "script"
      "chart:scr" ["Script"] ++ (ctxOf c base).scriptFields ((ctxOf c base).iri "chart:scr") sc))
      (by simp [chartKids, scriptKids, he])
    have hne : (ctxOf c base).iri "chart" ≠ (ctxOf c base).iri "chart:scr" := by simp
    refine ⟨optNode_some ⟨_, rfl⟩ (by lkc hcC; simp [he, scriptKids, eq_comm]), ?_⟩
    have h1 : Decode.optString (Index.of (project c base)) ((ctxOf c base).iri "chart:scr") "src" =
        .ok sc.src := optString_eq (by
          intro o; rw [hk.lookup (U_refl _) (by simp)]; simp [Ctx.node, Ctx.scriptFields, hne, strLit])
    have h2 : Decode.optExpr (Index.of (project c base)) (ctxOf c base).exprDt
        ((ctxOf c base).iri "chart:scr") "scriptBody" = .ok sc.body := optExpr_eq (by
          intro o; rw [hk.lookup (U_refl _) (by simp)]; simp [Ctx.node, Ctx.scriptFields, hne, strLit])
    simp only [Decode.script, h1, h2, bind, Except.bind, pure, Except.pure]
  have h_cho : Decode.ordered (Index.of (project c base)) ((ctxOf c base).iri "chart") "child" =
      .ok (c.states.map (fun s => (ctxOf c base).iri (stateFragment s.id))) := by
    refine ordered_eq (by lkc hcC) (top_nodup hwf.2.2.1) (fun m hm => ?_) (fun j hj => ?_)
    · obtain ⟨s, _, rfl⟩ := List.mem_map.1 hm; exact ⟨_, rfl⟩
    · simp only [List.length_map] at hj
      have hp : (c.states[j], j) ∈ c.states.zipIdx := by
        rw [List.mem_zipIdx_iff_getElem?]; simp [hj]
      simpa using (hSH _ hp).order_lk
  have h_st := dec_states (k := ctxOf c base) ((project c base).length + 1) (by omega) c.states 0
    ((project c base).length + 1) _ _ hSH hT
    (by have := (chart_statesCap c base).sub.length_le; omega)
  simp only [exprDt_ctxOf] at h_dam hSC1 h_st
  have hch := charts_eq c base
  simp only [List.map_map, List.mapM_map, List.map_id] at h_dao h_dam
  simp only [iri_ctxOf] at hch h_dm h_name h_dmx h_bx h_bind h_ini h_ext h_dao h_dam hSC0 hSC1 h_cho h_st
  unfold decode
  rcases hsc : c.script with _ | sc
  · have a1 := hSC0 hsc
    clear hSC0 hSC1
    rcases hb : c.binding with _ | _ <;> rw [hb] at h_bind <;>
    simp [bind, Except.bind, pure, Except.pure, hch, baseOf, h_dm, h_name, h_dmx, h_bx, h_bind,
      h_ini, h_ext, h_dao, h_dam, a1, h_cho, h_st] <;>
    (rw [← hsc, ← hb])
  · obtain ⟨a1, a2⟩ := hSC1 sc hsc
    clear hSC0 hSC1
    rcases hb : c.binding with _ | _ <;> rw [hb] at h_bind <;>
    simp [bind, Except.bind, pure, Except.pure, hch, baseOf, h_dm, h_name, h_dmx, h_bx, h_bind,
      h_ini, h_ext, h_dao, h_dam, a1, a2, h_cho, h_st] <;>
    (rw [← hsc, ← hb])


end RT

/-- **RDF Round Trip Theorem** (formal-model): reconstruction after projection is the identity
on well-formed charts, for every base. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]]
theorem fromRDF_project (c : Chart) (hwf : WellFormed c) (base : String) :
    fromRDF (project c base) = .ok (c, base) := by
  have hcw : closedWorld (Graph.ofList (project c base)) c base = true := by
    unfold closedWorld
    rw [List.all_eq_true]
    intro t ht
    simpa using ht
  simp only [fromRDF, RT.decode_project c hwf base, bind, Except.bind, hcw, ↓reduceIte, pure,
    Except.pure]

end Vertep.Rdf
