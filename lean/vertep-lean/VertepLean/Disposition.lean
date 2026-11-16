import VertepLean.AgentTheorems

/-!
# Decision dispositions (m12-decision-records 2.3)

What the shell records on a decision or refusal entry before it journals the input
(`vertep_agent::journal::Disposition`, set by `AgentMachine::disposition_of`), computed in the
decider model of `VertepLean.Agent` from the open offer and the state, with the same checks as
`decide` and without changing anything:

* `dispositionOf` — a decision: `stale` when no open offer has its correlation id;
  `refused-invalid` when its annotations fail validation; `refused-not-offered` when its event is
  not a candidate; `refused-invalid` when its payload does not conform; `refused-guard` when
  selection no longer takes the offered transition; otherwise `accepted`, or
  `fallback-on-deadline` when the open offer was made to the fallback decider after the deadline;
* `refusalDisposition` — a refusal is `refused-by-decider`;
* `offeredFor` — the events of the open offer with that correlation id, in offer order (empty
  when there is none), which the entry records as `offered`.

The theorems:

* `disposition_of_decide` — with valid annotations, the disposition of a decision that was not
  answered before is a function of the outcome `decide` reaches (`ofOutcome`): the shell records
  the result of the validation the machine performs, never something derived from the next
  state;
* `accepted_was_offered` — an `accepted` (or `fallback-on-deadline`) decision names an event of
  the open offer with its correlation id, listed in the recorded offer, and selection takes the
  offered transition; `accepted_safe` adds, for reachable states, that the transition was enabled
  when the offer was made, in a context the agent was really in (`decider_safety`);
* `not_offered_not_listed`, `stale_offers_nothing` — a `refused-not-offered` event is not in the
  recorded offer, and a `stale` answer records an empty one;
* `fallback_needs_fallback_offer` — `fallback-on-deadline` only on an offer made to the fallback;
* `fallback_on_refusal_not_produced` — no decision and no refusal is `fallback-on-refusal`: the
  value is reserved for a chart path that consults a fallback after a refusal, which does not
  exist.
-/

namespace Vertep.Agent

/-- `vertep_agent::journal::Disposition`. -/
inductive Disposition where
  | accepted
  | refusedNotOffered
  | refusedGuard
  | refusedInvalid
  | stale
  | fallbackOnDeadline
  | fallbackOnRefusal
  | refusedByDecider
  deriving DecidableEq, Repr

/-- The disposition's name as it appears in the journal (Rust `Disposition::as_str`). -/
def Disposition.name : Disposition → String
  | .accepted => "accepted"
  | .refusedNotOffered => "refused-not-offered"
  | .refusedGuard => "refused-guard"
  | .refusedInvalid => "refused-invalid"
  | .stale => "stale"
  | .fallbackOnDeadline => "fallback-on-deadline"
  | .fallbackOnRefusal => "fallback-on-refusal"
  | .refusedByDecider => "refused-by-decider"

/-- The disposition a validation outcome of `decide` stands for; `none` for a duplicate (which
the host drops before it journals anything) and for a reason `decide` does not produce. -/
def ofOutcome (fallback : Bool) : Outcome → Option Disposition
  | .taken _ => some (if fallback then .fallbackOnDeadline else .accepted)
  | .rejected r =>
    if r = "stale" then some .stale
    else if r = "not-a-candidate" then some .refusedNotOffered
    else if r = "payload-schema" then some .refusedInvalid
    else if r = "not-enabled" then some .refusedGuard
    else none
  | .none => none

section
variable {κ π : Type} (C : Core κ π)

/-- The events of the open offer with correlation id `corr`, in offer order; empty when no open
offer has it. -/
def offeredFor (s : AState κ) (corr : Nat) : List String :=
  match s.slot with
  | some o => if o.corr = corr then o.cands.map (·.event) else []
  | none => []

