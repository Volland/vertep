import VertepLean.Rdf.RoundTrip.Nodes
set_option linter.unusedSimpArgs false

/-!
# RDF round trip: states

A state's emission splits into its own triples (`stateSelf`), the emissions of its local kids
(`stateKids`: target references, `<initial>`, transitions, handlers, data, invokes, done data
and extensions, all at fragments below the state's) and its child states' emissions, which live
in other regions (the fragments of their own subtrees). The child links have the state as their
subject but sit at the head of each child's emission, so capture at the state's fragment is
stated for `stateCore`: its own triples, its kids and the child links (a sublist of the
emission). With that, the decoders of `Decode`, `Transition` and `Nodes` apply unchanged, and
`dec_state` decodes a state and its subtree by structural recursion.
-/
namespace Vertep.Rdf
open LeanRDF
namespace RT

/-! ## State fragments and regions -/

theorem nc_stateFragment (x : String) : NC (stateFragment x).toList := by
  simp only [stateFragment, String.toList_append, nc_append]
  exact ⟨by simp [NC], colon_not_mem_percentEncode x⟩

theorem stateFragment_inj {x y : String} (h : stateFragment x = stateFragment y) : x = y := by
  apply percentEncode_inj
  have := congrArg String.toList h
  simp only [stateFragment, String.toList_append, List.append_cancel_left_eq] at this
  exact String.toList_inj.1 this

theorem U_sf {x y g : String} (hx : U (stateFragment x) g) (hy : U (stateFragment y) g) : x = y :=
  stateFragment_inj (U_root (nc_stateFragment x) (nc_stateFragment y) hx hy)

/-- Regions given by a list of identifiers. -/
def RI (xs : List String) (g : String) : Prop := ∃ x ∈ xs, U (stateFragment x) g

theorem RI_disj {xs ys : List String} {g : String} (hx : RI xs g) (hy : RI ys g) :
    ∃ x ∈ xs, x ∈ ys := by
  obtain ⟨x, hx, ux⟩ := hx
  obtain ⟨y, hy, uy⟩ := hy
  exact ⟨x, hx, (U_sf ux uy) ▸ hy⟩

theorem RS_iff (s : State) (g : String) : RS s g ↔ RI (s.flatten.map State.id) g := by
  simp only [RS, RI, List.mem_map]
  constructor
  · rintro ⟨Y, hY, hU⟩; exact ⟨_, ⟨Y, hY, rfl⟩, hU⟩
  · rintro ⟨_, ⟨Y, hY, rfl⟩, hU⟩; exact ⟨Y, hY, hU⟩

theorem RSs_iff (cs : List State) (g : String) :
    RSs cs g ↔ RI ((State.flattenList cs).map State.id) g := by
  simp only [RSs, RI, List.mem_map]
  constructor
  · rintro ⟨Y, hY, hU⟩; exact ⟨_, ⟨Y, hY, rfl⟩, hU⟩
  · rintro ⟨_, ⟨Y, hY, rfl⟩, hU⟩; exact ⟨Y, hY, hU⟩

theorem flatten_mk (id : String) (idGen : Bool) (el : StateElement) (initial : List String)
    (initEl : Option Initial) (children : List State) (ts : List Transition)
    (onentry onexit : List Block) (data : List Data) (invokes : List Invoke)
    (dd : Option DoneData) (exts : List Extension) :
    (State.mk id idGen el initial initEl children ts onentry onexit data invokes dd exts).flatten =
      State.mk id idGen el initial initEl children ts onentry onexit data invokes dd exts ::
        State.flattenList children := by
  rw [State.flatten]

theorem flattenList_cons (c : State) (cs : List State) :
    State.flattenList (c :: cs) = c.flatten ++ State.flattenList cs := by
  rw [State.flattenList]

theorem chart_not_sf (x g : String) (h1 : U "chart" g) (h2 : U (stateFragment x) g) : False := by
  have := U_root (by simp [NC]) (nc_stateFragment x) h1 h2
  have h := congrArg (fun s => s.toList.head?) this
  simp [stateFragment, String.toList_append] at h

section
variable (k : Ctx)

/-! ## The parts of a state's emission -/

def histT (s : Term) : StateElement → List Triple
  | .history h => str s "historyType" (match h with
      | .shallow => "shallow"
      | .deep => "deep")
  | _ => []

@[simp] theorem mem_histT {t : Triple} {s : Term} {el : StateElement} :
    t ∈ histT s el ↔ ∃ h, el = .history h ∧
      t = ⟨s, vt "historyType", strLit (match h with
        | .shallow => "shallow"
        | .deep => "deep")⟩ := by
  cases el <;> simp [histT, str]

/-- The triples a state emits about itself, link from its parent first. -/
def stateSelf (order : Nat) (parent : Term) (ancestors : List Term) (id : String) (idGen : Bool)
    (el : StateElement) : List Triple :=
  k.node parent "child" (stateFragment id) ["StateNode", elementClass el] ++
    histT (k.iri (stateFragment id)) el ++
    str (k.iri (stateFragment id)) "id" id ++ bool (k.iri (stateFragment id)) "idGenerated" idGen ++
    ordT (k.iri (stateFragment id)) order ++
    ancestors.map (fun a => ⟨k.iri (stateFragment id), vt "ancestor", a⟩)

def scopeOf (parent : Term) (ancestors : List Term) (s : Term) : StateElement → Term
  | .history _ => parent
  | _ => ancestors.headD s

def initKidsOpt (frag : String) : Option Initial → List (String × List Triple)
  | some ie => [(frag ++ ":init", initEmission k (k.iri frag) (frag ++ ":init") ie)]
  | none => []

def transK (frag : String) (scope : Term) (p : Transition × Nat) : String × List Triple :=
  (frag ++ ":t" ++ toString p.2,
    k.transition p.1 (k.iri frag) "transition" (frag ++ ":t" ++ toString p.2) p.2 (k.iri frag) scope)

def hookK (frag seg prop cls : String) (p : List Exec × Nat) : String × List Triple :=
  (frag ++ seg ++ toString p.2, hookEmission k (k.iri frag) prop cls (frag ++ seg ++ toString p.2) p.2 p.1)

def dataK (frag : String) (p : Data × Nat) : String × List Triple :=
  (frag ++ ":d" ++ toString p.2, k.dataItem (k.iri frag) frag p.2 p.1)

def invK (frag : String) (p : Invoke × Nat) : String × List Triple :=
  (frag ++ ":inv" ++ toString p.2, k.invoke (k.iri frag) (frag ++ ":inv" ++ toString p.2) p.2 p.1)

def ddKidsOpt (frag : String) : Option DoneData → List (String × List Triple)
  | some d => [(frag ++ ":dd", k.donedata (k.iri frag) (frag ++ ":dd") d)]
  | none => []

/-- The kids of a state: nodes at fragments directly below the state's. -/
def stateKids (frag : String) (scope : Term) (initial : List String) (initEl : Option Initial)
    (trs : List Transition) (onentry onexit : List Block) (data : List Data)
    (invokes : List Invoke) (dd : Option DoneData) (exts : List Extension) :
    List (String × List Triple) :=
  refKids k (k.iri frag) "initial" (frag ++ ":ia") "" initial (k.iri frag) ++
  initKidsOpt k frag initEl ++
  trs.zipIdx.map (transK k frag scope) ++
  onentry.zipIdx.map (hookK k frag ":en" "onentry" "OnEntry") ++
  onexit.zipIdx.map (hookK k frag ":ex" "onexit" "OnExit") ++
  data.zipIdx.map (dataK k frag) ++
  invokes.zipIdx.map (invK k frag) ++
  ddKidsOpt k frag dd ++
  extKids k frag exts

/-- The links from a state node to its child states. -/
def childLinks (s : Term) (cs : List State) : List Triple :=
  cs.map (fun c => ⟨s, vt "child", k.iri (stateFragment c.id)⟩)

/-- A state's emission, split into its own triples, its kids and its child states. -/
theorem state_eq (order : Nat) (parent : Term) (ancestors : List Term) (id : String)
    (idGen : Bool) (el : StateElement) (initial : List String) (initEl : Option Initial)
    (children : List State) (trs : List Transition) (onentry onexit : List Block)
    (data : List Data) (invokes : List Invoke) (dd : Option DoneData) (exts : List Extension) :
    k.state order parent ancestors
      (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts) =
    stateSelf k order parent ancestors id idGen el ++
      (stateKids k (stateFragment id)
        (scopeOf parent ancestors (k.iri (stateFragment id)) el) initial initEl trs onentry
        onexit data invokes dd exts).flatMap Prod.snd ++
      k.states 0 (k.iri (stateFragment id)) (ancestors ++ [k.iri (stateFragment id)]) children := by
  rcases el with _ | _ | _ | (_ | _) <;> rcases initEl with _ | ie <;> rcases dd with _ | d <;>
    simp [Ctx.state, stateSelf, stateKids, histT, initKidsOpt, ddKidsOpt, initEmission, transK,
      hookK, dataK, invK, scopeOf, hookEmission, targets_eq, extensions_eq, List.flatMap_append,
      List.flatMap_map, Function.comp_def]

theorem state_head (order : Nat) (parent : Term) (ancestors : List Term) (c : State) :
    ∃ rest, k.state order parent ancestors c =
      ⟨parent, vt "child", k.iri (stateFragment c.id)⟩ :: rest := by
  rcases c with ⟨id, idGen, el, initial, initEl, children, trs, onentry, onexit, data, invokes,
    dd, exts⟩
  rw [state_eq]
  exact ⟨_, rfl⟩

theorem childLinks_sub (s : Term) (ancestors : List Term) :
    ∀ (i : Nat) (cs : List State), (childLinks k s cs).Sublist (k.states i s ancestors cs)
  | _, [] => by simp [childLinks, Ctx.states]
  | i, c :: cs => by
    obtain ⟨rest, h⟩ := state_head k i s ancestors c
    rw [Ctx.states, h]
    simp only [childLinks, List.map_cons, List.cons_append]
    exact List.Sublist.cons_cons _ ((childLinks_sub s ancestors (i + 1) cs).trans
      (List.sublist_append_right _ _))

theorem states_cov (s : Term) (ancestors : List Term) (i : Nat) (cs : List State) :
    ∀ t ∈ k.states i s ancestors cs, Rel t →
      t ∈ childLinks k s cs ∨ ∃ g, t.s = k.iri g ∧ RSs cs g := by
  intro t ht hr
  rcases locS_states k i s ancestors cs t ht hr with ⟨c, hc, rfl⟩ | h
  · exact .inl (List.mem_map.2 ⟨c, hc, rfl⟩)
  · exact .inr h

end

theorem tl_eq_comm {a : String} {l : List Char} : (l = a.toList) ↔ (a.toList = l) := eq_comm

syntax "nd2_tac" : tactic
macro_rules
  | `(tactic| nd2_tac) => `(tactic| (and_intros <;> first
      | exact nodup_zipIdx_map' _ (fun p q h => by
          simpa [← String.toList_inj, String.toList_append, digits_inj] using h) _ _
      | exact nodup_option_toList _
      | ((try simp only [tl_eq_comm] at *); intros; (try intro); subst_vars;
         simp_all [← String.toList_inj, String.toList_append, digits_inj])))

