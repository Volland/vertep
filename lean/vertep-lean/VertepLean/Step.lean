import VertepLean.Compiled
import VertepLean.Select

/-!
# The executable step

W3C SCXML Appendix D (`interpret`, `mainEventLoop`, `microstep`, `exitStates`, `enterStates`,
`exitInterpreter`) over a compiled chart, with the datamodel replaced by the answer tape of
`VertepLean.Tape` (design D5) and every interaction reported as an `Effect` (design D3).

Two monads keep the algorithm honest about what can change what:

* `EM` runs executable content and datamodel requests. It reads the configuration (for `In()`
  in the `null` datamodel) but cannot change it; it owns the tape, the request log, the
  effects, the internal queue and the identifier counter.
* `M` runs the interpreter proper: configuration, history values, invocations, the states to
  invoke and the running flag, lifting `EM` calls with the current configuration.

Every datamodel request that writes a location carries a proof that the location is declared by
the chart (`LocOk`), which is what the dynamic type-soundness theorem reads off.

Deviations from the literal pseudo-code, all invisible on well-formed charts or recorded here:
* recursion in the entry-set computation and in `isInFinalState` is bounded by fuel derived from
  the number of states (enough for every well-formed chart; exhaustion is `illFormed`);
* the macrostep loop is bounded by `1000 * (budget + 1)` iterations as well as by the microstep
  budget (`budgetExceeded` either way); the budget charges every microstep record, selections
  that fire transitions and internal events consumed without enabling any (Rust `charge`), and
  its exhaustion stops the macrostep where it is, leaving the partial result (`stepFull`);
* a history value is used only when non-empty, otherwise the default transition is taken.
-/

namespace Vertep

/-! ## Locations declared by a chart -/

mutual
/-- Locations an executable content element (and everything nested in it) may write. -/
def Exec.locs : Exec → List String
  | .mk k _ => ExecKind.locs k
/-- Locations an executable content element may write. -/
def ExecKind.locs : ExecKind → List String
  | .assign l _ _ => [l]
  | .send s => s.idlocation.toList
  | .if_ bs => branchLocs bs
  | .foreach _ item idx body => item :: (idx.toList ++ blockLocs body)
  | .raise _ => []
  | .cancel _ _ => []
  | .log _ _ => []
  | .script _ => []
  | .extension _ => []
/-- Locations a block may write. -/
def blockLocs : List Exec → List String
  | [] => []
  | e :: es => Exec.locs e ++ blockLocs es
/-- Locations the branches of an `if` may write. -/
def branchLocs : List (Option Expr × List Exec) → List String
  | [] => []
  | (_, b) :: bs => blockLocs b ++ branchLocs bs
end

/-- Locations declared or written by one compiled state: its data items, its `onentry` and
`onexit` content, and its invokes' `idlocation` and `finalize` content. -/
def CState.locs (s : CState) : List String :=
  s.data.map (·.id) ++ s.onentry.flatMap blockLocs ++ s.onexit.flatMap blockLocs ++
    s.invokes.flatMap (fun i => i.idlocation.toList ++ blockLocs i.finalize)

/-- Every data item identifier and location expression of the chart. -/
def CChart.locations (cc : CChart) : List String :=
  cc.states.toList.flatMap CState.locs ++ cc.trans.toList.flatMap (fun t => blockLocs t.body)

/-- A request whose written location, if any, is declared by the chart. -/
def LocOk (cc : CChart) (r : Request) : Prop := ∀ l, r.writes = some l → l ∈ cc.locations