/-- Rust `AgentMachine::disposition_of` on a decision: `annOk` is whether its annotations
validate, `fallback` whether the open offer was made to the fallback decider. -/
-- @tg: implements:: [[openspec:decision-records#Decision Disposition Is Recorded]] {note: "Lean model"}
def dispositionOf (s : AState κ) (corr : Nat) (e : String) (p : π) (annOk fallback : Bool) :
    Disposition :=
  match s.slot with
  | some o =>
    if o.corr = corr then
      if !annOk then .refusedInvalid
      else match o.cands.find? (·.event = e) with
        | none => .refusedNotOffered
        | some c =>
          if !C.conforms e p then .refusedInvalid
          else if C.select s.ctx e p = some c.trans then
            (if fallback then .fallbackOnDeadline else .accepted)
          else .refusedGuard
    else .stale
  | none => .stale

/-- Rust `AgentMachine::disposition_of` on a refusal. -/
def refusalDisposition : Disposition := .refusedByDecider

/-- **The disposition is the validation's.** With valid annotations, the disposition of a
decision that was not answered before is the one the outcome of `decide` stands for. -/
theorem disposition_of_decide (s : AState κ) {corr : Nat} (ha : corr ∉ s.answered) (e : String)
    (p : π) (fallback : Bool) :
    ofOutcome fallback (decide C s corr e p).2 = some (dispositionOf C s corr e p true fallback) := by
  unfold decide dispositionOf
  simp only [ha, ↓reduceIte]
  cases hs : s.slot with
  | none => simp [ofOutcome]
  | some o =>
    by_cases hc : o.corr = corr
    · simp only [hc, ↓reduceIte, Bool.not_true, Bool.false_eq_true]
      cases hf : o.cands.find? (·.event = e) with
      | none => simp [ofOutcome]
      | some c =>
        by_cases hk : C.conforms e p = true
        · by_cases hsel : C.select s.ctx e p = some c.trans
          · simp [hk, hsel, ofOutcome]
          · simp [hk, hsel, ofOutcome]
        · simp [hk, ofOutcome]
    · simp [hc, ofOutcome]

/-- An `accepted` or `fallback-on-deadline` decision names an event of the open offer with its
correlation id, listed in the recorded offer, and selection takes the offered transition. -/
theorem accepted_was_offered {s : AState κ} {corr : Nat} {e : String} {p : π} {annOk fb : Bool}
    (h : dispositionOf C s corr e p annOk fb = .accepted ∨
      dispositionOf C s corr e p annOk fb = .fallbackOnDeadline) :
    ∃ o c, s.slot = some o ∧ o.corr = corr ∧ c ∈ o.cands ∧ c.event = e ∧
      e ∈ offeredFor s corr ∧ C.select s.ctx e p = some c.trans := by
  unfold dispositionOf at h
  split at h
  · next o hs =>
    split at h
    · next hc =>
      split at h
      · simp at h
      · split at h
        · simp at h
        · next c hf =>
          split at h
          · simp at h
          · split at h
            · next hsel =>
              have hmem := List.mem_of_find?_eq_some hf
              have hev : c.event = e := by simpa using List.find?_some hf
              refine ⟨o, c, hs, hc, hmem, hev, ?_, hsel⟩
              simp only [offeredFor, hs, hc, ↓reduceIte, List.mem_map]
              exact ⟨c, hmem, hev⟩
            · simp at h
    · simp at h
  · simp at h

/-- **Accepted is safe.** In a reachable state, an `accepted` or `fallback-on-deadline` decision
names a candidate whose transition was enabled when the offer was made, in a context the agent
was really in, and is the one selection takes now. -/
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#An accepted answer is marked accepted]] {level: unit}
theorem accepted_safe {s : AState κ} (hr : Reachable C s) {corr : Nat} {e : String} {p : π}
    {annOk fb : Bool}
    (h : dispositionOf C s corr e p annOk fb = .accepted ∨
      dispositionOf C s corr e p annOk fb = .fallbackOnDeadline) :
    ∃ o c, s.slot = some o ∧ o.corr = corr ∧ c ∈ o.cands ∧ c.event = e ∧
      e ∈ offeredFor s corr ∧ C.probe o.ctx e = some c.trans ∧ o.ctx ∈ s.hist ∧
      C.select s.ctx e p = some c.trans := by
  obtain ⟨o, c, hs, hc, hm, he, hoff, hsel⟩ := accepted_was_offered C h
  obtain ⟨h₁, h₂⟩ := (inv_of_reachable C hr).offer o hs
  have hp := mem_enabledEvents C (h₁ ▸ hm)
  exact ⟨o, c, hs, hc, hm, he, hoff, he ▸ hp, h₂, hsel⟩

/-- A `refused-not-offered` event is not in the recorded offer. -/
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A refused answer is marked refused]] {level: unit}
theorem not_offered_not_listed {s : AState κ} {corr : Nat} {e : String} {p : π} {annOk fb : Bool}
    (h : dispositionOf C s corr e p annOk fb = .refusedNotOffered) : e ∉ offeredFor s corr := by
  unfold dispositionOf at h
  split at h
  · next o hs =>
    split at h
    · next hc =>
      split at h
      · simp at h
      · split at h
        · next hf =>
          simp only [offeredFor, hs, hc, ↓reduceIte, List.mem_map, not_exists, not_and]
          intro c hm he
          have := List.find?_eq_none.mp hf c hm
          simp [he] at this
        · split at h
          · simp at h
          · split at h
            · split at h <;> simp at h
            · simp at h
    · simp at h
  · simp at h

