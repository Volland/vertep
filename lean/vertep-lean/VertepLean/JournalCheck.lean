import Lean.Data.Json
import VertepLean.JournalTheorems

/-!
# The journal protocol as a test oracle

`vertep-journal` judges an observation of a Rust runtime run (the actor's journal with the
configuration after each entry, the messages submitted, the sends the receiver accepted, the
messages left in the mailbox) against the journal protocol model:
`accepts M o = true ↔ Admissible M o` (`accepts_iff`), and every reachable state of the model
whose recorded sends were all delivered is admissible (`reachable_admissible`), so a run the
oracle rejects is one the model cannot produce.

```
vertep-journal --obs OBS.json
```

`OBS.json`: `{"journal": [{"id", "input", "state", "sends"}], "submitted": [ids],
"delivered": [[position, index]], "mailbox": [ids]}` for the toggle machine (`toggle`): `go`
toggles `idle` and `busy` and sends one event, any other input changes nothing and sends nothing.
Exit codes: `0` admissible, `1` not (the first failed property on standard error), `3` an input
error.
-/

namespace Vertep.Journal

open Lean (Json)

variable {σ ι ε : Type} (M : Machine σ ι ε)

/-- What a test observes of a run. -/
structure Obs (σ ι ε : Type) where
  journal : Journal σ ι ε
  submitted : List String
  delivered : List SendId
  mailbox : List String

/-- What the model guarantees of a run whose dispatcher has caught up: the journal replays,
no id is accepted twice, exactly the recorded sends were delivered, and every submitted message
was journaled or is still waiting. -/
def Admissible (o : Obs σ ι ε) : Prop :=
  Records M o.journal ∧ (ids o.journal).Nodup ∧
    (∀ x ∈ o.delivered, x ∈ recorded o.journal) ∧
    (∀ x ∈ recorded o.journal, x ∈ o.delivered) ∧
    (∀ i ∈ o.submitted, i ∈ ids o.journal ∨ i ∈ o.mailbox)

/-- The decision procedure for `Admissible`. -/
def accepts [DecidableEq σ] [DecidableEq ε] (o : Obs σ ι ε) : Bool :=
  (check M o.journal).isOk && decide (ids o.journal).Nodup &&
    o.delivered.all (· ∈ recorded o.journal) && (recorded o.journal).all (· ∈ o.delivered) &&
    o.submitted.all (fun i => i ∈ ids o.journal || i ∈ o.mailbox)

theorem check_isOk_iff [DecidableEq σ] [DecidableEq ε] (j : Journal σ ι ε) :
    (check M j).isOk = true ↔ Records M j := by
  constructor
  · intro h
    cases hc : check M j with
    | ok t => exact ((checkFrom_ok_iff M 1 M.init t j).mp hc).1
    | error _ => rw [hc] at h; cases h
  · intro h
    have : check M j = .ok (replay M j) := (checkFrom_ok_iff M 1 _ _ _).mpr ⟨h, rfl⟩
    rw [this]; rfl

