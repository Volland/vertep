import Lean.Data.Json
import VertepLean.AgentTheorems

/-!
# Decision validation as a differential oracle (m3-agents 8.5)

`vertep-decide` replays the decisions of a Rust agent run (`AgentMachine::apply` on
`vt.decision` inputs) through the model's `Agent.decide` and requires the same outcome for each:
taken, rejected with the same reason (`stale`, `not-a-candidate`, `payload-schema`,
`not-enabled`), or dropped as a duplicate; for a taken decision, the transition the model takes
(a candidate of the open offer, enabled at offer time by `decider_safety`) must be among the
transitions Rust took.

As with the datamodel answer tape of `vertep-oracle`, whether selection now takes the offered
transition (`not-enabled` rejections) comes from the Rust run. Whether the payload conforms to
the candidate's schema (`payload-schema` rejections) is decided here when the line carries the
schema and the payload (`conformsJson`, an independent port of the payload validator of
`vertep-agent` for the JSON Schema projection of the type system, without `pattern`); a line
without them takes that answer from the Rust run too. Everything else is decided by the model:
correlation matching, candidate membership, deduplication and the order of the checks.

```
vertep-decide --obs DECISIONS.jsonl
```

One line per decision: `{"open": null | {"corr", "cands": [{"event", "source", "index"}]},
"answered": bool, "corr", "event", "outcome": "taken" | "rejected" | "duplicate",
"reason": string | null, "taken": [[source, index]], "schema"?: JSON Schema, "payload"?: JSON}`.
Exit codes: `0` all agree, `1` the first disagreement on standard error, `3` an input error
(including a schema keyword the checker does not support).
-/

namespace Vertep.Agent.Check

open Lean (Json)

/-! ## Payload conformance

The decision payload validator of `vertep-agent` (`schema::validate_json`) as a predicate: a
document conforms when the validator reports no violation. It covers `$ref` into the document,
`type`, `const`, `enum`, the four numeric bounds, `required`, `properties`,
`additionalProperties`, `items`, `allOf`, `anyOf` and `oneOf`, and ignores annotations
(`$schema`, `$defs`, `title`, `format`). Numbers are compared exactly; a number is an integer
when it has no fractional part. `pattern` on a string is not supported (an input error). -/

/-- Exact comparison of JSON numbers. -/
def numCmp (a b : Lean.JsonNumber) : Ordering :=
  compare (a.mantissa * ((10 ^ b.exponent : Nat) : Int)) (b.mantissa * ((10 ^ a.exponent : Nat) : Int))

/-- The JSON Schema type name of a value (`integer` for a number without fractional part). -/
def typeName : Json → String
  | .null => "null"
  | .bool _ => "boolean"
  | .num n => if n.mantissa % ((10 ^ n.exponent : Nat) : Int) == 0 then "integer" else "number"
  | .str _ => "string"
  | .arr _ => "array"
  | .obj _ => "object"

def isType (v : Json) (t : String) : Bool :=
  typeName v == t || (t == "number" && typeName v == "integer")

/-- Equality with numbers compared by value at the top level. -/
def jsonEq : Json → Json → Bool
  | .num a, .num b => numCmp a b == .eq
  | a, b => a == b

def key? (s : Json) (k : String) : Option Json := (s.getObjVal? k).toOption

def fields : Json → List (String × Json)
  | .obj kvs => kvs.toList
  | _ => []

/-- Whether `v` conforms to `schema` (inside the document `root`), with `fuel` the nesting
depth left (the validator reports a violation past 64 levels). -/
def conformsAt (root : Json) : Nat → Json → Json → Except String Bool
  | 0, _, _ => pure false
  | fuel + 1, schema, v => do
    match schema with
    | .obj _ =>
      let refOk ← match key? schema "$ref" with
        | some (.str r) =>
          match (r.dropPrefix? "#/").map (fun p => (p.toString.splitOn "/")) with
          | some segs =>
            match segs.foldlM (fun (j : Json) seg => key? j seg) root with
            | some t => conformsAt root fuel t v
            | none => pure false
          | none => pure false
        | _ => pure true
      let typeOk := match key? schema "type" with
        | some (.str t) => isType v t
        | some (.arr ts) => ts.toList.any fun t => match t with
          | .str t => isType v t
          | _ => false
        | _ => true
      if !typeOk then return false
      let constOk := match key? schema "const" with
        | some c => jsonEq c v
        | none => true
      let enumOk := match key? schema "enum" with
        | some (.arr e) => e.toList.any (jsonEq · v)
        | _ => true
      if (key? schema "pattern").isSome && (match v with | .str _ => true | _ => false) then
        throw "the schema keyword pattern is not supported"
      let boundsOk := match v with
        | .num n =>
          let b (k : String) (bad : Ordering → Bool) : Bool := match key? schema k with
            | some (.num m) => !bad (numCmp n m)
            | _ => true
          b "minimum" (· == .lt) && b "maximum" (· == .gt) &&
            b "exclusiveMinimum" (· != .gt) && b "exclusiveMaximum" (· != .lt)
        | _ => true
      let objOk ← match v with
        | .obj _ => do
          let reqOk := match key? schema "required" with
            | some (.arr req) => req.toList.all fun r => match r with
              | .str r => (key? v r).isSome
              | _ => true
            | _ => true
          let props := key? schema "properties"
          let propsOk ← (fields v).allM fun (k, x) =>
            match props.bind (key? · k) with
            | some ps => conformsAt root fuel ps x
            | none => match key? schema "additionalProperties" with
              | some (.bool false) => pure false
              | some a@(.obj _) => conformsAt root fuel a x
              | _ => pure true
          pure (reqOk && propsOk)
        | _ => pure true
      let itemsOk ← match key? schema "items", v with
        | some items, .arr a => a.toList.allM (conformsAt root fuel items)
        | _, _ => pure true
      let allOk ← match key? schema "allOf" with
        | some (.arr alls) => alls.toList.allM (conformsAt root fuel · v)
        | _ => pure true
      let anyOk ← match key? schema "anyOf" with
        | some (.arr alts) => do
          pure ((← alts.toList.filterM (conformsAt root fuel · v)).length != 0)
        | _ => pure true
      let oneOk ← match key? schema "oneOf" with
        | some (.arr alts) => do
          pure ((← alts.toList.filterM (conformsAt root fuel · v)).length == 1)
        | _ => pure true
      pure (refOk && constOk && enumOk && boundsOk && objOk && itemsOk && allOk && anyOk && oneOk)
    | .bool false => pure false
    | _ => pure true