/-- A `stale` answer records an empty offer. -/
theorem stale_offers_nothing {s : AState κ} {corr : Nat} {e : String} {p : π} {annOk fb : Bool}
    (h : dispositionOf C s corr e p annOk fb = .stale) : offeredFor s corr = [] := by
  unfold dispositionOf at h
  unfold offeredFor
  cases hs : s.slot with
  | none => rfl
  | some o =>
    rw [hs] at h
    by_cases hc : o.corr = corr
    · exfalso
      simp only [hc, ↓reduceIte] at h
      split at h
      · simp at h
      · split at h
        · simp at h
        · split at h
          · simp at h
          · split at h
            · split at h <;> simp at h
            · simp at h
    · simp [hc]

/-- `fallback-on-deadline` only on an offer made to the fallback decider. -/
-- @tg: verifies:: [[openspec:decision-records#Decision Disposition Is Recorded#A deadline fallback is marked]] {level: unit}
theorem fallback_needs_fallback_offer {s : AState κ} {corr : Nat} {e : String} {p : π}
    {annOk fb : Bool} (h : dispositionOf C s corr e p annOk fb = .fallbackOnDeadline) :
    fb = true := by
  unfold dispositionOf at h
  split at h
  · split at h
    · split at h
      · simp at h
      · split at h
        · simp at h
        · split at h
          · simp at h
          · split at h
            · cases fb <;> simp_all
            · simp at h
    · simp at h
  · simp at h

/-- **`fallback-on-refusal` is not produced.** No decision and no refusal has it: the value is
reserved for a chart path that consults a fallback after a refusal, which does not exist. -/
theorem fallback_on_refusal_not_produced (s : AState κ) (corr : Nat) (e : String) (p : π)
    (annOk fb : Bool) :
    dispositionOf C s corr e p annOk fb ≠ .fallbackOnRefusal ∧
      refusalDisposition ≠ .fallbackOnRefusal := by
  refine ⟨?_, by simp [refusalDisposition]⟩
  unfold dispositionOf
  split
  · split
    · split
      · simp
      · split
        · simp
        · split
          · simp
          · split
            · split <;> simp
            · simp
    · simp
  · simp

end

end Vertep.Agent
