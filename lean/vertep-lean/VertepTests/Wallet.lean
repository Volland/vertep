import VertepLean.WalletCheck
import VertepLean.WalletUnsafe
import VertepTests.Axioms

/-!
# Wallet

The exchange model's theorems and their axioms, the unsafe verifier's counterexample, and
exchange histories (`data/wallet/*.jsonl`, also the samples for `lake exe vertep-wallet
--history`) replayed through the checker, with histories it must reject.
-/

namespace Vertep.Wallet.Test

open Vertep.Wallet Vertep.Wallet.Unsafe Vertep.Wallet.Check

/-! ## The theorems -/

open Lean Elab Command in
/-- Fail unless each of the six exchange theorems depends only on the standard axioms. -/
elab "#check_wallet_axioms" : command => do
  for n in [``issue_needs_possession, ``issue_once_per_offer, ``present_needs_consent,
      ``disclosure_within_consent, ``accept_sound, ``accept_once_per_nonce] do
    let bad ← liftCoreM (Vertep.AxiomCheck.disallowed n)
    unless bad.isEmpty do throwError "'{n}' depends on disallowed axioms {bad.toList}"

-- @tg: verifies:: [[openspec:credential-exchange#Exchange Model In Lean#Theorems proven]] {level: unit}
#check_wallet_axioms

/-- In a reachable state, an acceptance with an honest holder key is of a presentation that an
owner of the key made for that very verifier and nonce, and is the only one for the nonce. -/
example (σ : Setup) (s : St) (hr : Reachable σ s) (a b : Acceptance) (ha : a ∈ s.accepted)
    (hb : b ∈ s.accepted) (hv : a.verifier = b.verifier) (hn : a.nonce = b.nonce)
    (hk : a.key ∉ σ.compromised) :
    (∃ H, σ.HasKey H a.key ∧
      (⟨H, a.key, a.verifier, a.nonce, a.cred, a.claims⟩ : Presentation) ∈ s.presentations) ∧
      a = b :=
  ⟨(accept_sound hr ha).2.2.2.2.2.2.2.2.2 hk, accept_once_per_nonce hr ha hb hv hn⟩

/-- An issuance with an honest holder key follows that holder's possession proof; issuances on
one offer are one. -/
example (σ : Setup) (s : St) (hr : Reachable σ s) (x y : Issuance) (hx : x ∈ s.issued)
    (hy : y ∈ s.issued) (hk : x.key ∉ σ.compromised) (hI : x.issuer = y.issuer)
    (hN : x.nonce = y.nonce) :
    (∃ H, σ.HasKey H x.key ∧ (H, x.key, x.issuer, x.nonce) ∈ s.pops) ∧ x = y :=
  ⟨(issue_needs_possession hr hx).2.2.2 hk, issue_once_per_offer hr hx hy hI hN⟩

/-- Disclosed ⊆ consented ⊆ requested. -/
example (σ : Setup) (s : St) (hr : Reachable σ s) (p : Presentation)
    (hp : p ∈ s.presentations) :
    ∃ D', (p.holder, p.verifier, p.nonce, D') ∈ s.consents ∧ p.claims ⊆ D' ∧
      ∃ m R, (m, Term.req p.verifier p.nonce R) ∈ s.msgs ∧ D' ⊆ R := by
  obtain ⟨⟨D', hc, hD⟩, _⟩ := present_needs_consent hr hp
  obtain ⟨m, R, hm, _, hR⟩ := disclosure_within_consent hr hc
  exact ⟨D', hc, hD, m, R, hm, hR⟩

/-! ## The unsafe verifier -/

/-- Without the audience check, a reachable state accepts a presentation made for another
verifier; the model refuses the same run, and no reachable state of the model has one. -/
-- @tg: verifies:: [[openspec:credential-exchange#Exchange Model In Lean#Unsafe variant rejected]] {level: unit}
example : (∃ s, ReachableU demo s ∧ ForeignAcceptance s) ∧
    run demo (demoRun ++ [demoAccept]) St.empty = none ∧
    ∀ s, Reachable demo s → ¬ ForeignAcceptance s :=
  ⟨audience_check_needed, demoRun_rejected, fun _ hr => no_foreign_acceptance hr⟩

#guard ForeignAcceptance demoState

/-! ## Histories -/

def issuePresent : String := include_str "data/wallet/issue-present.jsonl"
def replayRefused : String := include_str "data/wallet/replay-refused.jsonl"
def issueWithoutPop : String := include_str "data/wallet/issue-without-pop.jsonl"

/-- Replace line `n` (1-based) of a history. -/
def setLine (src : String) (n : Nat) (l : String) : String :=
  "\n".intercalate ((src.splitOn "\n").set (n - 1) l)

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

-- An issuance and a presentation over a network that duplicates the possession proof and the
-- presentation: the verifier accepts once and refuses the duplicate.
-- @tg: verifies:: [[openspec:credential-exchange#Exchange History Checker#Conformant history accepted]] {level: unit}
#guard acceptsWith issuePresent 21

-- The presentation replayed to a second verifier, as delivered and as an injected copy, is
-- refused; an injected request is harmless.
#guard acceptsWith replayRefused 26

-- A second verifier accepting the replayed presentation is not a move of the model.
#guard rejectsAt (setLine replayRefused 21
  "{\"kind\":\"verify\",\"verifier\":\"did:example:verifier2\",\"msg\":\"m5\",\"result\":\"accept\"}")
  21 "verify" "audience"

-- The verifier accepting the duplicate is a replay.
#guard rejectsAt (setLine issuePresent 19
  "{\"kind\":\"verify\",\"verifier\":\"did:example:verifier\",\"msg\":\"m5\",\"result\":\"accept\"}")
  19 "verify" "replay"

-- An `issue` with no possession proof delivered to the issuer is rejected at its line, naming
-- the missing proof.
-- @tg: verifies:: [[openspec:credential-exchange#Exchange History Checker#Disabled step reported]] {level: unit}
#guard rejectsAt issueWithoutPop 4 "issue" "no possession proof pop(k-holder, did:example:issuer, o1)"

-- The same `issue` over a nonce never offered is rejected for the missing proof too.
#guard rejectsAt (setLine issueWithoutPop 2
  "{\"kind\":\"send\",\"from\":\"did:example:issuer\",\"msg\":\"m1\"}") 4 "issue" "no possession proof"

-- A forged presentation signed with an uncompromised key is not derivable.
#guard rejectsAt (setLine replayRefused 22
  "{\"kind\":\"inject\",\"msg\":\"m7\",\"term\":{\"t\":\"pres\",\"key\":\"k-holder\",\"verifier\":\"did:example:verifier2\",\"nonce\":\"p2\",\"cred\":{\"issuer\":\"did:example:issuer\",\"ikey\":\"k-issuer\",\"id\":\"sha256:c1\",\"key\":\"k-holder\",\"claims\":[\"age\",\"name\"]},\"claims\":[\"age\"]}}")
  22 "inject" "not compromised"

-- With the holder key compromised, the same forgery is derivable, and the second verifier
-- accepts it: the theorems promise nothing about compromised keys.
#guard acceptsWith (setLine (setLine (setLine replayRefused 1
  "{\"kind\":\"setup\",\"agents\":[{\"id\":\"did:example:issuer\",\"keys\":[\"k-issuer\"]},{\"id\":\"did:example:holder\",\"keys\":[\"k-holder\"]},{\"id\":\"did:example:verifier\",\"keys\":[\"k-verifier\"]},{\"id\":\"did:example:verifier2\",\"keys\":[\"k-verifier2\"]}],\"trust\":[{\"verifier\":\"did:example:verifier\",\"issuer\":\"did:example:issuer\"},{\"verifier\":\"did:example:verifier2\",\"issuer\":\"did:example:issuer\"}],\"compromised\":[\"k-holder\"]}")
  22 "{\"kind\":\"inject\",\"msg\":\"m7\",\"term\":{\"t\":\"pres\",\"key\":\"k-holder\",\"verifier\":\"did:example:verifier2\",\"nonce\":\"p2\",\"cred\":{\"issuer\":\"did:example:issuer\",\"ikey\":\"k-issuer\",\"id\":\"sha256:c1\",\"key\":\"k-holder\",\"claims\":[\"age\",\"name\"]},\"claims\":[\"age\"]}}")
  24 "{\"kind\":\"verify\",\"verifier\":\"did:example:verifier2\",\"msg\":\"m7\",\"result\":\"accept\"}") 26

