import VertepLean.Config.Hoare
import VertepLean.Theorems
import VertepLean.Config.Exit

/-!
# The interpreter keeps the configuration in its frame

Most interpreter actions never touch the configuration, the history or the termination flag
(`Frame`); this file proves it for every action except `exitStates`, `enterStates` and
`exitInterpreter`, and gives those three their exact effect.
-/

namespace Vertep.Config

variable {cc : CChart}

/-- `x` keeps the frame when started from `core`. -/
def FrameFrom {α : Type} (core : Core) (x : M cc α) : Prop :=
  ∀ c h t, Hoare (fun c' => Fr c h t c' ∧ c' = core) x (fun _ => Fr c h t)

theorem FrameFrom.of {α : Type} {core : Core} {x : M cc α} (hx : Frame x) : FrameFrom core x :=
  fun c h t => hoare_conseq (hx c h t) (fun _ p => p.1) (fun _ _ p => p)

theorem Frame.get_bind {α : Type} {f : Core → M cc α} (hf : ∀ core, FrameFrom core (f core)) :
    Frame (get >>= f) :=
  fun c h t => hoare_bind hoare_get (fun core => hoare_conseq (hf core c h t)
    (fun _ p => ⟨p.1, p.2.symm⟩) (fun _ _ p => p))

/-- Keeping the frame of a core. -/
def Keeps (core core' : Core) : Prop :=
  core'.config = core.config ∧ core'.history = core.history ∧ core'.terminated = core.terminated

theorem FrameFrom.set_bind {α : Type} {core X : Core} {k : PUnit → M cc α} (hX : Keeps core X)
    (hk : ∀ u, Frame (k u)) : FrameFrom core (set X >>= k) := by
  intro c h t
  refine hoare_bind (Q := fun _ c' => Fr c h t c') ?_ (fun u => hk u c h t)
  intro c0 ex a c' ex' hpre hrun
  obtain ⟨hfr, rfl⟩ := hpre
  have := hoare_set (P := fun _ => True) X c0 ex a c' ex' trivial hrun
  subst this
  obtain ⟨e1, e2, e3⟩ := hX
  obtain ⟨h1, h2, h3⟩ := hfr
  exact ⟨e1.trans h1, e2.trans h2, e3.trans h3⟩

theorem FrameFrom.ite {α : Type} {core : Core} (b : Prop) [Decidable b] {x y : M cc α}
    (hx : b → FrameFrom core x) (hy : ¬b → FrameFrom core y) : FrameFrom core (if b then x else y) :=
  fun c h t => hoare_ite b (fun hb => hx hb c h t) (fun hb => hy hb c h t)


/-! ## Frame of the interpreter actions -/

theorem frame_runBlocks (bs : List Block) (h : ∀ b ∈ bs, ∀ l ∈ blockLocs b, l ∈ cc.locations) :
    Frame (Interp.runBlocks cc bs h) := by
  unfold Interp.runBlocks
  exact Frame.forM _ _ (fun ⟨b, hb⟩ => Frame.liftE _)

theorem frame_setEvent (e : Event) : Frame (Interp.setEvent cc e) := by
  unfold Interp.setEvent
  exact Frame.bind (Frame.liftE _) (fun _ => Frame.pure _)

theorem frame_record (ev : Option String) (ts : List Nat) : Frame (Interp.record cc ev ts) := by
  unfold Interp.record
  exact Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩)

theorem frame_runTrans (t : Nat) : Frame (Interp.runTrans cc t) := by
  unfold Interp.runTrans; exact Frame.liftE _

theorem frame_cancelInvocations (s : Nat) : Frame (Interp.cancelInvocations cc s) := by
  unfold Interp.cancelInvocations
  refine Frame.get_bind (fun core => ?_)
  exact FrameFrom.set_bind ⟨rfl, rfl, rfl⟩ (fun _ => Frame.forM _ _ (fun i =>
    Frame.ite _ (fun _ => Frame.pure _) (fun _ => Frame.liftE _)))

theorem frame_charge : Frame (Interp.charge cc) := by
  unfold Interp.charge
  refine Frame.get_bind (fun core => ?_)
  refine FrameFrom.ite _ (fun _ => FrameFrom.set_bind ⟨rfl, rfl, rfl⟩ (fun _ => Frame.pure _))
    (fun _ => FrameFrom.set_bind ⟨rfl, rfl, rfl⟩ (fun _ => Frame.pure _))

theorem frame_startInvokes : Frame (Interp.startInvokes cc) := by
  unfold Interp.startInvokes
  refine Frame.get_bind (fun core => FrameFrom.of ?_)
  refine Frame.bind (Frame.forIn _ _ _ (fun s _ => ?_)) (fun _ => Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩))
  refine Frame.bind (Frame.forIn _ _ _ (fun x _ => ?_)) (fun _ => Frame.pure _)
  rcases x with ⟨v, k⟩
  refine Frame.bind (Frame.liftE _) (fun r => ?_)
  cases r with
  | some inv => exact Frame.bind (Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩)) (fun _ => Frame.pure _)
  | none => exact Frame.bind (Frame.pure _) (fun _ => Frame.pure _)

