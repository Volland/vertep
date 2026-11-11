/-!
# The journal protocol (event-journal, storage)

A model of how the actor runtime (`vertep-runtime`, `Node::virtual_turn`, `Node::load`,
`Node::after_append`, `Node::recover`) makes a virtual actor durable, abstracted over the
machine: any deterministic `step : σ → ι → σ × List ε` (a macrostep: state, input, sends).

* A **journal** is the list of accepted inputs, each **entry** recording the message, the sends
  its macrostep produced and the resulting state digest; positions are `1, 2, …`.
* **Compare-and-set append** (`casAppend`) writes an entry only at the expected position.
* An **activation** (`Act`) holds the head position it has seen, the machine state and the
  message ids it has accepted; `activate` builds one by replaying the journal.
* A **turn** (`turn`) drops a message whose id the activation has seen (deduplication), runs the
  macrostep otherwise and appends with compare-and-set at the activation's head; a conflict
  deactivates the activation and leaves the message unacknowledged.
* The **outbox**: the store keeps `mark`, the position through which every recorded send was
  dispatched, and the delivery log `delivered`; the dispatcher may deliver any send recorded
  after the mark, any number of times (retries, crashes in between), and moves the mark only
  over entries whose sends were all delivered.
* The **system** (`Sys`) is the store, the mailbox (unacknowledged messages, redelivered until
  acknowledged), the ghost list of every message ever submitted, and the activations of any
  number of nodes. `Step` is every move the runtime or its environment can make: submit or
  redeliver a message, activate or crash any node at any time, take a turn on any node with any
  mailbox message, deliver, advance the mark.

The theorems (in `VertepLean.JournalTheorems`) hold in every state reachable from the empty
system, whatever the interleaving of nodes, crashes and redeliveries.

The Rust runtime follows the model here: it once advanced the outbox mark right after handing
the sends to the transport and kept a send the transport refused only as an in-memory retry, so
a crash between a refused send and its retry lost it; now its mark moves only over entries whose
sends were all delivered, as here and as the Transactional Outbox requirement demands
(`refused_send_survives_a_crash` in `crates/vertep-runtime/tests/event_journal.rs`).
-/

namespace Vertep.Journal

/-- A message: its id and its input. -/
structure Msg (ι : Type) where
  id : String
  input : ι
  deriving DecidableEq, Repr

/-- A deterministic machine: the initial state and the macrostep (new state and the sends). -/
structure Machine (σ ι ε : Type) where
  init : σ
  step : σ → ι → σ × List ε

/-- A journal entry: the accepted message, the sends of its macrostep and the state digest. -/
structure Entry (σ ι ε : Type) where
  msg : Msg ι
  effects : List ε
  digest : σ
  deriving DecidableEq, Repr

/-- A journal: the entries at positions `1, 2, …`. -/
abbrev Journal (σ ι ε : Type) := List (Entry σ ι ε)

/-- A send's stable id: the position of its entry and its index among the entry's sends (Rust
derives the send IRI from both, so retries carry the same message id). -/
abbrev SendId := Nat × Nat

section
variable {σ ι ε : Type} (M : Machine σ ι ε)

/-! ## Replay -/

