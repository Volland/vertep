import VertepLean.Agent

/-!
# Decider safety, decision replay and grant enforcement (m3-agents 8.2 to 8.4)

* `decider_safety` — whatever a decider returns, a transition taken because of a decision is
  one that was enabled when the candidate set was offered (in the context the offer was computed
  in, which the agent was really in) and is the one selection takes now; `stale_not_taken` and
  `unknown_not_taken` are its two most common instances;
* `decision_dedup`, `decision_once` — a decision delivered again is dropped;
* `replay_live` — replaying the journal of a live run, which records every decision and every
  model or tool result as an input, reaches the same state with no decider or provider at all;
* `chainGrant_mem`, `grants_do_not_escalate` — the effective grant is the intersection of the
  grants declared along the invoke chain; `grant_enforced`, `ungranted_not_delivered` — a send
  is delivered to a named target or the parent only if the grant allows it (the parent also
  receives the `vt.grant.denied` notice of a refused send).
-/

namespace Vertep.Agent

variable {κ π : Type} (C : Core κ π)

/-! ## The invariant -/

theorem reoffer_ctx (s : AState κ) : (reoffer C s).ctx = s.ctx := by
  unfold reoffer; split
  · split
    · split <;> rfl
    · rfl
  · rfl

theorem reoffer_hist (s : AState κ) : (reoffer C s).hist = s.hist := by
  unfold reoffer; split
  · split
    · split <;> rfl
    · rfl
  · rfl

theorem reoffer_answered (s : AState κ) : (reoffer C s).answered = s.answered := by
  unfold reoffer; split
  · split
    · split <;> rfl
    · rfl
  · rfl

/-- After `reoffer`, an open offer is the candidate set of the current context. -/
theorem reoffer_slot (s : AState κ) (o : Offer κ) (h : (reoffer C s).slot = some o) :
    o.cands = enabledEvents C s.ctx ∧ (o.ctx = s.ctx ∨ s.slot = some o) := by
  unfold reoffer at h; split at h
  · split at h
    · next o' hs =>
      split at h
      · next he => rw [hs] at h; cases h; exact ⟨he, Or.inr hs⟩
      · cases h; exact ⟨rfl, Or.inl rfl⟩
    · cases h; exact ⟨rfl, Or.inl rfl⟩
  · cases h

theorem reoffer_spec (s : AState κ) :
    (C.active s.ctx = true → ∃ o, (reoffer C s).slot = some o ∧ o.cands = enabledEvents C s.ctx) ∧
      (C.active s.ctx = false → (reoffer C s).slot = none) := by
  constructor
  · intro ha
    unfold reoffer
    simp only [ha, ↓reduceIte]
    split
    · next o hs =>
      split
      · next he => exact ⟨o, hs, he⟩
      · exact ⟨_, rfl, rfl⟩
    · exact ⟨_, rfl, rfl⟩
  · intro ha
    unfold reoffer
    simp only [ha, Bool.false_eq_true, ↓reduceIte]

theorem reoffer_fixed (r : AState κ)
    (h₁ : C.active r.ctx = true → ∃ o, r.slot = some o ∧ o.cands = enabledEvents C r.ctx)
    (h₂ : C.active r.ctx = false → r.slot = none) : reoffer C r = r := by
  unfold reoffer
  split
  · next ha =>
    obtain ⟨o, ho, hc⟩ := h₁ ha
    split
    · next o' hs =>
      rw [ho] at hs; cases hs
      split
      · rfl
      · next hne => exact absurd hc hne
    · next hs => rw [ho] at hs; cases hs
  · next ha =>
    have := h₂ (by simpa using ha)
    cases r; simp only at this ⊢; rw [this]

theorem reoffer_idem (s : AState κ) : reoffer C (reoffer C s) = reoffer C s := by
  obtain ⟨h₁, h₂⟩ := reoffer_spec C s
  apply reoffer_fixed
  · rw [reoffer_ctx]; exact h₁
  · rw [reoffer_ctx]; exact h₂

theorem reoffer_remember (s : AState κ) : reoffer C (remember s) = remember (reoffer C s) := by
  obtain ⟨ctx, slot, ans, nx, hist⟩ := s
  unfold reoffer remember
  simp only
  by_cases ha : C.active ctx = true
  · simp only [ha, ↓reduceIte]
    cases slot with
    | none => rfl
    | some o =>
      by_cases hc : o.cands = enabledEvents C ctx
      · simp only [hc, ↓reduceIte]
      · simp only [hc, ↓reduceIte]
  · simp only [ha, Bool.false_eq_true, ↓reduceIte]