theorem frame_dequeue : Frame (Interp.dequeue cc) := by
  unfold Interp.dequeue
  refine Frame.bind (Frame.lift (get : SM cc (ExecSt cc))) (fun st => ?_)
  split
  · exact Frame.pure _
  · rename_i e rest _
    exact Frame.bind (x := (StateT.lift (set (σ := ExecSt cc) { st with internal := rest } :
      SM cc PUnit) : M cc PUnit)) (Frame.lift _) (fun _ => Frame.pure _)

/-! ## Selection -/

theorem hoare_select (P : Core → Prop) (ev : Option String) :
    Hoare (cc := cc) P (Interp.select cc ev)
      (fun ts c => P c ∧ Selected cc c.config c.history ts) := by
  intro core ex ts core' ex' hP hrun
  obtain ⟨hc, g, hg⟩ := select_spec cc ev core core' ex ex' ts hrun
  obtain ⟨g', hen, hdis, _⟩ := transition_selection cc ev core core' ex ex' ts hrun
  subst hc
  refine ⟨hP, ⟨?_, ?_, hdis⟩⟩
  · rw [hg]; exact Select.selected_nodup _ _ _ _ _ _
  · intro t ht
    obtain ⟨i, k, ⟨a, ha, hk, _⟩, _⟩ := hen t ht
    have ha' := List.mem_of_getElem? ha
    rw [List.mem_filter] at ha'
    exact ⟨a, ha'.1, ha'.2, List.mem_of_getElem? hk⟩

/-! ## Exit -/

/-- The frame with a fixed configuration, history and termination flag. -/
theorem recordHistory_spec (c : List Nat) (t : Bool) (ex : List Nat) (H : List (Nat × List Nat)) :
    Hoare (cc := cc) (Fr c H t) (Interp.recordHistory cc ex) (fun _ => Fr c (cc.recordPure c ex H) t) := by
  unfold Interp.recordHistory CChart.recordPure
  refine hoare_forM_fold (fun H => Fr c H t) _ ex H _ (fun s H => ?_)
  refine hoare_forM_fold (fun H => Fr c H t) _ _ H _ (fun h H => ?_)
  refine hoare_bind hoare_get (fun core => ?_)
  intro c0 e a c' e' hpre hrun
  obtain ⟨⟨h1, h2, h3⟩, rfl⟩ := hpre
  have := hoare_set (P := fun _ => True) _ core e a c' e' trivial hrun
  subst this
  refine ⟨h1, ?_, h3⟩
  simp only [h1, h2]

