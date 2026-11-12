import VertepLean.Config.Interp
import VertepLean.Config.Compile
import VertepLean.Config.Entry

/-!
# The Configuration Well-Formedness Theorem

Every running session reachable by `start` and `step` from a well-formed chart has a legal
configuration (W3C SCXML 3.11). `reachable_inv` carries the interpreter invariant through
`start` and `step` from the parts of `VertepLean/Config/`: compile correctness (`compile_cwf`),
the exit lemma (`exit_legal`), fuel adequacy (`eff_adequate`), the entry at start
(`start_entry`) and the entry lemma (`entry_legal`). `compound_exclusivity` and
`parallel_completeness` are its two scenarios.
-/

namespace Vertep

/-- Sessions reachable from `start` by `step`. -/
inductive Reachable (cc : CChart) : Session → Prop
  | start (si : StartInput) (tape : Tape) (o : Out cc) :
      Vertep.start cc si tape = .ok o → Reachable cc o.session
  | step (s : Session) (i : Input) (tape : Tape) (o : Out cc) :
      Reachable cc s → Vertep.step cc s i tape = .ok o → Reachable cc o.session

/-- **Configuration Well-Formedness Theorem (statement).** For every well-formed chart, every
running session reachable by `start` and `step` has a legal configuration. -/
def ConfigurationWellFormedness : Prop :=
  ∀ c : Chart, WellFormed c → ∀ cc, compile c = .ok cc →
    ∀ s, Reachable cc s → s.terminated = false → Legal cc s.config

namespace Config

/-- The invariant on sessions. -/
structure SessInv (cc : CChart) (s : Session) : Prop where
  sorted : s.config.Pairwise (· < ·)
  hist : HistOk cc s.history
  legal : s.terminated = false → cc.Below 0 s.config

/-- The core an interpreter run starts from. -/
def startCore (s : Session) (sessions : Option (List String)) : Core :=
  { sessionid := s.sessionid, terminated := s.terminated, config := s.config,
    history := s.history, invocations := s.invocations, initialized := s.initialized,
    macrosteps := s.macrosteps, options := s.options, toInvoke := [], running := true,
    used := 0, sessions := sessions, micros := #[], terminatedNow := false }

theorem run_inv {cc : CChart} (input : TraceInput) (body : M cc Unit) (s : Session) (tape : Tape)
    (sessions : Option (List String)) (P : Core → Prop)
    (hb : Hoare P body (fun _ c => Inv cc c)) (hp : P (startCore s sessions)) (o : Out cc)
    (h : Interp.run cc input body s tape sessions = .ok o) : SessInv cc o.session := by
  unfold Interp.run Interp.runFull at h
  dsimp only at h
  generalize hr : StateT.run (StateT.run body _) _ = r at h
  cases r with
  | error e => cases h
  | ok v =>
    rcases v with ⟨⟨u, core⟩, ex⟩
    have hi := hb _ _ u core ex hp hr
    cases hx : core.exhausted
    · simp only [hx] at h
      cases h
      exact ⟨hi.sorted, hi.hist, hi.legal⟩
    · simp only [hx] at h
      cases h

variable {cc : CChart}

theorem startBody_spec (hw : CWF cc) (hEnt : EntryLegal) (si : StartInput) :
    Hoare (cc := cc) (Fr [] [] false) (Interp.startBody cc si) (fun _ c => Inv cc c) := by
  unfold Interp.startBody
  refine hoare_bind (Frame.liftE _ [] [] false) (fun r => ?_)
  extract_lets jp
  have hjp : ∀ u, Hoare (cc := cc) (Fr [] [] false) (jp u) (fun _ c => Inv cc c) := fun _ =>
    hoare_bind (Frame.liftE _ [] [] false) (fun _ => runStart_spec hw hEnt _)
  split
  · exact hjp ()
  · exact hoare_bind (hoare_throw _) hjp
  · exact hoare_bind (hoare_throw _) hjp

theorem stepBody_spec (hw : CWF cc) (hEnt : EntryLegal) (i : Input) :
    Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (Interp.stepBody cc i)
      (fun _ c => Inv cc c) := by
  unfold Interp.stepBody
  refine hoare_bind ((Frame.liftE _).inv _ inv_keeps) (fun _ => ?_)
  split
  · exact hoare_conseq (exitInterpreter_inv _) (fun _ p => p.1) (fun _ _ p => p)
  · exact runExternal_spec hw hEnt _

/-- The interpreter invariant holds on every reachable session. -/
theorem reachable_inv (hw : CWF cc) (hEnt : EntryLegal) (s : Session) (hs : Reachable cc s) :
    SessInv cc s := by
  induction hs with
  | start si tape o h =>
    unfold Vertep.start at h
    exact run_inv _ _ _ _ _ _ (startBody_spec hw hEnt si) ⟨rfl, rfl, rfl⟩ o h
  | step s i tape o _ h ih =>
    unfold Vertep.step at h
    cases ht : s.terminated
    · rw [ht] at h
      simp only [Bool.false_eq_true, ↓reduceIte] at h
      exact run_inv _ _ _ _ _ _ (stepBody_spec hw hEnt i)
        ⟨⟨ih.sorted, ih.hist, ih.legal⟩, ht⟩ o h
    · rw [ht] at h; cases h

end Config

/-- **Configuration Well-Formedness**, from compile correctness and the entry lemma. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem configuration_wellformedness_of (hc : CompileCWF) (hEnt : EntryLegal) :
    ConfigurationWellFormedness := by
  intro c hwf cc hcc s hs ht
  have hw := hc c cc hwf hcc
  exact (CWF.legal_iff_below hw s.config).2 ((Config.reachable_inv hw hEnt s hs).legal ht)

/-- **Configuration Well-Formedness**, from the entry lemma (compile correctness is proven). -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem configuration_wellformedness_of_entry (hEnt : EntryLegal) :
    ConfigurationWellFormedness :=
  configuration_wellformedness_of compile_cwf hEnt

/-- **Configuration Well-Formedness Theorem.** For every well-formed chart, every running session
reachable by `start` and `step` has a legal configuration (W3C SCXML 3.11). -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem configuration_wellformedness : ConfigurationWellFormedness :=
  configuration_wellformedness_of compile_cwf entry_legal

/-- Scenario *Compound exclusivity*: no active compound state has two active children. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem compound_exclusivity (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (hs : Reachable cc s) (ht : s.terminated = false) :
    ∀ x ∈ s.config, cc.isCompound x = true →
      ∀ a ∈ (cc.st x).children, ∀ b ∈ (cc.st x).children, a ∈ s.config → b ∈ s.config → a = b := by
  intro x hx hc a ha b hb sa sb
  have hw := compile_cwf c cc hwf hcc
  have hb' := (CWF.legal_iff_below hw s.config).1 (configuration_wellformedness c hwf cc hcc s hs ht)
  exact (hb'.compound x hx hc).2 a ha b hb sa sb

/-- Scenario *Parallel completeness*: every region of an active parallel state is active. -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]]
theorem parallel_completeness (c : Chart) (hwf : WellFormed c) (cc : CChart)
    (hcc : compile c = .ok cc) (s : Session) (hs : Reachable cc s) (ht : s.terminated = false) :
    ∀ x ∈ s.config, cc.isParallel x = true → ∀ r ∈ (cc.st x).children, r ∈ s.config := by
  intro x hx hp r hr
  have hw := compile_cwf c cc hwf hcc
  have hb' := (CWF.legal_iff_below hw s.config).1 (configuration_wellformedness c hwf cc hcc s hs ht)
  exact hb'.parallel x hx hp r hr

end Vertep
