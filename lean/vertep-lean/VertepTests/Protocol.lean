import VertepLean.ProtocolCheck
import VertepLean.ProtocolUnsafe
import VertepTests.Axioms

/-!
# Kolyada protocols (m10 8.6)

The protocol model's theorems and their axioms, the unsafe check's counterexample, and the
participant histories recorded by `crates/vertep-kolyada/tests/histories.rs`
(`data/kolyada/*.jsonl`, also the samples for `lake exe vertep-kolyada --history`) replayed
through the checker in strict mode, with doctored copies it must reject at the doctored line.
-/

namespace Vertep.Kolyada.Test

open Vertep.Kolyada Vertep.Kolyada.Unsafe Vertep.Kolyada.Check

/-! ## The theorems -/

open Lean Elab Command in
/-- Fail unless each protocol theorem depends only on the standard axioms. -/
elab "#check_protocol_axioms" : command => do
  for n in [``accepted_enabled_by_projection, ``log_enabled, ``unbound_refused, ``log_bound,
      ``causal_orders_converge, ``converged_accept, ``run_causal, ``Check.replay_sound] do
    let bad ← liftCoreM (Vertep.AxiomCheck.disallowed n)
    unless bad.isEmpty do throwError "'{n}' depends on disallowed axioms {bad.toList}"

