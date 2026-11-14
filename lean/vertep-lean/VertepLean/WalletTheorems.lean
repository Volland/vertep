import VertepLean.Wallet

/-!
# Exchange safety: the six theorems of the exchange model (m9 2.2)

In every state reachable in the exchange model (`VertepLean.Wallet`), whatever the network does:

* `issue_needs_possession` — every issuance is on a nonce its issuer offered, signed with the
  issuer's key, after a possession proof for that key, issuer and nonce was delivered to the
  issuer; when the holder key is not compromised, an owner of that key made that proof;
* `issue_once_per_offer` — an issuer issues at most once per offer nonce;
* `present_needs_consent` — every honest presentation discloses only claims its holder
  consented to disclose to that verifier on that nonce, from a credential the holder stores,
  bound to the signing key;
* `disclosure_within_consent` — every consent answers a request for that verifier and nonce
  delivered to the holder, and stays within its requested claims (so disclosed ⊆ consented ⊆
  requested);
* `accept_sound` — every acceptance is of a presentation delivered to the verifier, made for
  that verifier (audience) and a nonce it requested and had not accepted before, bound to the
  credential's holder key, disclosing requested claims the credential has, from a trusted
  issuer's key, and not revoked when accepted; when the issuer key is not compromised the issuer
  issued exactly that credential, and when the holder key is not compromised an owner of the key
  made exactly that presentation;
* `accept_once_per_nonce` — a verifier accepts at most once per nonce.

The proof is one inductive invariant (`Inv`). Its core is `Inv.genuine`: every signed term the
adversary knows whose key is not compromised was made by an honest step (`Genuine`). It is kept
by `inject` because the adversary only sends terms it can derive (`Derivable`): a signed term
with an honest key must already be known, so already genuine.
-/

namespace Vertep.Wallet

/-! ## The state only grows -/

/-- Every record of `s` is in `t`, and `t`'s revocations extend `s`'s. -/
structure Grows (s t : St) : Prop where
  msgs : ∀ x ∈ s.msgs, x ∈ t.msgs
  delivered : ∀ x ∈ s.delivered, x ∈ t.delivered
  offers : ∀ x ∈ s.offers, x ∈ t.offers
  pops : ∀ x ∈ s.pops, x ∈ t.pops
  issued : ∀ x ∈ s.issued, x ∈ t.issued
  stores : ∀ x ∈ s.stores, x ∈ t.stores
  requests : ∀ x ∈ s.requests, x ∈ t.requests
  consents : ∀ x ∈ s.consents, x ∈ t.consents
  presentations : ∀ x ∈ s.presentations, x ∈ t.presentations
  revocations : s.revocations <+: t.revocations

theorem grows_effect (s : St) (a : Action) : Grows s (effect s a) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact ⟨fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h,
        fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h, List.prefix_refl _⟩
    · simp only [effect]
      split <;>
        exact ⟨fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h,
          fun _ h => h, fun _ h => h, fun _ h => h, fun _ h => h, List.prefix_refl _⟩
  | _ =>
    constructor <;> simp (config := { contextual := true }) [effect, St.send, List.prefix_append]

/-- The messages a move sends. -/
def sent : Action → List (MsgId × Term)
  | .offer I _ N m => [(m, .offer I N)]
  | .prove _ K I N m => [(m, .pop K I N)]
  | .issue I IK C K _ CC m => [(m, .cred ⟨I, IK, C, K, CC⟩)]
  | .request V _ N R m => [(m, .req V N R)]
  | .present _ V N c D m => [(m, .pres c.key V N c D)]
  | .send _ m => [(m, .other)]
  | .inject m t => [(m, t)]
  | _ => []

theorem msgs_effect (s : St) (a : Action) : (effect s a).msgs = s.msgs ++ sent a := by
  cases a with
  | verify V m acc =>
    cases acc
    · simp [effect, sent]
    · simp only [effect, sent, List.append_nil]; split <;> rfl
  | _ => simp [effect, sent, St.send]

