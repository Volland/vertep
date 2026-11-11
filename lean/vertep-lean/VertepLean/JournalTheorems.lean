import VertepLean.Journal

/-!
# The journal protocol theorems

All of them hold in every state reachable from the empty system (`Reachable`), whatever the
interleaving of nodes, crashes, redeliveries and dispatcher retries:

* `replay_determinism` — replaying the journal reproduces, at every position, the recorded
  sends and state digest; `replay_no_divergence` — replay with divergence detection succeeds;
  `replay_equals_live` — an activation built by replay equals a live activation at the head;
* `turn_idempotent` — a turn on a message id the activation has accepted changes nothing, so
  processing a message twice equals processing it once; `accepted_once` — no message id is
  ever accepted twice, even by concurrent activations;
* `outbox_safe` — only recorded sends are ever delivered; `outbox_complete` — the dispatcher
  can always deliver every recorded send, whatever crashed before; `no_message_lost` — a
  message is acknowledged only once it is journaled;
* `cas_single` and `turn_cas_exclusive` — of two appends, or two activations' turns, at the same
  expected position at most one succeeds.

The proof is one inductive invariant (`Inv`) preserved by every `Step`.
-/

namespace Vertep.Journal

variable {σ ι ε : Type} (M : Machine σ ι ε)

/-! ## Replay lemmas -/

theorem replayFrom_append (s : σ) (j : Journal σ ι ε) (e : Entry σ ι ε) :
    replayFrom M s (j ++ [e]) = (M.step (replayFrom M s j) e.msg.input).1 := by
  induction j generalizing s with
  | nil => rfl
  | cons x xs ih => exact ih _

theorem recordsFrom_append (s : σ) (j : Journal σ ι ε) (e : Entry σ ι ε) :
    RecordsFrom M s (j ++ [e]) ↔ RecordsFrom M s j ∧
      e.effects = (M.step (replayFrom M s j) e.msg.input).2 ∧
      e.digest = (M.step (replayFrom M s j) e.msg.input).1 := by
  induction j generalizing s with
  | nil => simp [RecordsFrom, replayFrom]
  | cons x xs ih =>
    simp only [List.cons_append, RecordsFrom, replayFrom]
    rw [ih]
    constructor
    · rintro ⟨h₁, h₂, h₃, h₄⟩; exact ⟨⟨h₁, h₂, h₃⟩, h₄⟩
    · rintro ⟨⟨h₁, h₂, h₃⟩, h₄⟩; exact ⟨h₁, h₂, h₃, h₄⟩

/-- Each recorded entry is what its macrostep produced from the replay of the entries before. -/
theorem recordsFrom_getElem (s : σ) (j : Journal σ ι ε) (h : RecordsFrom M s j) (k : Nat)
    (hk : k < j.length) :
    j[k].effects = (M.step (replayFrom M s (j.take k)) j[k].msg.input).2 ∧
      j[k].digest = replayFrom M s (j.take (k + 1)) := by
  induction j generalizing s k with
  | nil => simp at hk
  | cons x xs ih =>
    obtain ⟨h₁, h₂, h₃⟩ := h
    cases k with
    | zero => exact ⟨h₁, h₂⟩
    | succ k =>
      have := ih _ h₃ k (by simpa using hk)
      simpa [replayFrom] using this

theorem checkFrom_ok_iff [DecidableEq σ] [DecidableEq ε] (p : Nat) (s t : σ)
    (j : Journal σ ι ε) :
    checkFrom M p s j = .ok t ↔ RecordsFrom M s j ∧ t = replayFrom M s j := by
  induction j generalizing p s with
  | nil =>
    simp only [checkFrom, RecordsFrom, replayFrom, true_and]
    constructor
    · intro h; cases h; rfl
    · intro h; rw [h]
  | cons e es ih =>
    simp only [checkFrom, RecordsFrom, replayFrom]
    split
    · next hc =>
      rw [ih]
      constructor
      · rintro ⟨h₁, h₂⟩; exact ⟨⟨hc.1, hc.2, h₁⟩, h₂⟩
      · rintro ⟨⟨_, _, h₁⟩, h₂⟩; exact ⟨h₁, h₂⟩
    · next hc =>
      constructor
      · intro h; cases h
      · rintro ⟨⟨h₁, h₂, _⟩, _⟩; exact absurd ⟨h₁, h₂⟩ hc

