/-!
# Durable-object custody: quorum-promised epochs over a replicated log (m7 10.1)

A model of `vertep_objects::custody` (OpenSpec change `m7-durable-objects`, design D3): the
journal of a durable object is kept by a set of custodians, and a host becomes the writer of
an epoch by gathering a quorum of promises. It is single-decree Paxos over the whole log:
ballot = epoch, accepted value = the custodian's log.

* **Custodians** (`Cust`, Rust `replica::State`): the highest epoch promised, the epoch the log
  was last accepted in, and the log itself (a list of entry hashes; Rust's entry hashes are
  chained, so a hash names the whole prefix it ends).
* **Prepare** (`prepare`, Rust `Replica::prepare`): a custodian promises `e` iff `e` is above
  every epoch it promised, and reports its accepted epoch and log with the promise.
* **Election** (`Step.elect`, Rust `CustodyHost::elect`): a host that has a quorum of promises
  for `e` adopts the log of the promise with the largest `(accepted epoch, length)` (`Adopts`).
  A host elects itself at most once per epoch.
* **Writing** (`Step.write`): the writer of `e` extends its log; it never rewrites it.
* **Append** (`decideAppend`, Rust `Replica::append`): the writer ships batches that always run to
  its log end at the time of shipping. Batches may be delayed and reordered, so any log between
  the adopted one and the writer's current one can arrive (`Step.append`).
* **Committed** (`Committed`): a log is committed in epoch `e` when a quorum of custodians
  accepted, in `e`, logs extending it.

Ghost state records the promises made, the elections, the writers' logs and every log a
custodian accepted, in which epoch. The theorems are in `VertepLean.CustodyTheorems`.

## The append decision

A batch `append(e, after, entries)` continues the writer's chain `w` (`entries = w.drop after`;
`w.take after` is what the first entry's `prev` hash commits to). With chained hashes, equal
hashes at a position mean equal prefixes up to it, so the decision compares prefixes:

