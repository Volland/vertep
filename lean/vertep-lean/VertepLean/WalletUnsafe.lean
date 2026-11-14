import VertepLean.WalletTheorems

/-!
# A verifier without the audience check, and why it is unsafe (m9 2.3)

A variant of the exchange model (`VertepLean.Wallet`) whose verifier skips the audience check
`V' = V` of `verify`: it accepts `pres(K, V', N, …)` for any `V'`, as long as every other check
passes (its own request for `N`, holder binding, trust, issuer key, status, claims).

Nonce freshness alone does not save it. Requests are unsigned, so the adversary can copy a
verifier's nonce into a request that names another verifier:

1. `did:b` requests a presentation with nonce `n2`; the request is on the network, so the
   adversary knows `n2`.
2. The adversary injects `req(did:a, n2, [age])` and delivers it to the holder.
3. The holder consents to disclose `age` to `did:a` and presents, for audience `did:a`.
4. The adversary delivers that presentation to `did:b`, which requested `n2`.

`audience_check_needed` proves the resulting state reachable in the weakened model, with an
acceptance by `did:b` of the presentation the holder made for `did:a`. In the model itself the
same run stops at the last step (`demoRun_rejected`), and no reachable state has such an
acceptance (`no_foreign_acceptance`, from `accept_sound`). This is the reason the audience is
inside the holder's signature and checked by every verifier, as in `CustodyReconfigUnsafe`.
-/

namespace Vertep.Wallet.Unsafe

open Vertep.Wallet

/-- The checks of the weakened verifier: `AcceptChecks` without `V' = V`. -/
def AcceptChecksU (σ : Setup) (s : St) (V : Agent) (K : Key) (N : Nonce) (c : Cred)
    (D : Claims) : Prop :=
  (∃ x ∈ s.requests, x.1 = V ∧ x.2.1 = N ∧ D ⊆ x.2.2) ∧
  (∀ a ∈ s.accepted, ¬ (a.verifier = V ∧ a.nonce = N)) ∧
  K = c.key ∧ (V, c.issuer) ∈ σ.trust ∧ σ.HasKey c.issuer c.ikey ∧
  (∀ r ∈ s.revocations, r.2 ≠ c.id) ∧ D ⊆ c.claims

instance (σ : Setup) (s : St) (V : Agent) (K : Key) (N : Nonce) (c : Cred) (D : Claims) :
    Decidable (AcceptChecksU σ s V K N c D) := by
  unfold AcceptChecksU; infer_instance

/-- The weakened verifier may accept a message whose term is `t`. -/
def AcceptsU (σ : Setup) (s : St) (V : Agent) : Option Term → Prop
  | some (.pres K _ N c D) => AcceptChecksU σ s V K N c D
  | _ => False

instance (σ : Setup) (s : St) (V : Agent) (t : Option Term) : Decidable (AcceptsU σ s V t) := by
  unfold AcceptsU; split <;> infer_instance

/-- Enabledness with the weakened verifier; every other move is the model's. -/
def EnabledU (σ : Setup) (s : St) : Action → Prop
  | .verify V m true => (m, V) ∈ s.delivered ∧ AcceptsU σ s V (s.termOf m)
  | a => Enabled σ s a

instance (σ : Setup) (s : St) (a : Action) : Decidable (EnabledU σ s a) := by
  unfold EnabledU; split <;> infer_instance

