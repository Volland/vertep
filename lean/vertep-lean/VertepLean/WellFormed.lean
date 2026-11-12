import VertepLean.IR

/-!
# Well-formedness

`WellFormed` states, as a `Prop`, every rule of the chart model's well-formedness validation
(chart-model, *Well-Formedness Validation*) exactly as `crates/vertep-core/src/validate.rs`
checks them, one conjunct per Rust violation kind:

* `EmptyChart` — the root has at least one child state;
* `MissingId`, `DuplicateStateId` — state identifiers are non-empty and unique;
* `UnknownTarget` — every target of a transition, an `initial` attribute, an `<initial>` element
  and the root `initial` names a state;
* `IllegalInitialTarget` — `initial` attribute and `<initial>` targets are proper descendants of
  the owning state;
* `IllegalTargetSet` — the targets of one transition (a history target at its parent's position)
  are pairwise distinct, unrelated by descent, and their lowest common proper ancestor is a
  parallel state;
* `HistoryDefaultCycle` — following history default entries never leads back to the history
  state it started from;
* `InitialOnNonCompound`, `InitialAttributeAndElement`, `IllegalInitialTransition`;
* `HistoryAtRoot`, `HistoryTransitionCount`, `IllegalHistoryTransition`,
  `IllegalHistoryTarget`, `HistoryContent`;
* `FinalContent`, `DoneDataOutsideFinal`;
* `ParallelHistoryOnly`, `StateHistoryOnly`;
* `DuplicateDataId`;
* `ConflictingAttributes` — on `data`, `donedata`, `param`, `invoke`, `send`, `cancel`,
  `assign`, `if` (an `else` only last and never first) and `foreach` (non-empty `item`).

`checkWF` is the decision procedure (the `Decidable` instance assembled from decidable parts,
with the recursive executable-content rules decided by `execOkB` and `execOkB_iff`), and
`checkWF_iff` is its correctness theorem.
-/

namespace Vertep

/-! ## Flattening -/

mutual
/-- The state and all its descendants, in document order (preorder). -/
def State.flatten : State → List State
  | s@(.mk _ _ _ _ _ cs ..) => s :: State.flattenList cs
/-- The flattening of a list of sibling states, in document order. -/
def State.flattenList : List State → List State
  | [] => []
  | c :: cs => c.flatten ++ State.flattenList cs
end

mutual
/-- Every state paired with its parent state (`none` for children of the root). -/
def State.parented (p : Option State) : State → List (Option State × State)
  | s@(.mk _ _ _ _ _ cs ..) => (p, s) :: State.parentedList (some s) cs
/-- `parented` over a list of siblings with the same parent. -/
def State.parentedList (p : Option State) : List State → List (Option State × State)
  | [] => []
  | c :: cs => State.parented p c ++ State.parentedList p cs
end

/-- Identifiers of the proper descendants of a state. -/
def State.descIds (s : State) : List String := (State.flattenList s.children).map State.id

/-- Whether a state is a history state. -/
def State.isHistory (s : State) : Bool :=
  match s.element with
  | .history _ => true
  | _ => false

namespace Chart

/-- Every state of the chart in document order. -/
def allStates (c : Chart) : List State := State.flattenList c.states

/-- Every state identifier of the chart in document order. -/
def ids (c : Chart) : List String := c.allStates.map State.id

/-- Every state with its parent. -/
def parented (c : Chart) : List (Option State × State) := State.parentedList none c.states

/-- Every data item: the root's, then each state's in document order. -/
def allData (c : Chart) : List Data := c.data ++ c.allStates.flatMap State.data

/-- The first state with identifier `x`. -/
def find (c : Chart) (x : String) : Option State := c.allStates.find? (·.id == x)

/-- The parent of the first state with the identifier of `s` (`none` at the top level). -/
def parentOf (c : Chart) (s : State) : Option State :=
  (c.parented.find? (fun p => p.2.id == s.id)).bind (·.1)

/-- Proper ancestors of `s` below the root, nearest first. -/
def ancestorsF (c : Chart) : Nat → State → List State
  | 0, _ => []
  | n + 1, s => match c.parentOf s with
    | some p => p :: c.ancestorsF n p
    | none => []

/-- Proper ancestors of `s` below the root, nearest first. -/
def ancestors (c : Chart) (s : State) : List State := c.ancestorsF c.allStates.length s

end Chart

/-! ## Rules -/

/-- `<param>`: exactly one of `expr` and `location`. -/
def ParamsOk (ps : List Param) : Prop := ∀ p ∈ ps, p.expr.isSome ≠ p.location.isSome

/-- `<send>` attribute conflicts. -/
def SendOk (s : Send) : Prop :=
  ¬(s.event.isSome ∧ s.eventexpr.isSome) ∧ ¬(s.target.isSome ∧ s.targetexpr.isSome) ∧
  ¬(s.type.isSome ∧ s.typeexpr.isSome) ∧ ¬(s.id.isSome ∧ s.idlocation.isSome) ∧
  ¬(s.delay.isSome ∧ s.delayexpr.isSome) ∧
  ¬(s.content.isSome ∧ (s.params ≠ [] ∨ s.namelist ≠ [])) ∧ ParamsOk s.params

instance (ps : List Param) : Decidable (ParamsOk ps) := by unfold ParamsOk; infer_instance
instance (s : Send) : Decidable (SendOk s) := by unfold SendOk; infer_instance

mutual
/-- The attribute rules of one executable content element and everything nested in it. -/
def ExecOk : Exec → Prop
  | .mk k _ => ExecKindOk k
/-- The attribute rules of one executable content element, by kind. -/
def ExecKindOk : ExecKind → Prop
  | .send s => SendOk s
  | .cancel a b => a.isSome ≠ b.isSome
  | .assign _ e c => ¬(e.isSome ∧ c.isSome)
  | .if_ bs => (∀ b ∈ bs.dropLast, b.1.isSome) ∧ (∀ b, bs.head? = some b → b.1.isSome) ∧
      BranchesOk bs
  | .foreach _ item _ body => item ≠ "" ∧ BlockOk body
  | .raise _ => True
  | .log _ _ => True
  | .script _ => True
  | .extension _ => True
/-- The rules of a block. -/
def BlockOk : List Exec → Prop
  | [] => True
  | e :: es => ExecOk e ∧ BlockOk es
/-- The rules of the branches of an `if`. -/
def BranchesOk : List (Option Expr × List Exec) → Prop
  | [] => True
  | (_, b) :: bs => BlockOk b ∧ BranchesOk bs
end

mutual
/-- Decide `ExecOk`. -/
def execOkB : Exec → Bool
  | .mk k _ => execKindOkB k
/-- Decide `ExecKindOk`. -/
def execKindOkB : ExecKind → Bool
  | .send s => decide (SendOk s)
  | .cancel a b => a.isSome != b.isSome
  | .assign _ e c => !(e.isSome && c.isSome)
  | .if_ bs => bs.dropLast.all (·.1.isSome) && (match bs.head? with
      | some b => b.1.isSome
      | none => true) && branchesOkB bs
  | .foreach _ item _ body => item != "" && blockOkB body
  | .raise _ => true
  | .log _ _ => true
  | .script _ => true
  | .extension _ => true
/-- Decide `BlockOk`. -/
def blockOkB : List Exec → Bool
  | [] => true
  | e :: es => execOkB e && blockOkB es
/-- Decide `BranchesOk`. -/
def branchesOkB : List (Option Expr × List Exec) → Bool
  | [] => true
  | (_, b) :: bs => blockOkB b && branchesOkB bs
end

mutual
-- @tg: implements:: [[openspec:formal-model#IR Model In Lean]]
theorem execOkB_iff : ∀ e : Exec, execOkB e = true ↔ ExecOk e
  | .mk k _ => by simp only [execOkB, ExecOk]; exact execKindOkB_iff k
theorem execKindOkB_iff : ∀ k : ExecKind, execKindOkB k = true ↔ ExecKindOk k
  | .send s => by simp [execKindOkB, ExecKindOk]
  | .cancel a b => by simp [execKindOkB, ExecKindOk]
  | .assign _ e c => by
    cases e <;> cases c <;> simp [execKindOkB, ExecKindOk]
  | .if_ bs => by
    have := branchesOkB_iff bs
    cases h : bs.head? <;> simp_all [execKindOkB, ExecKindOk, and_assoc]
  | .foreach _ item _ body => by
    have := blockOkB_iff body
    simp_all [execKindOkB, ExecKindOk]
  | .raise _ => by simp [execKindOkB, ExecKindOk]
  | .log _ _ => by simp [execKindOkB, ExecKindOk]
  | .script _ => by simp [execKindOkB, ExecKindOk]
  | .extension _ => by simp [execKindOkB, ExecKindOk]
theorem blockOkB_iff : ∀ b : List Exec, blockOkB b = true ↔ BlockOk b
  | [] => by simp [blockOkB, BlockOk]
  | e :: es => by
    have h1 := execOkB_iff e
    have h2 := blockOkB_iff es
    simp only [blockOkB, BlockOk, Bool.and_eq_true]
    rw [h1, h2]
theorem branchesOkB_iff : ∀ bs : List (Option Expr × List Exec), branchesOkB bs = true ↔ BranchesOk bs
  | [] => by simp [branchesOkB, BranchesOk]
  | (_, b) :: bs => by
    have h1 := blockOkB_iff b
    have h2 := branchesOkB_iff bs
    simp only [branchesOkB, BranchesOk, Bool.and_eq_true]
    rw [h1, h2]
end

instance (b : List Exec) : Decidable (BlockOk b) := decidable_of_iff _ (blockOkB_iff b)

/-- The lowest common proper ancestor of two targets is a parallel state. -/
def LcaParallel (c : Chart) (x y : State) : Prop :=
  match (c.ancestors x).find? (fun p => decide (y.id ∈ p.descIds)) with
  | some p => p.kind = .parallel
  | none => False

instance (c : Chart) (x y : State) : Decidable (LcaParallel c x y) := by
  unfold LcaParallel
  split <;> infer_instance

/-- Two targets of one transition may be entered together. -/
def PairOk (c : Chart) (x y : State) : Prop :=
  x.id ≠ y.id ∧ x.id ∉ y.descIds ∧ y.id ∉ x.descIds ∧ LcaParallel c x y

/-- The position a target takes in the target-set rule: the state itself, or for a history
state its parent, whose content it stands for (a history state at the root is reported on its
own and skipped here). -/
def targetPosition (c : Chart) (t : String) : Option State :=
  match c.find t with
  | some s => if s.isHistory then c.parentOf s else some s
  | none => none

/-- `IllegalTargetSet`: the existing targets, history targets at their parent's position, are
pairwise compatible. -/
def TargetSetOk (c : Chart) (ts : List String) : Prop :=
  (ts.filterMap (targetPosition c)).Pairwise (PairOk c)

instance (c : Chart) (ts : List String) : Decidable (TargetSetOk c ts) := by
  unfold TargetSetOk PairOk; infer_instance

/-- The rules of one transition: targets exist, form a legal set, content is well-formed. -/
def TransitionOk (c : Chart) (t : Transition) : Prop :=
  (∀ x ∈ t.targets, x ∈ c.ids) ∧ TargetSetOk c t.targets ∧ BlockOk t.body

/-- The rules of an `initial` attribute or `<initial>` target list on state `s`. -/
def InitialTargetsOk (c : Chart) (s : State) (ts : List String) : Prop :=
  (∀ x ∈ ts, x ∈ c.ids ∧ x ∈ s.descIds) ∧ TargetSetOk c ts

/-- History rules for history state `s` with parent `p`. -/
def HistoryOk (c : Chart) (p : Option State) (s : State) : Prop :=
  s.transitions.length ≤ 1 ∧
  (∀ t, s.transitions.head? = some t →
    t.events = [] ∧ t.cond = none ∧ t.targets ≠ [] ∧
    ∀ q, p = some q → ∀ x ∈ t.targets, x ∈ c.ids → x ∈ q.descIds) ∧
  s.children.isEmpty = true ∧ s.onentry = [] ∧ s.onexit = [] ∧ s.invokes.isEmpty = true ∧
  s.data = []

/-- `<invoke>` attribute rules. -/
def InvokeOk (v : Invoke) : Prop :=
  ¬(v.type.isSome ∧ v.typeexpr.isSome) ∧ ¬(v.src.isSome ∧ v.srcexpr.isSome) ∧
  ¬(v.id.isSome ∧ v.idlocation.isSome) ∧
  ¬(v.content.isSome ∧
    (v.srcexpr.isSome ∨ (v.src.isSome ∧ (v.content.bind (·.expr)).isSome))) ∧
  ¬(v.params ≠ [] ∧ v.namelist ≠ []) ∧ ParamsOk v.params ∧ BlockOk v.finalize

/-- Every rule about one state `s` with parent `p`. -/
def StateOk (c : Chart) (p : Option State) (s : State) : Prop :=
  -- initial attribute and element
  ((s.initial ≠ [] ∨ s.initialElement.isSome) → s.kind = .compound) ∧
  ¬(s.initial ≠ [] ∧ s.initialElement.isSome) ∧
  InitialTargetsOk c s s.initial ∧
  (∀ i, s.initialElement = some i →
    i.transition.events = [] ∧ i.transition.cond = none ∧ i.transition.targets ≠ [] ∧
    InitialTargetsOk c s i.transition.targets ∧ BlockOk i.transition.body) ∧
  -- transitions
  (∀ t ∈ s.transitions, TransitionOk c t) ∧
  -- kind-specific rules
  (s.isHistory = true → HistoryOk c p s) ∧
  (s.element = .final → s.children.isEmpty = true ∧ s.transitions.isEmpty = true ∧
    s.invokes.isEmpty = true ∧ s.data = []) ∧
  ((s.kind = .parallel ∨ s.kind = .compound) → s.children ≠ [] →
    s.children.any (fun c => !c.isHistory) = true) ∧
  -- donedata
  (s.donedata.isSome → s.element = .final) ∧
  (∀ dd, s.donedata = some dd → ¬(dd.content.isSome ∧ dd.params ≠ []) ∧ ParamsOk dd.params) ∧
  -- executable content and invokes
  (∀ b ∈ s.onentry ++ s.onexit, BlockOk b) ∧
  (∀ v ∈ s.invokes, InvokeOk v)

/-- Rust `resolve_initial` targets of a state: for a compound state its `initial` attribute,
else its `<initial>` transition's targets, else its first non-history child; nothing otherwise. -/
def initialTargets (s : State) : List String :=
  if s.kind != .compound then []
  else if !s.initial.isEmpty then s.initial
  else match s.initialElement with
    | some i => i.transition.targets
    | none => match s.children.find? (fun c => !c.isHistory) with
      | some c => [c.id]
      | none => []

/-- The history states the default entry of history state `h` targets (Rust
`history_default_edges`): through its default transition, or without one through its parent's
initial targets. -/
def historyDefaultEdges (c : Chart) (h : State) : List State :=
  let targets := match h.transitions.head?, c.parentOf h with
    | some t, _ => t.targets
    | none, some p => initialTargets p
    | none, none => []
  (targets.filterMap c.find).filter (·.isHistory)

/-- The states reachable from `frontier` along `historyDefaultEdges`, in at most `n` rounds. -/
def historyReach (c : Chart) : Nat → List State → List State
  | 0, _ => []
  | n + 1, frontier =>
    frontier ++ historyReach c n (frontier.flatMap (historyDefaultEdges c))

/-- `HistoryDefaultCycle`: default history entry never leads back to the history state it
started from (paths of every length up to the number of states are followed). -/
def HistoryDefaultsAcyclic (c : Chart) : Prop :=
  ∀ h ∈ c.allStates, h.isHistory = true →
    h.id ∉ (historyReach c c.allStates.length (historyDefaultEdges c h)).map State.id

instance (c : Chart) : Decidable (HistoryDefaultsAcyclic c) := by
  unfold HistoryDefaultsAcyclic; infer_instance

/-- `<data>`: not both `expr` and a source (`src`, inline content, or both: a resolved `src`
keeps the reference and the content it resolved to). -/
def DataOk (d : Data) : Prop := ([d.src.isSome || d.content.isSome, d.expr.isSome].count true) ≤ 1

/-- The chart model's well-formedness rules (Rust `validate::validate`) as a proposition. -/
-- @tg: implements:: [[openspec:formal-model#IR Model In Lean]]
def WellFormed (c : Chart) : Prop :=
  c.states ≠ [] ∧
  (∀ s ∈ c.allStates, s.id ≠ "") ∧
  c.ids.Nodup ∧
  (∀ x ∈ c.initial, x ∈ c.ids) ∧ TargetSetOk c c.initial ∧
  (∀ s ∈ c.states, s.isHistory = false) ∧
  (c.allData.map (·.id)).Nodup ∧ (∀ d ∈ c.allData, DataOk d) ∧
  (∀ p ∈ c.parented, StateOk c p.1 p.2) ∧
  HistoryDefaultsAcyclic c

/-! ## The decision procedure -/

instance (v : Invoke) : Decidable (InvokeOk v) := by unfold InvokeOk; infer_instance
instance (d : Data) : Decidable (DataOk d) := by unfold DataOk; infer_instance
instance (c : Chart) (t : Transition) : Decidable (TransitionOk c t) := by
  unfold TransitionOk; infer_instance
instance (c : Chart) (s : State) (ts : List String) : Decidable (InitialTargetsOk c s ts) := by
  unfold InitialTargetsOk; infer_instance
instance (c : Chart) (p : Option State) (s : State) : Decidable (HistoryOk c p s) := by
  unfold HistoryOk; infer_instance

instance (c : Chart) (p : Option State) (s : State) : Decidable (StateOk c p s) := by
  unfold StateOk
  -- `Option Initial`, `Option DoneData` and `StateElement` equalities
  infer_instance

/-- `WellFormed` is decidable: the instance is the decision procedure. -/
instance instDecidableWellFormed (c : Chart) : Decidable (WellFormed c) := by
  unfold WellFormed; infer_instance

/-- The decision procedure for `WellFormed`. -/
-- @tg: implements:: [[openspec:formal-model#IR Model In Lean]]
def checkWF (c : Chart) : Bool := decide (WellFormed c)

/-- The decision procedure is correct: it returns true exactly when the chart is well-formed. -/
-- @tg: implements:: [[openspec:formal-model#IR Model In Lean]]
theorem checkWF_iff (c : Chart) : checkWF c = true ↔ WellFormed c := by
  unfold checkWF
  exact decide_eq_true_iff

end Vertep

namespace Vertep

/-! ## Named violations (for `vt certify`) -/

/-- Every rule group of `WellFormed`, its verdict, and the states it names when it fails. -/
def ruleChecks (c : Chart) : List (String × Bool) :=
  let bad := fun (p : State → Prop) [DecidablePred p] =>
    String.intercalate ", " ((c.allStates.filter (fun s => !decide (p s))).map State.id)
  [ ("EmptyChart", decide (c.states ≠ [])),
    ("MissingId", decide (∀ s ∈ c.allStates, s.id ≠ "")),
    ("DuplicateStateId", decide c.ids.Nodup),
    ("UnknownTarget (root initial)", decide (∀ x ∈ c.initial, x ∈ c.ids)),
    ("IllegalTargetSet (root initial)", decide (TargetSetOk c c.initial)),
    ("HistoryAtRoot", decide (∀ s ∈ c.states, s.isHistory = false)),
    ("DuplicateDataId", decide (c.allData.map (·.id)).Nodup),
    ("ConflictingAttributes (data)", decide (∀ d ∈ c.allData, DataOk d)),
    ("state rules (" ++ bad (fun s => StateOk c (c.parentOf s) s) ++ ")",
      decide (∀ p ∈ c.parented, StateOk c p.1 p.2)),
    ("HistoryDefaultCycle", decide (HistoryDefaultsAcyclic c)) ]

/-- The violated rule groups, by name. -/
def violations (c : Chart) : List String := ((ruleChecks c).filter (fun r => !r.2)).map (·.1)

/-- No violation is reported exactly when the chart is well-formed. -/
-- @tg: implements:: [[openspec:formal-model#Chart Certificates]]
theorem violations_eq_nil_iff (c : Chart) : violations c = [] ↔ WellFormed c := by
  unfold violations ruleChecks WellFormed
  simp only [List.map_eq_nil_iff, List.filter_eq_nil_iff, List.mem_cons, List.not_mem_nil,
    or_false, forall_eq_or_imp, forall_eq, Bool.not_eq_true', decide_eq_false_iff_not,
    Decidable.not_not]

end Vertep