theorem exitStates_spec (c : List Nat) (H : List (Nat × List Nat)) (t : Bool) (ts : List Nat) :
    Hoare (cc := cc) (Fr c H t) (Interp.exitStates cc ts)
      (fun _ => Fr ((cc.exitSet c H ts).reverse.foldl List.erase c)
        (cc.recordPure c (cc.exitSet c H ts) H) t) := by
  unfold Interp.exitStates
  refine hoare_bind hoare_get (fun core => ?_)
  intro c0 e a c' e' hpre hrun
  obtain ⟨⟨h1, h2, h3⟩, rfl⟩ := hpre
  rw [h1, h2] at hrun
  refine (hoare_bind (P := Fr c H t) (Q := fun _ => Fr c H t)
    (R := fun _ => Fr ((cc.exitSet c H ts).reverse.foldl List.erase c)
      (cc.recordPure c (cc.exitSet c H ts) H) t) ?_ (fun _ => ?_)) core e a c' e' ⟨h1, h2, h3⟩ hrun
  · intro c1 e1 a1 c2 e2 _ hrun1
    have := hoare_set (P := fun _ => True) _ c1 e1 a1 c2 e2 trivial hrun1
    subst this
    exact ⟨rfl, rfl, h3⟩
  refine hoare_bind (recordHistory_spec c t _ H) (fun _ => ?_)
  refine hoare_bind (hoare_forIn_fold (fun c' => Fr c' (cc.recordPure c (cc.exitSet c H ts) H) t)
    (fun c s => c.erase s) _ _ c _ (fun s _ c' => ?_)) (fun _ => hoare_pure _)
  refine hoare_bind (frame_runBlocks _ _ _ _ _) (fun _ => hoare_bind (frame_cancelInvocations _ _ _ _)
    (fun _ => ?_))
  refine hoare_bind (hoare_modify _) (fun _ => ?_)
  intro c1 e1 a1 c2 e2 hpre hrun1
  obtain ⟨c3, ⟨k1, k2, k3⟩, rfl⟩ := hpre
  cases hrun1
  exact ⟨⟨_, rfl⟩, by simp only [k1], k2, k3⟩

/-! ## Entry -/

theorem hoare_ret {α : Type} {P : Core → Prop} (a : α) :
    Hoare (cc := cc) P (Pure.pure a) (fun r c => r = a ∧ P c) := by
  intro core ex b core' ex' hP h
  cases h; exact ⟨rfl, hP⟩

/-- `x` keeps the frame and returns a value satisfying `V`. -/
def FrameV {α : Type} (V : α → Prop) (x : M cc α) : Prop :=
  ∀ c h t, Hoare (Fr c h t) x (fun r c' => V r ∧ Fr c h t c')

/-- `FrameV` from a known starting core. -/
def FrameVFrom {α : Type} (V : α → Prop) (core : Core) (x : M cc α) : Prop :=
  ∀ c h t, Hoare (fun c' => Fr c h t c' ∧ c' = core) x (fun r c' => V r ∧ Fr c h t c')

theorem FrameV.bind {α β : Type} {V : β → Prop} {x : M cc α} {f : α → M cc β} (hx : Frame x)
    (hf : ∀ a, FrameV V (f a)) : FrameV V (x >>= f) :=
  fun c h t => hoare_bind (hx c h t) (fun a => hf a c h t)

theorem FrameV.ret {α : Type} {V : α → Prop} {a : α} (ha : V a) : FrameV (cc := cc) V (Pure.pure a) :=
  fun _ _ _ => hoare_conseq (hoare_ret a) (fun _ p => p) (fun _ _ p => ⟨p.1 ▸ ha, p.2⟩)

theorem FrameV.ite {α : Type} {V : α → Prop} (b : Prop) [Decidable b] {x y : M cc α}
    (hx : b → FrameV V x) (hy : ¬b → FrameV V y) : FrameV V (if b then x else y) :=
  fun c h t => hoare_ite b (fun hb => hx hb c h t) (fun hb => hy hb c h t)

theorem FrameVFrom.of {α : Type} {V : α → Prop} {core : Core} {x : M cc α} (hx : FrameV V x) :
    FrameVFrom V core x :=
  fun c h t => hoare_conseq (hx c h t) (fun _ p => p.1) (fun _ _ p => p)

theorem FrameV.get_bind {α : Type} {V : α → Prop} {f : Core → M cc α}
    (hf : ∀ core, FrameVFrom V core (f core)) : FrameV V (get >>= f) :=
  fun c h t => hoare_bind hoare_get (fun core => hoare_conseq (hf core c h t)
    (fun _ p => ⟨p.1, p.2.symm⟩) (fun _ _ p => p))

theorem FrameVFrom.ite {α : Type} {V : α → Prop} {core : Core} (b : Prop) [Decidable b]
    {x y : M cc α} (hx : b → FrameVFrom V core x) (hy : ¬b → FrameVFrom V core y) :
    FrameVFrom V core (if b then x else y) :=
  fun c h t => hoare_ite b (fun hb => hx hb c h t) (fun hb => hy hb c h t)

theorem FrameVFrom.set_bind {α : Type} {V : α → Prop} {core X : Core} {k : PUnit → M cc α}
    (hX : Keeps core X) (hk : ∀ u, FrameV V (k u)) : FrameVFrom V core (set X >>= k) := by
  intro c h t
  refine hoare_bind (Q := fun _ c' => Fr c h t c') ?_ (fun u => hk u c h t)
  intro c0 ex a c' ex' hpre hrun
  obtain ⟨hfr, rfl⟩ := hpre
  have := hoare_set (P := fun _ => True) X c0 ex a c' ex' trivial hrun
  subst this
  obtain ⟨e1, e2, e3⟩ := hX
  obtain ⟨h1, h2, h3⟩ := hfr
  exact ⟨e1.trans h1, e2.trans h2, e3.trans h3⟩

/-- Closes `Frame` goals for actions built from frame-keeping parts. -/
macro "frame_tac" : tactic => `(tactic| repeat (first
  | exact Frame.liftE _ | exact Frame.pure _ | exact frame_runBlocks _ _ | exact frame_runTrans _
  | exact Frame.throw _
  | exact Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩)
  | (refine Frame.get_bind (fun _ => ?_); first
      | (refine FrameFrom.ite _ (fun _ => FrameFrom.set_bind ⟨rfl, rfl, rfl⟩ (fun _ => ?_))
          (fun _ => FrameFrom.of ?_))
      | (refine FrameFrom.set_bind ⟨rfl, rfl, rfl⟩ (fun _ => ?_))
      | refine FrameFrom.of ?_)
  | refine Frame.bind ?_ (fun _ => ?_)
  | refine Frame.ite _ (fun _ => ?_) (fun _ => ?_)
  | split))

theorem enterStates_spec (c : List Nat) (H dh : List (Nat × List Nat)) (t : Bool)
    (ts : List Nat) :
    Hoare (cc := cc) (Fr c H t) (Interp.enterStates cc dh ts)
      (fun _ core => ∃ acc, cc.entrySet H dh ts = some acc ∧
        Fr ((acc.toEnter.mergeSort (· ≤ ·)).foldl (fun c s => Interp.insertSorted s c) c) H t core) := by
  unfold Interp.enterStates
  refine hoare_bind hoare_get (fun core => ?_)
  intro c0 e a c' e' hpre hrun
  obtain ⟨⟨h1, h2, h3⟩, rfl⟩ := hpre
  rw [h2] at hrun
  revert hrun
  cases hacc : cc.entrySet H dh ts with
  | none =>
    intro hrun
    exact absurd hrun (by intro h; cases h)
  | some acc =>
    intro hrun
    refine ⟨acc, rfl, ?_⟩
    refine (hoare_bind (P := Fr c H t)
      (Q := fun _ => Fr ((acc.toEnter.mergeSort (· ≤ ·)).foldl (fun c s => Interp.insertSorted s c) c) H t)
      (hoare_forIn_fold (fun c' => Fr c' H t) (fun c s => Interp.insertSorted s c) _ _ c _
        (fun s _ c' => ?_)) (fun _ => hoare_pure _)) core e a c' e' ⟨h1, h2, h3⟩ hrun
    refine hoare_bind (Q := fun _ => Fr (Interp.insertSorted s c') H t) ?_ (fun _ => ?_)
    · refine hoare_conseq (hoare_modify _) (fun _ p => p) ?_
      rintro _ _ ⟨c3, ⟨k1, k2, k3⟩, rfl⟩
      exact ⟨by simp only [k1], k2, k3⟩
    · refine (?_ : FrameV (cc := cc) (fun r : ForInStep PUnit => ∃ b', r = ForInStep.yield b') _)
        _ H t
      refine FrameV.get_bind (fun core => ?_)
      extract_lets parent jpF jpH jp1
      have hF : ∀ u, FrameV (fun r : ForInStep PUnit => ∃ b', r = ForInStep.yield b') (jpF u) := by
        intro u
        refine FrameV.ite _ (fun _ => ?_) (fun _ => FrameV.ret ⟨_, rfl⟩)
        refine FrameV.ite _ (fun _ => ?_) (fun _ => ?_)
        · exact FrameV.bind (Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩)) (fun _ => FrameV.ret ⟨_, rfl⟩)
        · refine FrameV.bind (Frame.liftE _) (fun _ => FrameV.bind (Frame.liftE _) (fun _ => ?_))
          split
          · refine FrameV.get_bind (fun _ => FrameVFrom.of ?_)
            exact FrameV.ite _ (fun _ => FrameV.bind (Frame.liftE _) (fun _ => FrameV.ret ⟨_, rfl⟩))
              (fun _ => FrameV.ret ⟨_, rfl⟩)
          · exact FrameV.ret ⟨_, rfl⟩
      have hH : ∀ u, FrameV (fun r : ForInStep PUnit => ∃ b', r = ForInStep.yield b') (jpH u) := by
        intro u
        dsimp only [jpH]
        split
        · exact FrameV.bind (frame_runTrans _) (fun x => hF x)
        · exact hF ()
      have hjp1 : ∀ u, FrameV (fun r : ForInStep PUnit => ∃ b', r = ForInStep.yield b') (jp1 u) := by
        intro u
        refine FrameV.bind (frame_runBlocks _ _) (fun _ => ?_)
        refine FrameV.ite _ (fun _ => ?_) (fun _ => hH ())
        split
        · exact FrameV.bind (frame_runTrans _) (fun x => hH x)
        · exact hH ()
      refine FrameVFrom.ite _ (fun _ => FrameVFrom.set_bind ⟨rfl, rfl, rfl⟩
        (fun _ => FrameV.bind (Frame.liftE _) (fun x => hjp1 x))) (fun _ => FrameVFrom.of (hjp1 ()))

/-! ## Lists -/

theorem mem_insertSorted (s x : Nat) : ∀ l : List Nat, x ∈ Interp.insertSorted s l ↔ x = s ∨ x ∈ l
  | [] => by simp [Interp.insertSorted]
  | y :: ys => by
    unfold Interp.insertSorted
    by_cases h1 : s < y
    · simp [h1]
    · by_cases h2 : s = y
      · subst h2; simp
      · have : (s == y) = false := by simp [h2]
        simp only [h1, ↓reduceIte, this, Bool.false_eq_true, List.mem_cons, mem_insertSorted s x ys]
        exact or_left_comm

theorem sorted_insertSorted (s : Nat) : ∀ l : List Nat, l.Pairwise (· < ·) →
    (Interp.insertSorted s l).Pairwise (· < ·)
  | [] => by simp [Interp.insertSorted]
  | y :: ys => by
    intro h
    rw [List.pairwise_cons] at h
    unfold Interp.insertSorted
    by_cases h1 : s < y
    · simp only [h1, ↓reduceIte]
      refine List.Pairwise.cons (fun z hz => ?_) (List.Pairwise.cons h.1 h.2)
      rcases List.mem_cons.1 hz with rfl | hz
      · exact h1
      · exact Nat.lt_trans h1 (h.1 z hz)
    · by_cases h2 : s = y
      · subst h2; simp only [Nat.lt_irrefl, ↓reduceIte, beq_self_eq_true, ↓reduceIte]
        exact List.Pairwise.cons h.1 h.2
      · have : (s == y) = false := by simp [h2]
        simp only [h1, ↓reduceIte, this, Bool.false_eq_true]
        refine List.Pairwise.cons (fun z hz => ?_) (sorted_insertSorted s ys h.2)
        rcases (mem_insertSorted s z ys).1 hz with rfl | hz
        · omega
        · exact h.1 z hz

theorem mem_foldl_insertSorted (l c : List Nat) (x : Nat) :
    x ∈ l.foldl (fun c s => Interp.insertSorted s c) c ↔ x ∈ c ∨ x ∈ l := by
  induction l generalizing c with
  | nil => simp
  | cons y ys ih => rw [List.foldl_cons, ih, mem_insertSorted]; simp only [List.mem_cons]; grind

theorem sorted_foldl_insertSorted (l c : List Nat) (h : c.Pairwise (· < ·)) :
    (l.foldl (fun c s => Interp.insertSorted s c) c).Pairwise (· < ·) := by
  induction l generalizing c with
  | nil => exact h
  | cons y ys ih => exact ih _ (sorted_insertSorted y c h)

theorem sorted_foldl_erase (l c : List Nat) (h : c.Pairwise (· < ·)) :
    (l.foldl List.erase c).Pairwise (· < ·) := by
  induction l generalizing c with
  | nil => exact h
  | cons y ys ih => exact ih _ (h.sublist (List.erase_sublist))

theorem mem_foldl_erase (l c : List Nat) (h : c.Nodup) (x : Nat) :
    x ∈ l.foldl List.erase c ↔ x ∈ c ∧ x ∉ l := by
  induction l generalizing c with
  | nil => simp
  | cons y ys ih =>
    rw [List.foldl_cons, ih _ (h.erase y), List.Nodup.mem_erase_iff h]
    simp only [List.mem_cons, not_or]
    constructor
    · rintro ⟨⟨h1, h2⟩, h3⟩; exact ⟨h2, h1, h3⟩
    · rintro ⟨h2, h1, h3⟩; exact ⟨⟨h1, h2⟩, h3⟩

/-! ## Invariants of the frame -/

/-- A frame-keeping action preserves every property of configuration, history and
termination flag. -/
theorem Frame.inv {α : Type} {x : M cc α} (hx : Frame x) (I : Core → Prop)
    (hI : ∀ c c', Keeps c c' → I c → I c') : Hoare I x (fun _ => I) := by
  intro core ex a core' ex' hP hrun
  obtain ⟨e1, e2, e3⟩ := hx core.config core.history core.terminated core ex a core' ex'
    ⟨rfl, rfl, rfl⟩ hrun
  exact hI core core' ⟨e1, e2, e3⟩ hP

theorem exitInterpreter_spec (H : List (Nat × List Nat)) (b : Bool) :
    Hoare (cc := cc) (fun c => c.config.Pairwise (· < ·) ∧ c.history = H)
      (Interp.exitInterpreter cc b)
      (fun _ c => c.config.Pairwise (· < ·) ∧ c.history = H ∧ c.terminated = true) := by
  let I : Core → Prop := fun c => c.config.Pairwise (· < ·) ∧ c.history = H
  have hI : ∀ c c', Keeps c c' → I c → I c' := by
    rintro c c' ⟨e1, e2, _⟩ ⟨h1, h2⟩; exact ⟨e1 ▸ h1, e2 ▸ h2⟩
  unfold Interp.exitInterpreter
  refine hoare_bind (Q := fun _ => I) (hoare_conseq hoare_get (fun _ p => p) (fun _ _ p => p.1))
    (fun core => ?_)
  refine hoare_bind (hoare_forIn _ _ _ (fun s d => ?_)) (fun _ => ?_)
  · refine hoare_bind ((frame_runBlocks _ _).inv I hI) (fun _ => ?_)
    refine hoare_bind ((frame_cancelInvocations _).inv I hI) (fun _ => ?_)
    refine hoare_bind (Q := fun _ => I) ?_ (fun _ => ?_)
    · refine hoare_conseq (hoare_modify _) (fun _ p => p) ?_
      rintro _ _ ⟨c, ⟨h1, h2⟩, rfl⟩
      exact ⟨h1.sublist List.erase_sublist, h2⟩
    · refine hoare_ite _ (fun _ => ?_) (fun _ => hoare_pure _)
      exact hoare_bind ((Frame.liftE _).inv I hI) (fun _ => hoare_pure _)
  · refine hoare_bind ((Frame.liftE _).inv I hI) (fun _ => ?_)
    refine hoare_bind ((Frame.lift (modify (σ := ExecSt cc) (fun st => { st with internal := [] }) :
      SM cc PUnit)).inv I hI) (fun _ => ?_)
    refine hoare_conseq (hoare_modify _) (fun _ p => p) ?_
    rintro _ _ ⟨c, ⟨h1, h2⟩, rfl⟩
    exact ⟨h1, h2, rfl⟩

/-! ## The microstep keeps the configuration legal -/

/-- The interpreter invariant: the configuration is sorted, history values are legal, and while
the session runs its configuration is legal. -/
structure Inv (cc : CChart) (core : Core) : Prop where
  sorted : core.config.Pairwise (· < ·)
  hist : HistOk cc core.history
  legal : core.terminated = false → cc.Below 0 core.config

theorem inv_keeps : ∀ c c', Keeps c c' → (Inv cc c ∧ c.terminated = false) →
    (Inv cc c' ∧ c'.terminated = false) := by
  rintro c c' ⟨e1, e2, e3⟩ ⟨⟨h1, h2, h3⟩, h4⟩
  refine ⟨⟨e1 ▸ h1, e2 ▸ h2, fun ht => e1 ▸ h3 (e3 ▸ ht)⟩, e3 ▸ h4⟩

theorem microstep_spec (hw : CWF cc) (hEnt : EntryLegal) (ev : Option String) (ts : List Nat) :
    Hoare (cc := cc) (fun c => (Inv cc c ∧ c.terminated = false) ∧ Selected cc c.config c.history ts)
      (Interp.microstep cc ev ts) (fun _ c => Inv cc c ∧ c.terminated = false) := by
  have hP : ∀ c c', Keeps c c' →
      ((Inv cc c ∧ c.terminated = false) ∧ Selected cc c.config c.history ts) →
      ((Inv cc c' ∧ c'.terminated = false) ∧ Selected cc c'.config c'.history ts) := by
    rintro c c' hk ⟨h1, h2⟩
    exact ⟨inv_keeps c c' hk h1, hk.1 ▸ hk.2.1 ▸ h2⟩
  unfold Interp.microstep
  refine hoare_bind (frame_charge.inv _ hP) (fun b => ?_)
  refine hoare_ite _ (fun _ => hoare_conseq (hoare_pure _) (fun _ p => p) (fun _ _ p => p.1))
    (fun _ => ?_)
  refine hoare_bind ((frame_record _ _).inv _ hP) (fun _ => ?_)
  refine hoare_bind hoare_get (fun core => ?_)
  intro c0 e a c' e' hpre hrun
  obtain ⟨⟨⟨⟨hsort, hho, hleg⟩, hterm⟩, hsel⟩, rfl⟩ := hpre
  have hb := hleg hterm
  have hnd : core.config.Nodup := hsort.imp Nat.ne_of_lt
  obtain ⟨hho', hpre'⟩ := exit_legal cc core.config core.history ts hw hb hnd hho hsel
  have heff := eff_adequate cc _ hw hho'
  have hfr : Fr core.config core.history core.terminated core := ⟨rfl, rfl, rfl⟩
  have := (hoare_bind (exitStates_spec core.config core.history core.terminated ts)
    (fun _ => hoare_bind (Frame.forIn _ _ _ (fun t _ =>
      Frame.bind (frame_runTrans t) (fun _ => Frame.pure _)) _ _ _)
      (fun _ => enterStates_spec _ _ core.history core.terminated ts))) core e a c' e' hfr hrun
  obtain ⟨acc, hacc, h1, h2, h3⟩ := this
  obtain ⟨acc', hacc', hL⟩ := hEnt cc _ _ _ ts hw heff hpre'
  rw [hacc] at hacc'; cases hacc'
  refine ⟨⟨?_, h2 ▸ hho', fun _ => h1 ▸ hL _ (fun x => ?_)⟩, h3.trans hterm⟩
  · rw [h1]
    exact sorted_foldl_insertSorted _ _ (sorted_foldl_erase _ _ hsort)
  · rw [mem_foldl_insertSorted, mem_foldl_erase _ _ hnd, List.mem_reverse, List.mem_mergeSort]
    simp

/-! ## The macrostep -/

theorem hoare_get_inv {P : Core → Prop} : Hoare (cc := cc) P get (fun _ => P) :=
  hoare_conseq hoare_get (fun _ p => p) (fun _ _ p => p.1)

/-- `exitInterpreter` ends with the invariant (the session is terminated). -/
theorem exitInterpreter_inv (b : Bool) :
    Hoare (cc := cc) (fun c => Inv cc c) (Interp.exitInterpreter cc b) (fun _ c => Inv cc c) := by
  intro core ex a core' ex' hP hrun
  obtain ⟨h1, h2, h3⟩ := exitInterpreter_spec core.history b core ex a core' ex' ⟨hP.sorted, rfl⟩ hrun
  exact ⟨h1, h2 ▸ hP.hist, fun ht => by rw [h3] at ht; cases ht⟩

theorem macroLoop_spec (hw : CWF cc) (hEnt : EntryLegal) (n : Nat) :
    Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (Interp.macroLoop cc n)
      (fun _ c => Inv cc c) := by
  induction n with
  | zero =>
    unfold Interp.macroLoop
    exact hoare_bind hoare_get_inv (fun _ => hoare_throw _)
  | succ n ih =>
    unfold Interp.macroLoop
    refine hoare_bind hoare_get_inv (fun c => ?_)
    refine hoare_ite _ (fun _ => hoare_conseq (hoare_pure _) (fun _ p => p) (fun _ _ p => p.1))
      (fun _ => ?_)
    refine hoare_bind hoare_get_inv (fun c => ?_)
    refine hoare_ite _ (fun _ => ?_) (fun _ => ?_)
    · exact hoare_bind (hoare_conseq (exitInterpreter_inv _) (fun _ p => p.1) (fun _ _ p => p))
        (fun _ => hoare_pure _)
    refine hoare_bind (hoare_select _ none) (fun ts => ?_)
    refine hoare_ite _ (fun _ => ?_) (fun _ => ?_)
    · exact hoare_bind (microstep_spec hw hEnt _ _) (fun _ => ih)
    refine hoare_conseq (P := fun c => Inv cc c ∧ c.terminated = false) ?_ (fun _ p => p.1)
      (fun _ _ p => p)
    refine hoare_bind (frame_dequeue.inv _ inv_keeps) (fun r => ?_)
    cases r with
    | some e =>
      refine hoare_bind ((frame_setEvent _).inv _ inv_keeps) (fun _ => ?_)
      refine hoare_bind (hoare_select _ _) (fun ts => ?_)
      extract_lets jp
      have hjp : ∀ u, Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (jp u)
          (fun _ c => Inv cc c) := fun _ => ih
      refine hoare_ite _ (fun _ => ?_) (fun _ => hoare_bind (microstep_spec hw hEnt _ _) hjp)
      refine hoare_conseq (P := fun c => Inv cc c ∧ c.terminated = false) ?_ (fun _ p => p.1)
        (fun _ _ p => p)
      refine hoare_bind (frame_charge.inv _ inv_keeps) (fun b => ?_)
      exact hoare_ite _ (fun _ => hoare_bind ((frame_record _ _).inv _ inv_keeps) hjp) (fun _ => hjp ())
    | none =>
      refine hoare_bind (frame_startInvokes.inv _ inv_keeps) (fun _ => ?_)
      refine hoare_bind ((Frame.lift (get : SM cc (ExecSt cc))).inv _ inv_keeps) (fun st => ?_)
      exact hoare_ite _ (fun _ => hoare_conseq (hoare_pure _) (fun _ p => p) (fun _ _ p => p.1))
        (fun _ => ih)

theorem runMacrostep_spec (hw : CWF cc) (hEnt : EntryLegal) :
    Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (Interp.runMacrostep cc)
      (fun _ c => Inv cc c) := by
  unfold Interp.runMacrostep
  exact hoare_bind hoare_get_inv (fun _ => macroLoop_spec hw hEnt _)

theorem runExternal_spec (hw : CWF cc) (hEnt : EntryLegal) (ev : Event) :
    Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (Interp.runExternal cc ev)
      (fun _ c => Inv cc c) := by
  unfold Interp.runExternal
  extract_lets jpM jp restStr
  have hM : ∀ u, Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (jpM u)
      (fun _ c => Inv cc c) := fun _ => runMacrostep_spec hw hEnt
  have hjp : ∀ u, Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (jp u)
      (fun _ c => Inv cc c) := by
    intro u
    refine hoare_bind ((frame_setEvent _).inv _ inv_keeps) (fun _ => ?_)
    refine hoare_bind hoare_get_inv (fun core => ?_)
    extract_lets invs
    refine hoare_bind (Frame.inv (Frame.forIn _ _ _ (fun inv _ => ?_)) _ inv_keeps) (fun _ => ?_)
    · extract_lets jp2
      have hj : ∀ u, Frame (cc := cc) (jp2 u) := fun _ =>
        Frame.ite _ (fun _ => Frame.bind (Frame.liftE _) (fun _ => Frame.pure _)) (fun _ => Frame.pure _)
      refine Frame.ite _ (fun _ => ?_) (fun _ => hj ())
      split
      · exact Frame.bind (Frame.liftE _) hj
      · exact hj ()
    refine hoare_bind (hoare_select _ _) (fun ts => ?_)
    refine hoare_ite _ (fun _ => ?_) (fun _ => hoare_bind (microstep_spec hw hEnt _ _) hM)
    exact hoare_conseq (hoare_bind ((frame_record _ _).inv _ inv_keeps) hM) (fun _ p => p.1)
      (fun _ _ p => p)
  refine hoare_ite _ (fun _ => ?_) (fun _ => hjp ())
  refine hoare_bind ((Frame.modify _ ?_).inv _ inv_keeps) hjp
  intro _; exact ⟨rfl, rfl, rfl⟩

theorem runStart_spec (hw : CWF cc) (hEnt : EntryLegal) (invoker : List (String × Value)) :
    Hoare (cc := cc) (Fr [] [] false) (Interp.runStart cc invoker) (fun _ c => Inv cc c) := by
  unfold Interp.runStart
  extract_lets jpM jpI jpS
  have hM : ∀ u, Hoare (cc := cc) (fun c => Inv cc c ∧ c.terminated = false) (jpM u)
      (fun _ c => Inv cc c) := fun _ => runMacrostep_spec hw hEnt
  have hI : ∀ u, Hoare (cc := cc) (Fr [] [] false) (jpI u) (fun _ c => Inv cc c) := by
    intro u
    dsimp only [jpI]
    have h0 := hw.initial_iff 0
    rw [CWF.isCompoundOrRoot_zero hw] at h0
    cases ht : (cc.st 0).initial with
    | none => rw [ht] at h0; cases h0
    | some t =>
      refine hoare_bind hoare_get (fun core => ?_)
      refine hoare_bind (Q := fun _ c => Inv cc c ∧ c.terminated = false) ?_ hM
      intro c0 e a c' e' hpre hrun
      obtain ⟨⟨h1, h2, h3⟩, rfl⟩ := hpre
      rw [h2] at hrun
      obtain ⟨acc, hacc, k1, k2, k3⟩ := enterStates_spec [] [] [] false [t] core e a c' e'
        ⟨h1, h2, h3⟩ hrun
      have hho : HistOk cc [] := fun h v hv => by simp [CChart.histValue] at hv
      obtain ⟨acc', hacc', hL⟩ := hEnt cc [] [] [] [t] hw (eff_adequate cc [] hw hho)
        (start_entry cc t hw ht)
      rw [hacc] at hacc'; cases hacc'
      refine ⟨⟨?_, k2 ▸ hho, fun _ => k1 ▸ hL _ (fun x => ?_)⟩, k3⟩
      · rw [k1]; exact sorted_foldl_insertSorted _ _ List.Pairwise.nil
      · rw [mem_foldl_insertSorted, List.mem_mergeSort]
  have hS : ∀ u, Hoare (cc := cc) (Fr [] [] false) (jpS u) (fun _ c => Inv cc c) := by
    intro u
    dsimp only [jpS]
    split
    · split
      · refine hoare_bind (Frame.liftE _ [] [] false) (fun r => ?_)
        split
        · exact hI ()
        · exact hoare_bind (Frame.liftE _ [] [] false) hI
      · exact hoare_bind (Frame.liftE _ [] [] false) hI
    · exact hI ()
  refine hoare_bind (Frame.forIn _ _ _ (fun i _ => ?_) [] [] false) (fun _ => ?_)
  · refine Frame.bind (Frame.forIn _ _ _ (fun x _ => ?_)) (fun _ => Frame.pure _)
    rcases x with ⟨d, hd⟩
    refine Frame.bind (Frame.liftE _) (fun r => ?_)
    split
    · exact Frame.pure _
    · exact Frame.bind (Frame.liftE _) (fun _ => Frame.pure _)
  split
  · refine hoare_bind (Frame.forIn _ _ _ (fun i _ => ?_) [] [] false) hS
    exact Frame.bind (Frame.liftE _) (fun _ => Frame.bind (Frame.modify _ (fun _ => ⟨rfl, rfl, rfl⟩))
      (fun _ => Frame.pure _))
  · refine hoare_bind (Frame.liftE _ [] [] false) (fun _ => hoare_bind (Frame.modify _ ?_ [] [] false) hS)
    intro _; exact ⟨rfl, rfl, rfl⟩

end Vertep.Config
