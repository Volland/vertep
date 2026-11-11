/-!
# Shard leases, epochs and fencing (m4-cluster 6.4)

A model of the single-activation machinery of `vertep-cluster`:

* **Lease authorities** (`LeaseTable`): per shard, the current grant and the highest epoch the
  authority has granted. `acquire` renews the caller's own live grant or, once the shard is free
  (no grant, or the grant expired), grants it at a new epoch `next highest`; `handover` moves a
  live grant to another node at a new epoch; `release` and `revoke` as in Rust. Several
  authorities may grant concurrently (a gossip leader on each side of a partition), each with
  its own `EpochPolicy`; the policies only have to be increasing and to have disjoint ranges
  (Rust `EpochPolicy::Tagged` with distinct tags; a single `Sequential` authority trivially).
* **Writers** may append under any lease they were ever granted, at any time: holder clocks,
  pauses and partitions are not trusted, so a stale holder keeps trying.
* **The fenced journal store** (`MemoryStore::append` with a fencing token): an append is
  `fenced` when its epoch is below the highest epoch the store accepted for the shard, a
  `conflict` when the actor's journal is not at the expected version or already holds the
  entry for the message (the entry IRI is taken), and accepted otherwise, raising the shard's
  highest epoch.
* **Receivers** (`EpochFilter`, also the replication receivers of standbys): a frame produced by
  an accepted append is admitted when its epoch is not below the highest the receiver has seen
  for the shard, and raises it; `observe` raises it from a view.

The theorems (`VertepLean.LeaseTheorems`) hold in every reachable state, whatever the
interleaving of grants, appends, frames and clock ticks.
-/

namespace Vertep.Lease

/-- An authority's epoch policy: increasing. -/
structure Policy where
  next : Nat → Nat
  lt_next : ∀ h, h < next h

/-- A grant held by the authority. -/
structure Grant where
  node : String
  epoch : Nat
  expires : Nat
  revoked : Bool

/-- An authority's slot for one shard (Rust `lease::Slot`). -/
structure Slot where
  highest : Nat
  grant : Option Grant

/-- A journal entry: the writer and the epoch of its lease. -/
structure Entry where
  writer : String
  epoch : Nat
  message : String
  deriving DecidableEq, Repr

/-- The store's decision on an append (Rust `AppendDecision`). -/
inductive AppendDecision where
  | accept
  | fenced
  | conflict
  deriving DecidableEq, Repr

/-- The cluster. -/
structure Sys where
  /-- The authorities' clock. -/
  now : Nat
  /-- Authority `a`'s slot for shard `k`. -/
  auth : Nat → Nat → Slot
  /-- Ghost: every grant ever made, as (authority, node, shard, epoch). -/
  granted : List (Nat × String × Nat × Nat)
  /-- Each actor's journal. -/
  journals : String → List Entry
  /-- The store's highest accepted epoch per shard. -/
  fence : Nat → Nat
  /-- Ghost: the accepted appends of each shard, in order. -/
  accepted : Nat → List Entry
  /-- Receiver `r`'s highest epoch for shard `k`. -/
  recv : String → Nat → Nat
  /-- Ghost: the epochs of the frames receiver `r` admitted for shard `k`, in order. -/
  admitted : String → Nat → List Nat

/-- The empty cluster. -/
def Sys.empty : Sys :=
  { now := 0, auth := fun _ _ => ⟨0, none⟩, granted := [], journals := fun _ => [],
    fence := fun _ => 0, accepted := fun _ => [], recv := fun _ _ => 0,
    admitted := fun _ _ => [] }

/-- Point updates. -/
def upd {α β : Type} [DecidableEq α] (f : α → β) (a : α) (b : β) : α → β :=
  fun x => if x = a then b else f x

def upd2 {α β γ : Type} [DecidableEq α] [DecidableEq β] (f : α → β → γ) (a : α) (b : β)
    (c : γ) : α → β → γ :=
  fun x y => if x = a ∧ y = b then c else f x y

/-- The store's decision (Rust `MemoryStore::append`: fencing first, then the version, then
the entry name; the fault-injection harness records a taken entry name as a conflict). -/
-- @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]] {note: "Lean model"}
def decide (s : Sys) (k : Nat) (e : Nat) (actor : String) (version : Nat) (msg : String) :
    AppendDecision :=
  if e < s.fence k then .fenced
  else if version ≠ (s.journals actor).length + 1 then .conflict
  else if msg ∈ (s.journals actor).map (·.message) then .conflict
  else .accept

/-- Whether receiver `r` admits a frame of shard `k` at epoch `e` (the admission check of Rust `EpochFilter`). -/
def admits (s : Sys) (r : String) (k e : Nat) : Bool := !(e < s.recv r k)

