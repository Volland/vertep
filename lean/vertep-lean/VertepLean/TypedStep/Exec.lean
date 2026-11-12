import VertepLean.TypedStep.Hoare

/-!
# Executable content keeps the typing invariant

`Inv Γ`: the unconsumed tape is typed, every logged request is typed, every queued internal
event and every emitted effect is typed. Every datamodel request, every `<send>`, `<raise>`,
`<assign>`, `foreach`, `<data>` initialization and `<invoke>` keeps it, given the checks of
`ChartOk`.
-/

namespace Vertep.Typed

open Types

variable (Γ : ChartTyping) {cc : CChart}

/-- The typing invariant of the datamodel state. -/
def Inv (st : ExecSt cc) : Prop :=
  TapeTyped Γ st.tape ∧ (∀ r ∈ st.log.toList, ReqTyped Γ r.1) ∧
    (∀ e ∈ st.internal, EventTyped Γ e) ∧ (∀ f ∈ st.effects.toList, EffectTyped Γ f)

variable {Γ}

theorem answer_beq {a b : Request} {ans : Answer} (h : (a == b) = true)
    (ha : AnswerTyped Γ a ans) : AnswerTyped Γ b ans := by
  cases b with
  | evalValue x =>
    cases a <;> simp [BEq.beq, instBEqRequest.beq, Request.ctorIdx] at h
    subst h; exact ha
  | evalContent x =>
    cases a <;> simp [BEq.beq, instBEqRequest.beq, Request.ctorIdx] at h
    subst h; exact ha
  | foreachItems x =>
    cases a <;> simp [BEq.beq, instBEqRequest.beq, Request.ctorIdx] at h
    subst h; exact ha
  | _ => cases ans <;> trivial

theorem nullAnswer_typed (hc : ChartOk Γ cc) (hn : cc.datamodel = "null") (config : List Nat)
    (r : Request) : AnswerTyped Γ r (Interp.nullAnswer cc config r) := by
  cases r with
  | evalContent c => exact hc.nullContent hn c
  | evalCond e =>
    simp only [Interp.nullAnswer]
    split <;> trivial
  | restore v =>
    simp only [Interp.nullAnswer]
    split <;> trivial
  | _ => trivial

theorem inv_push_log {st : ExecSt cc} (hI : Inv Γ st) (x : Req cc) (hx : ReqTyped Γ x.1)
    (tape : Tape) (ht : TapeTyped Γ tape) (le : Option String) :
    Inv Γ { st with tape := tape, log := st.log.push x, lastError := le } := by
  obtain ⟨_, hl, hi, he⟩ := hI
  refine ⟨ht, ?_, hi, he⟩
  intro r hr
  rw [Array.toList_push, List.mem_append] at hr
  rcases hr with hr | hr
  · exact hl r hr
  · rw [List.mem_singleton] at hr; subst hr; exact hx

theorem tape_tail {e : TapeEntry} {rest : Tape} (h : TapeTyped Γ (e :: rest)) :
    TapeTyped Γ rest ∧ AnswerTyped Γ e.request e.answer :=
  ⟨fun x hx => h x (List.mem_cons_of_mem _ hx), h e (List.mem_cons_self ..)⟩

/-- A datamodel request: a typed request keeps the invariant and gets a typed answer. -/
theorem request_spec (hc : ChartOk Γ cc) (r : Request) (hl : LocOk cc r) (hr : ReqTyped Γ r) :
    HE (Inv Γ) (Interp.request cc r hl) (fun a st => Inv Γ st ∧ AnswerTyped Γ r a) := by
  unfold Interp.request
  refine he_bind he_read (fun env => ?_)
  split
  · rename_i hn
    refine he_bind (he_modify _ (fun s h => ?_)) (fun _ => he_pure _ (fun s h => h))
    · exact ⟨inv_push_log h ⟨r, hl⟩ hr _ h.1 _,
        nullAnswer_typed hc (by simpa using hn) env.config r⟩
  · refine he_bind (he_get (Q := fun s' s => s' = s ∧ Inv Γ s) (fun s h => ⟨rfl, h⟩))
      (fun st0 => ?_)
    split
    · exact he_throw _
    · rename_i e rest htape
      split
      · rename_i hbeq
        split
        · refine he_bind (he_set _ (Q := fun _ s => Inv Γ s ∧ AnswerTyped Γ r e.answer)
            (fun ⟨s, hs⟩ => ?_)) (fun _ => he_pure _ (fun s h => h))
          obtain ⟨rfl, hI⟩ := hs
          have ht := hI.1
          rw [htape] at ht
          obtain ⟨ht', ha⟩ := tape_tail ht
          exact ⟨inv_push_log hI ⟨r, hl⟩ hr _ ht' _, answer_beq hbeq ha⟩
        · exact he_throw _
      · exact he_throw _

theorem ask_spec (hc : ChartOk Γ cc) (r : Request) (h : r.writes = none) (hr : ReqTyped Γ r) :
    HE (Inv Γ) (Interp.ask cc r h) (fun a st => Inv Γ st ∧ AnswerTyped Γ r a) :=
  request_spec hc r _ hr

