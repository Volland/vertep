/-!
# The chart IR

A Lean mirror of `crates/vertep-core/src/ir.rs`. Every Rust type has a Lean counterpart with the
same fields in the same order, except source spans: Rust spans compare equal to every other
span, so dropping them keeps Lean equality equal to Rust's structural chart equality.

Expression text stays uninterpreted (`Expr := String`); it is evaluated only by the datamodel,
which the step semantics abstracts as an answer tape (see `VertepLean.Tape`).
-/

namespace Vertep

/-- Uninterpreted expression text, evaluated by the chart's datamodel. -/
abbrev Expr := String

/-- The Vertep extension namespace. -/
def vtNs : String := "https://w3id.org/vertep/vt#"
/-- The SCXML namespace. -/
def scxmlNs : String := "http://www.w3.org/2005/07/scxml"

/-- The content of an extension node (Rust `ExtensionValue`). -/
inductive ExtensionValue where
  /-- An attribute and its value. -/
  | attribute (value : String)
  /-- An element, kept as its serialized XML. -/
  | element (xml : String)
  deriving Repr, BEq, DecidableEq, Inhabited

/-- An attribute or element from the `vt:` namespace or a foreign namespace (Rust `Extension`). -/
structure Extension where
  ns : String
  localName : String
  value : ExtensionValue
  /-- For elements: index among the owner's element children; for attributes: 0. -/
  position : Nat
  deriving Repr, BEq, DecidableEq, Inhabited

/-- Binding mode for data items (W3C SCXML 5.3). -/
inductive Binding where
  | early
  | late
  deriving Repr, BEq, DecidableEq, Inhabited

/-- History depth. -/
inductive HistoryType where
  | shallow
  | deep
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The element a state was written as (Rust `StateElement`). -/
inductive StateElement where
  | state
  | parallel
  | final
  | history (type : HistoryType)
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The classification of a state (W3C SCXML 3.1, Rust `StateKind`). -/
inductive StateKind where
  | atomic
  | compound
  | parallel
  | final
  | history (type : HistoryType)
  deriving Repr, BEq, DecidableEq, Inhabited

/-- Transition type (W3C SCXML 3.13). -/
inductive TransitionType where
  | external
  | internal
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<param>`. -/
structure Param where
  name : String
  expr : Option Expr := none
  location : Option Expr := none
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<content>`. -/
structure Content where
  expr : Option Expr := none
  body : Option String := none
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<script>`. -/
structure Script where
  src : Option String := none
  body : Option String := none
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<send>` (W3C SCXML 6.2). -/
structure Send where
  event : Option String := none
  eventexpr : Option Expr := none
  target : Option String := none
  targetexpr : Option Expr := none
  type : Option String := none
  typeexpr : Option Expr := none
  id : Option String := none
  idlocation : Option Expr := none
  delay : Option String := none
  delayexpr : Option Expr := none
  namelist : List String := []
  params : List Param := []
  content : Option Content := none
  deriving Repr, BEq, DecidableEq, Inhabited

mutual
/-- One executable content element with its extension attributes (Rust `Exec`). -/
inductive Exec where
  | mk (kind : ExecKind) (extensions : List Extension)
/-- The executable content elements of SCXML (Rust `ExecKind`). An `if` holds its branches as
`(condition, body)` pairs in document order; the `else` branch, if present, is last and has no
condition. -/
inductive ExecKind where
  | raise (event : String)
  | send (send : Send)
  | cancel (sendid : Option String) (sendidexpr : Option Expr)
  | log (label : Option String) (expr : Option Expr)
  | assign (location : Expr) (expr : Option Expr) (content : Option String)
  | script (script : Script)
  | if_ (branches : List (Option Expr × List Exec))
  | foreach (array : Expr) (item : String) (index : Option String) (body : List Exec)
  | extension (ext : Extension)
end

deriving instance Repr for Exec, ExecKind
deriving instance BEq for Exec, ExecKind

instance : Inhabited ExecKind := ⟨.raise ""⟩
instance : Inhabited Exec := ⟨.mk default []⟩

/-- An ordered block of executable content. -/
abbrev Block := List Exec