/-- One move of the weakened model (same effects as the model's). -/
def stepU (σ : Setup) (s : St) (a : Action) : Option St :=
  if EnabledU σ s a then some (effect s a) else none

/-- The states reachable with the weakened verifier. -/
inductive ReachableU (σ : Setup) : St → Prop
  | empty : ReachableU σ St.empty
  | step {s t : St} (a : Action) : ReachableU σ s → stepU σ s a = some t → ReachableU σ t

/-- Run moves in order in the weakened model. -/
def runU (σ : Setup) (as : List Action) (s : St) : Option St :=
  as.foldlM (stepU σ) s

theorem runU_reachable (σ : Setup) :
    ∀ (as : List Action) {s t : St}, ReachableU σ s → runU σ as s = some t → ReachableU σ t
  | [], s, t, hs, h => by
    simp only [runU, List.foldlM, Option.pure_def, Option.some.injEq] at h; exact h ▸ hs
  | a :: as, s, t, hs, h => by
    simp only [runU, List.foldlM] at h
    cases hx : stepU σ s a with
    | none => rw [hx] at h; cases h
    | some s' =>
      rw [hx] at h
      exact runU_reachable σ as (ReachableU.step a hs hx) h

/-- `s` has an acceptance of a presentation an honest holder made for another verifier. -/
def ForeignAcceptance (s : St) : Prop :=
  ∃ a ∈ s.accepted, ∃ p ∈ s.presentations, p.verifier ≠ a.verifier ∧
    a.term = .pres p.key p.verifier p.nonce p.cred p.claims

instance (s : St) : Decidable (ForeignAcceptance s) := by
  unfold ForeignAcceptance; infer_instance

/-! ## The counterexample -/

/-- An issuer, a holder and two verifiers that both trust the issuer; no key compromised. -/
def demo : Setup :=
  { agents := [("did:i", ["k-i"]), ("did:h", ["k-h"]), ("did:a", ["k-a"]), ("did:b", ["k-b"])]
    trust := [("did:a", "did:i"), ("did:b", "did:i")]
    compromised := [] }

/-- The credential the holder gets. -/
def demoCred : Cred := ⟨"did:i", "k-i", "sha256:c1", "k-h", ["age", "name"]⟩

/-- Issuance, then `did:b`'s nonce copied into a forged request for `did:a`, the holder's
presentation for `did:a`, and its delivery to `did:b`. -/
def demoRun : List Action :=
  [.offer "did:i" "did:h" "n1" "m1", .deliver "m1" "did:h",
   .prove "did:h" "k-h" "did:i" "n1" "m2", .deliver "m2" "did:i",
   .issue "did:i" "k-i" "sha256:c1" "k-h" "n1" ["age", "name"] "m3", .deliver "m3" "did:h",
   .store "did:h" demoCred,
   .request "did:b" "did:h" "n2" ["age"] "m4",
   .inject "m5" (.req "did:a" "n2" ["age"]), .deliver "m5" "did:h",
   .consent "did:h" "did:a" "n2" ["age"],
   .present "did:h" "did:a" "n2" demoCred ["age"] "m6",
   .deliver "m6" "did:b"]

/-- The weakened verifier `did:b` accepts the presentation made for `did:a`. -/
def demoAccept : Action := .verify "did:b" "m6" true

theorem demoRunU_some : (runU demo (demoRun ++ [demoAccept]) St.empty).isSome = true := by
  decide

/-- The state the weakened model reaches. -/
def demoState : St := (runU demo (demoRun ++ [demoAccept]) St.empty).get demoRunU_some

theorem demoState_reachable : ReachableU demo demoState :=
  runU_reachable demo _ ReachableU.empty (Option.some_get _).symm

/-- **Without the audience check, a presentation made for one verifier is accepted by
another.** -/
theorem audience_check_needed : ∃ s, ReachableU demo s ∧ ForeignAcceptance s :=
  ⟨demoState, demoState_reachable, by decide⟩

/-- The model runs the same steps up to the delivery to `did:b`… -/
theorem demoRun_enabled : (run demo demoRun St.empty).isSome = true := by decide

/-- …and refuses the acceptance. -/
theorem demoRun_rejected : run demo (demoRun ++ [demoAccept]) St.empty = none := by decide

/-- In the model, no reachable state accepts a presentation made for another verifier. -/
theorem no_foreign_acceptance {σ : Setup} {s : St} (hr : Reachable σ s) :
    ¬ ForeignAcceptance s := by
  rintro ⟨a, ha, p, _, hne, ht⟩
  have haud := (accept_sound hr ha).2.1
  simp only [Acceptance.term, Term.pres.injEq] at ht
  exact hne (ht.2.1 ▸ haud)

end Vertep.Wallet.Unsafe