/-- `ask` of a request that is typed and needs no typed answer. -/
theorem ask_inv (hc : ChartOk Γ cc) (r : Request) (h : r.writes = none) (hr : ReqTyped Γ r) :
    HE (Inv Γ) (Interp.ask cc r h) (fun _ => Inv Γ) :=
  he_conseq (ask_spec hc r h hr) (fun _ h => h) (fun _ _ h => h.1)

theorem evalValue_spec (hc : ChartOk Γ cc) (x : Expr) :
    HE (Inv Γ) (Interp.evalValue cc x)
      (fun o st => Inv Γ st ∧ ∀ v, o = some v → ValTyped Γ (Γ.expr x) v) := by
  unfold Interp.evalValue
  refine he_bind (ask_spec hc _ rfl trivial) (fun a => ?_)
  split
  · rename_i v
    exact he_pure _ (fun s h => ⟨h.1, fun w hw => by cases hw; exact h.2⟩)
  · exact he_pure _ (fun s h => ⟨h.1, fun w hw => by cases hw⟩)

theorem evalValue_inv (hc : ChartOk Γ cc) (x : Expr) :
    HE (Inv Γ) (Interp.evalValue cc x) (fun _ => Inv Γ) :=
  he_conseq (evalValue_spec hc x) (fun _ h => h) (fun _ _ h => h.1)

theorem evalContent_spec (hc : ChartOk Γ cc) (c : String) :
    HE (Inv Γ) (Interp.evalContent cc c)
      (fun o st => Inv Γ st ∧ ∀ v, o = some v → ValTyped Γ (Γ.content c) v) := by
  unfold Interp.evalContent
  refine he_bind (ask_spec hc _ rfl trivial) (fun a => ?_)
  split
  · rename_i v
    exact he_pure _ (fun s h => ⟨h.1, fun w hw => by cases hw; exact h.2⟩)
  · exact he_pure _ (fun s h => ⟨h.1, fun w hw => by cases hw⟩)

theorem evalString_inv (hc : ChartOk Γ cc) (x : Expr) :
    HE (Inv Γ) (Interp.evalString cc x) (fun _ => Inv Γ) := by
  unfold Interp.evalString
  exact he_bind (evalValue_inv hc x) (fun _ => he_pure _ (fun _ h => h))

theorem evalCond_inv (hc : ChartOk Γ cc) (x : Expr) :
    HE (Inv Γ) (Interp.evalCond cc x) (fun _ => Inv Γ) := by
  unfold Interp.evalCond
  refine he_bind (ask_inv hc _ rfl trivial) (fun a => ?_)
  split <;> exact he_pure _ (fun _ h => h)

theorem assignValue_inv (hc : ChartOk Γ cc) (l : String) (v : Value) (h : l ∈ cc.locations)
    (hv : LocTyped Γ l v) : HE (Inv Γ) (Interp.assignValue cc l v h) (fun _ => Inv Γ) := by
  unfold Interp.assignValue
  refine he_bind (he_conseq (request_spec hc _ _ hv) (fun _ h => h) (fun _ _ h => h.1))
    (fun a => ?_)
  split <;> exact he_pure _ (fun _ h => h)

theorem bindVar_inv (hc : ChartOk Γ cc) (l : String) (v : Value) (h : l ∈ cc.locations)
    (hv : LocTyped Γ l v) : HE (Inv Γ) (Interp.bindVar cc l v h) (fun _ => Inv Γ) := by
  unfold Interp.bindVar
  refine he_bind (he_conseq (request_spec hc _ _ hv) (fun _ h => h) (fun _ _ h => h.1))
    (fun a => ?_)
  split <;> exact he_pure _ (fun _ h => h)

/-- `attr`: a literal is returned as is, and no attribute at all gives `none`. -/
theorem attr_spec (hc : ChartOk Γ cc) (lit : Option String) (ex : Option Expr) :
    HE (Inv Γ) (Interp.attr cc lit ex) (fun r st => Inv Γ st ∧ ∀ o, r = some o →
      (∀ v, lit = some v → o = some v) ∧ (lit = none → ex = none → o = none)) := by
  unfold Interp.attr
  split
  · rename_i v _
    refine he_pure _ (fun _ h => ⟨h, ?_⟩)
    intro o ho
    cases ho
    exact ⟨fun w hw => (by cases hw; rfl), fun hn => (by cases hn)⟩
  · refine he_bind (evalString_inv hc _) (fun s => he_pure _ (fun _ h => ⟨h, ?_⟩))
    intro o _
    exact ⟨fun w hw => (by cases hw), fun _ hn => (by cases hn)⟩
  · refine he_pure _ (fun _ h => ⟨h, ?_⟩)
    intro o ho
    cases ho
    exact ⟨fun w hw => (by cases hw), fun _ _ => rfl⟩

