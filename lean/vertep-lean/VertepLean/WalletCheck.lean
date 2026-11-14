import Lean.Data.Json
import VertepLean.WalletTheorems

/-!
# The exchange history checker (m9 2.4)

`vertep-wallet` replays exchange histories (`crates/vertep-wallet/docs/exchange-history.md`,
recorded by `vertep_wallet::testkit`) through the exchange model (`VertepLean.Wallet`) in strict
mode: every line after the setup must be a move enabled in the state the lines before it reach,
and the next state is the model's `step`. The checker reports the first line that is not, with
the reason (`why`: the first enabledness condition that fails).

```
vertep-wallet --history HISTORY.jsonl [HISTORY.jsonl …]
```

For each file it prints `ok: <file>: <n> lines` (to standard output) or
`error: <file>:<line>: <kind>: <reason>` (to standard error), where `<kind>` is the line's
`kind` (`setup` for line 1, `parse` for a line that is not JSON). It exits `0` when every
history is accepted, `1` when one is not, and `2` on a usage error.

A `store` or `present` line names a credential by id; the checker resolves it to the credential
term the step is about (the first matching delivered, resp. stored, term), and the model checks
the step on that term.

The checker is sound by construction (`replay_sound`): an accepted history is a run of the
model, so its final state, and every state before it, is `Reachable`, and the theorems of
`VertepLean.WalletTheorems` hold in each of them.
-/

namespace Vertep.Wallet.Check

open Lean (Json)

/-! ## Lines -/

/-- One history line after the setup: a move, or a `store`/`present` naming a credential id. -/
inductive Line where
  | act (a : Action)
  | store (holder : Agent) (cred : CredId)
  | present (holder verifier : Agent) (nonce : Nonce) (cred : CredId) (claims : Claims)
      (msg : MsgId)
  deriving Repr

/-- A line with its 1-based number and `kind`. -/
structure Entry where
  num : Nat
  kind : String
  line : Line
  deriving Repr

/-- A failing line: its number, its `kind`, the reason. -/
structure Failure where
  num : Nat
  kind : String
  reason : String
  deriving Repr, DecidableEq

def strs (j : Json) : Except String (List String) := do
  (← j.getArr?).toList.mapM (·.getStr?)

def field (j : Json) (k : String) : Except String Json :=
  match j.getObjVal? k with
  | .ok v => pure v
  | .error _ => throw s!"missing field {k}"

def str (j : Json) (k : String) : Except String String := do
  match (← field j k).getStr? with
  | .ok s => pure s
  | .error _ => throw s!"field {k} is not a string"

def list (j : Json) (k : String) : Except String (List String) := do
  match strs (← field j k) with
  | .ok s => pure s
  | .error _ => throw s!"field {k} is not a list of strings"

def setupFromJson (j : Json) : Except String Setup := do
  unless (← str j "kind") == "setup" do throw "line 1 must be the setup"
  let agents ← (← (← field j "agents").getArr?).toList.mapM (fun a => do
    pure ((← str a "id"), (← list a "keys")))
  let trust ← (← (← field j "trust").getArr?).toList.mapM (fun t => do
    pure ((← str t "verifier"), (← str t "issuer")))
  let compromised ← list j "compromised"
  let keys := agents.flatMap (·.2)
  unless keys.Nodup do throw "a key belongs to more than one agent"
  pure { agents, trust, compromised }

def credFromJson (j : Json) : Except String Cred := do
  pure ⟨← str j "issuer", ← str j "ikey", ← str j "id", ← str j "key", ← list j "claims"⟩

def termFromJson (j : Json) : Except String Term := do
  match ← str j "t" with
  | "offer" => pure (.offer (← str j "issuer") (← str j "nonce"))
  | "pop" => pure (.pop (← str j "key") (← str j "issuer") (← str j "nonce"))
  | "cred" => pure (.cred (← credFromJson j))
  | "req" => pure (.req (← str j "verifier") (← str j "nonce") (← list j "claims"))
  | "pres" =>
    pure (.pres (← str j "key") (← str j "verifier") (← str j "nonce")
      (← credFromJson (← field j "cred")) (← list j "claims"))
  | "other" => pure .other
  | t => throw s!"unknown term {t}"

