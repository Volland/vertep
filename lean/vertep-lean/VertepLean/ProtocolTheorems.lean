import VertepLean.Protocol

/-!
# Kolyada protocol theorems (m10 8.2)

The three properties of the *Lean Protocol Model* requirement, for the model of
`VertepLean.Protocol`:

* `accept_enabled`, `accepted_enabled_by_projection` and `log_enabled`: **an accepted message
  is enabled in the role's projection.** A protocol message is accepted only when some state the
  participant may be in has a transition by which the message's role sends that type, to
  recipients that include its receivers; that transition is a step of the participant's
  projection. In every reachable state, every protocol message in the history was enabled in the
  knowledge it was judged in.
* `unbound_refused` and `log_bound`: **a role not bound in the instance is never accepted.** A
  message whose sender is not the DID the genesis bound to its role is refused, whatever else it
  says, and in every reachable state every applied message, the genesis included, came from the
  DID bound to its role.
* `causal_orders_converge` and `converged_accept`: **causally consistent orders converge.** Two
  runs that take the same inputs, in orders that put the protocol steps in the same order,
  reach the same state, and that state judges every later message alike. A run takes a message
  only after its dependencies (`run_causal`), so both orders are causally consistent; in a
  hash-linked history every protocol step depends on the one before it, so causal consistency
  fixes their order, and what differs between orders is where the joins, the violation notices
  and the completion fall. Those commute, because every part of the state but the knowledge
  only grows and the knowledge moves only on protocol steps (`applyAll_knowledge`).

The hypothesis on the protocol steps is needed: a participant that sees two concurrent protocol
messages, which a dishonest silent role can cause, may reach different knowledge in the two
orders.
-/

namespace Vertep.Kolyada

/-! ## The check, unfolded -/

theorem isControl_false {k : String} (h : isControl k = false) :
    k ≠ genesisKind ∧ k ≠ joinKind ∧ k ≠ declineKind ∧ k ≠ violationKind ∧ k ≠ completeKind := by
  simp only [isControl, Bool.or_eq_false_iff, beq_eq_false_iff_ne, ne_eq] at h
  exact ⟨h.1.1.1.1, h.1.1.1.2, h.1.1.2, h.1.2, h.2⟩

/-- What an accepted message satisfies: the checks every message passes. -/
theorem accept_spec {p : Protocol} {s : St} {m : Msg} (h : accept p s m = .accepted) :
    m.sigOk = true ∧ m.hash ∉ s.applied ∧ s.bindings.lookup m.role = some m.sender ∧
      (∀ d ∈ m.deps, d ∈ s.applied) ∧ s.lastSeq m.role < m.seq ∧ judgeKind p s m = .accepted := by
  unfold accept judge at h
  split at h
  · rename_i hs
    split at h; · cases h
    rename_i h1
    split at h; · cases h
    split at h; · cases h
    rename_i h3
    split at h; · cases h
    split at h; · cases h
    rename_i h5
    split at h; · cases h
    rename_i h6
    split at h; · cases h
    exact ⟨hs, h1, Classical.not_not.mp h3, Classical.not_not.mp h5, Nat.lt_of_not_le h6, h⟩
  · cases h

