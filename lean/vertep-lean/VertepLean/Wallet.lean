/-!
# Credential exchange: issuers, holders and verifiers over a hostile network (m9 2.1)

A model of the agent wallet's exchange protocols (OpenSpec change `m9-agent-wallet`, design D5),
whose actions are exactly the lines of an exchange history
(`crates/vertep-wallet/docs/exchange-history.md`), so the history checker
(`VertepLean.WalletCheck`) replays a history by running `step`.

* **Agents** (`Setup.agents`) are DIDs, each with the keys it owns. Verifiers trust issuers
  (`Setup.trust`). Some keys are **compromised** (`Setup.compromised`): the adversary can sign
  with them.
* **Messages** are symbolic terms (`Term`): an offer, a possession proof `pop` (signed by the
  holder key), a credential `cred` (signed by the issuer key, binding a credential id and claim
  names to a holder key), a presentation request `req`, a presentation `pres` (signed by the
  holder key over the audience, the verifier's nonce, the embedded credential and the disclosed
  claim names), and `other` (results, problems, refusals).
* **The network** (`St.msgs`, `St.delivered`) is the adversary's: every message ever sent is
  known, and the adversary delivers any message to anyone any number of times, drops,
  duplicates and reorders them, and injects new ones (`Action.inject`) built from what it knows
  (`Derivable`).
* **Signatures** are symbolic: a signed term is made by an honest step of its key's owner, or
  injected when its key is compromised or the exact term is already known — including a
  credential embedded in a known presentation. A signature cannot be stripped, re-targeted or
  re-bound, which is perfect cryptography.
* **Nonces** of honest offers and requests are fresh (`St.used`), so each names its creator.
* **Ghost state** records every offer, honest possession proof, issuance, store, request,
  consent, presentation, acceptance and revocation.

