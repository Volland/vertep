import VertepLean.Step

/-!
# The M0 theorems

* `step_deterministic`, `start_deterministic` — Step Determinism Theorem;
* `select_spec` and `transition_selection` — Transition Selection Theorem for the selection that
  every microstep of `step` runs;
* `step_requests_declared`, `start_requests_declared` — the dynamic type-soundness statement;
  `TypedStepSoundness` is the named M1 extension point and `dynamic_typed_soundness` shows the
  M0 theorem is its instance for the dynamic (unityped) type system.

The Configuration Well-Formedness Theorem is not proven yet; its statement lives in
`Pending/ConfigWF.lean`, outside the build.
-/

namespace Vertep

/-! ## Step determinism -/

/-- **Step determinism.** For every well-formed chart, equal session states, equal inputs and
equal datamodel answer tapes give equal results. -/
-- @tg: implements:: [[openspec:formal-model#Step Determinism Theorem]]
theorem step_deterministic (c : Chart) (_hwf : WellFormed c) (cc : CChart)
    (_hcc : compile c = .ok cc) (s₁ s₂ : Session) (i₁ i₂ : Input) (t₁ t₂ : Tape)
    (hs : s₁ = s₂) (hi : i₁ = i₂) (ht : t₁ = t₂) :
    step cc s₁ i₁ t₁ = step cc s₂ i₂ t₂ := by
  subst hs hi ht; rfl

/-- **Step determinism, componentwise.** Equal sessions, inputs and tapes give equal new
sessions, equal effect lists, equal traces, equal datamodel requests and equal remaining tapes
(and equal errors, by `step_deterministic`). -/
-- @tg: implements:: [[openspec:formal-model#Step Determinism Theorem]]
theorem step_deterministic_out (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s₁ s₂ : Session) (i₁ i₂ : Input) (t₁ t₂ : Tape)
    (hs : s₁ = s₂) (hi : i₁ = i₂) (ht : t₁ = t₂) (o₁ o₂ : Out cc)
    (h₁ : step cc s₁ i₁ t₁ = .ok o₁) (h₂ : step cc s₂ i₂ t₂ = .ok o₂) :
    o₁.session = o₂.session ∧ o₁.effects = o₂.effects ∧ o₁.trace = o₂.trace ∧
      o₁.requestList = o₂.requestList ∧ o₁.tape = o₂.tape := by
  rw [step_deterministic c hwf cc hcc s₁ s₂ i₁ i₂ t₁ t₂ hs hi ht, h₂] at h₁
  cases h₁
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- **Start determinism.** -/
-- @tg: implements:: [[openspec:formal-model#Step Determinism Theorem]]
theorem start_deterministic (cc : CChart) (si₁ si₂ : StartInput) (t₁ t₂ : Tape)
    (hs : si₁ = si₂) (ht : t₁ = t₂) : start cc si₁ t₁ = start cc si₂ t₂ := by
  subst hs ht; rfl

/-! ## Transition selection -/

theorem CChart.conflict_symm (cc : CChart) (config : List Nat) (hist : List (Nat × List Nat))
    (x y : Nat) : cc.conflict config hist x y = cc.conflict config hist y x := by
  unfold CChart.conflict
  apply Bool.eq_iff_iff.2
  simp only [List.any_eq_true, List.contains_iff_mem]
  constructor
  · rintro ⟨s, h1, h2⟩; exact ⟨s, h2, h1⟩
  · rintro ⟨s, h1, h2⟩; exact ⟨s, h2, h1⟩

/-- No conflict means disjoint exit sets. -/
theorem CChart.conflict_false (cc : CChart) (config : List Nat) (hist : List (Nat × List Nat))
    (x y : Nat) (h : cc.conflict config hist x y = false) :
    ∀ s ∈ cc.exitSetOf config hist x, s ∉ cc.exitSetOf config hist y := by
  intro s hx hy
  unfold CChart.conflict at h
  have : (cc.exitSetOf config hist x).any (fun s => (cc.exitSetOf config hist y).contains s)
      = true := List.any_eq_true.2 ⟨s, hx, by simpa using hy⟩
  rw [h] at this
  cases this

/-- The selection run by every microstep equals the pure selection of `Select` for the guard
answers the run observed, and leaves the interpreter state unchanged. -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem select_spec (cc : CChart) (ev : Option String) (core core' : Core)
    (ex ex' : ExecSt cc) (ts : List Nat)
    (h : ((Interp.select cc ev).run core).run ex = .ok ((ts, core'), ex')) :
    core' = core ∧ ∃ g : Nat → Nat → Bool,
      ts = Select.selectPure (core.config.filter cc.isAtomic) cc.candidates
        (Interp.matchP cc ev) g (cc.conflict core.config core.history) cc.srcDesc := by
  unfold Interp.select at h
  simp only [StateT.run_bind, StateT.run_get, StateT.run_lift, StateT.run_pure] at h
  simp only [bind, pure, Except.pure, Except.bind] at h
  generalize hc : (Select.choices cc.candidates
    (fun _ _ t => if Interp.matchP cc ev t then (Interp.guard cc t).run
      { config := core.config, sessionid := core.sessionid, options := core.options }
      else pure false) (core.config.filter cc.isAtomic) 0).run ex = r at h
  cases r with
  | error e => cases h
  | ok v =>
    rcases v with ⟨chs, ex₁⟩
    simp only at h
    cases h
    refine ⟨rfl, ?_⟩
    obtain ⟨g, hg⟩ := Select.choices_run cc.candidates (Interp.matchP cc ev)
      (fun _ _ t => (Interp.guard cc t).run
        { config := core.config, sessionid := core.sessionid, options := core.options })
      _ 0 ex _ chs hc
    exact ⟨g, by rw [hg]; rfl⟩

/-- **Transition selection.** In every microstep, the selected transition set is enabled (each
member is the first transition, on an active atomic state or its ancestors, that matches and
whose guard the datamodel answered true), conflict-free (the exit sets of two members are
disjoint), and optimal (every enabled transition is selected or preempted, through a chain, by a
selected one, by descendant priority or by document/selection order). -/
-- @tg: implements:: [[openspec:formal-model#Transition Selection Theorem]]
theorem transition_selection (cc : CChart) (ev : Option String) (core core' : Core)
    (ex ex' : ExecSt cc) (ts : List Nat)
    (h : ((Interp.select cc ev).run core).run ex = .ok ((ts, core'), ex')) :
    let atomics := core.config.filter cc.isAtomic
    let p := Interp.matchP cc ev
    let conflict := cc.conflict core.config core.history
    ∃ g : Nat → Nat → Bool,
      -- enabled
      (∀ t ∈ ts, ∃ i k, Select.EnabledAt atomics cc.candidates p g i k t ∧
        ∀ a k' t', atomics[i]? = some a → k' < k → (cc.candidates a)[k']? = some t' →
          (p t' && g i k') = false) ∧
      -- conflict-free
      (∀ x ∈ ts, ∀ y ∈ ts, x ≠ y →
        ∀ s ∈ cc.exitSetOf core.config core.history x,
          s ∉ cc.exitSetOf core.config core.history y) ∧
      -- optimal
      (∀ i k t, Select.EnabledAt atomics cc.candidates p g i k t →
        t ∈ ts ∨ ∃ t' ∈ ts,
          Relation.TransGen (Select.Preempts atomics cc.candidates p g conflict cc.srcDesc) t' t) := by
  obtain ⟨_, g, hg⟩ := select_spec cc ev core core' ex ex' ts h
  refine ⟨g, ?_, ?_, ?_⟩
  · intro t ht
    rw [hg] at ht
    exact Select.selected_enabled _ _ _ g _ _ t ht
  · intro x hx y hy hxy
    rw [hg] at hx hy
    apply cc.conflict_false
    exact Select.selected_conflictFree _ _ _ g _ cc.srcDesc (cc.conflict_symm _ _) x hx y hy hxy
  · intro i k t he
    rw [hg]
    exact Select.selected_optimal _ _ _ g _ _ i k t he

/-! ## Dynamic type soundness -/

theorem requests_ok {cc : CChart} (o : Out cc) : ∀ r ∈ o.requestList, LocOk cc r := by
  intro r hr
  unfold Out.requestList at hr
  obtain ⟨x, _, rfl⟩ := List.mem_map.1 hr
  exact x.2

/-- **Dynamic type soundness (M0).** For every well-formed chart, `step` never issues a
datamodel request that writes (`declare`, `assign_expr`, `assign_value`, `bind_var`) anything but
a data item the chart declares or a location expression written in the chart (an `assign`
location, a `send` or `invoke` `idlocation`, a `foreach` `item` or `index`). -/
-- @tg: implements:: [[openspec:formal-model#Type Soundness Statement]]
theorem step_requests_declared (c : Chart) (_hwf : WellFormed c) (cc : CChart)
    (_hcc : compile c = .ok cc) (s : Session) (i : Input) (tape : Tape) (o : Out cc)
    (_h : step cc s i tape = .ok o) :
    ∀ r ∈ o.requestList, ∀ l, r.writes = some l → l ∈ cc.locations :=
  requests_ok o

/-- **Dynamic type soundness (M0), session start.** -/
-- @tg: implements:: [[openspec:formal-model#Type Soundness Statement]]
theorem start_requests_declared (cc : CChart) (si : StartInput) (tape : Tape) (o : Out cc)
    (_h : start cc si tape = .ok o) :
    ∀ r ∈ o.requestList, ∀ l, r.writes = some l → l ∈ cc.locations :=
  requests_ok o

/-- What a type system supplies to the typed soundness theorem (delivered in M1): types, the
typing relation on values, the declared type of each location, and typing predicates for the
datamodel tape, the session and the inputs (which M1 derives from the chart's declarations). -/
structure TypeSystem where
  Ty : Type
  HasType : Value → Ty → Prop
  locType : String → Option Ty
  TapeTyped : Tape → Prop
  SessionTyped : Session → Prop
  InputTyped : Input → Prop

/-- **The M1 extension point: typed step soundness.** For a well-typed session, input and tape,
`step` writes only declared locations (the M0 statement), every value it assigns to a typed
location has that location's type, and the new session is well-typed. M1 proves this for the
Vertep type system (`HasType`, with the checker proven sound and complete); M0 proves the
instance for the dynamic type system (`dynamic_typed_soundness`). -/
-- @tg: implements:: [[openspec:formal-model#Type Soundness Statement]]
def TypedStepSoundness (cc : CChart) (T : TypeSystem) : Prop :=
  ∀ (s : Session) (i : Input) (tape : Tape) (o : Out cc),
    T.SessionTyped s → T.InputTyped i → T.TapeTyped tape → step cc s i tape = .ok o →
      (∀ r ∈ o.requestList, ∀ l, r.writes = some l → l ∈ cc.locations) ∧
      (∀ r ∈ o.requestList, ∀ l v τ, (r = .assignValue l v ∨ r = .bindVar l v) →
        T.locType l = some τ → T.HasType v τ) ∧
      T.SessionTyped o.session

/-- The dynamic type system of M0: one type, every value has it, every session, input and tape
is well-typed. -/
def dynamicTypes : TypeSystem where
  Ty := Unit
  HasType := fun _ _ => True
  locType := fun _ => some ()
  TapeTyped := fun _ => True
  SessionTyped := fun _ => True
  InputTyped := fun _ => True

/-- M0's dynamic soundness is the M1 statement instantiated with the dynamic type system. -/
-- @tg: implements:: [[openspec:formal-model#Type Soundness Statement]]
theorem dynamic_typed_soundness (cc : CChart) : TypedStepSoundness cc dynamicTypes := by
  intro s i tape o _ _ _ _
  exact ⟨requests_ok o, fun _ _ _ _ _ _ _ => trivial, trivial⟩

end Vertep
