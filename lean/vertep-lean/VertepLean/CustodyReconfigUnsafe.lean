import VertepLean.CustodyCheck

/-!
# The first custodian-set change rule, and why it is unsafe

An extension of the custody model (`VertepLean.Custody`) with configuration versions, as
`vertep_objects::custody` first implemented them (`CustodyHost::reconfigure`,
`CustodyHost::elect`, `Replica::append`), before the joint-consensus fix
(`VertepLean.CustodyJoint`). Kept as the record of why the fix was needed; Rust keeps both runs
below as regression tests (`shrink_keeps_committed_entries`,
`interrupted_change_keeps_one_writer`):

* The controller issues one configuration per version (`Confs`: version ↦ custodian set and
  quorum); versions change one at a time, consecutively.
* Every batch carries a configuration version. A custodian *fails* a batch whose version is
  below the one it accepted (no change), and on acknowledging a batch with a higher version
  adopts it (`decideR`).
* A promise reports the custodian's accepted version.
* A candidate prepares with a configuration it knows of (the genesis one, one in force at some
  writer, or one a promise reported); it is elected by promises of a quorum of that
  configuration, none of which reports a newer version (`Action.elect`). Rust re-prepares with
  the newer set when a promise reports one.
* The writer of an epoch proposes the next version (`Action.propose`), ships it with its
  batches, and switches to it once **a quorum of the old set and every newcomer** hold its
  journal with the new version (`switchReady`, Rust `reconfigure`).
* A log is committed in epoch `e` when a quorum of a configuration in force in `e` accepted in
  `e` logs extending it (`Committed`).

## The rule is unsafe

The fixed-set theorems (`VertepLean.CustodyTheorems`) do **not** extend to this rule; this file
proves so with two reachable states:

* `rust_switch_loses_committed`: shrinking `{a,b,c,d,e}` (quorum 3) to `{a,b,c}` (quorum 2)
  is accepted by the old quorum `{c,d,e}` alone (no newcomers), so the stale members `a`, `b`
  of the new set elect a host that adopts an empty log, losing a committed entry.
* `rust_pending_loses_committed`, `rust_pending_two_writers`: a candidate that sees a pending
  (never effective) version in one promise prepares with that set alone; the newcomers `d`, `f`
  elect it with an empty log, and the old set elects another host in the same epoch.

Both are the classical reason for joint consensus: a change must be accepted by quorums of
**both** the old and the new set before it takes effect, and an election while a change may be
pending must gather quorums of both sets.
-/

namespace Vertep.Custody.Unsafe

open Vertep.Custody

/-- The configurations the controller issued, by version. -/
abbrev Confs := Nat → Config

/-- A custodian: the fixed-set state and the configuration version it accepted. -/
structure RCust where
  core : Cust
  ver : Nat
  deriving DecidableEq, Repr

/-- A promise, with the configuration version the custodian reported. -/
structure RPromise where
  cust : String
  epoch : Nat
  host : String
  acc : Nat
  log : Log
  ver : Nat
  deriving DecidableEq, Repr

def RPromise.base (p : RPromise) : Promise := ⟨p.cust, p.epoch, p.host, p.acc, p.log⟩

/-- The system with configurations. -/
structure RSys where
  cust : String → RCust
  promises : List RPromise
  /-- Ghost: elections, as (epoch, host, configuration version prepared with, adopted log). -/
  elections : List (Nat × String × Nat × Log)
  wlog : Nat → String → Log
  /-- The configuration in force at the writer of (epoch, host). -/
  wver : Nat → String → Nat
  /-- The configuration it is changing to. -/
  wpend : Nat → String → Option Nat
  /-- Ghost: (epoch, version) for every configuration in force at a writer of the epoch. -/
  inForce : List (Nat × Nat)
  /-- Ghost: (custodian, epoch, batch version, log after the ack). -/
  accepted : List (String × Nat × Nat × Log)

/-- Fresh custodians at the genesis version `g`. -/
def RSys.empty (g : Nat) : RSys :=
  { cust := fun _ => ⟨Cust.empty, g⟩, promises := [], elections := [], wlog := fun _ _ => [],
    wver := fun _ _ => g, wpend := fun _ _ => none, inForce := [], accepted := [] }

