import VertepLean.TypedStep.Basic
import Lean

/-!
# Hoare triples over the datamodel state

`HS P x Q`: every successful run of a state action `x` from a state satisfying `P` ends in a
state satisfying `Q` (for the returned value). `HE` lifts it to executable content (`EM`, for
every environment) and `HM` to the interpreter (`M`, for every interpreter core): the typing
invariant lives in `ExecSt` (the tape, the request log, the internal queue and the effects),
which the interpreter only changes through executable content and `dequeue`. Failed runs are not
constrained.
-/

namespace Vertep.Typed

variable {cc : CChart}

/-- Partial-correctness triple over a state monad on `Except StepError`. -/
def HS {σ α : Type} (P : σ → Prop) (x : StateT σ (Except StepError) α) (Q : α → σ → Prop) :
    Prop :=
  ∀ st a st', P st → x st = .ok (a, st') → Q a st'

theorem hs_bind {σ α β : Type} {P : σ → Prop} {Q : α → σ → Prop} {R : β → σ → Prop}
    {x : StateT σ (Except StepError) α} {f : α → StateT σ (Except StepError) β}
    (h1 : HS P x Q) (h2 : ∀ a, HS (Q a) (f a) R) : HS P (x >>= f) R := by
  intro st b st' hP h
  have e : (x >>= f) st = (match x st with | .ok (a, s) => f a s | .error e => .error e) := by
    show Except.bind _ _ = _
    cases x st <;> rfl
  rw [e] at h
  cases hx : x st with
  | error e => rw [hx] at h; cases h
  | ok p => rw [hx] at h; exact h2 p.1 p.2 b st' (h1 st p.1 p.2 hP hx) h

theorem hs_pure {σ α : Type} {P : σ → Prop} {Q : α → σ → Prop} (a : α) (h : ∀ s, P s → Q a s) :
    HS P (pure a : StateT σ (Except StepError) α) Q := by
  intro st b st' hP hr
  cases hr; exact h st hP

theorem hs_get {σ : Type} {P : σ → Prop} {Q : σ → σ → Prop} (h : ∀ s, P s → Q s s) :
    HS P (get : StateT σ (Except StepError) σ) Q := by
  intro st b st' hP hr
  cases hr; exact h st hP

theorem hs_set {σ : Type} {P : σ → Prop} {Q : PUnit → σ → Prop} (s₀ : σ)
    (h : (∃ s, P s) → Q PUnit.unit s₀) : HS P (set s₀ : StateT σ (Except StepError) PUnit) Q := by
  intro st b st' hP hr
  cases hr; exact h ⟨st, hP⟩

