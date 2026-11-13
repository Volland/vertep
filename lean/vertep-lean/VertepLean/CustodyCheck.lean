import Lean.Data.Json
import VertepLean.CustodyTheorems

/-!
# The custody model as a differential oracle (m7 10.3)

`vertep-custody` replays a custody history (`vertep_objects::custody::History`, JSON lines)
through the model (`VertepLean.Custody`) in strict mode:

* every promise or refusal is the model's (`promises?`), with the custodian state it reports;
* every append outcome is the model's (`decideAppend`), with the resulting log and accepted
  epoch; a recorded `failed` (signature, verification or configuration failure in Rust) is
  accepted as long as it changes nothing;
* every election is backed by recorded promises of a quorum for that epoch and host, and adopts
  the log the max rule picks (`Adopts`);
* every batch comes from the elected writer of its epoch, runs from a position inside the
  writer's known log, does not end before the adopted log, and never forks the writer's log;
* every `committed` is backed, at that moment, by a quorum of custodians whose accepted epoch is
  the commit's and whose log extends the committed one;
* along the history, one writer per epoch (`one_writer_per_epoch`) and committed logs are
  prefixes of every later adopted and accepted log (`committed_survives_election`,
  `committed_survives_accept`).

```
vertep-custody --history HISTORY.jsonl
```