theorem enqueue_inv (e : Event) (he : EventTyped Γ e) :
    HE (Inv Γ) (Interp.enqueue cc e) (fun _ => Inv Γ) := by
  unfold Interp.enqueue
  refine he_modify _ (fun s ⟨h1, h2, h3, h4⟩ => ⟨h1, h2, ?_, h4⟩)
  intro x hx
  rw [List.mem_append, List.mem_singleton] at hx
  rcases hx with hx | rfl
  · exact h3 x hx
  · exact he

theorem emit_inv (f : Effect) (hf : EffectTyped Γ f) :
    HE (Inv Γ) (Interp.emit cc f) (fun _ => Inv Γ) := by
  unfold Interp.emit
  refine he_modify _ (fun s ⟨h1, h2, h3, h4⟩ => ⟨h1, h2, h3, ?_⟩)
  intro x hx
  rw [Array.toList_push, List.mem_append, List.mem_singleton] at hx
  rcases hx with hx | rfl
  · exact h4 x hx
  · exact hf

theorem queueError_inv (hc : ChartOk Γ cc) (sid : Option String) :
    HE (Inv Γ) (Interp.queueError cc sid) (fun _ => Inv Γ) := by
  unfold Interp.queueError
  refine he_modify _ (fun s ⟨h1, h2, h3, h4⟩ => ⟨h1, h2, ?_, h4⟩)
  intro x hx
  rw [List.mem_append, List.mem_singleton] at hx
  rcases hx with hx | rfl
  · exact h3 x hx
  · exact hc.errors _ _ _ (.inl rfl)

theorem he_ite {α : Type} {P : ExecSt cc → Prop} {Q : α → ExecSt cc → Prop} {c : Prop}
    [Decidable c] {x y : EM cc α} (hx : c → HE P x Q) (hy : ¬c → HE P y Q) :
    HE P (if c then x else y) Q := by
  by_cases h : c
  · simp only [h, ↓reduceIte]; exact hx h
  · simp only [h, ↓reduceIte]; exact hy h

/-- Use a fact carried by the precondition. -/
theorem he_fact {α : Type} {I : ExecSt cc → Prop} {F : Prop} {x : EM cc α}
    {Q : α → ExecSt cc → Prop} (h : F → HE I x Q) : HE (fun s => I s ∧ F) x Q :=
  fun env st a st' hP hr => h hP.2 env st a st' hP.1 hr

/-! ## Payloads -/

theorem entries_snoc {ss : List (String × Option Expr)} {es : List (String × Value)}
    (h : EntriesTyped Γ ss es) (k : String) (src : Option Expr) (v : Value)
    (hv : match src with
      | some x => ValTyped Γ (Γ.expr x) v
      | none => v = .undefined) :
    EntriesTyped Γ (ss ++ [(k, src)]) (es ++ [(k, v)]) := by
  induction ss generalizing es with
  | nil =>
    cases es with
    | nil => exact ⟨rfl, hv, trivial⟩
    | cons e es => exact absurd h (by simp [EntriesTyped])
  | cons s ss ih =>
    cases es with
    | nil => exact absurd h (by simp [EntriesTyped])
    | cons e es =>
      obtain ⟨k₀, src₀⟩ := s
      obtain ⟨k₁, v₁⟩ := e
      obtain ⟨h1, h2, h3⟩ := h
      exact ⟨h1, h2, ih h3⟩

/-- The `namelist` fold of `evalPayload`. -/
theorem names_fold (hc : ChartOk Γ cc) (names : List String) :
    ∀ (pre : List (String × Option Expr)) (acc : Option (List (String × Value))),
      (∀ a, acc = some a → EntriesTyped Γ pre a) →
      HE (Inv Γ) (names.foldlM (fun acc n => match acc with
        | none => pure none
        | some acc => do
          let v ← Interp.evalValue cc n
          pure (Option.map (fun v => acc ++ [(n, v)]) v)) acc)
        (fun r s => Inv Γ s ∧ ∀ a, r = some a →
          EntriesTyped Γ (pre ++ names.map (fun n => (n, some n))) a) := by
  induction names with
  | nil =>
    intro pre acc hacc
    exact he_pure _ (fun _ h => ⟨h, fun a ha => by simpa using hacc a ha⟩)
  | cons n ns ih =>
    intro pre acc hacc
    rw [List.foldlM_cons]
    refine he_bind (Q := fun r s => Inv Γ s ∧
      ∀ a, r = some a → EntriesTyped Γ (pre ++ [(n, some n)]) a) ?_ (fun r => ?_)
    · split
      · exact he_pure _ (fun _ h => ⟨h, fun a ha => by cases ha⟩)
      · rename_i a
        refine he_bind (evalValue_spec hc n) (fun o => he_pure _ (fun _ h => ⟨h.1, ?_⟩))
        intro b hb
        cases o with
        | none => cases hb
        | some v =>
          cases hb
          exact entries_snoc (hacc a rfl) n (some n) v (h.2 v rfl)
    · intro env st b st' hP hr
      have := ih (pre ++ [(n, some n)]) r hP.2 env st b st' hP.1 hr
      refine ⟨this.1, fun a ha => ?_⟩
      have h2 := this.2 a ha
      simpa using h2