def lineFromJson (kind : String) (j : Json) : Except String Line := do
  match kind with
  | "offer" =>
    pure (.act (.offer (← str j "issuer") (← str j "to") (← str j "nonce") (← str j "msg")))
  | "prove-possession" =>
    pure (.act (.prove (← str j "holder") (← str j "key") (← str j "issuer") (← str j "nonce")
      (← str j "msg")))
  | "issue" =>
    pure (.act (.issue (← str j "issuer") (← str j "ikey") (← str j "credential") (← str j "key")
      (← str j "nonce") (← list j "claims") (← str j "msg")))
  | "store" => pure (.store (← str j "holder") (← str j "credential"))
  | "request-presentation" =>
    pure (.act (.request (← str j "verifier") (← str j "to") (← str j "nonce")
      (← list j "claims") (← str j "msg")))
  | "consent" =>
    pure (.act (.consent (← str j "holder") (← str j "verifier") (← str j "nonce")
      (← list j "claims")))
  | "present" =>
    pure (.present (← str j "holder") (← str j "verifier") (← str j "nonce")
      (← str j "credential") (← list j "claims") (← str j "msg"))
  | "verify" =>
    let acc ← match ← str j "result" with
      | "accept" => pure true
      | "refuse" => pure false
      | r => throw s!"unknown result {r}"
    pure (.act (.verify (← str j "verifier") (← str j "msg") acc))
  | "revoke" => pure (.act (.revoke (← str j "issuer") (← str j "credential")))
  | "send" => pure (.act (.send (← str j "from") (← str j "msg")))
  | "deliver" => pure (.act (.deliver (← str j "msg") (← str j "to")))
  | "drop" => pure (.act (.drop (← str j "msg")))
  | "duplicate" => pure (.act (.duplicate (← str j "msg")))
  | "inject" => pure (.act (.inject (← str j "msg") (← termFromJson (← field j "term"))))
  | "setup" => throw "the setup must be line 1 only"
  | k => throw s!"unknown kind {k}"

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
      match lineFromJson kind j with
      | .ok l => pure (Entry.mk i kind l)
      | .error e => throw (Failure.mk i kind e))
    pure (σ, es)

/-! ## Replay -/

/-- The move a line stands for in state `s`: a `store` takes the first credential term with
that id, bound to a key of the holder, delivered to the holder; a `present` takes the first
credential term with that id the holder stores, bound to its key, with the disclosed claims. -/
def resolve (σ : Setup) (s : St) : Line → Except String Action
  | .act a => pure a
  | .store H C =>
    let hit := s.msgs.findSome? (fun x => match x.2 with
      | .cred c => if c.id = C ∧ σ.HasKey H c.key ∧ (x.1, H) ∈ s.delivered then some c else none
      | _ => none)
    match hit with
    | some c => pure (.store H c)
    | none => throw s!"no credential {C} bound to a key of {H} was delivered to {H}"
  | .present H V N C D m =>
    match s.stores.find? (fun x => x.1 = H ∧ x.2.id = C ∧ σ.HasKey H x.2.key ∧ D ⊆ x.2.claims) with
    | some (_, c) => pure (.present H V N c D m)
    | none => throw s!"{H} stores no credential {C} bound to its key with claims {D}"

