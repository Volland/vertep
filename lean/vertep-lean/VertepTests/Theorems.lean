import VertepTests.Step

/-! # The M0 theorems, applied -/

namespace Vertep.Test

/-- The determinism theorem quantifies over every well-formed chart, session, input and tape:
it applies to arbitrary ones. -/
-- @tg: verifies:: [[openspec:formal-model#Step Determinism Theorem#Theorem stated over all inputs]] {level: unit}
theorem determinism_for_all (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (i : Input) (t : Tape) :
    step cc s i t = step cc s i t :=
  step_deterministic c hwf cc hcc s s i i t t rfl rfl rfl

/-- Its conclusion covers the effect list as well as the new session state. -/
-- @tg: verifies:: [[openspec:formal-model#Step Determinism Theorem#Effects included]] {level: unit}
theorem determinism_covers_effects (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (i : Input) (t : Tape) (o₁ o₂ : Out cc)
    (h₁ : step cc s i t = .ok o₁) (h₂ : step cc s i t = .ok o₂) :
    o₁.effects = o₂.effects ∧ o₁.session = o₂.session :=
  let h := step_deterministic_out c hwf cc hcc s s i i t t rfl rfl rfl o₁ o₂ h₁ h₂
  ⟨h.2.1, h.1⟩

/-- Applied to any selection a microstep makes, the theorem gives disjoint exit sets for two
chosen transitions (in particular whenever neither source is an ancestor of the other). -/
-- @tg: verifies:: [[openspec:formal-model#Transition Selection Theorem#Conflict-free set]] {level: unit}
theorem selection_conflict_free (cc : CChart) (ev : Option String) (core core' : Core)
    (ex ex' : ExecSt cc) (ts : List Nat)
    (h : ((Interp.select cc ev).run core).run ex = .ok ((ts, core'), ex'))
    (x y : Nat) (hx : x ∈ ts) (hy : y ∈ ts) (hxy : x ≠ y) :
    ∀ s ∈ cc.exitSetOf core.config core.history x, s ∉ cc.exitSetOf core.config core.history y := by
  obtain ⟨_, _, hcf, _⟩ := transition_selection cc ev core core' ex ex' ts h
  exact hcf x hx y hy hxy

/-- Applied to an omitted enabled transition, the theorem gives a chosen transition that
preempts it by descendant priority or by document order. -/
-- @tg: verifies:: [[openspec:formal-model#Transition Selection Theorem#Priority respected]] {level: unit}
theorem selection_priority (cc : CChart) (ev : Option String) (core core' : Core)
    (ex ex' : ExecSt cc) (ts : List Nat)
    (h : ((Interp.select cc ev).run core).run ex = .ok ((ts, core'), ex')) :
    ∃ g, ∀ i k t, Select.EnabledAt (core.config.filter cc.isAtomic) cc.candidates
        (Interp.matchP cc ev) g i k t → t ∉ ts →
      ∃ t' ∈ ts, Relation.TransGen (Select.Preempts (core.config.filter cc.isAtomic)
        cc.candidates (Interp.matchP cc ev) g (cc.conflict core.config core.history)
        cc.srcDesc) t' t := by
  obtain ⟨g, _, _, hopt⟩ := transition_selection cc ev core core' ex ex' ts h
  refine ⟨g, fun i k t he hn => ?_⟩
  rcases hopt i k t he with hm | hp
  · exact absurd hm hn
  · exact hp

/-- Applied to a well-formed chart, the theorem gives that every assignment request names a
declared data item or location expression; checked on a run of `ecmaChart`, too. -/
-- @tg: verifies:: [[openspec:formal-model#Type Soundness Statement#No undeclared locations]] {level: unit}
theorem no_undeclared_locations (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (i : Input) (t : Tape) (o : Out cc)
    (h : step cc s i t = .ok o) (r : Request) (hr : r ∈ o.requestList) (l : String)
    (hw : r.writes = some l) : l ∈ (cc).locations :=
  step_requests_declared c hwf cc hcc s i t o h r hr l hw

#guard match runEvents ecmaChart ["go"] ecmaTape with
  | .ok outs => outs.all (fun o => o.requestList.all (fun r => match r.writes with
      | some l => (cc ecmaChart).locations.contains l
      | none => true))
  | .error _ => false

/-- The named M1 extension point exists and M0 proves its dynamic instance. -/
-- @tg: verifies:: [[openspec:formal-model#Type Soundness Statement#Extension point present]] {level: unit}
theorem extension_point_present (cc : CChart) : TypedStepSoundness cc dynamicTypes :=
  dynamic_typed_soundness cc

/-- Applied to any step of a well-formed chart, the Configuration Well-Formedness Theorem leaves
no compound state with two active children. -/
-- @tg: verifies:: [[openspec:formal-model#Configuration Well-Formedness Theorem#Compound exclusivity]] {level: unit}
theorem step_compound_exclusive (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (hs : Reachable cc s) (i : Input) (t : Tape)
    (o : Out cc) (h : step cc s i t = .ok o) (ht : o.session.terminated = false)
    (x : Nat) (hx : x ∈ o.session.config) (hc : cc.isCompound x = true)
    (a b : Nat) (ha : a ∈ (cc.st x).children) (hb : b ∈ (cc.st x).children)
    (sa : a ∈ o.session.config) (sb : b ∈ o.session.config) : a = b :=
  compound_exclusivity c hwf cc hcc o.session (.step s i t o hs h) ht x hx hc a ha b hb sa sb

/-- Every region of a parallel state active after a step is active. -/
-- @tg: verifies:: [[openspec:formal-model#Configuration Well-Formedness Theorem#Parallel completeness]] {level: unit}
theorem step_parallel_complete (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (hs : Reachable cc s) (i : Input) (t : Tape)
    (o : Out cc) (h : step cc s i t = .ok o) (ht : o.session.terminated = false)
    (x : Nat) (hx : x ∈ o.session.config) (hp : cc.isParallel x = true)
    (r : Nat) (hr : r ∈ (cc.st x).children) : r ∈ o.session.config :=
  parallel_completeness c hwf cc hcc o.session (.step s i t o hs h) ht x hx hp r hr

end Vertep.Test
