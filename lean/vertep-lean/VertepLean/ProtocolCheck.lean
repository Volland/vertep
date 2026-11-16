import Lean.Data.Json
import VertepLean.ProtocolTheorems

/-!
# The protocol history checker (m10 8.4)

`vertep-kolyada` replays the histories one participant of a Kolyada instance recorded
(`crates/vertep-kolyada/tests/histories.rs`) through the model of `VertepLean.Protocol` in strict
mode. Line 1 is the `setup`: the protocol, the bindings, the participant's DID, the genesis, the
participant's projection and its status, knowledge and heads. Every later line is one of

* `receive` (a message and the time it arrived): the events must be `receive`'s;
* `hidden` (the hash of a message hidden from the participant): the events must be
  `observeHidden`'s;
* `expire` (a time): the events must be `expire`'s;
* `send` (the participant's own message): it must be accepted by `accept` in the state before,
  and its dependencies must be the heads.

After every line the status, the knowledge and the heads must be the model's (the knowledge and
heads compared as sets), and the setup's projection must be `project` of the local role. The
checker reports the first line that differs.

```
vertep-kolyada --history HISTORY.jsonl [HISTORY.jsonl …]
```

For each file it prints `ok: <file>: <n> lines` (to standard output) or
`error: <file>:<line>: <kind>: <reason>` (to standard error), where `<kind>` is the line's `kind`
(`setup` for line 1, `parse` for a line that is not JSON). It exits `0` when every history is
accepted, `1` when one is not, and `2` on a usage error.

The checker is sound (`replay_sound`): an accepted history ends in a `Reachable` state, so the
theorems of `VertepLean.ProtocolTheorems` hold in it.
-/

namespace Vertep.Kolyada.Check

open Lean (Json)
open Vertep.Kolyada

/-- A failing line: its number, its `kind`, the reason. -/
structure Failure where
  num : Nat
  kind : String
  reason : String
  deriving Repr, DecidableEq

/-! ## Reading JSON -/

def field (j : Json) (k : String) : Except String Json :=
  match j.getObjVal? k with
  | .ok v => pure v
  | .error _ => throw s!"missing field {k}"

def str (j : Json) (k : String) : Except String String := do
  match (← field j k).getStr? with
  | .ok s => pure s
  | .error _ => throw s!"field {k} is not a string"

def nat (j : Json) (k : String) : Except String Nat := do
  match (← field j k).getNat? with
  | .ok n => pure n
  | .error _ => throw s!"field {k} is not a natural number"

def bool (j : Json) (k : String) : Except String Bool := do
  match (← field j k).getBool? with
  | .ok b => pure b
  | .error _ => throw s!"field {k} is not a boolean"

def strs (j : Json) (k : String) : Except String (List String) := do
  match (← field j k).getArr? with
  | .ok a => a.toList.mapM (fun x => match x.getStr? with
      | .ok s => pure s
      | .error _ => throw s!"field {k} is not a list of strings")
  | .error _ => throw s!"field {k} is not a list of strings"

def arr (j : Json) (k : String) : Except String (List Json) := do
  match (← field j k).getArr? with
  | .ok a => pure a.toList
  | .error _ => throw s!"field {k} is not a list"

def transitionFromJson (j : Json) : Except String Transition := do
  pure ⟨← str j "from", ← str j "sender", ← strs j "receivers", ← str j "message", ← str j "to"⟩

def protocolFromJson (j : Json) : Except String Protocol := do
  let roles ← strs j "roles"
  unless roles.Nodup do throw "a role is declared twice"
  pure { id := ← str j "id", roles, messages := ← strs j "messages", initial := ← str j "initial",
         finals := ← strs j "finals",
         transitions := ← (← arr j "transitions").mapM transitionFromJson,
         bufferSize := ← nat j "bufferSize", timeoutMs := ← nat j "timeoutMs" }

def msgFromJson (j : Json) : Except String Msg := do
  let inst ← match ← field j "instance" with
    | .null => pure none
    | .str s => pure (some s)
    | _ => throw "field instance is not a string or null"
  pure { hash := ← str j "hash", sigOk := ← bool j "sig", protocol := ← str j "protocol", inst,
         sender := ← str j "sender", role := ← str j "role", seq := ← nat j "seq",
         deps := ← strs j "deps", recipients := ← strs j "recipients", kind := ← str j "type",
         payloadEmpty := ← bool j "payloadEmpty", shapeOk := ← bool j "shape" }

