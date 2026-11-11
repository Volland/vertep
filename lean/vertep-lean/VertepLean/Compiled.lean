import VertepLean.WellFormed
import VertepLean.Tape

/-!
# The compiled chart

Design D2: validation turns a well-formed IR into a compiled chart with dense state indices in
document order. Index 0 is the `<scxml>` root; the states follow in preorder, so a parent always
has a smaller index than its children, entry order is ascending index and exit order descending.
Transitions get dense indices too; the `<initial>` transitions of compound states (synthesized
from the `initial` attribute or the first child when no `<initial>` element exists) and the
root's initial transition are ordinary entries that are never selected.

This file also holds the pure tree queries of Appendix D (`getProperAncestors`, `isDescendant`,
`findLCCA`, `getTransitionDomain`, `getEffectiveTargetStates`, `computeExitSet` and the entry-set
computation), shared by the step and the theorems.
-/

namespace Vertep

/-- The kind of a compiled state. `root` is the `<scxml>` element. -/
inductive Kind where
  | root
  | atomic
  | compound
  | parallel
  | final
  | history (type : HistoryType)
  deriving Repr, BEq, DecidableEq, Inhabited

/-- A compiled state. -/
structure CState where
  id : String
  kind : Kind
  /-- The parent's index; `none` only for the root. -/
  parent : Option Nat
  /-- Child states written as `state`, `parallel` or `final`, in document order. -/
  children : List Nat
  /-- History children, in document order. -/
  histories : List Nat
  /-- Outgoing transitions in document order; for a history state, its default transition. -/
  transitions : List Nat
  /-- The initial transition of a compound state or of the root. -/
  initial : Option Nat
  onentry : List Block
  onexit : List Block
  data : List Data
  invokes : List Invoke
  donedata : Option DoneData
  deriving Repr, Inhabited

/-- The state outside the chart: no children, no content. -/
def CState.empty : CState :=
  { id := "", kind := .atomic, parent := none, children := [], histories := [],
    transitions := [], initial := none, onentry := [], onexit := [], data := [], invokes := [],
    donedata := none }

/-- A compiled transition (Rust `CTransition`). `index` is the position in the source state's
IR transitions (`none` for synthesized default-entry and history default transitions);
`defaultEntry` marks the default-entry transitions of compound states and the root, whose
domain is their source. -/
structure CTrans where
  source : Nat
  index : Option Nat
  events : List String
  cond : Option Expr
  targets : List Nat
  internal : Bool
  body : Block
  defaultEntry : Bool
  deriving Repr, Inhabited

/-- The transition outside the chart. -/
def CTrans.empty : CTrans :=
  { source := 0, index := none, events := [], cond := none, targets := [], internal := false,
    body := [], defaultEntry := false }

/-- A compiled chart. -/
structure CChart where
  name : Option String
  datamodel : String
  binding : Binding
  script : Option Script
  states : Array CState
  trans : Array CTrans
  deriving Repr, Inhabited

/-! ## Compilation -/

namespace Compile

/-- The index of the first state with identifier `x` (root is 0, so states start at 1). -/
def idx (ids : List String) (x : String) : Option Nat := (ids.idxOf? x).map (· + 1)

/-- Resolve target identifiers, dropping unknown ones (Rust `filter_map` over `by_id`). -/
def resolve (ids : List String) (ts : List String) : List Nat := ts.filterMap (idx ids)

/-- The compiled kind of an IR state (Rust `State::kind`). -/
def kindOf (s : State) : Kind :=
  match s.kind with
  | .atomic => .atomic
  | .compound => .compound
  | .parallel => .parallel
  | .final => .final
  | .history h => .history h

/-- The first child that is not a history state (Rust `first_child`). -/
def firstChild (cs : List State) : List String :=
  match cs.find? (fun c => !c.isHistory) with
  | some c => [c.id]
  | none => []