The honest steps map to the Rust exchange vocabulary (`vertep_wallet::vocab::Step`): `offer`
(`Offer`), `prove` (`ProvePossession`), `issue` (`Issue`), `store` (`Store`), `request`
(`RequestPresentation`), `consent` (`Consent`, the holder chart's consent decision, design D7),
`present` (`Present`), `verify` (`Verify`, accepting or refusing), `revoke` (`Revoke`); a
`Refuse` and every other message a chart emits is a `send` of `other`.

What is abstracted: cryptography is perfect (above); claim values and credential bytes are not
modelled, only claim names and credential ids; and status is read fresh at acceptance — a
verifier sees every revocation made before it accepts (cached status is a risk the design names,
outside the model). The theorems are in `VertepLean.WalletTheorems`.
-/

namespace Vertep.Wallet

/-- An agent (a DID). -/
abbrev Agent := String
/-- A key id. -/
abbrev Key := String
/-- A nonce. -/
abbrev Nonce := String
/-- A credential id (a digest). -/
abbrev CredId := String
/-- A message id. -/
abbrev MsgId := String
/-- A set of claim names, as a list compared as a set (`⊆`). -/
abbrev Claims := List String

/-- A credential term: `cred(issuer, ikey, id, key, claims)`, signed by `ikey`, binding the
credential `id` with `claims` to the holder key `key`. -/
structure Cred where
  issuer : Agent
  ikey : Key
  id : CredId
  key : Key
  claims : Claims
  deriving DecidableEq, Repr

/-- A message term. -/
inductive Term where
  /-- `offer(issuer, nonce)`, unsigned. -/
  | offer (issuer : Agent) (nonce : Nonce)
  /-- `pop(key, issuer, nonce)`, signed by `key`. -/
  | pop (key : Key) (issuer : Agent) (nonce : Nonce)
  /-- A credential, signed by its `ikey`. -/
  | cred (c : Cred)
  /-- `req(verifier, nonce, claims)`, unsigned. -/
  | req (verifier : Agent) (nonce : Nonce) (claims : Claims)
  /-- `pres(key, verifier, nonce, cred, claims)`, signed by `key`. -/
  | pres (key : Key) (verifier : Agent) (nonce : Nonce) (c : Cred) (claims : Claims)
  /-- Anything else (results, problems, refusals), unsigned. -/
  | other
  deriving DecidableEq, Repr

/-- The credential a term carries inside it: the one embedded in a presentation. -/
def Term.embedded : Term → Option Cred
  | .pres _ _ _ c _ => some c
  | _ => none

/-- `t` occurs in `u`: it is `u`, or the credential embedded in the presentation `u`. -/
def Within (t u : Term) : Prop := t = u ∨ u.embedded.map Term.cred = some t

instance (t u : Term) : Decidable (Within t u) := by unfold Within; infer_instance

/-- The setup (line 1 of a history). -/
structure Setup where
  /-- Every honest agent and the keys it owns. -/
  agents : List (Agent × List Key)
  /-- `(verifier, issuer)`: the verifier trusts the issuer. -/
  trust : List (Agent × Agent)
  /-- The keys the adversary can sign with. -/
  compromised : List Key
  deriving Repr

/-- `a` is an agent of the setup. -/
def Setup.IsAgent (σ : Setup) (a : Agent) : Prop := ∃ x ∈ σ.agents, x.1 = a

/-- Agent `a` owns key `k`. -/
def Setup.HasKey (σ : Setup) (a : Agent) (k : Key) : Prop := ∃ x ∈ σ.agents, x.1 = a ∧ k ∈ x.2

instance (σ : Setup) (a : Agent) : Decidable (σ.IsAgent a) := by
  unfold Setup.IsAgent; infer_instance

instance (σ : Setup) (a : Agent) (k : Key) : Decidable (σ.HasKey a k) := by
  unfold Setup.HasKey; infer_instance

/-- An issuance: `issuer` signed credential `cred` with `ikey`, binding `claims` to `key`, on
the offer `nonce`. -/
structure Issuance where
  issuer : Agent
  ikey : Key
  cred : CredId
  key : Key
  nonce : Nonce
  claims : Claims
  deriving DecidableEq, Repr

/-- The credential term an issuance sends. -/
def Issuance.toCred (x : Issuance) : Cred := ⟨x.issuer, x.ikey, x.cred, x.key, x.claims⟩

/-- An honest presentation by `holder`, signed with its `key`, for `verifier` and `nonce`,
disclosing `claims` of the credential `cred`. -/
structure Presentation where
  holder : Agent
  key : Key
  verifier : Agent
  nonce : Nonce
  cred : Cred
  claims : Claims
  deriving DecidableEq, Repr

/-- An acceptance by `verifier` of the presentation `pres(key, aud, nonce, cred, claims)`;
`revSeen` is the number of revocations made before it (the status it read). -/
structure Acceptance where
  verifier : Agent
  key : Key
  aud : Agent
  nonce : Nonce
  cred : Cred
  claims : Claims
  revSeen : Nat
  deriving DecidableEq, Repr

/-- The presentation term an acceptance accepted. -/
def Acceptance.term (a : Acceptance) : Term := .pres a.key a.aud a.nonce a.cred a.claims

/-- The model state: the network and the ghost records. -/
structure St where
  /-- Every message ever sent, with its id: the adversary's knowledge. -/
  msgs : List (MsgId × Term)
  /-- Every delivery, as `(message, recipient)`. -/
  delivered : List (MsgId × Agent)
  /-- The nonces honest agents created. -/
  used : List Nonce
  /-- Ghost: offers, as `(issuer, nonce)`. -/
  offers : List (Agent × Nonce)
  /-- Ghost: honest possession proofs, as `(holder, key, issuer, nonce)`. -/
  pops : List (Agent × Key × Agent × Nonce)
  /-- Ghost: issuances. -/
  issued : List Issuance
  /-- Ghost: stored credentials, as `(holder, credential term)`. -/
  stores : List (Agent × Cred)
  /-- Ghost: requests, as `(verifier, nonce, requested claims)`. -/
  requests : List (Agent × Nonce × Claims)
  /-- Ghost: consents, as `(holder, verifier, nonce, consented claims)`. -/
  consents : List (Agent × Agent × Nonce × Claims)
  /-- Ghost: honest presentations. -/
  presentations : List Presentation
  /-- Ghost: acceptances. -/
  accepted : List Acceptance
  /-- The issuers' revocations, in order, as `(issuer, credential)`. -/
  revocations : List (Agent × CredId)
  deriving Repr

/-- The initial state: nothing sent, nothing recorded. -/
def St.empty : St := ⟨[], [], [], [], [], [], [], [], [], [], [], []⟩

/-- No message has id `m` yet. -/
def St.Fresh (s : St) (m : MsgId) : Prop := ∀ x ∈ s.msgs, x.1 ≠ m

/-- A message with id `m` exists. -/
def St.Exists (s : St) (m : MsgId) : Prop := ∃ x ∈ s.msgs, x.1 = m

/-- A message with term `t` was delivered to `a`. -/
def St.DeliveredTo (s : St) (t : Term) (a : Agent) : Prop :=
  ∃ x ∈ s.msgs, x.2 = t ∧ (x.1, a) ∈ s.delivered

/-- The adversary knows `t`: it occurs in some message. -/
def St.Knows (s : St) (t : Term) : Prop := ∃ x ∈ s.msgs, Within t x.2

/-- The term of message `m`. -/
def St.termOf (s : St) (m : MsgId) : Option Term :=
  (s.msgs.find? (fun x => x.1 == m)).map (·.2)

instance (s : St) (m : MsgId) : Decidable (s.Fresh m) := by unfold St.Fresh; infer_instance
instance (s : St) (m : MsgId) : Decidable (s.Exists m) := by unfold St.Exists; infer_instance
instance (s : St) (t : Term) (a : Agent) : Decidable (s.DeliveredTo t a) := by
  unfold St.DeliveredTo; infer_instance
instance (s : St) (t : Term) : Decidable (s.Knows t) := by unfold St.Knows; infer_instance

/-- A term the adversary can build: unsigned terms always; a signed term when its key is
compromised or the exact term is known; for a presentation, also its embedded credential. -/
def Derivable (σ : Setup) (s : St) : Term → Prop
  | .pop K I N => K ∈ σ.compromised ∨ s.Knows (.pop K I N)
  | .cred c => c.ikey ∈ σ.compromised ∨ s.Knows (.cred c)
  | .pres K V N c D =>
    (K ∈ σ.compromised ∨ s.Knows (.pres K V N c D)) ∧
      (c.ikey ∈ σ.compromised ∨ s.Knows (.cred c))
  | _ => True

instance (σ : Setup) (s : St) (t : Term) : Decidable (Derivable σ s t) := by
  cases t <;> unfold Derivable <;> infer_instance

/-- Message `u` is a request `req(V, N, R)` with `D ⊆ R`. -/
def Covers (V : Agent) (N : Nonce) (D : Claims) : Term → Prop
  | .req V' N' R => V' = V ∧ N' = N ∧ D ⊆ R
  | _ => False

instance (V : Agent) (N : Nonce) (D : Claims) (t : Term) : Decidable (Covers V N D t) := by
  cases t <;> unfold Covers <;> infer_instance

/-- The acceptance checks of verifier `V` on the presentation `pres(K, V', N, c, D)`. -/
def AcceptChecks (σ : Setup) (s : St) (V : Agent) (K : Key) (V' : Agent) (N : Nonce) (c : Cred)
    (D : Claims) : Prop :=
  -- the requested nonce, once
  (∃ x ∈ s.requests, x.1 = V ∧ x.2.1 = N ∧ D ⊆ x.2.2) ∧
  (∀ a ∈ s.accepted, ¬ (a.verifier = V ∧ a.nonce = N)) ∧
  -- holder binding
  K = c.key ∧
  -- a trusted issuer's key
  (V, c.issuer) ∈ σ.trust ∧ σ.HasKey c.issuer c.ikey ∧
  -- not revoked
  (∀ r ∈ s.revocations, r.2 ≠ c.id) ∧
  -- disclosed claims are in the credential
  D ⊆ c.claims ∧
  -- the audience
  V' = V

instance (σ : Setup) (s : St) (V : Agent) (K : Key) (V' : Agent) (N : Nonce) (c : Cred)
    (D : Claims) : Decidable (AcceptChecks σ s V K V' N c D) := by
  unfold AcceptChecks; infer_instance

/-- Verifier `V` may accept a message whose term is `t`. -/
def Accepts (σ : Setup) (s : St) (V : Agent) : Option Term → Prop
  | some (.pres K V' N c D) => AcceptChecks σ s V K V' N c D
  | _ => False

instance (σ : Setup) (s : St) (V : Agent) (t : Option Term) : Decidable (Accepts σ s V t) := by
  unfold Accepts; split <;> infer_instance

/-- The moves of the system: the honest steps and the network events of the history format. -/
inductive Action where
  /-- `offer`: the issuer sends `offer(issuer, nonce)` as `msg`. -/
  | offer (issuer to : Agent) (nonce : Nonce) (msg : MsgId)
  /-- `prove-possession`: the holder signs `pop(key, issuer, nonce)`. -/
  | prove (holder : Agent) (key : Key) (issuer : Agent) (nonce : Nonce) (msg : MsgId)
  /-- `issue`: the issuer signs `cred(issuer, ikey, cred, key, claims)`. -/
  | issue (issuer : Agent) (ikey : Key) (cred : CredId) (key : Key) (nonce : Nonce)
      (claims : Claims) (msg : MsgId)
  /-- `store`: the holder stores a credential term it received. -/
  | store (holder : Agent) (c : Cred)
  /-- `request-presentation`: the verifier sends `req(verifier, nonce, claims)`. -/
  | request (verifier to : Agent) (nonce : Nonce) (claims : Claims) (msg : MsgId)
  /-- `consent`: the holder consents to disclose `claims` to `verifier` on `nonce`. -/
  | consent (holder verifier : Agent) (nonce : Nonce) (claims : Claims)
  /-- `present`: the holder signs `pres(c.key, verifier, nonce, c, claims)`. -/
  | present (holder verifier : Agent) (nonce : Nonce) (c : Cred) (claims : Claims) (msg : MsgId)
  /-- `verify`: the verifier accepts (`true`) or refuses (`false`) a delivered message. -/
  | verify (verifier : Agent) (msg : MsgId) (accept : Bool)
  /-- `revoke`: the issuer revokes a credential it issued. -/
  | revoke (issuer : Agent) (cred : CredId)
  /-- `send`: an agent sends `other`. -/
  | send (sender : Agent) (msg : MsgId)
  /-- `deliver`: the network delivers `msg` to `to`. -/
  | deliver (msg : MsgId) (to : Agent)
  /-- `drop`: the network drops `msg`. -/
  | drop (msg : MsgId)
  /-- `duplicate`: the network duplicates `msg` (a later `deliver` is the copy). -/
  | duplicate (msg : MsgId)
  /-- `inject`: the adversary sends a term it can derive. -/
  | inject (msg : MsgId) (term : Term)
  deriving Repr

/-- When a move is enabled (the "Enabled when" columns of the history format). -/
def Enabled (σ : Setup) (s : St) : Action → Prop
  | .offer I _ N m => σ.IsAgent I ∧ N ∉ s.used ∧ s.Fresh m
  | .prove H K I N m => σ.HasKey H K ∧ s.DeliveredTo (.offer I N) H ∧ s.Fresh m
  | .issue I IK C K N _ m =>
    σ.HasKey I IK ∧ (I, N) ∈ s.offers ∧ (∀ x ∈ s.issued, x.nonce ≠ N) ∧
      s.DeliveredTo (.pop K I N) I ∧ (∀ x ∈ s.issued, x.cred ≠ C) ∧ s.Fresh m
  | .store H c => σ.HasKey H c.key ∧ s.DeliveredTo (.cred c) H
  | .request _ _ N _ m => N ∉ s.used ∧ s.Fresh m
  | .consent H V N D => ∃ x ∈ s.msgs, (x.1, H) ∈ s.delivered ∧ Covers V N D x.2
  | .present H V N c D m =>
    (∃ x ∈ s.consents, x.1 = H ∧ x.2.1 = V ∧ x.2.2.1 = N ∧ D ⊆ x.2.2.2) ∧
      (H, c) ∈ s.stores ∧ σ.HasKey H c.key ∧ D ⊆ c.claims ∧ s.Fresh m
  | .verify V m true => (m, V) ∈ s.delivered ∧ Accepts σ s V (s.termOf m)
  | .verify V m false => s.Exists m ∧ (m, V) ∈ s.delivered
  | .revoke I C => ∃ x ∈ s.issued, x.issuer = I ∧ x.cred = C
  | .send _ m => s.Fresh m
  | .deliver m _ => s.Exists m
  | .drop m => s.Exists m
  | .duplicate m => s.Exists m
  | .inject m t => s.Fresh m ∧ Derivable σ s t

instance (σ : Setup) (s : St) (a : Action) : Decidable (Enabled σ s a) := by
  cases a with
  | verify V m acc => cases acc <;> unfold Enabled <;> infer_instance
  | _ => unfold Enabled; infer_instance

/-- Send `t` as `m`. -/
def St.send (s : St) (m : MsgId) (t : Term) : St := { s with msgs := s.msgs ++ [(m, t)] }

/-- What a move does. -/
def effect (s : St) : Action → St
  | .offer I _ N m =>
    { s.send m (.offer I N) with used := s.used ++ [N], offers := s.offers ++ [(I, N)] }
  | .prove H K I N m => { s.send m (.pop K I N) with pops := s.pops ++ [(H, K, I, N)] }
  | .issue I IK C K N CC m =>
    { s.send m (.cred ⟨I, IK, C, K, CC⟩) with issued := s.issued ++ [⟨I, IK, C, K, N, CC⟩] }
  | .store H c => { s with stores := s.stores ++ [(H, c)] }
  | .request V _ N R m =>
    { s.send m (.req V N R) with used := s.used ++ [N], requests := s.requests ++ [(V, N, R)] }
  | .consent H V N D => { s with consents := s.consents ++ [(H, V, N, D)] }
  | .present H V N c D m =>
    { s.send m (.pres c.key V N c D) with
      presentations := s.presentations ++ [⟨H, c.key, V, N, c, D⟩] }
  | .verify V m true =>
    match s.termOf m with
    | some (.pres K V' N c D) =>
      { s with accepted := s.accepted ++ [⟨V, K, V', N, c, D, s.revocations.length⟩] }
    | _ => s
  | .verify _ _ false => s
  | .revoke I C => { s with revocations := s.revocations ++ [(I, C)] }
  | .send _ m => s.send m .other
  | .deliver m a => { s with delivered := s.delivered ++ [(m, a)] }
  | .drop _ => s
  | .duplicate _ => s
  | .inject m t => s.send m t

/-- One move: the next state when it is enabled. Executable, so the checker runs it. -/
-- @tg: implements:: [[openspec:credential-exchange#Exchange Model In Lean]]
def step (σ : Setup) (s : St) (a : Action) : Option St :=
  if Enabled σ s a then some (effect s a) else none

/-- The states reachable from the initial one. -/
inductive Reachable (σ : Setup) : St → Prop
  | empty : Reachable σ St.empty
  | step {s t : St} (a : Action) : Reachable σ s → step σ s a = some t → Reachable σ t

/-- Run moves in order; `none` when one is not enabled. -/
def run (σ : Setup) (as : List Action) (s : St) : Option St :=
  as.foldlM (step σ) s

theorem step_some {σ : Setup} {s t : St} {a : Action} (h : step σ s a = some t) :
    Enabled σ s a ∧ t = effect s a := by
  unfold step at h
  split at h
  · next he => exact ⟨he, (Option.some.inj h).symm⟩
  · cases h

theorem run_reachable (σ : Setup) :
    ∀ (as : List Action) {s t : St}, Reachable σ s → run σ as s = some t → Reachable σ t
  | [], s, t, hs, h => by
    simp only [run, List.foldlM, Option.pure_def, Option.some.injEq] at h; exact h ▸ hs
  | a :: as, s, t, hs, h => by
    simp only [run, List.foldlM] at h
    cases hx : step σ s a with
    | none => rw [hx] at h; cases h
    | some s' =>
      rw [hx] at h
      exact run_reachable σ as (Reachable.step a hs hx) h

end Vertep.Wallet
