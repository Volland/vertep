import VertepTests.Support

/-!
# The executable step

Scenario checks on the Lean `step`, several on the same charts as the Rust tests in
`crates/vertep-core/src/tests/interpretation.rs`, with the same expected results.
-/

namespace Vertep.Test

/-! ## Datamodel bridge -/

/-- `null` chart: `s0` leaves for `pass` when `In('b')` holds; `b` is in a parallel region. -/
def nullIn : Chart :=
  { states := [node "p" .parallel [node "r1" (children := [node "a", node "b0"]),
                                   node "r2" (children := [node "b"])]
                 (ts := [tr ["e"] ["pass"] (cond := some "In('b')"),
                         tr ["e"] ["fail"]]),
               node "pass", node "fail"] }

/-- A `null` chart needs no tape: `In()` is answered from the configuration, and the request
is still logged. -/
-- @tg: verifies:: [[openspec:formal-model#Executable Step Semantics#Null datamodel evaluated natively]] {level: unit}
def nullDatamodelNative : Bool :=
  match runEvents nullIn ["e"] [] with
  | .ok outs =>
    let last := outs.getLast!
    ids (cc nullIn) last.session == ["pass"] &&
      last.requestList.contains (.evalCond "In('b')") && last.tape.isEmpty
  | .error _ => false

#guard nullDatamodelNative

/-- An `ecmascript` chart: one guarded transition and an `assign` on entry. -/
def ecmaChart : Chart :=
  { datamodel := "ecmascript",
    data := [{ id := "x", expr := some "1" }],
    states := [node "s0" (ts := [tr ["go"] ["s1"] (cond := some "x > 0")]),
               node "s1" (onentry := [[.ofKind (.assign "x" (some "x + 1") none)]])] }

/-- The tape the Rust core records for `ecmaChart`: start, then event `go`. -/
def ecmaTape : Tape :=
  [ ⟨.initSession { sessionid := "s" }, .unit⟩, ⟨.setEnvironment {}, .unit⟩,
    ⟨.declare "x", .unit⟩, ⟨.evalValue "1", .value (.num 1 0)⟩,
    ⟨.assignValue "x" (.num 1 0), .unit⟩,
    ⟨.setEnvironment {}, .unit⟩, ⟨.setEvent { name := "go" }, .unit⟩,
    ⟨.evalCond "x > 0", .bool true⟩, ⟨.assignExpr "x" "x + 1", .unit⟩ ]

/-- The step consumes the recorded answers in order, and fails as soon as it requests an
operation other than the recorded one. -/
-- @tg: verifies:: [[openspec:formal-model#Executable Step Semantics#Datamodel answers from a tape]] {level: unit}
def tapeReplay : Bool :=
  let ok := match runEvents ecmaChart ["go"] ecmaTape with
    | .ok outs =>
      ids (cc ecmaChart) outs.getLast!.session == ["s1"] && outs.getLast!.tape.isEmpty &&
        (outs.flatMap (·.requestList)) == ecmaTape.map (·.request)
    | .error _ => false
  -- the recorded condition differs from the chart's: a mismatch
  let bad := ecmaTape.set 7 ⟨.evalCond "x > 1", .bool true⟩
  let mismatch := match runEvents ecmaChart ["go"] bad with
    | .error (.tapeMismatch (.evalCond "x > 0") (.evalCond "x > 1")) => true
    | _ => false
  -- the tape ends early
  let short := match runEvents ecmaChart ["go"] (ecmaTape.take 7) with
    | .error (.tapeExhausted _) => true
    | _ => false
  ok && mismatch && short

#guard tapeReplay

/-! ## Interpretation scenarios (same charts and results as the Rust tests) -/

/-- Session start enters `a` and its default child `a1`, running `onentry` in that order. -/
def initialEntry : Chart :=
  { states := [node "a" (children := [node "a1" (onentry := [[logE "enter a1"]])])
                 (onentry := [[logE "enter a"]])] }
#guard startIds initialEntry == ["a", "a1"]
#guard logsAfter initialEntry [] == ["enter a", "enter a1"]

/-- Descendant wins over ancestor. -/
def descendantWins : Chart :=
  { states := [node "a" (children := [node "a1" (ts := [tr ["e"] ["x"]])]) (ts := [tr ["e"] ["y"]]),
               node "x", node "y"] }
#guard idsAfter descendantWins ["e"] == ["x"]

/-- Two parallel regions (Rust `two_regions`). -/
def twoRegions (t1 t2 : String) : Chart :=
  { states := [node "p" .parallel
                 [node "r1" (children := [node "r1a" (ts := [tr ["e"] [t1]]), node "r1b"]),
                  node "r2" (children := [node "r2a" (ts := [tr ["e"] [t2]]), node "r2b"])],
               node "out1", node "out2"] }
#guard idsAfter (twoRegions "r1b" "r2b") ["e"] == ["p", "r1", "r1b", "r2", "r2b"]

/-- Conflict resolved by document order: only the first region's transition is taken. -/
def conflictTrace : Bool :=
  match runEvents (twoRegions "out1" "out2") ["e"] with
  | .ok outs =>
    ids (cc (twoRegions "out1" "out2")) outs.getLast!.session == ["out1"] &&
      outs.getLast!.trace.microsteps.map (·.transitions) ==
        [[({ source := "r1a", index := 0 } : TransitionRef)]]
  | .error _ => false
#guard conflictTrace

/-- Condition false skips the transition. -/
def condFalse : Chart :=
  { states := [node "s0" (ts := [tr ["e"] ["fail"] (cond := some "In('pass')"), tr ["e"] ["pass"]]),
               node "pass", node "fail"] }
#guard idsAfter condFalse ["e"] == ["pass"]

/-- Exit deepest first; transition content between exit and entry. -/
def exitOrder : Chart :=
  { states := [node "a" (children := [node "a1" (onexit := [[logE "exit a1"]])
                   (ts := [tr ["e"] ["b"] (body := [logE "transition"])])])
                 (onexit := [[logE "exit a"]]),
               node "b" (onentry := [[logE "enter b"]])] }
#guard logsAfter exitOrder ["e"] == ["exit a1", "exit a", "transition", "enter b"]

/-- Internal transition keeps the source; external re-enters it. -/
def internalTr (type : TransitionType) : Chart :=
  { states := [node "a" (children := [node "a1", node "a2"])
                 (ts := [tr ["e"] ["a2"] (type := type)])
                 (onentry := [[logE "enter a"]]) (onexit := [[logE "exit a"]])] }
#guard logsAfter (internalTr .internal) ["e"] == ["enter a"]
#guard logsAfter (internalTr .external) ["e"] == ["enter a", "exit a", "enter a"]

/-- History: shallow restores `a2` and its default child; deep restores the leaf. -/
def historyChart (h : HistoryType) : Chart :=
  { states := [node "a" (children := [node "h" (.history h),
                 node "a1" (ts := [tr ["next"] ["a2y"]]),
                 node "a2" (children := [node "a2x", node "a2y"])])
                 (ts := [tr ["out"] ["b"]]),
               node "b" (ts := [tr ["back"] ["h"]])] }
#guard idsAfter (historyChart .shallow) ["next", "out", "back"] == ["a", "a2", "a2x"]
#guard idsAfter (historyChart .deep) ["next", "out", "back"] == ["a", "a2", "a2y"]
#guard idsAfter (historyChart .shallow) ["back"] == ["a", "a1"]

/-- Regression (found by the Configuration Well-Formedness proof): a transition from inside a
parallel state with one region to its deep history, before any history is recorded, exits the
parallel state and must enter it again. With the domain recomputed on entry from the history
just recorded (Appendix D), the session ended in `["a", "a1"]` with `p` inactive; Rust test
`deep_history_of_single_region_parallel_reenters_the_parallel`. -/
def singleRegionDeepHistory : Chart :=
  { states := [node "p" .parallel [node "h" (.history .deep),
                 node "a" (children := [node "a1" (ts := [tr ["e"] ["h"]]), node "a2"])]] }
#guard checkWF singleRegionDeepHistory
#guard idsAfter singleRegionDeepHistory ["e"] == ["p", "a", "a1"]
#guard idsAfter singleRegionDeepHistory ["e", "e"] == ["p", "a", "a1"]

/-- A history state with an explicit default transition. -/
def histNode (id : String) (h : HistoryType) (targets : List String) : State :=
  .mk id false (.history h) [] none [] [tr [] targets] [] [] [] [] none []

/-- Regression (found by the Configuration Well-Formedness proof): `P`'s history defaults to
`A1`, deep inside region `R1` of the parallel `Q`; the transition `A2 -> h` has domain `A`, below
`P`, so only `A2` is exited. Entering the default with its ancestors up to `P` (Appendix D)
entered `Q` again and completed `R2` by default next to the active `B2`; the entry now stops at
the domain. Rust test `history_entry_stops_at_the_transition_domain`. -/
def historyBelowDomain : Chart :=
  { states := [node "P" (children := [histNode "h" .deep ["A1"],
      node "Q" .parallel [
        node "R1" (children := [node "A" (children := [node "A1" (ts := [tr ["g"] ["A2"]]),
                                                       node "A2" (ts := [tr ["e"] ["h"]])])]),
        node "R2" (children := [node "B1" (ts := [tr ["f"] ["B2"]]), node "B2"])]])] }
#guard checkWF historyBelowDomain
#guard idsAfter historyBelowDomain ["f", "g"] == ["P", "Q", "R1", "A", "A2", "R2", "B2"]
#guard idsAfter historyBelowDomain ["f", "g", "e"] == ["P", "Q", "R1", "A", "A1", "R2", "B2"]

/-- Final states: `done.state.a` is raised and handled; a top-level final ends the session. -/
def finals : Chart :=
  { states := [node "a" (children := [node "a1" (ts := [tr ["e"] ["f"]]), node "f" .final])
                 (ts := [tr ["done.state.a"] ["end"]]),
               node "end" .final] }
def finalsDone : Bool :=
  match runEvents finals ["e"] with
  | .ok outs => outs.getLast!.session.terminated &&
      outs.getLast!.effects == [.sessionDone .undefined false]
  | .error _ => false
#guard finalsDone

/-- Raise is internal; `send` to `#_internal` is internal; a delayed send is an effect. -/
def sends : Chart :=
  { states := [node "s" (onentry := [[raiseE "i1",
                  .ofKind (.send { event := some "i2", target := some "#_internal" }),
                  .ofKind (.send { event := some "tick", delay := some "1.5s" })]])
                 (ts := [tr ["i1"] ["t1"]]),
               node "t1" (ts := [tr ["i2"] ["t2"]]), node "t2"] }
def sendsOk : Bool :=
  match runEvents sends [] with
  | .ok outs =>
    ids (cc sends) outs.getLast!.session == ["t2"] &&
      outs.getLast!.effects.any (fun e => match e with
        | .send r => r.delayMs == 1500 && r.sendid == "s.send.2" && r.event.name == "tick"
        | _ => false)
  | .error _ => false
#guard sendsOk

/-- An eventless cycle is cut by the microstep budget. -/
def loopChart : Chart :=
  { states := [node "a" (ts := [tr [] ["b"]]), node "b" (ts := [tr [] ["a"]])] }
def budgetCut : Bool :=
  match start (cc loopChart) { info := { sessionid := "s" }, options := { microstepBudget := 50 } } [] with
  | .error (.budgetExceeded 50) => true
  | _ => false
#guard budgetCut

/-- The budget charges internal events consumed without enabling any (Rust `charge`): the
failing guard of a parallel state's eventless transition, evaluated once per active atomic
state, queues two `error.execution` per round and enables nothing, so only the budget ends the
macrostep (regression `5cbfa3338d26b03f` of the differential harness). The partial result has
one microstep record per charged round. -/
-- @tg: verifies:: [[openspec:formal-model#Differential Oracle#Budget exhaustion agrees with Rust]] {level: unit}
def idleLoop : Chart :=
  { states := [node "p" (el := .parallel) (ts := [tr [] [] (cond := some "x")])
                 (children := [node "a", node "b"])] }
def idleLoopCut : Bool :=
  let si : StartInput := { info := { sessionid := "s" }, options := { microstepBudget := 200 } }
  (match start (cc idleLoop) si [] with
   | .error (.budgetExceeded 200) => true
   | _ => false) &&
  (match startFull (cc idleLoop) si [] with
   | .ok (true, o) => o.trace.microsteps.length == 200 &&
       o.trace.microsteps.all (fun m => m.event == some "error.execution" && m.transitions.isEmpty) &&
       o.trace.configuration == ["p", "a", "b"]
   | _ => false)
#guard idleLoopCut

/-- Events after termination are rejected. -/
def terminatedRejects : Bool :=
  match runEvents finals ["e", "again"] with
  | .error .terminated => true
  | _ => false
#guard terminatedRejects

/-- Unsupported send type: `error.execution`, the session continues. -/
def badType : Chart :=
  { states := [node "s" (onentry := [[.ofKind (.send { event := some "x", type := some "nope" })]])
                 (ts := [tr ["error.execution"] ["caught"]]),
               node "caught"] }
#guard startIds badType == ["caught"]

end Vertep.Test
