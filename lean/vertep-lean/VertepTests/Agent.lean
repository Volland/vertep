import VertepLean.AgentTheorems
import VertepLean.DecideCheck

/-!
# Decide states and grants

A review chart in the abstract core: the decide state `review` offers `approve` (guarded by
`x > 0`), `reject`, and `escalate` (guard false). Adversarial decider outputs (an unknown event,
a bad payload, a stale correlation id, a duplicate) take no transition; the offered one does.
-/

namespace Vertep.Agent.Test

open Vertep.Agent

/-- The context: the active state and a counter. -/
abbrev Ctx := String × Nat

def review : Core Ctx Bool where
  names _ := ["approve", "reject", "escalate"]
  active k := k.1 == "review"
  probe k e :=
    if k.1 != "review" then none
    else if e == "approve" then (if k.2 > 0 then some ("review", 0) else none)
    else if e == "reject" then some ("review", 1)
    else none
  select k e _ :=
    if k.1 != "review" then none
    else if e == "approve" then (if k.2 > 0 then some ("review", 0) else none)
    else if e == "reject" then some ("review", 1)
    else none
  conforms e p := e != "approve" || p
  step k e _ :=
    if k.1 == "review" && e == "approve" && k.2 > 0 then ("approved", k.2)
    else if k.1 == "review" && e == "reject" then ("rejected", k.2)
    else if e == "bump" then (k.1, k.2 + 1)
    else k

def offered (s : AState Ctx) : List String := (s.slot.map (·.cands.map (·.event))).getD []
def corrOf (s : AState Ctx) : Nat := (s.slot.map (·.corr)).getD 999

/-- Only enabled transitions are offered; a context change re-offers with a new id. -/
-- @tg: verifies:: [[openspec:deciders#Enabled Event Offer#Only enabled transitions are offered]] {level: unit}
def offerChecks : Bool :=
  let s0 := init review ("review", 0)
  let s1 := (stepA review s0 (.event "bump" false)).1
  offered s0 == ["reject"] && corrOf s0 == 0 &&
    offered s1 == ["approve", "reject"] && corrOf s1 == 1
#guard offerChecks

/-- Adversarial decider outputs take no transition; a valid decision does, once. -/
-- @tg: verifies:: [[openspec:deciders#Decider Safety#Arbitrary decider output is safe]] {level: unit}
def decisionChecks : Bool :=
  let s1 := (stepA review (init review ("review", 0)) (.event "bump" false)).1
  let stale := stepA review s1 (.decision 0 "approve" true)
  let unknown := stepA review s1 (.decision 1 "delete_all" true)
  let badPayload := stepA review s1 (.decision 1 "approve" false)
  let ok := stepA review s1 (.decision 1 "approve" true)
  let again := stepA review ok.1 (.decision 1 "approve" true)
  stale.2 == .rejected "stale" && stale.1.ctx == ("review", 1) &&
    unknown.2 == .rejected "not-a-candidate" && unknown.1.ctx == ("review", 1) &&
    badPayload.2 == .rejected "payload-schema" &&
    ok.2 == .taken ⟨"approve", ("review", 0)⟩ && ok.1.ctx == ("approved", 1) &&
    again.2 == .none && again.1.ctx == ("approved", 1)
#guard decisionChecks

/-- Replaying the journal of a live run with a decider that always approves reaches the same
state. -/
-- @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Replay does not call the decider]] {level: unit}
def replayCheck : Bool :=
  let D : AState Ctx → Option (Input Bool) := fun s =>
    s.slot.map fun o => .decision o.corr "approve" true
  let s0 := init review ("review", 0)
  let r := live review D s0 [.event "bump" false, .event "noop" false]
  (replay review s0 r.2).ctx == r.1.ctx && r.1.ctx == ("approved", 1) && r.2.length == 3
#guard replayCheck

/-- Grants intersect down the invoke chain, and a send to an ungranted actor is replaced by the
error and the notice. -/
-- @tg: verifies:: [[openspec:llm-actors#Capability Grants#Ungranted send fails]] {level: unit}
def grantChecks : Bool :=
  let g := chainGrant [some ["#_parent", "search", "crm"], some ["#_parent", "search"]]
  let outs := [⟨.named "search", "q"⟩, ⟨.named "vt:actor/Payments/1", "pay"⟩, ⟨.parent, "done"⟩]
  g == some ["#_parent", "search"] &&
    filter ["#_parent", "search"] outs ==
      [⟨.named "search", "q"⟩, ⟨.itself, "error.communication"⟩, ⟨.parent, "vt.grant.denied"⟩,
       ⟨.parent, "done"⟩]
#guard grantChecks

/-- The payload checker of `vertep-decide` on schemas of the type system's JSON Schema
projection: a closed record, `int` bounds, `optional`, a union and a named type. -/
def conformsChecks : Bool :=
  let ok (s p : String) : Bool := match Lean.Json.parse s, Lean.Json.parse p with
    | .ok s, .ok p => (Check.conformsJson s p).toOption == some true
    | _, _ => false
  let rec_ := "{\"type\":\"object\",\"properties\":{\"n\":{\"type\":\"integer\",\"minimum\":-9223372036854775808,\"exclusiveMaximum\":9223372036854775808},\"b\":{\"anyOf\":[{\"type\":\"null\"},{\"type\":\"boolean\"}]}},\"required\":[\"n\"],\"additionalProperties\":false}"
  let union := "{\"anyOf\":[{\"type\":\"object\",\"properties\":{\"tag\":{\"const\":\"ok\"},\"value\":{\"type\":\"string\"}},\"required\":[\"tag\",\"value\"],\"additionalProperties\":false},{\"type\":\"object\",\"properties\":{\"tag\":{\"const\":\"none\"}},\"required\":[\"tag\"],\"additionalProperties\":false}]}"
  let named := "{\"$ref\":\"#/$defs/L\",\"$defs\":{\"L\":{\"type\":\"array\",\"items\":{\"type\":\"number\"}}}}"
  ok rec_ "{\"n\":3}" && ok rec_ "{\"n\":3.0,\"b\":true}" && !ok rec_ "{\"n\":1.5}" &&
    !ok rec_ "{}" && !ok rec_ "{\"n\":1,\"x\":1}" && !ok rec_ "{\"n\":1,\"b\":2}" &&
    !ok rec_ "null" && ok union "{\"tag\":\"none\"}" && ok union "{\"tag\":\"ok\",\"value\":\"v\"}" &&
    !ok union "{\"tag\":\"ok\"}" && !ok union "{\"tag\":\"bad\"}" && ok named "[1, 2.5]" &&
    !ok named "[1, \"x\"]" && ok "{}" "null" && !ok "{\"type\":\"null\"}" "1" &&
    (Check.conformsJson (.str "x") (.str "y")).toOption == some true
#guard conformsChecks

end Vertep.Agent.Test
