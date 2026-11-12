import VertepLean.TypedStep.Exec

/-!
# The interpreter keeps the typing invariant

Every interpreter action of `start` and `step` keeps `Inv Γ`: the interpreter changes the
datamodel state only through executable content (`liftE`), selection guards, `dequeue` and the
clearing of the internal queue at exit, and the events it queues itself (`done.state.*`) are
typed by `ChartOk`.
-/

namespace Vertep.Typed

open Types

variable {Γ : ChartTyping} {cc : CChart}

theorem hm_ite {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} {c : Prop}
    [Decidable c] {x y : M cc α} (hx : c → HM P x Q) (hy : ¬c → HM P y Q) :
    HM P (if c then x else y) Q := by
  by_cases h : c
  · simp only [h, ↓reduceIte]; exact hx h
  · simp only [h, ↓reduceIte]; exact hy h

theorem hm_getThe {P : ExecSt cc → Prop} :
    HM P (getThe (ExecSt cc) : M cc (ExecSt cc)) (fun s' s => s' = s ∧ P s) := by
  intro core st b st' hP hr
  cases hr; exact ⟨rfl, hP⟩

theorem hm_setThe {P : ExecSt cc → Prop} {Q : PUnit → ExecSt cc → Prop} (s₀ : ExecSt cc)
    (h : (∃ s, P s) → Q PUnit.unit s₀) : HM P (set s₀ : M cc PUnit) Q := by
  intro core st b st' hP hr
  cases hr; exact h ⟨st, hP⟩

theorem hm_modifyThe {P : ExecSt cc → Prop} {Q : PUnit → ExecSt cc → Prop}
    (f : ExecSt cc → ExecSt cc) (h : ∀ s, P s → Q PUnit.unit (f s)) :
    HM P (modifyThe (ExecSt cc) f : M cc PUnit) Q := by
  intro core st b st' hP hr
  cases hr; exact h st hP

/-- The selection loop keeps an invariant every test keeps. -/
theorem firstIdx_inv {I : ExecSt cc → Prop} (test : Nat → Nat → SM cc Bool)
    (h : ∀ k t, HS I (test k t) (fun _ => I)) :
    ∀ (l : List Nat) (k : Nat), HS I (Select.firstIdx test l k) (fun _ => I)
  | [], _ => by
    rw [Select.firstIdx]
    exact hs_pure _ (fun _ h => h)
  | t :: ts, k => by
    rw [Select.firstIdx]
    refine hs_bind (h k t) (fun b => ?_)
    by_cases hb : b = true
    · simp only [hb, ↓reduceIte]; exact hs_pure _ (fun _ h => h)
    · simp only [hb, Bool.false_eq_true, ↓reduceIte]; exact firstIdx_inv test h ts (k + 1)

theorem choices_inv {I : ExecSt cc → Prop} (cands : Nat → List Nat)
    (test : Nat → Nat → Nat → SM cc Bool) (h : ∀ i k t, HS I (test i k t) (fun _ => I)) :
    ∀ (l : List Nat) (i : Nat), HS I (Select.choices cands test l i) (fun _ => I)
  | [], _ => by
    rw [Select.choices]
    exact hs_pure _ (fun _ h => h)
  | a :: as, i => by
    rw [Select.choices]
    refine hs_bind (firstIdx_inv (test i) (h i) _ 0) (fun c => ?_)
    refine hs_bind (choices_inv cands test h as (i + 1)) (fun cs => ?_)
    exact hs_pure _ (fun _ h => h)

theorem select_inv (hc : ChartOk Γ cc) (ev : Option String) :
    HM (Inv Γ) (Interp.select cc ev) (fun _ => Inv Γ) := by
  unfold Interp.select
  refine hm_bind hm_get (fun core => ?_)
  refine hm_bind (hm_lift (choices_inv _ _ (fun i k t => ?_) _ 0)) (fun _ => hm_pure _ (fun _ h => h))
  split
  · exact guard_inv hc t _
  · exact hs_pure _ (fun _ h => h)

theorem runBlocks_inv (hc : ChartOk Γ cc) (bs : List Block) (h : ∀ b ∈ bs, ∀ l ∈ blockLocs b, l ∈ cc.locations)
    (hok : ∀ b ∈ bs, BlockOk Γ b) :
    HM (Inv Γ) (Interp.runBlocks cc bs h) (fun _ => Inv Γ) := by
  unfold Interp.runBlocks
  refine hm_forM _ _ (fun x _ => ?_)
  obtain ⟨b, hb⟩ := x
  exact hm_liftE (execTop_inv hc b _ (hok b hb))

