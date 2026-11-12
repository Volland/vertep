import VertepLean.Config.Tree

/-!
# Hoare triples for the interpreter monad

`Hoare P x Q`: every successful run of the interpreter action `x` from a core satisfying `P`
ends in a core satisfying `Q` (for the returned value). Failed runs (`Except.error`) are not
constrained: the theorems are about successful `start` and `step`. Rules for the monad
operations, `liftE`, conditionals and loops.
-/

namespace Vertep.Config

variable {cc : CChart}

/-- Partial-correctness triple over the interpreter state `Core`. -/
def Hoare {α : Type} (P : Core → Prop) (x : M cc α) (Q : α → Core → Prop) : Prop :=
  ∀ core ex a core' ex', P core → ((x.run core).run ex) = Except.ok ((a, core'), ex') → Q a core'

theorem run_bind {α β : Type} (x : M cc α) (f : α → M cc β) (core : Core) (ex : ExecSt cc) :
    (((x >>= f).run core).run ex) =
      (match ((x.run core).run ex) with
      | .ok ((a, c), e) => (((f a).run c).run e)
      | .error err => .error err : Except StepError ((β × Core) × ExecSt cc)) := by
  simp only [StateT.run]
  show Except.bind _ _ = _
  cases x core ex with
  | error e => rfl
  | ok v => rfl

theorem hoare_bind {α β : Type} {P : Core → Prop} {Q : α → Core → Prop} {R : β → Core → Prop}
    {x : M cc α} {f : α → M cc β} (h1 : Hoare P x Q) (h2 : ∀ a, Hoare (Q a) (f a) R) :
    Hoare P (x >>= f) R := by
  intro core ex b core' ex' hP h
  rw [run_bind] at h
  generalize hx : ((x.run core).run ex) = r at h
  cases r with
  | error e => cases h
  | ok v =>
    rcases v with ⟨⟨a, c⟩, e⟩
    exact h2 a c e b core' ex' (h1 core ex a c e hP hx) h

theorem hoare_pure {α : Type} {P : Core → Prop} (a : α) : Hoare (cc := cc) P (pure a) (fun _ => P) := by
  intro core ex b core' ex' hP h
  simp only [StateT.run_pure] at h
  cases h; exact hP

theorem hoare_conseq {α : Type} {P P' : Core → Prop} {Q Q' : α → Core → Prop} {x : M cc α}
    (h : Hoare P x Q) (hp : ∀ c, P' c → P c) (hq : ∀ a c, Q a c → Q' a c) : Hoare P' x Q' :=
  fun core ex a core' ex' hP hr => hq a core' (h core ex a core' ex' (hp core hP) hr)

theorem hoare_get {P : Core → Prop} : Hoare (cc := cc) P get (fun c c' => P c' ∧ c = c') := by
  intro core ex a core' ex' hP h
  cases h; exact ⟨hP, rfl⟩

theorem hoare_set {P : Core → Prop} (c : Core) : Hoare (cc := cc) P (set c) (fun _ c' => c' = c) := by
  intro core ex a core' ex' _ h
  simp only [StateT.run_set] at h
  cases h; rfl

theorem hoare_modify {P : Core → Prop} (f : Core → Core) :
    Hoare (cc := cc) P (modify f) (fun _ c' => ∃ c, P c ∧ c' = f c) := by
  intro core ex a core' ex' hP h
  simp only [StateT.run_modify] at h
  cases h; exact ⟨core, hP, rfl⟩

/-- Actions lifted from the inner monad leave the core unchanged. -/
theorem hoare_lift {α : Type} {P : Core → Prop} (x : SM cc α) :
    Hoare (cc := cc) P (StateT.lift x) (fun _ => P) := by
  intro core ex a core' ex' hP h
  have : ((StateT.lift x : M cc α).run core).run ex =
      (match x ex with
      | .ok p => .ok ((p.1, core), p.2)
      | .error e => .error e : Except StepError ((α × Core) × ExecSt cc)) := by
    simp only [StateT.run]
    show Except.bind _ _ = _
    cases x ex with
    | error e => rfl
    | ok v => rfl
  rw [this] at h
  generalize x ex = r at h
  cases r with
  | error e => cases h
  | ok v => cases h; exact hP

theorem hoare_liftE {α : Type} {P : Core → Prop} (x : EM cc α) :
    Hoare (cc := cc) P (Interp.liftE cc x) (fun _ => P) := by
  unfold Interp.liftE
  refine hoare_bind hoare_get (fun c => ?_)
  exact hoare_conseq (hoare_lift _) (fun c' h => h.2 ▸ h.1) (fun _ _ h => h)

theorem hoare_throw {α : Type} {P : Core → Prop} {Q : α → Core → Prop} (e : StepError) :
    Hoare (cc := cc) P (MonadExcept.throw e) Q := by
  intro core ex a core' ex' _ h
  cases h

theorem hoare_ite {α : Type} {P : Core → Prop} {Q : α → Core → Prop} (b : Prop) [Decidable b]
    {x y : M cc α} (hx : b → Hoare P x Q) (hy : ¬b → Hoare P y Q) :
    Hoare P (if b then x else y) Q := by
  by_cases h : b
  · simp only [h, ↓reduceIte]; exact hx h
  · simp only [h, ↓reduceIte]; exact hy h

/-- Loop rule for `forIn` over a list with an invariant (the loop state is ignored). -/
theorem hoare_forIn {α β : Type} {I : Core → Prop} (l : List α) (b : β)
    (f : α → β → M cc (ForInStep β)) (h : ∀ x b, Hoare I (f x b) (fun _ => I)) :
    Hoare I (forIn l b f) (fun _ => I) := by
  induction l generalizing b with
  | nil => simp only [List.forIn_nil]; exact hoare_pure b
  | cons x xs ih =>
    simp only [List.forIn_cons]
    refine hoare_bind (h x b) (fun r => ?_)
    cases r with
    | done b' => exact hoare_pure b'
    | yield b' => exact ih b'

/-- Loop rule for `forM` over a list with an invariant. -/
theorem hoare_forM {α : Type} {I : Core → Prop} (l : List α) (f : α → M cc PUnit)
    (h : ∀ x, Hoare I (f x) (fun _ => I)) : Hoare I (l.forM f) (fun _ => I) := by
  induction l with
  | nil => exact hoare_pure _
  | cons x xs ih =>
    have : (x :: xs).forM f = (f x >>= fun _ => xs.forM f) := by simp [List.forM_cons]
    rw [this]
    exact hoare_bind (h x) (fun _ => ih)

end Vertep.Config

namespace Vertep.Config

variable {cc : CChart}

/-- Fold rule for `forIn` over a list without `break`: the body moves the ghost state `s` to
`g s x`. -/
theorem hoare_forIn_fold {α β S : Type} (P : S → Core → Prop) (g : S → α → S) (l : List α)
    (b : β) (s : S) (f : α → β → M cc (ForInStep β))
    (h : ∀ x b s, Hoare (P s) (f x b) (fun r c => (∃ b', r = .yield b') ∧ P (g s x) c)) :
    Hoare (P s) (forIn l b f) (fun _ => P (l.foldl g s)) := by
  induction l generalizing b s with
  | nil => simp only [List.forIn_nil, List.foldl_nil]; exact hoare_pure b
  | cons x xs ih =>
    simp only [List.forIn_cons, List.foldl_cons]
    refine hoare_bind (h x b s) (fun r => ?_)
    cases r with
    | done b' =>
      intro core ex a core' ex' hP _
      obtain ⟨⟨_, hb⟩, _⟩ := hP
      cases hb
    | yield b' =>
      exact hoare_conseq (ih b' (g s x)) (fun _ h => h.2) (fun _ _ h => h)

/-- The frame of an interpreter action: configuration, history and termination flag. -/
def Fr (c : List Nat) (h : List (Nat × List Nat)) (t : Bool) (core : Core) : Prop :=
  core.config = c ∧ core.history = h ∧ core.terminated = t

/-- `x` leaves configuration, history and termination flag unchanged. -/
def Frame {α : Type} (x : M cc α) : Prop := ∀ c h t, Hoare (Fr c h t) x (fun _ => Fr c h t)

theorem Frame.bind {α β : Type} {x : M cc α} {f : α → M cc β} (hx : Frame x)
    (hf : ∀ a, Frame (f a)) : Frame (x >>= f) :=
  fun c h t => hoare_bind (hx c h t) (fun a => hf a c h t)

theorem Frame.pure {α : Type} (a : α) : Frame (cc := cc) (Pure.pure a) :=
  fun _ _ _ => hoare_pure a

theorem Frame.liftE {α : Type} (x : EM cc α) : Frame (Interp.liftE cc x) :=
  fun _ _ _ => hoare_liftE x

theorem Frame.lift {α : Type} (x : SM cc α) : Frame (cc := cc) (StateT.lift x) :=
  fun _ _ _ => hoare_lift x

theorem Frame.modify (f : Core → Core)
    (hf : ∀ c, (f c).config = c.config ∧ (f c).history = c.history ∧ (f c).terminated = c.terminated) :
    Frame (cc := cc) (modify f) := by
  intro c h t
  refine hoare_conseq (hoare_modify f) (fun _ x => x) ?_
  rintro _ core' ⟨core, ⟨h1, h2, h3⟩, rfl⟩
  obtain ⟨e1, e2, e3⟩ := hf core
  exact ⟨e1.trans h1, e2.trans h2, e3.trans h3⟩

/-- `let core ← get; set (f core)` with `f` keeping the frame. -/
theorem Frame.getSet {α : Type} (f : Core → Core) (k : M cc α)
    (hf : ∀ c, (f c).config = c.config ∧ (f c).history = c.history ∧ (f c).terminated = c.terminated)
    (hk : Frame k) : Frame (do let core ← get; set (f core); k) := by
  intro c h t
  refine hoare_bind hoare_get (fun core => ?_)
  intro core0 ex a core' ex' hpre hrun
  obtain ⟨⟨h1, h2, h3⟩, rfl⟩ := hpre
  obtain ⟨e1, e2, e3⟩ := hf core
  refine (hoare_bind (Q := fun _ c' => c' = f core) (R := fun _ => Fr c h t)
    (hoare_set (P := fun _ => True) (f core)) (fun _ => ?_)) core ex a core' ex' trivial hrun
  refine hoare_conseq (hk c h t) ?_ (fun _ _ x => x)
  rintro _ rfl
  exact ⟨e1.trans h1, e2.trans h2, e3.trans h3⟩

theorem Frame.ite {α : Type} (b : Prop) [Decidable b] {x y : M cc α} (hx : b → Frame x)
    (hy : ¬b → Frame y) : Frame (if b then x else y) :=
  fun c h t => hoare_ite b (fun hb => hx hb c h t) (fun hb => hy hb c h t)

theorem Frame.forIn {α β : Type} (l : List α) (b : β) (f : α → β → M cc (ForInStep β))
    (h : ∀ x b, Frame (f x b)) : Frame (forIn l b f) :=
  fun c hh t => hoare_forIn l b f (fun x b => h x b c hh t)

theorem Frame.forM {α : Type} (l : List α) (f : α → M cc PUnit) (h : ∀ x, Frame (f x)) :
    Frame (l.forM f) :=
  fun c hh t => hoare_forM l f (fun x => h x c hh t)

theorem Frame.throw {α : Type} (e : StepError) : Frame (cc := cc) (α := α) (MonadExcept.throw e) :=
  fun _ _ _ => hoare_throw e

/-- Fold rule for `forM` over a list: the body moves the ghost state `s` to `g s x`. -/
theorem hoare_forM_fold {α S : Type} (P : S → Core → Prop) (g : S → α → S) (l : List α)
    (s : S) (f : α → M cc PUnit) (h : ∀ x s, Hoare (P s) (f x) (fun _ => P (g s x))) :
    Hoare (P s) (l.forM f) (fun _ => P (l.foldl g s)) := by
  induction l generalizing s with
  | nil => exact hoare_pure _
  | cons x xs ih =>
    have : (x :: xs).forM f = (f x >>= fun _ => xs.forM f) := by simp [List.forM_cons]
    rw [this, List.foldl_cons]
    exact hoare_bind (h x s) (fun _ => ih (g s x))

end Vertep.Config