/-- What holds in every reachable state. -/
structure Inv (s : AState κ) : Prop where
  offer : ∀ o, s.slot = some o → o.cands = enabledEvents C o.ctx ∧ o.ctx ∈ s.hist
  stable : reoffer C s = s

theorem inv_remember_reoffer (s : AState κ)
    (hs : ∀ o, s.slot = some o → o.cands = enabledEvents C o.ctx ∧ o.ctx ∈ s.hist) :
    Inv C (remember (reoffer C s)) := by
  constructor
  · intro o ho
    have ho' : (reoffer C s).slot = some o := ho
    obtain ⟨h₁, h₂⟩ := reoffer_slot C s o ho'
    simp only [remember, reoffer_hist, reoffer_ctx]
    rcases h₂ with h₂ | h₂
    · rw [h₂]; exact ⟨h₁, List.mem_append_right _ (List.mem_singleton_self _)⟩
    · obtain ⟨h₃, h₄⟩ := hs o h₂
      exact ⟨h₃, List.mem_append_left _ h₄⟩
  · rw [reoffer_remember, reoffer_idem]

theorem decide_slot (s : AState κ) (corr : Nat) (e : String) (p : π) (o : Offer κ)
    (h : (decide C s corr e p).1.slot = some o) : s.slot = some o := by
  unfold decide at h
  split at h
  · exact h
  · split at h
    · next o' hs =>
      split at h
      · split at h
        · cases h
        · split at h
          · cases h
          · split at h <;> cases h
      · exact hs ▸ h
    · next hs => exact hs ▸ h

theorem decide_hist (s : AState κ) (corr : Nat) (e : String) (p : π) :
    (decide C s corr e p).1.hist = s.hist := by
  unfold decide
  split
  · rfl
  · split
    · split
      · split
        · rfl
        · split
          · rfl
          · split <;> rfl
      · rfl
    · rfl

theorem inv_step {s : AState κ} (hs : Inv C s) (i : Input π) : Inv C (stepA C s i).1 := by
  cases i with
  | event n p =>
    exact inv_remember_reoffer C _ (fun o ho => hs.offer o ho)
  | decision corr e p =>
    refine inv_remember_reoffer C _ (fun o ho => ?_)
    rw [decide_hist]; exact hs.offer o (decide_slot C s corr e p o ho)
  | refusal corr =>
    exact inv_remember_reoffer C _ (fun o ho => hs.offer o ho)

theorem inv_of_reachable {s : AState κ} (h : Reachable C s) : Inv C s := by
  induction h with
  | init k => exact inv_remember_reoffer C _ (fun o ho => by cases ho)
  | step i _ ih => exact inv_step C ih i

/-! ## Decider safety (8.2) -/

theorem mem_enabledEvents {k : κ} {c : Cand} (h : c ∈ enabledEvents C k) :
    C.probe k c.event = some c.trans := by
  unfold enabledEvents at h
  obtain ⟨n, _, hn⟩ := List.mem_filterMap.mp h
  cases hp : C.probe k n with
  | none => rw [hp] at hn; cases hn
  | some t => rw [hp] at hn; cases hn; exact hp

