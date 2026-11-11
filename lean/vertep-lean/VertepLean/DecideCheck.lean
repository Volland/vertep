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

As with the datamodel answer tape of `vertep-oracle`, the two answers the abstract core gives
the model at decision time come from the Rust run: whether the payload conforms to the
candidate's schema (`payload-schema` rejections) and whether selection now takes the offered
transition (`not-enabled` rejections). Everything else is decided by the model: correlation
matching, candidate membership, deduplication and the order of the checks.

```
vertep-decide --obs DECISIONS.jsonl
```

One line per decision: `{"open": null | {"corr", "cands": [{"event", "source", "index"}]},
"answered": bool, "corr", "event", "outcome": "taken" | "rejected" | "duplicate",
"reason": string | null, "taken": [[source, index]]}`. Exit codes: `0` all agree, `1` the first
disagreement on standard error, `3` an input error.
-/

namespace Vertep.Agent.Check

open Lean (Json)

/-- One observed decision. -/
structure Obs where
  open_ : Option (String × List Cand)
  answered : Bool
  corr : String
  event : String
  outcome : String
  reason : Option String
  taken : List TransRef

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
  pure { open_, answered := ← (← j.getObjVal? "answered").getBool?,
         corr := ← (← j.getObjVal? "corr").getStr?, event := ← (← j.getObjVal? "event").getStr?,
         outcome := ← (← j.getObjVal? "outcome").getStr?, reason, taken }

/-- The core the model sees for one decision: the context is irrelevant to `decide`; the two
decision-time answers come from the Rust run. -/
def coreOf (o : Obs) : Core Unit Unit where
  names _ := []
  active _ := true
  probe _ _ := none
  select _ e _ :=
    if o.reason = some "not-enabled" then none
    else ((o.open_.map (·.2)).getD [] |>.find? (·.event = e)).map (·.trans)
  conforms _ _ := o.reason != some "payload-schema"
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
