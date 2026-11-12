import VertepLean.Rdf.Equiv.Anc
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# The state rules the node shapes check

`StateOkN` is `StateOk` without the target-set rule (`IllegalTargetSet`), which the selection
nodes check. `sok_iff`: on a chart with unique state identifiers, `SOK` of a state is `StateOkN`
together with the state's non-empty identifier, its data items' rules and, for a history
state, a parent state.
-/

namespace Vertep.Rdf.Equiv

open LeanRDF LeanRDF.Shacl Vertep.Rdf.ShapesL Vertep.Rdf.RT

/-- `StateOk` without `TargetSetOk`. -/
def StateOkN (c : Chart) (p : Option State) (s : State) : Prop :=
  ((s.initial ≠ [] ∨ s.initialElement.isSome) → s.kind = .compound) ∧
  ¬(s.initial ≠ [] ∧ s.initialElement.isSome) ∧
  (∀ x ∈ s.initial, x ∈ c.ids ∧ x ∈ s.descIds) ∧
  (∀ i, s.initialElement = some i →
    i.transition.events = [] ∧ i.transition.cond = none ∧ i.transition.targets ≠ [] ∧
    (∀ x ∈ i.transition.targets, x ∈ c.ids ∧ x ∈ s.descIds) ∧ BlockOk i.transition.body) ∧
  (∀ t ∈ s.transitions, (∀ x ∈ t.targets, x ∈ c.ids) ∧ BlockOk t.body) ∧
  (s.isHistory = true → HistoryOk c p s) ∧
  (s.element = .final → s.children.isEmpty = true ∧ s.transitions.isEmpty = true ∧
    s.invokes.isEmpty = true ∧ s.data = []) ∧
  ((s.kind = .parallel ∨ s.kind = .compound) → s.children ≠ [] →
    s.children.any (fun c => !c.isHistory) = true) ∧
  (s.donedata.isSome → s.element = .final) ∧
  (∀ dd, s.donedata = some dd → ¬(dd.content.isSome ∧ dd.params ≠ []) ∧ ParamsOk dd.params) ∧
  (∀ b ∈ s.onentry ++ s.onexit, BlockOk b) ∧
  (∀ v ∈ s.invokes, InvokeOk v)

/-- The target sets of a state's lists. -/
def TSets (c : Chart) (s : State) : Prop :=
  TargetSetOk c s.initial ∧
  (∀ i, s.initialElement = some i → TargetSetOk c i.transition.targets) ∧
  (∀ t ∈ s.transitions, TargetSetOk c t.targets)

theorem stateOk_iff (c : Chart) (p : Option State) (s : State) :
    StateOk c p s ↔ StateOkN c p s ∧ TSets c s := by
  simp only [StateOk, StateOkN, TSets, InitialTargetsOk, TransitionOk]
  constructor
  · rintro ⟨a, b, ⟨c1, c2⟩, d, e, f, g, h, i, j, l, m⟩
    exact ⟨⟨a, b, c1, fun ie hie => ⟨(d ie hie).1, (d ie hie).2.1, (d ie hie).2.2.1, (d ie hie).2.2.2.1.1,
      (d ie hie).2.2.2.2⟩, fun t ht => ⟨(e t ht).1, (e t ht).2.2⟩, f, g, h, i, j, l, m⟩,
      c2, fun ie hie => (d ie hie).2.2.2.1.2, fun t ht => (e t ht).2.1⟩
  · rintro ⟨⟨a, b, c1, d, e, f, g, h, i, j, l, m⟩, c2, d2, e2⟩
    exact ⟨a, b, ⟨c1, c2⟩, fun ie hie => ⟨(d ie hie).1, (d ie hie).2.1, (d ie hie).2.2.1,
      ⟨(d ie hie).2.2.2.1, d2 ie hie⟩, (d ie hie).2.2.2.2⟩, fun t ht => ⟨(e t ht).1, e2 t ht, (e t ht).2⟩,
      f, g, h, i, j, l, m⟩

@[simp] theorem ish_mk (id : String) (ig : Bool) (el : StateElement) (ini : List String) (ie : Option Initial)
    (ch : List State) (trs : List Transition) (en ex : List Block) (da : List Data) (inv : List Invoke)
    (dd : Option DoneData) (xs : List Extension) :
    (State.mk id ig el ini ie ch trs en ex da inv dd xs).isHistory = (match el with | .history _ => true | _ => false) := rfl