section
variable (k : Ctx)

/-! ## Kid segments -/

@[simp] theorem fst_transK (frag : String) (scope : Term) (trs : List Transition) :
    (trs.zipIdx.map (transK k frag scope)).map Prod.fst =
      trs.zipIdx.map (fun p => frag ++ ":t" ++ toString p.2) := by
  simp [transK, List.map_map, Function.comp_def]
@[simp] theorem fst_hookK (frag seg prop cls : String) (bs : List Block) :
    (bs.zipIdx.map (hookK k frag seg prop cls)).map Prod.fst =
      bs.zipIdx.map (fun p => frag ++ seg ++ toString p.2) := by
  simp [hookK, List.map_map, Function.comp_def]
@[simp] theorem fst_dataK (frag : String) (ds : List Data) :
    (ds.zipIdx.map (dataK k frag)).map Prod.fst = ds.zipIdx.map (fun p => frag ++ ":d" ++ toString p.2) := by
  simp [dataK, List.map_map, Function.comp_def]
@[simp] theorem fst_invK (frag : String) (vs : List Invoke) :
    (vs.zipIdx.map (invK k frag)).map Prod.fst =
      vs.zipIdx.map (fun p => frag ++ ":inv" ++ toString p.2) := by
  simp [invK, List.map_map, Function.comp_def]
@[simp] theorem fst_initKidsOpt (frag : String) (ie : Option Initial) :
    (initKidsOpt k frag ie).map Prod.fst = (ie.map (fun _ => frag ++ ":init")).toList := by
  cases ie <;> simp [initKidsOpt]
@[simp] theorem fst_ddKidsOpt (frag : String) (dd : Option DoneData) :
    (ddKidsOpt k frag dd).map Prod.fst = (dd.map (fun _ => frag ++ ":dd")).toList := by
  cases dd <;> simp [ddKidsOpt]