-- @tg: verifies:: [[openspec:protocol-conformance#Lean Protocol Model#Theorems proven]] {level: unit}
#check_protocol_axioms

/-- The three properties, together, for a reachable state: an accepted protocol message is
enabled by the participant's projection; a message from a DID not bound to its role is refused;
two runs that put the protocol steps in one order converge and then judge alike. -/
example (p : Protocol) (s : St) (hr : Reachable p s) (m : Msg) (hk : isControl m.kind = false)
    (ha : accept p s m = .accepted) :
    ∃ steps, project p s.me = some steps ∧ ∃ t ∈ p.transitions, stepOf s.me t ∈ steps ∧
      t.src ∈ s.knowledge ∧ t.sender = m.role ∧ t.message = m.kind :=
  let ⟨steps, h1, t, ht, h2, h3, h4, h5, _⟩ := accepted_enabled_by_projection hr ha hk
  ⟨steps, h1, t, ht, h2, h3, h4, h5⟩

example (p : Protocol) (s : St) (m : Msg) (h : s.bindings.lookup m.role ≠ some m.sender) :
    accept p s m ≠ .accepted := unbound_refused h

example (p : Protocol) (s t₁ t₂ : St) (l₁ l₂ : List Input) (hp : l₁.Perm l₂)
    (ho : l₁.filter Input.advances = l₂.filter Input.advances)
    (h₁ : run p s l₁ = some t₁) (h₂ : run p s l₂ = some t₂) (m : Msg) :
    accept p t₁ m = accept p t₂ m :=
  converged_accept (causal_orders_converge hp ho h₁ h₂) m

/-! ## Concurrent joins commute -/

/-- The demo with a third role, so two joins are concurrent. -/
def trio : Protocol := { demo with roles := ["buyer", "seller", "auditor"] }

def trioBindings : List (Role × Did) :=
  [("auditor", "did:example:auditor"), ("buyer", "did:example:buyer"),
   ("seller", "did:example:seller")]

def trioGenesis : Msg := { demoGenesis with recipients := ["auditor", "seller"] }

def joinOf (h : Hash) (did : Did) (r : Role) : Msg :=
  { hash := h, sigOk := true, protocol := demo.id, inst := some "g", sender := did, role := r,
    seq := 1, deps := ["g"], recipients := (["auditor", "buyer", "seller"].filter (· ≠ r)),
    kind := joinKind, payloadEmpty := true, shapeOk := true }

def trioStart : St := ((init trio trioBindings "did:example:buyer" trioGenesis).getD demoStart)

def jS : Input := .msg (joinOf "js" "did:example:seller" "seller")
def jA : Input := .msg (joinOf "ja" "did:example:auditor" "auditor")

-- Both orders of the two joins are runs, and they open the instance alike.
#guard ((run trio trioStart [jS, jA]).map (·.status trio)) == some .opened
#guard ((run trio trioStart [jA, jS]).map (·.status trio)) == some .opened
example : ∀ t₁ t₂, run trio trioStart [jS, jA] = some t₁ → run trio trioStart [jA, jS] = some t₂ →
    t₁.Same t₂ :=
  fun _ _ h₁ h₂ => causal_orders_converge (List.Perm.swap _ _ []) (by decide) h₁ h₂

/-! ## The unsafe check -/

/-- Without the role binding, a reachable state has applied a message from a DID bound to
nothing; the model refuses that message, and no reachable state of the model has such an entry. -/
-- @tg: verifies:: [[openspec:protocol-conformance#Lean Protocol Model#Unsafe variant rejected]] {level: unit}
example : (∃ s, ReachableU demo s ∧ UnboundApplied s) ∧
    accept demo demoOpen thief = .refused .roleNotPermitted ∧
    ∀ s, Reachable demo s → ¬ UnboundApplied s :=
  ⟨role_check_needed, demo_refused, fun _ hr => no_unbound_applied hr⟩

#guard UnboundApplied demoState
#guard acceptU demo demoOpen thief == .accepted
#guard accept demo demoOpen thief == .refused .roleNotPermitted

/-! ## Recorded histories -/

def quoteBuyer : String := include_str "data/kolyada/quote-buyer.jsonl"
def quoteSeller : String := include_str "data/kolyada/quote-seller.jsonl"
def quoteAuditor : String := include_str "data/kolyada/quote-auditor.jsonl"
def refusals : String := include_str "data/kolyada/refusals.jsonl"
def declined : String := include_str "data/kolyada/declined.jsonl"
def buffer : String := include_str "data/kolyada/buffer.jsonl"

/-- Replace `old` by `new` in line `n` (1-based) of a history. -/
def doctor (src : String) (n : Nat) (old new : String) : String :=
  let ls := src.splitOn "\n"
  "\n".intercalate (ls.set (n - 1) ((ls.getD (n - 1) "").replace old new))

/-- Remove line `n` (1-based) of a history. -/
def dropLine (src : String) (n : Nat) : String :=
  "\n".intercalate ((src.splitOn "\n").eraseIdx (n - 1))

/-- The history is rejected at line `n`, a `kind` line, with a reason containing `part`. -/
def rejectsAt (src : String) (n : Nat) (kind part : String) : Bool :=
  match check src with
  | .error f => f.num == n && f.kind == kind && (part.isEmpty || (f.reason.splitOn part).length > 1)
  | .ok _ => false

/-- The history is accepted, with `n` lines. -/
def acceptsWith (src : String) (n : Nat) : Bool :=
  match check src with
  | .ok k => k == n
  | .error _ => false

-- Every recorded history replays in strict mode: the events, status, knowledge and heads of each
-- line are the model's.
-- @tg: verifies:: [[openspec:protocol-conformance#Lean Protocol Model#Rust history replays]] {level: unit}
#guard acceptsWith quoteBuyer 9
#guard acceptsWith quoteSeller 8
#guard acceptsWith quoteAuditor 9
#guard acceptsWith refusals 18
#guard acceptsWith declined 3
#guard acceptsWith buffer 12

-- The seller's request, doctored to come from the auditor, is refused by the model, so the
-- recorded `held` is not its event.
#guard rejectsAt (doctor quoteSeller 3 "\"sender\":\"did:example:buyer\"" "\"sender\":\"did:example:auditor\"")
  3 "receive" "refused"

-- An own message signed in another DID's name is refused before it leaves.
#guard rejectsAt (doctor quoteBuyer 4 "\"sender\":\"did:example:buyer\"" "\"sender\":\"did:example:thief\"")
  4 "send" "send refused: role not permitted"

-- A tampered message claimed to verify is a valid quote, which the model applies.
#guard rejectsAt (doctor refusals 6 "\"sig\":false" "\"sig\":true") 6 "receive" "events"

-- A buffer that never evicts.
#guard rejectsAt (doctor buffer 9 "\"reason\":\"buffer full\"" "\"reason\":\"payload shape\"")
  9 "receive" "events"

-- A completion that did not complete the instance.
#guard rejectsAt (doctor quoteAuditor 9 "\"status\":\"completed\"" "\"status\":\"open\"")
  9 "receive" "status"

-- Knowledge that does not follow the quote.
#guard rejectsAt (doctor quoteSeller 5 "\"knowledge\":[\"Decided\"]" "\"knowledge\":[\"Ordered\"]")
  5 "send" "knowledge"

-- A projection that is not the auditor's.
#guard rejectsAt (doctor quoteAuditor 1 "\"action\":\"silent\",\"from\":\"Ordered\"" "\"action\":{\"receive\":\"receipt\",\"sender\":\"seller\"},\"from\":\"Ordered\"")
  1 "setup" "projection"

-- Without the hidden request, the auditor's quote stays held and the next hidden line applies it.
#guard rejectsAt (dropLine quoteAuditor 5) 5 "hidden" "events"

-- A malformed line and an unknown kind are reported at their lines.
#guard rejectsAt (doctor declined 2 "\"kind\":\"receive\"" "\"kind\":\"gossip\"") 2 "gossip" "unknown kind"
#guard rejectsAt (doctor declined 3 "{" "[") 3 "parse" ""

end Vertep.Kolyada.Test