/-- The source of a `<param>`. -/
def paramSrc (p : Param) : Option Expr :=
  match p.expr, p.location with
  | some x, _ => some x
  | none, some l => some l
  | none, none => none

/-- The `<param>` fold of `evalPayload`. -/
theorem params_fold (hc : ChartOk Γ cc) (ps : List Param) :
    ∀ (pre : List (String × Option Expr)) (acc : Option (List (String × Value))),
      (∀ a, acc = some a → EntriesTyped Γ pre a) →
      HE (Inv Γ) (ps.foldlM (fun acc p => match acc with
        | none => pure none
        | some acc =>
          match p.expr, p.location with
          | some x, _ => do
            let v ← Interp.evalValue cc x
            pure (Option.map (fun v => acc ++ [(p.name, v)]) v)
          | none, some l => do
            let v ← Interp.evalValue cc l
            pure (Option.map (fun v => acc ++ [(p.name, v)]) v)
          | none, none => pure (some (acc ++ [(p.name, Value.undefined)]))) acc)
        (fun r s => Inv Γ s ∧ ∀ a, r = some a →
          EntriesTyped Γ (pre ++ ps.map (fun p => (p.name, paramSrc p))) a) := by
  induction ps with
  | nil =>
    intro pre acc hacc
    exact he_pure _ (fun _ h => ⟨h, fun a ha => by simpa using hacc a ha⟩)
  | cons p ps ih =>
    intro pre acc hacc
    rw [List.foldlM_cons]
    refine he_bind (Q := fun r s => Inv Γ s ∧
      ∀ a, r = some a → EntriesTyped Γ (pre ++ [(p.name, paramSrc p)]) a) ?_ (fun r => ?_)
    · split
      · exact he_pure _ (fun _ h => ⟨h, fun a ha => by cases ha⟩)
      · rename_i a
        split
        · rename_i x hx
          refine he_bind (evalValue_spec hc x) (fun o => he_pure _ (fun _ h => ⟨h.1, ?_⟩))
          intro b hb
          cases o with
          | none => cases hb
          | some v =>
            cases hb
            have hs : paramSrc p = some x := by simp [paramSrc, hx]
            rw [hs]
            exact entries_snoc (hacc a rfl) p.name (some x) v (h.2 v rfl)
        · rename_i l hx hl
          refine he_bind (evalValue_spec hc l) (fun o => he_pure _ (fun _ h => ⟨h.1, ?_⟩))
          intro b hb
          cases o with
          | none => cases hb
          | some v =>
            cases hb
            have hs : paramSrc p = some l := by simp [paramSrc, hx, hl]
            rw [hs]
            exact entries_snoc (hacc a rfl) p.name (some l) v (h.2 v rfl)
        · rename_i hx hl
          refine he_pure _ (fun _ h => ⟨h, ?_⟩)
          intro b hb
          cases hb
          have hs : paramSrc p = none := by simp [paramSrc, hx, hl]
          rw [hs]
          exact entries_snoc (hacc a rfl) p.name none _ rfl
    · intro env st b st' hP hr
      have := ih (pre ++ [(p.name, paramSrc p)]) r hP.2 env st b st' hP.1 hr
      refine ⟨this.1, fun a ha => ?_⟩
      have h2 := this.2 a ha
      simpa using h2

theorem entrySrcs_eq (names : List String) (ps : List Param) :
    entrySrcs names ps = names.map (fun n => (n, some n)) ++ ps.map (fun p => (p.name, paramSrc p)) :=
  rfl

