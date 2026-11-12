import VertepLean.Lease

/-!
# At most one writer per epoch, and totally ordered journals (m4-cluster 6.4)

In every state reachable from the empty cluster, with authorities whose epoch ranges are
disjoint (`Disjoint`):

* `grant_unique` — no `(shard, epoch)` is ever granted to two nodes;
* `one_writer_per_epoch` — for every shard and epoch, all accepted appends come from one
  writer, the node the epoch was granted to (`accepted_granted`);
* `journal_epochs_ordered`, `shard_epochs_ordered` — accepted journal versions are totally
  ordered across epochs: a later version never carries a lower epoch, per actor and per shard;
* `stale_append_fenced` — once an append at some epoch is accepted for a shard, every append
  at a lower epoch is fenced (the split-brain scenario);
* `frames_ordered`, `stale_frame_dropped` — a receiver (an epoch filter, or a standby's
  replication receiver) admits a shard's frames in non-decreasing epoch order, each produced
  by an accepted append, and drops a frame below an epoch it has admitted.

No assumption is made about clocks: stale holders keep appending under any lease they were
ever granted.
-/

namespace Vertep.Lease

variable (pol : Nat → Policy) (dur : Nat) (shardOf : String → Nat)

/-- What holds in every reachable state. -/
structure Inv (s : Sys) : Prop where
  uniq : ∀ a n k e a' n', (a, n, k, e) ∈ s.granted → (a', n', k, e) ∈ s.granted → n = n'
  bound : ∀ a n k e, (a, n, k, e) ∈ s.granted →
    e ≤ (s.auth a k).highest ∧ ∃ h, e = (pol a).next h
  jgrant : ∀ actor x, x ∈ s.journals actor → ∃ a, (a, x.writer, shardOf actor, x.epoch) ∈ s.granted
  jsorted : ∀ actor, (s.journals actor).Pairwise (fun x y => x.epoch ≤ y.epoch)
  jfence : ∀ actor x, x ∈ s.journals actor → x.epoch ≤ s.fence (shardOf actor)
  jonce : ∀ actor, ((s.journals actor).map (·.message)).Nodup
  asorted : ∀ k, (s.accepted k).Pairwise (fun x y => x.epoch ≤ y.epoch)
  afence : ∀ k x, x ∈ s.accepted k → x.epoch ≤ s.fence k
  agrant : ∀ k x, x ∈ s.accepted k → ∃ a, (a, x.writer, k, x.epoch) ∈ s.granted
  dsorted : ∀ r k, (s.admitted r k).Pairwise (· ≤ ·)
  drecv : ∀ r k e, e ∈ s.admitted r k → e ≤ s.recv r k
  dfrom : ∀ r k e, e ∈ s.admitted r k → ∃ x ∈ s.accepted k, x.epoch = e

theorem inv_empty : Inv pol shardOf Sys.empty where
  uniq := by intro _ _ _ _ _ _ h; cases h
  bound := by intro _ _ _ _ h; cases h
  jgrant := by intro _ _ h; cases h
  jsorted := fun _ => List.Pairwise.nil
  jfence := by intro _ _ h; cases h
  jonce := fun _ => List.nodup_nil
  asorted := fun _ => List.Pairwise.nil
  afence := by intro _ _ h; cases h
  agrant := by intro _ _ h; cases h
  dsorted := fun _ _ => List.Pairwise.nil
  drecv := by intro _ _ _ h; cases h
  dfrom := by intro _ _ _ h; cases h

/-- Updating an authority slot while keeping its highest epoch keeps every highest epoch. -/
theorem highest_keep (s : Sys) (a k : Nat) (g : Option Grant) (a' k' : Nat) :
    (upd2 s.auth a k (Slot.mk (s.auth a k).highest g) a' k').highest = (s.auth a' k').highest := by
  unfold upd2; split
  · next h => obtain ⟨rfl, rfl⟩ := h; rfl
  · rfl

theorem inv_keep_auth (s : Sys) (hs : Inv pol shardOf s) (a k : Nat) (g : Option Grant)
    (t : Sys) (ht : t = { s with auth := upd2 s.auth a k (Slot.mk (s.auth a k).highest g) }) :
    Inv pol shardOf t := by
  subst ht
  exact { hs with
    bound := fun a' n k' e h => by
      rw [highest_keep]; exact hs.bound a' n k' e h }