1. `e < promised`: **refused**.
2. `after > length log`: **behind**.
3. The custodian's log agrees with `w` up to `after`:
   * from a newer epoch than the one accepted (`acc < e`): **ack**, the log becomes `w`
     (an older uncommitted tail, also beyond the batch end, is replaced);
   * same epoch, the log is a prefix of `w`: **ack**, the log becomes `w`;
   * same epoch, `w` is a prefix of the log (a delayed, shorter batch): **ack**, no change;
   * otherwise **diverged** (Rust also records equivocation evidence when both entries are
     signed by the epoch's writer).
4. It does not agree up to `after` (the batch's chain link fails):
   * an empty batch of the accepted epoch: **ack**, no change;
   * a non-empty batch of the accepted epoch overlapping the log: **diverged** (the first
     overlapping hash differs);
   * otherwise **failed** (Rust: the chain check of the new entries fails).

One deliberate difference from Rust: an *empty* batch from a newer epoch whose `after` does
not continue the custodian's log is **failed** here, while `Replica::append` acknowledges it
(truncating to `after` and taking the epoch) without a link check. The Rust writer only ships
from positions the custodian acknowledged or promised with a matching head hash, so it should
never send such a batch; the oracle reports it if it does.

On every ack the accepted epoch becomes `e` and the promised epoch `max promised e`.
-/

namespace Vertep.Custody

/-- An entry hash (opaque). -/
abbrev Hash := String

/-- A custodian's log. -/
abbrev Log := List Hash

/-- A custodian's state (Rust `replica::State`, restricted to custody). -/
structure Cust where
  promised : Nat
  acc : Nat
  log : Log
  deriving DecidableEq, Repr

/-- A fresh custodian. -/
def Cust.empty : Cust := ⟨0, 0, []⟩

/-- A promise of epoch `epoch` by custodian `cust` to `host`, with the custodian's accepted
epoch and log at the time. -/
structure Promise where
  cust : String
  epoch : Nat
  host : String
  acc : Nat
  log : Log
  deriving DecidableEq, Repr

/-- The custodian's answer to an append. -/
inductive Outcome where
  | ack
  | refused
  | behind
  | diverged
  | failed
  deriving DecidableEq, Repr

/-- A configuration: the custodians and the quorum. -/
structure Config where
  custodians : List String
  quorum : Nat
  deriving Repr

/-- `Q` is a quorum: distinct custodians of the set, at least `quorum` of them. -/
def Quorum (cfg : Config) (Q : List String) : Prop :=
  Q.Nodup ∧ (∀ c ∈ Q, c ∈ cfg.custodians) ∧ cfg.quorum ≤ Q.length

instance (cfg : Config) (Q : List String) : Decidable (Quorum cfg Q) := by
  unfold Quorum; exact inferInstance

/-- The quorum is a majority of the custodian set. -/
def Config.Majority (cfg : Config) : Prop := cfg.custodians.length < 2 * cfg.quorum

/-- The system. -/
structure Sys where
  /-- Each custodian's state. -/
  cust : String → Cust
  /-- Ghost: every promise made, in order. -/
  promises : List Promise
  /-- Ghost: every election, as (epoch, host, adopted log). -/
  elections : List (Nat × String × Log)
  /-- Ghost: the writer's log of host `h` in epoch `e` (meaningful once `h` is elected). -/
  wlog : Nat → String → Log
  /-- Ghost: every log a custodian accepted, as (custodian, epoch, log after the ack). -/
  accepted : List (String × Nat × Log)

/-- The initial system: fresh custodians, nothing promised, elected or accepted. -/
def Sys.empty : Sys :=
  { cust := fun _ => Cust.empty, promises := [], elections := [], wlog := fun _ _ => [],
    accepted := [] }

/-- Point update. -/
def upd {α β : Type} [DecidableEq α] (f : α → β) (a : α) (b : β) : α → β :=
  fun x => if x = a then b else f x

/-- Point update of a two-argument function. -/
def upd2 {α β γ : Type} [DecidableEq α] [DecidableEq β] (f : α → β → γ) (a : α) (b : β)
    (c : γ) : α → β → γ :=
  fun x y => if x = a ∧ y = b then c else f x y

/-- The custodian after acknowledging a batch of epoch `e`, holding `l`. -/
def Cust.accept (c : Cust) (e : Nat) (l : Log) : Cust :=
  { promised := max c.promised e, acc := e, log := l }

/-- The custodian's decision on a batch of epoch `e` continuing the writer's chain `w` after
position `after` (entries `w.drop after`), and the custodian afterwards (Rust
`Replica::append`, see the module documentation). -/
-- @tg: implements:: [[openspec:object-custody#Quorum Before Effects]] {note: "Lean model"}
def decideAppend (c : Cust) (e after : Nat) (w : Log) : Outcome × Cust :=
  if e < c.promised then (.refused, c)
  else if c.log.length < after then (.behind, c)
  else if c.log.take after = w.take after then
    if c.acc < e then (.ack, c.accept e w)
    else if c.log <+: w then (.ack, c.accept e w)
    else if w <+: c.log then (.ack, c.accept e c.log)
    else (.diverged, c)
  else if w.length = after ∧ ¬ c.acc < e then (.ack, c.accept e c.log)
  else if ¬ c.acc < e ∧ after < c.log.length then (.diverged, c)
  else (.failed, c)

/-- Whether a custodian in state `c` promises epoch `e` (Rust `Replica::prepare`). -/
def promises? (c : Cust) (e : Nat) : Bool := c.promised < e

/-- A promise of `e` by `c` to `h`: the custodian's promised epoch rises to `e`, and the
promise is recorded with the custodian's accepted epoch and log. -/
def prepare (s : Sys) (c : String) (e : Nat) (h : String) : Sys :=
  { s with
    cust := upd s.cust c { s.cust c with promised := e }
    promises := s.promises ++ [⟨c, e, h, (s.cust c).acc, (s.cust c).log⟩] }

/-- The adoption rule: `A` is the log of a promise with the largest `(accepted epoch, length)`
among `ps` (Rust: sort by `(accepted_epoch, head)` descending, take the first). -/
def Adopts (ps : List Promise) (A : Log) : Prop :=
  ∃ p ∈ ps, A = p.log ∧
    ∀ p' ∈ ps, p'.acc < p.acc ∨ (p'.acc = p.acc ∧ p'.log.length ≤ p.log.length)

/-- An election of `h` in `e`, adopting `A`. -/
def elect (s : Sys) (h : String) (e : Nat) (A : Log) : Sys :=
  { s with elections := s.elections ++ [(e, h, A)], wlog := upd2 s.wlog e h A }

/-- Custodian `c` handles a batch of epoch `e` continuing chain `w` after `after`. -/
def append (s : Sys) (c : String) (e after : Nat) (w : Log) : Outcome × Sys :=
  let r := decideAppend (s.cust c) e after w
  (r.1, { s with
    cust := upd s.cust c r.2
    accepted := if r.1 = .ack then s.accepted ++ [(c, e, r.2.log)] else s.accepted })

section
variable (cfg : Config)

/-- Every move of the system. -/
inductive Step : Sys → Sys → Prop
  /-- Custodian `c` promises `e` to `h`. -/
  | prepare (s : Sys) (c : String) (e : Nat) (h : String) (hp : (s.cust c).promised < e) :
      Step s (prepare s c e h)
  /-- Host `h` wins epoch `e` with the promises `ps` of a quorum, adopting `A` by the max rule.
  A host elects itself at most once per epoch. -/
  | elect (s : Sys) (h : String) (e : Nat) (ps : List Promise) (A : Log)
      (hps : ∀ p ∈ ps, p ∈ s.promises ∧ p.epoch = e ∧ p.host = h)
      (hq : Quorum cfg (ps.map (·.cust))) (hA : Adopts ps A)
      (hnew : ∀ A', (e, h, A') ∉ s.elections) :
      Step s (elect s h e A)
  /-- The writer of `e` extends its log. -/
  | write (s : Sys) (h : String) (e : Nat) (A : Log) (he : (e, h, A) ∈ s.elections)
      (xs : Log) :
      Step s { s with wlog := upd2 s.wlog e h (s.wlog e h ++ xs) }
  /-- Custodian `c` receives a batch of the writer `h` of `e`: the batch runs to the end of a
  log the writer held, which extends the adopted one (batches may be delayed and reordered). -/
  | append (s : Sys) (c h : String) (e after : Nat) (A w : Log)
      (he : (e, h, A) ∈ s.elections) (hA : A <+: w) (hw : w <+: s.wlog e h)
      (hafter : after ≤ w.length) :
      Step s (append s c e after w).2
  /-- A refused prepare, a failed append (bad signature, unreadable entry), a restart (the
  state is durable), a lost message: nothing changes. -/
  | stutter (s : Sys) : Step s s

/-- The states reachable from the initial one. -/
inductive Reachable : Sys → Prop
  | empty : Reachable Sys.empty
  | step {s t : Sys} : Reachable s → Step cfg s t → Reachable t

end

/-- `L` is committed in epoch `e`: a quorum of custodians accepted, in `e`, logs extending `L`
(the writer announces it once a quorum acknowledged its whole log). -/
def Committed (cfg : Config) (s : Sys) (L : Log) (e : Nat) : Prop :=
  ∃ Q, Quorum cfg Q ∧ ∀ c ∈ Q, ∃ l, (c, e, l) ∈ s.accepted ∧ L <+: l

end Vertep.Custody
