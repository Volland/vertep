import VertepLean.Custody

/-!
# One writer per epoch, and committed entries survive (m7 10.2)

In every state reachable from the initial one, when the quorum is a majority of the custodian
set (`Config.Majority`):

* `one_writer_per_epoch` — two elections for the same epoch elect the same host;
* `committed_survives_election` — a log committed in epoch `e₀` is a prefix of the log adopted
  by every host elected in a later epoch;
* `committed_survives_accept` — and of every log any custodian accepts in a later epoch;
* `committed_in_custodian`, `committed_in_writer_log` — so of every custodian's log accepted in
  a later epoch, and of the log of every writer of a later epoch.

The election and the commit may happen in either order: an election of `e > e₀` after which a
quorum still accepts in `e₀` cannot happen, because the two quorums intersect in a custodian
that promised `e` and so refuses `e₀`.

The proof is the Paxos argument, phrased as an inductive invariant (`Inv`): every adopted and
every accepted log of epoch `e` is *safe at* `e` (`SafeAt`): it extends every log that is
*choosable* in an earlier epoch, that is, every log some quorum has accepted or can still
accept there (`Choosable`). Choosability only shrinks as the system moves (promises rise,
and a custodian accepts in `e₀` only if it could still do so), so safety, once established,
is kept.
-/

namespace Vertep.Custody

variable (cfg : Config)

/-! ## Quorums intersect -/