/-- Replay entries from a state. -/
-- @tg: implements:: [[openspec:event-journal#Deterministic Replay]] {note: "Lean model"}
def replayFrom : σ → Journal σ ι ε → σ
  | s, [] => s
  | s, e :: es => replayFrom (M.step s e.msg.input).1 es

/-- Replay a journal from the initial state. -/
def replay (j : Journal σ ι ε) : σ := replayFrom M M.init j

/-- Every entry records exactly what its macrostep produced from the replayed state. -/
def RecordsFrom : σ → Journal σ ι ε → Prop
  | _, [] => True
  | s, e :: es =>
    e.effects = (M.step s e.msg.input).2 ∧ e.digest = (M.step s e.msg.input).1 ∧
      RecordsFrom (M.step s e.msg.input).1 es

/-- `RecordsFrom` from the initial state. -/
def Records (j : Journal σ ι ε) : Prop := RecordsFrom M M.init j

/-- Replay with divergence detection (Rust `Node::load`): the replayed state, or the position of
the first entry whose recomputed sends or digest differ from the recorded ones. -/
def checkFrom [DecidableEq σ] [DecidableEq ε] : Nat → σ → Journal σ ι ε → Except Nat σ
  | _, s, [] => .ok s
  | p, s, e :: es =>
    if e.effects = (M.step s e.msg.input).2 ∧ e.digest = (M.step s e.msg.input).1 then
      checkFrom (p + 1) (M.step s e.msg.input).1 es
    else .error p

/-- `checkFrom` from position 1 and the initial state. -/
def check [DecidableEq σ] [DecidableEq ε] (j : Journal σ ι ε) : Except Nat σ :=
  checkFrom M 1 M.init j

/-! ## Sends -/

/-- The send ids of an entry at position `p`. -/
def sendsAt (p : Nat) (e : Entry σ ι ε) : List SendId :=
  (List.range e.effects.length).map (fun i => (p, i))

/-- The send ids of entries starting at position `p`. -/
def sendsFrom : Nat → Journal σ ι ε → List SendId
  | _, [] => []
  | p, e :: es => sendsAt p e ++ sendsFrom (p + 1) es

/-- Every send id the journal records. -/
def recorded (j : Journal σ ι ε) : List SendId := sendsFrom 1 j

/-- The message ids of a journal. -/
def ids (j : Journal σ ι ε) : List String := j.map (·.msg.id)

/-! ## Store, activations, turns -/

/-- The durable store of one actor: journal, outbox mark, and the delivery log. -/
structure Store (σ ι ε : Type) where
  journal : Journal σ ι ε
  /-- Every send of the entries at positions `1 … mark` was delivered. -/
  mark : Nat
  /-- Every delivery, in order, duplicates included. -/
  delivered : List SendId

/-- The sends recorded after the outbox mark: what the dispatcher still owes. -/
def pending (st : Store σ ι ε) : List SendId :=
  sendsFrom (st.mark + 1) (st.journal.drop st.mark)

/-- Compare-and-set append (Rust `JournalStore::append`): only at the expected position. -/
-- @tg: implements:: [[openspec:event-journal#Compare-And-Set Append]] {note: "Lean model"}
def casAppend (j : Journal σ ι ε) (expected : Nat) (e : Entry σ ι ε) : Option (Journal σ ι ε) :=
  if j.length = expected then some (j ++ [e]) else none

/-- An activation: the journal position it has seen, its state and the message ids it has
accepted (Rust `VirtualActivation`: `head`, `machine`, `seen`). -/
structure Act (σ : Type) where
  head : Nat
  state : σ
  seen : List String
  deriving DecidableEq, Repr

/-- Activate by replaying the journal (Rust `Node::activate`). -/
def activate (j : Journal σ ι ε) : Act σ :=
  { head := j.length, state := replay M j, seen := ids j }

/-- One turn of an activation on a message (Rust `Node::virtual_turn`): the new store, the
activation afterwards (`none` when it stepped down), and whether the message was acknowledged. -/
-- @tg: implements:: [[openspec:event-journal#Message Deduplication]] {note: "Lean model"}
def turn (st : Store σ ι ε) (a : Act σ) (m : Msg ι) : Store σ ι ε × Option (Act σ) × Bool :=
  if m.id ∈ a.seen then (st, some a, true)
  else
    match casAppend st.journal a.head
        ⟨m, (M.step a.state m.input).2, (M.step a.state m.input).1⟩ with
    | some j =>
      ({ st with journal := j },
        some ⟨a.head + 1, (M.step a.state m.input).1, a.seen ++ [m.id]⟩, true)
    | none => (st, none, false)

/-! ## The system -/

/-- The whole system around one actor. -/
structure Sys (σ ι ε : Type) where
  store : Store σ ι ε
  /-- Unacknowledged messages, redelivered until acknowledged. -/
  mailbox : List (Msg ι)
  /-- Ghost: every message ever submitted. -/
  sent : List (Msg ι)
  /-- The activation on each node, if any. -/
  nodes : Nat → Option (Act σ)

/-- Replace the activation of node `n`. -/
def setNode (f : Nat → Option (Act σ)) (n : Nat) (v : Option (Act σ)) : Nat → Option (Act σ) :=
  fun k => if k = n then v else f k

/-- The empty system. -/
def Sys.empty : Sys σ ι ε :=
  { store := ⟨[], 0, []⟩, mailbox := [], sent := [], nodes := fun _ => none }

/-- Every move of the runtime and its environment. -/
-- @tg: implements:: [[openspec:event-journal#Transactional Outbox]] {note: "Lean model"}
inductive Step [DecidableEq ι] : Sys σ ι ε → Sys σ ι ε → Prop
  /-- A message is submitted. -/
  | submit (s : Sys σ ι ε) (m : Msg ι) :
      Step s { s with mailbox := s.mailbox ++ [m], sent := s.sent ++ [m] }
  /-- A message submitted before is delivered again (at-least-once transports, retries). -/
  | redeliver (s : Sys σ ι ε) (m : Msg ι) (h : m ∈ s.sent) :
      Step s { s with mailbox := s.mailbox ++ [m] }
  /-- Some node activates the actor by replaying the journal. -/
  | activate (s : Sys σ ι ε) (n : Nat) :
      Step s { s with nodes := setNode s.nodes n (some (activate M s.store.journal)) }
  /-- Some node crashes or deactivates: its activation is lost, at any point. -/
  | crash (s : Sys σ ι ε) (n : Nat) :
      Step s { s with nodes := setNode s.nodes n none }
  /-- An activation takes a turn on a mailbox message. -/
  | turn (s : Sys σ ι ε) (n : Nat) (a : Act σ) (m : Msg ι) (ha : s.nodes n = some a)
      (hm : m ∈ s.mailbox) :
      Step s { s with
        store := (turn M s.store a m).1,
        nodes := setNode s.nodes n (turn M s.store a m).2.1,
        mailbox := if (turn M s.store a m).2.2 then s.mailbox.erase m else s.mailbox }
  /-- The dispatcher delivers sends recorded after the mark (any, any number of times). -/
  | deliver (s : Sys σ ι ε) (l : List SendId) (h : ∀ x ∈ l, x ∈ pending s.store) :
      Step s { s with store := { s.store with delivered := s.store.delivered ++ l } }
  /-- The dispatcher advances the mark over entries whose sends were all delivered. -/
  | markThrough (s : Sys σ ι ε) (k : Nat) (h₁ : s.store.mark ≤ k)
      (h₂ : k ≤ s.store.journal.length)
      (h : ∀ x ∈ sendsFrom 1 (s.store.journal.take k), x ∈ s.store.delivered) :
      Step s { s with store := { s.store with mark := k } }

/-- The states reachable from the empty system. -/
inductive Reachable [DecidableEq ι] : Sys σ ι ε → Prop
  | empty : Reachable Sys.empty
  | step {s t : Sys σ ι ε} : Reachable s → Step M s t → Reachable t

end

end Vertep.Journal