/-- Rust `resolve_initial` for a compound state: targets and default-entry content. -/
def resolveInitial (s : State) : List String × Block :=
  if !s.initial.isEmpty then (s.initial, [])
  else match s.initialElement with
    | some ie => (ie.transition.targets, ie.transition.body)
    | none => (firstChild s.children, [])

/-- Compile the states (without transitions) in document order, root first. -/
def states (c : Chart) : List CState :=
  let ids := c.ids
  let root : CState :=
    { id := "", kind := .root, parent := none,
      children := ((c.states.filter (fun s => !s.isHistory)).map (·.id)).filterMap (idx ids),
      histories := ((c.states.filter (fun s => s.isHistory)).map (·.id)).filterMap (idx ids),
      transitions := [], initial := none, onentry := [], onexit := [], data := c.data,
      invokes := [], donedata := none }
  root :: c.parented.map (fun (p, s) =>
    { id := s.id, kind := kindOf s,
      parent := some (match p with
        | some ps => (idx ids ps.id).getD 0
        | none => 0),
      children := ((s.children.filter (fun c => !c.isHistory)).map (·.id)).filterMap (idx ids),
      histories := ((s.children.filter (fun c => c.isHistory)).map (·.id)).filterMap (idx ids),
      transitions := [], initial := none, onentry := s.onentry, onexit := s.onexit,
      data := s.data, invokes := s.invokes, donedata := s.donedata })

end Compile