theorem hs_conseq {σ α : Type} {P P' : σ → Prop} {Q Q' : α → σ → Prop}
    {x : StateT σ (Except StepError) α} (h : HS P x Q) (hp : ∀ s, P' s → P s)
    (hq : ∀ a s, Q a s → Q' a s) : HS P' x Q' :=
  fun st a st' hP hr => hq a st' (h st a st' (hp st hP) hr)

/-! ## Executable content -/

/-- Triple for executable content, for every environment. -/
def HE {α : Type} (P : ExecSt cc → Prop) (x : EM cc α) (Q : α → ExecSt cc → Prop) : Prop :=
  ∀ env, HS P (x env) Q

theorem he_bind {α β : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop}
    {R : β → ExecSt cc → Prop} {x : EM cc α} {f : α → EM cc β}
    (h1 : HE P x Q) (h2 : ∀ a, HE (Q a) (f a) R) : HE P (x >>= f) R :=
  fun env => hs_bind (h1 env) (fun a => h2 a env)

theorem he_pure {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} (a : α)
    (h : ∀ s, P s → Q a s) : HE P (pure a : EM cc α) Q :=
  fun _ => hs_pure a h

theorem he_read {P : ExecSt cc → Prop} : HE P (read : EM cc Env) (fun _ => P) := by
  intro env st b st' hP hr
  cases hr; exact hP

theorem he_get {P : ExecSt cc → Prop} {Q : ExecSt cc → ExecSt cc → Prop}
    (h : ∀ s, P s → Q s s) : HE P (get : EM cc (ExecSt cc)) Q :=
  fun _ => hs_get h

theorem he_set {P : ExecSt cc → Prop} {Q : PUnit → ExecSt cc → Prop} (s₀ : ExecSt cc)
    (h : (∃ s, P s) → Q PUnit.unit s₀) : HE P (set s₀ : EM cc PUnit) Q := by
  intro env st b st' hP hr
  cases hr; exact h ⟨st, hP⟩

theorem he_modify {P : ExecSt cc → Prop} {Q : PUnit → ExecSt cc → Prop}
    (f : ExecSt cc → ExecSt cc) (h : ∀ s, P s → Q PUnit.unit (f s)) :
    HE P (modify f : EM cc PUnit) Q := by
  intro env st b st' hP hr
  cases hr; exact h st hP

theorem he_throw {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} (e : StepError) :
    HE P (throw e : EM cc α) Q := by
  intro env st b st' _ hr
  cases hr

theorem he_conseq {α : Type} {P P' : ExecSt cc → Prop} {Q Q' : α → ExecSt cc → Prop}
    {x : EM cc α} (h : HE P x Q) (hp : ∀ s, P' s → P s) (hq : ∀ a s, Q a s → Q' a s) :
    HE P' x Q' :=
  fun env => hs_conseq (h env) hp hq

/-- An invariant kept by every element is kept by `forM`. -/
theorem he_forM {α : Type} {I : ExecSt cc → Prop} (l : List α) (f : α → EM cc PUnit)
    (h : ∀ x ∈ l, HE I (f x) (fun _ => I)) : HE I (l.forM f) (fun _ => I) := by
  induction l with
  | nil => exact he_pure _ (fun _ h => h)
  | cons x xs ih =>
    have e : (x :: xs).forM f = (f x >>= fun _ => xs.forM f) := by simp [List.forM_cons]
    rw [e]
    exact he_bind (h x (List.mem_cons_self ..))
      (fun _ => ih (fun y hy => h y (List.mem_cons_of_mem _ hy)))

/-- An invariant kept by every step of a fold is kept by `foldlM`. -/
theorem he_foldlM {α β : Type} {I : ExecSt cc → Prop} (l : List α) (f : β → α → EM cc β)
    (h : ∀ x ∈ l, ∀ b, HE I (f b x) (fun _ => I)) (b : β) :
    HE I (l.foldlM f b) (fun _ => I) := by
  induction l generalizing b with
  | nil => exact he_pure _ (fun _ h => h)
  | cons x xs ih =>
    rw [List.foldlM_cons]
    exact he_bind (h x (List.mem_cons_self ..) b)
      (fun b' => ih (fun y hy => h y (List.mem_cons_of_mem _ hy)) b')

/-- A fold whose accumulator satisfies `A` throughout. -/
theorem he_foldlM_acc {α β : Type} {I : ExecSt cc → Prop} {A : β → Prop} (l : List α)
    (f : β → α → EM cc β)
    (h : ∀ x ∈ l, ∀ b, A b → HE I (f b x) (fun b' s => I s ∧ A b')) (b : β) (hb : A b) :
    HE I (l.foldlM f b) (fun b' s => I s ∧ A b') := by
  induction l generalizing b with
  | nil => exact he_pure _ (fun _ h => ⟨h, hb⟩)
  | cons x xs ih =>
    rw [List.foldlM_cons]
    refine he_bind (h x (List.mem_cons_self ..) b hb) (fun b' => ?_)
    intro env st r st' hP hr
    exact ih (fun y hy => h y (List.mem_cons_of_mem _ hy)) b' hP.2 env st r st' hP.1 hr

/-! ## The interpreter -/

/-- Triple for the interpreter, for every interpreter core, constraining only the datamodel
state. -/
def HM {α : Type} (P : ExecSt cc → Prop) (x : M cc α) (Q : α → ExecSt cc → Prop) : Prop :=
  ∀ core, HS P (x core) (fun p => Q p.1)

theorem hm_bind {α β : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop}
    {R : β → ExecSt cc → Prop} {x : M cc α} {f : α → M cc β}
    (h1 : HM P x Q) (h2 : ∀ a, HM (Q a) (f a) R) : HM P (x >>= f) R :=
  fun core => hs_bind (h1 core) (fun p => h2 p.1 p.2)

theorem hm_pure {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} (a : α)
    (h : ∀ s, P s → Q a s) : HM P (pure a : M cc α) Q :=
  fun _ => hs_pure _ h

theorem hm_get {P : ExecSt cc → Prop} : HM P (get : M cc Core) (fun _ => P) := by
  intro core st b st' hP hr
  cases hr; exact hP

theorem hm_set {P : ExecSt cc → Prop} (c : Core) : HM P (set c : M cc PUnit) (fun _ => P) := by
  intro core st b st' hP hr
  cases hr; exact hP

theorem hm_modify {P : ExecSt cc → Prop} (f : Core → Core) :
    HM P (modify f : M cc PUnit) (fun _ => P) := by
  intro core st b st' hP hr
  cases hr; exact hP

theorem hm_throw {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} (e : StepError) :
    HM P (throw e : M cc α) Q := by
  intro core st b st' _ hr
  cases hr

theorem hm_conseq {α : Type} {P P' : ExecSt cc → Prop} {Q Q' : α → ExecSt cc → Prop}
    {x : M cc α} (h : HM P x Q) (hp : ∀ s, P' s → P s) (hq : ∀ a s, Q a s → Q' a s) :
    HM P' x Q' :=
  fun core => hs_conseq (h core) hp (fun p s hq' => hq p.1 s hq')

theorem hm_lift {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} {x : SM cc α}
    (h : HS P x Q) : HM P (StateT.lift x : M cc α) Q := by
  intro core
  have e : (StateT.lift x : M cc α) core = (x >>= fun a => pure (a, core)) := rfl
  rw [e]
  exact hs_bind h (fun a => hs_pure _ (fun _ h => h))

theorem hm_liftE {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} {x : EM cc α}
    (h : HE P x Q) : HM P (Interp.liftE cc x) Q := by
  unfold Interp.liftE
  exact hm_bind hm_get (fun core => hm_lift (h _))

/-- Loop rule for `forIn` over a list with an invariant. -/
theorem hm_forIn {α β : Type} {I : ExecSt cc → Prop} (l : List α) (b : β)
    (f : α → β → M cc (ForInStep β)) (h : ∀ x ∈ l, ∀ b, HM I (f x b) (fun _ => I)) :
    HM I (forIn l b f) (fun _ => I) := by
  induction l generalizing b with
  | nil => simp only [List.forIn_nil]; exact hm_pure b (fun _ h => h)
  | cons x xs ih =>
    simp only [List.forIn_cons]
    refine hm_bind (h x (List.mem_cons_self ..) b) (fun r => ?_)
    cases r with
    | done b' => exact hm_pure b' (fun _ h => h)
    | yield b' => exact ih b' (fun y hy => h y (List.mem_cons_of_mem _ hy))

/-- Loop rule for `forM` over a list with an invariant. -/
theorem hm_forM {α : Type} {I : ExecSt cc → Prop} (l : List α) (f : α → M cc PUnit)
    (h : ∀ x ∈ l, HM I (f x) (fun _ => I)) : HM I (l.forM f) (fun _ => I) := by
  induction l with
  | nil => exact hm_pure _ (fun _ h => h)
  | cons x xs ih =>
    have e : (x :: xs).forM f = (f x >>= fun _ => xs.forM f) := by simp [List.forM_cons]
    rw [e]
    exact hm_bind (h x (List.mem_cons_self ..))
      (fun _ => ih (fun y hy => h y (List.mem_cons_of_mem _ hy)))

/-! ## Goal inspection

`he_is f` succeeds when the program of the goal `HE P x Q` or `HM P x Q` is an application of
`f`, `he_bind_is f` when it is `y >>= k` with `y` an application of `f`, and `he_is_match` when
it is a `match`. They let the proof scripts pick a rule without unifying a rule against a large
program, which can unfold it. -/

open Lean Meta Elab Tactic in
/-- The program of an `HE` or `HM` goal, beta-reduced. -/
def heProgram : TacticM Lean.Expr := do
  let t ← instantiateMVars (← getMainTarget)
  let t := t.consumeMData
  unless t.isAppOfArity ``HE 5 || t.isAppOfArity ``HM 5 do
    throwError "not an HE or HM goal"
  let mut x := (t.getArg! 3).headBeta
  repeat
    match x.consumeMData with
    | .letE _ _ v b _ => x := (b.instantiate1 v).headBeta
    | _ => break
  return x.consumeMData

open Lean Meta Elab Tactic in
elab "he_is " n:ident : tactic => do
  let c ← realizeGlobalConstNoOverload n
  let x ← heProgram
  unless x.getAppFn.isConstOf c do throwError "program is not an application of {c}"

open Lean Meta Elab Tactic in
elab "he_bind_is " n:ident : tactic => do
  let c ← realizeGlobalConstNoOverload n
  let x ← heProgram
  unless x.isAppOfArity ``Bind.bind 6 do throwError "program is not a bind"
  unless (x.getArg! 4).headBeta.getAppFn.isConstOf c do
    throwError "bind does not start with {c}"

open Lean Meta Elab Tactic in
elab "he_is_match" : tactic => do
  let x ← heProgram
  let some n := x.getAppFn.constName? | throwError "program is not a match"
  unless (← isMatcher n) do throwError "program is not a match"

end Vertep.Typed
