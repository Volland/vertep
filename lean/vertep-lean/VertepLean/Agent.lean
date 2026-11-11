/-!
# Decide states, decisions and grants (m3-agents, deciders and llm-actors)

A model of the agent machine of `vertep-agent` (`AgentMachine`: `enabled_events`, `offer`,
`on_decision`; `grant::effective`, `grant::filter`), abstracted over the SCXML core: a context
`κ` (configuration, history and datamodel) and payloads `π`, with the core given by `Core`:

* `names` — the event names the decide state's region listens for (declared candidates applied,
  reserved names dropped);
* `probe` — the transition SCXML selection takes for an event at offer time, whose payload is not
  known yet (Rust `enabled_events`, with guards evaluated by a probe datamodel);
* `select` — the transition selection takes for an event with its real payload (Rust
  `selected_transition`);
* `conforms` — the candidate's payload schema;
* `step` — the core macrostep on an event (Rust `session::step`).

The theorems hold for every core, so for the Lean `step` of the formal model in particular.

**Decisions.** After every input the agent recomputes the candidate set of the decide state
(`enabledEvents`) and keeps the open offer when the set is unchanged, or issues a new offer with
a fresh correlation id (`reoffer`). A decision is accepted only when its correlation id names the
open offer, its event is a candidate, its payload conforms and selection now takes the
transition that was offered (`decide`); anything else is rejected and takes no transition. A
correlation id answered once is dropped on redelivery.

**Grants.** The effective grant of an invoked actor is the intersection of the grants declared
along its invoke chain (`chainGrant`); every send it makes passes `filter`, which replaces a
send to an ungranted target by `error.communication` to itself and `vt.grant.denied` to its
parent. Sends to the origin of the current event and to itself are not named targets and pass,
as in Rust.
-/

namespace Vertep.Agent

/-- A transition: its source state id and its index among the source's transitions. -/
abbrev TransRef := String × Nat

/-- The SCXML core as the agent machine sees it. -/
structure Core (κ π : Type) where
  names : κ → List String
  active : κ → Bool
  probe : κ → String → Option TransRef
  select : κ → String → π → Option TransRef
  conforms : String → π → Bool
  step : κ → String → π → κ

/-- An offered candidate: the event and the transition enabled for it at offer time. -/
structure Cand where
  event : String
  trans : TransRef
  deriving DecidableEq, Repr

section
variable {κ π : Type} (C : Core κ π)

