import VertepLean.IR

/-!
# Values, events, effects and the datamodel answer tape

Mirrors `crates/vertep-core/src/datamodel.rs` (`Value`, `Event`, `EventType`, `SessionInfo`) and
defines the request/answer protocol that replaces the `Datamodel` trait in the Lean model
(design D5).

## The tape protocol

The Rust core, when tracing, wraps its datamodel in `tape::TapeDatamodel`. Every call the
interpreter makes on the datamodel (every method of the `Datamodel` trait except `name` and
`snapshot`) is one `Request`, and the recorder appends one `TapeEntry` (the request and its
`Answer`) per call, in call order; `set_environment` and `set_event` are recorded with the
answer `unit`. These types mirror `crates/vertep-core/src/tape.rs` constructor for constructor.

The Lean step consumes the tape front to back: each time it needs the datamodel it builds the
`Request` it would send, takes the next entry, and fails with `StepError.tapeMismatch` unless the
recorded request is equal to it, or with `StepError.tapeExhausted` if the tape is empty. An answer
of the wrong shape for its request (`Answer.wellShaped`) fails with `StepError.badAnswer`.

For the `null` datamodel (W3C SCXML B.1) the Lean model answers every request itself, exactly as
Rust's `NullDatamodel` does, and consumes no tape: `In(id)` conditions are evaluated against the
current configuration, `evalContent` gives the content as a string, `initSession`,
`setEnvironment` and `setEvent` succeed, `isLocation` is false and everything else is an
evaluation error. Requests answered natively are still logged, so Rust and Lean list the same
requests for every datamodel.
-/

namespace Vertep

/-- A datamodel-independent value (Rust `Value`). A number is kept as the exact decimal
`mantissa × 10^-exponent` it has in the JSON exchange format (serde writes the shortest
round-trip decimal of the `f64`), normalized by `Value.number`, so equality is exact and the
model never computes with floating point. -/
inductive Value where
  | undefined
  | null
  | bool (b : Bool)
  | num (mantissa : Int) (exponent : Nat)
  | str (s : String)
  | array (items : List Value)
  | object (fields : List (String × Value))
  deriving Repr, BEq

instance : Inhabited Value := ⟨.undefined⟩

/-- A normalized decimal number: trailing zeros of the mantissa are folded into the exponent. -/
def Value.number (m : Int) : Nat → Value
  | 0 => .num m 0
  | e + 1 => if m % 10 == 0 then Value.number (m / 10) e else .num m (e + 1)

/-- The number `n` as a value (Rust `Value::Number(n as f64)`). -/
def Value.ofNat (n : Nat) : Value := .num n 0

/-- Rust `value_to_string`: strings as they are, booleans as `true`/`false`, numbers in decimal
notation; anything else is an evaluation error. -/
def Value.toStr : Value → Option String
  | .str s => some s
  | .bool b => some (if b then "true" else "false")
  | .num m 0 => some (toString m)
  | .num m (e + 1) =>
    let a := m.natAbs
    let digits := toString a
    let padded := String.ofList (List.replicate (e + 1 + 1 - digits.length) '0') ++ digits
    let intLen := padded.length - (e + 1)
    let sign := if m < 0 then "-" else ""
    some (sign ++ String.ofList (padded.toList.take intLen) ++ "." ++
      String.ofList (padded.toList.drop intLen))
  | _ => none

/-- The `_event.type` field (Rust `EventType`). -/
inductive EventType where
  | platform
  | internal
  | external
  deriving Repr, BEq, DecidableEq, Inhabited

/-- An SCXML event (Rust `Event`). -/
structure Event where
  name : String
  type : EventType := .external
  sendid : Option String := none
  origin : Option String := none
  origintype : Option String := none
  invokeid : Option String := none
  data : Value := .undefined
  deriving Repr, BEq, Inhabited

/-- Values for the read-only session system variables (Rust `SessionInfo`). -/
structure SessionInfo where
  sessionid : String
  name : Option String := none
  ioprocessors : Value := .undefined
  deriving Repr, BEq, Inhabited

/-- Host-supplied sources of nondeterminism for one input (Rust `Environment`). -/
structure Environment where
  nowMs : Int := 0
  seed : Nat := 0
  deriving Repr, BEq, DecidableEq, Inhabited

/-- One datamodel operation (Rust `tape::Request`), one constructor per recorded method of the
`Datamodel` trait. -/
inductive Request where
  | initSession (info : SessionInfo)
  | setEnvironment (env : Environment)
  | declare (id : String)
  | setEvent (event : Event)
  | evalCond (expr : Expr)
  | evalValue (expr : Expr)
  | evalContent (content : String)
  | assignExpr (location : Expr) (expr : Expr)
  | assignValue (location : Expr) (value : Value)
  | bindVar (name : String) (value : Value)
  | isLocation (location : Expr)
  | execScript (script : String)
  | foreachItems (array : Expr)
  | restore (snapshot : Value)
  deriving Repr, BEq

