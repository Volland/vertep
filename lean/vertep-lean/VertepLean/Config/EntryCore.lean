import VertepLean.Config.EntryFuel

/-!
# Entry lemma: invariants

The invariants of the entry-set computation for one transition with domain `d` (the `floor` of
`addAnc`), stated on the membership of `toEnter` only:

* `PC` — the states entered below `d` are pairwise equal, related or compatible (no compound
  state ever gets two touched children);
* `Cond` — a state entered below a root `G` is no history, its parent is `G` or entered, a
  compound state has an entered descendant, every child of a parallel state is touched;
* `DoneD`/`DoneA` — what a call of `addDesc`/`addAnc` has achieved, in the shape of the call,
  so that calling it again changes nothing (`idem`): a recorded history value may list the same
  state twice.
-/

namespace Vertep.Entry

open CChart

section Defs

variable (cc : CChart) (hist : List (Nat × List Nat)) (d : Nat)

/-- The states below `d` are pairwise related or compatible. -/
def PC (E : List Nat) : Prop :=
  ∀ a ∈ E, ∀ b ∈ E, cc.isDesc a d = true → cc.isDesc b d = true → cc.Rel a b

/-- `s` is related or compatible to every state below `d`. -/
def Fits (E : List Nat) (s : Nat) : Prop := ∀ w ∈ E, cc.isDesc w d = true → cc.Rel s w

/-- `c` or one of its descendants is entered. -/
def Touched (E : List Nat) (c : Nat) : Prop := ∃ z ∈ E, z = c ∨ cc.isDesc z c = true

/-- The completion conditions of a compound or parallel state. -/
def C34 (E : List Nat) (w : Nat) : Prop :=
  (cc.isCompound w = true → ∃ z ∈ E, cc.isDesc z w = true) ∧
  (cc.isParallel w = true → ∀ c ∈ (cc.st w).children, Touched cc E c)

/-- A state entered below the root `G` is complete. -/
def Cond (E : List Nat) (G w : Nat) : Prop :=
  cc.isHistory w = false ∧ (∀ q, (cc.st w).parent = some q → q = G ∨ q ∈ E) ∧ C34 cc E w

/-- The root `G` of entering below `D` with floor `d`: `D` itself when it lies in the floor's
subtree, or the floor when that lies below `D`. -/
def Hg (D G : Nat) : Prop :=
  (G = D ∧ (D = d ∨ cc.isDesc D d = true)) ∨ (G = d ∧ cc.isDesc d D = true)

/-- The targets of a state's initial transition. -/
def initT (s : Nat) : List Nat :=
  match (cc.st s).initial with
  | some i => (cc.tr i).targets
  | none => []

/-- The ancestors `addAnc` visits. -/
def ancL (y : Nat) (stop : Option Nat) : List Nat :=
  (cc.ancestorsBelow y stop).filter (fun a => a != 0 && cc.isDesc a d)

mutual
/-- What `addDesc` with fuel `n` on `s` has achieved in `E`. -/
def DoneD (E : List Nat) : Nat → Nat → Prop
  | 0, _ => False
  | n + 1, s =>
    if cc.isHistory s then
      match CChart.histValue hist s with
      | some v => ∀ x ∈ v, DoneD E n x ∧ DoneA E n x (some ((cc.st s).parent.getD 0))
      | none => match cc.historyDefault s with
        | some d0 => ∀ x ∈ (cc.tr d0).targets,
            DoneD E n x ∧ DoneA E n x (some ((cc.st s).parent.getD 0))
        | none => True
    else s ∈ E ∧ (cc.isCompound s = true → ∀ x ∈ initT cc s, DoneD E n x ∧ DoneA E n x (some s)) ∧
      (cc.isParallel s = true → ∀ c ∈ (cc.st s).children,
        (∃ z ∈ E, cc.isDesc z c = true) ∨ DoneD E n c)
