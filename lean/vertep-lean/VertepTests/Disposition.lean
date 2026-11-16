import VertepLean.DecideCheck
import VertepTests.Agent

/-!
# Decision dispositions (m12-decision-records 2.3)

The model's disposition of every answer the shell can receive, on the review chart of
`VertepTests.Agent` and on a core whose guard reads the payload; then the conformance vectors
(`data/dispositions.jsonl`, written by `crates/vertep-conformance/tests/disposition_vectors.rs`
from journals of the Rust host) replayed through the decision oracle in strict mode, and copies
with a disposition altered or removed, which strict replay rejects.
-/

namespace Vertep.Agent.DispositionTest

open Vertep.Agent Vertep.Agent.Test Vertep.Agent.Check

/-- `review` after `bump`: offer 1 is open with `approve` and `reject`. -/
def s1 : AState Ctx := (stepA review (init review ("review", 0)) (.event "bump" false)).1

#guard offeredFor s1 1 == ["approve", "reject"]
#guard offeredFor s1 0 == []
-- an offered, enabled event with a conforming payload
#guard dispositionOf review s1 1 "approve" true true false == .accepted
-- the same answer from the fallback decider, after the deadline
#guard dispositionOf review s1 1 "approve" true true true == .fallbackOnDeadline
-- an event that was not offered, whatever the fallback flag
#guard dispositionOf review s1 1 "escalate" true true false == .refusedNotOffered
#guard dispositionOf review s1 1 "delete_all" true true true == .refusedNotOffered
-- a payload that does not conform, and annotations that do not validate
#guard dispositionOf review s1 1 "approve" false true false == .refusedInvalid
#guard dispositionOf review s1 1 "reject" true false false == .refusedInvalid
-- a superseded or forged correlation id
#guard dispositionOf review s1 0 "approve" true true false == .stale
#guard dispositionOf review s1 7 "reject" true true false == .stale
#guard refusalDisposition == .refusedByDecider

/-- A core whose `approve` guard reads the payload: offered while the payload is unknown,
not taken with a payload that makes it false. -/
def payloadGuard : Core Unit Bool where
  names _ := ["approve"]
  active _ := true
  probe _ e := if e == "approve" then some ("review", 0) else none
  select _ e p := if e == "approve" && p then some ("review", 0) else none
  conforms _ _ := true
  step _ _ _ := ()

def g0 : AState Unit := init payloadGuard ()

#guard offeredFor g0 0 == ["approve"]
#guard dispositionOf payloadGuard g0 0 "approve" true true false == .accepted
-- `refused-guard`: offered, conforming, but selection no longer takes it
#guard dispositionOf payloadGuard g0 0 "approve" false true false == .refusedGuard
-- and `decide` rejects it as `not-enabled`, the outcome refused-guard stands for
#guard (decide payloadGuard g0 0 "approve" false).2 == .rejected "not-enabled"
#guard ofOutcome false (decide payloadGuard g0 0 "approve" false).2 == some .refusedGuard

/-! ## The conformance vectors, strict -/

def vectors : String := include_str "data/dispositions.jsonl"

/-- Every line of the vectors carries a disposition and an offer, and both are the model's. -/
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#An accepted answer is marked accepted]] {level: unit}
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A refused answer is marked refused]] {level: unit}
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A deadline fallback is marked]] {level: unit}
def vectorsAgree : Bool :=
  match parse vectors >>= judgeAll true with
  | .ok n => n == 14
  | .error _ => false
#guard vectorsAgree

/-- Every disposition the shell produces appears in the vectors; `fallback-on-refusal` does
not. -/
def coverage : Bool :=
  match parse vectors with
  | .ok os =>
    let ds := os.filterMap (·.disposition)
    ["accepted", "refused-not-offered", "refused-guard", "refused-invalid", "stale",
      "fallback-on-deadline", "refused-by-decider"].all ds.contains &&
      !ds.contains "fallback-on-refusal"
  | .error _ => false
#guard coverage

def strictFails (src : String) (msg : String) : Bool :=
  match parse src >>= judgeAll true with
  | .error e => (e.splitOn msg).length > 1
  | .ok _ => false

-- a recorded disposition the model does not compute
#guard strictFails (vectors.replace "\"disposition\":\"refused-guard\"" "\"disposition\":\"accepted\"")
  "the model computes refused-guard"
#guard strictFails (vectors.replace "\"disposition\":\"fallback-on-deadline\""
  "\"disposition\":\"fallback-on-refusal\"") "the model computes fallback-on-deadline"
-- a recorded offer that is not the open offer's
#guard strictFails (vectors.replace "\"offered\":[]" "\"offered\":[\"approve\"]")
  "the model []"
-- an entry without a disposition fails in strict mode, and passes without it
#guard strictFails (vectors.replace "\"disposition\":\"stale\",\"event\":\"reject\",\"offered\":[]"
  "\"event\":\"reject\"") "no disposition and offer"
#guard (parse (vectors.replace "\"disposition\":\"stale\",\"event\":\"reject\",\"offered\":[]"
  "\"event\":\"reject\"") >>= judgeAll false) matches .ok _

end Vertep.Agent.DispositionTest