instance : Inhabited Request := ⟨.evalCond ""⟩

/-- The location a request writes to, if it creates a data item (`declare`) or assigns
(`assignExpr`, `assignValue`, `bindVar`). -/
def Request.writes : Request → Option String
  | .declare id => some id
  | .assignExpr l _ => some l
  | .assignValue l _ => some l
  | .bindVar n _ => some n
  | _ => none

/-- The datamodel's answer to one request (Rust `tape::Answer`). -/
inductive Answer where
  | unit
  | bool (b : Bool)
  | value (v : Value)
  | values (vs : List Value)
  | error (message : String)
  deriving Repr, BEq, Inhabited

/-- Whether an answer has the shape its request returns. -/
def Answer.wellShaped : Request → Answer → Bool
  | .setEnvironment _, .unit => true
  | .setEvent _, .unit => true
  | .isLocation _, .bool _ => true
  | .setEnvironment _, _ => false
  | .setEvent _, _ => false
  | .isLocation _, _ => false
  | _, .error _ => true
  | .evalCond _, .bool _ => true
  | .evalValue _, .value _ => true
  | .evalContent _, .value _ => true
  | .foreachItems _, .values _ => true
  | .initSession _, .unit => true
  | .declare _, .unit => true
  | .assignExpr .., .unit => true
  | .assignValue .., .unit => true
  | .bindVar .., .unit => true
  | .execScript _, .unit => true
  | .restore _, .unit => true
  | _, _ => false

/-- One recorded datamodel call. -/
structure TapeEntry where
  request : Request
  answer : Answer
  deriving Repr, BEq, Inhabited

/-- The answer tape: the recorded calls in call order. -/
abbrev Tape := List TapeEntry

/-- A request to deliver an event (Rust `SendRequest`). -/
structure SendRequest where
  sendid : String
  event : Event
  target : Option String
  type : String
  delayMs : Nat
  deriving Repr, BEq

/-- A request to start an invocation (Rust `InvokeRequest`). -/
structure InvokeRequest where
  invokeid : String
  state : String
  type : String
  src : Option String
  content : Option Value
  params : Value
  autoforward : Bool
  deriving Repr, BEq

/-- An effect (Rust `session::Effect`): every interaction of a step with the outside world,
in the order the step produced it (design D3). -/
inductive Effect where
  | send (r : SendRequest)
  | cancel (sendid : String)
  | invoke (r : InvokeRequest)
  | cancelInvoke (invokeid : String)
  | forward (invokeid : String) (event : Event)
  | log (label : Option String) (value : Option Value)
  | sessionDone (data : Value) (cancelled : Bool)
  deriving Repr, BEq

/-- Why a step was refused or aborted. -/
inductive StepError where
  /-- The session already terminated (interpretation, *Session Termination*). -/
  | terminated
  /-- The microstep budget of one macrostep was exceeded (interpretation, *Macrostep
  Execution*). -/
  | budgetExceeded (budget : Nat)
  /-- The tape ended where the step needed an answer for `request`. -/
  | tapeExhausted (request : Request)
  /-- The step requested `requested` where the tape recorded `recorded`. -/
  | tapeMismatch (requested : Request) (recorded : Request)
  /-- The tape answered `request` with an answer of the wrong shape. -/
  | badAnswer (request : Request) (answer : Answer)
  /-- `init_session` failed (Rust `StartError::Datamodel`). -/
  | initFailed (message : String)
  /-- The chart is not well-formed in a way the step cannot interpret (never raised on charts
  compiled from well-formed IR). -/
  | illFormed (reason : String)
  deriving Repr, BEq

/-- What an input asks the session to do (Rust `InputKind`). -/
inductive InputKind where
  /-- Process an external event. -/
  | event (e : Event)
  /-- Cancel the session. -/
  | cancel
  deriving Repr, BEq

/-- One input to `step` (Rust `Input`). -/
structure Input where
  env : Environment := {}
  kind : InputKind
  /-- The session identifiers the SCXML Event I/O Processor can deliver to, when the host
  knows them (Rust `Input::sessions`). -/
  sessions : Option (List String) := none
  deriving Repr, BEq

/-- Interpreter options fixed at session start (Rust `Options`). -/
structure Options where
  microstepBudget : Nat := 10000
  sendTypes : List String := ["http://www.w3.org/TR/scxml/#SCXMLEventProcessor", "scxml"]
  invokeTypes : List String :=
    ["http://www.w3.org/TR/scxml/", "http://www.w3.org/TR/scxml", "scxml"]
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The input that starts a session (Rust `StartInput`). -/
structure StartInput where
  info : SessionInfo
  env : Environment := {}
  options : Options := {}
  /-- Values for top-level data items supplied by the invoking session (W3C SCXML 6.4.4). -/
  data : List (String × Value) := []
  /-- The session identifiers the SCXML Event I/O Processor can deliver to, when known. -/
  sessions : Option (List String) := none
  deriving Repr, BEq

end Vertep