def bindingsFromJson (j : Json) : Except String (List (Role × Did)) := do
  match j with
  | .obj kvs => kvs.toList.mapM (fun (k, v) => match v.getStr? with
      | .ok d => pure (k, d)
      | .error _ => throw s!"the binding of {k} is not a DID")
  | _ => throw "bindings is not an object"

def stepFromJson (j : Json) : Except String Step := do
  let a ← field j "action"
  let action ← match a with
    | .str "silent" => pure Action.silent
    | _ => match a.getObjValAs? String "send", a.getObjValAs? String "receive" with
      | .ok m, _ => pure (Action.send m (← strs a "receivers"))
      | _, .ok m => pure (Action.receive m (← str a "sender"))
      | _, _ => throw "unknown projection action"
  pure ⟨← str j "from", ← str j "to", action⟩

def evFromJson (j : Json) : Except String Ev := do
  let h ← str j "hash"
  match ← str j "event" with
  | "applied" => pure (.applied h)
  | "duplicate" => pure (.duplicate h)
  | "held" => pure (.held h)
  | "refused" => pure (.refused h (← str j "reason"))
  | "stalled" => pure (.stalled h (← strs j "missing"))
  | "equivocation" => pure (.equivocation h (← str j "with"))
  | e => throw s!"unknown event {e}"

def Ev.text : Ev → String
  | .applied h => s!"applied {h}"
  | .duplicate h => s!"duplicate {h}"
  | .held h => s!"held {h}"
  | .refused h r => s!"refused {h} ({r})"
  | .stalled h ms => s!"stalled {h} missing {ms}"
  | .equivocation h o => s!"equivocation {h} with {o}"

def evsText (es : List Ev) : String := "[" ++ ", ".intercalate (es.map Ev.text) ++ "]"

/-! ## Lines -/

/-- What a line asks the model to do. -/
inductive Op where
  | receive (m : Msg) (now : Nat)
  | hidden (h : Hash)
  | expire (now : Nat)
  | send (m : Msg)

/-- The state a line records after it. -/
structure After where
  status : String
  knowledge : List StateId
  heads : List Hash

/-- One line after the setup. -/
structure Entry where
  num : Nat
  kind : String
  op : Op
  events : List Ev
  after : After

/-- The setup line. -/
structure Setup where
  protocol : Protocol
  bindings : List (Role × Did)
  localDid : Did
  genesis : Msg
  projection : List Step
  after : After

def afterFromJson (j : Json) : Except String After := do
  pure ⟨← str j "status", ← strs j "knowledge", ← strs j "heads"⟩

def setupFromJson (j : Json) : Except String Setup := do
  unless (← str j "kind") == "setup" do throw "line 1 must be the setup"
  pure { protocol := ← protocolFromJson (← field j "protocol"),
         bindings := ← bindingsFromJson (← field j "bindings"), localDid := ← str j "local",
         genesis := ← msgFromJson (← field j "genesis"),
         projection := ← (← arr j "projection").mapM stepFromJson,
         after := ← afterFromJson j }

def entryFromJson (num : Nat) (kind : String) (j : Json) : Except String Entry := do
  let op ← match kind with
    | "receive" => pure (Op.receive (← msgFromJson (← field j "msg")) (← nat j "now"))
    | "hidden" => pure (Op.hidden (← str j "hash"))
    | "expire" => pure (Op.expire (← nat j "now"))
    | "send" => pure (Op.send (← msgFromJson (← field j "msg")))
    | "setup" => throw "the setup must be line 1 only"
    | k => throw s!"unknown kind {k}"
  let events ← match kind with
    | "send" => pure []
    | _ => (← arr j "events").mapM evFromJson
  pure ⟨num, kind, op, events, ← afterFromJson j⟩

/-- Parse a history: the setup and the lines, with their numbers (blank lines skipped). -/
def parse (src : String) : Except Failure (Setup × List Entry) := do
  let lines := ((src.splitOn "\n").zipIdx.map (fun (l, i) => (l, i + 1))).filter
    (fun (l, _) => !(l.all Char.isWhitespace))
  match lines with
  | [] => throw ⟨1, "setup", "empty history"⟩
  | (first, n) :: rest =>
    let σ ← match Json.parse first >>= setupFromJson with
      | .ok σ => pure σ
      | .error e => throw ⟨n, "setup", e⟩
    let es ← rest.mapM (fun (l, i) => do
      let j ← match Json.parse l with
        | .ok j => pure j
        | .error e => throw (Failure.mk i "parse" e)
      let kind ← match j.getObjValAs? String "kind" with
        | .ok k => pure k
        | .error _ => throw (Failure.mk i "parse" "missing field kind")
      match entryFromJson i kind j with
      | .ok e => pure e
      | .error e => throw (Failure.mk i kind e))
    pure (σ, es)