theorem kidsL_transK (frag : String) (scope : Term) (trs : List Transition) :
    KidsL k frag "transition" (trs.zipIdx.map (transK k frag scope)) := by
  intro d hd
  simp only [List.mem_map] at hd
  obtain ⟨p, _, rfl⟩ := hd
  exact ⟨locL_transition k _ _ _ _ _ _ _, by simp [transK, Ctx.transition]⟩

theorem kidsL_hookK (frag seg prop cls : String) (bs : List Block) :
    KidsL k frag prop (bs.zipIdx.map (hookK k frag seg prop cls)) := by
  intro d hd
  simp only [List.mem_map] at hd
  obtain ⟨p, _, rfl⟩ := hd
  exact ⟨(decomp_hook k _ _ _ _ _ _).locL, by simp [hookK, hookEmission]⟩

theorem kidsL_dataK (frag : String) (ds : List Data) :
    KidsL k frag "data" (ds.zipIdx.map (dataK k frag)) := by
  intro d hd
  simp only [List.mem_map] at hd
  obtain ⟨p, _, rfl⟩ := hd
  exact ⟨(decomp_dataItem k _ _ _ _).locL, by simp [dataK, Ctx.dataItem]⟩

theorem kidsL_invK (frag : String) (vs : List Invoke) :
    KidsL k frag "invoke" (vs.zipIdx.map (invK k frag)) := by
  intro d hd
  simp only [List.mem_map] at hd
  obtain ⟨p, _, rfl⟩ := hd
  exact ⟨(decomp_invoke k _ _ _ _).locL, by simp [invK, Ctx.invoke]⟩

theorem kidsL_initKidsOpt (frag : String) (ie : Option Initial) :
    KidsL k frag "initialElement" (initKidsOpt k frag ie) := by
  intro d hd
  cases ie with
  | none => simp [initKidsOpt] at hd
  | some ie =>
    simp only [initKidsOpt, List.mem_singleton] at hd; subst hd
    exact ⟨(decomp_init k _ _ _).locL, by simp [initEmission]⟩

theorem kidsL_ddKidsOpt (frag : String) (dd : Option DoneData) :
    KidsL k frag "donedata" (ddKidsOpt k frag dd) := by
  intro d hd
  cases dd with
  | none => simp [ddKidsOpt] at hd
  | some dd =>
    simp only [ddKidsOpt, List.mem_singleton] at hd; subst hd
    exact ⟨(decomp_donedata k _ _ _).locL, by simp [Ctx.donedata]⟩

theorem ch_ia (frag : String) : ∀ j : Nat, Ch frag (frag ++ ":ia" ++ "" ++ toString j) := fun j =>
  by simpa using ch_lit frag ":ia" j (by simp) (by simp)

theorem self_refS (frag : String) (ids : List String) (scope : Term) {P o : Term}
    (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (refKids k (k.iri frag) "initial" (frag ++ ":ia") "" ids scope).flatMap Prod.snd ↔
      P = vt "initial" ∧
        o ∈ (refKids k (k.iri frag) "initial" (frag ++ ":ia") "" ids scope).map (fun d => k.iri d.1) := by
  rw [← targets_eq]; exact self_ref k frag "initial" _ "" (ch_ia frag) ids scope hr

theorem self_transS (frag : String) (scope : Term) (trs : List Transition) {P o : Term}
    (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (trs.zipIdx.map (transK k frag scope)).flatMap Prod.snd ↔
      P = vt "transition" ∧ o ∈ (trs.zipIdx.map (transK k frag scope)).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_transK k frag scope trs)
    (ch_of_fst (by simp only [fst_transK, List.forall_mem_map]; intro p _; exact ch_lit frag ":t" p.2 (by simp) (by simp))) hr

theorem self_hookS (frag seg prop cls : String) (hs : seg.toList.head? = some ':')
    (hn : NC seg.toList.tail) (bs : List Block) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (bs.zipIdx.map (hookK k frag seg prop cls)).flatMap Prod.snd ↔
      P = vt prop ∧ o ∈ (bs.zipIdx.map (hookK k frag seg prop cls)).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_hookK k frag seg prop cls bs)
    (ch_of_fst (by simp only [fst_hookK, List.forall_mem_map]; intro p _; exact ch_lit frag seg p.2 hs hn)) hr

theorem self_dataS (frag : String) (ds : List Data) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (ds.zipIdx.map (dataK k frag)).flatMap Prod.snd ↔
      P = vt "data" ∧ o ∈ (ds.zipIdx.map (dataK k frag)).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_dataK k frag ds)
    (ch_of_fst (by simp only [fst_dataK, List.forall_mem_map]; intro p _; exact ch_lit frag ":d" p.2 (by simp) (by simp))) hr

theorem self_invS (frag : String) (vs : List Invoke) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (vs.zipIdx.map (invK k frag)).flatMap Prod.snd ↔
      P = vt "invoke" ∧ o ∈ (vs.zipIdx.map (invK k frag)).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_invK k frag vs)
    (ch_of_fst (by simp only [fst_invK, List.forall_mem_map]; intro p _; exact ch_lit frag ":inv" p.2 (by simp) (by simp))) hr

theorem self_initS (frag : String) (ie : Option Initial) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (initKidsOpt k frag ie).flatMap Prod.snd ↔
      P = vt "initialElement" ∧ o ∈ (initKidsOpt k frag ie).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_initKidsOpt k frag ie)
    (ch_of_fst (by cases ie <;> simp [Ch, String.toList_append])) hr

theorem self_ddS (frag : String) (dd : Option DoneData) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (ddKidsOpt k frag dd).flatMap Prod.snd ↔
      P = vt "donedata" ∧ o ∈ (ddKidsOpt k frag dd).map (fun d => k.iri d.1) :=
  self_kids k (kidsL_ddKidsOpt k frag dd)
    (ch_of_fst (by cases dd <;> simp [Ch, String.toList_append])) hr

theorem self_extS (frag : String) (xs : List Extension) {P o : Term} (hr : Rel ⟨k.iri frag, P, o⟩) :
    ⟨k.iri frag, P, o⟩ ∈ (extKids k frag xs).flatMap Prod.snd ↔
      P = vt "extension" ∧ o ∈ (extKids k frag xs).map (fun d => k.iri d.1) := by
  rw [← extensions_eq]; exact self_ext k frag xs hr

/-! ## The core of a state -/

