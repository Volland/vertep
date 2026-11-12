import Lean.Data.Json
import VertepLean.Step

/-!
# The oracle exchange format

JSON encodings matching serde's default (externally tagged) representation of the Rust types in
`vertep_core::{datamodel, session, tape, trace}`, as `crates/vertep-core/src/tape.rs` documents:
one `OracleRecord` per line, `{"input": …, "tape": […], "trace": …}`. Numbers of `Value` are
read exactly as JSON decimals and written as decimals (`1.5`, `2`); Rust reads both as `f64`.
-/

namespace Vertep.Json

open Lean (Json JsonNumber)

abbrev R := Except String

def field (j : Json) (k : String) : R Json := j.getObjVal? k

/-- An optional field: absent and `null` are `none` (serde `Option` and `#[serde(default)]`). -/
def optField (j : Json) (k : String) : Option Json :=
  match j.getObjVal? k with
  | .ok .null => none
  | .ok v => some v
  | .error _ => none

def str (j : Json) (k : String) : R String := do (← field j k).getStr?
def optStr (j : Json) (k : String) : R (Option String) :=
  match optField j k with
  | some v => do pure (some (← v.getStr?))
  | none => pure none
def nat (j : Json) (k : String) : R Nat := do (← field j k).getNat?
def bool (j : Json) (k : String) : R Bool := do (← field j k).getBool?
def arr (j : Json) (k : String) : R (List Json) := do pure (← (← field j k).getArr?).toList

/-- An externally tagged enum value: the tag and its content (`null` for unit variants). -/
def tagged (j : Json) : R (String × Json) :=
  match j with
  | .str s => pure (s, .null)
  | .obj kvs => match kvs.toList with
    | [(k, v)] => pure (k, v)
    | _ => throw s!"expected a single-key object, got {j.compress}"
  | _ => throw s!"expected an enum value, got {j.compress}"

def optJ {α : Type} (f : α → Json) : Option α → Json
  | some a => f a
  | none => .null

/-! ## Values and events -/

partial def valueFromJson (j : Json) : R Value := do
  match ← tagged j with
  | ("Undefined", _) => pure .undefined
  | ("Null", _) => pure .null
  | ("Bool", b) => pure (.bool (← b.getBool?))
  | ("Number", n) =>
    let n ← n.getNum?
    pure (Value.number n.mantissa n.exponent)
  | ("String", s) => pure (.str (← s.getStr?))
  | ("Array", a) => pure (.array (← (← a.getArr?).toList.mapM valueFromJson))
  | ("Object", o) =>
    pure (.object (← (← o.getArr?).toList.mapM fun kv => do
      let kv ← kv.getArr?
      match kv.toList with
      | [k, v] => pure ((← k.getStr?), (← valueFromJson v))
      | _ => throw "an object entry is a [key, value] pair"))
  | (t, _) => throw s!"unknown value variant {t}"