/-- **The oracle decides the model's property.** -/
-- @tg: implements:: [[openspec:event-journal#Deterministic Replay]] {note: "test oracle"}
theorem accepts_iff [DecidableEq σ] [DecidableEq ε] (o : Obs σ ι ε) :
    accepts M o = true ↔ Admissible M o := by
  simp only [accepts, Admissible, Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true,
    Bool.or_eq_true, check_isOk_iff]
  constructor
  · rintro ⟨⟨⟨⟨h₁, h₂⟩, h₃⟩, h₄⟩, h₅⟩
    exact ⟨h₁, h₂, fun x hx => by simpa using h₃ x hx, fun x hx => by simpa using h₄ x hx,
      fun i hi => by simpa using h₅ i hi⟩
  · rintro ⟨h₁, h₂, h₃, h₄, h₅⟩
    exact ⟨⟨⟨⟨h₁, h₂⟩, fun x hx => by simpa using h₃ x hx⟩, fun x hx => by simpa using h₄ x hx⟩,
      fun i hi => by simpa using h₅ i hi⟩

/-- What a test observes of a model state. -/
def observe (s : Sys σ ι ε) : Obs σ ι ε :=
  { journal := s.store.journal, submitted := s.sent.map (·.id),
    delivered := s.store.delivered, mailbox := s.mailbox.map (·.id) }

/-- **The oracle accepts the model.** Every reachable state whose recorded sends were all
delivered (`outbox_complete` reaches one from any state) is admissible. -/
-- @tg: implements:: [[openspec:event-journal#Deterministic Replay]] {note: "the test oracle accepts every reachable state of the model"}
theorem reachable_admissible [DecidableEq ι] {s : Sys σ ι ε} (h : Reachable M s)
    (hd : ∀ x ∈ recorded s.store.journal, x ∈ s.store.delivered) :
    Admissible M (observe s) := by
  have hi := inv_of_reachable M h
  refine ⟨hi.records, hi.nodup, hi.delivered, hd, ?_⟩
  intro i hi'
  simp only [observe, List.mem_map] at hi' ⊢
  obtain ⟨m, hm, rfl⟩ := hi'
  rcases hi.sent m hm with h' | h'
  · exact Or.inr ⟨m, h', rfl⟩
  · exact Or.inl h'

/-! ## The toggle machine and the executable -/

/-- The machine of the Rust property tests: `go` toggles `idle` and `busy` and sends one event,
any other input changes nothing. States are the configuration joined with `,`. -/
def toggle : Machine String String Unit where
  init := "idle"
  step s i :=
    if i = "go" then (if s = "idle" then "busy" else "idle", [()]) else (s, [])

/-- Parse an observation of a toggle run. -/
def obsFromJson (j : Json) : Except String (Obs String String Unit) := do
  let entries ← (← (← j.getObjVal? "journal").getArr?).toList.mapM fun e => do
    let id ← (← e.getObjVal? "id").getStr?
    let input ← (← e.getObjVal? "input").getStr?
    let state ← (← e.getObjVal? "state").getStr?
    let sends ← (← e.getObjVal? "sends").getNat?
    pure ({ msg := ⟨id, input⟩, effects := List.replicate sends (), digest := state } :
      Entry String String Unit)
  let strs (k : String) : Except String (List String) := do
    (← (← j.getObjVal? k).getArr?).toList.mapM (·.getStr?)
  let delivered ← (← (← j.getObjVal? "delivered").getArr?).toList.mapM fun x => do
    match (← x.getArr?).toList with
    | [p, i] => pure ((← p.getNat?), (← i.getNat?))
    | _ => throw s!"a delivered send is [position, index], got {x.compress}"
  pure { journal := entries, submitted := ← strs "submitted", delivered,
         mailbox := ← strs "mailbox" }

/-- Why an observation is not admissible (the first failing property), or `none`. -/
def explain (o : Obs String String Unit) : Option String :=
  match check toggle o.journal with
  | .error p => some s!"replay divergence at position {p}"
  | .ok _ =>
    if !decide (ids o.journal).Nodup then some s!"a message id was journaled twice: {ids o.journal}"
    else match o.delivered.find? (· ∉ recorded o.journal) with
    | some x => some s!"an unrecorded send was delivered: {x}"
    | none => match (recorded o.journal).find? (· ∉ o.delivered) with
      | some x => some s!"a recorded send was never delivered: {x}"
      | none => match o.submitted.find? (fun i => i ∉ ids o.journal && i ∉ o.mailbox) with
        | some i => some s!"a submitted message was lost: {i}"
        | none => none

/-- The command line. -/
-- @tg: implements:: [[openspec:event-journal#Transactional Outbox]] {note: "test oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--obs", p] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-journal --obs OBS.json"; return 3
  let src ← IO.FS.readFile path
  let o ← match Json.parse src >>= obsFromJson with
    | .ok o => pure o
    | .error e => IO.eprintln s!"observation: {e}"; return 3
  if accepts toggle o then
    IO.println "admissible"
    return 0
  else
    IO.eprintln (explain o |>.getD "not admissible")
    return 1

end Vertep.Journal