theorem inv_grantNew (hd : Disjoint pol) (s : Sys) (hs : Inv pol shardOf s) (a k : Nat)
    (n : String) : Inv pol shardOf (grantNew pol dur s a k n) := by
  have hE := (pol a).lt_next (s.auth a k).highest
  -- the new epoch was never granted for shard `k`
  have fresh : ∀ a' n', (a', n', k, (pol a).next (s.auth a k).highest) ∉ s.granted := by
    intro a' n' h
    obtain ⟨hb, h', he⟩ := hs.bound a' n' k _ h
    by_cases ha : a' = a
    · subst ha; omega
    · exact hd a a' _ h' (Ne.symm ha) he
  have mono : ∀ q, q ∈ s.granted → q ∈ (grantNew pol dur s a k n).granted :=
    fun q h => List.mem_append_left _ h
  refine { hs with uniq := ?_, bound := ?_, jgrant := ?_, agrant := ?_ }
  · intro a₁ n₁ k₁ e a₂ n₂ h₁ h₂
    simp only [grantNew] at h₁ h₂
    rcases List.mem_append.mp h₁ with g₁ | g₁ <;> rcases List.mem_append.mp h₂ with g₂ | g₂
    · exact hs.uniq _ _ _ _ _ _ g₁ g₂
    · simp at g₂; obtain ⟨rfl, rfl, rfl, rfl⟩ := g₂; exact absurd g₁ (fresh _ _)
    · simp at g₁; obtain ⟨rfl, rfl, rfl, rfl⟩ := g₁; exact absurd g₂ (fresh _ _)
    · simp at g₁ g₂; rw [g₁.2.1, g₂.2.1]
  · intro a' n' k' e h
    simp only [grantNew, upd2] at h ⊢
    rcases List.mem_append.mp h with h | h
    · obtain ⟨hb, hr⟩ := hs.bound a' n' k' e h
      refine ⟨?_, hr⟩
      split
      · next hh => obtain ⟨rfl, rfl⟩ := hh; exact Nat.le_of_lt (Nat.lt_of_le_of_lt hb hE)
      · exact hb
    · simp at h; obtain ⟨rfl, rfl, rfl, rfl⟩ := h
      exact ⟨by simp, _, rfl⟩
  · intro actor x hx
    obtain ⟨a', h⟩ := hs.jgrant actor x hx; exact ⟨a', mono _ h⟩
  · intro k' x hx
    obtain ⟨a', h⟩ := hs.agrant k' x hx; exact ⟨a', mono _ h⟩

theorem inv_appendNew (s : Sys) (hs : Inv pol shardOf s) (n : String) (a e : Nat)
    (actor msg : String) (hg : (a, n, shardOf actor, e) ∈ s.granted)
    (hd : decide s (shardOf actor) e actor ((s.journals actor).length + 1) msg = .accept) :
    Inv pol shardOf (appendNew shardOf s n e actor msg) := by
  have hge : s.fence (shardOf actor) ≤ e := by
    unfold decide at hd
    by_cases h : e < s.fence (shardOf actor)
    · simp [h] at hd
    · omega
  have hnot : msg ∉ (s.journals actor).map (·.message) := by
    intro hm; unfold decide at hd
    by_cases h1 : e < s.fence (shardOf actor)
    · simp [h1] at hd
    · simp [h1, hm] at hd
  refine { hs with
    jonce := ?_
    jgrant := ?_
    jsorted := ?_
    jfence := ?_
    asorted := ?_
    afence := ?_
    agrant := ?_
    dfrom := ?_ }
  · intro actor' x hx
    simp only [appendNew, upd] at hx
    split at hx
    · next h =>
      subst h
      rcases List.mem_append.mp hx with hx | hx
      · exact hs.jgrant _ x hx
      · simp at hx; subst hx; exact ⟨a, hg⟩
    · exact hs.jgrant _ x hx
  · intro actor'
    simp only [appendNew, upd]
    split
    · next h =>
      subst h
      refine List.pairwise_append.mpr ⟨hs.jsorted _, List.pairwise_singleton _ _, ?_⟩
      intro x hx y hy
      simp at hy; subst hy
      exact Nat.le_trans (hs.jfence _ x hx) hge
    · exact hs.jsorted _
  · intro actor' x hx
    simp only [appendNew, upd] at hx ⊢
    by_cases hk : shardOf actor' = shardOf actor
    · simp only [hk, ↓reduceIte]
      split at hx
      · next h =>
        subst h
        rcases List.mem_append.mp hx with hx | hx
        · exact Nat.le_trans (hs.jfence _ x hx) hge
        · simp at hx; subst hx; exact Nat.le_refl _
      · exact Nat.le_trans (hk ▸ hs.jfence _ x hx) hge
    · simp only [hk, ↓reduceIte]
      split at hx
      · next h => subst h; exact absurd rfl hk
      · exact hs.jfence _ x hx
  · intro actor'
    simp only [appendNew, upd]
    split
    · next h =>
      subst h
      rw [List.map_append, List.nodup_append]
      refine ⟨hs.jonce _, by simp, ?_⟩
      intro x hx y hy hxy
      simp at hy; subst hy; subst hxy
      exact hnot hx
    · exact hs.jonce _
  · intro k
    simp only [appendNew, upd]
    split
    · next h =>
      subst h
      refine List.pairwise_append.mpr ⟨hs.asorted _, List.pairwise_singleton _ _, ?_⟩
      intro x hx y hy
      simp at hy; subst hy
      exact Nat.le_trans (hs.afence _ x hx) hge
    · exact hs.asorted _
  · intro k x hx
    simp only [appendNew, upd] at hx ⊢
    split at hx
    · next h =>
      subst h
      simp only [↓reduceIte]
      rcases List.mem_append.mp hx with hx | hx
      · exact Nat.le_trans (hs.afence _ x hx) hge
      · simp at hx; subst hx; exact Nat.le_refl _
    · next h => simp only [h, ↓reduceIte]; exact hs.afence _ x hx
  · intro k x hx
    simp only [appendNew, upd] at hx
    split at hx
    · next h =>
      subst h
      rcases List.mem_append.mp hx with hx | hx
      · exact hs.agrant _ x hx
      · simp at hx; subst hx; exact ⟨a, hg⟩
    · exact hs.agrant _ x hx
  · intro r k e' he
    obtain ⟨x, hx, hxe⟩ := hs.dfrom r k e' he
    refine ⟨x, ?_, hxe⟩
    simp only [appendNew, upd]
    split
    · next h => subst h; exact List.mem_append_left _ hx
    · exact hx