/-- Membership in a record list after a move: an old record, or the move's new one. -/
local macro "mem_tac" : tactic => `(tactic| (
  simp only [effect, St.send, List.mem_append, List.mem_singleton] at *
  first
  | exact Or.inl ‹_›
  | (rcases ‹_ ∨ _› with h | h
     · exact Or.inl h
     · subst h; right
       first
       | exact ⟨_, _, rfl, rfl⟩ | exact ⟨_, _, _, _, rfl, rfl⟩
       | exact ⟨_, _, _, _, _, rfl, rfl⟩ | exact ⟨_, _, _, _, _, _, rfl, rfl⟩
       | exact ⟨_, _, _, _, _, _, _, rfl, rfl⟩)))

theorem mem_delivered {s : St} {a : Action} {x : MsgId × Agent}
    (h : x ∈ (effect s a).delivered) :
    x ∈ s.delivered ∨ ∃ m b, a = .deliver m b ∧ x = (m, b) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_offers {s : St} {a : Action} {x : Agent × Nonce} (h : x ∈ (effect s a).offers) :
    x ∈ s.offers ∨ ∃ I b N m, a = .offer I b N m ∧ x = (I, N) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_pops {s : St} {a : Action} {x : Agent × Key × Agent × Nonce}
    (h : x ∈ (effect s a).pops) :
    x ∈ s.pops ∨ ∃ H K I N m, a = .prove H K I N m ∧ x = (H, K, I, N) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_issued {s : St} {a : Action} {x : Issuance} (h : x ∈ (effect s a).issued) :
    x ∈ s.issued ∨ ∃ I IK C K N CC m, a = .issue I IK C K N CC m ∧ x = ⟨I, IK, C, K, N, CC⟩ := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_stores {s : St} {a : Action} {x : Agent × Cred} (h : x ∈ (effect s a).stores) :
    x ∈ s.stores ∨ ∃ H c, a = .store H c ∧ x = (H, c) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_requests {s : St} {a : Action} {x : Agent × Nonce × Claims}
    (h : x ∈ (effect s a).requests) :
    x ∈ s.requests ∨ ∃ V b N R m, a = .request V b N R m ∧ x = (V, N, R) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_consents {s : St} {a : Action} {x : Agent × Agent × Nonce × Claims}
    (h : x ∈ (effect s a).consents) :
    x ∈ s.consents ∨ ∃ H V N D, a = .consent H V N D ∧ x = (H, V, N, D) := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_presentations {s : St} {a : Action} {x : Presentation}
    (h : x ∈ (effect s a).presentations) :
    x ∈ s.presentations ∨
      ∃ H V N c D m, a = .present H V N c D m ∧ x = ⟨H, c.key, V, N, c, D⟩ := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h; split at h <;> exact Or.inl h
  | _ => mem_tac

theorem mem_accepted {s : St} {a : Action} {x : Acceptance} (h : x ∈ (effect s a).accepted) :
    x ∈ s.accepted ∨ ∃ V m K V' N c D, a = .verify V m true ∧
      s.termOf m = some (.pres K V' N c D) ∧ x = ⟨V, K, V', N, c, D, s.revocations.length⟩ := by
  cases a with
  | verify V m acc =>
    cases acc
    · exact Or.inl h
    · simp only [effect] at h
      split at h
      · next K V' N c D ht =>
        rcases List.mem_append.mp h with h | h
        · exact Or.inl h
        · exact Or.inr ⟨V, m, K, V', N, c, D, rfl, ht, List.mem_singleton.mp h⟩
      · exact Or.inl h
  | _ => mem_tac

/-! ## What the adversary knows, and what is genuine -/

theorem Grows.knows {s t : St} (g : Grows s t) {u : Term} (h : s.Knows u) : t.Knows u :=
  let ⟨x, hx, hw⟩ := h; ⟨x, g.msgs x hx, hw⟩

theorem Grows.deliveredTo {s t : St} (g : Grows s t) {u : Term} {b : Agent}
    (h : s.DeliveredTo u b) : t.DeliveredTo u b :=
  let ⟨x, hx, h1, h2⟩ := h; ⟨x, g.msgs x hx, h1, g.delivered _ h2⟩

theorem knows_effect {s : St} {a : Action} {u : Term} (h : (effect s a).Knows u) :
    s.Knows u ∨ ∃ x ∈ sent a, Within u x.2 := by
  obtain ⟨x, hx, hw⟩ := h
  rw [msgs_effect] at hx
  rcases List.mem_append.mp hx with hx | hx
  · exact Or.inl ⟨x, hx, hw⟩
  · exact Or.inr ⟨x, hx, hw⟩

theorem within_cases {t u : Term} (h : Within t u) :
    t = u ∨ ∃ K V N c D, u = .pres K V N c D ∧ t = .cred c := by
  rcases h with h | h
  · exact Or.inl h
  · right
    cases u with
    | pres K V N c D =>
      simp only [Term.embedded, Option.map_some, Option.some.injEq] at h
      exact ⟨K, V, N, c, D, rfl, h.symm⟩
    | _ => simp [Term.embedded] at h

theorem termOf_mem {s : St} {m : MsgId} {t : Term} (h : s.termOf m = some t) :
    (m, t) ∈ s.msgs := by
  unfold St.termOf at h
  cases hf : s.msgs.find? (fun x => x.1 == m) with
  | none => rw [hf] at h; cases h
  | some x =>
    rw [hf] at h
    have h1 := List.mem_of_find?_eq_some hf
    have h2 := List.find?_some hf
    simp only [Option.map_some, Option.some.injEq] at h
    simp only [beq_iff_eq] at h2
    obtain ⟨m', t'⟩ := x
    simp only at h h2
    subst h h2
    exact h1

/-- A signed term whose key is not compromised was made by an honest step: a possession proof by
an owner of its key, a credential by an issuance, a presentation by an owner of its key. -/
def Genuine (σ : Setup) (s : St) : Term → Prop
  | .pop K I N => K ∉ σ.compromised → ∃ H, σ.HasKey H K ∧ (H, K, I, N) ∈ s.pops
  | .cred c => c.ikey ∉ σ.compromised → ∃ x ∈ s.issued, x.toCred = c
  | .pres K V N c D => K ∉ σ.compromised →
    ∃ H, σ.HasKey H K ∧ (⟨H, K, V, N, c, D⟩ : Presentation) ∈ s.presentations
  | _ => True

theorem Grows.genuine {σ : Setup} {s t : St} (g : Grows s t) {u : Term} (h : Genuine σ s u) :
    Genuine σ t u := by
  cases u with
  | pop K I N => intro hk; obtain ⟨H, hH, hp⟩ := h hk; exact ⟨H, hH, g.pops _ hp⟩
  | cred c => intro hk; obtain ⟨x, hx, he⟩ := h hk; exact ⟨x, g.issued _ hx, he⟩
  | pres K V N c D =>
    intro hk; obtain ⟨H, hH, hp⟩ := h hk; exact ⟨H, hH, g.presentations _ hp⟩
  | _ => trivial

theorem take_of_prefix {α : Type} {l l' : List α} (h : l <+: l') {n : Nat} (hn : n ≤ l.length) :
    l'.take n = l.take n := by
  obtain ⟨r, rfl⟩ := h
  exact List.take_append_of_le_length hn

/-- What an acceptance establishes (the `verify` checks, with the status it read). -/
structure AccFacts (σ : Setup) (s : St) (a : Acceptance) : Prop where
  delivered : s.DeliveredTo a.term a.verifier
  aud : a.aud = a.verifier
  binding : a.key = a.cred.key
  requested : ∃ R, (a.verifier, a.nonce, R) ∈ s.requests ∧ a.claims ⊆ R
  inCred : a.claims ⊆ a.cred.claims
  trusted : (a.verifier, a.cred.issuer) ∈ σ.trust
  ikey : σ.HasKey a.cred.issuer a.cred.ikey
  seen : a.revSeen ≤ s.revocations.length
  unrevoked : ∀ r ∈ s.revocations.take a.revSeen, r.2 ≠ a.cred.id

theorem Grows.accFacts {σ : Setup} {s t : St} (g : Grows s t) {a : Acceptance}
    (h : AccFacts σ s a) : AccFacts σ t a where
  delivered := g.deliveredTo h.delivered
  aud := h.aud
  binding := h.binding
  requested := let ⟨R, hR, hD⟩ := h.requested; ⟨R, g.requests _ hR, hD⟩
  inCred := h.inCred
  trusted := h.trusted
  ikey := h.ikey
  seen := Nat.le_trans h.seen g.revocations.length_le
  unrevoked := by rw [take_of_prefix g.revocations h.seen]; exact h.unrevoked

/-! ## The invariant -/

/-- What holds in every reachable state. -/
structure Inv (σ : Setup) (s : St) : Prop where
  /-- Every known signed term with an honest key was made by an honest step. -/
  genuine : ∀ t, s.Knows t → Genuine σ s t
  /-- A stored credential is bound to a key of its holder, and was received. -/
  stores : ∀ H c, (H, c) ∈ s.stores → σ.HasKey H c.key ∧ s.Knows (.cred c)
  /-- An issuance follows an offer of its issuer and a delivered possession proof. -/
  issued : ∀ x ∈ s.issued, (x.issuer, x.nonce) ∈ s.offers ∧ σ.HasKey x.issuer x.ikey ∧
    s.DeliveredTo (.pop x.key x.issuer x.nonce) x.issuer
  /-- One issuance per nonce. -/
  issuedOnce : ∀ x ∈ s.issued, ∀ y ∈ s.issued, x.nonce = y.nonce → x = y
  /-- A presentation follows a consent, from a stored credential bound to the holder's key. -/
  presented : ∀ p ∈ s.presentations,
    (∃ D', (p.holder, p.verifier, p.nonce, D') ∈ s.consents ∧ p.claims ⊆ D') ∧
      (p.holder, p.cred) ∈ s.stores ∧ σ.HasKey p.holder p.key ∧ p.key = p.cred.key ∧
      p.claims ⊆ p.cred.claims
  /-- A consent answers a delivered request and stays within it. -/
  consented : ∀ H V N D', (H, V, N, D') ∈ s.consents →
    ∃ x ∈ s.msgs, (x.1, H) ∈ s.delivered ∧ Covers V N D' x.2
  /-- An acceptance passed the verifier's checks. -/
  accepted : ∀ a ∈ s.accepted, AccFacts σ s a
  /-- One acceptance per verifier and nonce. -/
  acceptedOnce : ∀ a ∈ s.accepted, ∀ b ∈ s.accepted, a.verifier = b.verifier →
    a.nonce = b.nonce → a = b

theorem inv_empty (σ : Setup) : Inv σ St.empty where
  genuine := fun _ ⟨_, hx, _⟩ => by cases hx
  stores := fun _ _ h => by cases h
  issued := fun _ h => by cases h
  issuedOnce := fun _ h => by cases h
  presented := fun _ h => by cases h
  consented := fun _ _ _ _ h => by cases h
  accepted := fun _ h => by cases h
  acceptedOnce := fun _ h => by cases h

theorem mem_effect_self {α : Type} {l : List α} {x : α} : x ∈ l ++ [x] :=
  List.mem_append_right _ (List.mem_singleton_self _)

/-- A new message keeps `genuine`. -/
theorem genuine_sent {σ : Setup} {s : St} {a : Action} (hi : Inv σ s) (he : Enabled σ s a)
    {x : MsgId × Term} (hx : x ∈ sent a) {u : Term} (hw : Within u x.2) :
    Genuine σ (effect s a) u := by
  have g := grows_effect s a
  cases a with
  | offer I b N m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, _, _, h, _⟩
    · trivial
    · cases h
  | prove H K I N m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, _, _, h, _⟩
    · intro _; exact ⟨H, he.1, by simp only [effect, St.send]; exact mem_effect_self⟩
    · cases h
  | issue I IK C K N CC m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, _, _, h, _⟩
    · intro _
      exact ⟨⟨I, IK, C, K, N, CC⟩, by simp only [effect, St.send]; exact mem_effect_self, rfl⟩
    · cases h
  | request V b N R m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, _, _, h, _⟩
    · trivial
    · cases h
  | present H V N c D m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, c', _, h, rfl⟩
    · intro _
      exact ⟨H, he.2.2.1, by simp only [effect, St.send]; exact mem_effect_self⟩
    · cases h
      exact g.genuine (hi.genuine _ (hi.stores H c he.2.1).2)
  | send b m =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    rcases within_cases hw with rfl | ⟨_, _, _, _, _, h, _⟩
    · trivial
    · cases h
  | inject m t =>
    simp only [sent, List.mem_singleton] at hx; subst hx
    obtain ⟨_, hd⟩ := he
    rcases within_cases hw with rfl | ⟨K, V, N, c, D, rfl, rfl⟩
    · cases u with
      | pop K I N =>
        rcases hd with hk | hk
        · intro hn; exact absurd hk hn
        · exact g.genuine (hi.genuine _ hk)
      | cred c =>
        rcases hd with hk | hk
        · intro hn; exact absurd hk hn
        · exact g.genuine (hi.genuine _ hk)
      | pres K V N c D =>
        rcases hd.1 with hk | hk
        · intro hn; exact absurd hk hn
        · exact g.genuine (hi.genuine _ hk)
      | _ => trivial
    · rcases hd.2 with hk | hk
      · intro hn; exact absurd hk hn
      · exact g.genuine (hi.genuine _ hk)
  | _ => simp [sent] at hx

/-- Every enabled move keeps the invariant. -/
theorem inv_step {σ : Setup} {s : St} {a : Action} (hi : Inv σ s) (he : Enabled σ s a) :
    Inv σ (effect s a) := by
  have g := grows_effect s a
  constructor
  · intro u hu
    rcases knows_effect hu with hu | ⟨x, hx, hw⟩
    · exact g.genuine (hi.genuine u hu)
    · exact genuine_sent hi he hx hw
  · intro H c hc
    rcases mem_stores hc with hc | ⟨H', c', rfl, hx⟩
    · obtain ⟨h1, h2⟩ := hi.stores H c hc; exact ⟨h1, g.knows h2⟩
    · simp only [Prod.mk.injEq] at hx; obtain ⟨rfl, rfl⟩ := hx
      obtain ⟨hk, x, hx, ht, _⟩ := he
      exact ⟨hk, g.knows ⟨x, hx, Or.inl ht.symm⟩⟩
  · intro x hx
    rcases mem_issued hx with hx | ⟨I, IK, C, K, N, CC, m, rfl, rfl⟩
    · obtain ⟨h1, h2, h3⟩ := hi.issued x hx; exact ⟨g.offers _ h1, h2, g.deliveredTo h3⟩
    · obtain ⟨h1, h2, _, h4, _, _⟩ := he
      exact ⟨g.offers _ h2, h1, g.deliveredTo h4⟩
  · intro x hx y hy hn
    rcases mem_issued hx with hx₀ | ⟨I, IK, C, K, N, CC, m, ha, rfl⟩ <;>
      rcases mem_issued hy with hy₀ | ⟨I', IK', C', K', N', CC', m', ha', rfl⟩
    · exact hi.issuedOnce x hx₀ y hy₀ hn
    · subst ha'; exact absurd hn (he.2.2.1 x hx₀)
    · subst ha; exact absurd hn.symm (he.2.2.1 y hy₀)
    · subst ha; cases ha'; rfl
  · intro p hp
    rcases mem_presentations hp with hp | ⟨H, V, N, c, D, m, rfl, rfl⟩
    · obtain ⟨⟨D', h1, h2⟩, h3, h4⟩ := hi.presented p hp
      exact ⟨⟨D', g.consents _ h1, h2⟩, g.stores _ h3, h4⟩
    · obtain ⟨⟨⟨H', V', N', D'⟩, hc, h1, h2, h3, hD⟩, hs, hk, hDc, _⟩ := he
      simp only at h1 h2 h3; subst h1 h2 h3
      exact ⟨⟨D', g.consents _ hc, hD⟩, g.stores _ hs, hk, rfl, hDc⟩
  · intro H V N D hc
    rcases mem_consents hc with hc | ⟨H', V', N', D', rfl, hx⟩
    · obtain ⟨x, hx, h1, h2⟩ := hi.consented H V N D hc
      exact ⟨x, g.msgs _ hx, g.delivered _ h1, h2⟩
    · simp only [Prod.mk.injEq] at hx; obtain ⟨rfl, rfl, rfl, rfl⟩ := hx
      obtain ⟨x, hx, h1, h2⟩ := he
      exact ⟨x, g.msgs _ hx, g.delivered _ h1, h2⟩
  · intro b hb
    rcases mem_accepted hb with hb | ⟨V, m, K, V', N, c, D, rfl, ht, rfl⟩
    · exact g.accFacts (hi.accepted b hb)
    · obtain ⟨hd, hacc⟩ := he
      rw [ht] at hacc
      obtain ⟨⟨⟨V₀, N₀, R⟩, hR, h1, h2, hDR⟩, _, hK, htr, hik, hrev, hDc, haud⟩ := hacc
      simp only at h1 h2; subst V₀ N₀
      have hrv : (effect s (.verify V m true)).revocations = s.revocations := by
        simp only [effect]; split <;> rfl
      exact
        { delivered := ⟨(m, .pres K V' N c D), g.msgs _ (termOf_mem ht), rfl, g.delivered _ hd⟩
          aud := haud
          binding := hK
          requested := ⟨R, g.requests _ hR, hDR⟩
          inCred := hDc
          trusted := htr
          ikey := hik
          seen := by rw [hrv]; exact Nat.le_refl _
          unrevoked := by rw [hrv, List.take_length]; exact hrev }
  · intro b hb b' hb' hv hn
    rcases mem_accepted hb with hb₀ | ⟨V, m, K, V', N, c, D, ha, ht, rfl⟩ <;>
      rcases mem_accepted hb' with hb₀' | ⟨V₁, m₁, K₁, V₁', N₁, c₁, D₁, ha₁, ht₁, rfl⟩
    · exact hi.acceptedOnce b hb₀ b' hb₀' hv hn
    · subst ha₁
      obtain ⟨_, hacc⟩ := he
      rw [ht₁] at hacc
      obtain ⟨_, hnot, _⟩ := hacc
      exact absurd ⟨hv, hn⟩ (hnot b hb₀)
    · subst ha
      obtain ⟨_, hacc⟩ := he
      rw [ht] at hacc
      obtain ⟨_, hnot, _⟩ := hacc
      exact absurd ⟨hv.symm, hn.symm⟩ (hnot b' hb₀')
    · subst ha; cases ha₁; rw [ht] at ht₁; cases ht₁; rfl

/-- The invariant holds in every reachable state. -/
theorem inv_of_reachable {σ : Setup} {s : St} (h : Reachable σ s) : Inv σ s := by
  induction h with
  | empty => exact inv_empty σ
  | step a _ hst ih => obtain ⟨he, rfl⟩ := step_some hst; exact inv_step ih he

/-! ## The theorems -/

section
variable {σ : Setup} {s : St}

/-- **An issuance needs possession.** Every issuance is on a nonce its issuer offered, signed
with the issuer's key, after a possession proof `pop(key, issuer, nonce)` was delivered to the
issuer; when the holder key is not compromised, an owner of the key made that proof. -/
theorem issue_needs_possession (hr : Reachable σ s) {x : Issuance} (hx : x ∈ s.issued) :
    (x.issuer, x.nonce) ∈ s.offers ∧ σ.HasKey x.issuer x.ikey ∧
      s.DeliveredTo (.pop x.key x.issuer x.nonce) x.issuer ∧
      (x.key ∉ σ.compromised →
        ∃ H, σ.HasKey H x.key ∧ (H, x.key, x.issuer, x.nonce) ∈ s.pops) := by
  have hi := inv_of_reachable hr
  obtain ⟨h1, h2, h3⟩ := hi.issued x hx
  refine ⟨h1, h2, h3, fun hk => ?_⟩
  obtain ⟨y, hy, hyt, _⟩ := h3
  exact hi.genuine _ ⟨y, hy, Or.inl hyt.symm⟩ hk

/-- **One issuance per offer.** Two issuances by the same issuer on the same nonce are one (the
invariant `Inv.issuedOnce` has it even across issuers, as honest nonces are fresh). -/
theorem issue_once_per_offer (hr : Reachable σ s) {x y : Issuance} (hx : x ∈ s.issued)
    (hy : y ∈ s.issued) : x.issuer = y.issuer → x.nonce = y.nonce → x = y :=
  fun _ hN => (inv_of_reachable hr).issuedOnce x hx y hy hN

/-- **A presentation needs consent.** Every honest presentation discloses claims its holder
consented to disclose to that verifier on that nonce, from a credential it stores, bound to the
key it signs with, which is the holder's, and which has the disclosed claims. -/
theorem present_needs_consent (hr : Reachable σ s) {p : Presentation}
    (hp : p ∈ s.presentations) :
    (∃ D', (p.holder, p.verifier, p.nonce, D') ∈ s.consents ∧ p.claims ⊆ D') ∧
      (p.holder, p.cred) ∈ s.stores ∧ σ.HasKey p.holder p.key ∧ p.key = p.cred.key ∧
      p.claims ⊆ p.cred.claims :=
  (inv_of_reachable hr).presented p hp

/-- **Disclosure stays within consent, and consent within the request.** Every consent
`(H, V, N, D')` answers a request `req(V, N, R)` delivered to `H` with `D' ⊆ R`. -/
theorem disclosure_within_consent (hr : Reachable σ s) {H V : Agent} {N : Nonce} {D' : Claims}
    (h : (H, V, N, D') ∈ s.consents) :
    ∃ m R, (m, Term.req V N R) ∈ s.msgs ∧ (m, H) ∈ s.delivered ∧ D' ⊆ R := by
  obtain ⟨⟨m, t⟩, hx, hd, hc⟩ := (inv_of_reachable hr).consented H V N D' h
  cases t with
  | req V' N' R =>
    obtain ⟨h1, h2, hD⟩ := hc
    subst h1 h2
    exact ⟨m, R, hx, hd, hD⟩
  | _ => cases hc

/-- Disclosed ⊆ consented ⊆ requested: every honest presentation discloses only claims of a
request for its verifier and nonce that was delivered to its holder. -/
theorem presentation_within_request (hr : Reachable σ s) {p : Presentation}
    (hp : p ∈ s.presentations) :
    ∃ m R, (m, Term.req p.verifier p.nonce R) ∈ s.msgs ∧ (m, p.holder) ∈ s.delivered ∧
      p.claims ⊆ R := by
  obtain ⟨⟨D', hc, hD⟩, _⟩ := present_needs_consent hr hp
  obtain ⟨m, R, hx, hd, hR⟩ := disclosure_within_consent hr hc
  exact ⟨m, R, hx, hd, List.Subset.trans hD hR⟩

/-- **Acceptance is sound.** Every acceptance by `V` of `pres(K, V', N, cred(I, IK, C, K', CC),
D)` is of a message delivered to `V`, with `V' = V` (audience), `K = K'` (holder binding), a
request of `V` for `N` with `D ⊆ R`, `D ⊆ CC`, `V` trusting `I`, `IK` a key of `I`, and `C` not
revoked by any revocation made before the acceptance. When `IK` is not compromised, `I` issued
exactly that credential; when `K` is not compromised, an owner of `K` made exactly that
presentation, for `V` and `N`. -/
theorem accept_sound (hr : Reachable σ s) {a : Acceptance} (ha : a ∈ s.accepted) :
    s.DeliveredTo a.term a.verifier ∧ a.aud = a.verifier ∧ a.key = a.cred.key ∧
      (∃ R, (a.verifier, a.nonce, R) ∈ s.requests ∧ a.claims ⊆ R) ∧
      a.claims ⊆ a.cred.claims ∧
      (a.verifier, a.cred.issuer) ∈ σ.trust ∧ σ.HasKey a.cred.issuer a.cred.ikey ∧
      (∀ r ∈ s.revocations.take a.revSeen, r.2 ≠ a.cred.id) ∧
      (a.cred.ikey ∉ σ.compromised → ∃ x ∈ s.issued, x.toCred = a.cred) ∧
      (a.key ∉ σ.compromised → ∃ H, σ.HasKey H a.key ∧
        (⟨H, a.key, a.verifier, a.nonce, a.cred, a.claims⟩ : Presentation) ∈ s.presentations) := by
  have hi := inv_of_reachable hr
  have f := hi.accepted a ha
  obtain ⟨y, hy, hyt, _⟩ := f.delivered
  refine ⟨f.delivered, f.aud, f.binding, f.requested, f.inCred, f.trusted, f.ikey, f.unrevoked,
    fun hk => ?_, fun hk => ?_⟩
  · exact hi.genuine (.cred a.cred) ⟨y, hy, Or.inr (by rw [hyt]; rfl)⟩ hk
  · have := hi.genuine a.term ⟨y, hy, Or.inl hyt.symm⟩ hk
    rw [← f.aud]
    exact this

/-- **One acceptance per nonce.** Two acceptances by the same verifier on the same nonce are
one. -/
theorem accept_once_per_nonce (hr : Reachable σ s) {a b : Acceptance} (ha : a ∈ s.accepted)
    (hb : b ∈ s.accepted) (hv : a.verifier = b.verifier) (hn : a.nonce = b.nonce) : a = b :=
  (inv_of_reachable hr).acceptedOnce a ha b hb hv hn

end

end Vertep.Wallet
