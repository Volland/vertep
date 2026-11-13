import VertepLean.CustodyTheorems
import VertepLean.CustodyCheck

/-!
# Custodian-set changes by joint consensus (m7 10.1)

The custody model (`VertepLean.Custody`) extended with configuration changes as
`vertep_objects::custody` makes them after the joint-consensus fix (the earlier rule is unsafe:
`VertepLean.CustodyReconfigUnsafe`).

* The controller issues one configuration per version (`Confs`); versions change one at a time.
* Custody state carries a **record**: the effective configuration `C_v` and, during a change,
  the pending `C_{v+1}`. Records are totally ordered by version, joint above single at equal
  version, so a record is a **rank** `r`: `2v` for `C_v` alone, `2v+1` for `C_v` joint with
  `C_{v+1}`. A quorum of a rank (`RQuorum`) is a quorum of `C_v`, and for a joint rank also a
  quorum of `C_{v+1}`.
* A batch carries the writer's record. A custodian fails a batch whose version is below its own,
  or whose record is below its own within the epoch it accepted in (no change); on
  acknowledging it takes the batch's record (`decideJ`). A promise reports the
  custodian's record.
* The writer of an epoch changes `C_v → C_{v+1}` in two phases: it moves to the joint rank at
  will (`Action.propose`); it moves to `C_{v+1}` alone (`Action.switch`) once quorums of both
  sets hold its whole log with the joint record in its epoch.
* A candidate is elected with a record it knows of (the genesis one, one a promise reported, one
  in force at a writer, or a change pending on one of those) by the promises of a quorum of
  that rank, none of which reports a higher record; it adopts by the max rule and continues
  with that record.
* A log is committed in epoch `e` under a rank in force in `e` when a quorum of that rank
  accepted, in `e` and with batches of that rank, logs extending it (`Committed`).

## What is proven

* `rquorum_inter`: quorums of equal or consecutive ranks intersect, when every configuration's
  quorum is a majority. This is the property joint consensus rests on: `C_v` alone, `C_v` with
  `C_{v+1}`, and `C_{v+1}` alone pairwise share a configuration.
* `exec_noChange_project`: as long as no change is started, every move of this model is a
  move of the fixed-set model on the projected state (`project`), so `reachableNC_project`
  carries the fixed-set theorems to reconfiguration-free runs:
  `one_writer_per_epoch_noChange`, `committed_survives_noChange`.

The safety of runs that do change the custodian set (`OneWriterPerEpoch`,
`CommittedSurvives` below) is **stated, not proven**: `VertepTests.CustodyJoint` checks both
on many seeded random runs (and the same search finds the counterexamples of the earlier rule),
and the oracle checks them along every recorded history.
-/

namespace Vertep.Custody.Joint

open Vertep.Custody

/-- The configurations the controller issued, by version. -/
abbrev Confs := Nat → Config

/-- The version of a rank. -/
def verOf (r : Nat) : Nat := r / 2

/-- Whether a rank is joint (a change pending). -/
def isJoint (r : Nat) : Bool := r % 2 == 1

/-- At least the quorum of `C` among `Q`. -/
def QuorumIn (C : Config) (Q : List String) : Prop :=
  C.quorum ≤ (Q.filter (fun c => decide (c ∈ C.custodians))).length

instance (C : Config) (Q : List String) : Decidable (QuorumIn C Q) := by
  unfold QuorumIn; exact inferInstance

/-- `Q` (distinct custodians) holds a quorum of rank `r`: of `C_v`, and of `C_{v+1}` when
joint. -/
def RQuorum (conf : Confs) (r : Nat) (Q : List String) : Prop :=
  Q.Nodup ∧ QuorumIn (conf (verOf r)) Q ∧ (isJoint r = true → QuorumIn (conf (verOf r + 1)) Q) ∧
    ∀ c ∈ Q, c ∈ (conf (verOf r)).custodians ∨
      (isJoint r = true ∧ c ∈ (conf (verOf r + 1)).custodians)

instance (conf : Confs) (r : Nat) (Q : List String) : Decidable (RQuorum conf r Q) := by
  unfold RQuorum; exact inferInstance