/-- An element without extensions. -/
def Exec.ofKind (k : ExecKind) : Exec := .mk k []

/-- The element of an executable content node. -/
def Exec.kind : Exec → ExecKind
  | .mk k _ => k

/-- A transition (Rust `Transition`). -/
structure Transition where
  events : List String := []
  cond : Option Expr := none
  targets : List String := []
  type : TransitionType := .external
  typeExplicit : Bool := false
  body : Block := []
  extensions : List Extension := []
  deriving Repr, BEq, Inhabited

/-- The `<initial>` pseudo-state of a compound state. -/
structure Initial where
  transition : Transition
  extensions : List Extension := []
  deriving Repr, BEq, Inhabited

/-- A `<data>` item. -/
structure Data where
  id : String
  src : Option String := none
  expr : Option Expr := none
  content : Option String := none
  extensions : List Extension := []
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<donedata>`. -/
structure DoneData where
  content : Option Content := none
  params : List Param := []
  deriving Repr, BEq, DecidableEq, Inhabited

/-- `<invoke>` (W3C SCXML 6.4). -/
structure Invoke where
  type : Option String := none
  typeexpr : Option Expr := none
  src : Option String := none
  srcexpr : Option Expr := none
  id : Option String := none
  idlocation : Option Expr := none
  namelist : List String := []
  autoforward : Bool := false
  params : List Param := []
  content : Option Content := none
  finalize : Block := []
  extensions : List Extension := []
  deriving Repr, BEq, Inhabited

/-- A state node of any kind (Rust `State`). -/
inductive State where
  | mk (id : String) (idGenerated : Bool) (element : StateElement) (initial : List String)
      (initialElement : Option Initial) (children : List State) (transitions : List Transition)
      (onentry : List Block) (onexit : List Block) (data : List Data) (invokes : List Invoke)
      (donedata : Option DoneData) (extensions : List Extension)
  deriving Repr, BEq

instance : Inhabited State :=
  ⟨.mk "" false .state [] none [] [] [] [] [] [] none []⟩

namespace State
def id : State → String | .mk i .. => i
def idGenerated : State → Bool | .mk _ g .. => g
def element : State → StateElement | .mk _ _ e .. => e
def initial : State → List String | .mk _ _ _ i .. => i
def initialElement : State → Option Initial | .mk _ _ _ _ i .. => i
def children : State → List State | .mk _ _ _ _ _ c .. => c
def transitions : State → List Transition | .mk _ _ _ _ _ _ t .. => t
def onentry : State → List Block | .mk _ _ _ _ _ _ _ b .. => b
def onexit : State → List Block | .mk _ _ _ _ _ _ _ _ b .. => b
def data : State → List Data | .mk _ _ _ _ _ _ _ _ _ d .. => d
def invokes : State → List Invoke | .mk _ _ _ _ _ _ _ _ _ _ i .. => i
def donedata : State → Option DoneData | .mk _ _ _ _ _ _ _ _ _ _ _ d _ => d
def extensions : State → List Extension | .mk _ _ _ _ _ _ _ _ _ _ _ _ e => e

/-- A state with the given identifier and element and nothing else (Rust `State::new`). -/
def new (id : String) (element : StateElement) : State :=
  .mk id false element [] none [] [] [] [] [] [] none []

/-- The state's kind, derived from its element and children (Rust `State::kind`): a `state`
with any children is compound. (A `state` whose only children are history states is rejected by
`WellFormed`, so the W3C reading, which would call it atomic, never has to be chosen.) -/
def kind (s : State) : StateKind :=
  match s.element with
  | .state => if s.children.isEmpty then .atomic else .compound
  | .parallel => .parallel
  | .final => .final
  | .history h => .history h
end State

/-- The root of a chart: the `<scxml>` element (Rust `Chart`). -/
structure Chart where
  name : Option String := none
  datamodel : String := "null"
  datamodelExplicit : Bool := false
  binding : Binding := .early
  bindingExplicit : Bool := false
  initial : List String := []
  data : List Data := []
  script : Option Script := none
  states : List State := []
  extensions : List Extension := []
  deriving Repr, BEq, Inhabited

end Vertep