/-! ## Send lemmas -/

theorem sendsFrom_append (p : Nat) (j k : Journal σ ι ε) :
    sendsFrom p (j ++ k) = sendsFrom p j ++ sendsFrom (p + j.length) k := by
  induction j generalizing p with
  | nil => simp [sendsFrom]
  | cons x xs ih =>
    simp only [List.cons_append, sendsFrom, ih, List.append_assoc, List.length_cons]
    congr 3; omega

/-- What the journal records is what was dispatched through the mark plus what is pending. -/
theorem recorded_split (st : Store σ ι ε) (h : st.mark ≤ st.journal.length) :
    recorded st.journal =
      sendsFrom 1 (st.journal.take st.mark) ++ pending st := by
  unfold recorded pending
  conv => lhs; rw [← List.take_append_drop st.mark st.journal]
  rw [sendsFrom_append, List.length_take, Nat.min_eq_left h, Nat.add_comm]

theorem mem_recorded_append {j : Journal σ ι ε} {x : SendId} (e : Entry σ ι ε)
    (h : x ∈ recorded j) : x ∈ recorded (j ++ [e]) := by
  unfold recorded at *
  rw [sendsFrom_append]; exact List.mem_append_left _ h

/-! ## Turns -/

/-- The three outcomes of a turn. -/
theorem turn_cases (st : Store σ ι ε) (a : Act σ) (m : Msg ι) :
    (m.id ∈ a.seen ∧ turn M st a m = (st, some a, true)) ∨
    (m.id ∉ a.seen ∧ st.journal.length = a.head ∧
      turn M st a m =
        ({ st with journal := st.journal ++
            [⟨m, (M.step a.state m.input).2, (M.step a.state m.input).1⟩] },
          some ⟨a.head + 1, (M.step a.state m.input).1, a.seen ++ [m.id]⟩, true)) ∨
    (m.id ∉ a.seen ∧ st.journal.length ≠ a.head ∧ turn M st a m = (st, none, false)) := by
  unfold turn casAppend
  by_cases hs : m.id ∈ a.seen
  · exact Or.inl ⟨hs, by simp [hs]⟩
  · by_cases hl : st.journal.length = a.head
    · exact Or.inr (Or.inl ⟨hs, hl, by simp [hs, hl]⟩)
    · exact Or.inr (Or.inr ⟨hs, hl, by simp [hs, hl]⟩)

/-! ## The invariant -/

/-- What holds in every reachable state. -/
structure Inv (s : Sys σ ι ε) : Prop where
  records : Records M s.store.journal
  acts : ∀ n a, s.nodes n = some a →
    a.head ≤ s.store.journal.length ∧ a.state = replay M (s.store.journal.take a.head) ∧
      a.seen = ids (s.store.journal.take a.head)
  nodup : (ids s.store.journal).Nodup
  delivered : ∀ x ∈ s.store.delivered, x ∈ recorded s.store.journal
  mark_le : s.store.mark ≤ s.store.journal.length
  marked : ∀ x ∈ sendsFrom 1 (s.store.journal.take s.store.mark), x ∈ s.store.delivered
  sent : ∀ m ∈ s.sent, m ∈ s.mailbox ∨ m.id ∈ ids s.store.journal

theorem inv_empty : Inv M (Sys.empty : Sys σ ι ε) where
  records := trivial
  acts := by intro n a h; cases h
  nodup := List.nodup_nil
  delivered := by intro x h; cases h
  mark_le := Nat.le_refl _
  marked := by intro x h; simp [Sys.empty, sendsFrom] at h
  sent := by intro m h; cases h

theorem setNode_eq {f : Nat → Option (Act σ)} {n k : Nat} {v : Option (Act σ)} {a : Act σ}
    (h : setNode f n v k = some a) : (k = n ∧ v = some a) ∨ (k ≠ n ∧ f k = some a) := by
  unfold setNode at h
  by_cases hk : k = n
  · simp only [hk, ↓reduceIte] at h; exact Or.inl ⟨hk, h⟩
  · simp only [hk, ↓reduceIte] at h; exact Or.inr ⟨hk, h⟩

theorem ids_append (j : Journal σ ι ε) (e : Entry σ ι ε) : ids (j ++ [e]) = ids j ++ [e.msg.id] := by
  simp [ids]

