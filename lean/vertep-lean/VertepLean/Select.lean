/-!
# Transition selection

W3C SCXML Appendix D `selectTransitions` / `selectEventlessTransitions` and
`removeConflictingTransitions`, written once over abstract parameters so that the step function
uses exactly the definitions the theorems are about:

* `atomics` — the atomic states of the configuration in document order;
* `cands a` — the transitions of `a`, then of its parent, and so on up to the root, each state's
  transitions in document order (Appendix D's `for s in [state].append(getProperAncestors(state,
  null))` / `for t in s.transition.sort(documentOrder)`);
* `p t` — whether `t` matches (event descriptor match, or eventless);
* a guard test, monadic in the step (it consumes the answer tape) and a valuation
  `g i k` (the answer for the `k`-th candidate of the `i`-th atomic state) in the pure model;
* `conflict t₁ t₂` — the exit sets of `t₁` and `t₂` intersect;
* `desc t₁ t₂` — the source of `t₁` is a proper descendant of the source of `t₂`.

`firstIdx` and `choices` are the monadic loops the step runs; `firstP` and `choicesP` are their
pure counterparts, and `choices_run` shows that every successful run of the monadic loops equals
the pure loops for some guard valuation, namely the answers the run observed.
-/

namespace Vertep.Select

/-! ## Monadic loops (used by `step`) -/

/-- The first candidate, from position `k` on, whose test is true. Tests are run in order and
stop at the first true one (Appendix D `break loop`). -/
def firstIdx {m : Type → Type} [Monad m] (test : Nat → Nat → m Bool) :
    List Nat → Nat → m (Option Nat)
  | [], _ => pure none
  | t :: ts, k => do
    let b ← test k t
    if b then pure (some t) else firstIdx test ts (k + 1)

/-- One choice per atomic state, in document order of the atomic states. -/
def choices {m : Type → Type} [Monad m] (cands : Nat → List Nat)
    (test : Nat → Nat → Nat → m Bool) : List Nat → Nat → m (List (Option Nat))
  | [], _ => pure []
  | a :: as, i => do
    let c ← firstIdx (test i) (cands a) 0
    let cs ← choices cands test as (i + 1)
    pure (c :: cs)

/-! ## Pure loops (used by the theorems) -/

/-- Pure `firstIdx`. -/
def firstP (test : Nat → Nat → Bool) : List Nat → Nat → Option Nat
  | [], _ => none
  | t :: ts, k => if test k t then some t else firstP test ts (k + 1)

/-- Pure `choices`. -/
def choicesP (cands : Nat → List Nat) (test : Nat → Nat → Nat → Bool) :
    List Nat → Nat → List (Option Nat)
  | [], _ => []
  | a :: as, i => firstP (test i) (cands a) 0 :: choicesP cands test as (i + 1)

/-- `OrderedSet.add`: append unless already present. -/
def addUniq (l : List Nat) (t : Nat) : List Nat := if t ∈ l then l else l ++ [t]

/-- The ordered set of chosen transitions (Appendix D `enabledTransitions`). -/
def enabledOf (cs : List (Option Nat)) : List Nat :=
  cs.foldl (fun acc c => match c with
    | some t => addUniq acc t
    | none => acc) []

/-- The inner loop of `removeConflictingTransitions` for a new transition `t₁` over the already
filtered transitions: `none` when `t₁` is preempted, otherwise the transitions it removes. -/
def scan (conflict desc : Nat → Nat → Bool) (t₁ : Nat) : List Nat → Option (List Nat)
  | [] => some []
  | t₂ :: rest =>
    if conflict t₁ t₂ then
      if desc t₁ t₂ then (scan conflict desc t₁ rest).map (t₂ :: ·) else none
    else scan conflict desc t₁ rest

/-- One iteration of the outer loop of `removeConflictingTransitions`. -/
def rcStep (conflict desc : Nat → Nat → Bool) (filtered : List Nat) (t₁ : Nat) : List Nat :=
  match scan conflict desc t₁ filtered with
  | none => filtered
  | some rm => filtered.filter (fun x => !(rm.contains x)) ++ [t₁]

/-- Appendix D `removeConflictingTransitions`. -/
def removeConflicting (conflict desc : Nat → Nat → Bool) (ts : List Nat) : List Nat :=
  ts.foldl (rcStep conflict desc) []

/-- The pure selection: the guard of the `k`-th candidate of the `i`-th atomic state answered
`g i k`. -/
def selectPure (atomics : List Nat) (cands : Nat → List Nat) (p : Nat → Bool)
    (g : Nat → Nat → Bool) (conflict desc : Nat → Nat → Bool) : List Nat :=
  removeConflicting conflict desc
    (enabledOf (choicesP cands (fun i k t => p t && g i k) atomics 0))

/-! ## The monadic loops agree with the pure loops -/

section Bridge

variable {σ ε : Type}

theorem firstP_congr (test test' : Nat → Nat → Bool) :
    ∀ (l : List Nat) (k : Nat), (∀ j t, k ≤ j → test j t = test' j t) →
      firstP test l k = firstP test' l k
  | [], _, _ => rfl
  | t :: ts, k, h => by
    simp only [firstP]
    rw [h k t (Nat.le_refl _), firstP_congr test test' ts (k + 1)
      (fun j t' hj => h j t' (Nat.le_of_succ_le hj))]

/-- A successful run of the monadic first-choice loop equals the pure loop for the guard answers
the run observed. -/
theorem firstIdx_run (p : Nat → Bool) (guard : Nat → Nat → StateT σ (Except ε) Bool) :
    ∀ (l : List Nat) (k : Nat) (s s' : σ) (r : Option Nat),
      (firstIdx (fun k t => if p t then guard k t else pure false) l k).run s = .ok (r, s') →
      ∃ g : Nat → Bool, r = firstP (fun k t => p t && g k) l k
  | [], k, s, s', r, h => by
    simp only [firstIdx, StateT.run_pure] at h
    cases h
    exact ⟨fun _ => false, rfl⟩
  | t :: ts, k, s, s', r, h => by
    simp only [firstIdx, StateT.run_bind] at h
    by_cases hp : p t = true
    · simp only [hp, ↓reduceIte] at h
      cases hg : (guard k t).run s with
      | error e => rw [hg] at h; cases h
      | ok v =>
        rcases v with ⟨b, s₁⟩
        rw [hg] at h
        cases b with
        | true =>
          simp only [bind, Except.bind, ↓reduceIte, pure] at h
          cases h
          exact ⟨fun _ => true, by simp [firstP, hp]⟩
        | false =>
          simp only [bind, Except.bind, Bool.false_eq_true, ↓reduceIte] at h
          obtain ⟨g, hg'⟩ := firstIdx_run p guard ts (k + 1) s₁ s' r h
          refine ⟨fun j => if j = k then false else g j, ?_⟩
          simp only [firstP, hp, Bool.true_and, Bool.false_eq_true, ↓reduceIte]
          rw [hg']
          apply firstP_congr
          intro j t' hj
          have : j ≠ k := by omega
          simp [this]
    · have hpf : p t = false := by simpa using hp
      simp only [hpf, Bool.false_eq_true, ↓reduceIte, StateT.run_pure] at h
      simp only [bind, pure, Except.pure, Except.bind] at h
      obtain ⟨g, hg'⟩ := firstIdx_run p guard ts (k + 1) s s' r h
      refine ⟨g, ?_⟩
      simp only [firstP, hpf, Bool.false_and, Bool.false_eq_true, ↓reduceIte]
      exact hg'

theorem choicesP_congr (cands : Nat → List Nat) (test test' : Nat → Nat → Nat → Bool) :
    ∀ (l : List Nat) (i : Nat), (∀ j k t, i ≤ j → test j k t = test' j k t) →
      choicesP cands test l i = choicesP cands test' l i
  | [], _, _ => rfl
  | a :: as, i, h => by
    simp only [choicesP]
    rw [firstP_congr (test i) (test' i) (cands a) 0 (fun j t _ => h i j t (Nat.le_refl _)),
      choicesP_congr cands test test' as (i + 1) (fun j k t hj => h j k t (Nat.le_of_succ_le hj))]

/-- A successful run of the monadic selection loop equals the pure loop for the guard answers
the run observed. -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem choices_run (cands : Nat → List Nat) (p : Nat → Bool)
    (guard : Nat → Nat → Nat → StateT σ (Except ε) Bool) :
    ∀ (l : List Nat) (i : Nat) (s s' : σ) (r : List (Option Nat)),
      (choices cands (fun i k t => if p t then guard i k t else pure false) l i).run s
          = .ok (r, s') →
      ∃ g : Nat → Nat → Bool, r = choicesP cands (fun i k t => p t && g i k) l i
  | [], i, s, s', r, h => by
    simp only [choices, StateT.run_pure] at h
    cases h
    exact ⟨fun _ _ => false, rfl⟩
  | a :: as, i, s, s', r, h => by
    simp only [choices, StateT.run_bind] at h
    cases h1 : (firstIdx (fun k t => if p t then guard i k t else pure false) (cands a) 0).run s with
    | error e => rw [h1] at h; cases h
    | ok v =>
      rcases v with ⟨c, s₁⟩
      rw [h1] at h
      simp only [bind, Except.bind] at h
      cases h2 : (choices cands (fun i k t => if p t then guard i k t else pure false) as
          (i + 1)).run s₁ with
      | error e => rw [h2] at h; cases h
      | ok w =>
        rcases w with ⟨cs, s₂⟩
        rw [h2] at h
        simp only [pure] at h
        cases h
        obtain ⟨g₁, hg₁⟩ := firstIdx_run p (guard i) (cands a) 0 s s₁ c h1
        obtain ⟨g₂, hg₂⟩ := choices_run cands p guard as (i + 1) s₁ _ cs h2
        refine ⟨fun j k => if j = i then g₁ k else g₂ j k, ?_⟩
        simp only [choicesP, ite_true]
        rw [hg₁, hg₂]
        congr 1
        apply choicesP_congr
        intro j k t hj
        have : j ≠ i := by omega
        simp [this]

end Bridge

/-! ## Properties of the pure loops -/

theorem firstP_some (test : Nat → Nat → Bool) :
    ∀ (l : List Nat) (k t : Nat), firstP test l k = some t →
      ∃ j, l[j]? = some t ∧ test (k + j) t = true ∧
        ∀ j' t', j' < j → l[j']? = some t' → test (k + j') t' = false
  | [], _, _, h => by simp [firstP] at h
  | x :: xs, k, t, h => by
    simp only [firstP] at h
    by_cases hx : test k x = true
    · simp only [hx, ↓reduceIte, Option.some.injEq] at h
      subst h
      exact ⟨0, rfl, by simpa using hx, fun j' _ hj _ => absurd hj (Nat.not_lt_zero _)⟩
    · have hx' : test k x = false := by simpa using hx
      simp only [hx', Bool.false_eq_true, ↓reduceIte] at h
      obtain ⟨j, hj, ht, hpre⟩ := firstP_some test xs (k + 1) t h
      refine ⟨j + 1, by simpa using hj, by rw [show k + (j + 1) = k + 1 + j by omega]; exact ht, ?_⟩
      intro j' t' hj' hl
      cases j' with
      | zero => simp at hl; subst hl; simpa using hx'
      | succ j'' =>
        rw [show k + (j'' + 1) = k + 1 + j'' by omega]
        exact hpre j'' t' (by omega) (by simpa using hl)

theorem firstP_none (test : Nat → Nat → Bool) :
    ∀ (l : List Nat) (k : Nat), firstP test l k = none →
      ∀ j t, l[j]? = some t → test (k + j) t = false
  | [], _, _, j, t, hl => by simp at hl
  | x :: xs, k, h, j, t, hl => by
    simp only [firstP] at h
    by_cases hx : test k x = true
    · simp [hx] at h
    · have hx' : test k x = false := by simpa using hx
      simp only [hx', Bool.false_eq_true, ↓reduceIte] at h
      cases j with
      | zero => simp at hl; subst hl; simpa using hx'
      | succ j' =>
        rw [show k + (j' + 1) = k + 1 + j' by omega]
        exact firstP_none test xs (k + 1) h j' t (by simpa using hl)

theorem choicesP_getElem? (cands : Nat → List Nat) (test : Nat → Nat → Nat → Bool) :
    ∀ (l : List Nat) (i n : Nat),
      (choicesP cands test l i)[n]? = l[n]?.map (fun a => firstP (test (i + n)) (cands a) 0)
  | [], _, n => by simp [choicesP]
  | a :: as, i, n => by
    cases n with
    | zero => simp [choicesP]
    | succ n =>
      simp only [choicesP, List.getElem?_cons_succ]
      rw [choicesP_getElem? cands test as (i + 1) n, show i + 1 + n = i + (n + 1) by omega]

theorem mem_addUniq (l : List Nat) (t x : Nat) : x ∈ addUniq l t ↔ x ∈ l ∨ x = t := by
  unfold addUniq
  by_cases h : t ∈ l
  · simp only [h, ↓reduceIte]
    constructor
    · exact Or.inl
    · rintro (h' | h')
      · exact h'
      · exact h' ▸ h
  · simp [h]

theorem nodup_addUniq (l : List Nat) (t : Nat) (h : l.Nodup) : (addUniq l t).Nodup := by
  unfold addUniq
  by_cases ht : t ∈ l
  · simp [ht, h]
  · simp only [ht, ↓reduceIte]
    rw [List.nodup_append]
    refine ⟨h, by simp, ?_⟩
    intro a ha b hb
    simp at hb
    subst hb
    intro hab
    exact ht (hab ▸ ha)

theorem enabledOf_aux :
    ∀ (cs : List (Option Nat)) (acc : List Nat), acc.Nodup →
      (cs.foldl (fun acc c => match c with
        | some t => addUniq acc t
        | none => acc) acc).Nodup ∧
      ∀ x, x ∈ cs.foldl (fun acc c => match c with
        | some t => addUniq acc t
        | none => acc) acc ↔ x ∈ acc ∨ some x ∈ cs
  | [], acc, h => ⟨h, fun x => by simp⟩
  | c :: cs, acc, h => by
    simp only [List.foldl_cons]
    cases c with
    | none =>
      obtain ⟨h1, h2⟩ := enabledOf_aux cs acc h
      refine ⟨h1, fun x => ?_⟩
      rw [h2]
      simp
    | some t =>
      obtain ⟨h1, h2⟩ := enabledOf_aux cs (addUniq acc t) (nodup_addUniq acc t h)
      refine ⟨h1, fun x => ?_⟩
      rw [h2, mem_addUniq]
      simp only [List.mem_cons, Option.some.injEq]
      constructor
      · rintro ((h | h) | h)
        · exact Or.inl h
        · exact Or.inr (Or.inl h)
        · exact Or.inr (Or.inr h)
      · rintro (h | h | h)
        · exact Or.inl (Or.inl h)
        · exact Or.inl (Or.inr h)
        · exact Or.inr h

theorem enabledOf_nodup (cs : List (Option Nat)) : (enabledOf cs).Nodup :=
  (enabledOf_aux cs [] List.nodup_nil).1

theorem mem_enabledOf (cs : List (Option Nat)) (x : Nat) : x ∈ enabledOf cs ↔ some x ∈ cs := by
  unfold enabledOf
  rw [(enabledOf_aux cs [] List.nodup_nil).2 x]
  simp

/-! ## `removeConflictingTransitions` -/

section Conflicts

variable (conflict desc : Nat → Nat → Bool)

theorem scan_sub (t₁ : Nat) :
    ∀ (l rm : List Nat), scan conflict desc t₁ l = some rm →
      ∀ x ∈ rm, x ∈ l ∧ conflict t₁ x = true ∧ desc t₁ x = true
  | [], rm, h => by simp only [scan, Option.some.injEq] at h; subst h; simp
  | t₂ :: rest, rm, h => by
    simp only [scan] at h
    by_cases hc : conflict t₁ t₂ = true
    · by_cases hd : desc t₁ t₂ = true
      · simp only [hc, hd, ↓reduceIte] at h
        cases hs : scan conflict desc t₁ rest with
        | none => rw [hs] at h; cases h
        | some rm' =>
          rw [hs] at h
          simp only [Option.map_some, Option.some.injEq] at h
          subst h
          intro x hx
          simp only [List.mem_cons] at hx
          rcases hx with hx | hx
          · subst hx; exact ⟨List.mem_cons_self .., hc, hd⟩
          · obtain ⟨a, b, c⟩ := scan_sub t₁ rest rm' hs x hx
            exact ⟨List.mem_cons_of_mem _ a, b, c⟩
      · simp [hc, hd] at h
    · have hc' : conflict t₁ t₂ = false := by simpa using hc
      simp only [hc', Bool.false_eq_true, ↓reduceIte] at h
      intro x hx
      obtain ⟨a, b, c⟩ := scan_sub t₁ rest rm h x hx
      exact ⟨List.mem_cons_of_mem _ a, b, c⟩

theorem scan_complete (t₁ : Nat) :
    ∀ (l rm : List Nat), scan conflict desc t₁ l = some rm →
      ∀ x ∈ l, conflict t₁ x = true → x ∈ rm
  | [], _, _, x, hx, _ => by simp at hx
  | t₂ :: rest, rm, h, x, hx, hcx => by
    simp only [scan] at h
    by_cases hc : conflict t₁ t₂ = true
    · by_cases hd : desc t₁ t₂ = true
      · simp only [hc, hd, ↓reduceIte] at h
        cases hs : scan conflict desc t₁ rest with
        | none => rw [hs] at h; cases h
        | some rm' =>
          rw [hs] at h
          simp only [Option.map_some, Option.some.injEq] at h
          subst h
          simp only [List.mem_cons] at hx ⊢
          rcases hx with hx | hx
          · exact Or.inl hx
          · exact Or.inr (scan_complete t₁ rest rm' hs x hx hcx)
      · simp [hc, hd] at h
    · have hc' : conflict t₁ t₂ = false := by simpa using hc
      simp only [hc', Bool.false_eq_true, ↓reduceIte] at h
      simp only [List.mem_cons] at hx
      rcases hx with hx | hx
      · subst hx; rw [hc'] at hcx; cases hcx
      · exact scan_complete t₁ rest rm h x hx hcx

theorem scan_none (t₁ : Nat) :
    ∀ (l : List Nat), scan conflict desc t₁ l = none →
      ∃ x ∈ l, conflict t₁ x = true ∧ desc t₁ x = false
  | [], h => by simp [scan] at h
  | t₂ :: rest, h => by
    simp only [scan] at h
    by_cases hc : conflict t₁ t₂ = true
    · by_cases hd : desc t₁ t₂ = true
      · simp only [hc, hd, ↓reduceIte] at h
        cases hs : scan conflict desc t₁ rest with
        | none =>
          obtain ⟨x, hx, h1, h2⟩ := scan_none t₁ rest hs
          exact ⟨x, List.mem_cons_of_mem _ hx, h1, h2⟩
        | some rm' => rw [hs] at h; cases h
      · exact ⟨t₂, List.mem_cons_self .., hc, by simpa using hd⟩
    · have hc' : conflict t₁ t₂ = false := by simpa using hc
      simp only [hc', Bool.false_eq_true, ↓reduceIte] at h
      obtain ⟨x, hx, h1, h2⟩ := scan_none t₁ rest h
      exact ⟨x, List.mem_cons_of_mem _ hx, h1, h2⟩

theorem transGen_head {α : Type} {r : α → α → Prop} {a b c : α} (h : r a b)
    (h' : Relation.TransGen r b c) : Relation.TransGen r a c := by
  induction h' with
  | single hbc => exact .tail (.single h) hbc
  | tail _ hcd ih => exact .tail ih hcd

/-- `a` comes before `b` in the list. -/
def Before (l : List Nat) (a b : Nat) : Prop :=
  ∃ l₁ l₂ l₃, l = l₁ ++ a :: (l₂ ++ b :: l₃)

/-- The SCXML 3.13 preemption between two selected transitions: their exit sets intersect and
the winner's source is a descendant of the loser's, or the winner was selected earlier. -/
def Beats (sel : List Nat) (w l : Nat) : Prop :=
  (conflict w l = true ∨ conflict l w = true) ∧ (desc w l = true ∨ Before sel w l)

/-- The invariant of the outer loop of `removeConflictingTransitions` after processing the
prefix `pre` of `sel = pre ++ post`. -/
structure RcInv (sel pre filtered : List Nat) : Prop where
  sub : ∀ x ∈ filtered, x ∈ pre
  nodup : filtered.Nodup
  pairwise : filtered.Pairwise (fun x y => conflict y x = false)
  covered : ∀ t ∈ pre, t ∈ filtered ∨
    ∃ t' ∈ filtered, Relation.TransGen (Beats conflict desc sel) t' t

theorem before_of_prefix (pre post : List Nat) (x t : Nat) (hx : x ∈ pre) :
    Before (pre ++ t :: post) x t := by
  obtain ⟨l₁, l₂, rfl⟩ := List.mem_iff_append.1 hx
  exact ⟨l₁, l₂, post, by simp⟩

theorem rcInv_step (sel pre post : List Nat) (t : Nat) (hsel : sel = pre ++ t :: post)
    (hnd : sel.Nodup) (filtered : List Nat) (inv : RcInv conflict desc sel pre filtered) :
    RcInv conflict desc sel (pre ++ [t]) (rcStep conflict desc filtered t) := by
  have htf : t ∉ filtered := by
    intro h
    have hp := inv.sub t h
    rw [hsel] at hnd
    rw [List.nodup_append] at hnd
    exact hnd.2.2 t hp t (List.mem_cons_self ..) rfl
  unfold rcStep
  cases hs : scan conflict desc t filtered with
  | none =>
    obtain ⟨x, hx, hc, _⟩ := scan_none conflict desc t filtered hs
    refine ⟨fun y hy => List.mem_append_left _ (inv.sub y hy), inv.nodup, inv.pairwise, ?_⟩
    intro y hy
    rcases List.mem_append.1 hy with hy | hy
    · exact inv.covered y hy
    · simp at hy
      subst hy
      refine Or.inr ⟨x, hx, Relation.TransGen.single ⟨Or.inr hc, Or.inr ?_⟩⟩
      rw [hsel]
      exact before_of_prefix pre post x y (inv.sub x hx)
  | some rm =>
    have hrm := scan_sub conflict desc t filtered rm hs
    have hcomp := scan_complete conflict desc t filtered rm hs
    refine ⟨?_, ?_, ?_, ?_⟩
    · intro y hy
      rcases List.mem_append.1 hy with hy | hy
      · exact List.mem_append_left _ (inv.sub y (List.mem_filter.1 hy).1)
      · simp at hy; subst hy; simp
    · rw [List.nodup_append]
      refine ⟨inv.nodup.sublist List.filter_sublist, by simp, ?_⟩
      intro a ha b hb hab
      simp at hb
      subst hb
      subst hab
      exact htf (List.mem_filter.1 ha).1
    · rw [List.pairwise_append]
      refine ⟨inv.pairwise.sublist List.filter_sublist, by simp, ?_⟩
      intro a ha b hb
      simp at hb
      subst hb
      have ha' := List.mem_filter.1 ha
      cases hc : conflict b a with
      | false => rfl
      | true =>
        have := hcomp a ha'.1 hc
        simp [this] at ha'
    · intro y hy
      rcases List.mem_append.1 hy with hy | hy
      · rcases inv.covered y hy with hyf | ⟨t', ht', hgen⟩
        · by_cases hyr : y ∈ rm
          · obtain ⟨_, hc, hd⟩ := hrm y hyr
            exact Or.inr ⟨t, by simp, Relation.TransGen.single ⟨Or.inl hc, Or.inl hd⟩⟩
          · exact Or.inl (List.mem_append_left _ (List.mem_filter.2 ⟨hyf, by simp [hyr]⟩))
        · by_cases htr : t' ∈ rm
          · obtain ⟨_, hc, hd⟩ := hrm t' htr
            refine Or.inr ⟨t, by simp, ?_⟩
            exact transGen_head ⟨Or.inl hc, Or.inl hd⟩ hgen
          · exact Or.inr ⟨t', List.mem_append_left _ (List.mem_filter.2 ⟨ht', by simp [htr]⟩), hgen⟩
      · simp at hy; subst hy; exact Or.inl (by simp)

theorem rcInv_foldl (sel : List Nat) (hnd : sel.Nodup) :
    ∀ (post pre : List Nat), sel = pre ++ post →
      ∀ filtered, RcInv conflict desc sel pre filtered →
        RcInv conflict desc sel sel (post.foldl (rcStep conflict desc) filtered)
  | [], pre, h, filtered, inv => by simp at h; subst h; simpa using inv
  | t :: post, pre, h, filtered, inv => by
    simp only [List.foldl_cons]
    apply rcInv_foldl sel hnd post (pre ++ [t]) (by simp [h])
    exact rcInv_step conflict desc sel pre post t h hnd filtered inv

theorem removeConflicting_inv (sel : List Nat) (hnd : sel.Nodup) :
    RcInv conflict desc sel sel (removeConflicting conflict desc sel) :=
  rcInv_foldl conflict desc sel hnd sel [] (by simp) []
    ⟨fun _ h => by simp at h, List.nodup_nil, List.Pairwise.nil, fun _ h => by simp at h⟩

end Conflicts

/-! ## The selection theorem -/

section Theorem

variable (atomics : List Nat) (cands : Nat → List Nat) (p : Nat → Bool)
  (g : Nat → Nat → Bool) (conflict desc : Nat → Nat → Bool)

/-- Transition `t` is enabled as the `k`-th candidate of the `i`-th atomic state `a` of the
configuration: it is on `a` or an ancestor, it matches, and its guard answered true. -/
def EnabledAt (i k t : Nat) : Prop :=
  ∃ a, atomics[i]? = some a ∧ (cands a)[k]? = some t ∧ p t = true ∧ g i k = true

/-- The selected list before conflict removal. -/
def enabledSet : List Nat := enabledOf (choicesP cands (fun i k t => p t && g i k) atomics 0)

/-- SCXML 3.13 priority between transitions: `w` precedes `l` among the candidates of one atomic
state (descendant priority, then document order), or `w` beats `l` in conflict removal. -/
def Preempts (w l : Nat) : Prop :=
  (∃ (i a kw kl : Nat), atomics[i]? = some a ∧ (cands a)[kw]? = some w ∧ (cands a)[kl]? = some l ∧
      kw < kl) ∨
    Beats conflict desc (enabledSet atomics cands p g) w l

theorem mem_enabledSet (t : Nat) :
    t ∈ enabledSet atomics cands p g ↔
      ∃ i a, atomics[i]? = some a ∧ firstP (fun k t => p t && g i k) (cands a) 0 = some t := by
  unfold enabledSet
  rw [mem_enabledOf, List.mem_iff_getElem?]
  constructor
  · rintro ⟨n, hn⟩
    rw [choicesP_getElem?] at hn
    cases ha : atomics[n]? with
    | none => rw [ha] at hn; cases hn
    | some a =>
      rw [ha] at hn
      simp only [Option.map_some, Option.some.injEq, Nat.zero_add] at hn
      exact ⟨n, a, ha, hn⟩
  · rintro ⟨i, a, ha, hf⟩
    refine ⟨i, ?_⟩
    rw [choicesP_getElem?, ha]
    simpa using hf

/-- **Enabled.** Every selected transition is the first enabled candidate of some atomic state
of the configuration. -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem selected_enabled (t : Nat)
    (ht : t ∈ selectPure atomics cands p g conflict desc) :
    ∃ i k, EnabledAt atomics cands p g i k t ∧
      ∀ a k' t', atomics[i]? = some a → k' < k → (cands a)[k']? = some t' →
        (p t' && g i k') = false := by
  have hnd := enabledOf_nodup (choicesP cands (fun i k t => p t && g i k) atomics 0)
  have inv := removeConflicting_inv conflict desc _ hnd
  have hmem := inv.sub t ht
  obtain ⟨i, a, ha, hf⟩ := (mem_enabledSet atomics cands p g t).1 hmem
  obtain ⟨k, hk, htest, hpre⟩ := firstP_some _ (cands a) 0 t hf
  simp only [Nat.zero_add, Bool.and_eq_true] at htest hpre
  refine ⟨i, k, ⟨a, ha, hk, htest.1, htest.2⟩, ?_⟩
  intro a' k' t' ha' hk' hl
  rw [ha] at ha'
  cases ha'
  exact hpre k' t' hk' hl

/-- **Conflict-free.** No two selected transitions have intersecting exit sets (with `conflict`
symmetric, as exit-set intersection is). -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem selected_conflictFree (hsym : ∀ x y, conflict x y = conflict y x) :
    ∀ x ∈ selectPure atomics cands p g conflict desc,
      ∀ y ∈ selectPure atomics cands p g conflict desc, x ≠ y → conflict x y = false := by
  have hnd := enabledOf_nodup (choicesP cands (fun i k t => p t && g i k) atomics 0)
  have inv := removeConflicting_inv conflict desc _ hnd
  intro x hx y hy hxy
  have hpw := inv.pairwise
  unfold selectPure at hx hy
  obtain ⟨l₁, l₂, hl⟩ := List.mem_iff_append.1 hx
  rw [hl] at hpw hy
  rw [List.pairwise_append] at hpw
  rcases List.mem_append.1 hy with hy | hy
  · exact hpw.2.2 y hy x (List.mem_cons_self ..)
  · simp only [List.mem_cons] at hy
    rcases hy with hy | hy
    · exact absurd hy.symm hxy
    · rw [List.pairwise_cons] at hpw
      rw [hsym]
      exact hpw.2.1.1 y hy

/-- **Selected transitions are distinct.** -/
theorem selected_nodup : (selectPure atomics cands p g conflict desc).Nodup :=
  (removeConflicting_inv conflict desc _
    (enabledOf_nodup (choicesP cands (fun i k t => p t && g i k) atomics 0))).nodup

/-- **Optimal.** Every enabled transition is selected, or a selected transition has priority over
it through a chain of preemptions (descendant priority and document order among the candidates
of one atomic state, then descendant priority and selection order in conflict removal). -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem selected_optimal (i k t : Nat) (he : EnabledAt atomics cands p g i k t) :
    t ∈ selectPure atomics cands p g conflict desc ∨
      ∃ t' ∈ selectPure atomics cands p g conflict desc,
        Relation.TransGen (Preempts atomics cands p g conflict desc) t' t := by
  obtain ⟨a, ha, hk, hp, hg⟩ := he
  have hnd := enabledOf_nodup (choicesP cands (fun i k t => p t && g i k) atomics 0)
  have inv := removeConflicting_inv conflict desc _ hnd
  -- the first enabled candidate of `a`
  cases hf : firstP (fun k t => p t && g i k) (cands a) 0 with
  | none =>
    have := firstP_none _ (cands a) 0 hf k t hk
    simp [hp, hg] at this
  | some t₀ =>
    have hmem : t₀ ∈ enabledSet atomics cands p g :=
      (mem_enabledSet atomics cands p g t₀).2 ⟨i, a, ha, hf⟩
    obtain ⟨k₀, hk₀, _, hpre⟩ := firstP_some _ (cands a) 0 t₀ hf
    have hk₀k : k₀ ≤ k := by
      apply Nat.le_of_not_lt
      intro hlt
      have := hpre k t hlt hk
      simp [hp, hg] at this
    -- lift `Beats` chains to `Preempts` chains
    have lift : ∀ x y, Relation.TransGen (Beats conflict desc (enabledSet atomics cands p g)) x y →
        Relation.TransGen (Preempts atomics cands p g conflict desc) x y := by
      intro x y h
      induction h with
      | single h => exact .single (Or.inr h)
      | tail _ h ih => exact .tail ih (Or.inr h)
    rcases Nat.lt_or_eq_of_le hk₀k with hlt | heq
    · have hpr : Preempts atomics cands p g conflict desc t₀ t :=
        Or.inl ⟨i, a, k₀, k, ha, hk₀, hk, hlt⟩
      rcases inv.covered t₀ hmem with h | ⟨t', ht', hgen⟩
      · exact Or.inr ⟨t₀, h, .single hpr⟩
      · exact Or.inr ⟨t', ht', .tail (lift _ _ hgen) hpr⟩
    · subst heq
      rw [hk] at hk₀
      cases hk₀
      rcases inv.covered _ hmem with h | ⟨t', ht', hgen⟩
      · exact Or.inl h
      · exact Or.inr ⟨t', ht', lift _ _ hgen⟩

end Theorem

end Vertep.Select