partial def valueToJson : Value → Json
  | .undefined => .str "Undefined"
  | .null => .str "Null"
  | .bool b => .mkObj [("Bool", .bool b)]
  | .num m e => .mkObj [("Number", .num ⟨m, e⟩)]
  | .str s => .mkObj [("String", .str s)]
  | .array vs => .mkObj [("Array", .arr (vs.map valueToJson).toArray)]
  | .object kvs => .mkObj [("Object", .arr (kvs.map fun (k, v) => Lean.Json.arr #[.str k, valueToJson v]).toArray)]

def eventTypeFromJson (j : Json) : R EventType := do
  match ← j.getStr? with
  | "Platform" => pure .platform
  | "Internal" => pure .internal
  | "External" => pure .external
  | t => throw s!"unknown event type {t}"

def eventTypeToJson : EventType → Json
  | .platform => "Platform"
  | .internal => "Internal"
  | .external => "External"

def eventFromJson (j : Json) : R Event := do
  pure { name := ← str j "name", type := ← eventTypeFromJson (← field j "type_"),
         sendid := ← optStr j "sendid", origin := ← optStr j "origin",
         origintype := ← optStr j "origintype",
         originverified := ← match optField j "originverified" with
           | some v => v.getBool?
           | none => pure false,
         invokeid := ← optStr j "invokeid",
         data := ← valueFromJson (← field j "data") }

def eventToJson (e : Event) : Json :=
  .mkObj [("name", .str e.name), ("type_", eventTypeToJson e.type),
    ("sendid", optJ .str e.sendid), ("origin", optJ .str e.origin),
    ("origintype", optJ .str e.origintype), ("originverified", .bool e.originverified),
    ("invokeid", optJ .str e.invokeid), ("data", valueToJson e.data)]

def infoFromJson (j : Json) : R SessionInfo := do
  pure { sessionid := ← str j "sessionid", name := ← optStr j "name",
         ioprocessors := ← match optField j "ioprocessors" with
           | some v => valueFromJson v
           | none => pure .undefined }

def infoToJson (i : SessionInfo) : Json :=
  .mkObj [("sessionid", .str i.sessionid), ("name", optJ .str i.name),
    ("ioprocessors", valueToJson i.ioprocessors)]

def envFromJson (j : Json) : R Environment := do
  pure { nowMs := ← (← field j "now_ms").getInt?, seed := ← nat j "seed" }

def envToJson (e : Environment) : Json :=
  .mkObj [("now_ms", .num (JsonNumber.fromInt e.nowMs)), ("seed", .num (JsonNumber.fromNat e.seed))]

def optionsFromJson (j : Json) : R Options := do
  pure { microstepBudget := ← nat j "microstep_budget",
         sendTypes := ← (← arr j "send_types").mapM (·.getStr?),
         invokeTypes := ← (← arr j "invoke_types").mapM (·.getStr?) }

def sessionsFromJson (j : Json) : R (Option (List String)) :=
  match optField j "sessions" with
  | some v => do pure (some (← (← v.getArr?).toList.mapM (·.getStr?)))
  | none => pure none

def startInputFromJson (j : Json) : R StartInput := do
  let data ← match optField j "data" with
    | some v => (← v.getArr?).toList.mapM fun kv => do
        match (← kv.getArr?).toList with
        | [k, v] => pure ((← k.getStr?), (← valueFromJson v))
        | _ => throw "a data entry is a [name, value] pair"
    | none => pure []
  pure { info := ← infoFromJson (← field j "info"), env := ← envFromJson (← field j "env"),
         options := ← optionsFromJson (← field j "options"), data,
         sessions := ← sessionsFromJson j }

def inputFromJson (j : Json) : R Input := do
  let kind ← match ← tagged (← field j "kind") with
    | ("Event", e) => pure (InputKind.event (← eventFromJson e))
    | ("Cancel", _) => pure InputKind.cancel
    | (t, _) => throw s!"unknown input kind {t}"
  pure { env := ← envFromJson (← field j "env"), kind, sessions := ← sessionsFromJson j }

/-! ## The tape -/

def requestFromJson (j : Json) : R Request := do
  match ← tagged j with
  | ("InitSession", v) => pure (.initSession (← infoFromJson v))
  | ("SetEnvironment", v) => pure (.setEnvironment (← envFromJson v))
  | ("Declare", v) => pure (.declare (← str v "id"))
  | ("SetEvent", v) => pure (.setEvent (← eventFromJson v))
  | ("EvalCond", v) => pure (.evalCond (← str v "expr"))
  | ("EvalValue", v) => pure (.evalValue (← str v "expr"))
  | ("EvalContent", v) => pure (.evalContent (← str v "content"))
  | ("AssignExpr", v) => pure (.assignExpr (← str v "location") (← str v "expr"))
  | ("AssignValue", v) => pure (.assignValue (← str v "location") (← valueFromJson (← field v "value")))
  | ("BindVar", v) => pure (.bindVar (← str v "name") (← valueFromJson (← field v "value")))
  | ("IsLocation", v) => pure (.isLocation (← str v "location"))
  | ("ExecScript", v) => pure (.execScript (← str v "script"))
  | ("ForeachItems", v) => pure (.foreachItems (← str v "array"))
  | ("Restore", v) => pure (.restore (← valueFromJson v))
  | (t, _) => throw s!"unknown request {t}"

def answerFromJson (j : Json) : R Answer := do
  match ← tagged j with
  | ("Unit", _) => pure .unit
  | ("Bool", b) => pure (.bool (← b.getBool?))
  | ("Value", v) => pure (.value (← valueFromJson v))
  | ("Values", vs) => pure (.values (← (← vs.getArr?).toList.mapM valueFromJson))
  | ("Error", e) => pure (.error (← str e "message"))
  | (t, _) => throw s!"unknown answer {t}"

def tapeEntryFromJson (j : Json) : R TapeEntry := do
  pure { request := ← requestFromJson (← field j "request"),
         answer := ← answerFromJson (← field j "answer") }

/-! ## Traces -/

def effectToJson : Effect → Json
  | .send r => .mkObj [("Send", .mkObj [("sendid", .str r.sendid), ("event", eventToJson r.event),
      ("target", optJ .str r.target), ("type_", .str r.type),
      ("delay_ms", .num (JsonNumber.fromNat r.delayMs))])]
  | .cancel id => .mkObj [("Cancel", .mkObj [("sendid", .str id)])]
  | .invoke r => .mkObj [("Invoke", .mkObj [("invokeid", .str r.invokeid), ("state", .str r.state),
      ("type_", .str r.type), ("src", optJ .str r.src), ("content", optJ valueToJson r.content),
      ("params", valueToJson r.params), ("autoforward", .bool r.autoforward)])]
  | .cancelInvoke id => .mkObj [("CancelInvoke", .mkObj [("invokeid", .str id)])]
  | .forward id e => .mkObj [("Forward", .mkObj [("invokeid", .str id), ("event", eventToJson e)])]
  | .log l v => .mkObj [("Log", .mkObj [("label", optJ .str l), ("value", optJ valueToJson v)])]
  | .sessionDone d c => .mkObj [("SessionDone", .mkObj [("data", valueToJson d), ("cancelled", .bool c)])]

def effectFromJson (j : Json) : R Effect := do
  match ← tagged j with
  | ("Send", v) => do
    let r : SendRequest :=
      { sendid := ← str v "sendid", event := ← eventFromJson (← field v "event"),
        target := ← optStr v "target", type := ← str v "type_", delayMs := ← nat v "delay_ms" }
    pure (.send r)
  | ("Cancel", v) => pure (.cancel (← str v "sendid"))
  | ("Invoke", v) => do
    let content ← match optField v "content" with
      | some c => do pure (some (← valueFromJson c))
      | none => pure none
    let r : InvokeRequest :=
      { invokeid := ← str v "invokeid", state := ← str v "state", type := ← str v "type_",
        src := ← optStr v "src", content, params := ← valueFromJson (← field v "params"),
        autoforward := ← bool v "autoforward" }
    pure (.invoke r)
  | ("CancelInvoke", v) => pure (.cancelInvoke (← str v "invokeid"))
  | ("Forward", v) => pure (.forward (← str v "invokeid") (← eventFromJson (← field v "event")))
  | ("Log", v) => pure (.log (← optStr v "label") (← match optField v "value" with
      | some x => do pure (some (← valueFromJson x))
      | none => pure none))
  | ("SessionDone", v) => pure (.sessionDone (← valueFromJson (← field v "data")) (← bool v "cancelled"))
  | (t, _) => throw s!"unknown effect {t}"

def traceInputToJson : TraceInput → Json
  | .start sid => .mkObj [("Start", .mkObj [("sessionid", .str sid)])]
  | .event e => .mkObj [("Event", eventToJson e)]
  | .cancel => "Cancel"

def traceInputFromJson (j : Json) : R TraceInput := do
  match ← tagged j with
  | ("Start", v) => pure (.start (← str v "sessionid"))
  | ("Event", e) => pure (.event (← eventFromJson e))
  | ("Cancel", _) => pure .cancel
  | (t, _) => throw s!"unknown trace input {t}"

def microToJson (m : MicrostepTrace) : Json :=
  .mkObj [("event", optJ .str m.event), ("transitions", .arr (m.transitions.map fun r =>
    Lean.Json.mkObj [("source", .str r.source), ("index", .num (JsonNumber.fromNat r.index))]).toArray)]

def microFromJson (j : Json) : R MicrostepTrace := do
  pure { event := ← optStr j "event",
         transitions := ← (← arr j "transitions").mapM fun r => do
           pure { source := ← str r "source", index := ← nat r "index" } }

/-- A `MacrostepTrace` exactly as serde writes it. -/
def traceToJson (t : MacrostepTrace) : Json :=
  .mkObj [("index", .num (JsonNumber.fromNat t.index)), ("input", traceInputToJson t.input),
    ("microsteps", .arr (t.microsteps.map microToJson).toArray),
    ("configuration", .arr (t.configuration.map Lean.Json.str).toArray),
    ("effects", .arr (t.effects.map effectToJson).toArray), ("terminated", .bool t.terminated)]

def traceFromJson (j : Json) : R MacrostepTrace := do
  pure { index := ← nat j "index", input := ← traceInputFromJson (← field j "input"),
         microsteps := ← (← arr j "microsteps").mapM microFromJson,
         configuration := ← (← arr j "configuration").mapM (·.getStr?),
         effects := ← (← arr j "effects").mapM effectFromJson, terminated := ← bool j "terminated" }

/-! ## Oracle records -/

/-- The input of one macrostep (Rust `OracleInput`). -/
inductive OracleInput where
  | start (si : StartInput)
  | step (i : Input)

/-- One line of the exchange (Rust `OracleRecord`). -/
structure OracleRecord where
  input : OracleInput
  tape : Tape
  trace : MacrostepTrace

def recordFromJson (j : Json) : R OracleRecord := do
  let input ← match ← tagged (← field j "input") with
    | ("Start", v) => pure (OracleInput.start (← startInputFromJson v))
    | ("Step", v) => pure (OracleInput.step (← inputFromJson v))
    | (t, _) => throw s!"unknown oracle input {t}"
  pure { input, tape := ← (← arr j "tape").mapM tapeEntryFromJson,
         trace := ← traceFromJson (← field j "trace") }

end Vertep.Json