-- Presenting without consent, and consenting beyond the request, are rejected.
#guard rejectsAt (setLine issuePresent 13
  "{\"kind\":\"send\",\"from\":\"did:example:holder\",\"msg\":\"m9\"}") 14 "present" "did not consent"
#guard rejectsAt (setLine issuePresent 13
  "{\"kind\":\"consent\",\"holder\":\"did:example:holder\",\"verifier\":\"did:example:verifier\",\"nonce\":\"p1\",\"claims\":[\"age\",\"name\"]}")
  13 "consent" "no request"

-- A revoked credential is refused.
#guard rejectsAt (setLine issuePresent 16
  "{\"kind\":\"revoke\",\"issuer\":\"did:example:issuer\",\"credential\":\"sha256:c1\"}") 18 "verify" "revoked"

-- A malformed line and a bad setup are reported at their lines.
#guard rejectsAt (setLine issuePresent 5 "{\"kind\":\"duplicate\"}") 5 "duplicate" "missing field msg"
#guard rejectsAt (setLine issuePresent 5 "not json") 5 "parse" ""
#guard rejectsAt (setLine issuePresent 1
  "{\"kind\":\"setup\",\"agents\":[{\"id\":\"a\",\"keys\":[\"k\"]},{\"id\":\"b\",\"keys\":[\"k\"]}],\"trust\":[],\"compromised\":[]}")
  1 "setup" "more than one agent"

end Vertep.Wallet.Test