theorem evalPayload_spec (hc : ChartOk Γ cc) (names : List String) (ps : List Param)
    (c : Option Content) :
    HE (Inv Γ) (Interp.evalPayload cc names ps c)
      (fun o st => Inv Γ st ∧ ∀ v, o = some v → PayloadRes Γ names ps c v) := by
  unfold Interp.evalPayload
  split
  · rename_i c
    split
    · rename_i x hx
      refine he_conseq (evalValue_spec hc x) (fun _ h => h) (fun o s h => ⟨h.1, fun v hv => ?_⟩)
      simp only [PayloadRes, hx]
      exact h.2 v hv
    · rename_i b hx hb
      refine he_conseq (evalContent_spec hc b) (fun _ h => h) (fun o s h => ⟨h.1, fun v hv => ?_⟩)
      simp only [PayloadRes, hx, hb]
      exact h.2 v hv
    · rename_i hx hb
      refine he_pure _ (fun _ h => ⟨h, fun v hv => ?_⟩)
      cases hv
      simp only [PayloadRes, hx, hb]
  · split
    · rename_i hne
      refine he_pure _ (fun _ h => ⟨h, fun v hv => ?_⟩)
      cases hv
      simp only [PayloadRes, hne, ↓reduceIte]
    · rename_i hne
      refine he_bind (names_fold hc names [] (some []) (fun a ha => by cases ha; trivial))
        (fun nl => ?_)
      split
      · rename_i nl'
        refine he_fact (fun hnl => ?_)
        refine he_bind (params_fold hc ps _ (some nl') (fun a ha => by cases ha; exact hnl _ rfl))
          (fun pl => he_pure _ (fun _ h => ⟨h.1, fun v hv => ?_⟩))
        cases pl with
        | none => cases hv
        | some es =>
          cases hv
          simp only [PayloadRes, hne, Bool.false_eq_true, ↓reduceIte]
          refine ⟨es, rfl, ?_⟩
          rw [entrySrcs_eq]
          simpa using h.2 es rfl
      · exact he_pure _ (fun _ h => ⟨h.1, fun v hv => by cases hv⟩)

theorem doneData_spec (hc : ChartOk Γ cc) (dd : Option DoneData) :
    HE (Inv Γ) (Interp.doneData cc dd) (fun v st => Inv Γ st ∧ DoneRes Γ dd v) := by
  unfold Interp.doneData
  split
  · exact he_pure _ (fun _ h => ⟨h, .inl rfl⟩)
  · rename_i d
    refine he_bind (evalPayload_spec hc [] d.params d.content) (fun o => ?_)
    split
    · rename_i v
      exact he_pure _ (fun _ h => ⟨h.1, .inr ⟨d, rfl, h.2 v rfl⟩⟩)
    · exact he_bind (he_conseq (queueError_inv hc none) (fun _ h => h.1) (fun _ _ h => h))
        (fun _ => he_pure _ (fun _ h => ⟨h, .inl rfl⟩))

/-! ## Data, identifiers and `<send>` -/

theorem initData_inv (hc : ChartOk Γ cc) (items : List Data)
    (h : ∀ d ∈ items, d.id ∈ cc.locations) (invoker : List (String × Value))
    (hok : ∀ d ∈ items, DataOk Γ d) (hinv : ∀ p ∈ invoker, LocTyped Γ p.1 p.2) :
    HE (Inv Γ) (Interp.initData cc items h invoker) (fun _ => Inv Γ) := by
  unfold Interp.initData
  refine he_forM _ _ (fun x _ => ?_)
  obtain ⟨d, hd⟩ := x
  have hdok := hok d hd
  have hjp : ∀ ok, HE (Inv Γ) (if (!ok) = true then Interp.queueError cc else pure ())
      (fun _ => Inv Γ) := fun ok =>
    he_ite (fun _ => queueError_inv hc none) (fun _ => he_pure _ (fun _ h => h))
  dsimp only
  split
  · rename_i v hv
    exact he_bind (assignValue_inv hc _ _ _ (hinv _ (Types.lookup_mem' hv))) hjp
  · split
    · rename_i x hx
      simp only [DataOk, hx] at hdok
      refine he_bind (evalValue_spec hc x) (fun o => ?_)
      split
      · rename_i v
        exact he_fact (fun hv => he_bind (assignValue_inv hc _ _ _ (hdok v (hv v rfl))) hjp)
      · exact he_bind (he_pure _ (fun _ h => h.1)) hjp
    · rename_i c hx hc'
      simp only [DataOk, hx, hc'] at hdok
      refine he_bind (evalContent_spec hc c) (fun o => ?_)
      split
      · rename_i v
        exact he_fact (fun hv => he_bind (assignValue_inv hc _ _ _ (hdok v (hv v rfl))) hjp)
      · exact he_bind (he_pure _ (fun _ h => h.1)) hjp
    · exact he_bind (he_pure _ (fun _ h => h)) hjp
    · exact he_bind (he_pure _ (fun _ h => h)) hjp

theorem sendId_inv (hc : ChartOk Γ cc) (s : Send)
    (hid : ∀ l, s.idlocation = some l → l ∈ cc.locations)
    (hs : ∀ l, s.idlocation = some l → StrLoc Γ l) :
    HE (Inv Γ) (Interp.sendId cc s hid) (fun _ => Inv Γ) := by
  unfold Interp.sendId
  split
  · exact he_pure _ (fun _ h => h)
  · refine he_bind he_read (fun env => ?_)
    refine he_bind (he_get (Q := fun s' s => s' = s ∧ Inv Γ s) (fun s h => ⟨rfl, h⟩))
      (fun st0 => ?_)
    refine he_bind (he_set _ (Q := fun _ => Inv Γ) (fun ⟨s, hs⟩ => ?_)) (fun _ => ?_)
    · obtain ⟨rfl, hI⟩ := hs
      exact hI
    · split
      · rename_i loc hl
        refine he_bind (assignValue_inv hc _ _ _ (hs loc hl _)) (fun b => ?_)
        split
        · exact he_pure _ (fun _ h => h)
        · exact he_bind (queueError_inv hc _) (fun _ => he_pure _ (fun _ h => h))
      · exact he_pure _ (fun _ h => h)

theorem attr_inv (hc : ChartOk Γ cc) (lit : Option String) (ex : Option Expr) :
    HE (Inv Γ) (Interp.attr cc lit ex) (fun _ => Inv Γ) :=
  he_conseq (attr_spec hc lit ex) (fun _ h => h) (fun _ _ h => h.1)

theorem execSend_inv (hc : ChartOk Γ cc) (s : Send)
    (hid : ∀ l, s.idlocation = some l → l ∈ cc.locations) (hs : SendOk Γ s) :
    HE (Inv Γ) (Interp.execSend cc s hid) (fun _ => Inv Γ) := by
  unfold Interp.execSend
  refine he_bind he_read (fun env => ?_)
  refine he_bind (sendId_inv hc s hid hs.2) (fun r => ?_)
  split
  · rename_i sendid visible
    have hfail : HE (Inv Γ) (do Interp.queueError cc (some sendid); pure false : EM cc Bool)
        (fun _ => Inv Γ) :=
      he_bind (queueError_inv hc _) (fun _ => he_pure _ (fun _ h => h))
    refine he_bind (attr_spec hc _ _) (fun rn => ?_)
    split
    · rename_i name
      refine he_fact (fun hn => ?_)
      obtain ⟨hn1, hn2⟩ := hn name rfl
      refine he_bind (attr_inv hc _ _) (fun rt => ?_)
      split
      · rename_i target
        refine he_bind (attr_inv hc _ _) (fun rty => ?_)
        split
        · rename_i type
          refine he_bind (attr_inv hc _ _) (fun rd => ?_)
          split
          · rename_i delayText
            split
            · rename_i delayMs _
              refine he_bind (evalPayload_spec hc _ _ _) (fun rp => ?_)
              split
              · rename_i data
                refine he_fact (fun hdata => ?_)
                have hev : PayloadTyped Γ (name.getD "") data := by
                  refine hs.1 _ (fun m hm => ?_) (fun h1 h2 => ?_) data (hdata data rfl)
                  · rw [hn1 m hm]; rfl
                  · rw [hn2 h1 h2]; rfl
                refine he_ite (fun _ => hfail) (fun _ => ?_)
                dsimp only
                repeat' (first
                  | refine he_ite (fun _ => ?_) (fun _ => ?_)
                  | refine he_bind (enqueue_inv _ ?_) (fun _ => he_pure _ (fun _ h => h))
                  | refine he_bind (emit_inv _ ?_) (fun _ => he_pure _ (fun _ h => h))
                  | exact hfail
                  | split)
                all_goals first | exact hev | exact hc.errors _ _ _ (.inr rfl)
              · exact he_conseq hfail (fun _ h => h.1) (fun _ _ h => h)
            · exact hfail
          · exact hfail
        · exact hfail
      · exact hfail
    · exact he_conseq hfail (fun _ h => h.1) (fun _ _ h => h)
  · exact he_pure _ (fun _ h => h)

/-! ## Executable content -/

theorem evalContent_inv (hc : ChartOk Γ cc) (c : String) :
    HE (Inv Γ) (Interp.evalContent cc c) (fun _ => Inv Γ) :=
  he_conseq (evalContent_spec hc c) (fun _ h => h) (fun _ _ h => h.1)

theorem fail_inv (hc : ChartOk Γ cc) :
    HE (Inv Γ) (do Interp.queueError cc; pure false : EM cc Bool) (fun _ => Inv Γ) :=
  he_bind (queueError_inv hc none) (fun _ => he_pure _ (fun _ h => h))

theorem okFail_inv (hc : ChartOk Γ cc) (ok : Bool) :
    HE (Inv Γ) (if ok = true then pure true else do Interp.queueError cc; pure false : EM cc Bool)
      (fun _ => Inv Γ) :=
  he_ite (fun _ => he_pure _ (fun _ h => h)) (fun _ => fail_inv hc)

mutual
/-- Every element of a checked chart keeps the invariant. -/
theorem execOne_inv (hc : ChartOk Γ cc) : (e : Exec) → (h : ∀ l ∈ e.locs, l ∈ cc.locations) →
    ExecOk Γ e → HE (Inv Γ) (Interp.execOne cc e h) (fun _ => Inv Γ)
  | .mk k _, h, hok => by
    rw [Interp.execOne]
    exact execKind_inv hc k _ (by simpa only [ExecOk] using hok)

theorem execKind_inv (hc : ChartOk Γ cc) : (k : ExecKind) →
    (h : ∀ l ∈ k.locs, l ∈ cc.locations) → KindOk Γ k →
    HE (Inv Γ) (Interp.execKind cc k h) (fun _ => Inv Γ)
  | .raise ev, h, hok => by
    rw [Interp.execKind]
    simp only [KindOk] at hok
    exact he_bind (enqueue_inv _ hok) (fun _ => he_pure _ (fun _ h => h))
  | .send s, h, hok => by
    rw [Interp.execKind]
    exact execSend_inv hc s _ (by simpa only [KindOk] using hok)
  | .cancel sid sidexpr, h, _ => by
    cases sid with
    | some id =>
      rw [Interp.execKind]
      exact he_bind (emit_inv _ trivial) (fun _ => he_pure _ (fun _ h => h))
    | none =>
      cases sidexpr with
      | some x =>
        rw [Interp.execKind]
        refine he_bind (evalString_inv hc x) (fun r => ?_)
        split
        · exact he_bind (emit_inv _ trivial) (fun _ => he_pure _ (fun _ h => h))
        · exact fail_inv hc
      | none =>
        rw [Interp.execKind]
        exact fail_inv hc
  | .log label ex, h, _ => by
    cases ex with
    | none =>
      rw [Interp.execKind]
      exact he_bind (emit_inv _ trivial) (fun _ => he_pure _ (fun _ h => h))
    | some x =>
      rw [Interp.execKind]
      refine he_bind (evalValue_inv hc x) (fun r => ?_)
      split
      · exact he_bind (emit_inv _ trivial) (fun _ => he_pure _ (fun _ h => h))
      · exact fail_inv hc
  | .assign loc ex content, h, hok => by
    cases ex with
    | some x =>
      rw [Interp.execKind]
      simp only [KindOk] at hok
      refine he_bind (he_conseq (request_spec hc _ _ hok) (fun _ h => h) (fun _ _ h => h.1))
        (fun a => ?_)
      split
      · exact he_bind (he_pure _ (fun _ h => h)) (okFail_inv hc)
      · exact he_bind (he_pure _ (fun _ h => h)) (okFail_inv hc)
    | none =>
      cases content with
      | some c =>
        rw [Interp.execKind]
        simp only [KindOk] at hok
        refine he_bind (evalContent_spec hc c) (fun o => ?_)
        split
        · rename_i v
          exact he_fact (fun hv => he_bind (assignValue_inv hc _ _ _ (hok v (hv v rfl)))
            (okFail_inv hc))
        · exact he_bind (he_pure _ (fun _ h => h.1)) (okFail_inv hc)
      | none =>
        rw [Interp.execKind]
        simp only [KindOk] at hok
        exact he_bind (assignValue_inv hc _ _ _ hok) (okFail_inv hc)
  | .script sc, h, _ => by
    rw [Interp.execKind]
    split
    · refine he_bind (ask_inv hc _ rfl trivial) (fun a => ?_)
      split
      · exact he_pure _ (fun _ h => h)
      · exact fail_inv hc
    · exact fail_inv hc
  | .if_ bs, h, hok => by
    rw [Interp.execKind]
    exact execBranches_inv hc bs _ (by simpa only [KindOk] using hok)
  | .foreach arr item idx body, h, hok => by
    rw [Interp.execKind]
    simp only [KindOk] at hok
    obtain ⟨hitem, hidx, hbody⟩ := hok
    refine he_bind (ask_spec hc _ rfl trivial) (fun a => ?_)
    split
    · rename_i items
      refine he_fact (fun hans => ?_)
      obtain ⟨hlen, hval⟩ := hans
      refine he_foldlM _ _ (fun x hx b => ?_) _
      obtain ⟨v, k⟩ := x
      have hm := List.mem_zipIdx_iff_getElem?.1 hx
      have hv : v ∈ items := List.mem_of_getElem? hm
      have hk : k < items.length := (List.getElem?_eq_some_iff.1 hm).1
      dsimp only
      refine he_ite (fun _ => he_pure _ (fun _ h => h)) (fun hb => ?_)
      refine he_bind (bindVar_inv hc _ _ _ (hitem v (hval v hv))) (fun ok => ?_)
      refine he_ite (fun _ => fail_inv hc) (fun hok' => ?_)
      split
      · rename_i i
        refine he_bind (bindVar_inv hc _ _ _ (hidx i rfl k (by omega))) (fun okIdx => ?_)
        exact he_ite (fun _ => fail_inv hc) (fun _ => execBlock_inv hc body _ hbody)
      · refine he_bind (Q := fun _ => Inv Γ) (he_pure _ (fun _ h => h)) (fun okIdx => ?_)
        exact he_ite (fun _ => fail_inv hc) (fun _ => execBlock_inv hc body _ hbody)
    · exact he_conseq (fail_inv hc) (fun _ h => h.1) (fun _ _ h => h)
  | .extension _, h, _ => by
    rw [Interp.execKind]
    exact he_pure _ (fun _ h => h)

theorem execBlock_inv (hc : ChartOk Γ cc) : (b : List Exec) →
    (h : ∀ l ∈ blockLocs b, l ∈ cc.locations) → BlockOk Γ b →
    HE (Inv Γ) (Interp.execBlock cc b h) (fun _ => Inv Γ)
  | [], h, _ => by
    rw [Interp.execBlock]
    exact he_pure _ (fun _ h => h)
  | e :: es, h, hok => by
    rw [Interp.execBlock]
    simp only [BlockOk] at hok
    refine he_bind (execOne_inv hc e _ hok.1) (fun r => ?_)
    refine he_ite (fun _ => execBlock_inv hc es _ hok.2) (fun _ => he_pure _ (fun _ h => h))

theorem execBranches_inv (hc : ChartOk Γ cc) : (bs : List (Option Expr × List Exec)) →
    (h : ∀ l ∈ branchLocs bs, l ∈ cc.locations) → BranchesOk Γ bs →
    HE (Inv Γ) (Interp.execBranches cc bs h) (fun _ => Inv Γ)
  | [], h, _ => by
    rw [Interp.execBranches]
    exact he_pure _ (fun _ h => h)
  | (c, b) :: bs, h, hok => by
    simp only [BranchesOk] at hok
    have hk : ∀ (take : Option Bool), HE (Inv Γ) ((fun take => match take with
        | none => do Interp.queueError cc; pure false
        | some true => Interp.execBlock cc b (fun l hl => h l (by simp [branchLocs, hl]))
        | some false => Interp.execBranches cc bs (fun l hl => h l (by simp [branchLocs, hl])))
          take) (fun _ => Inv Γ) := by
      intro take
      dsimp only
      split
      · exact fail_inv hc
      · exact execBlock_inv hc b _ hok.1
      · exact execBranches_inv hc bs _ hok.2
    cases c with
    | none =>
      rw [Interp.execBranches]
      exact he_bind (he_pure _ (fun _ h => h)) hk
    | some e =>
      rw [Interp.execBranches]
      refine he_bind (Q := fun _ => Inv Γ) ?_
        (fun a => he_bind (Q := fun _ => Inv Γ) (he_pure _ (fun _ h => h)) hk)
      exact evalCond_inv hc e
end

theorem execTop_inv (hc : ChartOk Γ cc) (b : List Exec) (h : ∀ l ∈ blockLocs b, l ∈ cc.locations)
    (hok : BlockOk Γ b) : HE (Inv Γ) (Interp.execTop cc b h) (fun _ => Inv Γ) := by
  unfold Interp.execTop
  exact he_bind (execBlock_inv hc b h hok) (fun _ => he_pure _ (fun _ h => h))

theorem evalPayload_inv (hc : ChartOk Γ cc) (names : List String) (ps : List Param)
    (c : Option Content) : HE (Inv Γ) (Interp.evalPayload cc names ps c) (fun _ => Inv Γ) :=
  he_conseq (evalPayload_spec hc names ps c) (fun _ h => h) (fun _ _ h => h.1)

/-! ## `<invoke>` and guards -/

/-- Close a goal `HE (Inv Γ) x (fun _ => Inv Γ)` for an action built from evaluations,
`queueError`, effects without events, conditionals and matches. -/
local macro "inv_auto " hc:term : tactic => `(tactic| repeat' (first
    | (he_is ite; refine he_ite (fun _ => ?_) (fun _ => ?_))
    | (he_is Pure.pure; refine he_pure _ (fun _ h => h))
    | (he_bind_is Pure.pure;
        refine he_bind (Q := fun _ => Inv _) (he_pure _ (fun _ h => h)) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.evalString;
        refine he_bind (evalString_inv $hc _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.evalValue; refine he_bind (evalValue_inv $hc _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.evalPayload;
        refine he_bind (evalPayload_inv $hc _ _ _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.queueError;
        refine he_bind (queueError_inv $hc _) (fun _ => ?_))
    | (he_bind_is Vertep.Interp.emit; refine he_bind (emit_inv _ trivial) (fun _ => ?_))
    | (he_is_match; split)))

theorem startInvoke_inv (hc : ChartOk Γ cc) (s index : Nat) (v : Invoke)
    (hid : ∀ l, v.idlocation = some l → l ∈ cc.locations)
    (hv : ∀ l, v.idlocation = some l → StrLoc Γ l) :
    HE (Inv Γ) (Interp.startInvoke cc s index v hid) (fun _ => Inv Γ) := by
  unfold Interp.startInvoke
  refine he_bind he_read (fun env => ?_)
  dsimp only
  split
  · inv_auto hc
  · refine he_bind (he_get (Q := fun s' s => s' = s ∧ Inv Γ s) (fun s h => ⟨rfl, h⟩))
      (fun st0 => ?_)
    refine he_bind (he_set _ (Q := fun _ => Inv Γ) (fun ⟨s, hs⟩ => by
      obtain ⟨rfl, hI⟩ := hs; exact hI)) (fun _ => ?_)
    split
    · rename_i loc hl
      refine he_bind (assignValue_inv hc _ _ _ (hv loc hl _)) (fun _ => ?_)
      inv_auto hc
    · inv_auto hc

theorem guard_inv (hc : ChartOk Γ cc) (t : Nat) :
    HE (Inv Γ) (Interp.guard cc t) (fun _ => Inv Γ) := by
  unfold Interp.guard
  split
  · exact he_pure _ (fun _ h => h)
  · refine he_bind (evalCond_inv hc _) (fun r => ?_)
    split
    · exact he_pure _ (fun _ h => h)
    · exact he_bind (enqueue_inv _ (hc.errors _ _ _ (.inl rfl))) (fun _ => he_pure _ (fun _ h => h))

end Vertep.Typed