/-- Every configuration's quorum is a majority. -/
def Majorities (conf : Confs) : Prop := ∀ v, (conf v).Majority

theorem quorum_of_quorumIn {C : Config} {Q : List String} (hn : Q.Nodup) (h : QuorumIn C Q) :
    Quorum C (Q.filter (fun c => decide (c ∈ C.custodians))) :=
  ⟨hn.sublist List.filter_sublist,
    fun c hc => by simpa using (List.mem_filter.mp hc).2, h⟩

/-- Two lists with quorums of the same configuration intersect. -/
theorem quorumIn_inter {C : Config} (hm : C.Majority) {Q Q' : List String} (hn : Q.Nodup)
    (hn' : Q'.Nodup) (h : QuorumIn C Q) (h' : QuorumIn C Q') : ∃ c, c ∈ Q ∧ c ∈ Q' := by
  obtain ⟨c, hc, hc'⟩ := quorum_inter C hm (quorum_of_quorumIn hn h) (quorum_of_quorumIn hn' h')
  exact ⟨c, (List.mem_filter.mp hc).1, (List.mem_filter.mp hc').1⟩

theorem rquorum_inter_succ (conf : Confs) (hm : Majorities conf) {r : Nat} {Q Q' : List String}
    (h : RQuorum conf r Q) (h' : RQuorum conf (r + 1) Q') : ∃ c, c ∈ Q ∧ c ∈ Q' := by
  obtain ⟨hn, h1, h2, -⟩ := h
  obtain ⟨hn', h1', h2', -⟩ := h'
  by_cases hr : r % 2 = 1
  · -- `r = 2v+1` holds `C_{v+1}`, and `r+1 = 2(v+1)` is `C_{v+1}`
    have hv : verOf (r + 1) = verOf r + 1 := by unfold verOf; omega
    rw [hv] at h1'
    exact quorumIn_inter (hm _) hn hn' (h2 (by simp [isJoint, hr])) h1'
  · -- `r = 2v` is `C_v`, and `r+1 = 2v+1` holds `C_v`
    have hv : verOf (r + 1) = verOf r := by unfold verOf; omega
    rw [hv] at h1'
    exact quorumIn_inter (hm _) hn hn' h1 h1'

/-- **Joint quorums intersect.** Quorums of equal or consecutive ranks share a custodian. -/
theorem rquorum_inter (conf : Confs) (hm : Majorities conf) {r r' : Nat} {Q Q' : List String}
    (h : RQuorum conf r Q) (h' : RQuorum conf r' Q') (h1 : r' ≤ r + 1) (h2 : r ≤ r' + 1) :
    ∃ c, c ∈ Q ∧ c ∈ Q' := by
  rcases Nat.lt_trichotomy r r' with hl | rfl | hl
  · have : r' = r + 1 := by omega
    subst this; exact rquorum_inter_succ conf hm h h'
  · exact quorumIn_inter (hm _) h.1 h'.1 h.2.1 h'.2.1
  · have : r = r' + 1 := by omega
    subst this
    obtain ⟨c, h₁, h₂⟩ := rquorum_inter_succ conf hm h' h
    exact ⟨c, h₂, h₁⟩

/-! ## The model -/

/-- A custodian: the fixed-set state and its record's rank. -/
structure JCust where
  core : Cust
  rank : Nat
  deriving DecidableEq, Repr

/-- A promise, with the rank of the record the custodian reported. -/
structure JPromise where
  cust : String
  epoch : Nat
  host : String
  acc : Nat
  log : Log
  rank : Nat
  deriving DecidableEq, Repr

def JPromise.base (p : JPromise) : Promise := ⟨p.cust, p.epoch, p.host, p.acc, p.log⟩

/-- The system with configuration records. -/
structure JSys where
  cust : String → JCust
  promises : List JPromise
  /-- Ghost: elections, as (epoch, host, rank elected under, adopted log). -/
  elections : List (Nat × String × Nat × Log)
  wlog : Nat → String → Log
  /-- The rank of the writer of (epoch, host). -/
  wrank : Nat → String → Nat
  /-- Ghost: (epoch, rank) for every record in force at a writer of the epoch. -/
  inForce : List (Nat × Nat)
  /-- Ghost: (custodian, epoch, batch rank, log after the ack). -/
  accepted : List (String × Nat × Nat × Log)

/-- Fresh custodians at the genesis version `g` (rank `2g`). -/
def JSys.empty (g : Nat) : JSys :=
  { cust := fun _ => ⟨Cust.empty, 2 * g⟩, promises := [], elections := [],
    wlog := fun _ _ => [], wrank := fun _ _ => 2 * g, inForce := [], accepted := [] }

/-- The custodian's decision on a batch of epoch `e` and rank `r` (Rust `Replica::append`): a
lower epoch is refused; an older configuration version fails, and so does a lower record within
the accepted epoch; then the fixed-set decision. An ack takes the batch's record (its
configuration has a version at least the custodian's, and its pending configuration replaces
the custodian's). -/
def decideJ (c : JCust) (e after r : Nat) (w : Log) : Outcome × JCust :=
  if e < c.core.promised then (.refused, c)
  else if verOf r < verOf c.rank then (.failed, c)
  else if c.core.acc = e ∧ r < c.rank then (.failed, c)
  else
    let d := decideAppend c.core e after w
    (d.1, ⟨d.2, if d.1 = .ack then r else c.rank⟩)

/-- The election of `h` in `e`, as (rank, adopted log). -/
def elected? (s : JSys) (e : Nat) (h : String) : Option (Nat × Log) :=
  (s.elections.find? (fun (e', h', _, _) => e' = e ∧ h' = h)).map (fun (_, _, r, A) => (r, A))

def appendJ (s : JSys) (c : String) (e after r : Nat) (w : Log) : Outcome × JSys :=
  let d := decideJ (s.cust c) e after r w
  (d.1, { s with
    cust := upd s.cust c d.2
    accepted := if d.1 = .ack then s.accepted ++ [(c, e, r, d.2.core.log)] else s.accepted })

/-- A record some custodian or writer has: the genesis one, one a promise reported, or one in
force at a writer. -/
def seen (g : Nat) (s : JSys) (r : Nat) : Bool :=
  r == 2 * g || s.promises.any (·.rank == r) || s.inForce.any (·.2 == r)

/-- A record a candidate can be elected under: one seen, or a change pending on one seen (as if
elected under it and then proposing the change). -/
def known (g : Nat) (s : JSys) (r : Nat) : Bool :=
  seen g s r || (isJoint r && seen g s (r - 1))

/-- Custodian `c` holds the whole log of the writer `h` of `e` with a record of rank `r` or
above. -/
def holds (s : JSys) (e r : Nat) (h c : String) : Bool :=
  (s.cust c).core.acc == e && decide (r ≤ (s.cust c).rank) &&
    decide (s.wlog e h <+: (s.cust c).core.log)

/-- The switch rule: quorums of both sets hold the writer's whole log with the joint record. -/
def switchReady (conf : Confs) (s : JSys) (e : Nat) (h : String) : Bool :=
  let r := s.wrank e h
  isJoint r &&
    decide ((conf (verOf r)).quorum ≤ ((conf (verOf r)).custodians.filter (holds s e r h)).length) &&
    decide ((conf (verOf r + 1)).quorum ≤
      ((conf (verOf r + 1)).custodians.filter (holds s e r h)).length)

/-- The promises of `qs` for `e` to `h`. -/
def promisesOf (s : JSys) (e : Nat) (h : String) (qs : List String) : List JPromise :=
  qs.filterMap (fun c => s.promises.find? (fun p => p.cust = c ∧ p.epoch = e ∧ p.host = h))

/-- The moves of the system. -/
inductive Action where
  | prepare (c : String) (e : Nat) (h : String)
  | elect (h : String) (e r : Nat) (qs : List String) (A : Log)
  | write (h : String) (e : Nat) (xs : Log)
  | propose (h : String) (e : Nat)
  | append (c h : String) (e after r : Nat) (w : Log)
  | switch (h : String) (e : Nat)
  deriving Repr

/-- The guard of an election. -/
def electOk (conf : Confs) (g : Nat) (s : JSys) (h : String) (e r : Nat) (qs : List String)
    (A : Log) : Bool :=
  let ps := promisesOf s e h qs
  known g s r && decide (RQuorum conf r qs) && ps.length == qs.length &&
    ps.all (·.rank ≤ r) && Check.adoptsB (ps.map JPromise.base) A && (elected? s e h).isNone

/-- Whether a move starts a change: a proposal, or an election under a joint record. -/
def Action.changes : Action → Bool
  | .propose .. => true
  | .elect _ _ r _ _ => isJoint r
  | _ => false

/-- A move, when it is enabled. -/
def exec (conf : Confs) (g : Nat) (s : JSys) : Action → Option JSys
  | .prepare c e h =>
    if (s.cust c).core.promised < e then
      some { s with
        cust := upd s.cust c ⟨{ (s.cust c).core with promised := e }, (s.cust c).rank⟩
        promises := s.promises ++
          [⟨c, e, h, (s.cust c).core.acc, (s.cust c).core.log, (s.cust c).rank⟩] }
    else none
  | .elect h e r qs A =>
    if electOk conf g s h e r qs A then
      some { s with
        elections := s.elections ++ [(e, h, r, A)]
        wlog := upd2 s.wlog e h A
        wrank := upd2 s.wrank e h r
        inForce := s.inForce ++ [(e, r)] }
    else none
  | .write h e xs =>
    if (elected? s e h).isSome then some { s with wlog := upd2 s.wlog e h (s.wlog e h ++ xs) }
    else none
  | .propose h e =>
    if (elected? s e h).isSome && !isJoint (s.wrank e h) then
      some { s with
        wrank := upd2 s.wrank e h (s.wrank e h + 1)
        inForce := s.inForce ++ [(e, s.wrank e h + 1)] }
    else none
  | .append c h e after r w =>
    match elected? s e h with
    | some (r₀, A) =>
      if decide (A <+: w) && decide (w <+: s.wlog e h) && decide (after ≤ w.length) &&
          decide (r₀ ≤ r) && decide (r ≤ s.wrank e h) then
        some (appendJ s c e after r w).2
      else none
    | none => none
  | .switch h e =>
    if (elected? s e h).isSome && switchReady conf s e h then
      some { s with
        wrank := upd2 s.wrank e h (s.wrank e h + 1)
        inForce := s.inForce ++ [(e, s.wrank e h + 1)] }
    else none

/-- The states reachable from fresh custodians at the genesis version `g`. -/
inductive Reachable (conf : Confs) (g : Nat) : JSys → Prop
  | empty : Reachable conf g (JSys.empty g)
  | step {s t : JSys} (a : Action) : Reachable conf g s → exec conf g s a = some t →
      Reachable conf g t

/-- `L` is committed in `e`: a quorum of a rank in force in `e` accepted, in `e` and with
batches of that rank, logs extending `L`. -/
def Committed (conf : Confs) (s : JSys) (L : Log) (e : Nat) : Prop :=
  ∃ r, (e, r) ∈ s.inForce ∧ ∃ Q, RQuorum conf r Q ∧
    ∀ c ∈ Q, ∃ l, (c, e, r, l) ∈ s.accepted ∧ L <+: l

/-- **Conjecture (not proven here)**: one writer per epoch, with custodian-set changes. -/
def OneWriterPerEpoch (conf : Confs) (g : Nat) : Prop :=
  ∀ s, Reachable conf g s → ∀ e h₁ h₂ r₁ r₂ A₁ A₂, (e, h₁, r₁, A₁) ∈ s.elections →
    (e, h₂, r₂, A₂) ∈ s.elections → h₁ = h₂

/-- **Conjecture (not proven here)**: committed logs survive, with custodian-set changes. -/
def CommittedSurvives (conf : Confs) (g : Nat) : Prop :=
  ∀ s, Reachable conf g s → ∀ L e₀, Committed conf s L e₀ →
    (∀ e h r A, (e, h, r, A) ∈ s.elections → e₀ < e → L <+: A) ∧
    (∀ c e r l, (c, e, r, l) ∈ s.accepted → e₀ < e → L <+: l)

/-! ## Without a change, the fixed-set model -/

/-- The fixed-set state of a state. -/
def project (s : JSys) : Sys :=
  { cust := fun c => (s.cust c).core
    promises := s.promises.map JPromise.base
    elections := s.elections.map (fun (e, h, _, A) => (e, h, A))
    wlog := s.wlog
    accepted := s.accepted.map (fun (c, e, _, l) => (c, e, l)) }

/-- Nothing has left the genesis record `2g`. -/
structure NoChange (g : Nat) (s : JSys) : Prop where
  cust : ∀ c, (s.cust c).rank = 2 * g
  promises : ∀ p ∈ s.promises, p.rank = 2 * g
  elections : ∀ x ∈ s.elections, x.2.2.1 = 2 * g
  wrank : ∀ e h, s.wrank e h = 2 * g
  inForce : ∀ x ∈ s.inForce, x.2 = 2 * g

theorem elected?_some {s : JSys} {e : Nat} {h : String} {r : Nat} {A : Log}
    (hx : elected? s e h = some (r, A)) : (e, h, r, A) ∈ s.elections := by
  unfold elected? at hx
  cases hf : s.elections.find? (fun (e', h', _, _) => e' = e ∧ h' = h) with
  | none => rw [hf] at hx; cases hx
  | some x =>
    rw [hf] at hx
    simp only [Option.map_some, Option.some.injEq] at hx
    have hm := List.mem_of_find?_eq_some hf
    have hp := List.find?_some hf
    obtain ⟨e', h', r', A'⟩ := x
    simp only [decide_eq_true_eq] at hp
    obtain ⟨rfl, rfl⟩ := hp
    simp only [Prod.mk.injEq] at hx
    obtain ⟨rfl, rfl⟩ := hx
    exact hm

theorem elected?_none {s : JSys} {e : Nat} {h : String} (hx : elected? s e h = none) :
    ∀ r A, (e, h, r, A) ∉ s.elections := by
  intro r A hm
  unfold elected? at hx
  cases hf : s.elections.find? (fun (e', h', _, _) => e' = e ∧ h' = h) with
  | none =>
    have := List.find?_eq_none.mp hf _ hm
    simp at this
  | some x => rw [hf] at hx; cases hx

theorem promisesOf_spec {s : JSys} {e : Nat} {h : String} {qs : List String}
    (hl : (promisesOf s e h qs).length = qs.length) :
    (promisesOf s e h qs).map (·.cust) = qs ∧
      ∀ p ∈ promisesOf s e h qs, p ∈ s.promises ∧ p.epoch = e ∧ p.host = h := by
  induction qs with
  | nil => simp [promisesOf]
  | cons c qs ih =>
    unfold promisesOf at hl ⊢
    simp only [List.filterMap_cons] at hl ⊢
    cases hf : s.promises.find? (fun p => p.cust = c ∧ p.epoch = e ∧ p.host = h) with
    | none =>
      rw [hf] at hl
      have := List.length_filterMap_le
        (fun c => s.promises.find? (fun p => p.cust = c ∧ p.epoch = e ∧ p.host = h)) qs
      simp only [List.length_cons] at hl; omega
    | some p =>
      rw [hf] at hl
      simp only [List.length_cons, Nat.add_right_cancel_iff] at hl
      obtain ⟨ih1, ih2⟩ := ih hl
      have hp := List.find?_some hf
      simp only [decide_eq_true_eq] at hp
      refine ⟨?_, ?_⟩
      · simp only [List.map_cons]; rw [hp.1]; exact congrArg _ ih1
      · intro q hq
        rcases List.mem_cons.mp hq with rfl | hq
        · exact ⟨List.mem_of_find?_eq_some hf, hp.2.1, hp.2.2⟩
        · exact ih2 q hq

theorem project_cust_upd (s : JSys) (c : String) (x : JCust) :
    (fun y => (upd s.cust c x y).core) = upd (fun c => (s.cust c).core) c x.core := by
  funext y; unfold upd; split <;> rfl

theorem mem_project_elections {s : JSys} {e : Nat} {h : String} {r : Nat} {A : Log}
    (hm : (e, h, r, A) ∈ s.elections) : (e, h, A) ∈ (project s).elections :=
  List.mem_map.mpr ⟨_, hm, rfl⟩

theorem decideJ_noChange {g : Nat} {c : JCust} (hc : c.rank = 2 * g) (e after : Nat) (w : Log) :
    decideJ c e after (2 * g) w =
      ((decideAppend c.core e after w).1, ⟨(decideAppend c.core e after w).2, 2 * g⟩) := by
  unfold decideJ
  by_cases hp : e < c.core.promised
  · simp only [hp, ↓reduceIte, decideAppend]
    rw [← hc]
  · simp only [hp, ↓reduceIte, hc, Nat.lt_irrefl, and_false, ite_self]

/-- Reachable without starting a change. -/
inductive ReachableNC (conf : Confs) (g : Nat) : JSys → Prop
  | empty : ReachableNC conf g (JSys.empty g)
  | step {s t : JSys} (a : Action) (hna : a.changes = false) :
      ReachableNC conf g s → exec conf g s a = some t → ReachableNC conf g t

/-- **Reconfiguration-free moves are fixed-set moves.** While no change has been started, every
move of this model that does not start one is a move of the fixed-set model on the projected
state, for the genesis configuration. -/
theorem exec_noChange_project (conf : Confs) (g : Nat) {s t : JSys} (hs : NoChange g s)
    (a : Action) (hna : a.changes = false) (hx : exec conf g s a = some t) :
    Step (conf g) (project s) (project t) ∧ NoChange g t := by
  have hv : verOf (2 * g) = g := by unfold verOf; omega
  have hj : isJoint (2 * g) = false := by simp only [isJoint, beq_eq_false_iff_ne]; omega
  cases a with
  | prepare c e h =>
    simp only [exec] at hx
    split at hx
    · next hp =>
      cases hx
      refine ⟨?_, ?_⟩
      · have heq : project { s with
            cust := upd s.cust c ⟨{ (s.cust c).core with promised := e }, (s.cust c).rank⟩
            promises := s.promises ++
              [⟨c, e, h, (s.cust c).core.acc, (s.cust c).core.log, (s.cust c).rank⟩] } =
            Custody.prepare (project s) c e h := by
          simp only [project, Custody.prepare, List.map_append, List.map_cons, List.map_nil,
            JPromise.base]
          rw [project_cust_upd]
        rw [heq]; exact Step.prepare _ c e h hp
      · refine ⟨fun c' => ?_, fun p hp' => ?_, hs.elections, hs.wrank, hs.inForce⟩
        · show (upd s.cust c _ c').rank = 2 * g
          unfold upd; split
          · exact hs.cust c
          · exact hs.cust c'
        · rcases List.mem_append.mp hp' with hp' | hp'
          · exact hs.promises p hp'
          · simp only [List.mem_singleton] at hp'; subst hp'; exact hs.cust c
    · cases hx
  | elect h e r qs A =>
    simp only [exec] at hx
    split at hx
    · next hok =>
      cases hx
      simp only [electOk, Bool.and_eq_true, beq_iff_eq, decide_eq_true_eq,
        Option.isNone_iff_eq_none] at hok
      obtain ⟨⟨⟨⟨⟨hk, hq⟩, hl⟩, -⟩, hA⟩, hnone⟩ := hok
      have hr : r = 2 * g := by
        have hk' : seen g s r = true := by
          simp only [known, Bool.or_eq_true, Bool.and_eq_true] at hk
          rcases hk with hk | ⟨hj', -⟩
          · exact hk
          · simp only [Action.changes] at hna; rw [hna] at hj'; cases hj'
        simp only [seen, Bool.or_eq_true, beq_iff_eq, List.any_eq_true] at hk'
        rcases hk' with (hk | ⟨p, hp, hpr⟩) | ⟨x, hx', hxr⟩
        · exact hk
        · rw [← hpr]; exact hs.promises p hp
        · rw [← hxr]; exact hs.inForce x hx'
      subst hr
      obtain ⟨hpq, hpm⟩ := promisesOf_spec hl
      let ps := (promisesOf s e h qs).map JPromise.base
      have hquo : Quorum (conf g) (ps.map (·.cust)) := by
        have : ps.map (·.cust) = qs := by
          simp only [ps, List.map_map]; exact hpq
        rw [this]
        obtain ⟨hn, h1, -, h3⟩ := hq
        refine ⟨hn, fun c hc => ?_, ?_⟩
        · rcases h3 c hc with h | ⟨h, -⟩
          · rw [← hv]; exact h
          · rw [hj] at h; cases h
        · rw [hv] at h1; unfold QuorumIn at h1
          have := List.length_filter_le (fun c => decide (c ∈ (conf g).custodians)) qs
          omega
      refine ⟨?_, ?_⟩
      · have hps : ∀ p ∈ ps, p ∈ (project s).promises ∧ p.epoch = e ∧ p.host = h := by
          intro p hp
          obtain ⟨q, hq', rfl⟩ := List.mem_map.mp hp
          obtain ⟨h1, h2, h3⟩ := hpm q hq'
          exact ⟨List.mem_map.mpr ⟨q, h1, rfl⟩, h2, h3⟩
        have hnew : ∀ A', (e, h, A') ∉ (project s).elections := by
          intro A' hm
          obtain ⟨⟨e', h', r', A''⟩, hm', heq⟩ := List.mem_map.mp hm
          simp only [Prod.mk.injEq] at heq
          obtain ⟨rfl, rfl, rfl⟩ := heq
          exact elected?_none hnone r' _ hm'
        have heq : project { s with
            elections := s.elections ++ [(e, h, 2 * g, A)]
            wlog := upd2 s.wlog e h A
            wrank := upd2 s.wrank e h (2 * g)
            inForce := s.inForce ++ [(e, 2 * g)] } = Custody.elect (project s) h e A := by
          simp only [project, Custody.elect, List.map_append, List.map_cons, List.map_nil]
        rw [heq]
        exact Step.elect _ h e ps A hps hquo (Check.adoptsB_sound hA) hnew
      · refine ⟨hs.cust, hs.promises, fun x hx' => ?_, fun e' h' => ?_, fun x hx' => ?_⟩
        · rcases List.mem_append.mp hx' with hx' | hx'
          · exact hs.elections x hx'
          · simp only [List.mem_singleton] at hx'; subst hx'; rfl
        · show upd2 s.wrank e h (2 * g) e' h' = 2 * g
          unfold upd2; split
          · rfl
          · exact hs.wrank e' h'
        · rcases List.mem_append.mp hx' with hx' | hx'
          · exact hs.inForce x hx'
          · simp only [List.mem_singleton] at hx'; subst hx'; rfl
    · cases hx
  | write h e xs =>
    simp only [exec] at hx
    split at hx
    · next he =>
      cases hx
      obtain ⟨⟨r, A⟩, hra⟩ := Option.isSome_iff_exists.mp he
      exact ⟨Step.write _ h e A (mem_project_elections (elected?_some hra)) xs,
        { hs with }⟩
    · cases hx
  | propose h e => simp [Action.changes] at hna
  | append c h e after r w =>
    simp only [exec] at hx
    split at hx
    · next r₀ A hra =>
      split at hx
      · next hok =>
        cases hx
        simp only [Bool.and_eq_true, decide_eq_true_eq] at hok
        obtain ⟨⟨⟨⟨hA, hw⟩, hafter⟩, h0⟩, h1⟩ := hok
        have hm := elected?_some hra
        have hr0 : r₀ = 2 * g := hs.elections _ hm
        have hr : r = 2 * g := by rw [hs.wrank] at h1; omega
        subst hr
        have hd := decideJ_noChange (hs.cust c) e after w
        refine ⟨?_, ?_⟩
        · have heq : project (appendJ s c e after (2 * g) w).2 =
              (Custody.append (project s) c e after w).2 := by
            rw [append_snd]
            simp only [appendJ, hd, project]
            by_cases hk : (decideAppend (s.cust c).core e after w).1 = .ack
            · simp only [hk, ↓reduceIte, List.map_append, List.map_cons, List.map_nil]
              congr 1
              funext x; unfold upd; split <;> rfl
            · simp only [hk, ↓reduceIte]
              congr 1
              funext x; unfold upd; split <;> rfl
          rw [heq]
          exact Step.append _ c h e after A w (mem_project_elections hm) hA hw hafter
        · refine ⟨fun c' => ?_, hs.promises, hs.elections, hs.wrank, hs.inForce⟩
          show (upd s.cust c (decideJ (s.cust c) e after (2 * g) w).2 c').rank = 2 * g
          rw [hd]; unfold upd; split
          · rfl
          · exact hs.cust c'
      · cases hx
    · cases hx
  | switch h e =>
    simp only [exec] at hx
    split at hx
    · next hok =>
      simp only [Bool.and_eq_true, switchReady, hs.wrank, hj, Bool.false_and] at hok
      exact absurd hok.2 (by simp)
    · cases hx

theorem reachableNC_project (conf : Confs) (g : Nat) {s : JSys} (h : ReachableNC conf g s) :
    Custody.Reachable (conf g) (project s) ∧ NoChange g s := by
  induction h with
  | empty =>
    refine ⟨?_, ⟨fun _ => rfl, by simp [JSys.empty], by simp [JSys.empty], fun _ _ => rfl,
      by simp [JSys.empty]⟩⟩
    have : project (JSys.empty g) = Sys.empty := rfl
    rw [this]; exact Custody.Reachable.empty
  | step a hna _ hx ih =>
    obtain ⟨h1, h2⟩ := exec_noChange_project conf g ih.2 a hna hx
    exact ⟨Custody.Reachable.step ih.1 h1, h2⟩

/-- **One writer per epoch, without a change.** -/
theorem one_writer_per_epoch_noChange (conf : Confs) (g : Nat) (hm : (conf g).Majority)
    {s : JSys} (hr : ReachableNC conf g s) {e : Nat} {h₁ h₂ : String} {r₁ r₂ : Nat}
    {A₁ A₂ : Log} (e₁ : (e, h₁, r₁, A₁) ∈ s.elections) (e₂ : (e, h₂, r₂, A₂) ∈ s.elections) :
    h₁ = h₂ :=
  one_writer_per_epoch (conf g) hm (reachableNC_project conf g hr).1
    (mem_project_elections e₁) (mem_project_elections e₂)

/-- A commit of this model, without a change, is a fixed-set commit. -/
theorem committed_project (conf : Confs) (g : Nat) {s : JSys} (hs : NoChange g s) {L : Log}
    {e₀ : Nat} (hc : Committed conf s L e₀) : Custody.Committed (conf g) (project s) L e₀ := by
  obtain ⟨r, hrm, Q, hq, hall⟩ := hc
  have hr : r = 2 * g := hs.inForce _ hrm
  subst hr
  have hv : verOf (2 * g) = g := by unfold verOf; omega
  have hj : isJoint (2 * g) = false := by simp only [isJoint, beq_eq_false_iff_ne]; omega
  obtain ⟨hn, h1, -, h3⟩ := hq
  refine ⟨Q, ⟨hn, fun c hc => ?_, ?_⟩, fun c hc => ?_⟩
  · rcases h3 c hc with h | ⟨h, -⟩
    · rw [← hv]; exact h
    · rw [hj] at h; cases h
  · rw [hv] at h1; unfold QuorumIn at h1
    have := List.length_filter_le (fun c => decide (c ∈ (conf g).custodians)) Q
    omega
  · obtain ⟨l, hl, hL⟩ := hall c hc
    exact ⟨l, List.mem_map.mpr ⟨_, hl, rfl⟩, hL⟩

/-- **Committed logs survive, without a change**: in every later election and acceptance. -/
theorem committed_survives_noChange (conf : Confs) (g : Nat) (hm : (conf g).Majority)
    {s : JSys} (hr : ReachableNC conf g s) {L : Log} {e₀ : Nat} (hc : Committed conf s L e₀) :
    (∀ e h r A, (e, h, r, A) ∈ s.elections → e₀ < e → L <+: A) ∧
    (∀ c e r l, (c, e, r, l) ∈ s.accepted → e₀ < e → L <+: l) := by
  have hp := reachableNC_project conf g hr
  have hc' := committed_project conf g hp.2 hc
  exact ⟨fun e h r A he hlt => committed_survives_election (conf g) hm hp.1 hc'
      (mem_project_elections he) hlt,
    fun c e r l ha hlt => committed_survives_accept (conf g) hm hp.1 hc'
      (List.mem_map.mpr ⟨(c, e, r, l), ha, rfl⟩) hlt⟩

end Vertep.Custody.Joint