/-- The custodian's decision on a batch of epoch `e` and configuration version `v` (Rust
`Replica::append`: a lower epoch is refused, an older configuration fails, then the
fixed-set decision; an ack adopts the batch's configuration). -/
def decideR (c : RCust) (e after v : Nat) (w : Log) : Outcome × RCust :=
  if e < c.core.promised then (.refused, c)
  else if v < c.ver then (.failed, c)
  else
    let r := decideAppend c.core e after w
    (r.1, ⟨r.2, if r.1 = .ack then max c.ver v else c.ver⟩)

/-- The election of `h` in `e`, as (configuration version, adopted log). -/
def elected? (s : RSys) (e : Nat) (h : String) : Option (Nat × Log) :=
  (s.elections.find? (fun (e', h', _, _) => e' = e ∧ h' = h)).map (fun (_, _, v, A) => (v, A))

/-- The version the writer ships: the pending one during a change. -/
def shipping (s : RSys) (e : Nat) (h : String) : Nat := (s.wpend e h).getD (s.wver e h)

def appendR (s : RSys) (c : String) (e after v : Nat) (w : Log) : Outcome × RSys :=
  let r := decideR (s.cust c) e after v w
  (r.1, { s with
    cust := upd s.cust c r.2
    accepted := if r.1 = .ack then s.accepted ++ [(c, e, v, r.2.core.log)] else s.accepted })

/-- A configuration version a candidate can know of. -/
def known (g : Nat) (s : RSys) (v : Nat) : Bool :=
  v == g || s.promises.any (·.ver == v) || s.inForce.any (·.2 == v)

/-- Custodian `c` holds the journal of the writer `h` of `e` with version `v`. -/
def holds (s : RSys) (e v : Nat) (h c : String) : Bool :=
  (s.cust c).core.acc == e && (s.cust c).ver == v && decide (s.wlog e h <+: (s.cust c).core.log)

