import VertepLean.CustodyCheck

/-!
# Custody

The append decision on its own, a failover history (`data/custody-failover.jsonl`, also the
sample for `lake exe vertep-custody --history`) replayed through the oracle, and histories it
must reject.
-/

namespace Vertep.Custody.Test

open Vertep.Custody Vertep.Custody.Check

/-! ## The append decision -/

/-- A custodian that accepted `[x1, x2]` in epoch 1 and promised 2. -/
def c1 : Cust := ⟨2, 1, ["x1", "x2"]⟩

-- below the promised epoch: refused
#guard (decideAppend c1 1 2 ["x1", "x2", "x3"]).1 = .refused
-- after its log end: behind
#guard (decideAppend c1 2 3 ["x1", "x2", "x3", "x4"]).1 = .behind
-- a newer epoch replaces the uncommitted tail beyond the agreeing prefix
#guard decideAppend c1 2 1 ["x1", "y2"] = (.ack, ⟨2, 2, ["x1", "y2"]⟩)
-- and drops what lies beyond the batch end
#guard decideAppend c1 2 1 ["x1"] = (.ack, ⟨2, 2, ["x1"]⟩)

/-- The same custodian once it accepted `[x1, y2]` in epoch 2. -/
def c2 : Cust := ⟨2, 2, ["x1", "y2"]⟩

-- same epoch: a batch extending the log is taken
#guard decideAppend c2 2 2 ["x1", "y2", "y3"] = (.ack, ⟨2, 2, ["x1", "y2", "y3"]⟩)
-- a delayed, shorter batch of the same epoch changes nothing
#guard decideAppend c2 2 0 ["x1"] = (.ack, c2)
-- a different entry of the same epoch: diverged, no change
#guard decideAppend c2 2 1 ["x1", "z2"] = (.diverged, c2)
-- a batch whose chain does not continue the log at `after`: failed (the link check)
#guard decideAppend c2 3 2 ["x1", "z2", "z3"] = (.failed, c2)
-- an empty batch of a newer epoch at a position where the chains differ: failed in the
-- model (Rust acknowledges it without a link check; its writer never sends it)
#guard decideAppend c2 3 2 ["x1", "z2"] = (.failed, c2)

/-- A refused prepare or a non-ack append changes nothing; an ack takes the epoch. -/
example : promises? c2 2 = false := rfl
example : promises? c2 3 = true := rfl

/-! ## Histories -/

def failover : String := include_str "data/custody-failover.jsonl"

def run (src : String) : Except String Nat := do
  let (cfg, es) ← parse src
  replay cfg es

/-- Replace line `n` (1-based) of a history. -/
def setLine (src : String) (n : Nat) (l : String) : String :=
  "\n".intercalate ((src.splitOn "\n").set (n - 1) l)

/-- Drop line `n` (1-based) of a history. -/
def dropLine (src : String) (n : Nat) : String :=
  "\n".intercalate ((src.splitOn "\n").eraseIdx (n - 1))

def rejectsAt (src : String) (n : Nat) : Bool :=
  match run src with
  | .error e => e.startsWith s!"line {n}:"
  | .ok _ => false

/-- Writer `h1` of epoch 1 commits `[x1]` and writes `x2` to one custodian; `h2` wins epoch 2
from the two others, adopts `[x1]`, replaces `a`'s uncommitted tail and commits `[x1, y2]`;
`h1`'s late batch is refused where epoch 2 was promised; a delayed short batch and a failed
batch change nothing. -/
-- @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Stale tail replaced]] {level: unit}
-- @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Uncommitted entry discarded]] {level: unit}
-- @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs#Epoch lost to a higher one]] {level: unit}
def failoverAgrees : Bool :=
  match run failover with
  | .ok 20 => true
  | _ => false
#guard failoverAgrees

