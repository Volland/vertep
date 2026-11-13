import Lean.Data.Json
import VertepLean.CustodyJoint

/-!
# The joint-consensus custody model as a differential oracle (m7 10.3)

`vertep-custody` replays histories with configuration records (an event carries `"config"`)
through the joint-consensus model (`VertepLean.CustodyJoint`), and histories without them
through the fixed-set model (`VertepLean.CustodyCheck`).

A record is `"config":{"version":V,"custodians":[…],"quorum":Q}` with `"next"`: a configuration
of version `V+1`, or `null`. Records appear on `prepare` (the custodian's record), `accept`
(`config`/`next`: the batch's record; `config_after`/`next_after`: the custodian's record
after), `elected` (the record it was elected under) and `committed` (the record whose quorums
backed it). The configurations named by version must agree across the history, and every quorum
must be a majority.

In strict mode, beyond the fixed-set checks:

* a promise reports the model's record of the custodian, and an append leaves the model's
  record (`decideJ`: an older version, or a lower record within the accepted epoch, fails);
* a writer's batches move its record one step at a time: to the joint record at will, and out
  of it (`switchReady`) only once quorums of both sets hold its whole log with the joint record;
* an election is backed by promises of a quorum of its record (both sets when joint), none of
  which reports a higher record, under a record some custodian or writer had;
* a commit is backed, at that moment, by quorums of its record (both sets when joint) of
  custodians that accepted in its epoch a log extending it, under a record its writer held.
-/

namespace Vertep.Custody.JointCheck

open Lean (Json)
open Vertep.Custody Vertep.Custody.Joint

/-- A configuration record from a history. -/
structure Rec where
  ver : Nat
  cfg : Config
  next : Option Config

def Rec.rank (r : Rec) : Nat := 2 * r.ver + (if r.next.isSome then 1 else 0)

def configObj (j : Json) : Except String (Nat × Config) := do
  let v ← (← j.getObjVal? "version").getNat?
  let cs ← Check.strs (← j.getObjVal? "custodians")
  let q ← (← j.getObjVal? "quorum").getNat?
  pure (v, ⟨cs, q⟩)

/-- The record in fields `ck` and `nk`, if present. -/
def recOf (j : Json) (ck nk : String) : Except String (Option Rec) := do
  match j.getObjVal? ck with
  | .error _ => pure none
  | .ok c =>
    let (v, cfg) ← configObj c
    let next ← match j.getObjVal? nk with
      | .error _ => pure none
      | .ok .null => pure none
      | .ok n => do
        let (v', cfg') ← configObj n
        if v' ≠ v + 1 then throw s!"pending configuration version {v'} does not follow {v}"
        pure (some cfg')
    pure (some ⟨v, cfg, next⟩)

/-- One history event, with its records. -/
inductive JEvent where
  | prepare (cust cand : String) (epoch : Nat) (promised : Bool) (highestBefore accEpoch : Nat)
      (log : Log) (rec : Option Rec)
  | accept (cust writer : String) (epoch after : Nat) (entries : Log) (outcome : Outcome)
      (logAfter : Log) (accAfter : Nat) (rec recAfter : Option Rec)
  | elected (host : String) (epoch : Nat) (promisedBy : List String) (adopted : Log)
      (rec : Option Rec)
  | committed (host : String) (epoch : Nat) (log : Log) (rec : Option Rec)
  | restart (cust : String)

def eventFromJson (j : Json) : Except String JEvent := do
  let str (k : String) : Except String String := do (← j.getObjVal? k).getStr?
  let nat (k : String) : Except String Nat := do (← j.getObjVal? k).getNat?
  let list (k : String) : Except String (List String) := do Check.strs (← j.getObjVal? k)
  match ← str "kind" with
  | "prepare" =>
    pure (.prepare (← str "custodian") (← str "candidate") (← nat "epoch")
      (← (← j.getObjVal? "promised").getBool?) (← nat "highest_before") (← nat "accepted_epoch")
      (← list "log") (← recOf j "config" "next"))
  | "accept" =>
    pure (.accept (← str "custodian") (← str "writer") (← nat "epoch") (← nat "after")
      (← list "entries") (← Check.outcomeFromString (← str "outcome")) (← list "log_after")
      (← nat "accepted_epoch_after") (← recOf j "config" "next")
      (← recOf j "config_after" "next_after"))
  | "elected" =>
    pure (.elected (← str "host") (← nat "epoch") (← list "promised_by") (← list "adopted")
      (← recOf j "config" "next"))
  | "committed" =>
    pure (.committed (← str "host") (← nat "epoch") (← list "log") (← recOf j "config" "next"))
  | "restart" => pure (.restart (← str "custodian"))
  | k => throw s!"unknown kind {k}"

def recsOf : JEvent → List Rec
  | .prepare _ _ _ _ _ _ _ r => r.toList
  | .accept _ _ _ _ _ _ _ _ r r' => r.toList ++ r'.toList
  | .elected _ _ _ _ r => r.toList
  | .committed _ _ _ r => r.toList
  | .restart _ => []

/-- The configurations of a history by version, checked to agree and to be majorities. -/
def configTable (g : Nat) (genesis : Config) (es : List JEvent) :
    Except String (List (Nat × Config)) := do
  let entries := (g, genesis) :: (es.flatMap recsOf).flatMap (fun r =>
    (r.ver, r.cfg) :: r.next.toList.map (fun c => (r.ver + 1, c)))
  entries.foldlM (fun (acc : List (Nat × Config)) (v, c) => do
    if !(c.custodians.length < 2 * c.quorum) then
      throw s!"configuration version {v}: quorum {c.quorum} is not a majority of {c.custodians.length} custodians"
    if !c.custodians.Nodup then throw s!"configuration version {v}: custodians repeat"
    match acc.lookup v with
    | some c' =>
      if c'.custodians = c.custodians ∧ c'.quorum = c.quorum then pure acc
      else throw s!"configuration version {v} names two different custodian sets or quorums"
    | none => pure ((v, c) :: acc)) []

/-- The replay state. -/
structure St where
  sys : JSys
  commits : List (Nat × Log)

/-- The static context: configurations, genesis version, every custodian named. -/
structure Ctx where
  conf : Confs
  g : Nat
  members : List String

def showRank (r : Nat) : String :=
  if isJoint r then s!"version {verOf r} joint with {verOf r + 1}" else s!"version {verOf r}"

def replayOne (cx : Ctx) (st : St) (i : Nat) : JEvent → Except String St
  | .prepare c h e promised hb ae log rec => do
    unless cx.members.contains c do throw s!"line {i}: {c} is not a custodian"
    let cc := st.sys.cust c
    if hb ≠ cc.core.promised then
      throw s!"line {i}: prepare({e}) at {c}: recorded highest promised {hb}, the model has {cc.core.promised}"
    if ae ≠ cc.core.acc ∨ log ≠ cc.core.log then
      throw s!"line {i}: prepare({e}) at {c}: recorded state (accepted {ae}, {log}), the model has (accepted {cc.core.acc}, {cc.core.log})"
    if let some r := rec then
      if r.rank ≠ cc.rank then
        throw s!"line {i}: prepare({e}) at {c}: recorded record {showRank r.rank}, the model has {showRank cc.rank}"
    let m := promises? cc.core e
    if m != promised then
      throw s!"line {i}: prepare({e}) by {h} at {c}: recorded {if promised then "promise" else "refusal"}, the model decides {if m then "promise" else "refusal"}"
    if m then
      match exec cx.conf cx.g st.sys (.prepare c e h) with
      | some t => pure { st with sys := t }
      | none => throw s!"line {i}: internal: prepare not enabled"
    else pure st
  | .accept c h e after entries outcome logAfter aeAfter rec recAfter => do
    unless cx.members.contains c do throw s!"line {i}: {c} is not a custodian"
    let cc := st.sys.cust c
    if outcome = .failed then
      if logAfter ≠ cc.core.log ∨ aeAfter ≠ cc.core.acc ∨
          (recAfter.map Rec.rank).any (· ≠ cc.rank) then
        throw s!"line {i}: a failed append at {c} changed its state"
      return st
    let some (r₀, A) := elected? st.sys e h
      | throw s!"line {i}: batch from {h} at epoch {e}, which {h} was not elected in"
    let mut sys := st.sys
    let r := (rec.map Rec.rank).getD (sys.wrank e h)
    -- the writer's record moves one step at a time
    if sys.wrank e h < r then
      if r ≠ sys.wrank e h + 1 then
        throw s!"line {i}: the writer {h} of epoch {e} moved from {showRank (sys.wrank e h)} to {showRank r}"
      if isJoint (sys.wrank e h) then
        match exec cx.conf cx.g sys (.switch h e) with
        | some t => sys := t
        | none => throw s!"line {i}: the writer {h} of epoch {e} left {showRank (sys.wrank e h)} before quorums of both sets held its log with it"
      else
        match exec cx.conf cx.g sys (.propose h e) with
        | some t => sys := t
        | none => throw s!"line {i}: internal: propose not enabled"
    if r < r₀ then
      throw s!"line {i}: batch from {h} at epoch {e} under {showRank r}, below the record it was elected under"
    let wl := sys.wlog e h
    if wl.length < after then
      throw s!"line {i}: batch from {h} at epoch {e} follows position {after}, beyond the writer's log ({wl.length} entries)"
    let w := wl.take after ++ entries
    unless A <+: w do
      throw s!"line {i}: batch from {h} at epoch {e} ends before the log it adopted"
    if !(w <+: wl) then
      if wl <+: w then
        match exec cx.conf cx.g sys (.write h e (w.drop wl.length)) with
        | some t => sys := t
        | none => throw s!"line {i}: internal: write not enabled"
      else throw s!"line {i}: the writer {h} of epoch {e} rewrote its log at position {after}"
    let (o, sys') := appendJ sys c e after r w
    if o ≠ outcome then
      throw s!"line {i}: append({e}, after {after}, {entries.length} entries, {showRank r}) by {h} at {c}: recorded {Check.outcomeName outcome}, the model decides {Check.outcomeName o}"
    let nc := sys'.cust c
    if logAfter ≠ nc.core.log ∨ aeAfter ≠ nc.core.acc then
      throw s!"line {i}: append({e}) at {c}: recorded (accepted {aeAfter}, {logAfter}), the model has (accepted {nc.core.acc}, {nc.core.log})"
    if let some ra := recAfter then
      if ra.rank ≠ nc.rank then
        throw s!"line {i}: append({e}) at {c}: recorded record after {showRank ra.rank}, the model has {showRank nc.rank}"
    if o = .ack then
      if let some (e₀, _) := st.commits.find? (fun (e₀, L) => e₀ < e ∧ !(L <+: nc.core.log)) then
        throw s!"line {i}: safety: {c} accepted in epoch {e} a log without the log committed in epoch {e₀}"
    pure { st with sys := sys' }
  | .elected h e qs A rec => do
    let r := (rec.map Rec.rank).getD (2 * cx.g)
    unless known cx.g st.sys r do
      throw s!"line {i}: {h} elected in epoch {e} under {showRank r}, neither seen nor pending on a record seen"
    unless decide (RQuorum cx.conf r qs) do
      throw s!"line {i}: {h} elected in epoch {e} by {qs}, not a quorum of {showRank r}"
    let ps := promisesOf st.sys e h qs
    unless ps.length = qs.length do
      throw s!"line {i}: {h} elected in epoch {e} without a recorded promise from each of {qs}"
    if let some p := ps.find? (fun p => r < p.rank) then
      throw s!"line {i}: {h} elected in epoch {e} under {showRank r}, but {p.cust} promised with {showRank p.rank}"
    unless Check.adoptsB (ps.map JPromise.base) A do
      throw s!"line {i}: {h} elected in epoch {e} adopted {A}, the max rule picks {(Check.best (ps.map JPromise.base)).map (·.log)}"
    if let some (_, h', _, _) := st.sys.elections.find? (fun (e', _, _, _) => e' = e) then
      if h' = h then throw s!"line {i}: {h} elected twice in epoch {e}"
      else throw s!"line {i}: safety: {h'} and {h} both elected in epoch {e}"
    if let some (e₀, _) := st.commits.find? (fun (e₀, L) => e₀ < e ∧ !(L <+: A)) then
      throw s!"line {i}: safety: {h} adopted in epoch {e} a log without the log committed in epoch {e₀}"
    match exec cx.conf cx.g st.sys (.elect h e r qs A) with
    | some t => pure { st with sys := t }
    | none => throw s!"line {i}: internal: election not enabled"
  | .committed h e L rec => do
    let some (r₀, _) := elected? st.sys e h
      | throw s!"line {i}: {h} announced a commit in epoch {e}, which it was not elected in"
    let r := (rec.map Rec.rank).getD (st.sys.wrank e h)
    unless r₀ ≤ r ∧ r ≤ st.sys.wrank e h do
      throw s!"line {i}: {h} announced a commit in epoch {e} under {showRank r}, a record it did not hold"
    let holders := cx.members.filter (fun c =>
      (st.sys.cust c).core.acc = e ∧ L <+: (st.sys.cust c).core.log)
    unless decide (QuorumIn (cx.conf (verOf r)) holders) ∧
        (isJoint r → decide (QuorumIn (cx.conf (verOf r + 1)) holders)) do
      throw s!"line {i}: {h} announced {L} committed in epoch {e} under {showRank r}, held in that epoch by {holders} only"
    if let some (e', h', _, _) := st.sys.elections.find? (fun (e', _, _, A) => e < e' ∧ !(L <+: A)) then
      throw s!"line {i}: safety: {h'} adopted in epoch {e'} a log without the log committed in epoch {e}"
    if let some (c, e', _, _) := st.sys.accepted.find? (fun (_, e', _, l) => e < e' ∧ !(L <+: l)) then
      throw s!"line {i}: safety: {c} accepted in epoch {e'} a log without the log committed in epoch {e}"
    pure { st with commits := st.commits ++ [(e, L)] }
  | .restart _ => pure st

/-- Parse a history with records: the context and the events with their line numbers. -/
def parse (src : String) : Except String (Ctx × List (JEvent × Nat)) := do
  let lines := ((src.splitOn "\n").zipIdx.map (fun (l, i) => (l, i + 1))).filter
    (fun (l, _) => !(l.all Char.isWhitespace))
  match lines with
  | [] => throw "empty history"
  | (first, _) :: rest =>
    let j ← Json.parse first
    let genesis ← Check.configFromJson j
    let g := (j.getObjValAs? Nat "version").toOption.getD 1
    let es ← rest.mapM (fun (l, i) => do
      match Json.parse l >>= eventFromJson with
      | .ok e => pure (e, i)
      | .error err => throw s!"line {i}: {err}")
    let table ← configTable g genesis (es.map (·.1))
    let conf : Confs := fun v => (table.lookup v).getD genesis
    let members := (table.flatMap (·.2.custodians)).eraseDups
    pure (⟨conf, g, members⟩, es)

def replay (cx : Ctx) (es : List (JEvent × Nat)) : Except String Nat := do
  let _ ← es.foldlM (fun st (e, i) => replayOne cx st i e)
    ({ sys := JSys.empty cx.g, commits := [] } : St)
  pure es.length

/-- Whether a history carries configuration records. -/
def hasRecords (src : String) : Bool :=
  ((src.splitOn "\n").drop 1).any (fun l =>
    match Json.parse l with
    | .ok j => (j.getObjVal? "config").toOption.isSome
    | .error _ => false)

/-- Replay a history through the model it calls for. -/
def replayAny (src : String) : Except (Bool × String) Nat :=
  if hasRecords src then
    match parse src with
    | .error e => .error (true, e)
    | .ok (cx, es) => (replay cx es).mapError (false, ·)
  else
    match Check.parse src with
    | .error e => .error (true, e)
    | .ok (cfg, es) => (Check.replay cfg es).mapError (false, ·)

/-- The command line: `vertep-custody --history HISTORY.jsonl`. -/
-- @tg: implements:: [[openspec:object-custody#Custodian Set Changes]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--history", p] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-custody --history HISTORY.jsonl"; return 3
  let src ← match ← (IO.FS.readFile path).toBaseIO with
    | .ok s => pure s
    | .error e => IO.eprintln s!"history: {e}"; return 3
  match replayAny src with
  | .ok n => IO.println s!"{n} events agree"; return 0
  | .error (true, e) => IO.eprintln s!"history: {e}"; return 3
  | .error (false, e) => IO.eprintln e; return 1

end Vertep.Custody.JointCheck
