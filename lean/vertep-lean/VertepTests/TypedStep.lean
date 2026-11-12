import VertepTests.Step
import VertepLean.TypedStep.Theorem

/-!
# Chart type soundness, applied

The theorem applied to the `ecmascript` chart of `VertepTests/Step.lean` with `x : int`, the
untyped typing under which every chart is checked, and the values the core generates itself:
a bare `<assign>`, an `idlocation` and `foreach` indices write values that a typed location may
not accept, which is why `ChartOk` (and the Rust checker) checks them.
-/

namespace Vertep.Test

open Typed Types

/-- `x : int`, and the literal `1` is an `int`. -/
def ecmaΓ : ChartTyping where
  decls := []
  loc := fun l => if l = "x" then some (.prim .int) else none
  event := fun _ => none
  expr := fun e => if e = "1" then some (.prim .int) else none
  content := fun _ => none
  items := fun _ => none

/-- `ReqTyped` for the writes, decided with the checker. -/
def writeTypedB (Γ : ChartTyping) : Request → Bool
  | .assignValue l v | .bindVar l v =>
    match Γ.loc l with
    | some T => check Γ.decls [] T v
    | none => true
  | _ => true

theorem writeTypedB_sound (Γ : ChartTyping) (l : String) (v : Value)
    (h : writeTypedB Γ (.assignValue l v) = true) : ReqTyped Γ (.assignValue l v) := by
  intro T hT
  simp only [writeTypedB, hT] at h
  exact check_sound _ _ _ _ h

/-- The recorded tape of `ecmaChart` is typed under `ecmaΓ`: its only value answer is the
`int` `1`. -/
theorem ecmaTape_typed : TapeTyped ecmaΓ ecmaTape := by
  intro e he
  simp only [ecmaTape, List.mem_cons, List.mem_nil_iff, or_false] at he
  rcases he with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;>
    first
    | trivial
    | (intro T hT
       simp only [ecmaΓ, ↓reduceIte, Option.some.injEq] at hT
       subst hT
       exact .prim _ _ (by decide))