/-- Compile a well-formed chart (Rust `CompiledChart::new`): states in document order with the
root at 0; transitions as the root's default entry, then per state its default entry (compound
states) and its own transitions; then a synthesized default transition for every history state
without one (its parent's default entry targets, or all regions of a parallel parent). -/
def compile (c : Chart) : Except String CChart := do
  if !checkWF c then throw "chart is not well-formed"
  let ids := c.ids
  let sts0 := Compile.states c
  let mut sts : Array CState := sts0.toArray
  let rootInit : CTrans :=
    { source := 0, index := none, events := [], cond := none,
      targets := Compile.resolve ids (if c.initial.isEmpty then Compile.firstChild c.states
        else c.initial),
      internal := true, body := [], defaultEntry := true }
  let mut ts : Array CTrans := #[rootInit]
  sts := sts.modify 0 (fun r => { r with initial := some 0 })
  let mut i := 1
  for (_, s) in c.parented do
    if s.kind == .compound then
      let (tg, body) := Compile.resolveInitial s
      let t := ts.size
      ts := ts.push { source := i, index := none, events := [], cond := none,
                      targets := Compile.resolve ids tg, internal := true, body,
                      defaultEntry := true }
      sts := sts.modify i (fun x => { x with initial := some t })
    let mut k := 0
    for tr in s.transitions do
      let t := ts.size
      ts := ts.push { source := i, index := some k, events := tr.events, cond := tr.cond,
                      targets := Compile.resolve ids tr.targets,
                      internal := tr.type == .internal, body := tr.body, defaultEntry := false }
      sts := sts.modify i (fun x => { x with transitions := x.transitions ++ [t] })
      k := k + 1
    i := i + 1
  -- history states without a default transition
  for h in List.range sts.size do
    let st := sts.getD h CState.empty
    match st.kind with
    | .history _ =>
      if st.transitions.isEmpty then
        let p := st.parent.getD 0
        let ps := sts.getD p CState.empty
        let targets := match ps.initial with
          | some ti => (ts.getD ti CTrans.empty).targets
          | none => ps.children
        let t := ts.size
        ts := ts.push { source := h, index := none, events := [], cond := none, targets,
                        internal := false, body := [], defaultEntry := false }
        sts := sts.modify h (fun x => { x with transitions := [t] })
    | _ => pure ()
  pure { name := c.name, datamodel := (if c.datamodel.isEmpty then "null" else c.datamodel),
         binding := c.binding, script := c.script, states := sts, trans := ts }

/-! ## Tree queries -/

namespace CChart

variable (cc : CChart)

/-- The state at index `i` (a default state outside the chart). -/
def st (i : Nat) : CState := cc.states.getD i CState.empty
/-- The transition at index `t`. -/
def tr (t : Nat) : CTrans := cc.trans.getD t CTrans.empty
/-- Number of states including the root. -/
def size : Nat := cc.states.size

/-- Proper ancestors of `i`, nearest first, ending at the root (Appendix D
`getProperAncestors(i, null)`). Parents always have smaller indices; a malformed parent link
ends the chain. -/
def ancestors (i : Nat) : List Nat :=
  match (cc.st i).parent with
  | some p => if p < i then p :: ancestors p else []
  | none => []
termination_by i

/-- `isDescendant(a, b)`: `a` is a proper descendant of `b`. -/
def isDesc (a b : Nat) : Bool := (cc.ancestors a).contains b

/-- Proper ancestors of `i` strictly below `stop` (all of them when `stop` is `none`). -/
def ancestorsBelow (i : Nat) (stop : Option Nat) : List Nat :=
  match stop with
  | none => cc.ancestors i
  | some a => (cc.ancestors i).takeWhile (· != a)

def isCompound (i : Nat) : Bool := (cc.st i).kind == .compound
def isParallel (i : Nat) : Bool := (cc.st i).kind == .parallel
def isFinal (i : Nat) : Bool := (cc.st i).kind == .final
def isHistory (i : Nat) : Bool :=
  match (cc.st i).kind with
  | .history _ => true
  | _ => false
def isDeepHistory (i : Nat) : Bool := (cc.st i).kind == .history .deep
def isCompoundOrRoot (i : Nat) : Bool :=
  (cc.st i).kind == .compound || (cc.st i).kind == .root

/-- Appendix D `isAtomicState` (Rust `is_atomic`): not the root, no child states, not a
history state. -/
def isAtomic (i : Nat) : Bool :=
  i != 0 && (cc.st i).children.isEmpty && !cc.isHistory i

/-- Index of the state with identifier `x`. -/
def indexOf (x : String) : Option Nat := cc.states.toList.findIdx? (fun s => s.id == x)

/-- The transitions considered for atomic state `a`: those of `a`, then of its parent and so
on, each in document order. -/
def candidates (a : Nat) : List Nat := (a :: cc.ancestors a).flatMap (fun s => (cc.st s).transitions)

/-- The default transition of a history state. -/
def historyDefault (h : Nat) : Option Nat := (cc.st h).transitions.head?

/-- Append unless present (`OrderedSet.add`). -/
def pushUnique (acc : List Nat) (x : Nat) : List Nat := if acc.contains x then acc else acc ++ [x]

/-- The recorded value of history state `h`, if it was ever recorded. -/
def histValue (hist : List (Nat × List Nat)) (h : Nat) : Option (List Nat) := hist.lookup h

/-- Appendix D `getEffectiveTargetStates` (Rust `effective_targets`), with recursion fuel for
history defaults that target history states. -/
def effTargetsF (hist : List (Nat × List Nat)) : Nat → Nat → List Nat
  | 0, _ => []
  | n + 1, t =>
    (cc.tr t).targets.foldl (fun acc s =>
      if cc.isHistory s then
        match histValue hist s with
        | some v => v.foldl pushUnique acc
        | none => match cc.historyDefault s with
          | some d => (effTargetsF hist n d).foldl pushUnique acc
          | none => acc
      else pushUnique acc s) []

/-- Appendix D `getEffectiveTargetStates`. -/
def effectiveTargets (hist : List (Nat × List Nat)) (t : Nat) : List Nat :=
  cc.effTargetsF hist (cc.size + 1) t

/-- Appendix D `findLCCA`: the nearest proper ancestor of the head that is compound (or the
root) and has every other state as a descendant; the root when none is found. -/
def findLCCA (head : Nat) (rest : List Nat) : Nat :=
  ((cc.ancestors head).find? (fun a => cc.isCompoundOrRoot a && rest.all (fun s => cc.isDesc s a))).getD 0

/-- Appendix D `getTransitionDomain` (Rust `transition_domain`). -/
def domain (hist : List (Nat × List Nat)) (t : Nat) : Option Nat :=
  let tr := cc.tr t
  if tr.defaultEntry then some tr.source
  else if tr.targets.isEmpty then none
  else
    let ts := cc.effectiveTargets hist t
    if ts.isEmpty then none
    else if tr.internal && cc.isCompound tr.source && ts.all (fun s => cc.isDesc s tr.source)
    then some tr.source
    else some (cc.findLCCA tr.source ts)

/-- Appendix D `computeExitSet` for one transition, in document order. -/
def exitSetOf (config : List Nat) (hist : List (Nat × List Nat)) (t : Nat) : List Nat :=
  if (cc.tr t).targets.isEmpty then []
  else match cc.domain hist t with
    | some d => config.filter (fun s => cc.isDesc s d)
    | none => []

/-- Appendix D `computeExitSet` for a set of transitions, in document order. -/
def exitSet (config : List Nat) (hist : List (Nat × List Nat)) (ts : List Nat) : List Nat :=
  config.filter (fun s => ts.any (fun t => (cc.exitSetOf config hist t).contains s))

/-- The conflict test of `removeConflictingTransitions`: the exit sets intersect. -/
def conflict (config : List Nat) (hist : List (Nat × List Nat)) (t₁ t₂ : Nat) : Bool :=
  (cc.exitSetOf config hist t₁).any (fun s => (cc.exitSetOf config hist t₂).contains s)

/-- The preemption test of `removeConflictingTransitions`: the source of `t₁` is a proper
descendant of the source of `t₂`. -/
def srcDesc (t₁ t₂ : Nat) : Bool := cc.isDesc (cc.tr t₁).source (cc.tr t₂).source

/-- Appendix D `isInFinalState` (Rust `is_in_final_state`), with recursion fuel. -/
def inFinalF (config : List Nat) : Nat → Nat → Bool
  | 0, _ => false
  | n + 1, s =>
    if cc.isCompound s then
      (cc.st s).children.any (fun c => cc.isFinal c && config.contains c)
    else if cc.isParallel s then
      (cc.st s).children.all (fun c => inFinalF config n c)
    else false

/-- Appendix D `isInFinalState`. -/
def inFinal (config : List Nat) (s : Nat) : Bool := cc.inFinalF config (cc.size + 1) s

/-! ## Entry set (Appendix D `computeEntrySet`) -/

/-- The accumulators of `computeEntrySet` (Rust `EntryPlan`). -/
structure EntryAcc where
  toEnter : List Nat := []
  defaultEntry : List Nat := []
  /-- `defaultHistoryContent`: parent state ↦ the history default transition. -/
  histContent : List (Nat × Nat) := []
  deriving Repr, Inhabited

def EntryAcc.add (acc : EntryAcc) (s : Nat) : EntryAcc :=
  if acc.toEnter.contains s then acc else { acc with toEnter := acc.toEnter ++ [s] }

mutual
/-- Appendix D `addDescendantStatesToEnter` (Rust `add_descendants`), with recursion fuel. -/
def addDesc (hist : List (Nat × List Nat)) : Nat → Nat → EntryAcc → Option EntryAcc
  | 0, _, _ => none
  | n + 1, s, acc =>
    if cc.isHistory s then
      let par := ((cc.st s).parent).getD 0
      match histValue hist s with
      | some v => do
        let acc ← v.foldlM (fun acc x => addDesc hist n x acc) acc
        v.foldlM (fun acc x => addAnc hist n x (some par) acc) acc
      | none =>
        match cc.historyDefault s with
        | some d => do
          let acc := { acc with histContent :=
            acc.histContent.filter (fun e => e.1 != par) ++ [(par, d)] }
          let tg := (cc.tr d).targets
          let acc ← tg.foldlM (fun acc x => addDesc hist n x acc) acc
          tg.foldlM (fun acc x => addAnc hist n x (some par) acc) acc
        | none => pure acc
    else
      let acc := acc.add s
      if cc.isCompound s then do
        let acc := if acc.defaultEntry.contains s then acc
          else { acc with defaultEntry := acc.defaultEntry ++ [s] }
        let tg := match (cc.st s).initial with
          | some i => (cc.tr i).targets
          | none => []
        let acc ← tg.foldlM (fun acc x => addDesc hist n x acc) acc
        tg.foldlM (fun acc x => addAnc hist n x (some s) acc) acc
      else if cc.isParallel s then
        (cc.st s).children.foldlM (fun acc c =>
          if acc.toEnter.any (fun x => cc.isDesc x c) then pure acc
          else addDesc hist n c acc) acc
      else pure acc
/-- Appendix D `addAncestorStatesToEnter` (Rust `add_ancestors`, which never adds the root),
with recursion fuel. -/
def addAnc (hist : List (Nat × List Nat)) : Nat → Nat → Option Nat → EntryAcc → Option EntryAcc
  | 0, _, _, _ => none
  | n + 1, s, stop, acc =>
    ((cc.ancestorsBelow s stop).filter (· != 0)).foldlM (fun acc anc =>
      let acc := acc.add anc
      if cc.isParallel anc then
        (cc.st anc).children.foldlM (fun acc c =>
          if acc.toEnter.any (fun x => cc.isDesc x c) then pure acc
          else addDesc hist n c acc) acc
      else pure acc) acc
end

/-- Fuel for the entry-set recursion; each nested call descends in the tree except a history
state's step to its recorded or default states, so this bound is never reached on well-formed
charts. -/
def entryFuel : Nat := 4 * cc.size + 4

/-- Appendix D `computeEntrySet` (Rust `compute_entry_set`). -/
def entrySet (hist : List (Nat × List Nat)) (ts : List Nat) : Option EntryAcc :=
  ts.foldlM (fun acc t => do
    let acc ← (cc.tr t).targets.foldlM (fun acc x => cc.addDesc hist cc.entryFuel x acc) acc
    let dom := cc.domain hist t
    (cc.effectiveTargets hist t).foldlM (fun acc s => cc.addAnc hist cc.entryFuel s dom acc) acc)
    {}

end CChart

/-! ## Event descriptor matching (W3C SCXML 3.12.1) -/

/-- Whitespace as Rust's `str::trim` sees it (ASCII subset). -/
def isWs (c : Char) : Bool := c == ' ' || c == '\t' || c == '\n' || c == '\r'

/-- Trim leading and trailing whitespace. -/
def trimWs (s : String) : String :=
  String.ofList (((s.toList.dropWhile isWs).reverse.dropWhile isWs).reverse)

/-- Drop the last `n` characters. -/
def dropLast (s : String) (n : Nat) : String := String.ofList (s.toList.take (s.length - n))

/-- One descriptor against an event name: `*` matches everything, a trailing `.*` or `.` is
ignored, and otherwise the descriptor must equal the name or be a prefix of it ending at a dot. -/
def descriptorMatches (d name : String) : Bool :=
  let d := trimWs d
  if d == "*" then true
  else
    let d := if d.endsWith ".*" then dropLast d 2 else d
    let d := if d.endsWith "." then dropLast d 1 else d
    if d.isEmpty then false
    else name == d || name.startsWith (d ++ ".")

/-- A transition's descriptors against an event name. -/
def nameMatch (events : List String) (name : String) : Bool :=
  events.any (fun d => descriptorMatches d name)

end Vertep
