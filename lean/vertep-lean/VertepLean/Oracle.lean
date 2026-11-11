import LeanRDF.Parse.NQuads.Parser
import VertepLean.Json
import VertepLean.Rdf.Reconstruct

/-!
# The differential oracle

`vertep-oracle` reads a chart as N-Quads of its projection (design D6), decodes it with
`Rdf.fromRDF`, decides `WellFormed` with `checkWF` (proven correct), compiles it, and replays a
Rust run given as `OracleRecord` JSON Lines: for each record it runs `start` or `step` on the
record's input with the record's datamodel answer tape and writes the Lean `MacrostepTrace`.
With `--compare` it compares each Lean trace with the record's and reports the first differing
record. A run whose datamodel requests differ from the tape (other requests, fewer, or more) is a
difference too.

```
vertep-oracle --chart CHART.nq --records RECORDS.jsonl [--out TRACES.jsonl] [--compare]
```

Exit codes: `0` traces produced (and, with `--compare`, all equal); `1` a record differs (or the
Lean step failed); `3` an input error (unreadable files, a graph that is not the projection of a
chart, a chart that is not well-formed, malformed JSON).
-/

namespace Vertep.Oracle

open Lean (Json)

/-- Read a chart from N-Quads (graph names ignored): decode, check, compile. -/
def loadChart (src : String) : Except String (Chart × CChart) := do
  let ds ← match LeanRDF.NQuads.parse src with
    | .ok d => pure d
    | .error e => throw s!"N-Quads: {e}"
  let triples := ds.toList.map (·.triple)
  let (c, _) ← Rdf.fromRDF triples
  let cc ← compile c
  pure (c, cc)

/-- The outcome of replaying one record. -/
inductive Replay where
  /-- The Lean trace, the record's trace, and the next session. -/
  | ok (lean : MacrostepTrace) (rust : MacrostepTrace) (next : Option Session)
  /-- The Lean step failed or issued other datamodel requests than the tape records. -/
  | failed (msg : String) (rust : MacrostepTrace)

/-- Replay one record from the current session. -/
def replay (cc : CChart) (cur : Option Session) (r : Json.OracleRecord) : Replay :=
  let res := match r.input, cur with
    | .start si, _ => start cc si r.tape
    | .step i, some s => step cc s i r.tape
    | .step _, none => .error (.illFormed "a step record before the start record")
  match res with
  | .error e => .failed s!"the Lean step failed: {repr e}" r.trace
  | .ok o =>
    if o.requestList != r.tape.map (·.request) then
      .failed s!"datamodel requests differ from the tape: Lean issued {o.requestList.length}, the tape records {r.tape.length}" r.trace
    else .ok o.trace r.trace (some o.session)

/-- Replay every record; the Lean traces (or failures) in order. -/
def replayAll (cc : CChart) (rs : List Json.OracleRecord) : List Replay :=
  (rs.foldl (fun (acc : List Replay × Option Session) r =>
    let x := replay cc acc.2 r
    match x with
    | .ok _ _ next => (acc.1 ++ [x], next)
    | .failed .. => (acc.1 ++ [x], acc.2)) ([], none)).1

/-- Index and description of the first record whose Lean trace differs from the Rust one. -/
def firstDifference (rs : List Replay) : Option (Nat × String) :=
  (rs.zipIdx.find? fun (x, _) => match x with
    | .ok l r _ => l != r
    | .failed .. => true).map fun (x, i) => match x with
    | .ok l r _ =>
      (i, s!"lean: {(Json.traceToJson l).compress}\nrust: {(Json.traceToJson r).compress}")
    | .failed m r => (i, s!"{m}\nrust: {(Json.traceToJson r).compress}")

/-- Parse the records file. -/
def parseRecords (src : String) : Except String (List Json.OracleRecord) :=
  ((src.splitOn "\n").filter (fun l => !l.isEmpty)).mapM fun l => do
    Json.recordFromJson (← Lean.Json.parse l)

/-- The command line: exit code, standard output and standard error. -/
-- @tg: implements:: [[openspec:formal-model#Differential Oracle]]
def run (args : List String) : IO UInt32 := do
  let rec opt : List String → String → Option String
    | k :: v :: rest, key => if k == key then some v else opt (v :: rest) key
    | _, _ => none
  let usage := "usage: vertep-oracle --chart CHART.nq --records RECORDS.jsonl [--out TRACES.jsonl] [--compare]"
  let (some chartPath, some recPath) := (opt args "--chart", opt args "--records")
    | IO.eprintln usage; return 3
  let chartSrc ← IO.FS.readFile chartPath
  let recSrc ← IO.FS.readFile recPath
  let (_, cc) ← match loadChart chartSrc with
    | .ok x => pure x
    | .error e => IO.eprintln s!"chart: {e}"; return 3
  let records ← match parseRecords recSrc with
    | .ok x => pure x
    | .error e => IO.eprintln s!"records: {e}"; return 3
  let replays := replayAll cc records
  let lines := replays.map fun x => match x with
    | .ok l _ _ => (Json.traceToJson l).compress
    | .failed m _ => (Lean.Json.mkObj [("error", .str m)]).compress
  let out := String.join (lines.map (· ++ "\n"))
  match opt args "--out" with
  | some p => IO.FS.writeFile p out
  | none => IO.print out
  if args.contains "--compare" then
    match firstDifference replays with
    | some (i, d) =>
      IO.eprintln s!"first differing record: {i}\n{d}"
      return 1
    | none => return 0
  else
    return (if replays.any (fun x => match x with
      | .failed .. => true
      | _ => false) then 1 else 0)

end Vertep.Oracle
