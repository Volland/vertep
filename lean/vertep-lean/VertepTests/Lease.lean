import VertepLean.LeaseCheck

/-!
# Leases and fencing

The split-brain scenario replayed through the differential oracle, and histories it must
reject.
-/

namespace Vertep.Lease.Test

open Vertep.Lease Vertep.Lease.Check

/-- Node `a` holds shard 9 at epoch 1 and appends; a partition hands the shard to `b` at epoch
2; `b` appends; `a`'s next append under epoch 1 is fenced; a receiver that has admitted `b`'s
frame drops `a`'s late one. -/
def splitBrain : List Event :=
  [.grant "a" 9 1,
   .append "a" 9 1 "urn:x" 1 "m1" .accept,
   .frame "r" 9 1 "m1" true,
   .grant "b" 9 2,
   .append "b" 9 2 "urn:x" 2 "m2" .accept,
   .append "a" 9 1 "urn:x" 3 "m3" .fenced,
   .append "b" 9 2 "urn:x" 2 "m4" .conflict,
   .append "b" 9 2 "urn:x" 3 "m2" .conflict,
   .frame "r" 9 2 "m2" true,
   .frame "r" 9 1 "m1" false,
   .ack "m1", .ack "m2"]

-- @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Stale owner append fails after split brain]] {level: unit}
def splitBrainAgrees : Bool :=
  match replay splitBrain with
  | .ok 8 => true
  | _ => false
#guard splitBrainAgrees

/-- A recorded decision the model would not make is reported. -/
def wrongDecision : Bool :=
  match replay (splitBrain.set 5 (.append "a" 9 1 "urn:x" 3 "m3" .accept)) with
  | .error e => e.startsWith "event 5"
  | .ok _ => false
#guard wrongDecision

/-- An epoch granted twice, an append under an epoch never granted, and a frame no accepted
append produced are histories the model cannot produce. -/
-- @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: unit}
def impossibleHistories : Bool :=
  (match replay [.grant "a" 1 5, .grant "b" 1 5] with | .error _ => true | .ok _ => false) &&
  (match replay [.append "a" 1 5 "urn:y" 1 "m" .accept] with | .error _ => true | .ok _ => false) &&
  (match replay [.grant "a" 1 5, .frame "r" 1 5 "m" true] with | .error _ => true | .ok _ => false)
#guard impossibleHistories

end Vertep.Lease.Test