theorem setEvent_inv (hc : ChartOk Γ cc) (e : Event) (he : EventTyped Γ e) :
    HM (Inv Γ) (Interp.setEvent cc e) (fun _ => Inv Γ) := by
  unfold Interp.setEvent
  exact hm_bind (hm_liftE (ask_inv hc _ rfl he)) (fun _ => hm_pure _ (fun _ h => h))

theorem runTrans_inv (hc : ChartOk Γ cc) (t : Nat) : HM (Inv Γ) (Interp.runTrans cc t) (fun _ => Inv Γ) := by
  unfold Interp.runTrans
  exact hm_liftE (execTop_inv hc _ _ (hc.trans t))

theorem record_inv (ev : Option String) (ts : List Nat) :
    HM (Inv Γ) (Interp.record cc ev ts) (fun _ => Inv Γ) := by
  unfold Interp.record
  exact hm_modify _

theorem cancelInvocations_inv (s : Nat) :
    HM (Inv Γ) (Interp.cancelInvocations cc s) (fun _ => Inv Γ) := by
  unfold Interp.cancelInvocations
  refine hm_bind hm_get (fun _ => hm_bind (hm_set _) (fun _ => hm_forM _ _ (fun i _ => ?_)))
  exact hm_ite (fun _ => hm_pure _ (fun _ h => h)) (fun _ => hm_liftE (emit_inv _ trivial))

theorem recordHistory_inv (ex : List Nat) :
    HM (Inv Γ) (Interp.recordHistory cc ex) (fun _ => Inv Γ) := by
  unfold Interp.recordHistory
  refine hm_forM _ _ (fun s _ => hm_forM _ _ (fun h _ => ?_))
  exact hm_bind hm_get (fun _ => hm_set _)

theorem charge_inv : HM (Inv Γ) (Interp.charge cc) (fun _ => Inv Γ) := by
  unfold Interp.charge
  refine hm_bind hm_get (fun _ => hm_ite (fun _ => ?_) (fun _ => ?_)) <;>
    exact hm_bind (hm_set _) (fun _ => hm_pure _ (fun _ h => h))

theorem hm_fact {α : Type} {I : ExecSt cc → Prop} {F : Prop} {x : M cc α}
    {Q : α → ExecSt cc → Prop} (h : F → HM I x Q) : HM (fun s => I s ∧ F) x Q :=
  fun core st a st' hP hr => h hP.2 core st a st' hP.1 hr

/-- Glue: the structure of interpreter actions (conditionals, matches, loops, core updates)
around actions with known specifications. -/
local macro "m_step " hc:term : tactic => `(tactic| (first
    | (he_is ite; refine hm_ite (fun _ => ?_) (fun _ => ?_))
    | (he_is Pure.pure; refine hm_pure _ (fun _ h => h))
    | (he_bind_is Pure.pure;
        refine hm_bind (Q := fun _ => Inv _) (hm_pure _ (fun _ h => h)) (fun _ => ?_))
    | (he_bind_is MonadState.get; refine hm_bind hm_get (fun _ => ?_))
    | (he_bind_is MonadStateOf.set; refine hm_bind (hm_set _) (fun _ => ?_))
    | (he_bind_is modify; refine hm_bind (hm_modify _) (fun _ => ?_))
    | (he_is modify; exact hm_modify _)
    | (he_is MonadStateOf.set; exact hm_set _)
    | (he_is List.forM; refine hm_forM _ _ (fun _ _ => ?_))
    | (he_bind_is List.forM; refine hm_bind (hm_forM _ _ (fun _ _ => ?_)) (fun _ => ?_))
    | (he_bind_is forIn; refine hm_bind (hm_forIn _ _ _ (fun _ _ _ => ?_)) (fun _ => ?_))
    | (he_is forIn; refine hm_forIn _ _ _ (fun _ _ _ => ?_))
    | (he_bind_is Vertep.Interp.runBlocks; first
        | refine hm_bind (runBlocks_inv $hc _ _ (fun _ hb => ($hc).onexit _ _ hb)) (fun _ => ?_)
        | refine hm_bind (runBlocks_inv $hc _ _ (fun _ hb => ($hc).onentry _ _ hb)) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.cancelInvocations;
        refine hm_bind (cancelInvocations_inv _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.recordHistory; refine hm_bind (recordHistory_inv _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.runTrans; refine hm_bind (runTrans_inv $hc _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE; first
        | refine hm_bind (hm_liftE (queueError_inv $hc _)) (fun _ => ?_)
        | refine hm_bind (hm_liftE (emit_inv _ trivial)) (fun _ => ?_))
    | (he_is Vertep.Interp.liftE; first
        | exact hm_liftE (queueError_inv $hc _)
        | exact hm_liftE (emit_inv _ trivial))
    | (he_is_match; split)))

local macro "m_auto " hc:term : tactic => `(tactic| repeat' m_step $hc)

theorem exitStates_inv (hc : ChartOk Γ cc) (ts : List Nat) :
    HM (Inv Γ) (Interp.exitStates cc ts) (fun _ => Inv Γ) := by
  unfold Interp.exitStates
  m_auto hc

theorem enterStates_inv (hc : ChartOk Γ cc) (dhist : List (Nat × List Nat)) (ts : List Nat) :
    HM (Inv Γ) (Interp.enterStates cc dhist ts) (fun _ => Inv Γ) := by
  unfold Interp.enterStates
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (initData_inv hc _ _ _ (hc.data _) (fun p hp => by simp at hp)))
          (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (doneData_spec hc _)) (fun _ => hm_fact (fun hd =>
          hm_bind (hm_liftE (enqueue_inv _ (hc.done _ _ (by assumption) hd))) (fun _ => ?_))))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (enqueue_inv _ (hc.doneParallel _))) (fun _ => ?_))
    | (he_bind_is MonadExcept.throw; exact hm_throw _)
    | (he_is MonadExcept.throw; exact hm_throw _))