@[simp] theorem kind_mk (id : String) (ig : Bool) (el : StateElement) (ini : List String) (ie : Option Initial)
    (ch : List State) (trs : List Transition) (en ex : List Block) (da : List Data) (inv : List Invoke)
    (dd : Option DoneData) (xs : List Extension) :
    (State.mk id ig el ini ie ch trs en ex da inv dd xs).kind = (match el with
      | .state => if ch.isEmpty then .atomic else .compound
      | .parallel => .parallel | .final => .final | .history h => .history h) := by
  cases el <;> rfl

theorem descIds_nil {s : State} (h : s.children = []) : s.descIds = [] := by
  simp [State.descIds, h, State.flattenList]

section
variable (c : Chart) (base : String) (hnd : c.ids.Nodup)
include hnd

set_option maxHeartbeats 4000000 in
theorem sok_iff {q : Option State × State} (hq : q ∈ c.parented) :
    SOK (ctxOf c base) (project c base) (· ∈ c.ids) q.1 q.2 ↔
      q.2.id ≠ "" ∧ (∀ d ∈ q.2.data, DataOk d) ∧ StateOkN c q.1 q.2 ∧
        (q.2.isHistory = true → q.1.isSome = true) := by
  have hY := parented_allStates hq
  have hP : ∀ P, q.1 = some P → P ∈ c.allStates := fun P hP => parent_allStates hq hP
  rcases q with ⟨pp, Y⟩
  simp only at hY hP ⊢
  have tY := fun x => tr_state c base hnd x hY
  have tC := tr_chart c base hnd
  have hsc : ∀ x, TR (ctxOf c base) (project c base) (· ∈ c.ids) x
      (scopeOf (parT (ctxOf c base) pp) [(ctxOf c base).iri "chart"] ((ctxOf c base).iri (stateFragment Y.id)) Y.element) ↔
      x ∈ c.ids ∧ (Y.isHistory = true → ∀ P, pp = some P → x ∈ P.descIds) := by
    intro x
    rcases Y with ⟨id, ig, el, ini, ie, ch, trs, en, ex, da, inv, dd, xs⟩
    cases el <;> simp only [scopeOf, State.element, List.headD, State.isHistory] <;> try simp [tC]
    rcases pp with _ | P
    · simp [parT, tC]
    · simp only [parT, tr_state c base hnd x (hP P rfl)]; simp
  rcases Y with ⟨id, ig, el, ini, ie, ch, trs, en, ex, da, inv, dd, xs⟩
  simp only [State.id, State.initial, State.initialElement, State.children, State.transitions, State.onentry,
    State.onexit, State.data, State.invokes, State.donedata, State.element] at tY hsc ⊢
  have hD : ∀ x, x ∈ (State.mk id ig el ini ie ch trs en ex da inv dd xs).descIds → ch ≠ [] := by
    intro x hx hch; rw [descIds_nil (by simp [State.children, hch])] at hx; simp at hx
  have hD2 : ∀ (l : List String), l ≠ [] →
      (∀ x ∈ l, x ∈ (State.mk id ig el ini ie ch trs en ex da inv dd xs).descIds) → ch ≠ [] := by
    intro l hl h
    cases l with
    | nil => exact absurd rfl hl
    | cons x l => exact hD x (h x (by simp))
  have hini := hD2 ini
  have hiet : ∀ i : Initial, ie = some i → i.transition.targets ≠ [] →
      (∀ x ∈ i.transition.targets, x ∈ (State.mk id ig el ini ie ch trs en ex da inv dd xs).descIds) →
      ch ≠ [] := fun i _ => hD2 _
  have hiet' : ie ≠ none → (∀ i : Initial, ie = some i → i.transition.targets ≠ []) →
      (∀ i : Initial, ie = some i → ∀ x ∈ i.transition.targets,
        x ∈ (State.mk id ig el ini ie ch trs en ex da inv dd xs).descIds) → ch ≠ [] := by
    intro h1 h2 h3
    rcases hie : ie with _ | i
    · exact absurd hie h1
    · exact hiet i hie (h2 i hie) (h3 i hie)
  have hhead : trs.length ≤ 1 → ∀ t, (t ∈ trs ↔ trs.head? = some t) := by
    intro hl t
    match trs, hl with
    | [], _ => simp
    | [a], _ => simp [eq_comm]
  cases el <;>
  simp only [SOK, ElemOK, KidsOK, StateOkN, HistoryOk, notHistoryOnly', ish_mk, kind_mk, State.element,
    State.initial, State.initialElement, State.children, State.transitions, State.onentry, State.onexit,
    State.data, State.invokes, State.donedata, State.id, tY, hsc, List.isEmpty_iff, List.any_eq_true,
    Bool.not_eq_true', Option.isSome_iff_ne_none] at hD hD2 hini hiet hiet' ⊢
  all_goals grind

end

end Vertep.Rdf.Equiv