/-- A recorded outcome the model would not decide. -/
def wrongOutcome : Bool :=
  rejectsAt (setLine failover 18
    "{\"kind\":\"accept\",\"custodian\":\"did:a\",\"writer\":\"did:h2\",\"epoch\":2,\"after\":1,\"entries\":[\"y2\"],\"outcome\":\"diverged\",\"log_after\":[\"x1\",\"x2\",\"x3\"],\"accepted_epoch_after\":1}") 18
#guard wrongOutcome

/-- A stale writer's batch acknowledged where a higher epoch was promised. -/
def staleAcked : Bool :=
  rejectsAt (setLine failover 17
    "{\"kind\":\"accept\",\"custodian\":\"did:b\",\"writer\":\"did:h1\",\"epoch\":1,\"after\":3,\"entries\":[\"x4\"],\"outcome\":\"ack\",\"log_after\":[\"x1\",\"x2\",\"x3\",\"x4\"],\"accepted_epoch_after\":1}") 17
#guard staleAcked

/-- An election that drops the committed entry: not the max rule's log. -/
-- @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Committed entry survives failover]] {level: unit}
def adoptionChecked : Bool :=
  rejectsAt (setLine failover 12
    "{\"kind\":\"elected\",\"host\":\"did:h2\",\"epoch\":2,\"promised_by\":[\"did:b\",\"did:c\"],\"adopted\":[]}") 12
#guard adoptionChecked

/-- An election without a quorum, and one without recorded promises (a second writer). -/
-- @tg: verifies:: [[openspec:object-custody#Failover Without A Shared Authority#No quorum reachable]] {level: unit}
def electionsBacked : Bool :=
  rejectsAt (setLine failover 12
    "{\"kind\":\"elected\",\"host\":\"did:h2\",\"epoch\":2,\"promised_by\":[\"did:c\"],\"adopted\":[]}") 12 &&
  rejectsAt (setLine failover 19
    "{\"kind\":\"elected\",\"host\":\"did:h3\",\"epoch\":2,\"promised_by\":[\"did:b\",\"did:c\"],\"adopted\":[\"x1\"]}") 19
#guard electionsBacked

/-- A commit announced before a quorum holds the log in the epoch. -/
def commitBacked : Bool :=
  rejectsAt (dropLine failover 6) 6
#guard commitBacked

/-- A batch from a host that did not win the epoch. -/
def writerElected : Bool :=
  rejectsAt (setLine failover 13
    "{\"kind\":\"accept\",\"custodian\":\"did:a\",\"writer\":\"did:h3\",\"epoch\":2,\"after\":0,\"entries\":[\"x1\"],\"outcome\":\"ack\",\"log_after\":[\"x1\"],\"accepted_epoch_after\":2}") 13
#guard writerElected

/-- A configuration whose quorum is not a majority is an input error. -/
def minorityQuorum : Bool :=
  match parse (setLine failover 1
    "{\"kind\":\"config\",\"custodians\":[\"did:a\",\"did:b\",\"did:c\",\"did:d\"],\"quorum\":2}") with
  | .error e => e.startsWith "config:"
  | .ok _ => false
-- @tg: verifies:: [[openspec:object-custody#Custodian Set In The DID Document#Quorum below a majority refused]] {level: unit}
#guard minorityQuorum

/-! ## The theorems, instantiated -/

/-- In a reachable state, two hosts elected in the same epoch are one. -/
example (cfg : Config) (hm : cfg.Majority) (s : Sys) (hr : Reachable cfg s)
    (h₁ h₂ : String) (A₁ A₂ : Log) (e₁ : (7, h₁, A₁) ∈ s.elections)
    (e₂ : (7, h₂, A₂) ∈ s.elections) : h₁ = h₂ :=
  one_writer_per_epoch cfg hm hr e₁ e₂

/-- A log committed in epoch 6 is in the log adopted in epoch 7. -/
example (cfg : Config) (hm : cfg.Majority) (s : Sys) (hr : Reachable cfg s) (L A : Log)
    (h : String) (hc : Committed cfg s L 6) (he : (7, h, A) ∈ s.elections) : L <+: A :=
  committed_survives_election cfg hm hr hc he (by decide)

end Vertep.Custody.Test