theorem microstep_inv (hc : ChartOk Γ cc) (ev : Option String) (ts : List Nat) :
    HM (Inv Γ) (Interp.microstep cc ev ts) (fun _ => Inv Γ) := by
  unfold Interp.microstep
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.charge; refine hm_bind charge_inv (fun _ => ?_))
    | (he_bind_is Vertep.Interp.record; refine hm_bind (record_inv _ _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.exitStates; refine hm_bind (exitStates_inv hc _) (fun _ => ?_))
    | (he_is Vertep.Interp.enterStates; exact enterStates_inv hc _ _))

theorem startInvokes_inv (hc : ChartOk Γ cc) :
    HM (Inv Γ) (Interp.startInvokes cc) (fun _ => Inv Γ) := by
  unfold Interp.startInvokes
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (startInvoke_inv hc _ _ _ _
          (fun l hl => (hc.invokes _ _ (Subtype.property _)).1 l hl))) (fun _ => ?_)))

theorem dequeue_spec :
    HM (Inv Γ) (Interp.dequeue cc) (fun r st => Inv Γ st ∧ ∀ e, r = some e → EventTyped Γ e) := by
  unfold Interp.dequeue
  refine hm_bind hm_getThe (fun st0 => ?_)
  split
  · exact hm_pure _ (fun _ h => ⟨h.2, fun e he => by cases he⟩)
  · rename_i e rest hint
    refine hm_bind (hm_setThe _ (Q := fun _ s => Inv Γ s ∧ EventTyped Γ e) (fun ⟨s, hs⟩ => ?_))
      (fun _ => hm_pure _ (fun _ h => ⟨h.1, fun e' he' => by cases he'; exact h.2⟩))
    obtain ⟨rfl, h1, h2, h3, h4⟩ := hs
    rw [hint] at h3
    exact ⟨⟨h1, h2, fun x hx => h3 x (List.mem_cons_of_mem _ hx), h4⟩,
      h3 e (List.mem_cons_self ..)⟩

theorem hm_bind_dequeue {β : Type} {f : Option Event → M cc β} {R : β → ExecSt cc → Prop}
    (hn : HM (Inv Γ) (f none) R) (hs : ∀ e, EventTyped Γ e → HM (Inv Γ) (f (some e)) R) :
    HM (Inv Γ) (Interp.dequeue cc >>= f) R := by
  refine hm_bind dequeue_spec (fun r => ?_)
  cases r with
  | none => exact hm_conseq hn (fun _ h => h.1) (fun _ _ h => h)
  | some e => exact hm_fact (fun he => hs e (he e rfl))

theorem exitInterpreter_inv (hc : ChartOk Γ cc) (cancelled : Bool) :
    HM (Inv Γ) (Interp.exitInterpreter cc cancelled) (fun _ => Inv Γ) := by
  unfold Interp.exitInterpreter
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (he_conseq (doneData_spec hc _) (fun _ h => h)
          (fun _ _ h => h.1))) (fun _ => ?_))
    | (he_bind_is modifyThe;
        refine hm_bind (hm_modifyThe (Q := fun _ => Inv Γ) _ (fun s ⟨h1, h2, _, h4⟩ =>
          ⟨h1, h2, fun x hx => (by cases hx), h4⟩)) (fun _ => ?_)))