Line 1 is `{"kind":"config","custodians":[…],"quorum":Q}`; the other lines are `prepare`,
`accept`, `elected`, `committed` and `restart` events. An `elected` event must precede every
`accept` from that writer in that epoch (including its own replica's).

Exit codes: `0` every event agrees, `1` the first disagreement on standard error, `3` an input
error.
-/

namespace Vertep.Custody.Check

open Lean (Json)

/-- One history event. -/
inductive Event where
  | prepare (cust cand : String) (epoch : Nat) (promised : Bool) (highestBefore accEpoch : Nat)
      (log : Log)
  | accept (cust writer : String) (epoch after : Nat) (entries : Log) (outcome : Outcome)
      (logAfter : Log) (accAfter : Nat)
  | elected (host : String) (epoch : Nat) (promisedBy : List String) (adopted : Log)
  | committed (host : String) (epoch : Nat) (log : Log)
  | restart (cust : String)
  deriving Repr

def strs (j : Json) : Except String (List String) := do
  (← j.getArr?).toList.mapM (·.getStr?)

def configFromJson (j : Json) : Except String Config := do
  match ← (← j.getObjVal? "kind").getStr? with
  | "config" => pure ()
  | k => throw s!"line 1 must be the config, found {k}"
  let cs ← strs (← j.getObjVal? "custodians")
  let q ← (← j.getObjVal? "quorum").getNat?
  if !cs.Nodup then throw "config: custodians repeat"
  if !(cs.length < 2 * q) then throw s!"config: quorum {q} is not a majority of {cs.length} custodians"
  pure ⟨cs, q⟩

def outcomeFromString : String → Except String Outcome
  | "ack" => pure .ack
  | "refused" => pure .refused
  | "behind" => pure .behind
  | "diverged" => pure .diverged
  | "failed" => pure .failed
  | o => throw s!"unknown outcome {o}"

def outcomeName : Outcome → String
  | .ack => "ack"
  | .refused => "refused"
  | .behind => "behind"
  | .diverged => "diverged"
  | .failed => "failed"

def eventFromJson (j : Json) : Except String Event := do
  let str (k : String) : Except String String := do (← j.getObjVal? k).getStr?
  let nat (k : String) : Except String Nat := do (← j.getObjVal? k).getNat?
  let list (k : String) : Except String (List String) := do strs (← j.getObjVal? k)
  match ← str "kind" with
  | "prepare" =>
    pure (.prepare (← str "custodian") (← str "candidate") (← nat "epoch")
      (← (← j.getObjVal? "promised").getBool?) (← nat "highest_before") (← nat "accepted_epoch")
      (← list "log"))
  | "accept" =>
    pure (.accept (← str "custodian") (← str "writer") (← nat "epoch") (← nat "after")
      (← list "entries") (← outcomeFromString (← str "outcome")) (← list "log_after")
      (← nat "accepted_epoch_after"))
  | "elected" =>
    pure (.elected (← str "host") (← nat "epoch") (← list "promised_by") (← list "adopted"))
  | "committed" => pure (.committed (← str "host") (← nat "epoch") (← list "log"))
  | "restart" => pure (.restart (← str "custodian"))
  | k => throw s!"unknown kind {k}"

/-- The max rule as a check: `A` is the log of a promise with the largest
`(accepted epoch, length)` among `ps`. -/
def adoptsB (ps : List Promise) (A : Log) : Bool :=
  ps.any (fun p => A == p.log &&
    ps.all (fun p' => p'.acc < p.acc || (p'.acc == p.acc && p'.log.length ≤ p.log.length)))

theorem adoptsB_sound {ps : List Promise} {A : Log} (h : adoptsB ps A = true) : Adopts ps A := by
  simp only [adoptsB, List.any_eq_true, List.all_eq_true, Bool.and_eq_true, beq_iff_eq,
    Bool.or_eq_true, decide_eq_true_eq] at h
  obtain ⟨p, hp, hA, hall⟩ := h
  exact ⟨p, hp, hA, hall⟩

/-- The log the max rule picks (the first of the largest), for messages. -/
def best (ps : List Promise) : Option Promise :=
  ps.foldl (fun b p => match b with
    | none => some p
    | some b => if b.acc < p.acc ∨ (b.acc = p.acc ∧ b.log.length < p.log.length) then some p
      else some b) none

/-- The replay state: the model state and the recorded commits. -/
structure St where
  sys : Sys
  commits : List (Nat × Log)

def showLog (l : Log) : String := s!"{l}"

/-- Replay one event (at line `i`): the next state, or the disagreement. -/
def replayOne (cfg : Config) (st : St) (i : Nat) : Event → Except String St
  | .prepare c h e promised hb ae log => do
    unless cfg.custodians.contains c do throw s!"line {i}: {c} is not a custodian"
    let cc := st.sys.cust c
    if hb ≠ cc.promised then
      throw s!"line {i}: prepare({e}) at {c}: recorded highest promised {hb}, the model has {cc.promised}"
    if ae ≠ cc.acc ∨ log ≠ cc.log then
      throw s!"line {i}: prepare({e}) at {c}: recorded state (accepted {ae}, {showLog log}), the model has (accepted {cc.acc}, {showLog cc.log})"
    let m := promises? cc e
    if m != promised then
      throw s!"line {i}: prepare({e}) by {h} at {c}: recorded {if promised then "promise" else "refusal"}, the model decides {if m then "promise" else "refusal"}"
    if m then pure { st with sys := prepare st.sys c e h } else pure st
  | .accept c h e after entries outcome logAfter aeAfter => do
    unless cfg.custodians.contains c do throw s!"line {i}: {c} is not a custodian"
    if outcome = .failed then
      -- a signature, verification or configuration failure: nothing changes, and the batch
      -- (possibly not the writer's) teaches nothing about the writer's log
      let cc := st.sys.cust c
      if logAfter ≠ cc.log ∨ aeAfter ≠ cc.acc then
        throw s!"line {i}: a failed append at {c} changed its state"
      return st
    let some (_, _, A) := st.sys.elections.find? (fun (e', h', _) => e' = e ∧ h' = h)
      | throw s!"line {i}: batch from {h} at epoch {e}, which {h} was not elected in"
    let wl := st.sys.wlog e h
    if wl.length < after then
      throw s!"line {i}: batch from {h} at epoch {e} follows position {after}, beyond the writer's log ({wl.length} entries)"
    let w := wl.take after ++ entries
    unless A <+: w do
      throw s!"line {i}: batch from {h} at epoch {e} ends before the log it adopted"
    let sys ← if w <+: wl then pure st.sys
      else if wl <+: w then pure { st.sys with wlog := upd2 st.sys.wlog e h w }
      else throw s!"line {i}: the writer {h} of epoch {e} rewrote its log at position {after}"
    let (o, sys') := append sys c e after w
    if o ≠ outcome then
      throw s!"line {i}: append({e}, after {after}, {entries.length} entries) by {h} at {c}: recorded {outcomeName outcome}, the model decides {outcomeName o}"
    let nc := sys'.cust c
    if logAfter ≠ nc.log ∨ aeAfter ≠ nc.acc then
      throw s!"line {i}: append({e}) at {c}: recorded (accepted {aeAfter}, {showLog logAfter}), the model has (accepted {nc.acc}, {showLog nc.log})"
    if o = .ack then
      if let some (e₀, _) := st.commits.find? (fun (e₀, L) => e₀ < e ∧ !(L <+: nc.log)) then
        throw s!"line {i}: safety: {c} accepted in epoch {e} a log without the log committed in epoch {e₀}"
    pure { st with sys := sys' }
  | .elected h e qs A => do
    unless decide (Quorum cfg qs) do
      throw s!"line {i}: {h} elected in epoch {e} by {qs}, not a quorum of distinct custodians"
    let ps ← qs.mapM (fun c =>
      match st.sys.promises.find? (fun p => p.cust = c ∧ p.epoch = e ∧ p.host = h) with
      | some p => pure p
      | none => throw s!"line {i}: {h} elected in epoch {e} without a recorded promise from {c}")
    unless adoptsB ps A do
      let b := (best ps).map (·.log)
      throw s!"line {i}: {h} elected in epoch {e} adopted {showLog A}, the max rule picks {b.map showLog}"
    if let some (_, h', _) := st.sys.elections.find? (fun (e', _, _) => e' = e) then
      if h' = h then throw s!"line {i}: {h} elected twice in epoch {e}"
      else throw s!"line {i}: safety: {h'} and {h} both elected in epoch {e}"
    if let some (e₀, _) := st.commits.find? (fun (e₀, L) => e₀ < e ∧ !(L <+: A)) then
      throw s!"line {i}: safety: {h} adopted in epoch {e} a log without the log committed in epoch {e₀}"
    pure { st with sys := elect st.sys h e A }
  | .committed h e L => do
    unless st.sys.elections.any (fun (e', h', _) => e' = e ∧ h' = h) do
      throw s!"line {i}: {h} announced a commit in epoch {e}, which it was not elected in"
    let holders := cfg.custodians.filter (fun c =>
      (st.sys.cust c).acc = e ∧ L <+: (st.sys.cust c).log)
    if holders.length < cfg.quorum then
      throw s!"line {i}: {h} announced {showLog L} committed in epoch {e}, held in that epoch by {holders} only"
    if let some (e', h', _) := st.sys.elections.find? (fun (e', _, A) => e < e' ∧ !(L <+: A)) then
      throw s!"line {i}: safety: {h'} adopted in epoch {e'} a log without the log committed in epoch {e}"
    if let some (c, e', _) := st.sys.accepted.find? (fun (_, e', l) => e < e' ∧ !(L <+: l)) then
      throw s!"line {i}: safety: {c} accepted in epoch {e'} a log without the log committed in epoch {e}"
    pure { st with commits := st.commits ++ [(e, L)] }
  | .restart c => do
    unless cfg.custodians.contains c do throw s!"line {i}: {c} is not a custodian"
    pure st

/-- Replay a history (events with their line numbers); `ok` with the number of events. -/
def replay (cfg : Config) (es : List (Event × Nat)) : Except String Nat := do
  let _ ← es.foldlM (fun st (e, i) => replayOne cfg st i e)
    ({ sys := Sys.empty, commits := [] } : St)
  pure es.length

/-- Parse a history: the configuration and the events with their line numbers. -/
def parse (src : String) : Except String (Config × List (Event × Nat)) := do
  let lines := ((src.splitOn "\n").zipIdx.map (fun (l, i) => (l, i + 1))).filter
    (fun (l, _) => !(l.all Char.isWhitespace))
  match lines with
  | [] => throw "empty history"
  | (first, _) :: rest =>
    let cfg ← configFromJson (← Json.parse first)
    let es ← rest.mapM (fun (l, i) => do
      match Json.parse l >>= eventFromJson with
      | .ok e => pure (e, i)
      | .error err => throw s!"line {i}: {err}")
    pure (cfg, es)

/-- The command line. -/
-- @tg: implements:: [[openspec:object-custody#Quorum-Promised Custody Epochs]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--history", p] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-custody --history HISTORY.jsonl"; return 3
  let src ← match ← (IO.FS.readFile path).toBaseIO with
    | .ok s => pure s
    | .error e => IO.eprintln s!"history: {e}"; return 3
  match parse src with
  | .error e => IO.eprintln s!"history: {e}"; return 3
  | .ok (cfg, es) =>
    match replay cfg es with
    | .ok n => IO.println s!"{n} events agree"; return 0
    | .error e => IO.eprintln e; return 1

end Vertep.Custody.Check
