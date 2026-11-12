import VertepLean.TypedStep.Interp

/-!
# Chart type soundness for `start` and `step`

For a chart that passed the type checker under a typing `Γ` (`ChartOk Γ cc`), a typed answer
tape (`TapeTyped Γ`, the datamodel's obligation), a session whose queued events are typed and a
typed input event (checked by the hosts on admission), every successful `start` and `step`

* issues only typed requests: every value written to a typed location (`assignValue`,
  `bindVar`) has the location's type, every `assignExpr` assigns an expression whose static type
  is assignable to it, and every event made current is typed (`ReqTyped`);
* keeps the datamodel typed, for any datamodel whose typing invariant typed requests keep
  (`Datamodel`);
* emits only typed events (`send` and `forward` effects) and leaves only typed events in the
  internal queue.

The same holds for the partial result when the microstep budget runs out (`stepFull`,
`startFull`). `vertep_typed_soundness` is the M0 extension point `TypedStepSoundness` proven for
the Vertep type system.
-/

namespace Vertep.Typed

open Types

variable {Γ : ChartTyping} {cc : CChart}

/-- What a successful `start` or `step` produces is typed: its requests, its effects and the
internal queue of its session. -/
def OutTyped (Γ : ChartTyping) (o : Out cc) : Prop :=
  (∀ r ∈ o.requestList, ReqTyped Γ r) ∧ (∀ f ∈ o.effects, EffectTyped Γ f) ∧
    SessionTyped Γ o.session

theorem runFull_typed (input : TraceInput) (body : M cc Unit) (s : Session) (tape : Tape)
    (sessions : Option (List String)) (hb : HM (Inv Γ) body (fun _ => Inv Γ))
    (hs : SessionTyped Γ s) (ht : TapeTyped Γ tape) (b : Bool) (o : Out cc)
    (h : Interp.runFull cc input body s tape sessions = .ok (b, o)) : OutTyped Γ o := by
  unfold Interp.runFull at h
  dsimp only at h
  split at h
  · rename_i core' ex' hrun
    cases h
    have hI := hb _ _ ((), core') ex'
      (by exact ⟨ht, fun r hr => (by simp at hr), hs, fun f hf => (by simp at hf)⟩) hrun
    obtain ⟨_, hl, hi, he⟩ := hI
    refine ⟨?_, he, hi⟩
    intro r hr
    simp only [Out.requestList, Interp.finish, List.mem_map] at hr
    obtain ⟨x, hx, rfl⟩ := hr
    exact hl x hx
  · cases h

theorem run_typed (input : TraceInput) (body : M cc Unit) (s : Session) (tape : Tape)
    (sessions : Option (List String)) (hb : HM (Inv Γ) body (fun _ => Inv Γ))
    (hs : SessionTyped Γ s) (ht : TapeTyped Γ tape) (o : Out cc)
    (h : Interp.run cc input body s tape sessions = .ok o) : OutTyped Γ o := by
  unfold Interp.run at h
  split at h
  · cases h
  · rename_i o' hr
    cases h
    exact runFull_typed input body s tape sessions hb hs ht false o hr
  · cases h

/-- **Typed requests, effects and queue of `step`.** -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem step_typed (hc : ChartOk Γ cc) (s : Session) (i : Input) (tape : Tape) (o : Out cc)
    (hs : SessionTyped Γ s) (hi : InputTyped Γ i) (ht : TapeTyped Γ tape)
    (h : step cc s i tape = .ok o) : OutTyped Γ o := by
  unfold step at h
  split at h
  · cases h
  · exact run_typed _ _ s tape _ (stepBody_inv hc i hi) hs ht o h

/-- The same for the partial result of a macrostep that ran out of budget. -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem stepFull_typed (hc : ChartOk Γ cc) (s : Session) (i : Input) (tape : Tape) (b : Bool)
    (o : Out cc) (hs : SessionTyped Γ s) (hi : InputTyped Γ i) (ht : TapeTyped Γ tape)
    (h : stepFull cc s i tape = .ok (b, o)) : OutTyped Γ o := by
  unfold stepFull at h
  split at h
  · cases h
  · exact runFull_typed _ _ s tape _ (stepBody_inv hc i hi) hs ht b o h

/-- **Typed requests, effects and queue of `start`.** -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem start_typed (hc : ChartOk Γ cc) (si : StartInput) (tape : Tape) (o : Out cc)
    (hsi : StartTyped Γ si) (ht : TapeTyped Γ tape) (h : start cc si tape = .ok o) :
    OutTyped Γ o :=
  run_typed _ _ _ tape _ (startBody_inv hc si hsi) (fun e he => by cases he) ht o h

/-- The same for the partial result of a start that ran out of budget. -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem startFull_typed (hc : ChartOk Γ cc) (si : StartInput) (tape : Tape) (b : Bool)
    (o : Out cc) (hsi : StartTyped Γ si) (ht : TapeTyped Γ tape)
    (h : startFull cc si tape = .ok (b, o)) : OutTyped Γ o :=
  runFull_typed _ _ _ tape _ (startBody_inv hc si hsi) (fun e he => by cases he) ht b o h

/-- Typed requests keep a datamodel typed. -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem datamodel_typed (dm : Datamodel Γ) (σ : dm.St) (hσ : dm.Typed σ) :
    ∀ rs : List Request, (∀ r ∈ rs, ReqTyped Γ r) → dm.Typed (rs.foldl dm.next σ) := by
  intro rs
  induction rs generalizing σ with
  | nil => intro _; exact hσ
  | cons r rs ih =>
    intro h
    exact ih (dm.next σ r) (dm.keeps σ r hσ (h r (List.mem_cons_self ..)))
      (fun x hx => h x (List.mem_cons_of_mem _ hx))

/-- **Chart type soundness for `step`.** For a chart that passed the type checker under `Γ`, a
typed datamodel, a typed session, a typed input event and a typed answer tape, `step` yields a
typed datamodel, every value it writes to a typed location has that location's type, every
event it sends or forwards carries a payload of its declared type, and the new session's queued
events are typed. -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem typed_step_soundness (Γ : ChartTyping) (cc : CChart) (hc : ChartOk Γ cc)
    (dm : Datamodel Γ) (σ : dm.St) (s : Session) (i : Input) (tape : Tape) (o : Out cc)
    (hσ : dm.Typed σ) (hs : SessionTyped Γ s) (hi : InputTyped Γ i) (ht : TapeTyped Γ tape)
    (h : step cc s i tape = .ok o) :
    dm.Typed (o.requestList.foldl dm.next σ) ∧ (∀ r ∈ o.requestList, ReqTyped Γ r) ∧
      (∀ f ∈ o.effects, EffectTyped Γ f) ∧ SessionTyped Γ o.session := by
  obtain ⟨hr, he, hs'⟩ := step_typed hc s i tape o hs hi ht h
  exact ⟨datamodel_typed dm σ hσ _ hr, hr, he, hs'⟩

/-- **Chart type soundness for `start`.** -/
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem typed_start_soundness (Γ : ChartTyping) (cc : CChart) (hc : ChartOk Γ cc)
    (dm : Datamodel Γ) (σ : dm.St) (si : StartInput) (tape : Tape) (o : Out cc)
    (hσ : dm.Typed σ) (hsi : StartTyped Γ si) (ht : TapeTyped Γ tape)
    (h : start cc si tape = .ok o) :
    dm.Typed (o.requestList.foldl dm.next σ) ∧ (∀ r ∈ o.requestList, ReqTyped Γ r) ∧
      (∀ f ∈ o.effects, EffectTyped Γ f) ∧ SessionTyped Γ o.session := by
  obtain ⟨hr, he, hs'⟩ := start_typed hc si tape o hsi ht h
  exact ⟨datamodel_typed dm σ hσ _ hr, hr, he, hs'⟩

/-- The Vertep type system as an instance of the M0 extension point. -/
def vertepTypes (Γ : ChartTyping) : TypeSystem where
  Ty := Ty
  HasType := fun v t => HasType Γ.decls v t
  locType := Γ.loc
  TapeTyped := TapeTyped Γ
  SessionTyped := SessionTyped Γ
  InputTyped := InputTyped Γ

/-- **The M1 typed theorem.** The M0 extension point `TypedStepSoundness` holds for the Vertep
type system on every chart that passed the type checker. -/
-- @tg: implements:: [[openspec:formal-model#Type Soundness Statement]]
-- @tg: implements:: [[openspec:formal-model#Chart Type Soundness]]
theorem vertep_typed_soundness (Γ : ChartTyping) (cc : CChart) (hc : ChartOk Γ cc) :
    TypedStepSoundness cc (vertepTypes Γ) := by
  intro s i tape o hs hi ht h
  obtain ⟨hr, _, hs'⟩ := step_typed hc s i tape o hs hi ht h
  refine ⟨requests_ok o, ?_, hs'⟩
  intro r hrm l v τ hrv hl
  have := hr r hrm
  rcases hrv with rfl | rfl
  · exact this τ hl
  · exact this τ hl

end Vertep.Typed