/-- Applied to the start of `ecmaChart` on its typed tape, the run writes `x` only with an
`int`, as the theorem says for any checked chart. -/
-- @tg: verifies:: [[openspec:formal-model#Chart Type Soundness#Typed step keeps the datamodel typed]] {level: unit}
def ecmaWritesTyped : Bool :=
  match runEvents ecmaChart ["go"] ecmaTape with
  | .ok outs => outs.all (fun o => o.requestList.all (writeTypedB ecmaΓ))
  | .error _ => false

#guard ecmaWritesTyped

/-! ## The untyped typing -/

/-- No location, event or expression is typed. -/
def untypedΓ : ChartTyping where
  decls := []
  loc := fun _ => none
  event := fun _ => none
  expr := fun _ => none
  content := fun _ => none
  items := fun _ => none

mutual
theorem untyped_exec : (e : Exec) → Typed.ExecOk untypedΓ e
  | .mk k _ => by simpa only [Typed.ExecOk] using untyped_kind k
theorem untyped_kind : (k : ExecKind) → Typed.KindOk untypedΓ k
  | .raise _ => by simp only [Typed.KindOk, PayloadTyped, untypedΓ]
  | .send _ => ⟨fun _ _ _ _ _ => (by simp only [PayloadTyped, untypedΓ]),
      fun _ _ _ _ h => (by cases h)⟩
  | .cancel _ _ => trivial
  | .log _ _ => trivial
  | .assign _ x c => by
    cases x with
    | some _ => intro _ _ h; cases h
    | none =>
      cases c with
      | some _ => intro _ _ _ h; cases h
      | none => intro _ h; cases h
  | .script _ => trivial
  | .if_ bs => by simpa only [Typed.KindOk] using untyped_branches bs
  | .foreach _ _ _ body => by
    simp only [Typed.KindOk]
    exact ⟨fun _ _ _ h => (by cases h), fun _ _ _ _ _ h => (by cases h), untyped_block body⟩
  | .extension _ => trivial
theorem untyped_block : (b : List Exec) → Typed.BlockOk untypedΓ b
  | [] => trivial
  | e :: es => ⟨untyped_exec e, untyped_block es⟩
theorem untyped_branches : (bs : List (Option Expr × List Exec)) → Typed.BranchesOk untypedΓ bs
  | [] => trivial
  | (_, b) :: bs => ⟨untyped_block b, untyped_branches bs⟩
end

/-- Every chart is checked under the untyped typing (Opt-In Chart Typing: an untyped chart has
`value` everywhere), so the theorem applies to every chart and every tape. -/
-- @tg: verifies:: [[openspec:formal-model#Chart Type Soundness#Typed step keeps the datamodel typed]] {level: unit}
theorem untyped_chartOk (cc : CChart) : ChartOk untypedΓ cc where
  data := fun _ d _ => by
    unfold Typed.DataOk
    split
    · intro _ _ _ h; cases h
    · intro _ _ _ h; cases h
    · trivial
  onentry := fun _ b _ => untyped_block b
  onexit := fun _ b _ => untyped_block b
  trans := fun t => untyped_block _
  invokes := fun _ _ _ => ⟨fun _ _ _ _ h => (by cases h), untyped_block _⟩
  done := fun _ _ _ _ => by simp only [PayloadTyped, untypedΓ]
  doneParallel := fun _ => by simp only [PayloadTyped, untypedΓ]
  errors := fun _ _ _ _ => by simp only [EventTyped, PayloadTyped, untypedΓ]
  nullContent := fun _ _ _ h => by cases h

/-- Applied to any step of any chart: every effect and queued event is typed (vacuously, as no
event is declared). -/
-- @tg: verifies:: [[openspec:formal-model#Chart Type Soundness#Sent and queued events carry their declared payloads]] {level: unit}
example (cc : CChart) (s : Session) (i : Input) (tape : Tape) (o : Out cc)
    (ht : TapeTyped untypedΓ tape) (h : step cc s i tape = .ok o) :
    (∀ f ∈ o.effects, EffectTyped untypedΓ f) ∧ SessionTyped untypedΓ o.session :=
  (step_typed (untyped_chartOk cc) s i tape o
    (fun _ _ => by simp only [EventTyped, PayloadTyped, untypedΓ])
    (fun _ _ => by simp only [EventTyped, PayloadTyped, untypedΓ]) ht h).2

/-! ## Values the core generates -/

/-- `x : int` and a bare `<assign location="x"/>` on entry: the core writes `undefined`. -/
def bareAssignChart : Chart :=
  { datamodel := "ecmascript",
    data := [{ id := "x", expr := some "1" }],
    states := [node "s0" (onentry := [[.ofKind (.assign "x" none none)]])] }

def bareAssignTape : Tape :=
  [ ⟨.initSession { sessionid := "s" }, .unit⟩, ⟨.setEnvironment {}, .unit⟩,
    ⟨.declare "x", .unit⟩, ⟨.evalValue "1", .value (.num 1 0)⟩,
    ⟨.assignValue "x" (.num 1 0), .unit⟩, ⟨.assignValue "x" .undefined, .unit⟩ ]

/-- `<send event="tick" target="#_parent" idlocation="x"/>`. -/
def tickSend : Send := { event := some "tick", target := some "#_parent", idlocation := some "x" }

/-- `x : int` and `<send event="tick" idlocation="x"/>` on entry: the core writes the generated
send identifier, a string. -/
def idlocationChart : Chart :=
  { datamodel := "ecmascript",
    data := [{ id := "x", expr := some "1" }],
    states := [node "s0" (onentry := [[.ofKind (.send tickSend)]])] }

def idlocationTape : Tape :=
  [ ⟨.initSession { sessionid := "s" }, .unit⟩, ⟨.setEnvironment {}, .unit⟩,
    ⟨.declare "x", .unit⟩, ⟨.evalValue "1", .value (.num 1 0)⟩,
    ⟨.assignValue "x" (.num 1 0), .unit⟩, ⟨.assignValue "x" (.str "s.send.1"), .unit⟩ ]

/-- The values the core generates break a typed location, so the checker must check them: the
bare `<assign>` and the `idlocation` write values that are no `int`, and `ChartOk` refuses both
(as `bare_assign_to_a_required_location_is_rejected` and `idlocation_must_hold_a_string` in
`crates/vertep-core/src/types/tests.rs`). -/
-- @tg: verifies:: [[openspec:formal-model#Chart Type Soundness#Generated values are checked]] {level: unit}
def generatedValuesBreakTyping : Bool :=
  let writes (c : Chart) (t : Tape) : List Request := match runEvents c [] t with
    | .ok outs => outs.flatMap (fun o => o.requestList)
    | .error _ => []
  (writes bareAssignChart bareAssignTape).contains (.assignValue "x" .undefined) &&
    !(writes bareAssignChart bareAssignTape).all (writeTypedB ecmaΓ) &&
  (writes idlocationChart idlocationTape).contains (.assignValue "x" (.str "s.send.1")) &&
    !(writes idlocationChart idlocationTape).all (writeTypedB ecmaΓ)

#guard generatedValuesBreakTyping

theorem bare_assign_refused : ¬ Typed.KindOk ecmaΓ (.assign "x" none none) := by
  intro h
  have hT : HasType ecmaΓ.decls .undefined (.prim .int) := h (.prim .int) (by simp [ecmaΓ])
  cases hT with
  | prim _ _ hp => simp [primOk] at hp

theorem idlocation_refused : ¬ Typed.KindOk ecmaΓ (.send { idlocation := some "x" }) := by
  intro h
  have hT : HasType ecmaΓ.decls (.str "s.send.1") (.prim .int) :=
    h.2 "x" rfl "s.send.1" (.prim .int) (by simp [ecmaΓ])
  cases hT with
  | prim _ _ hp => simp [primOk] at hp

/-- A `foreach` index is an `int`: a `string` index location is refused. -/
theorem foreach_index_refused :
    ¬ Typed.KindOk { ecmaΓ with loc := fun _ => some (.prim .string) }
      (.foreach "xs" "y" (some "i") []) := by
  intro h
  have hT : HasType [] (Value.ofNat 0) (.prim .string) := h.2.1 "i" rfl 0 (by decide) _ rfl
  cases hT with
  | prim _ _ hp => simp [primOk, Value.ofNat] at hp

end Vertep.Test