/-- **Unbound roles are refused.** A message whose sender is not the DID bound to its role is
never accepted. -/
-- @tg: implements:: [[openspec:protocol-conformance#Lean Protocol Model]]
theorem unbound_refused {p : Protocol} {s : St} {m : Msg}
    (h : s.bindings.lookup m.role ≠ some m.sender) : accept p s m ≠ .accepted :=
  fun ha => h (accept_spec ha).2.2.1

/-- A protocol message accepted by `judgeKind` is accepted by `judgeProtocol`. -/
theorem judgeKind_protocol {p : Protocol} {s : St} {m : Msg} (hk : isControl m.kind = false)
    (h : judgeKind p s m = .accepted) : judgeProtocol p s m = .accepted := by
  obtain ⟨g, j, d, v, c⟩ := isControl_false hk
  unfold judgeKind at h
  simp only [g, j, d, v, c, or_self, ite_false] at h
  exact h

/-- **An accepted protocol message is enabled.** Some state the participant may be in has a
transition by which the message's role sends its type, to recipients including its receivers. -/
theorem accept_enabled {p : Protocol} {s : St} {m : Msg} (h : accept p s m = .accepted)
    (hk : isControl m.kind = false) :
    ∃ t ∈ p.transitions, t.src ∈ s.knowledge ∧ t.sender = m.role ∧ t.message = m.kind ∧
      ∀ r ∈ t.receivers, r ∈ m.recipients := by
  have hp := judgeKind_protocol hk (accept_spec h).2.2.2.2.2
  unfold judgeProtocol at hp
  split at hp; · cases hp
  split at hp; · cases hp
  split at hp
  · split at hp <;> cases hp
  · rename_i t rest hm
    have ht : t ∈ matching p s.knowledge m.role m.kind := hm ▸ List.mem_cons_self
    simp only [matching, List.mem_filter, decide_eq_true_eq] at ht
    split at hp; · cases hp
    rename_i h3
    exact ⟨t, ht.1, ht.2.1, ht.2.2.2, ht.2.2.1, Classical.not_not.mp h3⟩

/-! ## Invariants of reachable states -/

/-- The bindings and the local role never change, and the local role is a role of the
protocol bound to the participant. -/
theorem reachable_me {p : Protocol} {s : St} (hr : Reachable p s) :
    s.me ∈ p.roles ∧ ∀ b ∈ s.bindings, b.1 ∈ p.roles := by
  induction hr with
  | @init b d g s h =>
    unfold init at h
    split at h
    · rename_i hc
      split at h
      · cases h
      · rename_i me d' hf
        cases h
        exact ⟨hc.2.2.2.2.2.1 _ (List.mem_of_find?_eq_some hf), hc.2.2.2.2.2.1⟩
    · cases h
  | apply _ _ ih => exact ih
  | hide _ _ ih => exact ih

/-- **Accepted messages are enabled by the projection.** In a reachable state, an accepted
protocol message is enabled by a step of the participant's projection from a state it may be
in. -/
-- @tg: implements:: [[openspec:protocol-conformance#Lean Protocol Model]]
theorem accepted_enabled_by_projection {p : Protocol} {s : St} {m : Msg} (hr : Reachable p s)
    (h : accept p s m = .accepted) (hk : isControl m.kind = false) :
    ∃ steps, project p s.me = some steps ∧ ∃ t ∈ p.transitions, stepOf s.me t ∈ steps ∧
      t.src ∈ s.knowledge ∧ t.sender = m.role ∧ t.message = m.kind ∧
      ∀ r ∈ t.receivers, r ∈ m.recipients := by
  obtain ⟨t, ht, h1, h2, h3, h4⟩ := accept_enabled h hk
  refine ⟨_, ite_eq_left_of_eq_true _ _ (eq_true (reachable_me hr).1), t, ht, List.mem_map_of_mem ht, h1, h2, h3, h4⟩

/-- **The history is enabled.** In every reachable state, every protocol message in the history
was enabled by a step of the participant's projection in the knowledge it was judged in. -/
theorem log_enabled {p : Protocol} {s : St} (hr : Reachable p s) :
    ∀ e ∈ s.log, isControl e.1.kind = false →
      ∃ steps, project p s.me = some steps ∧ ∃ t ∈ p.transitions, stepOf s.me t ∈ steps ∧
        t.src ∈ e.2 ∧ t.sender = e.1.role ∧ t.message = e.1.kind := by
  induction hr with
  | @init b d g s h =>
    intro e he hk
    unfold init at h
    split at h
    · rename_i hc
      split at h
      · cases h
      · cases h
        simp only [List.mem_singleton] at he
        subst he
        simp only [hc.2.1, isControl, genesisKind, BEq.rfl, Bool.true_or, reduceCtorEq] at hk
    · cases h
  | @apply s m hs ha ih =>
    intro e he hk
    simp only [apply, List.mem_append, List.mem_singleton] at he
    rcases he with he | rfl
    · exact ih e he hk
    · obtain ⟨steps, hst, t, ht, h1, h2, h3, h4, _⟩ := accepted_enabled_by_projection hs ha hk
      exact ⟨steps, hst, t, ht, h1, h2, h3, h4⟩
  | hide _ _ ih => exact ih

/-- **Unbound roles are never accepted.** In every reachable state, every applied message, the
genesis included, came from the DID bound to its role. -/
-- @tg: implements:: [[openspec:protocol-conformance#Lean Protocol Model]]
theorem log_bound {p : Protocol} {s : St} (hr : Reachable p s) :
    ∀ e ∈ s.log, s.bindings.lookup e.1.role = some e.1.sender := by
  induction hr with
  | @init b d g s h =>
    intro e he
    unfold init at h
    split at h
    · rename_i hc
      split at h
      · cases h
      · cases h
        simp only [List.mem_singleton] at he
        subst he
        exact hc.2.2.2.2.2.2.1
    · cases h
  | @apply s m _ ha ih =>
    intro e he
    simp only [apply, List.mem_append, List.mem_singleton] at he
    rcases he with he | rfl
    · exact ih e he
    · exact (accept_spec ha).2.2.1
  | hide _ _ ih => exact ih

/-! ## Convergence -/

/-- Apply an input without checking it. -/
def applyIn (p : Protocol) (s : St) : Input → St
  | .msg m => apply p s m
  | .hidden h => hide p s h

/-- Apply inputs in order without checking them. -/
def applyAll (p : Protocol) (s : St) (l : List Input) : St := l.foldl (applyIn p) s

def Input.hash : Input → Hash
  | .msg m => m.hash
  | .hidden h => h

def Input.depsOf : Input → List Hash
  | .msg m => m.deps
  | .hidden _ => []

def Input.seqsOf : Input → List (Role × Nat)
  | .msg m => [(m.role, m.seq)]
  | .hidden _ => []

def Input.seenOf : Input → List (Role × Nat × Hash)
  | .msg m => [(m.role, m.seq, m.hash)]
  | .hidden _ => []

def Input.joinsOf (b : List (Role × Did)) : Input → List Role
  | .msg m => if m.kind = joinKind then rolesOf b m.sender else []
  | .hidden _ => []

def Input.declines : Input → Bool
  | .msg m => decide (m.kind = declineKind)
  | .hidden _ => false

def Input.completes : Input → Bool
  | .msg m => decide (m.kind = completeKind)
  | .hidden _ => false

theorem run_eq {p : Protocol} : ∀ {s t : St} {l : List Input}, run p s l = some t → t = applyAll p s l
  | s, t, [], h => by simp only [run, Option.some.injEq] at h; exact h.symm
  | s, t, i :: l, h => by
    simp only [run] at h
    cases hi : step p s i with
    | none => rw [hi] at h; cases h
    | some s' =>
      rw [hi] at h
      simp only at h
      have : s' = applyIn p s i := by
        cases i with
        | msg m =>
          simp only [step] at hi
          split at hi
          · cases hi; rfl
          · cases hi
        | hidden h' =>
          simp only [step] at hi
          split at hi
          · cases hi
          · cases hi; rfl
      subst this
      exact (run_eq h : t = applyAll p (applyIn p s i) l)

/-- The fields of a state after unchecked inputs: what only grows has grown by the inputs, and
the knowledge has moved by each input in order. -/
theorem applyAll_fields (p : Protocol) : ∀ (s : St) (l : List Input),
    let t := applyAll p s l
    t.id = s.id ∧ t.bindings = s.bindings ∧ t.me = s.me ∧
    t.applied = (l.map Input.hash).reverse ++ s.applied ∧
    t.deps = l.reverse.flatMap Input.depsOf ++ s.deps ∧
    t.seqs = l.reverse.flatMap Input.seqsOf ++ s.seqs ∧
    t.seen = l.reverse.flatMap Input.seenOf ++ s.seen ∧
    t.joined = l.reverse.flatMap (Input.joinsOf s.bindings) ++ s.joined ∧
    t.declined = (s.declined || l.any Input.declines) ∧
    t.completed = (s.completed || l.any Input.completes) ∧
    t.knowledge = l.foldl (kstep p s.me) s.knowledge
  | s, [] => by simp [applyAll]
  | s, i :: l => by
    have ih := applyAll_fields p (applyIn p s i) l
    simp only [applyAll, List.foldl_cons] at ih ⊢
    obtain ⟨h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11⟩ := ih
    refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
    · rw [h1]; cases i <;> rfl
    · rw [h2]; cases i <;> rfl
    · rw [h3]; cases i <;> rfl
    · rw [h4]; cases i <;> simp [applyIn, apply, hide, Input.hash]
    · rw [h5]; cases i <;> simp [applyIn, apply, hide, Input.depsOf]
    · rw [h6]; cases i <;> simp [applyIn, apply, hide, Input.seqsOf]
    · rw [h7]; cases i <;> simp [applyIn, apply, hide, Input.seenOf]
    · rw [h8]
      cases i with
      | msg m =>
        simp only [applyIn, apply, List.reverse_cons, List.flatMap_append, List.flatMap_cons,
          List.flatMap_nil, List.append_nil, Input.joinsOf, List.append_assoc]
        split <;> simp
      | hidden _ => simp [applyIn, hide, Input.joinsOf]
    · rw [h9]; cases i <;> simp [applyIn, apply, hide, Input.declines, Bool.or_assoc]
    · rw [h10]; cases i <;> simp [applyIn, apply, hide, Input.completes, Bool.or_assoc]
    · rw [h11]; cases i <;> rfl

/-- Inputs that do not move the choreography leave the knowledge as it is. -/
theorem foldl_kstep_filter (p : Protocol) (me : Role) :
    ∀ (l : List Input) (K : List StateId),
      l.foldl (kstep p me) K = (l.filter Input.advances).foldl (kstep p me) K
  | [], K => rfl
  | i :: l, K => by
    cases ha : i.advances with
    | true =>
      simp only [List.foldl_cons, List.filter_cons, ha, ite_true]
      exact foldl_kstep_filter p me l _
    | false =>
      simp only [List.foldl_cons, List.filter_cons, ha, Bool.false_eq_true, ite_false]
      have : kstep p me K i = K := by
        cases i with
        | msg m =>
          simp only [Input.advances, Bool.not_eq_false'] at ha
          simp [kstep, ha]
        | hidden _ => simp [Input.advances] at ha
      rw [this]
      exact foldl_kstep_filter p me l K

/-- **The knowledge moves only on protocol steps.** -/
theorem applyAll_knowledge (p : Protocol) (s : St) (l : List Input) :
    (applyAll p s l).knowledge = (l.filter Input.advances).foldl (kstep p s.me) s.knowledge := by
  rw [(applyAll_fields p s l).2.2.2.2.2.2.2.2.2.2, foldl_kstep_filter]

/-- Two states that agree on everything a later check reads: the same instance, bindings and
role, the same applied hashes, dependencies, sequence numbers, joins, decline and completion,
and the same knowledge. Only the order of the history differs. -/
def St.Same (a b : St) : Prop :=
  a.id = b.id ∧ a.bindings = b.bindings ∧ a.me = b.me ∧
  (∀ x, x ∈ a.applied ↔ x ∈ b.applied) ∧ (∀ x, x ∈ a.deps ↔ x ∈ b.deps) ∧
  (∀ x, x ∈ a.seqs ↔ x ∈ b.seqs) ∧ (∀ x, x ∈ a.seen ↔ x ∈ b.seen) ∧
  (∀ x, x ∈ a.joined ↔ x ∈ b.joined) ∧ a.declined = b.declined ∧ a.completed = b.completed ∧
  a.knowledge = b.knowledge

/-- A run takes a message only after every message it depends on. -/
theorem run_causal {p : Protocol} {s t : St} {m : Msg} :
    ∀ {a b : List Input}, run p s (a ++ .msg m :: b) = some t →
      ∀ d ∈ m.deps, d ∈ (applyAll p s a).applied
  | [], b, h => by
    simp only [List.nil_append, run, step] at h
    by_cases ha : accept p s m = .accepted
    · exact (accept_spec ha).2.2.2.1
    · simp [ha] at h
  | i :: a, b, h => by
    simp only [List.cons_append, run] at h
    cases hi : step p s i with
    | none => rw [hi] at h; cases h
    | some s' =>
      rw [hi] at h
      simp only at h
      have := run_causal (s := s') h
      have hs' : s' = applyIn p s i := run_eq (l := [i]) (by simp [run, hi])
      simpa [applyAll, hs'] using this

/-- **Causally consistent orders converge.** Two runs from one state that take the same inputs,
with the protocol steps in the same order, reach the same state. -/
-- @tg: implements:: [[openspec:protocol-conformance#Lean Protocol Model]]
theorem causal_orders_converge {p : Protocol} {s t₁ t₂ : St} {l₁ l₂ : List Input}
    (hp : l₁.Perm l₂) (ho : l₁.filter Input.advances = l₂.filter Input.advances)
    (h₁ : run p s l₁ = some t₁) (h₂ : run p s l₂ = some t₂) : t₁.Same t₂ := by
  rw [run_eq h₁, run_eq h₂]
  obtain ⟨a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, _⟩ := applyAll_fields p s l₁
  obtain ⟨b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, _⟩ := applyAll_fields p s l₂
  have hk : (applyAll p s l₁).knowledge = (applyAll p s l₂).knowledge := by
    rw [applyAll_knowledge, applyAll_knowledge, ho]
  refine ⟨a1.trans b1.symm, a2.trans b2.symm, a3.trans b3.symm, ?_, ?_, ?_, ?_, ?_, ?_, ?_, hk⟩
  · intro x; rw [a4, b4]; simp only [List.mem_append, List.mem_reverse, (hp.map _).mem_iff]
  · intro x; rw [a5, b5]; simp only [List.mem_append, List.mem_flatMap, List.mem_reverse, hp.mem_iff]
  · intro x; rw [a6, b6]; simp only [List.mem_append, List.mem_flatMap, List.mem_reverse, hp.mem_iff]
  · intro x; rw [a7, b7]; simp only [List.mem_append, List.mem_flatMap, List.mem_reverse, hp.mem_iff]
  · intro x; rw [a8, b8]; simp only [List.mem_append, List.mem_flatMap, List.mem_reverse, hp.mem_iff]
  · rw [a9, b9, hp.any_eq]
  · rw [a10, b10, hp.any_eq]

/-! ## Converged states judge alike -/

/-- The highest number paired with `r`. -/
theorem lastSeq_le {L : List (Role × Nat)} {r : Role} :
    ∀ x ∈ L, x.1 = r → x.2 ≤ L.foldr (fun x acc => if x.1 = r then max x.2 acc else acc) 0 := by
  induction L with
  | nil => intro x hx; cases hx
  | cons y L ih =>
    intro x hx hr
    simp only [List.foldr_cons]
    rcases List.mem_cons.mp hx with rfl | hx
    · simp only [hr, ite_true]; exact Nat.le_max_left _ _
    · have := ih x hx hr
      split
      · exact Nat.le_trans this (Nat.le_max_right _ _)
      · exact this

theorem lastSeq_mem {L : List (Role × Nat)} {r : Role} :
    L.foldr (fun x acc => if x.1 = r then max x.2 acc else acc) 0 = 0 ∨
      (r, L.foldr (fun x acc => if x.1 = r then max x.2 acc else acc) 0) ∈ L := by
  induction L with
  | nil => left; rfl
  | cons y L ih =>
    simp only [List.foldr_cons]
    split
    · rename_i hy
      by_cases hm : y.2 ≤ L.foldr (fun x acc => if x.1 = r then max x.2 acc else acc) 0
      · rw [Nat.max_eq_right hm]
        rcases ih with h | h
        · left; exact h
        · right; exact List.mem_cons_of_mem _ h
      · rw [Nat.max_eq_left (Nat.le_of_lt (Nat.lt_of_not_le hm))]
        right; rw [← hy]; exact List.mem_cons_self
    · rcases ih with h | h
      · left; exact h
      · right; exact List.mem_cons_of_mem _ h

/-- The last sequence number of a role depends only on the pairs applied, not their order. -/
theorem lastSeq_congr {a b : St} (h : ∀ x, x ∈ a.seqs ↔ x ∈ b.seqs) (r : Role) :
    a.lastSeq r = b.lastSeq r := by
  unfold St.lastSeq
  apply Nat.le_antisymm
  · rcases lastSeq_mem (L := a.seqs) (r := r) with h0 | hm
    · rw [h0]; exact Nat.zero_le _
    · exact lastSeq_le _ ((h _).mp hm) rfl
  · rcases lastSeq_mem (L := b.seqs) (r := r) with h0 | hm
    · rw [h0]; exact Nat.zero_le _
    · exact lastSeq_le _ ((h _).mpr hm) rfl

theorem St.Same.status {p : Protocol} {a b : St} (h : a.Same b) : a.status p = b.status p := by
  obtain ⟨_, _, _, _, _, _, _, hj, hd, hc, _⟩ := h
  have hall : a.AllJoined p ↔ b.AllJoined p := by
    simp only [St.AllJoined, hj]
  unfold St.status
  rw [hd, hc]
  by_cases ha : a.AllJoined p
  · simp [ha, hall.mp ha]
  · simp [ha, mt hall.mpr ha]

theorem St.Same.heads {a b : St} (h : a.Same b) (x : Hash) : x ∈ a.heads ↔ x ∈ b.heads := by
  obtain ⟨_, _, _, ha, hd, _⟩ := h
  simp only [St.heads, List.mem_filter, decide_eq_true_eq, ha, hd]

/-- **Converged states judge alike.** Two states that are the same up to the order of the
history give every later message the same verdict. -/
theorem converged_accept {p : Protocol} {a b : St} (h : a.Same b) (m : Msg) :
    accept p a m = accept p b m := by
  have hst := h.status (p := p)
  obtain ⟨hi, hb, _, ha, _, hs, he, hj, _, _, hk⟩ := h
  have hl := lastSeq_congr hs m.role
  have heq : a.Equivocates m.role m.seq m.hash ↔ b.Equivocates m.role m.seq m.hash := by
    simp only [St.Equivocates, he]
  have hc : a.Complete p ↔ b.Complete p := by simp only [St.Complete, hk]
  simp only [accept, judge, judgeKind, judgeProtocol, hi, hb, hl, ha, hj, hst, hk,
    propext heq, propext hc]

end Vertep.Kolyada