/-- The Rust switch rule: a quorum of the old set and every newcomer hold the journal with the
pending version. -/
def switchReady (conf : Confs) (s : RSys) (e : Nat) (h : String) : Option Nat :=
  match s.wpend e h with
  | some v' =>
    let old := conf (s.wver e h)
    if old.quorum ≤ (old.custodians.filter (holds s e v' h)).length &&
        ((conf v').custodians.filter (· ∉ old.custodians)).all (holds s e v' h) then some v'
    else none
  | none => none

/-- The promises of `qs` for `e` to `h`. -/
def promisesOf (s : RSys) (e : Nat) (h : String) (qs : List String) : List RPromise :=
  qs.filterMap (fun c => s.promises.find? (fun p => p.cust = c ∧ p.epoch = e ∧ p.host = h))

/-- The moves of the system. -/
inductive Action where
  | prepare (c : String) (e : Nat) (h : String)
  | elect (h : String) (e v : Nat) (qs : List String) (A : Log)
  | write (h : String) (e : Nat) (xs : Log)
  | propose (h : String) (e : Nat)
  | append (c h : String) (e after v : Nat) (w : Log)
  | switch (h : String) (e : Nat)
  deriving Repr

/-- A move, when it is enabled. -/
def exec (conf : Confs) (g : Nat) (s : RSys) : Action → Option RSys
  | .prepare c e h =>
    if (s.cust c).core.promised < e then
      some { s with
        cust := upd s.cust c ⟨{ (s.cust c).core with promised := e }, (s.cust c).ver⟩
        promises := s.promises ++
          [⟨c, e, h, (s.cust c).core.acc, (s.cust c).core.log, (s.cust c).ver⟩] }
    else none
  | .elect h e v qs A =>
    let ps := promisesOf s e h qs
    if known g s v && decide (Quorum (conf v) qs) && ps.length == qs.length &&
        ps.all (·.ver ≤ v) && Check.adoptsB (ps.map RPromise.base) A &&
        (elected? s e h).isNone then
      some { s with
        elections := s.elections ++ [(e, h, v, A)]
        wlog := upd2 s.wlog e h A
        wver := upd2 s.wver e h v
        inForce := s.inForce ++ [(e, v)] }
    else none
  | .write h e xs =>
    if (elected? s e h).isSome then some { s with wlog := upd2 s.wlog e h (s.wlog e h ++ xs) }
    else none
  | .propose h e =>
    if (elected? s e h).isSome && (s.wpend e h).isNone then
      some { s with wpend := upd2 s.wpend e h (some (s.wver e h + 1)) }
    else none
  | .append c h e after v w =>
    match elected? s e h with
    | some (v₀, A) =>
      if decide (A <+: w) && decide (w <+: s.wlog e h) && decide (after ≤ w.length) &&
          decide (v₀ ≤ v) && decide (v ≤ shipping s e h) then
        some (appendR s c e after v w).2
      else none
    | none => none
  | .switch h e =>
    match switchReady conf s e h with
    | some v' => some { s with
        wver := upd2 s.wver e h v'
        wpend := upd2 s.wpend e h none
        inForce := s.inForce ++ [(e, v')] }
    | none => none

/-- The states reachable from fresh custodians at the genesis version `g`. -/
inductive Reachable (conf : Confs) (g : Nat) : RSys → Prop
  | empty : Reachable conf g (RSys.empty g)
  | step {s t : RSys} (a : Action) : Reachable conf g s → exec conf g s a = some t →
      Reachable conf g t

/-- Run moves in order; `none` when one is not enabled. -/
def run (conf : Confs) (g : Nat) (as : List Action) (s : RSys) : Option RSys :=
  as.foldlM (exec conf g) s

theorem run_reachable (conf : Confs) (g : Nat) :
    ∀ (as : List Action) {s t : RSys}, Reachable conf g s → run conf g as s = some t →
      Reachable conf g t
  | [], s, t, hs, h => by simp only [run, List.foldlM, Option.pure_def, Option.some.injEq] at h; exact h ▸ hs
  | a :: as, s, t, hs, h => by
    simp only [run, List.foldlM] at h
    cases hx : exec conf g s a with
    | none => rw [hx] at h; cases h
    | some s' =>
      rw [hx] at h
      exact run_reachable conf g as (Reachable.step a hs hx) h

/-- `L` is committed in `e`: a quorum of a configuration in force in `e` accepted in `e` logs
extending `L`. -/
def Committed (conf : Confs) (s : RSys) (L : Log) (e : Nat) : Prop :=
  ∃ v, (e, v) ∈ s.inForce ∧ ∃ Q, Quorum (conf v) Q ∧
    ∀ c ∈ Q, ∃ v' l, (c, e, v', l) ∈ s.accepted ∧ L <+: l

/-- Some reachable state loses a committed log: a later election adopts a log without it. -/
def LosesCommitted (conf : Confs) (g : Nat) : Prop :=
  ∃ s, Reachable conf g s ∧ ∃ L e₀ e h v A, Committed conf s L e₀ ∧
    (e, h, v, A) ∈ s.elections ∧ e₀ < e ∧ ¬ L <+: A

/-- Some reachable state has two writers in one epoch. -/
def TwoWriters (conf : Confs) (g : Nat) : Prop :=
  ∃ s, Reachable conf g s ∧ ∃ e h₁ h₂ v₁ v₂ A₁ A₂, (e, h₁, v₁, A₁) ∈ s.elections ∧
    (e, h₂, v₂, A₂) ∈ s.elections ∧ h₁ ≠ h₂

/-! ## Counterexample 1: a shrink accepted by the old quorum alone -/

/-- Version 0: `{a,b,c,d,e}`, quorum 3; version 1: `{a,b,c}`, quorum 2. -/
def shrink : Confs := fun v =>
  if v = 0 then ⟨["a", "b", "c", "d", "e"], 3⟩ else ⟨["a", "b", "c"], 2⟩

/-- `h1` wins epoch 1 from `{c,d,e}` and commits `[x]`; `h2` wins epoch 2 from `{c,d,e}`,
proposes version 1 and ships it to `{c,d,e}`, which is a quorum of the old set (and there are
no newcomers), so it switches; `h3` wins epoch 3 with version 1 from the stale `{a,b}`. -/
def shrinkRun : List Action :=
  [.prepare "c" 1 "h1", .prepare "d" 1 "h1", .prepare "e" 1 "h1",
   .elect "h1" 1 0 ["c", "d", "e"] [],
   .write "h1" 1 ["x"],
   .append "c" "h1" 1 0 0 ["x"], .append "d" "h1" 1 0 0 ["x"], .append "e" "h1" 1 0 0 ["x"],
   .prepare "c" 2 "h2", .prepare "d" 2 "h2", .prepare "e" 2 "h2",
   .elect "h2" 2 0 ["c", "d", "e"] ["x"],
   .propose "h2" 2,
   .append "c" "h2" 2 0 1 ["x"], .append "d" "h2" 2 0 1 ["x"], .append "e" "h2" 2 0 1 ["x"],
   .switch "h2" 2,
   .prepare "a" 3 "h3", .prepare "b" 3 "h3",
   .elect "h3" 3 1 ["a", "b"] []]

theorem shrinkRun_some : (run shrink 0 shrinkRun (RSys.empty 0)).isSome = true := by decide

def shrinkState : RSys := (run shrink 0 shrinkRun (RSys.empty 0)).get shrinkRun_some

theorem shrink_majority : ∀ v, (shrink v).Majority := by
  intro v; unfold shrink Config.Majority; split <;> decide

/-- **The Rust switch rule loses committed entries.** -/
theorem rust_switch_loses_committed : LosesCommitted shrink 0 := by
  refine ⟨shrinkState,
    run_reachable shrink 0 shrinkRun Reachable.empty (Option.some_get _).symm,
    ["x"], 1, 3, "h3", 1, [], ?_, by decide, by decide, by decide⟩
  refine ⟨0, by decide, ["c", "d", "e"], by decide, fun c hc => ?_⟩
  simp only [List.mem_cons, List.not_mem_nil, or_false] at hc
  rcases hc with rfl | rfl | rfl <;> exact ⟨0, ["x"], by decide, List.prefix_refl _⟩

/-! ## Counterexample 2: an election with a pending configuration alone -/

/-- Version 0: `{a,b,c}`; version 1: `{c,d,f}`; quorum 2. -/
def swap : Confs := fun v =>
  if v = 0 then ⟨["a", "b", "c"], 2⟩ else ⟨["c", "d", "f"], 2⟩

/-- `h1` wins epoch 1 from `{a,b}` and commits `[x]`, proposes version 1 and ships it to `c`
only, then stops. `h2` prepares epoch 2 with version 0, sees version 1 in `c`'s promise,
prepares epoch 3 with version 1 and is elected by the newcomers `{d,f}`, adopting an empty
log. `h3` is elected in epoch 3 too, with version 0, by `{a,b}`. -/
def swapRun : List Action :=
  [.prepare "a" 1 "h1", .prepare "b" 1 "h1",
   .elect "h1" 1 0 ["a", "b"] [],
   .write "h1" 1 ["x"],
   .append "a" "h1" 1 0 0 ["x"], .append "b" "h1" 1 0 0 ["x"],
   .propose "h1" 1,
   .append "c" "h1" 1 0 1 ["x"],
   .prepare "b" 2 "h2", .prepare "c" 2 "h2",
   .prepare "d" 3 "h2", .prepare "f" 3 "h2",
   .elect "h2" 3 1 ["d", "f"] [],
   .prepare "a" 3 "h3", .prepare "b" 3 "h3",
   .elect "h3" 3 0 ["a", "b"] ["x"]]

theorem swapRun_some : (run swap 0 swapRun (RSys.empty 0)).isSome = true := by decide

def swapState : RSys := (run swap 0 swapRun (RSys.empty 0)).get swapRun_some

theorem swap_majority : ∀ v, (swap v).Majority := by
  intro v; unfold swap Config.Majority; split <;> decide

theorem swap_reachable : Reachable swap 0 swapState :=
  run_reachable swap 0 swapRun Reachable.empty (Option.some_get _).symm

/-- **Electing with a pending configuration alone loses committed entries.** -/
theorem rust_pending_loses_committed : LosesCommitted swap 0 := by
  refine ⟨swapState, swap_reachable, ["x"], 1, 3, "h2", 1, [], ?_, by decide, by decide,
    by decide⟩
  refine ⟨0, by decide, ["a", "b"], by decide, fun c hc => ?_⟩
  simp only [List.mem_cons, List.not_mem_nil, or_false] at hc
  rcases hc with rfl | rfl <;> exact ⟨0, ["x"], by decide, List.prefix_refl _⟩

/-- **…and elects two writers in one epoch.** -/
theorem rust_pending_two_writers : TwoWriters swap 0 :=
  ⟨swapState, swap_reachable, 3, "h2", "h3", 1, 0, [], ["x"], by decide, by decide, by decide⟩

end Vertep.Custody.Unsafe

