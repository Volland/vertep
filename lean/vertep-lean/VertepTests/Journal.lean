import VertepLean.JournalCheck

/-!
# The journal protocol model

Checks of the toggle machine, the turn function and the oracle's decision procedure on small
observations, including the failures the Rust property tests must be able to detect.
-/

namespace Vertep.Journal.Test

open Vertep.Journal

/-- An entry of the toggle machine with one send per `go`. -/
def e (id input state : String) : Entry String String Unit :=
  { msg := ⟨id, input⟩, effects := if input = "go" then [()] else [], digest := state }

def j3 : Journal String String Unit := [e "m1" "go" "busy", e "m2" "noop" "busy", e "m3" "go" "idle"]

/-- Replay reproduces the recorded states; a wrong digest is a divergence at its position. -/
-- @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay divergence detected]] {level: unit}
def replayChecks : Bool :=
  (match check toggle j3 with | .ok s => s == "idle" | .error _ => false) &&
  (match check toggle [e "m1" "go" "busy", e "m2" "go" "busy"] with
   | .error 2 => true
   | _ => false)
#guard replayChecks

/-- A redelivered message is dropped: the second turn changes nothing. -/
-- @tg: verifies:: [[openspec:event-journal#Message Deduplication#Redelivered message dropped]] {level: unit}
def dedupCheck : Bool :=
  let st : Store String String Unit := ⟨[], 0, []⟩
  let a := activate toggle st.journal
  let m : Msg String := ⟨"m1", "go"⟩
  let (st1, a1, ack1) := turn toggle st a m
  match a1 with
  | some a1 =>
    let (st2, a2, ack2) := turn toggle st1 a1 m
    ack1 && ack2 && st2.journal.length == 1 && st1.journal.length == 1 && a2 == some a1
  | none => false
#guard dedupCheck

/-- Two activations at the same head: the second append conflicts and steps down. -/
-- @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Conflicting append rejected]] {level: unit}
def casCheck : Bool :=
  let st : Store String String Unit := ⟨[], 0, []⟩
  let a := activate toggle st.journal
  let (st1, _, _) := turn toggle st a ⟨"one", "go"⟩
  let (st2, a2, ack2) := turn toggle st1 a ⟨"two", "go"⟩
  st2.journal.length == 1 && a2.isNone && !ack2
#guard casCheck

/-- The oracle accepts a complete observation and names what is wrong with broken ones. -/
-- @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Stable send ids enable receiver dedup]] {level: unit}
def oracleChecks : Bool :=
  let ok : Obs String String Unit :=
    { journal := j3, submitted := ["m1", "m2", "m3", "m4"], delivered := [(1, 0), (3, 0), (1, 0)],
      mailbox := ["m4"] }
  accepts toggle ok && (explain ok).isNone &&
  -- a send of position 2 (which sends nothing) was delivered
  !accepts toggle { ok with delivered := (2, 0) :: ok.delivered } &&
  -- a recorded send was never delivered
  !accepts toggle { ok with delivered := [(1, 0)] } &&
  -- a submitted message is neither journaled nor waiting
  !accepts toggle { ok with mailbox := [] } &&
  -- a message id journaled twice
  !accepts toggle { ok with journal := j3 ++ [e "m1" "go" "busy"],
                            delivered := [(1, 0), (3, 0), (4, 0)] }
#guard oracleChecks

end Vertep.Journal.Test