/-- The kids of a state, by its fields. -/
def kidsOf (parent : Term) (ancestors : List Term) (s : State) : List (String × List Triple) :=
  stateKids k (stateFragment s.id) (scopeOf parent ancestors (k.iri (stateFragment s.id)) s.element)
    s.initial s.initialElement s.transitions s.onentry s.onexit s.data s.invokes s.donedata
    s.extensions

/-- A state's own triples, its kids and its child links: everything about or below its node. -/
def core (order : Nat) (parent : Term) (ancestors : List Term) (s : State) : List Triple :=
  stateSelf k order parent ancestors s.id s.idGenerated s.element ++
    (kidsOf k parent ancestors s).flatMap Prod.snd ++
    childLinks k (k.iri (stateFragment s.id)) s.children

theorem state_eq' (order : Nat) (parent : Term) (ancestors : List Term) (s : State) :
    k.state order parent ancestors s =
      stateSelf k order parent ancestors s.id s.idGenerated s.element ++
        (kidsOf k parent ancestors s).flatMap Prod.snd ++
        k.states 0 (k.iri (stateFragment s.id)) (ancestors ++ [k.iri (stateFragment s.id)])
          s.children := by
  rcases s with ⟨id, idGen, el, initial, initEl, children, trs, onentry, onexit, data, invokes,
    dd, exts⟩
  exact state_eq k ..

theorem decomp_core (order : Nat) (parent : Term) (ancestors : List Term) (s : State) :
    Decomp k parent "child" (stateFragment s.id) (core k order parent ancestors s)
      (kidsOf k parent ancestors s) := by
  constructor
  · intro t ht hr
    simp only [core, List.mem_append] at ht
    rcases ht with (ht | ht) | ht
    · simp only [stateSelf, List.mem_append, mem_node, mem_histT, mem_str, mem_bool, mem_ordT,
        List.mem_map, List.mem_cons, List.not_mem_nil, or_false] at ht
      rcases ht with (((((h | ⟨_, _, h⟩) | ⟨_, _, h⟩) | h) | h) | h) | ⟨_, _, h⟩ <;>
        first | exact .inl h | exact .inr (.inl (by subst h; rfl))
    · exact .inr (.inr (List.mem_flatMap.1 ht))
    · simp only [childLinks, List.mem_map] at ht
      obtain ⟨c, _, rfl⟩ := ht
      exact .inr (.inl rfl)
  · apply ch_of_fst
    simp only [kidsOf, stateKids, List.map_append, fst_refKids, fst_initKidsOpt, fst_transK,
      fst_hookK, fst_dataK, fst_invK, fst_ddKidsOpt, fst_extKids, List.forall_mem_append,
      List.forall_mem_map]
    refine ⟨⟨⟨⟨⟨⟨⟨⟨fun p _ => ch_ia _ p.2, ?_⟩, ?_⟩, ?_⟩, ?_⟩, ?_⟩, ?_⟩, ?_⟩, ?_⟩
    · cases s.initialElement <;> simp [Ch, String.toList_append]
    all_goals first
      | (cases s.donedata <;> simp [Ch, String.toList_append])
      | ch_tac
  · simp only [kidsOf, stateKids]
    exact kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsLoc_append k
      (kidsLoc_append k (kidsLoc_append k (kidsLoc_append k (kidsL_ref k _ _ _ _ _ _).loc
      (kidsL_initKidsOpt k _ _).loc) (kidsL_transK k _ _ _).loc) (kidsL_hookK k _ _ _ _ _).loc)
      (kidsL_hookK k _ _ _ _ _).loc) (kidsL_dataK k _ _).loc) (kidsL_invK k _ _).loc)
      (kidsL_ddKidsOpt k _ _).loc) (kidsL_ext k _ _).loc
  · simp only [kidsOf, stateKids, List.map_append, fst_refKids, fst_initKidsOpt, fst_transK,
      fst_hookK, fst_dataK, fst_invK, fst_ddKidsOpt, fst_extKids]
    cases s.initialElement <;> cases s.donedata <;> simp [List.nodup_append] <;> nd2_tac
    all_goals (simp only [tl_eq_comm]; intros; (try intro); simp_all)
  · intro d hd
    simp only [core, stateSelf, Ctx.node, List.cons_append, List.tail_cons]
    have := sub_flatMap hd
    sub_tac

end

/-! ## Capture of a state's core and of its child states -/

variable {k : Ctx} {ts : List Triple}

/-- What decoding a state needs: capture of its emission over its subtree's region, unique
identifiers in the subtree, and an owner outside the region. -/
structure SH (k : Ctx) (ts : List Triple) (order : Nat) (parent : Term) (ancestors : List Term)
    (s : State) : Prop where
  cap : Cap k ts (RS s) (k.state order parent ancestors s)
  nd : (s.flatten.map State.id).Nodup
  own : ∀ g, RS s g → parent ≠ k.iri g

theorem flatten_eq (s : State) : s.flatten = s :: State.flattenList s.children := by
  rcases s with ⟨id, idGen, el, initial, initEl, children, trs, onentry, onexit, data, invokes,
    dd, exts⟩
  rw [State.flatten]; rfl