theorem inv_step [DecidableEq ι] {s t : Sys σ ι ε} (hs : Inv M s) (h : Step M s t) : Inv M t := by
  cases h with
  | submit m =>
    refine { hs with sent := ?_ }
    intro m' hm'
    rcases List.mem_append.mp hm' with h | h
    · rcases hs.sent m' h with h' | h'
      · exact Or.inl (List.mem_append_left _ h')
      · exact Or.inr h'
    · simp at h; subst h; exact Or.inl (by simp)
  | redeliver m _ =>
    refine { hs with sent := ?_ }
    intro m' hm'
    rcases hs.sent m' hm' with h' | h'
    · exact Or.inl (List.mem_append_left _ h')
    · exact Or.inr h'
  | activate n =>
    refine { hs with acts := ?_ }
    intro k a hk
    rcases setNode_eq hk with ⟨_, h⟩ | ⟨_, h⟩
    · cases h
      exact ⟨Nat.le_refl _, by simp [activate, List.take_length],
        by simp [activate, List.take_length]⟩
    · exact hs.acts k a h
  | crash n =>
    refine { hs with acts := ?_ }
    intro k a hk
    rcases setNode_eq hk with ⟨_, h⟩ | ⟨_, h⟩
    · cases h
    · exact hs.acts k a h
  | turn n a m ha hm =>
    obtain ⟨hhead, hstate, hseen⟩ := hs.acts n a ha
    rcases turn_cases M s.store a m with ⟨hin, he⟩ | ⟨hin, hl, he⟩ | ⟨_, _, he⟩
    · -- dropped: the id was accepted before
      simp only [he]
      refine { hs with acts := ?_, sent := ?_ }
      · intro k b hk
        rcases setNode_eq hk with ⟨hk', h⟩ | ⟨_, h⟩
        · cases h; subst hk'; exact hs.acts _ a ha
        · exact hs.acts k b h
      · intro m' hm'
        simp only [↓reduceIte]
        by_cases hmm : m' = m
        · subst hmm; right
          rw [hseen] at hin
          rw [← List.take_append_drop a.head s.store.journal]
          unfold ids at *; rw [List.map_append]; exact List.mem_append_left _ hin
        · rcases hs.sent m' hm' with h' | h'
          · exact Or.inl ((List.mem_erase_of_ne hmm).mpr h')
          · exact Or.inr h'
    · -- appended at the activation's head
      simp only [he]
      have htake : s.store.journal.take a.head = s.store.journal := by
        rw [← hl]; exact List.take_length
      have hst : a.state = replay M s.store.journal := by rw [hstate, htake]
      have hsn : a.seen = ids s.store.journal := by rw [hseen, htake]
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
      · show RecordsFrom M M.init _
        rw [recordsFrom_append]
        have hst' : a.state = replayFrom M M.init s.store.journal := hst
        exact ⟨hs.records, by rw [← hst'], by rw [← hst']⟩
      · intro k b hk
        rcases setNode_eq hk with ⟨_, h⟩ | ⟨_, h⟩
        · cases h
          have hfull : ∀ x : Entry σ ι ε,
              List.take (a.head + 1) (s.store.journal ++ [x]) = s.store.journal ++ [x] := by
            intro x; apply List.take_of_length_le; simp; omega
          refine ⟨by simp; omega, ?_, ?_⟩
          · show (M.step a.state m.input).1 =
              replayFrom M M.init (List.take (a.head + 1) (s.store.journal ++ _))
            have hst' : a.state = replayFrom M M.init s.store.journal := hst
            rw [hfull, replayFrom_append, ← hst']
          · show a.seen ++ [m.id] = ids (List.take (a.head + 1) (s.store.journal ++ _))
            rw [hfull, ids_append, hsn]
        · obtain ⟨b1, b2, b3⟩ := hs.acts k b h
          refine ⟨by simp; omega, ?_, ?_⟩
          · show _ = replay M (List.take b.head (s.store.journal ++ _))
            rw [List.take_append_of_le_length b1]; exact b2
          · show _ = ids (List.take b.head (s.store.journal ++ _))
            rw [List.take_append_of_le_length b1]; exact b3
      · show (ids (s.store.journal ++ _)).Nodup
        rw [ids_append, List.nodup_append]
        refine ⟨hs.nodup, List.nodup_cons.mpr ⟨by simp, List.nodup_nil⟩, ?_⟩
        intro x hx y hy hxy
        simp at hy; subst hy; subst hxy
        exact hin (hsn ▸ hx)
      · intro x hx; exact mem_recorded_append _ (hs.delivered x hx)
      · show s.store.mark ≤ (s.store.journal ++ _).length
        simp; have := hs.mark_le; omega
      · intro x hx
        apply hs.marked
        rw [List.take_append_of_le_length hs.mark_le] at hx; exact hx
      · intro m' hm'
        simp only [↓reduceIte]
        by_cases hmm : m' = m
        · subst hmm; right; show m'.id ∈ ids (s.store.journal ++ _); rw [ids_append]; simp
        · rcases hs.sent m' hm' with h' | h'
          · exact Or.inl ((List.mem_erase_of_ne hmm).mpr h')
          · right; show m'.id ∈ ids (s.store.journal ++ _); rw [ids_append]
            exact List.mem_append_left _ h'
    · -- compare-and-set lost: the activation steps down, nothing else changes
      simp only [he]
      refine { hs with acts := ?_ }
      intro k b hk
      rcases setNode_eq hk with ⟨_, h⟩ | ⟨_, h⟩
      · cases h
      · exact hs.acts k b h
  | deliver l hl =>
    refine { hs with delivered := ?_, marked := ?_ }
    · intro x hx
      rcases List.mem_append.mp hx with h | h
      · exact hs.delivered x h
      · rw [recorded_split s.store hs.mark_le]; exact List.mem_append_right _ (hl x h)
    · intro x hx; exact List.mem_append_left _ (hs.marked x hx)
  | markThrough k _ h₂ h =>
    exact { hs with mark_le := h₂, marked := h }

theorem inv_of_reachable [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) : Inv M s := by
  induction h with
  | empty => exact inv_empty M
  | step _ hst ih => exact inv_step M ih hst

/-! ## Replay determinism (verification-backlog 5.2) -/

/-- **Replay determinism.** In every reachable state, replaying the journal reproduces each
entry: its sends are what its macrostep produces from the replay of the entries before it, and
its state digest is the replay of the journal through it. -/
-- @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay equals recorded state]] {level: unit}
theorem replay_determinism [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) (k : Nat)
    (hk : k < s.store.journal.length) :
    s.store.journal[k].effects =
        (M.step (replay M (s.store.journal.take k)) s.store.journal[k].msg.input).2 ∧
      s.store.journal[k].digest = replay M (s.store.journal.take (k + 1)) :=
  recordsFrom_getElem M _ _ (inv_of_reachable M h).records k hk

/-- **No replay divergence.** In every reachable state, replay with divergence detection (Rust
`Node::load`) succeeds with the replayed state. -/
-- @tg: verifies:: [[openspec:event-journal#Deterministic Replay]] {level: unit}
theorem replay_no_divergence [DecidableEq ι] [DecidableEq σ] [DecidableEq ε] {s : Sys σ ι ε}
    (h : Reachable M s) : check M s.store.journal = .ok (replay M s.store.journal) :=
  (checkFrom_ok_iff M 1 _ _ _).mpr ⟨(inv_of_reachable M h).records, rfl⟩

/-- **Replay equals the live state.** An activation built by replaying the journal equals any
live activation that has seen the whole journal: same head, state and accepted ids. -/
-- @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay equals recorded state]] {level: unit}
theorem replay_equals_live [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) {n : Nat}
    {a : Act σ} (hn : s.nodes n = some a) (hh : a.head = s.store.journal.length) :
    activate M s.store.journal = a := by
  obtain ⟨_, h₂, h₃⟩ := (inv_of_reachable M h).acts n a hn
  rw [hh, List.take_length] at h₂ h₃
  cases a; simp only at hh h₂ h₃; subst hh h₂ h₃; rfl

/-! ## Deduplication idempotence (5.3) -/

/-- **Deduplication idempotence.** After a turn on a message, the same message again (a
redelivery) changes nothing: processing a message twice equals processing it once. -/
-- @tg: verifies:: [[openspec:event-journal#Message Deduplication#Redelivered message dropped]] {level: unit}
theorem turn_idempotent (st : Store σ ι ε) (a : Act σ) (m : Msg ι) (a' : Act σ)
    (h : (turn M st a m).2.1 = some a') :
    turn M (turn M st a m).1 a' m = ((turn M st a m).1, some a', true) := by
  rcases turn_cases M st a m with ⟨hin, he⟩ | ⟨_, _, he⟩ | ⟨_, _, he⟩
  · rw [he] at h ⊢; cases h; simp [turn, hin]
  · rw [he] at h ⊢; cases h; simp [turn]
  · rw [he] at h; cases h

/-- **Accepted at most once.** In every reachable state no message id appears twice in the
journal, whatever the redeliveries and concurrent activations. -/
-- @tg: verifies:: [[openspec:event-journal#Message Deduplication]] {level: unit}
theorem accepted_once [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) :
    (ids s.store.journal).Nodup :=
  (inv_of_reachable M h).nodup

/-! ## The outbox (5.4) -/

/-- **Outbox safety.** Only recorded sends are ever delivered: a send is dispatched only after
the entry recording it was appended. -/
-- @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash before append]] {level: unit}
theorem outbox_safe [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) :
    ∀ x ∈ s.store.delivered, x ∈ recorded s.store.journal :=
  (inv_of_reachable M h).delivered

/-- **Outbox completeness.** From every reachable state, whatever crashed before, one dispatcher
step delivers every recorded send not delivered yet: every recorded send is delivered at least
once. -/
-- @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash after append, before dispatch]] {level: unit}
theorem outbox_complete [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) :
    ∃ t, Step M s t ∧ t.store.journal = s.store.journal ∧
      ∀ x ∈ recorded t.store.journal, x ∈ t.store.delivered := by
  have hi := inv_of_reachable M h
  refine ⟨_, Step.deliver s (pending s.store) (fun _ hx => hx), rfl, ?_⟩
  intro x hx
  simp only at hx ⊢
  rw [recorded_split s.store hi.mark_le] at hx
  rcases List.mem_append.mp hx with h' | h'
  · exact List.mem_append_left _ (hi.marked x h')
  · exact List.mem_append_right _ h'

/-- **No message lost.** Every submitted message is still in the mailbox (so it is redelivered)
or was journaled: a message is acknowledged only after its entry is appended, and a crash
before the append leaves it to be redelivered. -/
-- @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash before append]] {level: unit}
theorem no_message_lost [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s) :
    ∀ m ∈ s.sent, m ∈ s.mailbox ∨ m.id ∈ ids s.store.journal :=
  (inv_of_reachable M h).sent

/-! ## Compare-and-set single activation (5.5) -/

/-- **Compare-and-set.** Of two appends at the same expected position, at most one succeeds. -/
-- @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Conflicting append rejected]] {level: unit}
theorem cas_single (j j₁ : Journal σ ι ε) (k : Nat) (e₁ e₂ : Entry σ ι ε)
    (h : casAppend j k e₁ = some j₁) : casAppend j₁ k e₂ = none := by
  unfold casAppend at *
  by_cases hk : j.length = k
  · simp only [hk, ↓reduceIte] at h; cases h; simp [← hk]
  · simp only [hk, ↓reduceIte] at h; cases h

/-- **Single activation.** Two activations at the same head (say on two nodes): once one has
appended a turn, the other's turn on a new message loses compare-and-set, steps down, and
leaves the store unchanged and the message unacknowledged. -/
-- @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Losing activation steps down]] {level: unit}
theorem turn_cas_exclusive (st : Store σ ι ε) (a₁ a₂ : Act σ) (m₁ m₂ : Msg ι)
    (h₁ : m₁.id ∉ a₁.seen) (happ : (turn M st a₁ m₁).2.2 = true) (hh : a₂.head = a₁.head)
    (h₂ : m₂.id ∉ a₂.seen) :
    turn M (turn M st a₁ m₁).1 a₂ m₂ = ((turn M st a₁ m₁).1, none, false) := by
  rcases turn_cases M st a₁ m₁ with ⟨hin, _⟩ | ⟨_, hl, he⟩ | ⟨_, _, he⟩
  · exact absurd hin h₁
  · rw [he]
    rcases turn_cases M { st with journal := st.journal ++
        [⟨m₁, (M.step a₁.state m₁.input).2, (M.step a₁.state m₁.input).1⟩] } a₂ m₂ with
      ⟨hin, _⟩ | ⟨_, hl', _⟩ | ⟨_, _, he'⟩
    · exact absurd hin h₂
    · simp at hl'; omega
    · exact he'
  · rw [he] at happ; cases happ

end Vertep.Journal