/-- A request together with the proof that it only writes declared locations. -/
abbrev Req (cc : CChart) := { r : Request // LocOk cc r }

theorem CChart.st_locs (cc : CChart) (i : Nat) : ∀ l ∈ (cc.st i).locs, l ∈ cc.locations := by
  intro l hl
  unfold CChart.st at hl
  rw [Array.getD_eq_getD_getElem?] at hl
  unfold CChart.locations
  apply List.mem_append_left
  rw [List.mem_flatMap]
  cases h : cc.states[i]? with
  | none =>
    rw [h] at hl
    simp [CState.locs, CState.empty] at hl
  | some s =>
    rw [h] at hl
    refine ⟨s, ?_, hl⟩
    exact Array.mem_toList_iff.2 (Array.mem_of_getElem? h)

theorem CChart.tr_locs (cc : CChart) (t : Nat) : ∀ l ∈ blockLocs (cc.tr t).body, l ∈ cc.locations := by
  intro l hl
  unfold CChart.tr at hl
  rw [Array.getD_eq_getD_getElem?] at hl
  unfold CChart.locations
  apply List.mem_append_right
  rw [List.mem_flatMap]
  cases h : cc.trans[t]? with
  | none =>
    rw [h] at hl
    simp [CTrans.empty, blockLocs] at hl
  | some x =>
    rw [h] at hl
    exact ⟨x, Array.mem_toList_iff.2 (Array.mem_of_getElem? h), hl⟩

theorem CChart.onentry_locs (cc : CChart) (i : Nat) :
    ∀ b ∈ (cc.st i).onentry, ∀ l ∈ blockLocs b, l ∈ cc.locations := by
  intro b hb l hl
  apply cc.st_locs i
  simp only [CState.locs, List.mem_append, List.mem_flatMap]
  exact Or.inl (Or.inl (Or.inr ⟨b, hb, hl⟩))

theorem CChart.onexit_locs (cc : CChart) (i : Nat) :
    ∀ b ∈ (cc.st i).onexit, ∀ l ∈ blockLocs b, l ∈ cc.locations := by
  intro b hb l hl
  apply cc.st_locs i
  simp only [CState.locs, List.mem_append, List.mem_flatMap]
  exact Or.inl (Or.inr ⟨b, hb, hl⟩)

theorem CChart.data_locs (cc : CChart) (i : Nat) :
    ∀ d ∈ (cc.st i).data, d.id ∈ cc.locations := by
  intro d hd
  apply cc.st_locs i
  simp only [CState.locs, List.mem_append, List.mem_map]
  exact Or.inl (Or.inl (Or.inl ⟨d, hd, rfl⟩))

theorem CChart.invoke_locs (cc : CChart) (i : Nat) :
    ∀ v ∈ (cc.st i).invokes, (∀ l, v.idlocation = some l → l ∈ cc.locations) ∧
      ∀ l ∈ blockLocs v.finalize, l ∈ cc.locations := by
  intro v hv
  constructor
  · intro l hl
    apply cc.st_locs i
    simp only [CState.locs, List.mem_append, List.mem_flatMap]
    exact Or.inr ⟨v, hv, Or.inl (by simp [hl])⟩
  · intro l hl
    apply cc.st_locs i
    simp only [CState.locs, List.mem_append, List.mem_flatMap]
    exact Or.inr ⟨v, hv, Or.inr hl⟩

/-! ## Sessions and traces -/

/-- A started invocation (Rust `Invocation`). -/
structure Invocation where
  state : Nat
  index : Nat
  invokeid : String
  autoforward : Bool
  completed : Bool
  deriving Repr, BEq, DecidableEq, Inhabited

/-- The session value (Rust `Session`, design D1): the configuration as state indices in
ascending (document) order, never containing the root; history values; the internal queue;
started invocations; late-binding status; the send and invoke counters; the number of completed
macrosteps; the options. -/
structure Session where
  sessionid : String
  terminated : Bool
  config : List Nat
  history : List (Nat × List Nat)
  internal : List Event
  invocations : List Invocation
  initialized : List Nat
  sendCounter : Nat
  invokeCounter : Nat
  macrosteps : Nat
  options : Options
  deriving Repr, BEq, Inhabited

/-- A declared transition: source state identifier and position among its transitions. -/
structure TransitionRef where
  source : String
  index : Nat
  deriving Repr, BEq, DecidableEq

/-- What started a macrostep (Rust `TraceInput`). -/
inductive TraceInput where
  | start (sessionid : String)
  | event (e : Event)
  | cancel
  deriving Repr, BEq

/-- One selection round (Rust `MicrostepTrace`). -/
structure MicrostepTrace where
  event : Option String
  transitions : List TransitionRef
  deriving Repr, BEq

/-- The record of one macrostep (Rust `MacrostepTrace`). -/
structure MacrostepTrace where
  index : Nat
  input : TraceInput
  microsteps : List MicrostepTrace
  configuration : List String
  effects : List Effect
  terminated : Bool
  deriving Repr, BEq

/-- State owned by executable content and the datamodel bridge. -/
structure ExecSt (cc : CChart) where
  tape : Tape
  log : Array (Req cc)
  effects : Array Effect
  internal : List Event
  sendCounter : Nat
  invokeCounter : Nat
  /-- The error message of the most recent datamodel call, if it failed (Rust `Capture`). -/
  lastError : Option String := none

/-- State owned by the interpreter (the rest of Rust's `Session` and `Driver`). -/
structure Core where
  sessionid : String
  terminated : Bool
  config : List Nat
  history : List (Nat × List Nat)
  invocations : List Invocation
  initialized : List Nat
  macrosteps : Nat
  options : Options
  /-- States entered in this macrostep, to invoke at its end. -/
  toInvoke : List Nat
  /-- Cleared when a top-level final state is entered. -/
  running : Bool
  /-- Microsteps taken in this macrostep. -/
  used : Nat
  /-- The reachable session identifiers of the current input, if the host listed them. -/
  sessions : Option (List String)
  micros : Array MicrostepTrace
  terminatedNow : Bool
  /-- The microstep budget ran out: the macrostep stopped where it was (Rust `Err(Budget)`). -/
  exhausted : Bool := false
  deriving Inhabited

/-- What executable content may read: the configuration (for `In()`) and the session id. -/
structure Env where
  config : List Nat
  sessionid : String
  options : Options
  sessions : Option (List String) := none

/-- The datamodel and effect state monad. -/
abbrev SM (cc : CChart) := StateT (ExecSt cc) (Except StepError)
/-- Executable content. -/
abbrev EM (cc : CChart) := ReaderT Env (SM cc)
/-- The interpreter. -/
abbrev M (cc : CChart) := StateT Core (SM cc)

/-- The result of `start` and `step` (Rust `Outcome` plus the datamodel side): the new session,
the effects in order, the macrostep trace, the datamodel requests issued in order (each with its
proof that it writes only declared locations), and the unconsumed tape. -/
structure Out (cc : CChart) where
  session : Session
  effects : List Effect
  trace : MacrostepTrace
  requests : List (Req cc)
  tape : Tape

/-- The requests without their proofs, as written to traces. -/
def Out.requestList {cc : CChart} (o : Out cc) : List Request := o.requests.map Subtype.val

/-- The type URI of the SCXML Event I/O Processor. -/
def scxmlEventProcessor : String := "http://www.w3.org/TR/scxml/#SCXMLEventProcessor"
/-- The type URI of SCXML invocations. -/
def scxmlInvokeType : String := "http://www.w3.org/TR/scxml/"

namespace Interp

variable (cc : CChart)

/-! ## The datamodel bridge -/

/-- Rust `parse_in_predicate`: `In('id')` or `In("id")`, whitespace allowed around the parts. -/
def parseIn (e : String) : Option String :=
  match (trimWs e).toList with
  | 'I' :: 'n' :: rest =>
    match (rest.dropWhile isWs) with
    | '(' :: r =>
      match r.reverse with
      | ')' :: inner =>
        let inner := (trimWs (String.ofList inner.reverse)).toList
        match inner with
        | q :: r2 =>
          if q != '\'' && q != '"' then none
          else match r2.reverse with
            | q' :: x =>
              if q' != q then none
              else
                let id := x.reverse
                if id.isEmpty || id.contains q then none else some (String.ofList id)
            | [] => none
        | [] => none
      | _ => none
    | _ => none
  | _ => none

/-- The `null` datamodel (Rust `NullDatamodel`), answered natively against the configuration. -/
def nullAnswer (config : List Nat) : Request → Answer
  | .initSession _ => .unit
  | .setEnvironment _ => .unit
  | .setEvent _ => .unit
  | .evalCond e => match parseIn e with
    | some x => .bool (match cc.indexOf x with
      | some i => config.contains i
      | none => false)
    | none => .error "the null datamodel does not support conditions other than In()"
  | .evalContent c => .value (.str c)
  | .isLocation _ => .bool false
  | .declare _ => .error "the null datamodel does not support data"
  | .evalValue _ => .error "the null datamodel does not support value expressions"
  | .assignExpr .. => .error "the null datamodel does not support assign"
  | .assignValue .. => .error "the null datamodel does not support assign"
  | .bindVar .. => .error "the null datamodel does not support variables"
  | .execScript _ => .error "the null datamodel does not support script"
  | .foreachItems _ => .error "the null datamodel does not support foreach"
  | .restore .undefined => .unit
  | .restore _ => .error "a null datamodel snapshot is undefined"

/-- Rust `Capture`: the error remembered after a call (`set_environment` and `set_event` keep
it, `is_location` clears it, every other call records its own result). -/
def noteError (r : Request) (a : Answer) (last : Option String) : Option String :=
  match r with
  | .setEnvironment _ => last
  | .setEvent _ => last
  | .isLocation _ => none
  | _ => match a with
    | .error m => some m
    | _ => none

/-- Issue one datamodel request: answered natively for the `null` datamodel, otherwise taken
from the tape, which must record exactly this request next with an answer of the right shape. -/
def request (r : Request) (h : LocOk cc r) : EM cc Answer := do
  let env ← read
  if cc.datamodel == "null" then
    let a := nullAnswer cc env.config r
    modify fun st => { st with log := st.log.push ⟨r, h⟩, lastError := noteError r a st.lastError }
    pure a
  else
    let st ← get
    match st.tape with
    | [] => throw (.tapeExhausted r)
    | e :: rest =>
      if e.request == r then
        if Answer.wellShaped r e.answer then
          set { st with tape := rest, log := st.log.push ⟨r, h⟩,
                        lastError := noteError r e.answer st.lastError }
          pure e.answer
        else throw (.badAnswer r e.answer)
      else throw (.tapeMismatch r e.request)

/-- A request that writes nothing. -/
def ask (r : Request) (h : r.writes = none) : EM cc Answer :=
  request cc r (fun l hl => by rw [h] at hl; cases hl)

/-- The message of an `error.execution` the core raises itself. -/
def executionErrorMessage : String := "execution error"
/-- The message of an `error.communication` for a session the host says does not exist. -/
def communicationErrorMessage : String := "target session does not exist"

/-- Rust `platform_error`: a platform event with data `{kind, message}`, `kind` being the name
after `error.`. -/
def platformError (name message : String) (sendid : Option String) : Event :=
  let kind := if name.startsWith "error." then String.ofList (name.toList.drop 6) else name
  { name, type := .platform, sendid,
    data := .object [("kind", .str kind), ("message", .str message)] }

/-- Queue `error.execution` (Rust `queue_error`): its message is the error of the most recent
datamodel call if that call failed (and is then forgotten), else `execution error`. -/
def queueError (sendid : Option String := none) : EM cc Unit :=
  modify fun st => { st with
    internal := st.internal ++
      [platformError "error.execution" (st.lastError.getD executionErrorMessage) sendid],
    lastError := none }

/-- Queue an event. -/
def enqueue (e : Event) : EM cc Unit :=
  modify fun st => { st with internal := st.internal ++ [e] }

/-- Emit an effect. -/
def emit (e : Effect) : EM cc Unit :=
  modify fun st => { st with effects := st.effects.push e }

/-- `eval_value`. -/
def evalValue (e : Expr) : EM cc (Option Value) := do
  match ← ask cc (.evalValue e) rfl with
  | .value v => pure (some v)
  | _ => pure none

/-- `eval_content`. -/
def evalContent (text : String) : EM cc (Option Value) := do
  match ← ask cc (.evalContent text) rfl with
  | .value v => pure (some v)
  | _ => pure none

/-- `eval_value` followed by Rust `value_to_string`. -/
def evalString (e : Expr) : EM cc (Option String) := do
  pure ((← evalValue cc e).bind Value.toStr)

/-- `eval_cond`. -/
def evalCond (e : Expr) : EM cc (Except String Bool) := do
  match ← ask cc (.evalCond e) rfl with
  | .bool b => pure (.ok b)
  | .error m => pure (.error m)
  | _ => pure (.error executionErrorMessage)

/-- `assign_value`; `false` on an error. -/
def assignValue (loc : String) (v : Value) (h : loc ∈ cc.locations) : EM cc Bool := do
  match ← request cc (.assignValue loc v) (fun l hl => by cases hl; exact h) with
  | .unit => pure true
  | _ => pure false

/-- `bind_var`; `false` on an error. -/
def bindVar (name : String) (v : Value) (h : name ∈ cc.locations) : EM cc Bool := do
  match ← request cc (.bindVar name v) (fun l hl => by cases hl; exact h) with
  | .unit => pure true
  | _ => pure false

/-- An attribute given literally or as an expression (evaluated to a string): `some none` when
absent, `none` when the expression failed. -/
def attr (lit : Option String) (ex : Option Expr) : EM cc (Option (Option String)) :=
  match lit, ex with
  | some v, _ => pure (some (some v))
  | none, some e => do pure ((← evalString cc e).map some)
  | none, none => pure (some none)

/-- Rust `parse_delay`: a CSS2 time (`5s`, `250ms`, `1.5s`) or a bare number of milliseconds,
fractions truncated to whole milliseconds. -/
def parseDelay (text : String) : Option Nat :=
  let t := (trimWs text).toList
  let (num, factor) :=
    if t.reverse.take 2 == ['s', 'm'] then (t.take (t.length - 2), 1000 / 1000)
    else if t.reverse.take 1 == ['s'] then (t.take (t.length - 1), 1000)
    else (t, 1)
  let num := (trimWs (String.ofList num)).toList
  if num.isEmpty then none
  else
    let (int, frac) := match num.span (· != '.') with
      | (i, '.' :: f) => (i, f)
      | (i, _) => (i, [])
    if (int.isEmpty && frac.isEmpty) || !int.all Char.isDigit || !frac.all Char.isDigit then none
    else
      let digit := fun (c : Char) => c.toNat - '0'.toNat
      let ms := int.foldl (fun n c => 10 * n + digit c) 0 * factor
      let (ms, _) := frac.foldl (fun (acc : Nat × Nat) c =>
        let (ms, scale) := acc
        let scale := scale / 10
        if scale == 0 then (ms, 0) else (ms + digit c * scale, scale)) (ms, factor)
      some ms

/-- Rust `eval_payload`: the content's value if there is content; otherwise undefined without
`namelist` and `<param>`, else an object of the `namelist` then `<param>` values. -/
def evalPayload (names : List String) (ps : List Param) (c : Option Content) :
    EM cc (Option Value) := do
  match c with
  | some c =>
    match c.expr, c.body with
    | some x, _ => evalValue cc x
    | none, some b => evalContent cc b
    | none, none => pure (some .undefined)
  | none =>
    if names.isEmpty && ps.isEmpty then return some .undefined
    let nl ← names.foldlM (fun acc n => match acc with
      | none => pure none
      | some acc => do pure ((← evalValue cc n).map (fun v => acc ++ [(n, v)]))) (some [])
    let some nl := nl | return none
    let pl ← ps.foldlM (fun acc p => match acc with
      | none => pure none
      | some acc =>
        match p.expr, p.location with
        | some x, _ => do pure ((← evalValue cc x).map (fun v => acc ++ [(p.name, v)]))
        | none, some l => do pure ((← evalValue cc l).map (fun v => acc ++ [(p.name, v)]))
        | none, none => pure (some (acc ++ [(p.name, .undefined)]))) (some nl)
    pure (pl.map Value.object)

/-- Rust `done_data`: undefined without `<donedata>`; an error queues `error.execution`. -/
def doneData (dd : Option DoneData) : EM cc Value := do
  match dd with
  | none => pure .undefined
  | some dd =>
    match ← evalPayload cc [] dd.params dd.content with
    | some v => pure v
    | none => queueError cc; pure .undefined

/-- Rust `init_data_with`: each item from the invoker's `given` value, else `expr`, else inline
content, else `src` (unresolved, an error); an error queues `error.execution` and moves on to
the next item. -/
def initData (items : List Data) (h : ∀ d ∈ items, d.id ∈ cc.locations)
    (invoker : List (String × Value) := []) : EM cc Unit :=
  items.attach.forM fun ⟨d, hd⟩ => do
    let ok ← match invoker.lookup d.id with
    | some v => assignValue cc d.id v (h d hd)
    | none => match d.expr, d.content, d.src with
      | some e, _, _ => do
        match ← evalValue cc e with
        | some v => assignValue cc d.id v (h d hd)
        | none => pure false
      | none, some c, _ => do
        match ← evalContent cc c with
        | some v => assignValue cc d.id v (h d hd)
        | none => pure false
      | none, none, some _ => pure false
      | none, none, none => pure true
    if !ok then queueError cc

/-- The send identifier: declared (visible), or generated as `<sessionid>.send.<n>` and stored
at `idlocation` if given (visible then); `none` when storing failed (error already queued). -/
def sendId (s : Send) (hid : ∀ l, s.idlocation = some l → l ∈ cc.locations) :
    EM cc (Option (String × Bool)) := do
  match s.id with
  | some id => pure (some (id, true))
  | none =>
    let env ← read
    let st ← get
    set { st with sendCounter := st.sendCounter + 1 }
    let id := s!"{env.sessionid}.send.{st.sendCounter + 1}"
    match hl : s.idlocation with
    | some loc =>
      if ← assignValue cc loc (.str id) (hid loc hl) then pure (some (id, true))
      else do queueError cc (some id); pure none
    | none => pure (some (id, false))

/-- Rust `exec_send` (W3C SCXML 6.2): identifier (stored at `idlocation` first), event name,
target, type, delay, payload, then routing. -/
def execSend (s : Send) (hid : ∀ l, s.idlocation = some l → l ∈ cc.locations) :
    EM cc Bool := do
  let env ← read
  let some (sendid, visible) ← sendId cc s hid | pure false
  let fail : EM cc Bool := do queueError cc (some sendid); pure false
  let some name ← attr cc s.event s.eventexpr | fail
  let some target ← attr cc s.target s.targetexpr | fail
  let some type ← attr cc s.type s.typeexpr | fail
  let type := type.getD scxmlEventProcessor
  let some delayText ← attr cc s.delay s.delayexpr | fail
  let some delayMs := (match delayText with
    | none => some 0
    | some t => parseDelay t) | fail
  let some data ← evalPayload cc s.namelist s.params s.content | fail
  if !env.options.sendTypes.contains type then return (← fail)
  let scxml := type == scxmlEventProcessor || type == "scxml"
  if scxml then
    match target with
    | some t => if !(t.startsWith "#_" || t.toList.contains ':') then return (← fail)
    | none => pure ()
  -- a session the host says does not exist: error.communication at once, the block continues
  if scxml && delayMs == 0 then
    match target, env.sessions with
    | some t, some known =>
      if t.startsWith "#_scxml_" then
        let id := String.ofList (t.toList.drop "#_scxml_".length)
        if !known.contains id then
          enqueue cc (platformError "error.communication" communicationErrorMessage (some sendid))
          return true
    | _, _ => pure ()
  let exposed := if visible then some sendid else none
  if target == some "#_internal" && delayMs == 0 then
    enqueue cc { name := name.getD "", type := .internal, sendid := exposed, data }
    return true
  emit cc (.send
    { sendid, target, type, delayMs,
      event := { name := name.getD "", type := .external, sendid := exposed,
                 origin := if scxml then some s!"#_scxml_{env.sessionid}" else none,
                 origintype := if scxml then some scxmlEventProcessor else none,
                 invokeid := none, data } })
  pure true

/-! ## Executable content (Rust `exec`, `exec_block`) -/

mutual
/-- Run one element; `false` when it failed (its `error.execution` is already queued) and the
enclosing block must stop. -/
def execOne : (e : Exec) → (∀ l ∈ e.locs, l ∈ cc.locations) → EM cc Bool
  | .mk k _, h => execKind k (fun l hl => h l (by simpa [Exec.locs] using hl))
/-- Run one element by kind. -/
def execKind : (k : ExecKind) → (∀ l ∈ k.locs, l ∈ cc.locations) → EM cc Bool
  | .raise ev, _ => do enqueue cc { name := ev, type := .internal }; pure true
  | .send s, h => execSend cc s (fun l hl => h l (by simp [ExecKind.locs, hl]))
  | .cancel sid sidexpr, _ => do
    match sid, sidexpr with
    | some id, _ => emit cc (.cancel id); pure true
    | none, some x =>
      match ← evalString cc x with
      | some id => emit cc (.cancel id); pure true
      | none => queueError cc; pure false
    | none, none => queueError cc; pure false
  | .log label ex, _ => do
    match ex with
    | none => emit cc (.log label none); pure true
    | some x =>
      match ← evalValue cc x with
      | some v => emit cc (.log label (some v)); pure true
      | none => queueError cc; pure false
  | .assign loc ex content, h => do
    let hloc : loc ∈ cc.locations := h loc (by simp [ExecKind.locs])
    let ok ← match ex, content with
      | some x, _ => do
        match ← request cc (.assignExpr loc x) (fun l hl => by cases hl; exact hloc) with
        | .unit => pure true
        | _ => pure false
      | none, some c => do
        match ← evalContent cc c with
        | some v => assignValue cc loc v hloc
        | none => pure false
      | none, none => assignValue cc loc .undefined hloc
    if ok then pure true else do queueError cc; pure false
  | .script sc, _ => do
    match sc.body with
    | some b =>
      match ← ask cc (.execScript b) rfl with
      | .unit => pure true
      | _ => queueError cc; pure false
    | none => queueError cc; pure false
  | .if_ bs, h => execBranches bs (fun l hl => h l (by simpa [ExecKind.locs] using hl))
  | .foreach arr item idx body, h => do
    let hitem : item ∈ cc.locations := h item (by simp [ExecKind.locs])
    let hbody : ∀ l ∈ blockLocs body, l ∈ cc.locations :=
      fun l hl => h l (by simp [ExecKind.locs, hl])
    match ← ask cc (.foreachItems arr) rfl with
    | .values items =>
      items.zipIdx.foldlM (fun ok (v, k) => do
        if !ok then return false
        if !(← bindVar cc item v hitem) then
          queueError cc
          return false
        let okIdx ← match hidx : idx with
          | some i => bindVar cc i (Value.ofNat k) (h i (by simp [ExecKind.locs, hidx]))
          | none => pure true
        if !okIdx then
          queueError cc
          return false
        execBlock body hbody) true
    | _ => queueError cc; pure false
  | .extension _, _ => pure true
/-- Run a block until an element fails. -/
def execBlock : (b : List Exec) → (∀ l ∈ blockLocs b, l ∈ cc.locations) → EM cc Bool
  | [], _ => pure true
  | e :: es, h => do
    if ← execOne e (fun l hl => h l (by simp [blockLocs, hl])) then
      execBlock es (fun l hl => h l (by simp [blockLocs, hl]))
    else pure false
/-- Run the first branch whose condition holds; a failing condition fails the `if`. -/
def execBranches : (bs : List (Option Expr × List Exec)) →
    (∀ l ∈ branchLocs bs, l ∈ cc.locations) → EM cc Bool
  | [], _ => pure true
  | (c, b) :: bs, h => do
    let take ← match c with
      | none => pure (some true)
      | some e => do pure (← evalCond cc e).toOption
    match take with
    | none => queueError cc; pure false
    | some true => execBlock b (fun l hl => h l (by simp [branchLocs, hl]))
    | some false => execBranches bs (fun l hl => h l (by simp [branchLocs, hl]))
end

/-- Run a top-level block (`let _ = exec_block(..)`). -/
def execTop (b : List Exec) (h : ∀ l ∈ blockLocs b, l ∈ cc.locations) : EM cc Unit := do
  let _ ← execBlock cc b h
  pure ()

/-- Rust `invoke`: identifier (generated as `<state>.<n>` and stored at `idlocation`), type,
source, content and parameters; on success the invocation to record, after emitting its
effect. -/
def startInvoke (s index : Nat) (v : Invoke)
    (hid : ∀ l, v.idlocation = some l → l ∈ cc.locations) : EM cc (Option Invocation) := do
  let env ← read
  let stateId := (cc.st s).id
  let invokeid? ← match v.id with
    | some id => pure (some id)
    | none => do
      let st ← get
      set { st with invokeCounter := st.invokeCounter + 1 }
      let id := s!"{stateId}.{st.invokeCounter + 1}"
      match hl : v.idlocation with
      | some loc =>
        if ← assignValue cc loc (.str id) (hid loc hl) then pure (some id)
        else do queueError cc; pure none
      | none => pure (some id)
  let some invokeid := invokeid? | pure none
  let type ← match v.type, v.typeexpr with
    | some t, _ => pure (some t)
    | none, some x => evalString cc x
    | none, none => pure (some scxmlInvokeType)
  let src ← match v.src, v.srcexpr with
    | some t, _ => pure (some (some t))
    | none, some x => do pure ((← evalString cc x).map some)
    | none, none => pure (some none)
  let content ← match v.content with
    | some { expr := some x, .. } => do pure ((← evalValue cc x).map some)
    | some { expr := none, body := some b } => pure (some (some (Value.str b)))
    | _ => pure (some none)
  let params ← evalPayload cc v.namelist v.params none
  match type, src, content, params with
  | some type, some src, some content, some params =>
    if !env.options.invokeTypes.contains type then
      queueError cc
      return none
    emit cc (.invoke { invokeid, state := stateId, type, src, content, params,
                       autoforward := v.autoforward })
    pure (some { state := s, index, invokeid, autoforward := v.autoforward, completed := false })
  | _, _, _, _ => queueError cc; pure none

/-! ## The interpreter (Rust `Driver`) -/

/-- Run executable content with the current configuration. -/
def liftE {α : Type} (x : EM cc α) : M cc α := do
  let core ← get
  StateT.lift (x.run { config := core.config, sessionid := core.sessionid,
                       options := core.options, sessions := core.sessions })

/-- Run a list of top-level blocks. -/
def runBlocks (bs : List Block) (h : ∀ b ∈ bs, ∀ l ∈ blockLocs b, l ∈ cc.locations) :
    M cc Unit :=
  bs.attach.forM (fun ⟨b, hb⟩ => liftE cc (execTop cc b (h b hb)))

/-- `set_event`. -/
def setEvent (e : Event) : M cc Unit := do
  let _ ← liftE cc (ask cc (.setEvent e) rfl)
  pure ()

/-- Whether transition `t` matches: eventless (`ev = none`), or with a descriptor matching the
event name. -/
def matchP (ev : Option String) (t : Nat) : Bool :=
  match ev with
  | none => (cc.tr t).events.isEmpty
  | some n => !(cc.tr t).events.isEmpty && nameMatch (cc.tr t).events n

/-- The condition of a transition during selection: absent is true; an error queues
`error.execution` and counts as false. -/
def guard (t : Nat) : EM cc Bool :=
  match (cc.tr t).cond with
  | none => pure true
  | some e => do
    match ← evalCond cc e with
    | .ok b => pure b
    | .error m => enqueue cc (platformError "error.execution" m none); pure false

/-- Appendix D `selectEventlessTransitions` (`ev = none`) and `selectTransitions` (`ev = some
name`), including `removeConflictingTransitions` (Rust `kernel::select_transitions`). The loops
are `Select.choices` and `Select.removeConflicting`, the definitions the selection theorem is
about. -/
def select (ev : Option String) : M cc (List Nat) := do
  let core ← get
  let env : Env := { config := core.config, sessionid := core.sessionid, options := core.options,
                     sessions := core.sessions }
  let chs ← StateT.lift (Select.choices cc.candidates
    (fun _ _ t => if matchP cc ev t then (guard cc t).run env else pure false)
    (core.config.filter cc.isAtomic) 0)
  pure (Select.removeConflicting (cc.conflict core.config core.history) cc.srcDesc
    (Select.enabledOf chs))

/-- Record one selection round in the trace. -/
def record (ev : Option String) (ts : List Nat) : M cc Unit :=
  let refs : List TransitionRef := ts.filterMap (fun t => (cc.tr t).index.map (fun k =>
    ({ source := (cc.st (cc.tr t).source).id, index := k } : TransitionRef)))
  let m : MicrostepTrace := { event := ev, transitions := refs }
  modify fun core => { core with micros := core.micros.push m }

/-- Rust `cancel_invocations`: drop the invocations of `s`, cancelling those not completed. -/
def cancelInvocations (s : Nat) : M cc Unit := do
  let core ← get
  let (mine, rest) := core.invocations.partition (fun i => i.state == s)
  set { core with invocations := rest }
  mine.forM (fun i => if i.completed then pure () else liftE cc (emit cc (.cancelInvoke i.invokeid)))

/-- Rust `record_history`. -/
def recordHistory (exitSet : List Nat) : M cc Unit :=
  exitSet.forM fun s => (cc.st s).histories.forM fun h => do
    let core ← get
    let v := if cc.isDeepHistory h then
        core.config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
      else core.config.filter (fun x => (cc.st x).parent == some s)
    set { core with history := (h, v) :: core.history.filter (fun e => e.1 != h) }

/-- Appendix D `exitStates` (Rust `exit_states`). -/
def exitStates (ts : List Nat) : M cc Unit := do
  let core ← get
  let exitSet := cc.exitSet core.config core.history ts
  set { core with toInvoke := core.toInvoke.filter (fun s => !exitSet.contains s) }
  recordHistory cc exitSet
  for s in exitSet.reverse do
    runBlocks cc (cc.st s).onexit (cc.onexit_locs s)
    cancelInvocations cc s
    modify fun core => { core with config := core.config.erase s }

/-- Insert into an ascending list without duplicates. -/
def insertSorted (x : Nat) : List Nat → List Nat
  | [] => [x]
  | y :: ys => if x < y then x :: y :: ys else if x == y then y :: ys else y :: insertSorted x ys

/-- Run the body of transition `t`. -/
def runTrans (t : Nat) : M cc Unit := liftE cc (execTop cc (cc.tr t).body (cc.tr_locs t))

/-- Appendix D `enterStates` (Rust `enter_states`); `dhist` is the history the transitions'
domains are computed under (the history before the exit, see `CChart.entrySet`). -/
def enterStates (dhist : List (Nat × List Nat)) (ts : List Nat) : M cc Unit := do
  let core ← get
  let some acc := cc.entrySet core.history dhist ts
    | throw (.illFormed "entry set recursion exhausted")
  for s in acc.toEnter.mergeSort (· ≤ ·) do
    modify fun core => { core with config := insertSorted s core.config,
                                   toInvoke := insertSorted s core.toInvoke }
    let core ← get
    if cc.binding == .late && !core.initialized.contains s then
      set { core with initialized := insertSorted s core.initialized }
      liftE cc (initData cc (cc.st s).data (cc.data_locs s))
    runBlocks cc (cc.st s).onentry (cc.onentry_locs s)
    if acc.defaultEntry.contains s then
      match (cc.st s).initial with
      | some i => runTrans cc i
      | none => pure ()
    match acc.histContent.lookup s with
    | some d => runTrans cc d
    | none => pure ()
    if cc.isFinal s then
      let parent := ((cc.st s).parent).getD 0
      if parent == 0 then
        modify fun core => { core with running := false }
      else
        let data ← liftE cc (doneData cc (cc.st s).donedata)
        liftE cc (enqueue cc { name := s!"done.state.{(cc.st parent).id}", type := .platform,
                               data })
        match (cc.st parent).parent with
        | some gp =>
          let core ← get
          if gp != 0 && cc.isParallel gp &&
              (cc.st gp).children.all (fun c => cc.inFinal core.config c) then
            liftE cc (enqueue cc { name := s!"done.state.{(cc.st gp).id}", type := .platform })
        | none => pure ()

/-- Rust `charge`: count one microstep record (a selection that fires transitions, or an
internal event consumed without enabling any) against the budget; `false`, with the macrostep
marked `exhausted`, when the budget has run out. -/
def charge : M cc Bool := do
  let core ← get
  if core.used ≥ core.options.microstepBudget then
    set { core with exhausted := true }
    return false
  set { core with used := core.used + 1 }
  return true

/-- Appendix D `microstep` (Rust `microstep`), counted against the budget. -/
def microstep (ev : Option String) (ts : List Nat) : M cc Unit := do
  if !(← charge cc) then return
  record cc ev ts
  let dhist := (← get).history
  exitStates cc ts
  for t in ts do runTrans cc t
  enterStates cc dhist ts

/-- Rust `start_invokes`: the states entered in this macrostep, in entry order. -/
def startInvokes : M cc Unit := do
  let core ← get
  for s in core.toInvoke do
    for (v, k) in (cc.st s).invokes.attach.zipIdx do
      match ← liftE cc (startInvoke cc s k v.1 ((cc.invoke_locs s v.1 v.2).1)) with
      | some inv => modify fun core => { core with invocations := core.invocations ++ [inv] }
      | none => pure ()
  modify fun core => { core with toInvoke := [] }

/-- Dequeue one internal event. -/
def dequeue : M cc (Option Event) := do
  let st ← getThe (ExecSt cc)
  match st.internal with
  | [] => pure none
  | e :: rest =>
    set (σ := ExecSt cc) { st with internal := rest }
    pure (some e)

/-- Appendix D `exitInterpreter` (Rust `exit_interpreter`). -/
def exitInterpreter (cancelled : Bool) : M cc Unit := do
  let core ← get
  let mut data := Value.undefined
  for s in core.config.reverse do
    runBlocks cc (cc.st s).onexit (cc.onexit_locs s)
    cancelInvocations cc s
    modify fun core => { core with config := core.config.erase s }
    if cc.isFinal s && (cc.st s).parent == some 0 then
      data ← liftE cc (doneData cc (cc.st s).donedata)
  liftE cc (emit cc (.sessionDone data cancelled))
  modifyThe (ExecSt cc) fun st => { st with internal := [] }
  modify fun core => { core with toInvoke := [], terminated := true, running := false,
                                 terminatedNow := true }

/-- Rust `run_macrostep`: the inner loop of `mainEventLoop` until the macrostep is complete,
then the invokes, repeated while invoking queued internal events. `fuel` bounds the iterations
(the microstep budget bounds the microsteps). -/
def macroLoop : Nat → M cc Unit
  | 0 => do throw (.budgetExceeded (← get).options.microstepBudget)
  | fuel + 1 => do
    if (← get).exhausted then return
    if !(← get).running then
      exitInterpreter cc false
      return
    let ts ← select cc none
    if !ts.isEmpty then
      microstep cc none ts
      macroLoop fuel
    else
      match ← dequeue cc with
      | some e =>
        setEvent cc e
        let ts ← select cc (some e.name)
        if ts.isEmpty then
          if ← charge cc then record cc (some e.name) []
        else microstep cc (some e.name) ts
        macroLoop fuel
      | none =>
        startInvokes cc
        if (← getThe (ExecSt cc)).internal.isEmpty then return
        macroLoop fuel

/-- Run a macrostep to completion. -/
def runMacrostep : M cc Unit := do
  let core ← get
  macroLoop cc (1000 * (core.options.microstepBudget + 1))

/-- Rust `run_external`. -/
def runExternal (ev : Event) : M cc Unit := do
  -- every delivered event is processed; only active invocations run finalize or forward
  if ev.name.startsWith "done.invoke." then
    let rest := String.ofList (ev.name.toList.drop "done.invoke.".length)
    modify fun core => { core with invocations := core.invocations.map (fun i =>
      if i.invokeid == rest || ev.invokeid == some i.invokeid then { i with completed := true }
      else i) }
  setEvent cc ev
  let core ← get
  let invs := core.invocations.mergeSort (fun a b => a.state < b.state ||
    (a.state == b.state && a.index ≤ b.index))
  for inv in invs do
    if ev.invokeid == some inv.invokeid then
      match h : (cc.st inv.state).invokes[inv.index]? with
      | some v =>
        have hv : v ∈ (cc.st inv.state).invokes := List.mem_of_getElem? h
        liftE cc (execTop cc v.finalize ((cc.invoke_locs inv.state v hv).2))
      | none => pure ()
    if inv.autoforward && !inv.completed then
      liftE cc (emit cc (.forward inv.invokeid ev))
  let ts ← select cc (some ev.name)
  if ts.isEmpty then record cc (some ev.name) []
  else microstep cc (some ev.name) ts
  runMacrostep cc

/-- Rust `run_start`: create every data item, initialize per the binding mode, run the global
script, enter the initial configuration, run the first macrostep. -/
def runStart (invoker : List (String × Value)) : M cc Unit := do
  for i in List.range cc.size do
    for ⟨d, hd⟩ in (cc.st i).data.attach do
      let h := cc.data_locs i d hd
      match ← liftE cc (request cc (.declare d.id) (fun l hl => by cases hl; exact h)) with
      | .unit => pure ()
      | _ => liftE cc (queueError cc)
  match cc.binding with
  | .early =>
    for i in List.range cc.size do
      liftE cc (initData cc (cc.st i).data (cc.data_locs i) (if i == 0 then invoker else []))
      modify fun core => { core with initialized := insertSorted i core.initialized }
  | .late =>
    liftE cc (initData cc (cc.st 0).data (cc.data_locs 0) invoker)
    modify fun core => { core with initialized := insertSorted 0 core.initialized }
  match cc.script with
  | some sc =>
    match sc.body with
    | some b =>
      match ← liftE cc (ask cc (.execScript b) rfl) with
      | .unit => pure ()
      | _ => liftE cc (queueError cc)
    | none => liftE cc (queueError cc)
  | none => pure ()
  match (cc.st 0).initial with
  | some t => enterStates cc (← get).history [t]
  | none => pure ()
  runMacrostep cc

/-- Assemble the result (Rust `Driver::finish`). -/
def finish (input : TraceInput) (core : Core) (ex : ExecSt cc) : Out cc :=
  { session :=
      { sessionid := core.sessionid, terminated := core.terminated, config := core.config,
        history := core.history, internal := ex.internal, invocations := core.invocations,
        initialized := core.initialized, sendCounter := ex.sendCounter,
        invokeCounter := ex.invokeCounter, macrosteps := core.macrosteps + 1,
        options := core.options },
    effects := ex.effects.toList,
    trace :=
      { index := core.macrosteps, input, microsteps := core.micros.toList,
        configuration := core.config.map (fun i => (cc.st i).id), effects := ex.effects.toList,
        terminated := core.terminatedNow },
    requests := ex.log.toList,
    tape := ex.tape }

/-- Run an interpreter body from a session and a tape: whether the microstep budget ran out, and
the result (partial when it did, like Rust's `BudgetExceeded { partial }`). -/
def runFull (input : TraceInput) (body : M cc Unit) (s : Session) (tape : Tape)
    (sessions : Option (List String)) : Except StepError (Bool × Out cc) :=
  let core : Core :=
    { sessionid := s.sessionid, terminated := s.terminated, config := s.config,
      history := s.history, invocations := s.invocations, initialized := s.initialized,
      macrosteps := s.macrosteps, options := s.options, toInvoke := [], running := true,
      used := 0, sessions := sessions, micros := #[], terminatedNow := false }
  let ex : ExecSt cc :=
    { tape, log := #[], effects := #[], internal := s.internal, sendCounter := s.sendCounter,
      invokeCounter := s.invokeCounter }
  match (body.run core).run ex with
  | .ok (((), core), ex) => .ok (core.exhausted, finish cc input core ex)
  | .error e => .error e

/-- Run an interpreter body from a session and a tape; budget exhaustion is an error. -/
def run (input : TraceInput) (body : M cc Unit) (s : Session) (tape : Tape)
    (sessions : Option (List String)) : Except StepError (Out cc) :=
  match runFull cc input body s tape sessions with
  | .ok (true, _) => .error (.budgetExceeded s.options.microstepBudget)
  | .ok (false, o) => .ok o
  | .error e => .error e

/-- The start body: `init_session` (a failure aborts the start), `set_environment`, then
`runStart`. -/
def startBody (si : StartInput) : M cc Unit := do
  match ← liftE cc (ask cc (.initSession si.info) rfl) with
  | .unit => pure ()
  | .error m => throw (.initFailed m)
  | _ => throw (.initFailed "unexpected answer")
  let _ ← liftE cc (ask cc (.setEnvironment si.env) rfl)
  runStart cc si.data

/-- The step body: `set_environment`, then cancel or the external event. -/
def stepBody (i : Input) : M cc Unit := do
  let _ ← liftE cc (ask cc (.setEnvironment i.env) rfl)
  match i.kind with
  | .cancel => exitInterpreter cc true
  | .event e => runExternal cc e

end Interp

/-- A fresh session value before the start macrostep (Rust `new_session`). -/
def Session.fresh (sessionid : String) (options : Options) : Session :=
  { sessionid, terminated := false, config := [], history := [], internal := [],
    invocations := [], initialized := [], sendCounter := 0, invokeCounter := 0,
    macrosteps := 0, options }

/-- Start a session (Rust `session::start`, Appendix D `interpret` up to the first wait for an
external event). -/
-- @tg: implements:: [[openspec:formal-model#Executable Step Semantics]]
def start (cc : CChart) (si : StartInput) (tape : Tape) : Except StepError (Out cc) :=
  Interp.run cc (.start si.info.sessionid) (Interp.startBody cc si)
    (Session.fresh si.info.sessionid si.options) tape si.sessions

/-- Process one input (Rust `session::step`): one macrostep for an external event, or
termination for a cancel. Same inputs and outputs as the Rust step, with the datamodel replaced
by the tape: the new session, the effects, the trace, the datamodel requests and the unconsumed
tape. -/
-- @tg: implements:: [[openspec:formal-model#Executable Step Semantics]]
def step (cc : CChart) (s : Session) (i : Input) (tape : Tape) : Except StepError (Out cc) :=
  if s.terminated then .error .terminated
  else Interp.run cc (match i.kind with
      | .event e => .event e
      | .cancel => .cancel) (Interp.stepBody cc i) s tape i.sessions

/-- `start` that also returns the partial result when the microstep budget runs out (Rust
`StartError::BudgetExceeded { partial }`): whether it ran out, and the result. -/
def startFull (cc : CChart) (si : StartInput) (tape : Tape) : Except StepError (Bool × Out cc) :=
  Interp.runFull cc (.start si.info.sessionid) (Interp.startBody cc si)
    (Session.fresh si.info.sessionid si.options) tape si.sessions

/-- `step` that also returns the partial result when the microstep budget runs out (Rust
`StepError::BudgetExceeded { partial }`). -/
def stepFull (cc : CChart) (s : Session) (i : Input) (tape : Tape) :
    Except StepError (Bool × Out cc) :=
  if s.terminated then .error .terminated
  else Interp.runFull cc (match i.kind with
      | .event e => .event e
      | .cancel => .cancel) (Interp.stepBody cc i) s tape i.sessions

end Vertep