/-- What `addAnc` with fuel `n` on `y` has achieved in `E`. -/
def DoneA (E : List Nat) : Nat → Nat → Option Nat → Prop
  | 0, _, _ => False
  | n + 1, y, stop => ∀ a ∈ ancL cc d y stop, a ∈ E ∧ (cc.isParallel a = true →
      ∀ c ∈ (cc.st a).children, (∃ z ∈ E, cc.isDesc z c = true) ∨ DoneD E n c)
end

/-- The effective targets one history state contributes. -/
def EffOfH (h e : Nat) : Prop :=
  (∃ v, CChart.histValue hist h = some v ∧ e ∈ v) ∨
  (CChart.histValue hist h = none ∧ ∃ d0, cc.historyDefault h = some d0 ∧ EffMem cc hist d0 e)

/-- The effective targets one target contributes. -/
def EffOf (x e : Nat) : Prop :=
  (cc.isHistory x = false ∧ e = x) ∨ (cc.isHistory x = true ∧ EffOfH cc hist x e)

end Defs

variable {cc : CChart} {hist : List (Nat × List Nat)} {d : Nat}

/-! ## Accumulator steps -/

/-- The two accumulators have the same states to enter. -/
def Same (a b : EntryAcc) : Prop := ∀ x, x ∈ b.toEnter ↔ x ∈ a.toEnter