/-- Why a move is not enabled: the first condition that fails. -/
def why (σ : Setup) (s : St) : Action → String
  | .offer I _ N m =>
    if ¬ σ.IsAgent I then s!"{I} is not an agent"
    else if N ∈ s.used then s!"nonce {N} was used before"
    else if ¬ s.Fresh m then s!"message id {m} is not new"
    else "not enabled"
  | .prove H K I N m =>
    if ¬ σ.HasKey H K then s!"{K} is not a key of {H}"
    else if ¬ s.DeliveredTo (.offer I N) H then s!"no offer({I}, {N}) was delivered to {H}"
    else if ¬ s.Fresh m then s!"message id {m} is not new"
    else "not enabled"
  | .issue I IK C K N _ m =>
    if ¬ σ.HasKey I IK then s!"{IK} is not a key of {I}"
    else if ¬ s.DeliveredTo (.pop K I N) I then
      s!"no possession proof pop({K}, {I}, {N}) was delivered to {I}"
    else if (I, N) ∉ s.offers then s!"{I} did not offer nonce {N}"
    else if ¬ (∀ x ∈ s.issued, x.nonce ≠ N) then s!"a credential was already issued on nonce {N}"
    else if ¬ (∀ x ∈ s.issued, x.cred ≠ C) then s!"credential {C} was issued before"
    else if ¬ s.Fresh m then s!"message id {m} is not new"
    else "not enabled"
  | .store H c =>
    if ¬ σ.HasKey H c.key then s!"{c.key} is not a key of {H}"
    else if ¬ s.DeliveredTo (.cred c) H then s!"credential {c.id} was not delivered to {H}"
    else "not enabled"
  | .request _ _ N _ m =>
    if N ∈ s.used then s!"nonce {N} was used before"
    else if ¬ s.Fresh m then s!"message id {m} is not new"
    else "not enabled"
  | .consent H V N D =>
    s!"no request req({V}, {N}, R) with {D} ⊆ R was delivered to {H}"
  | .present H V N c D m =>
    if ¬ (∃ x ∈ s.consents, x.1 = H ∧ x.2.1 = V ∧ x.2.2.1 = N ∧ D ⊆ x.2.2.2) then
      s!"{H} did not consent to disclose {D} to {V} on nonce {N}"
    else if (H, c) ∉ s.stores then s!"{H} does not store credential {c.id}"
    else if ¬ σ.HasKey H c.key then s!"{c.key} is not a key of {H}"
    else if ¬ D ⊆ c.claims then s!"credential {c.id} does not have the claims {D}"
    else if ¬ s.Fresh m then s!"message id {m} is not new"
    else "not enabled"
  | .verify V m acc =>
    if ¬ s.Exists m then s!"message {m} does not exist"
    else if (m, V) ∉ s.delivered then s!"message {m} was not delivered to {V}"
    else if !acc then "not enabled"
    else match s.termOf m with
      | some (.pres K V' N c D) =>
        if V' ≠ V then s!"audience: the presentation is for {V'}, not {V}"
        else if ¬ (∃ x ∈ s.requests, x.1 = V ∧ x.2.1 = N) then s!"{V} did not request nonce {N}"
        else if ¬ (∀ a ∈ s.accepted, ¬ (a.verifier = V ∧ a.nonce = N)) then
          s!"replay: {V} already accepted a presentation for nonce {N}"
        else if ¬ (∃ x ∈ s.requests, x.1 = V ∧ x.2.1 = N ∧ D ⊆ x.2.2) then
          s!"disclosed claims {D} were not requested by {V} on nonce {N}"
        else if K ≠ c.key then
          s!"holder binding: signed with {K}, the credential is bound to {c.key}"
        else if (V, c.issuer) ∉ σ.trust then s!"{V} does not trust {c.issuer}"
        else if ¬ σ.HasKey c.issuer c.ikey then s!"{c.ikey} is not a key of {c.issuer}"
        else if ¬ (∀ r ∈ s.revocations, r.2 ≠ c.id) then s!"credential {c.id} is revoked"
        else if ¬ D ⊆ c.claims then s!"credential {c.id} does not have the claims {D}"
        else "not enabled"
      | _ => s!"message {m} is not a presentation"
  | .revoke I C => s!"{I} did not issue {C}"
  | .send _ m => s!"message id {m} is not new"
  | .deliver m _ | .drop m | .duplicate m => s!"message {m} does not exist"
  | .inject m t =>
    if ¬ s.Fresh m then s!"message id {m} is not new"
    else match t with
      | .pop K _ _ => s!"pop signed by {K}, which is not compromised, and not a known term"
      | .cred c => s!"credential signed by {c.ikey}, which is not compromised, and not a known term"
      | .pres K _ _ c _ =>
        if ¬ (K ∈ σ.compromised ∨ s.Knows t) then
          s!"presentation signed by {K}, which is not compromised, and not a known term"
        else s!"embedded credential signed by {c.ikey}, which is not compromised, and not a known term"
      | _ => "not enabled"