/-- **Decider safety.** For every reachable state and every decider output, a transition taken
because of a decision is a candidate of the open offer with that correlation id, was enabled
when the offer was made, in a context the agent was really in, and is the transition selection
takes now. -/
-- @tg: implements:: [[openspec:deciders#Decider Safety]]
-- @tg: verifies:: [[openspec:deciders#Decider Safety#Safety theorem checked in CI]] {level: unit}
theorem decider_safety {s : AState κ} (h : Reachable C s) {corr : Nat} {e : String} {p : π}
    {c : Cand} (ht : (stepA C s (.decision corr e p)).2 = .taken c) :
    ∃ o, s.slot = some o ∧ o.corr = corr ∧ c ∈ o.cands ∧ c.event = e ∧
      C.probe o.ctx e = some c.trans ∧ o.ctx ∈ s.hist ∧ C.select s.ctx e p = some c.trans := by
  have hi := inv_of_reachable C h
  simp only [stepA] at ht
  unfold decide at ht
  split at ht
  · cases ht
  · split at ht
    · next o hs =>
      split at ht
      · next hc =>
        split at ht
        · cases ht
        · next c' hf =>
          split at ht
          · cases ht
          · split at ht
            · next hsel =>
              cases ht
              have hmem := List.mem_of_find?_eq_some hf
              have hev : c.event = e := by simpa using List.find?_some hf
              obtain ⟨h₁, h₂⟩ := hi.offer o hs
              have hp := mem_enabledEvents C (h₁ ▸ hmem)
              exact ⟨o, hs, hc, hmem, hev, hev ▸ hp, h₂, hsel⟩
            · cases ht
      · cases ht
    · cases ht

/-- A decision whose correlation id is not the open offer's (stale, or from nowhere) takes no
transition. -/
-- @tg: verifies:: [[openspec:deciders#Decision Validation Against Candidates#Stale decision rejected]] {level: unit}
theorem stale_not_taken {s : AState κ} (h : Reachable C s) {corr : Nat} {e : String} {p : π}
    (hs : ∀ o, s.slot = some o → o.corr ≠ corr) (c : Cand) :
    (stepA C s (.decision corr e p)).2 ≠ .taken c := by
  intro ht
  obtain ⟨o, ho, hc, _⟩ := decider_safety C h ht
  exact hs o ho hc

/-- A decision for an event that was not offered takes no transition. -/
-- @tg: verifies:: [[openspec:deciders#Decision Validation Against Candidates#Decision outside the candidate set rejected]] {level: unit}
theorem unknown_not_taken {s : AState κ} (h : Reachable C s) {corr : Nat} {e : String} {p : π}
    (hs : ∀ o, s.slot = some o → ∀ c ∈ o.cands, c.event ≠ e) (c : Cand) :
    (stepA C s (.decision corr e p)).2 ≠ .taken c := by
  intro ht
  obtain ⟨o, ho, _, hm, he, _⟩ := decider_safety C h ht
  exact hs o ho c hm he

/-! ## Deduplication -/

/-- A decision whose correlation id was answered is dropped: nothing changes but the ghost
history. -/
-- @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Duplicate decision deduplicated]] {level: unit}
theorem decision_dedup {s : AState κ} (h : Reachable C s) {corr : Nat} (e : String) (p : π)
    (ha : corr ∈ s.answered) :
    stepA C s (.decision corr e p) = (remember s, .none) := by
  have hi := inv_of_reachable C h
  simp only [stepA, decide, ha, ↓reduceIte, hi.stable]

theorem decide_answers (s : AState κ) (corr : Nat) (e : String) (p : π) :
    corr ∈ (decide C s corr e p).1.answered := by
  unfold decide
  split
  · assumption
  · split
    · split
      · split
        · exact List.mem_cons_self
        · split
          · exact List.mem_cons_self
          · split <;> exact List.mem_cons_self
      · exact List.mem_cons_self
    · exact List.mem_cons_self

/-- **Exactly once.** Delivering the same decision twice equals delivering it once. -/
-- @tg: implements:: [[openspec:deciders#Decisions Are Journaled Inputs]]
theorem decision_once {s : AState κ} (h : Reachable C s) (corr : Nat) (e : String) (p : π) :
    stepA C (stepA C s (.decision corr e p)).1 (.decision corr e p) =
      (remember (stepA C s (.decision corr e p)).1, .none) := by
  apply decision_dedup C (Reachable.step _ h)
  simp only [stepA, remember, reoffer_answered]
  exact decide_answers C s corr e p

/-! ## Replay with journaled decisions and results (8.4) -/

/-- **Replay without deciders.** For every environment of deciders, models and tools, replaying
the journal of a live run from the same start reaches the same state; `replay` takes no
decider or provider, so none is consulted. -/
-- @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Replay does not call the decider]] {level: unit}
-- @tg: verifies:: [[openspec:llm-actors#Journaled Model Results#Replay without provider]] {level: unit}
theorem replay_live (R : AState κ → Option (Input π)) (s : AState κ) (is : List (Input π)) :
    replay C s (live C R s is).2 = (live C R s is).1 := by
  induction is generalizing s with
  | nil => rfl
  | cons i is ih =>
    simp only [live]
    split
    · next d _ => simp only [replay, List.foldl_cons]; exact ih _
    · simp only [replay, List.foldl_cons]; exact ih _

/-! ## Grants (8.3) -/

theorem effective_mem (a d : Option Grant) (t : String) :
    (∀ x, effective a d = some x → t ∈ x) ↔
      (∀ x, a = some x → t ∈ x) ∧ (∀ x, d = some x → t ∈ x) := by
  cases a <;> cases d <;> simp [effective]

theorem foldl_effective_mem (ds : List (Option Grant)) (a : Option Grant) (g : Grant)
    (h : ds.foldl effective a = some g) (t : String) :
    t ∈ g ↔ (∀ x, a = some x → t ∈ x) ∧ ∀ d ∈ ds, ∀ g', d = some g' → t ∈ g' := by
  induction ds generalizing a with
  | nil =>
    simp only [List.foldl_nil] at h; subst h
    simp
  | cons d ds ih =>
    simp only [List.foldl_cons] at h
    rw [ih _ h, effective_mem]
    simp only [List.mem_cons, forall_eq_or_imp]
    constructor
    · rintro ⟨⟨h₁, h₂⟩, h₃⟩; exact ⟨h₁, h₂, h₃⟩
    · rintro ⟨h₁, h₂, h₃⟩; exact ⟨⟨h₁, h₂⟩, h₃⟩

/-- **Intersection.** A target is in the effective grant at the end of an invoke chain exactly
when every grant declared along the chain allows it. -/
-- @tg: implements:: [[openspec:llm-actors#Capability Grants]]
theorem chainGrant_mem (ds : List (Option Grant)) (g : Grant) (h : chainGrant ds = some g)
    (t : String) : t ∈ g ↔ ∀ d ∈ ds, ∀ g', d = some g' → t ∈ g' := by
  rw [foldl_effective_mem ds none g h t]; simp

/-- **Grants do not escalate.** A child invoked with declared grant `H` by an actor with
effective grant `G` has effective grant `G ∩ H`. -/
-- @tg: verifies:: [[openspec:llm-actors#Capability Grants#Grants do not escalate]] {level: unit}
theorem grants_do_not_escalate (ds : List (Option Grant)) (G H g : Grant)
    (hG : chainGrant ds = some G) (hg : chainGrant (ds ++ [some H]) = some g) (t : String) :
    t ∈ g ↔ t ∈ G ∧ t ∈ H := by
  rw [chainGrant_mem _ g hg, chainGrant_mem _ G hG]
  constructor
  · intro h'
    exact ⟨fun d hd g' hg' => h' d (List.mem_append_left _ hd) g' hg',
      h' (some H) (List.mem_append_right _ (List.mem_singleton_self _)) H rfl⟩
  · rintro ⟨h₁, h₂⟩ d hd g' hg'
    rcases List.mem_append.mp hd with hd | hd
    · exact h₁ d hd g' hg'
    · simp at hd; subst hd; cases hg'; exact h₂

/-- **Grant enforcement.** Every send `filter` lets through to a named target or the parent is
allowed by the grant, except the `vt.grant.denied` notice to the parent. -/
-- @tg: implements:: [[openspec:llm-actors#Capability Grants]]
-- @tg: verifies:: [[openspec:llm-actors#Capability Grants#Ungranted send fails]] {level: unit}
theorem grant_enforced (g : Grant) (outs : List Out) (o : Out) (h : o ∈ filter g outs)
    (t : String) (ht : o.target.name = some t) :
    t ∈ g ∨ (o.target = .parent ∧ o.event = "vt.grant.denied") := by
  unfold filter at h
  obtain ⟨o', _, h'⟩ := List.mem_flatMap.mp h
  split at h'
  · next t' ht' =>
    split at h'
    · next hin => simp at h'; subst h'; rw [ht] at ht'; cases ht'; exact Or.inl hin
    · simp at h'
      rcases h' with h' | h'
      · subst h'; simp [Target.name] at ht
      · subst h'; exact Or.inr ⟨rfl, rfl⟩
  · next hn => simp at h'; subst h'; rw [hn] at ht; cases ht

/-- **Nothing reaches an ungranted actor.** -/
-- @tg: verifies:: [[openspec:llm-actors#Capability Grants#Granted send succeeds]] {level: unit}
theorem ungranted_not_delivered (g : Grant) (outs : List Out) (t : String) (ht : t ∉ g)
    (o : Out) (h : o ∈ filter g outs) : o.target ≠ .named t := by
  intro he
  rcases grant_enforced g outs o h t (by rw [he]; rfl) with h' | ⟨h', _⟩
  · exact ht h'
  · rw [he] at h'; cases h'

/-- **Granted sends are delivered.** -/
-- @tg: implements:: [[openspec:llm-actors#Capability Grants]]
theorem granted_delivered (g : Grant) (outs : List Out) (o : Out) (ho : o ∈ outs)
    (hg : ∀ t, o.target.name = some t → t ∈ g) : o ∈ filter g outs := by
  unfold filter
  refine List.mem_flatMap.mpr ⟨o, ho, ?_⟩
  split
  · next t ht => simp only [hg t ht, ↓reduceIte]; exact List.mem_singleton_self _
  · exact List.mem_singleton_self _

end Vertep.Agent