/-- Whether `payload` conforms to `schema`. -/
-- @tg: implements:: [[decisions#The decision oracle checks payload conformance itself]]
def conformsJson (schema payload : Json) : Except String Bool :=
  conformsAt schema 65 schema payload

/-- One observed decision. -/
structure Obs where
  open_ : Option (String × List Cand)
  answered : Bool
  corr : String
  event : String
  outcome : String
  reason : Option String
  taken : List TransRef
  /-- Whether the payload conforms to the candidate's schema, when the line carries both. -/
  conforms : Option Bool := none

def candFromJson (j : Json) : Except String Cand := do
  pure ⟨← (← j.getObjVal? "event").getStr?,
    (← (← j.getObjVal? "source").getStr?, ← (← j.getObjVal? "index").getNat?)⟩

def obsFromJson (j : Json) : Except String Obs := do
  let open_ ← match j.getObjVal? "open" with
    | .ok .null | .error _ => pure none
    | .ok o => do
      let cands ← (← (← o.getObjVal? "cands").getArr?).toList.mapM candFromJson
      pure (some (← (← o.getObjVal? "corr").getStr?, cands))
  let reason ← match j.getObjVal? "reason" with
    | .ok .null | .error _ => pure none
    | .ok r => pure (some (← r.getStr?))
  let taken ← (← (← j.getObjVal? "taken").getArr?).toList.mapM fun t => do
    match (← t.getArr?).toList with
    | [s, i] => pure ((← s.getStr?), (← i.getNat?))
    | _ => throw s!"a taken transition is [source, index], got {t.compress}"
  let conforms ← match j.getObjVal? "schema", j.getObjVal? "payload" with
    | .ok schema, .ok payload => pure (some (← conformsJson schema payload))
    | _, _ => pure none
  pure { open_, answered := ← (← j.getObjVal? "answered").getBool?,
         corr := ← (← j.getObjVal? "corr").getStr?, event := ← (← j.getObjVal? "event").getStr?,
         outcome := ← (← j.getObjVal? "outcome").getStr?, reason, taken, conforms }

/-- The core the model sees for one decision: the context is irrelevant to `decide`; whether
selection takes the offered transition comes from the Rust run, conformance from `conformsJson`
when the line carries the schema and the payload. -/
def coreOf (o : Obs) : Core Unit Unit where
  names _ := []
  active _ := true
  probe _ _ := none
  select _ e _ :=
    if o.reason = some "not-enabled" then none
    else ((o.open_.map (·.2)).getD [] |>.find? (·.event = e)).map (·.trans)
  conforms _ _ := o.conforms.getD (o.reason != some "payload-schema")
  step _ _ _ := ()

/-- The model state before the decision: correlation ids as indices (1 for the open offer, 2
for anything else). -/
def stateOf (o : Obs) : AState Unit :=
  let idx (c : String) : Nat := if (o.open_.map (·.1)) = some c then 1 else 2
  { ctx := (), slot := o.open_.map fun (c, cs) => ⟨idx c, cs, ()⟩,
    answered := if o.answered then [idx o.corr] else [], next := 3, hist := [()] }

/-- Compare the model's outcome with the recorded one. -/
def judge (o : Obs) : Except String Unit := do
  let idx : Nat := if (o.open_.map (·.1)) = some o.corr then 1 else 2
  match (decide (coreOf o) (stateOf o) idx o.event ()).2, o.outcome with
  | .none, "duplicate" => pure ()
  | .rejected r, "rejected" =>
    if o.reason = some r then pure ()
    else throw s!"decision {o.corr} {o.event}: Rust rejected it as {o.reason}, the model as {r}"
  | .taken c, "taken" =>
    if o.taken.contains c.trans then pure ()
    else throw s!"decision {o.corr} {o.event}: the model takes {c.trans}, Rust took {o.taken}"
  | m, r => throw s!"decision {o.corr} {o.event}: Rust outcome {r} ({o.reason}), the model {repr m}"

/-- The command line. -/
-- @tg: implements:: [[openspec:deciders#Decider Safety]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--obs", p] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-decide --obs DECISIONS.jsonl"; return 3
  let src ← IO.FS.readFile path
  let os ← match ((src.splitOn "\n").filter (fun l => !(l.all Char.isWhitespace))).mapM
      (fun l => Json.parse l >>= obsFromJson) with
    | .ok os => pure os
    | .error e => IO.eprintln s!"observations: {e}"; return 3
  match os.mapM judge with
  | .ok _ => IO.println s!"{os.length} decisions agree"; return 0
  | .error e => IO.eprintln e; return 1

end Vertep.Agent.Check
