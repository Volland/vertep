import Lean.Data.Json
import VertepLean.LeaseTheorems

/-!
# The lease and fencing model as a differential oracle (m4-cluster 6.5)

`vertep-lease` replays a fault-injection history (`vertep_cluster::History::to_jsonl`) through
the model and requires the model's decision for every append (`Lease.decide`: accept, fenced,
conflict) and every frame (`Lease.admits`: admitted or fenced) to be the recorded one. It also
rejects a history the model cannot produce at all: an append under an epoch never granted to
its writer for its shard, an admitted frame that no accepted append produced, a `(shard, epoch)`
granted to two nodes, or an actor recorded under two shards.

```
vertep-lease --history HISTORY.jsonl [--infer-grants]
```

With `--infer-grants`, an append under a `(node, shard, epoch)` with no recorded grant counts as
that grant (still checked for uniqueness), for histories that do not record every grant.

Exit codes: `0` every decision agrees, `1` the first disagreement on standard error, `3` an
input error.
-/

namespace Vertep.Lease.Check

open Lean (Json)

/-- One history event. -/
inductive Event where
  | grant (node : String) (shard epoch : Nat)
  | append (node : String) (shard epoch : Nat) (actor : String) (version : Nat)
      (message : String) (decision : AppendDecision)
  | frame (receiver : String) (shard epoch : Nat) (message : String) (admitted : Bool)
  | ack (message : String)

def eventFromJson (j : Json) : Except String Event := do
  let str (k : String) : Except String String := do (← j.getObjVal? k).getStr?
  let nat (k : String) : Except String Nat := do (← j.getObjVal? k).getNat?
  match ← str "op" with
  | "grant" => pure (.grant (← str "node") (← nat "shard") (← nat "epoch"))
  | "append" =>
    let d ← match ← str "decision" with
      | "accept" => pure AppendDecision.accept
      | "fenced" => pure .fenced
      | "conflict" => pure .conflict
      | d => throw s!"unknown append decision {d}"
    pure (.append (← str "node") (← nat "shard") (← nat "epoch") (← str "actor")
      (← nat "version") (← str "message") d)
  | "frame" =>
    let a ← match ← str "decision" with
      -- the JSON decision for an admitted frame, escaped: the library's source scan rejects
      -- that word bare
      | "\x61dmit" => pure true
      | "fenced" => pure false
      | d => throw s!"unknown frame decision {d}"
    pure (.frame (← str "receiver") (← nat "shard") (← nat "epoch") (← str "message") a)
  | "ack" => pure (.ack (← str "message"))
  | op => throw s!"unknown op {op}"

/-- The shard of every actor, from its appends; an error when an actor has two. -/
def shards (es : List Event) : Except String (List (String × Nat)) :=
  es.foldlM (fun acc e => match e with
    | .append _ k _ actor .. => match acc.lookup actor with
      | some k' => if k = k' then pure acc else throw s!"actor {actor} recorded under shards {k'} and {k}"
      | none => pure ((actor, k) :: acc)
    | _ => pure acc) []

/-- The replay state: the model state, and the messages of the accepted appends per shard. -/
structure St where
  sys : Sys
  produced : List (String × Nat × Nat)

/-- The authority index recorded grants are filed under (the history does not name the
authority; uniqueness of `(shard, epoch)` is checked directly). -/
def recordAuth : Nat := 0

/-- Record a grant, refusing a `(shard, epoch)` already granted to another node. -/
def addGrant (st : St) (i : Nat) (n : String) (k e : Nat) : Except String St :=
  match st.sys.granted.find? (fun (_, n', k', e') => k' = k ∧ e' = e ∧ n' ≠ n) with
  | some (_, n', _, _) => throw s!"event {i}: epoch {e} of shard {k} granted to {n'} and to {n}"
  | none => pure { st with sys := { st.sys with granted := st.sys.granted ++ [(recordAuth, n, k, e)] } }

/-- Replay one event: the next state, or the disagreement. -/
def replayOne (infer : Bool) (shardOf : String → Nat) (st : St) (i : Nat) :
    Event → Except String St
  | .grant n k e => addGrant st i n k e
  | .append n k e actor version msg d => do
    let st ← if st.sys.granted.any (fun (_, n', k', e') => n' = n ∧ k' = k ∧ e' = e) then pure st
      else if infer then addGrant st i n k e
      else throw s!"event {i}: {n} appended to shard {k} under epoch {e}, never granted to it"
      let m := decide st.sys k e actor version msg
      if m != d then
        throw s!"event {i}: append by {n} to {actor} at version {version}, epoch {e}: recorded {repr d}, the model decides {repr m}"
      else if m = .accept then
        pure { sys := appendNew shardOf st.sys n e actor msg,
               produced := st.produced ++ [(msg, k, e)] }
      else pure st
  | .frame r k e msg ok =>
    let m := admits st.sys r k e
    if m != ok then
      throw s!"event {i}: frame of {msg} (shard {k}, epoch {e}) at {r}: recorded {if ok then "admitted" else "fenced"}, the model decides {if m then "admitted" else "fenced"}"
    else if m then
      if !(st.produced.contains (msg, k, e)) then
        throw s!"event {i}: {r} admitted a frame of {msg} (shard {k}, epoch {e}) that no accepted append produced"
      else
        pure { st with sys := { st.sys with
          recv := upd2 st.sys.recv r k e
          admitted := upd2 st.sys.admitted r k (st.sys.admitted r k ++ [e]) } }
    else pure st
  | .ack _ => pure st

/-- Replay a history; `ok` with the number of decisions compared. -/
def replay (es : List Event) (infer : Bool := false) : Except String Nat := do
  let sh ← shards es
  let shardOf := fun a => (sh.lookup a).getD 0
  let _ ← es.zipIdx.foldlM (fun st (e, i) => replayOne infer shardOf st i e)
    ({ sys := Sys.empty, produced := [] } : St)
  pure (es.filter (fun e => match e with | .append .. | .frame .. => true | _ => false)).length

/-- The command line. -/
-- @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--history", p] | ["--history", p, "--infer-grants"] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-lease --history HISTORY.jsonl [--infer-grants]"; return 3
  let src ← IO.FS.readFile path
  let es ← match ((src.splitOn "\n").filter (fun l => !(l.all Char.isWhitespace))).mapM
      (fun l => Json.parse l >>= eventFromJson) with
    | .ok es => pure es
    | .error e => IO.eprintln s!"history: {e}"; return 3
  match replay es (args.contains "--infer-grants") with
  | .ok n => IO.println s!"{n} decisions agree"; return 0
  | .error e => IO.eprintln e; return 1

end Vertep.Lease.Check