section
variable (pol : Nat → Policy) (dur : Nat) (shardOf : String → Nat)

/-- A new grant of shard `k` to `n` by authority `a`, at the next epoch. -/
def grantNew (s : Sys) (a k : Nat) (n : String) : Sys :=
  { s with
    auth := upd2 s.auth a k (Slot.mk ((pol a).next (s.auth a k).highest)
      (some (Grant.mk n ((pol a).next (s.auth a k).highest) (s.now + dur) false)))
    granted := s.granted ++ [(a, n, k, (pol a).next (s.auth a k).highest)] }

/-- An accepted append of `msg` to `actor`'s journal by `n` under epoch `e`. -/
def appendNew (s : Sys) (n : String) (e : Nat) (actor msg : String) : Sys :=
  { s with
    journals := upd s.journals actor (s.journals actor ++ [Entry.mk n e msg])
    fence := upd s.fence (shardOf actor) e
    accepted := upd s.accepted (shardOf actor)
      (s.accepted (shardOf actor) ++ [Entry.mk n e msg]) }

/-- Every move of the cluster. -/
inductive Step : Sys → Sys → Prop
  /-- The authorities' clock advances. -/
  | tick (s : Sys) : Step s { s with now := s.now + 1 }
  /-- Authority `a` renews node `n`'s live grant of shard `k`. -/
  | renew (s : Sys) (a k : Nat) (g : Grant) (hg : (s.auth a k).grant = some g)
      (hl : s.now < g.expires) (hr : g.revoked = false) :
      Step s { s with
        auth := upd2 s.auth a k
          (Slot.mk (s.auth a k).highest (some { g with expires := s.now + dur })) }
  /-- Authority `a` grants a free shard `k` (no grant, or an expired one) to `n` at a new
  epoch. -/
  | acquire (s : Sys) (a k : Nat) (n : String)
      (hf : ∀ g, (s.auth a k).grant = some g → g.expires ≤ s.now) :
      Step s (grantNew pol dur s a k n)
  /-- Authority `a` hands a live grant of shard `k` over to `n` at a new epoch. -/
  | handover (s : Sys) (a k : Nat) (n : String) (g : Grant) (hg : (s.auth a k).grant = some g)
      (hl : s.now < g.expires) (hr : g.revoked = false) :
      Step s (grantNew pol dur s a k n)
  /-- The holder releases shard `k`. -/
  | release (s : Sys) (a k : Nat) :
      Step s { s with auth := upd2 s.auth a k (Slot.mk (s.auth a k).highest none) }
  /-- Authority `a` revokes its grant of shard `k`. -/
  | revoke (s : Sys) (a k : Nat) (g : Grant) (hg : (s.auth a k).grant = some g) :
      Step s { s with
        auth := upd2 s.auth a k
          (Slot.mk (s.auth a k).highest (some { g with revoked := true })) }
  /-- A node appends under a lease it was granted at some time (it may have expired, been
  revoked or superseded: the store decides). An accepted append extends the actor's journal
  and raises the shard's fence. -/
  | appendAccept (s : Sys) (n : String) (a e : Nat) (actor : String) (msg : String)
      (hg : (a, n, shardOf actor, e) ∈ s.granted)
      (hd : decide s (shardOf actor) e actor ((s.journals actor).length + 1) msg = .accept) :
      Step s (appendNew shardOf s n e actor msg)
  /-- A refused append (fenced or conflict) changes nothing. -/
  | appendRefused (s : Sys) : Step s s
  /-- Receiver `r` admits a frame produced by an accepted append of shard `k`. -/
  | frame (s : Sys) (r : String) (k : Nat) (x : Entry) (hx : x ∈ s.accepted k)
      (ha : admits s r k x.epoch = true) :
      Step s { s with
        recv := upd2 s.recv r k x.epoch
        admitted := upd2 s.admitted r k (s.admitted r k ++ [x.epoch]) }
  /-- Receiver `r` learns a higher epoch for shard `k` from a view (Rust `EpochFilter::observe`). -/
  | observe (s : Sys) (r : String) (k e : Nat) :
      Step s { s with recv := upd2 s.recv r k (max (s.recv r k) e) }

/-- The states reachable from the empty cluster. -/
inductive Reachable : Sys → Prop
  | empty : Reachable Sys.empty
  | step {s t : Sys} : Reachable s → Step pol dur shardOf s t → Reachable t

end

/-- Policies whose epoch ranges are disjoint: two authorities never grant the same epoch
(Rust `EpochPolicy::Tagged` with distinct tags). -/
def Disjoint (pol : Nat → Policy) : Prop :=
  ∀ a b h h', a ≠ b → (pol a).next h ≠ (pol b).next h'

end Vertep.Lease