/-! ## Replay -/

/-- Equal as sets. -/
def sameSet {α : Type} [DecidableEq α] (a b : List α) : Bool :=
  a.all (· ∈ b) && b.all (· ∈ a)

/-- The recorded status, knowledge and heads are the model's. -/
def compareAfter (p : Protocol) (s : St) (a : After) : Except String Unit := do
  unless (s.status p).text == a.status do
    throw s!"status: the model has {(s.status p).text}, the history {a.status}"
  unless sameSet s.knowledge a.knowledge do
    throw s!"knowledge: the model has {s.knowledge}, the history {a.knowledge}"
  unless sameSet s.heads a.heads do
    throw s!"heads: the model has {s.heads}, the history {a.heads}"

/-- The model's next receiver for an operation, with its events. -/
def perform (p : Protocol) (x : Rx) : Op → Except String (Rx × List Ev)
  | .receive m now => pure (receive p x m now)
  | .hidden h => pure (observeHidden p x h)
  | .expire now => pure (expire p x now)
  | .send m =>
    match accept p x.st m with
    | .accepted =>
      if sameSet m.deps x.st.heads then pure (⟨apply p x.st m, x.held⟩, [])
      else throw s!"the dependencies {m.deps} are not the heads {x.st.heads}"
    | .duplicate => throw "send refused: duplicate"
    | .refused r => throw s!"send refused: {r.text}"

/-- Replay one line: the model's next receiver, or why the line differs from the model. -/
def replayLine (p : Protocol) (x : Rx) (e : Entry) : Except String Rx :=
  match perform p x e.op with
  | .error r => .error r
  | .ok (y, evs) =>
    if evs == e.events then
      match compareAfter p y.st e.after with
      | .ok _ => .ok y
      | .error r => .error r
    else .error s!"events: the model has {evsText evs}, the history {evsText e.events}"

/-- Replay lines from `x`; the receiver reached, or the first failing line. -/
def replayFrom (p : Protocol) : Rx → List Entry → Except Failure Rx
  | x, [] => .ok x
  | x, e :: es =>
    match replayLine p x e with
    | .ok y => replayFrom p y es
    | .error r => .error ⟨e.num, e.kind, r⟩

/-- The receiver after the setup, or why the setup differs from the model. -/
def start (σ : Setup) : Except String Rx := do
  let p := σ.protocol
  match init p σ.bindings σ.localDid σ.genesis with
  | none => throw "the model refuses the genesis"
  | some s =>
    match project p s.me with
    | none => throw s!"{s.me} is not a role"
    | some steps =>
      unless sameSet steps σ.projection do
        throw s!"projection of {s.me}: the model has {repr steps}, the history {repr σ.projection}"
    compareAfter p s σ.after
    pure ⟨s, []⟩

/-- Parse and replay a history; the number of lines, setup included. -/
def check (src : String) : Except Failure Nat := do
  let (σ, es) ← parse src
  let x ← match start σ with
    | .ok x => pure x
    | .error e => throw (Failure.mk 1 "setup" e)
  let _ ← replayFrom σ.protocol x es
  pure (es.length + 1)

/-! ## Soundness -/

/-- One pass of `drainPass` keeps the state reachable. -/
theorem drainPass_reachable {p : Protocol} :
    ∀ (hs : List (Msg × Nat)) {s : St}, Reachable p s → Reachable p (drainPass p s hs).1
  | [], s, hr => hr
  | (m, t) :: rest, s, hr => by
    unfold drainPass
    split
    · exact drainPass_reachable rest hr
    · rename_i ha
      exact drainPass_reachable rest (Reachable.apply hr ha)
    · exact drainPass_reachable rest hr
    · exact drainPass_reachable rest hr

theorem drainN_reachable {p : Protocol} :
    ∀ (n : Nat) (hs : List (Msg × Nat)) {s : St}, Reachable p s → Reachable p (drainN p n s hs).1
  | 0, hs, s, hr => hr
  | n + 1, hs, s, hr => by
    unfold drainN
    dsimp only
    split
    · exact drainN_reachable n _ (drainPass_reachable hs hr)
    · exact drainPass_reachable hs hr

