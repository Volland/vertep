import Lean.Data.Json
import VertepLean.Disposition

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
vertep-decide --obs DECISIONS.jsonl [--strict]
```

One line per decision: `{"open": null | {"corr", "cands": [{"event", "source", "index"}],
"fallback"?: bool}, "answered": bool, "corr", "event", "outcome": "taken" | "rejected" |
"duplicate", "reason": string | null, "taken": [[source, index]], "schema"?: JSON Schema,
"payload"?: JSON, "annotations"?: object, "disposition"?: string, "offered"?: [string]}`, or one
per refusal: `{"kind": "refusal", "open", "corr", "disposition"?, "offered"?}`.

**Dispositions (m12-decision-records 2.3).** A line that carries the disposition and the offer
recorded on its journal entry is checked against the model (`Vertep.Agent.dispositionOf`,
`refusalDisposition`, `offeredFor`): the disposition must be the one the model computes from the
open offer (its candidates and whether it was made to the fallback decider), the annotations
(`confidence` within [0, 1]), the payload's conformance and the guard answer above, and the offer
must be the open offer's events. With `--strict` every line except a duplicate must carry both,
so a journal entry without a disposition, or with one the model does not compute, fails.

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

/-- One observed decision or refusal. -/
structure Obs where
  /-- `true` for a refusal line. -/
  refusal : Bool := false
  open_ : Option (String × List Cand)
  /-- Whether the open offer was made to the fallback decider. -/
  fallback : Bool := false
  answered : Bool
  corr : String
  event : String
  outcome : String
  reason : Option String
  taken : List TransRef
  /-- Whether the payload conforms to the candidate's schema, when the line carries both. -/
  conforms : Option Bool := none
  /-- Whether the annotations validate (Rust `Annotations::validate`: a `confidence`, when
  given, is within [0, 1]). -/
  annOk : Bool := true
  /-- The disposition recorded on the journal entry. -/
  disposition : Option String := none
  /-- The offer recorded on the journal entry. -/
  offered : Option (List String) := none

def candFromJson (j : Json) : Except String Cand := do
  pure ⟨← (← j.getObjVal? "event").getStr?,
    (← (← j.getObjVal? "source").getStr?, ← (← j.getObjVal? "index").getNat?)⟩

/-- Rust `Annotations::validate` on the annotations' JSON. -/
def annotationsOk (a : Json) : Bool :=
  match key? a "confidence" with
  | some (.num c) => numCmp c 0 != .lt && numCmp c 1 != .gt
  | _ => true

/-- An optional field: absent and `null` are `none`. -/
def opt (j : Json) (k : String) : Option Json :=
  match j.getObjVal? k with
  | .ok .null | .error _ => none
  | .ok v => some v

def obsFromJson (j : Json) : Except String Obs := do
  let refusal := (opt j "kind").bind (·.getStr?.toOption) == some "refusal"
  let (open_, fallback) ← match opt j "open" with
    | none => pure (none, false)
    | some o => do
      let cands ← (← (← o.getObjVal? "cands").getArr?).toList.mapM candFromJson
      let fb ← match opt o "fallback" with
        | some f => f.getBool?
        | none => pure false
      pure (some (← (← o.getObjVal? "corr").getStr?, cands), fb)
  let reason ← match opt j "reason" with
    | none => pure none
    | some r => pure (some (← r.getStr?))
  let taken ← match opt j "taken" with
    | none => pure []
    | some ts => (← ts.getArr?).toList.mapM fun t => do
      match (← t.getArr?).toList with
      | [s, i] => pure ((← s.getStr?), (← i.getNat?))
      | _ => throw s!"a taken transition is [source, index], got {t.compress}"
  let conforms ← match j.getObjVal? "schema", j.getObjVal? "payload" with
    | .ok schema, .ok payload => pure (some (← conformsJson schema payload))
    | _, _ => pure none
  let disposition ← match opt j "disposition" with
    | none => pure none
    | some d => pure (some (← d.getStr?))
  let offered ← match opt j "offered" with
    | none => pure none
    | some o => pure (some (← (← o.getArr?).toList.mapM (·.getStr?)))
  let str (k : String) : Except String String := do
    if refusal then pure ((opt j k).bind (·.getStr?.toOption) |>.getD "") else (← j.getObjVal? k).getStr?
  pure { refusal, open_, fallback,
         answered := ← (if refusal then pure false else do (← j.getObjVal? "answered").getBool?),
         corr := ← (← j.getObjVal? "corr").getStr?, event := ← str "event",
         outcome := ← str "outcome", reason, taken, conforms,
         annOk := ((opt j "annotations").map annotationsOk).getD true, disposition, offered }

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

/-- The index of the line's correlation id in `stateOf`. -/
def idxOf (o : Obs) : Nat := if (o.open_.map (·.1)) = some o.corr then 1 else 2

/-- Compare the model's outcome with the recorded one. A decision whose annotations fail
validation, answering the open offer, is rejected (`annotation`) before its event is looked at,
as in Rust; the model's `decide` does not see annotations. -/
def judge (o : Obs) : Except String Unit := do
  if o.refusal then return
  let idx := idxOf o
  if !o.annOk && !o.answered && idx = 1 then
    if o.outcome = "rejected" && o.reason = some "annotation" then return
    else throw s!"decision {o.corr} {o.event}: invalid annotations, Rust outcome {o.outcome} ({o.reason})"
  match (decide (coreOf o) (stateOf o) idx o.event ()).2, o.outcome with
  | .none, "duplicate" => pure ()
  | .rejected r, "rejected" =>
    if o.reason = some r then pure ()
    else throw s!"decision {o.corr} {o.event}: Rust rejected it as {o.reason}, the model as {r}"
  | .taken c, "taken" =>
    if o.taken.contains c.trans then pure ()
    else throw s!"decision {o.corr} {o.event}: the model takes {c.trans}, Rust took {o.taken}"
  | m, r => throw s!"decision {o.corr} {o.event}: Rust outcome {r} ({o.reason}), the model {repr m}"

/-- The model's disposition and offer for a line. -/
def dispositionFor (o : Obs) : Disposition × List String :=
  let s := stateOf o
  let idx := idxOf o
  let d := if o.refusal then refusalDisposition
    else dispositionOf (coreOf o) s idx o.event () o.annOk o.fallback
  (d, offeredFor s idx)

/-- Compare the disposition and the offer recorded on the entry with the model's; in strict
mode a line other than a duplicate must carry both. -/
-- @tg: implements:: [[openspec:decision-records#Decision Disposition Is Recorded]] {note: "strict replay through the Lean model"}
def judgeDisposition (strict : Bool) (o : Obs) : Except String Unit := do
  let what := if o.refusal then s!"refusal {o.corr}" else s!"decision {o.corr} {o.event}"
  match o.disposition, o.offered with
  | some d, some off =>
    let (md, moff) := dispositionFor o
    if d != md.name then
      throw s!"{what}: the entry records disposition {d}, the model computes {md.name}"
    if off != moff then
      throw s!"{what}: the entry records offer {off}, the model {moff}"
  | none, none =>
    if strict && o.outcome != "duplicate" then
      throw s!"{what}: no disposition and offer on the entry (strict)"
  | _, _ => throw s!"{what}: the entry records only one of disposition and offer"

/-- Judge every line: its outcome and, when recorded (always in strict mode), its disposition.
Returns the number of dispositions checked. -/
def judgeAll (strict : Bool) (os : List Obs) : Except String Nat := do
  for o in os do
    judge o
    judgeDisposition strict o
  pure (os.filter (·.disposition.isSome)).length

/-- Parse the lines of an observation file. -/
def parse (src : String) : Except String (List Obs) :=
  ((src.splitOn "\n").filter (fun l => !(l.all Char.isWhitespace))).mapM
    (fun l => Json.parse l >>= obsFromJson)

/-- The command line. -/
-- @tg: implements:: [[openspec:deciders#Decider Safety]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some (path, strict)) := (match args with
    | ["--obs", p] => some (p, false)
    | ["--obs", p, "--strict"] | ["--strict", "--obs", p] => some (p, true)
    | _ => none)
    | IO.eprintln "usage: vertep-decide --obs DECISIONS.jsonl [--strict]"; return 3
  let src ← IO.FS.readFile path
  let os ← match parse src with
    | .ok os => pure os
    | .error e => IO.eprintln s!"observations: {e}"; return 3
  match judgeAll strict os with
  | .ok n =>
    IO.println s!"{os.length} decisions and refusals agree, {n} dispositions{if strict then " (strict)" else ""}"
    return 0
  | .error e => IO.eprintln e; return 1

end Vertep.Agent.Check
