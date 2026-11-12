import VertepLean.CustodyJointCheck
import VertepLean.CustodyReconfigUnsafe

/-!
# Custodian-set changes

The joint-consensus append decision, a reconfiguration history
(`data/custody-joint.jsonl`) replayed through the oracle and histories it must reject, and a
seeded random search of both reconfiguration models: no violation of one writer per epoch or of
committed-log survival in the joint-consensus model, and violations found in the earlier rule's
model (which `VertepLean.CustodyReconfigUnsafe` also proves outright).
-/

namespace Vertep.Custody.JointTest

open Vertep.Custody Vertep.Custody.Joint Vertep.Custody.JointCheck

/-! ## The append decision with records -/

/-- A custodian at version 2 (rank 4) that accepted `[x1]` in epoch 1. -/
def c4 : JCust := ⟨⟨1, 1, ["x1"]⟩, 4⟩

-- an older configuration version fails
#guard (decideJ c4 2 1 2 ["x1", "x2"]).1 = .failed
-- a lower record within the accepted epoch fails
#guard (decideJ ⟨⟨1, 1, ["x1"]⟩, 5⟩ 1 1 4 ["x1", "x2"]).1 = .failed
-- from a newer epoch, the same version without the pending change is taken, and so is its record
#guard decideJ ⟨⟨1, 1, ["x1"]⟩, 5⟩ 2 1 4 ["x1", "x2"] = (.ack, ⟨⟨2, 2, ["x1", "x2"]⟩, 4⟩)
-- a newer record is taken
#guard decideJ c4 1 1 5 ["x1", "x2"] = (.ack, ⟨⟨1, 1, ["x1", "x2"]⟩, 5⟩)
-- a lower epoch is refused first
#guard (decideJ ⟨⟨3, 1, ["x1"]⟩, 4⟩ 2 1 2 ["x1"]).1 = .refused

/-! ## Histories -/

def joint : String := include_str "data/custody-joint.jsonl"

def runJ (src : String) : Except String Nat :=
  match replayAny src with
  | .ok n => .ok n
  | .error (_, e) => .error e

def setLine (src : String) (n : Nat) (l : String) : String :=
  "\n".intercalate ((src.splitOn "\n").set (n - 1) l)

def dropLine (src : String) (n : Nat) : String :=
  "\n".intercalate ((src.splitOn "\n").eraseIdx (n - 1))

def rejectsAt (src : String) (n : Nat) (part : String) : Bool :=
  match runJ src with
  | .error e => e.startsWith s!"line {n}:" && (e.splitOn part).length > 1
  | .ok _ => false

/-- `a` wins epoch 1 under `{a,b,c}`, commits `[x1]`, moves to the joint record with `{b,c,d}`
(the newcomer `d` takes the log), switches once `{a,b}` and `{b,d}` hold it, and commits
`[x1, x2]` under `{b,c,d}`; `c` then wins epoch 2 under `{b,c,d}` and keeps both entries. -/
-- @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Replace a keeper]] {level: unit}
def jointAgrees : Bool :=
  match runJ joint with
  | .ok 22 => true
  | _ => false
#guard jointAgrees

/-- Switching before a quorum of the old set holds the log with the joint record. -/
def switchTooEarly : Bool :=
  rejectsAt (dropLine joint 9) 11 "before quorums of both sets"
#guard switchTooEarly