theorem length_le_of_nodup_subset :
    ∀ {l m : List String}, l.Nodup → (∀ x ∈ l, x ∈ m) → l.length ≤ m.length
  | [], _, _, _ => Nat.zero_le _
  | a :: t, m, hn, hs => by
    have ha : a ∈ m := hs a (List.mem_cons_self ..)
    have hn' := List.nodup_cons.mp hn
    have ht : ∀ x ∈ t, x ∈ m.erase a := fun x hx =>
      (List.mem_erase_of_ne (fun h => hn'.1 (by rw [← h]; exact hx))).mpr (hs x (List.mem_cons_of_mem _ hx))
    have h1 := length_le_of_nodup_subset hn'.2 ht
    rw [List.length_erase_of_mem ha] at h1
    have h2 : 0 < m.length := List.length_pos_of_mem ha
    simp only [List.length_cons]; omega

/-- **Quorum intersection.** Two quorums of a majority configuration share a custodian. -/
theorem quorum_inter (hm : cfg.Majority) {Q₁ Q₂ : List String} (h₁ : Quorum cfg Q₁)
    (h₂ : Quorum cfg Q₂) : ∃ c, c ∈ Q₁ ∧ c ∈ Q₂ := by
  apply Classical.byContradiction
  intro hn
  have hd : (Q₁ ++ Q₂).Nodup :=
    List.nodup_append.mpr ⟨h₁.1, h₂.1, fun a ha b hb hab => hn ⟨a, ha, hab ▸ hb⟩⟩
  have hl := length_le_of_nodup_subset hd (fun x hx => by
    rcases List.mem_append.mp hx with hx | hx
    · exact h₁.2.1 x hx
    · exact h₂.2.1 x hx)
  rw [List.length_append] at hl
  have := h₁.2.2
  have := h₂.2.2
  unfold Config.Majority at hm
  omega

/-! ## The append decision -/

/-- What an append does: acknowledge and take the writer's chain (from a newer epoch, or
extending the custodian's log), acknowledge without changing the log (a stale batch of the
accepted epoch), or change nothing. -/
theorem decideAppend_cases (c : Cust) (e after : Nat) (w : Log) :
    (decideAppend c e after w = (.ack, c.accept e w) ∧ c.promised ≤ e ∧
        (c.acc < e ∨ c.log <+: w)) ∨
      (decideAppend c e after w = (.ack, c.accept e c.log) ∧ c.promised ≤ e ∧ ¬ c.acc < e) ∨
      ∃ o, o ≠ .ack ∧ decideAppend c e after w = (o, c) := by
  by_cases h1 : e < c.promised
  · refine Or.inr (Or.inr ⟨.refused, by simp, ?_⟩)
    simp only [decideAppend, h1, ↓reduceIte]
  have hp : c.promised ≤ e := by omega
  by_cases h2 : c.log.length < after
  · refine Or.inr (Or.inr ⟨.behind, by simp, ?_⟩)
    simp only [decideAppend, h1, h2, ↓reduceIte]
  by_cases h3 : List.take after c.log = List.take after w
  · by_cases h4 : c.acc < e
    · refine Or.inl ⟨?_, hp, Or.inl h4⟩
      simp only [decideAppend, h1, h2, h3, h4, ↓reduceIte]
    by_cases h5 : c.log <+: w
    · refine Or.inl ⟨?_, hp, Or.inr h5⟩
      simp only [decideAppend, h1, h2, h3, h4, h5, ↓reduceIte]
    by_cases h6 : w <+: c.log
    · refine Or.inr (Or.inl ⟨?_, hp, h4⟩)
      simp only [decideAppend, h1, h2, h3, h4, h5, h6, ↓reduceIte]
    · refine Or.inr (Or.inr ⟨.diverged, by simp, ?_⟩)
      simp only [decideAppend, h1, h2, h3, h4, h5, h6, ↓reduceIte]
  · by_cases h4 : w.length = after ∧ ¬ c.acc < e
    · refine Or.inr (Or.inl ⟨?_, hp, h4.2⟩)
      simp only [decideAppend, h1, h2, h3, h4, ↓reduceIte, not_false_eq_true, and_self]
    by_cases h5 : ¬ c.acc < e ∧ after < c.log.length
    · have h4' : ¬ w.length = after := fun h => h4 ⟨h, h5.1⟩
      refine Or.inr (Or.inr ⟨.diverged, by simp, ?_⟩)
      simp only [decideAppend, h1, h2, h3, h4', h5, ↓reduceIte, not_false_eq_true, and_self,
        false_and]
    · refine Or.inr (Or.inr ⟨.failed, by simp, ?_⟩)
      simp only [decideAppend, h1, h2, h3, h4, h5, ↓reduceIte]

theorem append_snd (s : Sys) (c : String) (e after : Nat) (w : Log) :
    (append s c e after w).2 =
      { s with
        cust := upd s.cust c (decideAppend (s.cust c) e after w).2
        accepted := if (decideAppend (s.cust c) e after w).1 = .ack then
            s.accepted ++ [(c, e, (decideAppend (s.cust c) e after w).2.log)]
          else s.accepted } := rfl

theorem upd_self {α β : Type} [DecidableEq α] (f : α → β) (a : α) : upd f a (f a) = f := by
  funext x; unfold upd; split
  · next h => subst h; rfl
  · rfl

/-! ## The invariant -/

/-- `L` can be chosen in epoch `e₀`: some quorum's custodians each accepted in `e₀` a log
extending `L`, or can still accept in `e₀` (have promised no higher epoch). -/
def Choosable (s : Sys) (L : Log) (e₀ : Nat) : Prop :=
  ∃ Q, Quorum cfg Q ∧
    ∀ c ∈ Q, (∃ l, (c, e₀, l) ∈ s.accepted ∧ L <+: l) ∨ (s.cust c).promised ≤ e₀

/-- `l` is safe at `e`: it extends every log choosable in an earlier epoch. -/
def SafeAt (s : Sys) (l : Log) (e : Nat) : Prop :=
  ∀ e₀ L, e₀ < e → Choosable cfg s L e₀ → L <+: l

/-- What holds in every reachable state. -/
structure Inv (s : Sys) : Prop where
  /-- A custodian's accepted epoch is at most its promised one. -/
  cp : ∀ c, (s.cust c).acc ≤ (s.cust c).promised
  /-- A custodian's current log is fresh or one it accepted. -/
  cur : ∀ c, ((s.cust c).acc = 0 ∧ (s.cust c).log = []) ∨
    (c, (s.cust c).acc, (s.cust c).log) ∈ s.accepted
  /-- A custodian accepted only up to its accepted epoch, and its logs of that epoch are
  prefixes of its current log. -/
  ca : ∀ c e₀ l, (c, e₀, l) ∈ s.accepted →
    e₀ ≤ (s.cust c).acc ∧ (e₀ = (s.cust c).acc → l <+: (s.cust c).log)
  pbound : ∀ p ∈ s.promises, p.epoch ≤ (s.cust p.cust).promised
  ppos : ∀ p ∈ s.promises, 0 < p.epoch
  /-- A custodian promises an epoch to one host. -/
  puniq : ∀ p ∈ s.promises, ∀ p' ∈ s.promises, p.cust = p'.cust → p.epoch = p'.epoch →
    p.host = p'.host
  /-- A promise reports a fresh state or one the custodian accepted. -/
  prep : ∀ p ∈ s.promises, (p.acc = 0 ∧ p.log = []) ∨ (p.cust, p.acc, p.log) ∈ s.accepted
  /-- What a custodian accepted below an epoch it promised was accepted before the promise. -/
  pacc : ∀ p ∈ s.promises, ∀ e₀ l, (p.cust, e₀, l) ∈ s.accepted → e₀ < p.epoch →
    e₀ ≤ p.acc ∧ (e₀ = p.acc → l <+: p.log)
  equorum : ∀ e h A, (e, h, A) ∈ s.elections → ∃ Q, Quorum cfg Q ∧
    ∀ c ∈ Q, ∃ p ∈ s.promises, p.cust = c ∧ p.epoch = e ∧ p.host = h
  epos : ∀ e h A, (e, h, A) ∈ s.elections → 0 < e
  ewlog : ∀ e h A, (e, h, A) ∈ s.elections → A <+: s.wlog e h
  /-- Every accepted log lies between the adopted log and the writer's log of its epoch. -/
  aw : ∀ c e l, (c, e, l) ∈ s.accepted →
    ∃ h A, (e, h, A) ∈ s.elections ∧ A <+: l ∧ l <+: s.wlog e h
  esafe : ∀ e h A, (e, h, A) ∈ s.elections → SafeAt cfg s A e
  asafe : ∀ c e l, (c, e, l) ∈ s.accepted → SafeAt cfg s l e

theorem inv_empty : Inv cfg Sys.empty where
  cp := fun _ => Nat.le_refl _
  cur := fun _ => Or.inl ⟨rfl, rfl⟩
  ca := by intro _ _ _ h; cases h
  pbound := by intro _ h; cases h
  ppos := by intro _ h; cases h
  puniq := by intro _ h; cases h
  prep := by intro _ h; cases h
  pacc := by intro _ h; cases h
  equorum := by intro _ _ _ h; cases h
  epos := by intro _ _ _ h; cases h
  ewlog := by intro _ _ _ h; cases h
  aw := by intro _ _ _ h; cases h
  esafe := by intro _ _ _ h; cases h
  asafe := by intro _ _ _ h; cases h

/-- **One host per epoch**, from the invariant. -/
theorem one_host (hm : cfg.Majority) {s : Sys} (hs : Inv cfg s) {e : Nat} {h₁ h₂ : String}
    {A₁ A₂ : Log} (e₁ : (e, h₁, A₁) ∈ s.elections) (e₂ : (e, h₂, A₂) ∈ s.elections) :
    h₁ = h₂ := by
  obtain ⟨Q₁, hQ₁, hp₁⟩ := hs.equorum _ _ _ e₁
  obtain ⟨Q₂, hQ₂, hp₂⟩ := hs.equorum _ _ _ e₂
  obtain ⟨c, hc₁, hc₂⟩ := quorum_inter cfg hm hQ₁ hQ₂
  obtain ⟨p₁, hm₁, rfl, he₁, rfl⟩ := hp₁ c hc₁
  obtain ⟨p₂, hm₂, hc, he₂, rfl⟩ := hp₂ p₁.cust hc₂
  exact hs.puniq p₁ hm₁ p₂ hm₂ hc.symm (he₁.trans he₂.symm)

/-- Choosability only shrinks along a move that raises no promise downwards and adds only
acceptances the custodian could still make. -/
theorem choosable_back {s t : Sys} (hp : ∀ c, (s.cust c).promised ≤ (t.cust c).promised)
    (hacc : ∀ c e l, (c, e, l) ∈ t.accepted → (c, e, l) ∈ s.accepted ∨ (s.cust c).promised ≤ e)
    {L : Log} {e₀ : Nat} (h : Choosable cfg t L e₀) : Choosable cfg s L e₀ := by
  obtain ⟨Q, hQ, hc⟩ := h
  refine ⟨Q, hQ, fun c hc' => ?_⟩
  rcases hc c hc' with ⟨l, hl, hL⟩ | hle
  · rcases hacc c e₀ l hl with h | h
    · exact Or.inl ⟨l, h, hL⟩
    · exact Or.inr h
  · exact Or.inr (Nat.le_trans (hp c) hle)

theorem safeAt_back {s t : Sys} (hp : ∀ c, (s.cust c).promised ≤ (t.cust c).promised)
    (hacc : ∀ c e l, (c, e, l) ∈ t.accepted → (c, e, l) ∈ s.accepted ∨ (s.cust c).promised ≤ e)
    {l : Log} {e : Nat} (h : SafeAt cfg s l e) : SafeAt cfg t l e :=
  fun e₀ L hlt hc => h e₀ L hlt (choosable_back cfg hp hacc hc)

/-! ## Each move keeps the invariant -/

theorem inv_prepare {s : Sys} (hs : Inv cfg s) (c : String) (e : Nat) (h : String)
    (hp : (s.cust c).promised < e) : Inv cfg (prepare s c e h) := by
  have hcust : ∀ x, (prepare s c e h).cust x =
      if x = c then { s.cust c with promised := e } else s.cust x := fun _ => rfl
  have hacc : ∀ x, ((prepare s c e h).cust x).acc = (s.cust x).acc := by
    intro x; rw [hcust]; split
    · next hx => subst hx; rfl
    · rfl
  have hlog : ∀ x, ((prepare s c e h).cust x).log = (s.cust x).log := by
    intro x; rw [hcust]; split
    · next hx => subst hx; rfl
    · rfl
  have hprom : ∀ x, (s.cust x).promised ≤ ((prepare s c e h).cust x).promised := by
    intro x; rw [hcust]; split
    · next hx => subst hx; exact Nat.le_of_lt hp
    · exact Nat.le_refl _
  have hmem : ∀ p, p ∈ (prepare s c e h).promises ↔
      p ∈ s.promises ∨ p = ⟨c, e, h, (s.cust c).acc, (s.cust c).log⟩ := by
    intro p; simp [prepare]
  have hback : ∀ x e' l, (x, e', l) ∈ (prepare s c e h).accepted →
      (x, e', l) ∈ s.accepted ∨ (s.cust x).promised ≤ e' := fun _ _ _ h => Or.inl h
  refine
    { cp := ?_, cur := ?_, ca := ?_, pbound := ?_, ppos := ?_, puniq := ?_, prep := ?_,
      pacc := ?_, equorum := ?_, epos := hs.epos, ewlog := hs.ewlog, aw := hs.aw,
      esafe := fun e' h' A he => safeAt_back cfg hprom hback (hs.esafe e' h' A he),
      asafe := fun c' e' l ha => safeAt_back cfg hprom hback (hs.asafe c' e' l ha) }
  · intro x; rw [hacc]; exact Nat.le_trans (hs.cp x) (hprom x)
  · intro x; rw [hacc, hlog]; exact hs.cur x
  · intro x e₀ l hx; rw [hacc, hlog]; exact hs.ca x e₀ l hx
  · intro p hpm
    rcases (hmem p).mp hpm with hpm | rfl
    · exact Nat.le_trans (hs.pbound p hpm) (hprom _)
    · show e ≤ ((prepare s c e h).cust c).promised
      rw [hcust]; simp
  · intro p hpm
    rcases (hmem p).mp hpm with hpm | rfl
    · exact hs.ppos p hpm
    · show 0 < e; omega
  · intro p hpm p' hpm' hc he
    rcases (hmem p).mp hpm with hpm | rfl <;> rcases (hmem p').mp hpm' with hpm' | rfl
    · exact hs.puniq p hpm p' hpm' hc he
    · have := hs.pbound p hpm; simp only at hc he; rw [hc, he] at this; omega
    · have := hs.pbound p' hpm'; simp only at hc he; rw [← hc, ← he] at this; omega
    · rfl
  · intro p hpm
    rcases (hmem p).mp hpm with hpm | rfl
    · exact hs.prep p hpm
    · exact hs.cur c
  · intro p hpm e₀ l hl hlt
    rcases (hmem p).mp hpm with hpm | rfl
    · exact hs.pacc p hpm e₀ l hl hlt
    · exact hs.ca c e₀ l hl
  · intro e' h' A he
    obtain ⟨Q, hQ, hc⟩ := hs.equorum e' h' A he
    exact ⟨Q, hQ, fun x hx => by
      obtain ⟨p, hp, h1⟩ := hc x hx
      exact ⟨p, (hmem p).mpr (Or.inl hp), h1⟩⟩

theorem inv_elect (hm : cfg.Majority) {s : Sys} (hs : Inv cfg s) (h : String) (e : Nat)
    (ps : List Promise) (A : Log)
    (hps : ∀ p ∈ ps, p ∈ s.promises ∧ p.epoch = e ∧ p.host = h)
    (hq : Quorum cfg (ps.map (·.cust))) (hA : Adopts ps A)
    (hnew : ∀ A', (e, h, A') ∉ s.elections) : Inv cfg (elect s h e A) := by
  have hmem : ∀ x, x ∈ (elect s h e A).elections ↔ x ∈ s.elections ∨ x = (e, h, A) := by
    intro x; simp [elect]
  have hw : ∀ e' h' A', (e', h', A') ∈ s.elections → (elect s h e A).wlog e' h' = s.wlog e' h' := by
    intro e' h' A' he
    show upd2 s.wlog e h A e' h' = s.wlog e' h'
    unfold upd2; split
    · next hh => obtain ⟨rfl, rfl⟩ := hh; exact absurd he (hnew A')
    · rfl
  -- the new election is safe
  have hsafe : SafeAt cfg s A e := by
    intro e₀ L hlt ⟨Q₀, hQ₀, hc₀⟩
    obtain ⟨c, hc0, hce⟩ := quorum_inter cfg hm hQ₀ hq
    obtain ⟨pc, hpc, rfl⟩ := List.mem_map.mp hce
    obtain ⟨hpcm, hpce, -⟩ := hps pc hpc
    have hb := hs.pbound pc hpcm
    obtain ⟨l, hl, hL⟩ : ∃ l, (pc.cust, e₀, l) ∈ s.accepted ∧ L <+: l := by
      rcases hc₀ pc.cust hc0 with h1 | h1
      · exact h1
      · omega
    obtain ⟨hle0, hlog0⟩ := hs.pacc pc hpcm e₀ l hl (hpce ▸ hlt)
    obtain ⟨pm, hpm, rfl, hmax⟩ := hA
    obtain ⟨hpmm, -, -⟩ := hps pm hpm
    by_cases hlt' : e₀ < pm.acc
    · rcases hs.prep pm hpmm with ⟨h0, -⟩ | hpa
      · omega
      · exact hs.asafe _ _ _ hpa e₀ L hlt' ⟨Q₀, hQ₀, hc₀⟩
    · have hcmp := hmax pc hpc
      have heq : e₀ = pc.acc := by omega
      have hlen : pc.log.length ≤ pm.log.length := by
        rcases hcmp with h1 | h1
        · omega
        · exact h1.2
      have hpre : pc.log <+: pm.log := by
        rcases hs.prep pc hpcm with ⟨_, h0⟩ | hpa
        · rw [h0]; exact List.nil_prefix
        rcases hs.prep pm hpmm with ⟨_, h0⟩ | hpb
        · rw [h0, List.length_nil] at hlen
          have : pc.log = [] := List.eq_nil_of_length_eq_zero (by omega)
          rw [this]; exact List.nil_prefix
        have hacc' : pm.acc = pc.acc := by omega
        obtain ⟨h₁, A₁, he₁, -, hw₁⟩ := hs.aw _ _ _ hpa
        obtain ⟨h₂, A₂, he₂, -, hw₂⟩ := hs.aw _ _ _ hpb
        rw [hacc'] at he₂ hw₂
        have := one_host cfg hm hs he₁ he₂
        subst this
        exact List.prefix_of_prefix_length_le hw₁ hw₂ hlen
      exact (hL.trans (hlog0 heq)).trans hpre
  refine
    { cp := hs.cp, cur := hs.cur, ca := hs.ca, pbound := hs.pbound, ppos := hs.ppos,
      puniq := hs.puniq, prep := hs.prep, pacc := hs.pacc, equorum := ?_, epos := ?_,
      ewlog := ?_, aw := ?_, esafe := ?_, asafe := hs.asafe }
  · intro e' h' A' he
    rcases (hmem _).mp he with he | he
    · exact hs.equorum e' h' A' he
    · simp only [Prod.mk.injEq] at he; obtain ⟨rfl, rfl, rfl⟩ := he
      refine ⟨_, hq, fun c hc => ?_⟩
      obtain ⟨p, hp, rfl⟩ := List.mem_map.mp hc
      obtain ⟨h1, h2, h3⟩ := hps p hp
      exact ⟨p, h1, rfl, h2, h3⟩
  · intro e' h' A' he
    rcases (hmem _).mp he with he | he
    · exact hs.epos e' h' A' he
    · simp only [Prod.mk.injEq] at he; obtain ⟨rfl, rfl, rfl⟩ := he
      obtain ⟨p, hp, -, -⟩ := hA
      obtain ⟨h1, h2, -⟩ := hps p hp
      exact h2 ▸ hs.ppos p h1
  · intro e' h' A' he
    rcases (hmem _).mp he with he | he
    · rw [hw e' h' A' he]; exact hs.ewlog e' h' A' he
    · simp only [Prod.mk.injEq] at he; obtain ⟨rfl, rfl, rfl⟩ := he
      show A' <+: upd2 s.wlog e' h' A' e' h'
      simp [upd2]
  · intro c e' l ha
    obtain ⟨h', A', he, h1, h2⟩ := hs.aw c e' l ha
    exact ⟨h', A', (hmem _).mpr (Or.inl he), h1, (hw e' h' A' he) ▸ h2⟩
  · intro e' h' A' he
    rcases (hmem _).mp he with he | he
    · exact hs.esafe e' h' A' he
    · simp only [Prod.mk.injEq] at he; obtain ⟨rfl, rfl, rfl⟩ := he
      exact hsafe

theorem inv_write {s : Sys} (hs : Inv cfg s) (h : String) (e : Nat) (xs : Log) :
    Inv cfg { s with wlog := upd2 s.wlog e h (s.wlog e h ++ xs) } := by
  have hext : ∀ e' h', s.wlog e' h' <+: upd2 s.wlog e h (s.wlog e h ++ xs) e' h' := by
    intro e' h'; unfold upd2; split
    · next hh => obtain ⟨rfl, rfl⟩ := hh; exact List.prefix_append _ _
    · exact List.prefix_refl _
  exact { hs with
    ewlog := fun e' h' A he => (hs.ewlog e' h' A he).trans (hext e' h')
    aw := fun c e' l ha => by
      obtain ⟨h', A, he, h1, h2⟩ := hs.aw c e' l ha
      exact ⟨h', A, he, h1, h2.trans (hext e' h')⟩ }

/-- An acknowledged append keeps the invariant: the custodian takes `l`, which is either a log
between the adopted log and the writer's log of `e` (the writer's chain), or its own log when it
already accepted in `e`. -/
theorem inv_ack {s : Sys} (hs : Inv cfg s) (c : String) (e : Nat) (l : Log) (h : String)
    (A : Log) (he : (e, h, A) ∈ s.elections) (hpe : (s.cust c).promised ≤ e)
    (hcase : (A <+: l ∧ l <+: s.wlog e h ∧ ((s.cust c).acc < e ∨ (s.cust c).log <+: l)) ∨
      (l = (s.cust c).log ∧ ¬ (s.cust c).acc < e)) :
    Inv cfg { s with
      cust := upd s.cust c ((s.cust c).accept e l)
      accepted := s.accepted ++ [(c, e, l)] } := by
  let t : Sys := { s with
      cust := upd s.cust c ((s.cust c).accept e l)
      accepted := s.accepted ++ [(c, e, l)] }
  show Inv cfg t
  have hcust : ∀ x, t.cust x = if x = c then (s.cust c).accept e l else s.cust x :=
    fun _ => rfl
  have hprom : ∀ x, (s.cust x).promised ≤ (t.cust x).promised := by
    intro x; rw [hcust]; split
    · next hx => subst hx; exact Nat.le_max_left _ _
    · exact Nat.le_refl _
  have hmem : ∀ r, r ∈ t.accepted ↔ r ∈ s.accepted ∨ r = (c, e, l) := by
    intro r; simp [t]
  have hback : ∀ x e' l', (x, e', l') ∈ t.accepted →
      (x, e', l') ∈ s.accepted ∨ (s.cust x).promised ≤ e' := by
    intro x e' l' hx
    rcases (hmem _).mp hx with hx | hx
    · exact Or.inl hx
    · simp only [Prod.mk.injEq] at hx; obtain ⟨rfl, rfl, rfl⟩ := hx; exact Or.inr hpe
  have haccle : (s.cust c).acc ≤ e := Nat.le_trans (hs.cp c) hpe
  -- in the second case, the custodian already accepted its log in `e`
  have hold : l = (s.cust c).log → ¬ (s.cust c).acc < e → (c, e, l) ∈ s.accepted := by
    intro hl ha
    have hae : (s.cust c).acc = e := by omega
    rcases hs.cur c with ⟨h0, -⟩ | hc
    · have := hs.epos _ _ _ he; omega
    · rw [hl, ← hae]; exact hc
  refine
    { cp := ?_, cur := ?_, ca := ?_, pbound := ?_, ppos := hs.ppos, puniq := hs.puniq,
      prep := ?_, pacc := ?_, equorum := hs.equorum, epos := hs.epos, ewlog := hs.ewlog,
      aw := ?_, esafe := ?_, asafe := ?_ }
  · intro x; rw [hcust]; split
    · exact Nat.le_max_right _ _
    · exact hs.cp x
  · intro x; rw [hcust]; split
    · next hx => subst hx; exact Or.inr ((hmem _).mpr (Or.inr rfl))
    · rcases hs.cur x with h0 | h0
      · exact Or.inl h0
      · exact Or.inr ((hmem _).mpr (Or.inl h0))
  · intro x e₀ l₀ hx
    rw [hcust]
    rcases (hmem _).mp hx with hx' | hx'
    · split
      · next hxc =>
        subst hxc
        obtain ⟨h1, h2⟩ := hs.ca x e₀ l₀ hx'
        refine ⟨Nat.le_trans h1 haccle, fun h3 => ?_⟩
        show l₀ <+: l
        have h4 : e₀ = (s.cust x).acc := by simp only [Cust.accept] at h3; omega
        rcases hcase with ⟨-, -, h5 | h5⟩ | ⟨h5, -⟩
        · simp only [Cust.accept] at h3; omega
        · exact (h2 h4).trans h5
        · rw [h5]; exact h2 h4
      · exact hs.ca x e₀ l₀ hx'
    · simp only [Prod.mk.injEq] at hx'; obtain ⟨rfl, rfl, rfl⟩ := hx'
      simp only [↓reduceIte, Cust.accept]
      exact ⟨Nat.le_refl _, fun _ => List.prefix_refl _⟩
  · intro p hp; exact Nat.le_trans (hs.pbound p hp) (hprom _)
  · intro p hp
    rcases hs.prep p hp with h0 | h0
    · exact Or.inl h0
    · exact Or.inr ((hmem _).mpr (Or.inl h0))
  · intro p hp e₀ l₀ hx hlt
    rcases (hmem _).mp hx with hx | hx
    · exact hs.pacc p hp e₀ l₀ hx hlt
    · simp only [Prod.mk.injEq] at hx; obtain ⟨hx1, rfl, rfl⟩ := hx
      have := hs.pbound p hp; rw [hx1] at this; omega
  · intro x e' l' hx
    rcases (hmem _).mp hx with hx | hx
    · exact hs.aw x e' l' hx
    · simp only [Prod.mk.injEq] at hx; obtain ⟨rfl, rfl, rfl⟩ := hx
      rcases hcase with ⟨h1, h2, -⟩ | ⟨h1, h2⟩
      · exact ⟨h, A, he, h1, h2⟩
      · exact hs.aw _ _ _ (hold h1 h2)
  · intro e' h' A' he'
    exact safeAt_back cfg hprom hback (hs.esafe e' h' A' he')
  · intro x e' l' hx
    rcases (hmem _).mp hx with hx | hx
    · exact safeAt_back cfg hprom hback (hs.asafe x e' l' hx)
    · simp only [Prod.mk.injEq] at hx; obtain ⟨rfl, rfl, rfl⟩ := hx
      rcases hcase with ⟨h1, -, -⟩ | ⟨h1, h2⟩
      · exact safeAt_back cfg hprom hback
          (fun e₀ L hlt hc => (hs.esafe _ _ _ he e₀ L hlt hc).trans h1)
      · exact safeAt_back cfg hprom hback (hs.asafe _ _ _ (hold h1 h2))

theorem inv_append {s : Sys} (hs : Inv cfg s) (c h : String) (e after : Nat) (A w : Log)
    (he : (e, h, A) ∈ s.elections) (hA : A <+: w) (hw : w <+: s.wlog e h) :
    Inv cfg (append s c e after w).2 := by
  rw [append_snd]
  rcases decideAppend_cases (s.cust c) e after w with ⟨hd, hp, hc⟩ | ⟨hd, hp, hc⟩ | ⟨o, ho, hd⟩
  · rw [hd]
    simp only [↓reduceIte]
    exact inv_ack cfg hs c e w h A he hp (Or.inl ⟨hA, hw, hc⟩)
  · rw [hd]
    simp only [↓reduceIte]
    exact inv_ack cfg hs c e _ h A he hp (Or.inr ⟨rfl, hc⟩)
  · rw [hd]
    simp only [ho, ↓reduceIte, upd_self]
    exact hs

theorem inv_step (hm : cfg.Majority) {s t : Sys} (hs : Inv cfg s) (h : Step cfg s t) :
    Inv cfg t := by
  cases h with
  | prepare c e h hp => exact inv_prepare cfg hs c e h hp
  | elect h e ps A hps hq hA hnew => exact inv_elect cfg hm hs h e ps A hps hq hA hnew
  | write h e A _ xs => exact inv_write cfg hs h e xs
  | append c h e after A w he hA hw _ => exact inv_append cfg hs c h e after A w he hA hw
  | stutter => exact hs

theorem inv_of_reachable (hm : cfg.Majority) {s : Sys} (h : Reachable cfg s) : Inv cfg s := by
  induction h with
  | empty => exact inv_empty cfg
  | step _ hst ih => exact inv_step cfg hm ih hst

/-! ## The theorems -/

/-- **One writer per epoch.** Two elections for the same epoch elect the same host. -/
-- @tg: implements:: [[openspec:durable-objects#Single Writer Per Object]] {note: "Lean proof"}
-- @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs#Epoch won]] {level: unit}
theorem one_writer_per_epoch (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s) {e : Nat}
    {h₁ h₂ : String} {A₁ A₂ : Log} (e₁ : (e, h₁, A₁) ∈ s.elections)
    (e₂ : (e, h₂, A₂) ∈ s.elections) : h₁ = h₂ :=
  one_host cfg hm (inv_of_reachable cfg hm hr) e₁ e₂

/-- A host elects itself once per epoch, so an epoch has at most one adopted log. -/
theorem one_adoption_per_epoch (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s) {e : Nat}
    {h₁ h₂ : String} {A₁ A₂ : Log} (e₁ : (e, h₁, A₁) ∈ s.elections)
    (e₂ : (e, h₂, A₂) ∈ s.elections) : h₁ = h₂ ∧ (A₁ <+: A₂ ∨ A₂ <+: A₁) := by
  have hi := inv_of_reachable cfg hm hr
  have hh := one_host cfg hm hi e₁ e₂
  subst hh
  have w₁ := hi.ewlog _ _ _ e₁
  have w₂ := hi.ewlog _ _ _ e₂
  refine ⟨rfl, ?_⟩
  by_cases hl : A₁.length ≤ A₂.length
  · exact Or.inl (List.prefix_of_prefix_length_le w₁ w₂ hl)
  · exact Or.inr (List.prefix_of_prefix_length_le w₂ w₁ (by omega))

/-- A committed log is choosable in its epoch. -/
theorem committed_choosable {s : Sys} {L : Log} {e₀ : Nat} (hc : Committed cfg s L e₀) :
    Choosable cfg s L e₀ := by
  obtain ⟨Q, hQ, h⟩ := hc
  exact ⟨Q, hQ, fun c hc => Or.inl (h c hc)⟩

/-- **Committed entries survive elections.** A log committed in `e₀` is a prefix of the log
adopted by every host elected in a later epoch, whichever happened first. -/
-- @tg: implements:: [[openspec:object-custody#Quorum Before Effects]] {note: "Lean proof"}
-- @tg: verifies:: [[openspec:object-custody#Quorum Before Effects#Committed entry survives failover]] {level: unit}
theorem committed_survives_election (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s)
    {L : Log} {e₀ e : Nat} {h : String} {A : Log} (hc : Committed cfg s L e₀)
    (he : (e, h, A) ∈ s.elections) (hlt : e₀ < e) : L <+: A :=
  (inv_of_reachable cfg hm hr).esafe e h A he e₀ L hlt (committed_choosable cfg hc)

/-- **Committed entries survive in every later acceptance.** Every log any custodian accepts in
an epoch after `e₀` extends a log committed in `e₀`. -/
theorem committed_survives_accept (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s)
    {L : Log} {e₀ e : Nat} {c : String} {l : Log} (hc : Committed cfg s L e₀)
    (ha : (c, e, l) ∈ s.accepted) (hlt : e₀ < e) : L <+: l :=
  (inv_of_reachable cfg hm hr).asafe c e l ha e₀ L hlt (committed_choosable cfg hc)

/-- Every custodian whose log was accepted after `e₀` holds a log committed in `e₀`. -/
theorem committed_in_custodian (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s)
    {L : Log} {e₀ : Nat} (hc : Committed cfg s L e₀) (c : String)
    (hlt : e₀ < (s.cust c).acc) : L <+: (s.cust c).log := by
  rcases (inv_of_reachable cfg hm hr).cur c with ⟨h0, -⟩ | ha
  · omega
  · exact committed_survives_accept cfg hm hr hc ha hlt

/-- The writer of every later epoch holds a log committed in `e₀`. -/
theorem committed_in_writer_log (hm : cfg.Majority) {s : Sys} (hr : Reachable cfg s)
    {L : Log} {e₀ e : Nat} {h : String} {A : Log} (hc : Committed cfg s L e₀)
    (he : (e, h, A) ∈ s.elections) (hlt : e₀ < e) : L <+: s.wlog e h :=
  (committed_survives_election cfg hm hr hc he hlt).trans
    ((inv_of_reachable cfg hm hr).ewlog e h A he)

end Vertep.Custody