theorem not_RSs_self {s : State} (hnd : (s.flatten.map State.id).Nodup) {g : String}
    (hU : U (stateFragment s.id) g) : ¬ RSs s.children g := by
  rintro ⟨Y, hY, hY'⟩
  rw [flatten_eq, List.map_cons, List.nodup_cons] at hnd
  exact hnd.1 (List.mem_map.2 ⟨Y, hY, U_sf hY' hU⟩)

theorem RS_self (s : State) {g : String} (h : U (stateFragment s.id) g) : RS s g :=
  ⟨s, by rw [flatten_eq]; exact List.mem_cons_self, h⟩

theorem RS_kid (s : State) {g : String} (h : RSs s.children g) : RS s g := by
  obtain ⟨Y, hY, hU⟩ := h
  exact ⟨Y, by rw [flatten_eq]; exact List.mem_cons_of_mem _ hY, hU⟩

theorem SH.coreCap {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    Cap k ts (U (stateFragment s.id)) (core k order parent ancestors s) := by
  refine Cap.split h.cap ?_ (fun g hU => RS_self s hU) ?_
  · rw [state_eq', core]
    exact (childLinks_sub k _ _ 0 _).append_left _
  · intro t ht hr
    rw [state_eq'] at ht
    rcases List.mem_append.1 ht with ht | ht
    · exact .inl (List.mem_append_left _ ht)
    · rcases states_cov k _ _ 0 _ t ht hr with h' | ⟨g', hg', hR⟩
      · exact .inl (List.mem_append_right _ h')
      · refine .inr fun g hg hU => ?_
        rw [hg', iri_inj] at hg; subst hg
        exact not_RSs_self h.nd hU hR

theorem SH.ownFrag {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) : Above (k := k) parent (stateFragment s.id) :=
  fun g hU => h.own g (RS_self s hU)

theorem SH.kidsCap {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    Cap k ts (RSs s.children) (k.states 0 (k.iri (stateFragment s.id))
      (ancestors ++ [k.iri (stateFragment s.id)]) s.children) := by
  refine Cap.split h.cap ?_ (fun g hR => RS_kid s hR) ?_
  · rw [state_eq']; exact List.sublist_append_right _ _
  · intro t ht hr
    rw [state_eq'] at ht
    rcases List.mem_append.1 ht with ht | ht
    · have hc : t ∈ core k order parent ancestors s := List.mem_append_left _ ht
      refine .inr fun g hg hR => ?_
      rcases (decomp_core k order parent ancestors s).locL t hc hr with rfl | ⟨g', hg', hU⟩
      · exact h.own g (RS_kid s hR) hg
      · rw [hg', iri_inj] at hg; subst hg
        exact not_RSs_self h.nd hU hR
    · exact .inl ht

theorem SH.kidsOwn {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    ∀ g, RSs s.children g → k.iri (stateFragment s.id) ≠ k.iri g := by
  intro g hR he
  rw [iri_inj] at he; subst he
  exact not_RSs_self h.nd (U_refl _) hR

theorem SH.kidsNd {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    ((State.flattenList s.children).map State.id).Nodup := by
  have := h.nd
  rw [flatten_eq, List.map_cons, List.nodup_cons] at this
  exact this.2

theorem states_cons (i : Nat) (P : Term) (A : List Term) (c : State) (cs : List State) :
    k.states i P A (c :: cs) = k.state i P A c ++ k.states (i + 1) P A cs := by
  rw [Ctx.states]

theorem RSs_cons {c : State} {cs : List State} {g : String} :
    RSs (c :: cs) g ↔ RS c g ∨ RSs cs g := by
  simp only [RSs, RS, flattenList_cons, List.mem_append]
  constructor
  · rintro ⟨Y, hY | hY, hU⟩
    · exact .inl ⟨Y, hY, hU⟩
    · exact .inr ⟨Y, hY, hU⟩
  · rintro (⟨Y, hY, hU⟩ | ⟨Y, hY, hU⟩)
    · exact ⟨Y, .inl hY, hU⟩
    · exact ⟨Y, .inr hY, hU⟩

/-- Capture, unique identifiers and ownership pass to each of a list of sibling states. -/
theorem capList (P : Term) (A : List Term) :
    ∀ (cs : List State) (i : Nat), Cap k ts (RSs cs) (k.states i P A cs) →
      ((State.flattenList cs).map State.id).Nodup → (∀ g, RSs cs g → P ≠ k.iri g) →
      ∀ p ∈ cs.zipIdx i, SH k ts p.2 P A p.1
  | [], _, _, _, _ => by simp
  | c :: cs, i, hc, hnd, hown => by
    rw [flattenList_cons, List.map_append, List.nodup_append] at hnd
    obtain ⟨hnd1, hnd2, hdis⟩ := hnd
    have disj : ∀ g, RS c g → RSs cs g → False := fun g h1 h2 => by
      rw [RS_iff] at h1; rw [RSs_iff] at h2
      obtain ⟨x, hx, hx'⟩ := RI_disj h1 h2
      exact hdis x hx x hx' rfl
    have own1 : ∀ g, RS c g → P ≠ k.iri g := fun g h => hown g (RSs_cons.2 (.inl h))
    have own2 : ∀ g, RSs cs g → P ≠ k.iri g := fun g h => hown g (RSs_cons.2 (.inr h))
    rw [states_cons] at hc
    have c1 : Cap k ts (RS c) (k.state i P A c) := by
      refine Cap.split hc (List.sublist_append_left _ _) (fun g h => RSs_cons.2 (.inl h)) ?_
      intro t ht hr
      rcases List.mem_append.1 ht with ht | ht
      · exact .inl ht
      · refine .inr fun g hg hR => ?_
        rcases states_cov k P A (i + 1) cs t ht hr with h' | ⟨g', hg', hR'⟩
        · simp only [childLinks, List.mem_map] at h'
          obtain ⟨_, _, rfl⟩ := h'
          exact own1 g hR hg
        · rw [hg', iri_inj] at hg; subst hg
          exact disj _ hR hR'
    have c2 : Cap k ts (RSs cs) (k.states (i + 1) P A cs) := by
      refine Cap.split hc (List.sublist_append_right _ _) (fun g h => RSs_cons.2 (.inr h)) ?_
      intro t ht hr
      rcases List.mem_append.1 ht with ht | ht
      · refine .inr fun g hg hR => ?_
        rcases locS_state k i P A c t ht hr with rfl | ⟨g', hg', hR'⟩
        · exact own2 g hR hg
        · rw [hg', iri_inj] at hg; subst hg
          exact disj _ hR' hR
      · exact .inl ht
    intro p hp
    simp only [List.zipIdx_cons, List.mem_cons] at hp
    rcases hp with rfl | hp
    · exact ⟨c1, hnd1, own1⟩
    · exact capList P A cs (i + 1) c2 hnd2 own2 p hp

/-! ## Lookups about a state node -/

syntax "lksm" term : tactic
macro_rules
  | `(tactic| lksm $ho) => `(tactic|
    (simp only [core, kidsOf, stateKids, List.flatMap_append, List.mem_append]
     simp only [self_refS, self_initS, self_transS,
       self_hookS _ _ ":en" "onentry" "OnEntry" (by simp) (by simp),
       self_hookS _ _ ":ex" "onexit" "OnExit" (by simp) (by simp), self_dataS, self_invS,
       self_ddS, self_extS, rel_mk, rel_type, ne_eq, String.reduceEq, not_false_eq_true,
       and_self, and_true, true_and]
     simp [stateSelf, childLinks, strLit, intLit, boolLit, ($ho), (Ne.symm $ho)]))

syntax "lks" term "," term : tactic
macro_rules
  | `(tactic| lks $h, $ho) => `(tactic|
    (intro o
     rw [($h).lookup (U_refl _) (by simp)]
     lksm $ho))

theorem SH.id_lk {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    Lk ts (k.iri (stateFragment s.id)) "id" [strLit s.id] := by
  have hc := h.coreCap
  have ho := above_ne h.ownFrag
  lks hc, ho

theorem SH.order_lk {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (h : SH k ts order parent ancestors s) :
    Lk ts (k.iri (stateFragment s.id)) "order" [intLit order] := by
  have hc := h.coreCap
  have ho := above_ne h.ownFrag
  lks hc, ho

/-! ## Decoding a state -/

/-- The targets of a state's `initial` attribute, `<initial>` element and transitions satisfy
`L`. -/
def TgtL (L : String → Prop) (Y : State) : Prop :=
  (∀ x ∈ Y.initial, L x) ∧ (∀ ie, Y.initialElement = some ie → ∀ x ∈ ie.transition.targets, L x) ∧
    (∀ t ∈ Y.transitions, ∀ x ∈ t.targets, L x)

/-- The node of state `x` carries exactly its identifier. -/
def IdL (k : Ctx) (ts : List Triple) (x : String) : Prop :=
  Lk ts (k.iri (stateFragment x)) "id" [strLit x]

theorem children_sub : ∀ cs : List State, cs.Sublist (State.flattenList cs)
  | [] => by simp [State.flattenList]
  | c :: cs => by
    rw [flattenList_cons, flatten_eq, List.cons_append]
    exact (children_sub cs).trans (List.sublist_append_right _ _) |>.cons_cons c

theorem kids_nodup {s : State} (hnd : (s.flatten.map State.id).Nodup) :
    (s.children.map (fun c => k.iri (stateFragment c.id))).Nodup := by
  rw [flatten_eq, List.map_cons, List.nodup_cons] at hnd
  have h1 : (s.children.map State.id).Nodup := (hnd.2.sublist ((children_sub _).map _))
  have : s.children.map (fun c => k.iri (stateFragment c.id)) =
      (s.children.map State.id).map (fun x => k.iri (stateFragment x)) := by
    simp [List.map_map, Function.comp_def]
  rw [this]
  rw [List.Nodup, List.pairwise_map]
  exact h1.imp (fun hne he => hne (stateFragment_inj ((iri_inj k).1 he)))

theorem fst_mem_zipIdx {α : Type} {l : List α} {p : α × Nat} {i : Nat} (hp : p ∈ l.zipIdx i) :
    p.1 ∈ l := by
  rcases p with ⟨x, j⟩
  have := List.mem_zipIdx hp
  exact this.2.2 ▸ List.getElem_mem _

theorem State.eta (s : State) :
    State.mk s.id s.idGenerated s.element s.initial s.initialElement s.children s.transitions
      s.onentry s.onexit s.data s.invokes s.donedata s.extensions = s := by
  rcases s; rfl

syntax "fin_tac" : tactic
set_option hygiene false in
macro_rules
  | `(tactic| fin_tac) => `(tactic|
    (rcases hel : s.element with _ | _ | _ | (_ | _) <;> rw [hel] at h_type <;>
     first
     | (have a5 := h_ht _ hel
        clear h_ht
        simp [Decode.state, bind, Except.bind, pure, Except.pure, Functor.map, Except.map,
          elementClass, *, -hel]
        rw [← hel, ← hie, ← hdd]; exact State.eta s)
     | (clear h_ht
        simp [Decode.state, bind, Except.bind, pure, Except.pure, Functor.map, Except.map,
          elementClass, *, -hel]
        rw [← hel, ← hie, ← hdd]; exact State.eta s)))

theorem dec_state_step {order : Nat} {parent : Term} {ancestors : List Term} {s : State}
    (fuel d : Nat) (hf : ts.length ≤ fuel) (h : SH k ts order parent ancestors s)
    (hT : TgtL (IdL k ts) s)
    (hkids : (s.children.map (fun c => k.iri (stateFragment c.id))).mapM
      (Decode.state (Index.of ts) k.exprDt fuel d) = .ok s.children) :
    Decode.state (Index.of ts) k.exprDt fuel (d + 1) (k.iri (stateFragment s.id)) = .ok s := by
  have hc := h.coreCap
  have ho := above_ne h.ownFrag
  have cx : Ctxt k ts parent "child" (stateFragment s.id) _ _ :=
    ⟨hc, decomp_core k order parent ancestors s, h.ownFrag⟩
  have h_id : Decode.string (Index.of ts) (k.iri (stateFragment s.id)) "id" = .ok s.id :=
    string_eq h.id_lk
  have h_gen : Decode.boolean (Index.of ts) (k.iri (stateFragment s.id)) "idGenerated" =
      .ok s.idGenerated := boolean_eq (by lks hc, ho)
  have h_ini : Decode.targetIds (Index.of ts) (k.iri (stateFragment s.id)) "initial" =
      .ok s.initial :=
    dec_targets cx "initial" _ "" (ch_ia _) s.initial (k.iri (stateFragment s.id))
      (fun p hp => by simp [kidsOf, stateKids, refKids, List.mem_map_of_mem hp])
      (by lks hc, ho) hT.1
  have h_ext : Decode.extensions (Index.of ts) (k.iri (stateFragment s.id)) = .ok s.extensions :=
    dec_extensions cx s.extensions (fun d hd => by simp [kidsOf, stateKids, hd]) (by lks hc, ho)
  have h_type : ∀ cls, Decode.hasType (Index.of ts) (k.iri (stateFragment s.id)) cls =
      (cls == "StateNode" || cls == elementClass s.element) := fun cls =>
    hasType_of hc cls (by lksm ho)
  have h_ht : ∀ hty, s.element = .history hty →
      Decode.string (Index.of ts) (k.iri (stateFragment s.id)) "historyType" =
        .ok (match hty with
          | .shallow => "shallow"
          | .deep => "deep") := fun hty he =>
    string_eq (by intro o; rw [hc.lookup (U_refl _) (by simp)]; lksm ho; simp [he])
  -- child states
  have h_cho : Decode.ordered (Index.of ts) (k.iri (stateFragment s.id)) "child" =
      .ok (s.children.map (fun c => k.iri (stateFragment c.id))) := by
    refine ordered_eq (by lks hc, ho) (kids_nodup h.nd) (fun m hm => ?_) (fun j hj => ?_)
    · obtain ⟨c, _, rfl⟩ := List.mem_map.1 hm; exact ⟨_, rfl⟩
    · simp only [List.length_map] at hj
      have hp : (s.children[j], j) ∈ s.children.zipIdx := by
        rw [List.mem_zipIdx_iff_getElem?]; simp [hj]
      have := (capList _ _ s.children 0 h.kidsCap h.kidsNd h.kidsOwn _ hp).order_lk
      simpa using this
  -- transitions
  have hTK : ∀ p ∈ s.transitions.zipIdx, Cap k ts (U (stateFragment s.id ++ ":t" ++ toString p.2))
      (k.transition p.1 (k.iri (stateFragment s.id)) "transition"
        (stateFragment s.id ++ ":t" ++ toString p.2) p.2 (k.iri (stateFragment s.id))
        (scopeOf parent ancestors (k.iri (stateFragment s.id)) s.element)) := fun p hp =>
    cx.kid (d := transK k (stateFragment s.id) (scopeOf parent ancestors (k.iri (stateFragment s.id)) s.element) p) (by simp [kidsOf, stateKids, List.mem_map_of_mem hp])
  have h_tro : Decode.ordered (Index.of ts) (k.iri (stateFragment s.id)) "transition" =
      .ok ((s.transitions.zipIdx.map (transK k (stateFragment s.id)
        (scopeOf parent ancestors (k.iri (stateFragment s.id)) s.element))).map (fun d => k.iri d.1)) :=
    ordered_kids s.transitions _ (by lks hc, ho) (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _)
      (fun p hp => by
        have h' := hTK p hp
        have hoK := above_ne (k := k) (above_of_ch (ch_lit (stateFragment s.id) ":t" p.2 (by simp) (by simp)))
        simp only [transK]
        lkt h', hoK)
  have h_trm := mapM_kids (k := k) (Decode.transition (Index.of ts) k.exprDt fuel)
    (transK k (stateFragment s.id) (scopeOf parent ancestors (k.iri (stateFragment s.id)) s.element))
    id s.transitions 0 (fun p hp => dec_transition _ _ _ _ _ _ _ (hTK p hp)
      (above_of_ch (ch_lit _ ":t" p.2 (by simp) (by simp))) (hT.2.2 p.1 (fst_mem_zipIdx hp)) fuel hf)
  -- handlers
  have hEN : ∀ p ∈ s.onentry.zipIdx,
      Decode.block (Index.of ts) k.exprDt fuel (k.iri (stateFragment s.id ++ ":en" ++ toString p.2))
        "action" = .ok p.1 ∧ Lk ts (k.iri (stateFragment s.id ++ ":en" ++ toString p.2)) "order"
          [intLit p.2] := fun p hp =>
    dec_hook (cx.kid (d := hookK k (stateFragment s.id) ":en" "onentry" "OnEntry" p)
      (by simp [kidsOf, stateKids, List.mem_map_of_mem hp]))
      (above_of_ch (ch_lit _ ":en" p.2 (by simp) (by simp))) fuel hf
  have hEX : ∀ p ∈ s.onexit.zipIdx,
      Decode.block (Index.of ts) k.exprDt fuel (k.iri (stateFragment s.id ++ ":ex" ++ toString p.2))
        "action" = .ok p.1 ∧ Lk ts (k.iri (stateFragment s.id ++ ":ex" ++ toString p.2)) "order"
          [intLit p.2] := fun p hp =>
    dec_hook (cx.kid (d := hookK k (stateFragment s.id) ":ex" "onexit" "OnExit" p)
      (by simp [kidsOf, stateKids, List.mem_map_of_mem hp]))
      (above_of_ch (ch_lit _ ":ex" p.2 (by simp) (by simp))) fuel hf
  have h_eno := ordered_kids (k := k) (ts := ts) (s := k.iri (stateFragment s.id)) (prop := "onentry")
    s.onentry (hookK k (stateFragment s.id) ":en" "onentry" "OnEntry") (by lks hc, ho)
    (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _) (fun p hp => (hEN p hp).2)
  have h_enm := mapM_kids (k := k) (fun h => Decode.block (Index.of ts) k.exprDt fuel h "action")
    (hookK k (stateFragment s.id) ":en" "onentry" "OnEntry") id s.onentry 0
    (fun p hp => (hEN p hp).1)
  have h_exo := ordered_kids (k := k) (ts := ts) (s := k.iri (stateFragment s.id)) (prop := "onexit")
    s.onexit (hookK k (stateFragment s.id) ":ex" "onexit" "OnExit") (by lks hc, ho)
    (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _) (fun p hp => (hEX p hp).2)
  have h_exm := mapM_kids (k := k) (fun h => Decode.block (Index.of ts) k.exprDt fuel h "action")
    (hookK k (stateFragment s.id) ":ex" "onexit" "OnExit") id s.onexit 0
    (fun p hp => (hEX p hp).1)
  -- data and invokes
  have hDA : ∀ p ∈ s.data.zipIdx, _ := fun p hp =>
    dec_dataItem (cx.kid (d := dataK k (stateFragment s.id) p) (by simp [kidsOf, stateKids, List.mem_map_of_mem hp]))
      (above_of_ch (ch_lit _ ":d" p.2 (by simp) (by simp)))
  have h_dao := ordered_kids (k := k) (ts := ts) (s := k.iri (stateFragment s.id)) (prop := "data")
    s.data (dataK k (stateFragment s.id)) (by lks hc, ho)
    (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _) (fun p hp => (hDA p hp).2)
  have h_dam := mapM_kids (k := k) (Decode.dataItem (Index.of ts) k.exprDt)
    (dataK k (stateFragment s.id)) id s.data 0 (fun p hp => (hDA p hp).1)
  have hIV : ∀ p ∈ s.invokes.zipIdx, _ := fun p hp =>
    dec_invoke (cx.kid (d := invK k (stateFragment s.id) p) (by simp [kidsOf, stateKids, List.mem_map_of_mem hp]))
      (above_of_ch (ch_lit _ ":inv" p.2 (by simp) (by simp))) fuel hf
  have h_ivo := ordered_kids (k := k) (ts := ts) (s := k.iri (stateFragment s.id)) (prop := "invoke")
    s.invokes (invK k (stateFragment s.id)) (by lks hc, ho)
    (nd_kids _ (fun q r h => frag_inj _ _ _ _ h) _ _) (fun p hp => (hIV p hp).2)
  have h_ivm := mapM_kids (k := k) (Decode.invoke (Index.of ts) k.exprDt fuel)
    (invK k (stateFragment s.id)) id s.invokes 0 (fun p hp => (hIV p hp).1)
  -- `<initial>` and done data
  have hIE0 : s.initialElement = none →
      Decode.optNode (Index.of ts) (k.iri (stateFragment s.id)) "initialElement" = .ok none :=
    fun he => optNode_none (by
      intro o; rw [hc.lookup (U_refl _) (by simp)]; lksm ho; simp [he, initKidsOpt])
  have hIE1 : ∀ ie, s.initialElement = some ie →
      Decode.optNode (Index.of ts) (k.iri (stateFragment s.id)) "initialElement" =
        .ok (some (k.iri (stateFragment s.id ++ ":init"))) ∧ _ := fun ie he =>
    ⟨optNode_some ⟨_, rfl⟩ (by
      intro o; rw [hc.lookup (U_refl _) (by simp)]; lksm ho; simp [he, initKidsOpt, eq_comm]),
     dec_init (cx.kid (d := (stateFragment s.id ++ ":init",
        initEmission k (k.iri (stateFragment s.id)) (stateFragment s.id ++ ":init") ie))
        (by simp [kidsOf, stateKids, initKidsOpt, he]))
      (above_of_ch (by simp [Ch, String.toList_append])) (hT.2.1 ie he) fuel hf⟩
  have hDD0 : s.donedata = none →
      Decode.optNode (Index.of ts) (k.iri (stateFragment s.id)) "donedata" = .ok none :=
    fun he => optNode_none (by
      intro o; rw [hc.lookup (U_refl _) (by simp)]; lksm ho; simp [he, ddKidsOpt])
  have hDD1 : ∀ dd, s.donedata = some dd →
      Decode.optNode (Index.of ts) (k.iri (stateFragment s.id)) "donedata" =
        .ok (some (k.iri (stateFragment s.id ++ ":dd"))) ∧ _ := fun dd he =>
    ⟨optNode_some ⟨_, rfl⟩ (by
      intro o; rw [hc.lookup (U_refl _) (by simp)]; lksm ho; simp [he, ddKidsOpt, eq_comm]),
     dec_donedata (cx.kid (d := (stateFragment s.id ++ ":dd",
        k.donedata (k.iri (stateFragment s.id)) (stateFragment s.id ++ ":dd") dd))
        (by simp [kidsOf, stateKids, ddKidsOpt, he]))
      (above_of_ch (by simp [Ch, String.toList_append]))⟩
  simp only [List.map_id] at h_trm h_enm h_exm h_dam h_ivm
  simp only [List.map_map, List.mapM_map] at h_tro h_trm h_eno h_enm h_exo h_exm h_dao h_dam h_ivo h_ivm
  clear hTK hEN hEX hDA hIV cx hc
  rcases hie : s.initialElement with _ | ie
  · have a6 := hIE0 hie
    clear hIE0 hIE1
    rcases hdd : s.donedata with _ | dd
    · have b6 := hDD0 hdd
      clear hDD0 hDD1
      fin_tac
    · obtain ⟨b1, b2, b3⟩ := hDD1 _ hdd
      clear hDD0 hDD1
      fin_tac
  · obtain ⟨a1, a2, a3, a4⟩ := hIE1 _ hie
    clear hIE0 hIE1
    rcases hdd : s.donedata with _ | dd
    · have b6 := hDD0 hdd
      clear hDD0 hDD1
      fin_tac
    · obtain ⟨b1, b2, b3⟩ := hDD1 _ hdd
      clear hDD0 hDD1
      fin_tac

theorem state_len_pos (order : Nat) (parent : Term) (ancestors : List Term) (s : State) :
    1 ≤ (k.state order parent ancestors s).length := by
  obtain ⟨rest, h⟩ := state_head k order parent ancestors s
  rw [h]; simp

theorem states_len (order : Nat) (parent : Term) (ancestors : List Term) (s : State) :
    (k.states 0 (k.iri (stateFragment s.id)) (ancestors ++ [k.iri (stateFragment s.id)])
      s.children).length + 1 ≤ (k.state order parent ancestors s).length := by
  rw [state_eq']
  simp only [stateSelf, Ctx.node, List.length_append, List.length_cons]
  omega

mutual
/-- **Decoding inverts projecting a state**, with its whole subtree, given capture of its
emission over the subtree's region, unique identifiers below it, an owner outside it, and the
`vt:id` of every target state. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "states decoded from any graph capturing their projection"}
theorem dec_state (fuel : Nat) (hf : ts.length ≤ fuel) :
    ∀ (s : State) (depth order : Nat) (parent : Term) (ancestors : List Term),
      SH k ts order parent ancestors s → (∀ Y ∈ s.flatten, TgtL (IdL k ts) Y) →
      (k.state order parent ancestors s).length ≤ depth →
      Decode.state (Index.of ts) k.exprDt fuel depth (k.iri (stateFragment s.id)) = .ok s
  | .mk id idGen el initial initEl children trs onentry onexit data invokes dd exts, depth, order,
      parent, ancestors, h, hT, hl => by
    obtain _ | d := depth
    · have := state_len_pos (k := k) order parent ancestors
        (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts)
      omega
    have hl' := states_len (k := k) order parent ancestors
      (.mk id idGen el initial initEl children trs onentry onexit data invokes dd exts)
    refine dec_state_step fuel d hf h (hT _ (by rw [flatten_eq]; exact List.mem_cons_self)) ?_
    exact dec_states fuel hf children 0 d _ _
      (capList _ _ _ 0 h.kidsCap h.kidsNd h.kidsOwn)
      (fun Y hY => hT Y (by rw [flatten_eq]; exact List.mem_cons_of_mem _ hY))
      (by simp only [State.children, State.id] at hl' ⊢; omega)
/-- Decoding sibling states. -/
theorem dec_states (fuel : Nat) (hf : ts.length ≤ fuel) :
    ∀ (cs : List State) (i d : Nat) (P : Term) (A : List Term),
      (∀ p ∈ cs.zipIdx i, SH k ts p.2 P A p.1) → (∀ Y ∈ State.flattenList cs, TgtL (IdL k ts) Y) →
      (k.states i P A cs).length ≤ d →
      (cs.map (fun c => k.iri (stateFragment c.id))).mapM
        (Decode.state (Index.of ts) k.exprDt fuel d) = .ok cs
  | [], _, _, _, _, _, _, _ => rfl
  | c :: cs, i, d, P, A, h, hT, hl => by
    rw [states_cons, List.length_append] at hl
    have h1 := dec_state fuel hf c d i P A (h (c, i) (by simp))
      (fun Y hY => hT Y (by rw [flattenList_cons]; exact List.mem_append_left _ hY)) (by omega)
    have h2 := dec_states fuel hf cs (i + 1) d P A (fun p hp => h p (by simp [hp]))
      (fun Y hY => hT Y (by rw [flattenList_cons]; exact List.mem_append_right _ hY)) (by omega)
    simp only [List.map_cons, List.mapM_cons, h1, h2]
    rfl
end

end RT
end Vertep.Rdf