/-- An election under the old set alone, once the new set is in force. -/
-- @tg: verifies:: [[openspec:object-custody#Custodian Set Changes#Old set cannot elect after a change]] {level: unit}
def oldSetCannotElect : Bool :=
  rejectsAt (setLine joint 19
    "{\"kind\":\"elected\",\"host\":\"c\",\"epoch\":2,\"promised_by\":[\"a\",\"b\",\"c\"],\"adopted\":[\"x1\",\"x2\"],\"config\":{\"version\":1,\"custodians\":[\"a\",\"b\",\"c\"],\"quorum\":2},\"next\":null}") 19 "elected in epoch 2"
#guard oldSetCannotElect

/-- A commit under the new set that only the old set backs. -/
def commitBacked : Bool :=
  rejectsAt (dropLine joint 13) 14 "held in that epoch"
#guard commitBacked

/-- A batch with an older configuration acknowledged. -/
def olderConfigAcked : Bool :=
  rejectsAt (setLine joint 21
    "{\"kind\":\"accept\",\"custodian\":\"d\",\"writer\":\"c\",\"epoch\":2,\"after\":2,\"entries\":[],\"outcome\":\"ack\",\"log_after\":[\"x1\",\"x2\"],\"accepted_epoch_after\":2,\"config\":{\"version\":1,\"custodians\":[\"a\",\"b\",\"c\"],\"quorum\":2},\"next\":null,\"config_after\":{\"version\":1,\"custodians\":[\"a\",\"b\",\"c\"],\"quorum\":2},\"next_after\":null}") 21 "epoch 2"
#guard olderConfigAcked

/-- One version naming two custodian sets is an input error. -/
def versionsAgree : Bool :=
  match replayAny (setLine joint 2
    "{\"kind\":\"prepare\",\"custodian\":\"a\",\"candidate\":\"a\",\"epoch\":1,\"promised\":true,\"highest_before\":0,\"accepted_epoch\":0,\"log\":[],\"config\":{\"version\":1,\"custodians\":[\"a\",\"b\",\"d\"],\"quorum\":2},\"next\":null}") with
  | .error (true, _) => true
  | _ => false
#guard versionsAgree

/-- Histories without records still replay through the fixed-set model. -/
def oldFormat : Bool :=
  match replayAny (include_str "data/custody-failover.jsonl") with
  | .ok 20 => true
  | _ => false
#guard oldFormat

/-! ## Random search

A seeded search drives either reconfiguration model through random campaigns (prepare the
record's custodians, re-prepare with a higher record a promise reports, elect with whoever
promised), writes, shipments (including delayed ones under older records), proposals and
switches, with random message loss, and checks one writer per epoch and that every committed log
is in every later adopted and accepted log. -/

/-- A move, common to both models. -/
inductive Mv where
  | prepare (c : String) (e : Nat) (h : String)
  | elect (h : String) (e x : Nat) (qs : List String) (A : Log)
  | write (h : String) (e : Nat) (xs : Log)
  | propose (h : String) (e : Nat)
  | append (c h : String) (e after x : Nat) (w : Log)
  | switch (h : String) (e : Nat)

/-- What the search needs from a model. -/
structure Sim (σ : Type) where
  init : σ
  exec : σ → Mv → Option σ
  /-- (promised, accepted epoch, log, record) of a custodian. -/
  view : σ → String → Nat × Nat × Log × Nat
  wlog : σ → Nat → String → Log
  /-- The record the writer ships with. -/
  wrec : σ → Nat → String → Nat
  elections : σ → List (Nat × String × Nat × Log)
  accepted : σ → List (String × Nat × Nat × Log)
  inForce : σ → List (Nat × Nat)
  /-- The custodians a candidate prepares for a record. -/
  members : Nat → List String
  /-- A quorum of a record, for commits. -/
  quorum : Nat → List String → Bool
  /-- The highest record. -/
  top : Nat

/-- A linear congruential generator. -/
def next (seed : Nat) : Nat := (seed * 6364136223846793005 + 1442695040888963407) % 2 ^ 64

def pick {α : Type} (seed : Nat) (xs : List α) (d : α) : α := xs.getD ((seed / 65536) % (xs.length.max 1)) d

def hosts : List String := ["a", "b", "c"]
def all : List String := ["a", "b", "c", "d", "f"]

/-- The best promise by the max rule. -/
def bestLog (ps : List (Nat × Log)) : Log :=
  (ps.foldl (fun (b : Option (Nat × Log)) p => match b with
    | none => some p
    | some b => if b.1 < p.1 ∨ (b.1 = p.1 ∧ b.2.length < p.2.length) then some p else some b)
    none).elim [] (·.2)

/-- A campaign of `h` at epoch `e` with record `x`: prepare, maybe again higher, elect. -/
partial def campaign {σ : Type} (m : Sim σ) (s : σ) (seed : Nat) (h : String) (e x : Nat)
    (rounds : Nat) : σ × Nat :=
  let (s, seed, ps) := (m.members x).foldl (fun (s, seed, ps) c =>
    let seed := next seed
    if seed % 5 = 0 then (s, seed, ps)
    else
      let v := m.view s c
      match m.exec s (.prepare c e h) with
      | some t => (t, seed, ps ++ [(c, v.2.1, v.2.2.1, v.2.2.2)])
      | none => (s, seed, ps)) (s, seed, [])
  let hi := ps.foldl (fun a p => max a p.2.2.2) x
  if x < hi && rounds > 0 then campaign m s seed h (e + 1) hi (rounds - 1)
  else
    let A := bestLog (ps.map (fun p => (p.2.1, p.2.2.1)))
    match m.exec s (.elect h e x (ps.map (·.1)) A) with
    | some t => (t, seed)
    | none => (s, seed)

/-- The first violation in a state, if any. -/
def violation {σ : Type} (m : Sim σ) (s : σ) : Option String :=
  let els := m.elections s
  let acc := m.accepted s
  let two := els.find? (fun (e, h, _, _) => els.any (fun (e', h', _, _) => e = e' ∧ h ≠ h'))
  match two with
  | some (e, _, _, _) => some s!"two writers in epoch {e}"
  | none =>
    -- committed logs: for each record in force, the accepted logs a quorum of it extends
    (m.inForce s).findSome? (fun (e₀, x) =>
      let logs := acc.filter (fun (_, e, x', _) => e = e₀ ∧ x' = x)
      logs.findSome? (fun (_, _, _, L) =>
        let holders := (all.filter (fun c => logs.any (fun (c', _, _, l) => c' = c ∧ L <+: l)))
        if m.quorum x holders then
          if els.any (fun (e, _, _, A) => e₀ < e ∧ !(L <+: A)) then
            some s!"a log committed in epoch {e₀} missing from a later adoption"
          else if acc.any (fun (_, e, _, l) => e₀ < e ∧ !(L <+: l)) then
            some s!"a log committed in epoch {e₀} missing from a later acceptance"
          else none
        else none))

/-- One random run of `n` steps: the first violation found. -/
def runOnce {σ : Type} (m : Sim σ) (seed0 n : Nat) : Option String := Id.run do
  let mut s := m.init
  let mut seed := next (seed0 + 1)
  let mut fresh := 0
  for _ in [0:n] do
    seed := next seed
    let els := m.elections s
    let maxE := els.foldl (fun a (e, _, _, _) => max a e) 0
    let k := seed % 10
    if k < 2 then
      seed := next seed
      let h := pick seed hosts "a"
      seed := next seed
      let x := (pick seed (List.range (m.top + 1)) 0).min
        ((els.foldl (fun a (_, _, x, _) => max a x) 0) + 1)
      let e := maxE + 1 + seed % 2
      let r := campaign m s seed h e x 2
      s := r.1; seed := r.2
    else if els.isEmpty then pure ()
    else
      seed := next seed
      let (e, h, _, _) := pick seed els (0, "a", 0, [])
      if k < 4 then
        fresh := fresh + 1
        if let some t := m.exec s (.write h e [s!"x{fresh}"]) then s := t
      else if k < 6 then
        seed := next seed
        let c := pick seed all "a"
        let w := m.wlog s e h
        seed := next seed
        let w := if seed % 4 = 0 then w.take (w.length - 1) else w
        let held := (m.view s c).2.2.1.length
        seed := next seed
        let after := if seed % 3 = 0 then 0 else min held w.length
        seed := next seed
        let x := m.wrec s e h
        let x := if seed % 5 = 0 && x > 0 then x - 1 else x
        if let some t := m.exec s (.append c h e after x w) then s := t
      else if k < 8 then
        -- the writer brings every custodian of its record level, losing some messages
        let w := m.wlog s e h
        let x := m.wrec s e h
        for c in m.members x do
          seed := next seed
          if seed % 5 ≠ 0 then
            let after := min (m.view s c).2.2.1.length w.length
            if let some t := m.exec s (.append c h e after x w) then s := t
      else if k < 9 then
        if let some t := m.exec s (.propose h e) then s := t
      else
        if let some t := m.exec s (.switch h e) then s := t
    if let some v := violation m s then return some v
  return none

/-- The first seed below `runs` whose run violates safety. -/
def search {σ : Type} (m : Sim σ) (runs n : Nat) : Option (Nat × String) :=
  (List.range runs).findSome? (fun i => (runOnce m (i * 7919) n).map (fun v => (i, v)))

/-- Configurations for the search: versions 0 … 3. -/
def confs : Nat → Config
  | 0 => ⟨["a", "b", "c"], 2⟩
  | 1 => ⟨["c", "d", "f"], 2⟩
  | 2 => ⟨["a", "b", "c", "d", "f"], 3⟩
  | _ => ⟨["a", "d"], 2⟩

/-- The joint-consensus model (records are ranks, 0 … 7). -/
def jointSim : Sim JSys where
  init := JSys.empty 0
  exec s
    | .prepare c e h => exec confs 0 s (.prepare c e h)
    | .elect h e x qs A => exec confs 0 s (.elect h e x qs A)
    | .write h e xs => exec confs 0 s (.write h e xs)
    | .propose h e =>
      if verOf (s.wrank e h) < 3 then exec confs 0 s (.propose h e) else none
    | .append c h e after x w => exec confs 0 s (.append c h e after x w)
    | .switch h e => exec confs 0 s (.switch h e)
  view s c := ((s.cust c).core.promised, (s.cust c).core.acc, (s.cust c).core.log, (s.cust c).rank)
  wlog s := s.wlog
  wrec s := s.wrank
  elections s := s.elections
  accepted s := s.accepted
  inForce s := s.inForce
  members x := ((confs (verOf x)).custodians ++
    (if isJoint x then (confs (verOf x + 1)).custodians else [])).eraseDups
  quorum x Q := decide (RQuorum confs x Q)
  top := 6

/-- The earlier rule's model (records are versions, 0 … 3). -/
def unsafeSim : Sim Unsafe.RSys where
  init := Unsafe.RSys.empty 0
  exec s
    | .prepare c e h => Unsafe.exec confs 0 s (.prepare c e h)
    | .elect h e x qs A => Unsafe.exec confs 0 s (.elect h e x qs A)
    | .write h e xs => Unsafe.exec confs 0 s (.write h e xs)
    | .propose h e =>
      if s.wver e h < 3 then Unsafe.exec confs 0 s (.propose h e) else none
    | .append c h e after x w => Unsafe.exec confs 0 s (.append c h e after x w)
    | .switch h e => Unsafe.exec confs 0 s (.switch h e)
  view s c := ((s.cust c).core.promised, (s.cust c).core.acc, (s.cust c).core.log, (s.cust c).ver)
  wlog s := s.wlog
  wrec s e h := Unsafe.shipping s e h
  elections s := s.elections
  accepted s := s.accepted
  inForce s := s.inForce
  members x := (confs x).custodians
  quorum x Q := decide (Quorum (confs x) Q)
  top := 3

/-- The joint-consensus model keeps both properties on every run searched. -/
def jointSafe : Bool := (search jointSim 400 120).isNone
#guard jointSafe

/-- The same search breaks the earlier rule. -/
def unsafeFound : Bool := (search unsafeSim 400 120).isSome
#guard unsafeFound

/-! ## The theorems, instantiated -/

/-- Without a change, one writer per epoch. -/
example (conf : Confs) (g : Nat) (hm : (conf g).Majority) (s : JSys)
    (hr : ReachableNC conf g s) (h₁ h₂ : String) (r₁ r₂ : Nat) (A₁ A₂ : Log)
    (e₁ : (7, h₁, r₁, A₁) ∈ s.elections) (e₂ : (7, h₂, r₂, A₂) ∈ s.elections) : h₁ = h₂ :=
  one_writer_per_epoch_noChange conf g hm hr e₁ e₂

/-- The earlier rule is unsafe. -/
example : Unsafe.LosesCommitted Unsafe.shrink 0 := Unsafe.rust_switch_loses_committed

end Vertep.Custody.JointTest