theorem mem_add (acc : EntryAcc) (s x : Nat) :
    x ∈ (acc.add s).toEnter ↔ x ∈ acc.toEnter ∨ x = s := by
  unfold EntryAcc.add
  split
  · rename_i h
    constructor
    · exact Or.inl
    · rintro (h' | rfl)
      · exact h'
      · simpa using h
  · simp

theorem add_of_mem {acc : EntryAcc} {s : Nat} (h : s ∈ acc.toEnter) : Same acc (acc.add s) := by
  intro x; rw [mem_add]
  constructor
  · rintro (h' | rfl)
    · exact h'
    · exact h
  · exact Or.inl

theorem bind_some {α β : Type} {x : Option α} {f : α → Option β} {b : β}
    (h : (x >>= f) = some b) : ∃ a, x = some a ∧ f a = some b := by
  cases hx : x with
  | none => rw [hx] at h; cases h
  | some a => rw [hx] at h; exact ⟨a, rfl, h⟩

theorem foldlM_cons_some {α : Type} {f : EntryAcc → α → Option EntryAcc} {a : α} {l : List α}
    {init r : EntryAcc} (h : (a :: l).foldlM f init = some r) :
    ∃ b, f init a = some b ∧ l.foldlM f b = some r := by
  rw [List.foldlM_cons] at h
  exact bind_some h

theorem foldlM_nil_some {α : Type} {f : EntryAcc → α → Option EntryAcc}
    {init r : EntryAcc} (h : ([] : List α).foldlM f init = some r) : r = init := by
  simp at h; exact h.symm

theorem toEnter_ite (c : Prop) [Decidable c] (a : EntryAcc) (x : List Nat) :
    (if c then a else { toEnter := a.toEnter, defaultEntry := x, histContent := a.histContent }).toEnter
      = a.toEnter := by
  split <;> rfl

/-! ## Monotonicity -/

theorem touched_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {c : Nat} (ht : Touched cc E c) :
    Touched cc E' c := by
  obtain ⟨z, hz, h'⟩ := ht; exact ⟨z, h z hz, h'⟩

theorem c34_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {w : Nat} (hc : C34 cc E w) :
    C34 cc E' w := by
  refine ⟨fun hk => ?_, fun hk c hc' => touched_mono h (hc.2 hk c hc')⟩
  obtain ⟨z, hz, h'⟩ := hc.1 hk; exact ⟨z, h z hz, h'⟩

theorem cond_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {G w : Nat} (hc : Cond cc E G w) :
    Cond cc E' G w := by
  refine ⟨hc.1, fun q hq => ?_, c34_mono h hc.2.2⟩
  rcases hc.2.1 q hq with h' | h'
  · exact Or.inl h'
  · exact Or.inr (h q h')

theorem done_mono : ∀ n, (∀ (E E' : List Nat) s, (∀ x ∈ E, x ∈ E') → DoneD cc hist d E n s →
      DoneD cc hist d E' n s) ∧
    (∀ (E E' : List Nat) y stop, (∀ x ∈ E, x ∈ E') → DoneA cc hist d E n y stop →
      DoneA cc hist d E' n y stop) := by
  intro n
  induction n with
  | zero =>
    refine ⟨fun E E' s _ h => ?_, fun E E' y stop _ h => ?_⟩
    · simp [DoneD] at h
    · simp [DoneA] at h
  | succ n ih =>
    have par : ∀ (E E' : List Nat) (cs : List Nat), (∀ x ∈ E, x ∈ E') →
        (∀ c ∈ cs, (∃ z ∈ E, cc.isDesc z c = true) ∨ DoneD cc hist d E n c) →
        ∀ c ∈ cs, (∃ z ∈ E', cc.isDesc z c = true) ∨ DoneD cc hist d E' n c := by
      intro E E' cs hEE h c hc
      rcases h c hc with ⟨z, hz, h'⟩ | h'
      · exact Or.inl ⟨z, hEE z hz, h'⟩
      · exact Or.inr ((ih.1 E E' c hEE h'))
    refine ⟨fun E E' s hEE h => ?_, fun E E' y stop hEE h => ?_⟩
    · rw [DoneD] at h ⊢
      split
      · rename_i hh
        simp only [hh, ↓reduceIte] at h
        split
        · rename_i v hv
          simp only [hv] at h
          intro x hx
          exact ⟨ih.1 E E' x hEE (h x hx).1, ih.2 E E' x _ hEE (h x hx).2⟩
        · rename_i hv
          simp only [hv] at h
          split
          · rename_i d0 hd0
            simp only [hd0] at h
            intro x hx
            exact ⟨ih.1 E E' x hEE (h x hx).1, ih.2 E E' x _ hEE (h x hx).2⟩
          · trivial
      · rename_i hh
        simp only [hh, Bool.false_eq_true, ↓reduceIte] at h
        refine ⟨hEE s h.1, fun hk x hx => ?_, fun hk => par E E' _ hEE (h.2.2 hk)⟩
        exact ⟨ih.1 E E' x hEE (h.2.1 hk x hx).1, ih.2 E E' x _ hEE (h.2.1 hk x hx).2⟩
    · rw [DoneA] at h ⊢
      intro a ha
      exact ⟨hEE a (h a ha).1, fun hk => par E E' _ hEE ((h a ha).2 hk)⟩

theorem doneD_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {n s : Nat}
    (hd : DoneD cc hist d E n s) : DoneD cc hist d E' n s := (done_mono n).1 E E' s h hd

theorem doneA_mono {E E' : List Nat} (h : ∀ x ∈ E, x ∈ E') {n y : Nat} {stop : Option Nat}
    (hd : DoneA cc hist d E n y stop) : DoneA cc hist d E' n y stop :=
  (done_mono n).2 E E' y stop h hd

/-! ## Calling a call again changes nothing -/

theorem same_refl (a : EntryAcc) : Same a a := fun _ => Iff.rfl

theorem same_trans {a b c : EntryAcc} (h₁ : Same a b) (h₂ : Same b c) : Same a c :=
  fun x => (h₂ x).trans (h₁ x)

theorem same_sub {a b : EntryAcc} (h : Same a b) : ∀ x ∈ a.toEnter, x ∈ b.toEnter :=
  fun x hx => (h x).2 hx

theorem fold_same {α : Type} (f : EntryAcc → α → Option EntryAcc) (P : List Nat → α → Prop)
    (hP : ∀ (E E' : List Nat) x, (∀ z ∈ E, z ∈ E') → P E x → P E' x)
    (hf : ∀ acc acc' x, P acc.toEnter x → f acc x = some acc' → Same acc acc') :
    ∀ (l : List α) acc acc', (∀ x ∈ l, P acc.toEnter x) → l.foldlM f acc = some acc' →
      Same acc acc' := by
  intro l
  induction l with
  | nil => intro acc acc' _ h; rw [foldlM_nil_some h]; exact same_refl _
  | cons a l ih =>
    intro acc acc' hl h
    obtain ⟨b, hb, hr⟩ := foldlM_cons_some h
    have h1 := hf acc b a (hl a List.mem_cons_self) hb
    have h2 := ih b acc' (fun x hx => hP _ _ x (same_sub h1) (hl x (List.mem_cons_of_mem _ hx))) hr
    exact same_trans h1 h2

theorem idem : ∀ n, (∀ s (acc acc' : EntryAcc), DoneD cc hist d acc.toEnter n s →
      cc.addDesc hist d n s acc = some acc' → Same acc acc') ∧
    (∀ y stop (acc acc' : EntryAcc), DoneA cc hist d acc.toEnter n y stop →
      cc.addAnc hist d n y stop acc = some acc' → Same acc acc') := by
  intro n
  induction n with
  | zero =>
    refine ⟨fun s acc acc' h _ => ?_, fun y stop acc acc' h _ => ?_⟩
    · simp [DoneD] at h
    · simp [DoneA] at h
  | succ n ih =>
    have loop : ∀ (cs : List Nat) (acc acc' : EntryAcc),
        (∀ c ∈ cs, (∃ z ∈ acc.toEnter, cc.isDesc z c = true) ∨ DoneD cc hist d acc.toEnter n c) →
        cs.foldlM (fun acc c => if (acc.toEnter.any fun x => cc.isDesc x c) = true then pure acc
          else cc.addDesc hist d n c acc) acc = some acc' → Same acc acc' := by
      apply fold_same _ (fun E c => (∃ z ∈ E, cc.isDesc z c = true) ∨ DoneD cc hist d E n c)
      · intro E E' c hEE h
        rcases h with ⟨z, hz, h'⟩ | h'
        · exact Or.inl ⟨z, hEE z hz, h'⟩
        · exact Or.inr (doneD_mono hEE h')
      · intro acc acc' c h hc
        split at hc
        · simp at hc; rw [hc]; exact same_refl _
        · rename_i hany
          rcases h with ⟨z, hz, h'⟩ | h'
          · exact absurd (List.any_eq_true.2 ⟨z, hz, h'⟩) hany
          · exact ih.1 c acc acc' h' hc
    have foldD : ∀ (l : List Nat) (acc acc' : EntryAcc),
        (∀ x ∈ l, DoneD cc hist d acc.toEnter n x) →
        l.foldlM (fun acc x => cc.addDesc hist d n x acc) acc = some acc' → Same acc acc' :=
      fold_same _ (fun E x => DoneD cc hist d E n x) (fun _ _ _ h hd => doneD_mono h hd)
        (fun acc acc' x h hc => ih.1 x acc acc' h hc)
    have foldA : ∀ (stop : Option Nat) (l : List Nat) (acc acc' : EntryAcc),
        (∀ x ∈ l, DoneA cc hist d acc.toEnter n x stop) →
        l.foldlM (fun acc x => cc.addAnc hist d n x stop acc) acc = some acc' → Same acc acc' :=
      fun stop => fold_same _ (fun E x => DoneA cc hist d E n x stop)
        (fun _ _ _ h hd => doneA_mono h hd) (fun acc acc' x h hc => ih.2 x stop acc acc' h hc)
    have both : ∀ (stop : Option Nat) (l : List Nat) (acc acc' : EntryAcc),
        (∀ x ∈ l, DoneD cc hist d acc.toEnter n x ∧ DoneA cc hist d acc.toEnter n x stop) →
        (do let acc ← l.foldlM (fun acc x => cc.addDesc hist d n x acc) acc
            l.foldlM (fun acc x => cc.addAnc hist d n x stop acc) acc) = some acc' →
        Same acc acc' := by
      intro stop l acc acc' h hc
      obtain ⟨b, hb, hr⟩ := bind_some hc
      have h1 := foldD l acc b (fun x hx => (h x hx).1) hb
      exact same_trans h1 (foldA stop l b acc' (fun x hx => doneA_mono (same_sub h1) (h x hx).2) hr)
    refine ⟨fun s acc acc' h hc => ?_, fun y stop acc acc' h hc => ?_⟩
    · rw [DoneD] at h
      rw [CChart.addDesc] at hc
      split at h
      · rename_i hh
        simp only [hh, ↓reduceIte] at hc
        split at h
        · rename_i v hv
          simp only [hv] at hc
          exact both _ v acc acc' h hc
        · rename_i hv
          simp only [hv] at hc
          split at h
          · rename_i d0 hd0
            simp only [hd0] at hc
            have := both _ _ _ acc' ?_ hc
            · exact this
            · exact h
          · rename_i hd0
            simp only [hd0] at hc
            simp at hc; rw [hc]; exact same_refl _
      · rename_i hh
        simp only [hh, Bool.false_eq_true, ↓reduceIte] at hc
        obtain ⟨hs, hcomp, hpar⟩ := h
        have hsame := add_of_mem (acc := acc) hs
        split at hc
        · rename_i hk
          have hi := hcomp hk
          unfold initT at hi
          split at hi
          · rename_i i hi'
            simp only [hi'] at hc
            refine same_trans hsame (same_trans ?_ (both _ _ _ acc' (fun x hx => ?_) hc))
            · intro x; rw [toEnter_ite]
            · rw [toEnter_ite]
              exact ⟨doneD_mono (same_sub hsame) (hi x hx).1,
                doneA_mono (same_sub hsame) (hi x hx).2⟩
          · rename_i hi'
            simp only [hi'] at hc
            simp at hc
            rw [← hc]; intro x; rw [toEnter_ite]; exact hsame x
        · split at hc
          · rename_i _ hk
            refine same_trans hsame (loop _ _ acc' (fun c hc' => ?_) hc)
            rcases hpar hk c hc' with ⟨z, hz, h'⟩ | h'
            · exact Or.inl ⟨z, same_sub hsame z hz, h'⟩
            · exact Or.inr (doneD_mono (same_sub hsame) h')
          · simp at hc; rw [← hc]; exact hsame
    · rw [DoneA] at h
      rw [CChart.addAnc] at hc
      refine fold_same _ (fun E a => a ∈ E ∧ (cc.isParallel a = true →
          ∀ c ∈ (cc.st a).children, (∃ z ∈ E, cc.isDesc z c = true) ∨ DoneD cc hist d E n c))
        ?_ ?_ _ acc acc' h hc
      · intro E E' a hEE ⟨ha, hp⟩
        refine ⟨hEE a ha, fun hk c hc' => ?_⟩
        rcases hp hk c hc' with ⟨z, hz, h'⟩ | h'
        · exact Or.inl ⟨z, hEE z hz, h'⟩
        · exact Or.inr (doneD_mono hEE h')
      · intro acc acc' a ⟨ha, hp⟩ hc
        have hsame := add_of_mem (acc := acc) ha
        simp only at hc
        split at hc
        · rename_i hk
          refine same_trans hsame (loop _ _ acc' (fun c hc' => ?_) hc)
          rcases hp hk c hc' with ⟨z, hz, h'⟩ | h'
          · exact Or.inl ⟨z, same_sub hsame z hz, h'⟩
          · exact Or.inr (doneD_mono (same_sub hsame) h')
        · simp at hc; rw [← hc]; exact hsame

end Vertep.Entry