theorem macroLoop_inv (hc : ChartOk Γ cc) :
    ∀ fuel, HM (Inv Γ) (Interp.macroLoop cc fuel) (fun _ => Inv Γ)
  | 0 => by
    rw [Interp.macroLoop]
    exact hm_bind hm_get (fun _ => hm_throw _)
  | fuel + 1 => by
    rw [Interp.macroLoop]
    repeat' (first
      | m_step hc
      | (he_bind_is Vertep.Interp.exitInterpreter;
          refine hm_bind (exitInterpreter_inv hc _) (fun _ => ?_))
      | (he_bind_is Vertep.Interp.select; refine hm_bind (select_inv hc _) (fun _ => ?_))
      | (he_bind_is Vertep.Interp.microstep; refine hm_bind (microstep_inv hc _ _) (fun _ => ?_))
      | (he_bind_is Vertep.Interp.charge; refine hm_bind charge_inv (fun _ => ?_))
      | (he_bind_is Vertep.Interp.record; refine hm_bind (record_inv _ _) (fun _ => ?_))
      | (he_bind_is Vertep.Interp.startInvokes;
          refine hm_bind (startInvokes_inv hc) (fun _ => ?_))
      | (he_bind_is getThe; refine hm_bind (hm_conseq hm_getThe (fun _ h => h)
          (fun _ _ h => h.2)) (fun _ => ?_))
      | (he_bind_is Vertep.Interp.dequeue; refine hm_bind_dequeue ?_ (fun e he => ?_) <;> dsimp only)
      | (he_bind_is Vertep.Interp.setEvent;
          refine hm_bind (setEvent_inv hc _ (by assumption)) (fun _ => ?_))
      | (he_is Vertep.Interp.macroLoop; exact macroLoop_inv hc fuel))

theorem runMacrostep_inv (hc : ChartOk Γ cc) :
    HM (Inv Γ) (Interp.runMacrostep cc) (fun _ => Inv Γ) := by
  unfold Interp.runMacrostep
  exact hm_bind hm_get (fun _ => macroLoop_inv hc _)

theorem runExternal_inv (hc : ChartOk Γ cc) (ev : Event) (hev : EventTyped Γ ev) :
    HM (Inv Γ) (Interp.runExternal cc ev) (fun _ => Inv Γ) := by
  unfold Interp.runExternal
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.setEvent; refine hm_bind (setEvent_inv hc _ hev) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (emit_inv (.forward _ ev) hev)) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (execTop_inv hc _ _
          ((hc.invokes _ _ (List.mem_of_getElem? (by assumption))).2))) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.select; refine hm_bind (select_inv hc _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.microstep; refine hm_bind (microstep_inv hc _ _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.record; refine hm_bind (record_inv _ _) (fun _ => ?_))
    | (he_is Vertep.Interp.runMacrostep; exact runMacrostep_inv hc))

theorem invoker_ite (invoker : List (String × Value))
    (hinv : ∀ p ∈ invoker, LocTyped Γ p.1 p.2) (c : Bool) :
    ∀ p ∈ (if c = true then invoker else []), LocTyped Γ p.1 p.2 := by
  intro p hp
  cases c
  · simp at hp
  · exact hinv p (by simpa using hp)

theorem runStart_inv (hc : ChartOk Γ cc) (invoker : List (String × Value))
    (hinv : ∀ p ∈ invoker, LocTyped Γ p.1 p.2) :
    HM (Inv Γ) (Interp.runStart cc invoker) (fun _ => Inv Γ) := by
  unfold Interp.runStart
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (he_conseq (request_spec hc _ _ trivial) (fun _ h => h)
          (fun _ _ h => h.1))) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (ask_inv hc _ rfl trivial)) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (initData_inv hc _ _ _ (hc.data _) hinv)) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (initData_inv hc _ _ _ (hc.data _) (invoker_ite _ hinv _)))
          (fun _ => ?_))
    | (he_bind_is Vertep.Interp.enterStates;
        refine hm_bind (enterStates_inv hc _ _) (fun _ => ?_))
    | (he_is Vertep.Interp.runMacrostep; exact runMacrostep_inv hc))

theorem startBody_inv (hc : ChartOk Γ cc) (si : StartInput) (hsi : StartTyped Γ si) :
    HM (Inv Γ) (Interp.startBody cc si) (fun _ => Inv Γ) := by
  unfold Interp.startBody
  repeat' (first
    | m_step hc
    | (he_bind_is Vertep.Interp.liftE;
        refine hm_bind (hm_liftE (ask_inv hc _ rfl trivial)) (fun _ => ?_))
    | (he_bind_is MonadExcept.throw; exact hm_throw _)
    | (he_is MonadExcept.throw; exact hm_throw _)
    | (he_is Vertep.Interp.runStart; exact runStart_inv hc _ hsi))

theorem stepBody_inv (hc : ChartOk Γ cc) (i : Input) (hi : InputTyped Γ i) :
    HM (Inv Γ) (Interp.stepBody cc i) (fun _ => Inv Γ) := by
  unfold Interp.stepBody
  refine hm_bind (hm_liftE (ask_inv hc _ rfl trivial)) (fun _ => ?_)
  split
  · exact exitInterpreter_inv hc true
  · rename_i e he
    exact runExternal_inv hc e (hi e he)

end Vertep.Typed
