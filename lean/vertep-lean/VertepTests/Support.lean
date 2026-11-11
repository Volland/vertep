import VertepLean

/-! Builders for test charts. -/

namespace Vertep.Test

/-- A transition. -/
def tr (events : List String) (targets : List String) (cond : Option String := none)
    (body : Block := []) (type : TransitionType := .external) : Transition :=
  { events, targets, cond, body, type }

/-- A state of any element with the given children and transitions. -/
def node (id : String) (el : StateElement := .state) (children : List State := [])
    (ts : List Transition := []) (onentry : List Block := []) (onexit : List Block := [])
    (initial : List String := []) (data : List Data := []) (invokes : List Invoke := [])
    (donedata : Option DoneData := none) : State :=
  .mk id false el initial none children ts onentry onexit data invokes donedata []

/-- `<log label="..."/>`, used to observe execution order. -/
def logE (label : String) : Exec := .ofKind (.log (some label) none)

/-- `<raise event="..."/>`. -/
def raiseE (ev : String) : Exec := .ofKind (.raise ev)

instance {cc : CChart} : Inhabited (Out cc) :=
  ⟨{ session := default, effects := [],
     trace := { index := 0, input := .cancel, microsteps := [], configuration := [],
                effects := [], terminated := false },
     requests := [], tape := [] }⟩

/-- Compile a chart, panicking on ill-formed charts (tests only). -/
def cc (c : Chart) : CChart :=
  match compile c with
  | .ok x => x
  | .error _ => default

/-- Identifiers of the active states. -/
def ids (cc : CChart) (s : Session) : List String :=
  s.config.map (fun i => (cc.st i).id)

/-- Log labels among the effects. -/
def logs (es : List Effect) : List String :=
  es.filterMap (fun e => match e with
    | .log (some l) _ => some l
    | _ => none)

/-- Active state ids after `start`, or `["<error>"]`. -/
def startIds (c : Chart) (tape : Tape := []) : List String :=
  match start (cc c) { info := { sessionid := "s" } } tape with
  | .ok o => ids (cc c) o.session
  | .error _ => ["<error>"]

/-- Start and then feed events; the outputs in order. -/
def runEvents (c : Chart) (evs : List String) (tape : Tape := []) :
    Except StepError (List (Out (cc c))) := do
  let o ← start (cc c) { info := { sessionid := "s" } } tape
  let mut outs := [o]
  let mut cur := o
  for e in evs do
    let o' ← step (cc c) cur.session { kind := .event { name := e } } cur.tape
    outs := outs ++ [o']
    cur := o'
  pure outs

/-- Active state ids after start and the events. -/
def idsAfter (c : Chart) (evs : List String) (tape : Tape := []) : List String :=
  match runEvents c evs tape with
  | .ok outs => match outs.getLast? with
    | some o => ids (cc c) o.session
    | none => []
  | .error _ => ["<error>"]

/-- Log labels of start and all events, concatenated. -/
def logsAfter (c : Chart) (evs : List String) (tape : Tape := []) : List String :=
  match runEvents c evs tape with
  | .ok outs => outs.flatMap (fun o => logs o.effects)
  | .error _ => ["<error>"]

/-- All effects of start and the events. -/
def effectsAfter (c : Chart) (evs : List String) (tape : Tape := []) : List Effect :=
  match runEvents c evs tape with
  | .ok outs => outs.flatMap (·.effects)
  | .error _ => []

end Vertep.Test