/-- The candidate set (Rust `enabled_events`): every name whose transition selection would take
at offer time, with that transition, in order. -/
-- @tg: implements:: [[openspec:deciders#Enabled Event Offer]] {note: "Lean model"}
def enabledEvents (k : κ) : List Cand :=
  (C.names k).filterMap fun n => (C.probe k n).map fun t => ⟨n, t⟩

/-- An offer: its correlation id, its candidates and (ghost) the context it was computed in. -/
structure Offer (κ : Type) where
  corr : Nat
  cands : List Cand
  ctx : κ

/-- The agent's state: the core context, the open offer, the answered correlation ids, the next
correlation id, and (ghost) every context the agent has been in. -/
structure AState (κ : Type) where
  ctx : κ
  slot : Option (Offer κ)
  answered : List Nat
  next : Nat
  hist : List κ

/-- An input: an ordinary event, a decision (from any decider, right or wrong), a refusal. -/
inductive Input (π : Type) where
  | event (name : String) (payload : π)
  | decision (corr : Nat) (event : String) (payload : π)
  | refusal (corr : Nat)

/-- What a decision did. -/
inductive Outcome where
  /-- Accepted: the chosen candidate's transition is taken. -/
  | taken (c : Cand)
  /-- Rejected with `error.decide.invalid` and this reason; no transition is taken. -/
  | rejected (reason : String)
  /-- Not a decision, or a duplicate dropped. -/
  | none
  deriving DecidableEq, Repr

/-- Keep the open offer while the candidate set is unchanged, issue a new one otherwise, and
cancel it when the decide state is not active (Rust `AgentMachine` after each macrostep). -/
-- @tg: implements:: [[openspec:deciders#Re-decision After Context Change]] {note: "Lean model"}
def reoffer (s : AState κ) : AState κ :=
  if C.active s.ctx then
    match s.slot with
    | some o =>
      if o.cands = enabledEvents C s.ctx then s
      else { s with slot := some ⟨s.next, enabledEvents C s.ctx, s.ctx⟩, next := s.next + 1 }
    | none => { s with slot := some ⟨s.next, enabledEvents C s.ctx, s.ctx⟩, next := s.next + 1 }
  else { s with slot := none }

/-- Rust `on_decision`: validate a decision against the open offer. -/
-- @tg: implements:: [[openspec:deciders#Decision Validation Against Candidates]] {note: "Lean model"}
def decide (s : AState κ) (corr : Nat) (e : String) (p : π) : AState κ × Outcome :=
  if corr ∈ s.answered then (s, .none)
  else
    let s₁ := { s with answered := corr :: s.answered }
    match s.slot with
    | some o =>
      if o.corr = corr then
        let s₂ := { s₁ with slot := none }
        match o.cands.find? (·.event = e) with
        | none => (s₂, .rejected "not-a-candidate")
        | some c =>
          if !C.conforms e p then (s₂, .rejected "payload-schema")
          else if C.select s.ctx e p = some c.trans then
            ({ s₂ with ctx := C.step s.ctx e p }, .taken c)
          else (s₂, .rejected "not-enabled")
      else (s₁, .rejected "stale")
    | none => (s₁, .rejected "stale")

/-- Record the context in the ghost history. -/
def remember (s : AState κ) : AState κ := { s with hist := s.hist ++ [s.ctx] }

/-- One input of the agent machine. -/
def stepA (s : AState κ) : Input π → AState κ × Outcome
  | .event n p => (remember (reoffer C { s with ctx := C.step s.ctx n p }), .none)
  | .decision corr e p =>
    let r := decide C s corr e p
    (remember (reoffer C r.1), r.2)
  | .refusal corr => (remember (reoffer C { s with answered := corr :: s.answered }), .none)

/-- The machine after its start macrostep in context `k`. -/
def init (k : κ) : AState κ := remember (reoffer C ⟨k, none, [], 0, []⟩)

/-- The states reachable from a start, whatever the inputs: any events and any decider output
(unknown events, bad payloads, stale or duplicate correlation ids). -/
inductive Reachable : AState κ → Prop
  | init (k : κ) : Reachable (init C k)
  | step {s : AState κ} (i : Input π) : Reachable s → Reachable (stepA C s i).1

/-- Replay a journal of inputs: no decider, no provider, just the inputs. -/
def replay (s : AState κ) (j : List (Input π)) : AState κ :=
  j.foldl (fun s i => (stepA C s i).1) s

/-- A live run: after each input the environment (deciders, LLM providers, tools) may answer the
agent's state with a further input, which is journaled too. Returns the final state and the
journal. -/
def live (R : AState κ → Option (Input π)) : AState κ → List (Input π) →
    AState κ × List (Input π)
  | s, [] => (s, [])
  | s, i :: is =>
    let s₁ := (stepA C s i).1
    match R s₁ with
    | some d =>
      let r := live R (stepA C s₁ d).1 is
      (r.1, i :: d :: r.2)
    | none =>
      let r := live R s₁ is
      (r.1, i :: r.2)

end

/-! ## Grants -/

/-- A grant: the targets and tools it allows (Rust `Grant::allows` checks both). -/
abbrev Grant := List String

/-- Rust `grant::effective`: `none` is unrestricted (a root actor). -/
def effective : Option Grant → Option Grant → Option Grant
  | some p, some d => some (p.filter (· ∈ d))
  | some p, none => some p
  | none, some d => some d
  | none, none => none

/-- The effective grant at the end of an invoke chain of declared grants, from the root. -/
-- @tg: implements:: [[openspec:llm-actors#Capability Grants]] {note: "Lean model"}
def chainGrant (ds : List (Option Grant)) : Option Grant := ds.foldl effective none

/-- The target of a send. -/
inductive Target where
  | parent
  | named (iri : String)
  | origin
  | itself
  | invalid
  deriving DecidableEq, Repr

/-- The name a grant checks (Rust `target_name`). -/
def Target.name : Target → Option String
  | .parent => some "#_parent"
  | .named t => some t
  | _ => none

/-- An outgoing send. -/
structure Out where
  target : Target
  event : String
  deriving DecidableEq, Repr

/-- Rust `grant::filter`. -/
-- @tg: implements:: [[openspec:llm-actors#Capability Grants]] {note: "Lean model"}
def filter (g : Grant) (outs : List Out) : List Out :=
  outs.flatMap fun o => match o.target.name with
    | some t => if t ∈ g then [o] else [⟨.itself, "error.communication"⟩, ⟨.parent, "vt.grant.denied"⟩]
    | none => [o]

end Vertep.Agent