/-- Replay one line: the model's next state, or why the line is not enabled. -/
def replayLine (σ : Setup) (s : St) (l : Line) : Except String St := do
  let a ← resolve σ s l
  match step σ s a with
  | some t => pure t
  | none => throw (why σ s a)

/-- Replay lines from `s`; the state reached, or the first failing line. -/
def replayFrom (σ : Setup) : St → List Entry → Except Failure St
  | s, [] => .ok s
  | s, e :: es =>
    match replayLine σ s e.line with
    | .ok t => replayFrom σ t es
    | .error r => .error ⟨e.num, e.kind, r⟩

/-- Parse and replay a history from the initial state; the number of lines, setup included. -/
def check (src : String) : Except Failure Nat := do
  let (σ, es) ← parse src
  let _ ← replayFrom σ St.empty es
  pure (es.length + 1)

/-! ## Soundness -/

theorem replayLine_sound {σ : Setup} {s t : St} {l : Line} (h : replayLine σ s l = .ok t) :
    ∃ a, step σ s a = some t := by
  unfold replayLine at h
  cases hr : resolve σ s l with
  | error e => rw [hr] at h; cases h
  | ok a =>
    rw [hr] at h
    cases hs : step σ s a with
    | none => simp only [hs, bind, Except.bind] at h; cases h
    | some t' =>
      simp only [hs, bind, Except.bind, pure, Except.pure, Except.ok.injEq] at h
      exact ⟨a, h ▸ hs⟩

/-- **The checker is sound.** An accepted replay is a run of the model: the state it reaches is
reachable, so every theorem of the model holds in it. -/
theorem replay_sound {σ : Setup} :
    ∀ (es : List Entry) {s t : St}, Reachable σ s → replayFrom σ s es = .ok t → Reachable σ t
  | [], s, t, hs, h => by simp only [replayFrom, Except.ok.injEq] at h; exact h ▸ hs
  | e :: es, s, t, hs, h => by
    simp only [replayFrom] at h
    cases hl : replayLine σ s e.line with
    | error r => rw [hl] at h; cases h
    | ok s' =>
      rw [hl] at h
      obtain ⟨a, ha⟩ := replayLine_sound hl
      exact replay_sound es (Reachable.step a hs ha) h

/-- An accepted history satisfies `accept_sound` in its final state, e.g. every acceptance in it
is of a presentation made for that verifier. -/
theorem replay_accept_aud {σ : Setup} {es : List Entry} {t : St}
    (h : replayFrom σ St.empty es = .ok t) {a : Acceptance} (ha : a ∈ t.accepted) :
    a.aud = a.verifier :=
  (accept_sound (replay_sound es Reachable.empty h) ha).2.1

/-! ## The command line -/

/-- Check one file; the line to print and whether it was accepted. -/
def checkFile (path : String) : IO (Bool × String) := do
  match ← (IO.FS.readFile path).toBaseIO with
  | .error e => pure (false, s!"error: {path}:0: io: {e}")
  | .ok src =>
    match check src with
    | .ok n => pure (true, s!"ok: {path}: {n} lines")
    | .error f => pure (false, s!"error: {path}:{f.num}: {f.kind}: {f.reason}")

/-- The command line: `vertep-wallet --history FILE [FILE …]`. -/
-- @tg: implements:: [[openspec:credential-exchange#Exchange History Checker]]
def run (args : List String) : IO UInt32 := do
  let files := match args with
    | "--history" :: fs => fs
    | _ => []
  if files.isEmpty then
    IO.eprintln "usage: vertep-wallet --history HISTORY.jsonl [HISTORY.jsonl …]"
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

end Vertep.Wallet.Check