theorem inv_step (hd : Disjoint pol) {s t : Sys} (hs : Inv pol shardOf s)
    (h : Step pol dur shardOf s t) : Inv pol shardOf t := by
  cases h with
  | tick => exact { hs with }
  | renew a k g _ _ _ => exact inv_keep_auth pol shardOf s hs a k _ _ rfl
  | acquire a k n _ => exact inv_grantNew pol dur shardOf hd s hs a k n
  | handover a k n _ _ _ _ => exact inv_grantNew pol dur shardOf hd s hs a k n
  | release a k => exact inv_keep_auth pol shardOf s hs a k _ _ rfl
  | revoke a k g _ => exact inv_keep_auth pol shardOf s hs a k _ _ rfl
  | appendAccept n a e actor msg hg hdec =>
    exact inv_appendNew pol shardOf s hs n a e actor msg hg hdec
  | appendRefused => exact hs
  | frame r k x hx ha =>
    have hle : s.recv r k ≤ x.epoch := by
      simp only [admits, Bool.not_eq_true', decide_eq_false_iff_not] at ha; omega
    refine { hs with dsorted := ?_, drecv := ?_, dfrom := ?_ }
    · intro r' k'
      simp only [upd2]
      split
      · next h =>
        obtain ⟨rfl, rfl⟩ := h
        refine List.pairwise_append.mpr ⟨hs.dsorted _ _, List.pairwise_singleton _ _, ?_⟩
        intro e he e' he'
        simp at he'; subst he'
        exact Nat.le_trans (hs.drecv _ _ e he) hle
      · exact hs.dsorted _ _
    · intro r' k' e he
      simp only [upd2] at he ⊢
      split at he
      · next h =>
        simp only [h, and_self, ↓reduceIte]
        obtain ⟨rfl, rfl⟩ := h
        rcases List.mem_append.mp he with he | he
        · exact Nat.le_trans (hs.drecv _ _ e he) hle
        · simp at he; subst he; exact Nat.le_refl _
      · next h => simp only [h, ↓reduceIte]; exact hs.drecv _ _ e he
    · intro r' k' e he
      simp only [upd2] at he
      split at he
      · next h =>
        obtain ⟨rfl, rfl⟩ := h
        rcases List.mem_append.mp he with he | he
        · exact hs.dfrom _ _ e he
        · simp at he; subst he; exact ⟨x, hx, rfl⟩
      · exact hs.dfrom _ _ e he
  | observe r k e =>
    refine { hs with drecv := ?_ }
    intro r' k' e' he
    simp only [upd2]
    split
    · next h => obtain ⟨rfl, rfl⟩ := h; exact Nat.le_trans (hs.drecv _ _ e' he) (Nat.le_max_left _ _)
    · exact hs.drecv _ _ e' he

theorem inv_of_reachable (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s) :
    Inv pol shardOf s := by
  induction h with
  | empty => exact inv_empty pol shardOf
  | step _ hst ih => exact inv_step pol dur shardOf hd ih hst

/-! ## The theorems -/

/-- **Grant uniqueness.** No `(shard, epoch)` is granted to two nodes, even by concurrent
authorities. -/
-- @tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]
theorem grant_unique (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    {a a' k e : Nat} {n n' : String} (h₁ : (a, n, k, e) ∈ s.granted)
    (h₂ : (a', n', k, e) ∈ s.granted) : n = n' :=
  (inv_of_reachable pol dur shardOf hd h).uniq a n k e a' n' h₁ h₂

/-- Every accepted append was made under an epoch granted to its writer for its shard. -/
theorem accepted_granted (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    {k : Nat} {x : Entry} (hx : x ∈ s.accepted k) :
    ∃ a, (a, x.writer, k, x.epoch) ∈ s.granted :=
  (inv_of_reachable pol dur shardOf hd h).agrant k x hx

/-- **At most one writer per shard and epoch.** -/
-- @tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]
-- @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Concurrent activation attempts]] {level: unit}
theorem one_writer_per_epoch (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    {k : Nat} {x y : Entry} (hx : x ∈ s.accepted k) (hy : y ∈ s.accepted k)
    (he : x.epoch = y.epoch) : x.writer = y.writer := by
  have hi := inv_of_reachable pol dur shardOf hd h
  obtain ⟨a, ha⟩ := hi.agrant k x hx
  obtain ⟨b, hb⟩ := hi.agrant k y hy
  rw [he] at ha
  exact hi.uniq a _ k _ b _ ha hb

/-- **Journal versions are ordered by epoch.** In each actor's journal, a later version never
carries a lower epoch, so no journal version can hold entries of two epochs out of order. -/
-- @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: unit}
theorem journal_epochs_ordered (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    (actor : String) (i j : Nat) (hi : i < (s.journals actor).length)
    (hj : j < (s.journals actor).length) (hij : i < j) :
    ((s.journals actor)[i]).epoch ≤ ((s.journals actor)[j]).epoch :=
  List.pairwise_iff_getElem.mp ((inv_of_reachable pol dur shardOf hd h).jsorted actor) i j hi hj hij

/-- **Once per actor.** No message is journaled twice for an actor, whatever the retries. -/
theorem message_once (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    (actor : String) : ((s.journals actor).map (·.message)).Nodup :=
  (inv_of_reachable pol dur shardOf hd h).jonce actor

/-- **Accepted appends of a shard are ordered by epoch**, across all its actors. -/
-- @tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]
theorem shard_epochs_ordered (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    (k : Nat) (i j : Nat) (hi : i < (s.accepted k).length) (hj : j < (s.accepted k).length)
    (hij : i < j) : ((s.accepted k)[i]).epoch ≤ ((s.accepted k)[j]).epoch :=
  List.pairwise_iff_getElem.mp ((inv_of_reachable pol dur shardOf hd h).asorted k) i j hi hj hij

/-- **Fencing backstop.** Once an append at epoch `e'` is accepted for a shard, an append at a
lower epoch is fenced, whatever version it expects. -/
-- @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Stale owner append fails after split brain]] {level: unit}
theorem stale_append_fenced (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    {k : Nat} {x : Entry} (hx : x ∈ s.accepted k) {e : Nat} (he : e < x.epoch)
    (actor : String) (version : Nat) (msg : String) : decide s k e actor version msg = .fenced := by
  have := (inv_of_reachable pol dur shardOf hd h).afence k x hx
  unfold decide
  simp only [Nat.lt_of_lt_of_le he this, ↓reduceIte]

/-- **Receivers take frames in epoch order**, each produced by an accepted append. -/
-- @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Fenced outbound messages are dropped]] {level: unit}
theorem frames_ordered (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    (r : String) (k : Nat) :
    (s.admitted r k).Pairwise (· ≤ ·) ∧ ∀ e ∈ s.admitted r k, ∃ x ∈ s.accepted k, x.epoch = e :=
  ⟨(inv_of_reachable pol dur shardOf hd h).dsorted r k,
    (inv_of_reachable pol dur shardOf hd h).dfrom r k⟩

/-- **Stale frames are dropped.** A frame below an epoch the receiver admitted is not
admitted. -/
-- @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
theorem stale_frame_dropped (hd : Disjoint pol) {s : Sys} (h : Reachable pol dur shardOf s)
    {r : String} {k e e' : Nat} (he' : e' ∈ s.admitted r k) (he : e < e') :
    admits s r k e = false := by
  have := (inv_of_reachable pol dur shardOf hd h).drecv r k e' he'
  simp only [admits, Bool.not_eq_false', decide_eq_true_eq]
  omega

end Vertep.Lease