theorem receive_reachable {p : Protocol} {x : Rx} {m : Msg} {now : Nat}
    (hr : Reachable p x.st) : Reachable p (receive p x m now).1.st := by
  unfold receive
  split
  · exact hr
  · rename_i ha
    exact drainN_reachable _ _ (Reachable.apply hr ha)
  · split
    · exact hr
    · split
      · split <;> exact hr
      · exact hr
  · exact hr

theorem observeHidden_reachable {p : Protocol} {x : Rx} {h : Hash}
    (hr : Reachable p x.st) : Reachable p (observeHidden p x h).1.st := by
  unfold observeHidden
  split
  · exact hr
  · rename_i hn
    exact drainN_reachable _ _ (Reachable.hide hr hn)

theorem perform_reachable {p : Protocol} {x y : Rx} {evs : List Ev} :
    ∀ {op : Op}, Reachable p x.st → perform p x op = .ok (y, evs) → Reachable p y.st
  | .receive m now, hr, h => by
    have h' : receive p x m now = (y, evs) := by injection h
    rw [← show (receive p x m now).1 = y from congrArg Prod.fst h']
    exact receive_reachable hr
  | .hidden hh, hr, h => by
    have h' : observeHidden p x hh = (y, evs) := by injection h
    rw [← show (observeHidden p x hh).1 = y from congrArg Prod.fst h']
    exact observeHidden_reachable hr
  | .expire now, hr, h => by
    have h' : expire p x now = (y, evs) := by injection h
    rw [← show (expire p x now).1 = y from congrArg Prod.fst h']
    exact hr
  | .send m, hr, h => by
    simp only [perform] at h
    split at h
    · rename_i ha
      split at h
      · have h' : ((⟨apply p x.st m, x.held⟩ : Rx), ([] : List Ev)) = (y, evs) := by injection h
        rw [← show (⟨apply p x.st m, x.held⟩ : Rx) = y from congrArg Prod.fst h']
        exact Reachable.apply hr ha
      · cases h
    · cases h
    · cases h

theorem replayLine_reachable {p : Protocol} {x y : Rx} {e : Entry} (hr : Reachable p x.st)
    (h : replayLine p x e = .ok y) : Reachable p y.st := by
  unfold replayLine at h
  split at h
  · cases h
  · rename_i y' evs hp
    split at h
    · split at h
      · cases h
        exact perform_reachable hr hp
      · cases h
    · cases h

/-- **The checker is sound.** A replay that is accepted from a reachable state ends in a
reachable state, so every theorem of the model holds in it. -/
theorem replay_sound {p : Protocol} :
    ∀ (es : List Entry) {x y : Rx}, Reachable p x.st → replayFrom p x es = .ok y →
      Reachable p y.st
  | [], x, y, hr, h => by simp only [replayFrom, Except.ok.injEq] at h; exact h ▸ hr
  | e :: es, x, y, hr, h => by
    simp only [replayFrom] at h
    cases hl : replayLine p x e with
    | error r => rw [hl] at h; cases h
    | ok x' =>
      rw [hl] at h
      exact replay_sound es (replayLine_reachable hr hl) h

/-! ## The command line -/

/-- Check one file; the line to print and whether it was accepted. -/
def checkFile (path : String) : IO (Bool × String) := do
  match ← (IO.FS.readFile path).toBaseIO with
  | .error e => pure (false, s!"error: {path}:0: io: {e}")
  | .ok src =>
    match check src with
    | .ok n => pure (true, s!"ok: {path}: {n} lines")
    | .error f => pure (false, s!"error: {path}:{f.num}: {f.kind}: {f.reason}")

/-- The command line: `vertep-kolyada --history FILE [FILE …]`. -/
-- @tg: implements:: [[openspec:protocol-conformance#Lean Protocol Model]]
def run (args : List String) : IO UInt32 := do
  let files := match args with
    | "--history" :: fs => fs
    | _ => []
  if files.isEmpty then
    IO.eprintln "usage: vertep-kolyada --history HISTORY.jsonl [HISTORY.jsonl …]"
    return 2
  let mut code : UInt32 := 0
  for f in files do
    let (ok, msg) ← checkFile f
    if ok then
      IO.println msg
    else
      IO.eprintln msg
      code := 1
  return code

end Vertep.Kolyada.Check
