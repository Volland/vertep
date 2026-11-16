import VertepLean.ProtocolTheorems

/-!
# A check without the role binding, and why it is unsafe (m10 8.3)

A variant of the conformance check (`VertepLean.Protocol`) that skips the role binding: it does
not compare the sender's DID with the DID the genesis bound to the role the message names. Every
other check stays, the signature included, so the message must still be validly signed, but by
any key.

Then anyone can speak for a role. In the counterexample a buyer opens an instance with a seller,
the seller joins, and `did:example:thief`, bound to nothing, signs a `quote.request` that names
the role `buyer`. The weakened seller applies it (`role_check_needed`): a reachable state of the
weakened model has a history entry from a DID not bound to its role. The model refuses the same
message with `role not permitted` (`demo_refused`), and no reachable state of the model has such
an entry (`no_unbound_applied`, from `log_bound`).
-/

namespace Vertep.Kolyada.Unsafe

open Vertep.Kolyada

/-- `judge` without the role binding. -/
def judgeU (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if m.hash ∈ s.applied then .duplicate
  else if m.protocol ≠ p.id ∨ m.inst ≠ some s.id then .refused .wrongInstance
  else if s.Equivocates m.role m.seq m.hash then .refused .equivocation
  else if ¬ ∀ d ∈ m.deps, d ∈ s.applied then .refused .missingDependencies
  else if m.seq ≤ s.lastSeq m.role then .refused .sequence
  else if m.recipients = [] ∨ ¬ ∀ r ∈ m.recipients, r ∈ p.roles ∧ r ≠ m.role then
    .refused .recipients
  else judgeKind p s m

/-- The weakened check. -/
def acceptU (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if m.sigOk then judgeU p s m else .refused .signature

/-- The states reachable with the weakened check. -/
inductive ReachableU (p : Protocol) : St → Prop
  | init {b : List (Role × Did)} {d : Did} {g : Msg} {s : St} :
      init p b d g = some s → ReachableU p s
  | apply {s : St} {m : Msg} :
      ReachableU p s → acceptU p s m = .accepted → ReachableU p (apply p s m)
  | hide {s : St} {h : Hash} : ReachableU p s → h ∉ s.applied → ReachableU p (hide p s h)

/-- The history has an entry from a DID that is not bound to the role it names. -/
def UnboundApplied (s : St) : Prop :=
  ∃ e ∈ s.log, s.bindings.lookup e.1.role ≠ some e.1.sender

instance (s : St) : Decidable (UnboundApplied s) := by
  unfold UnboundApplied; infer_instance

/-! ## The counterexample -/

/-- A buyer asks a seller for a quote. -/
def demo : Protocol :=
  { id := "urn:vertep:protocol:demo", roles := ["buyer", "seller"],
    messages := ["quote.request", "quote"], initial := "Idle", finals := ["Done"],
    transitions := [⟨"Idle", "buyer", ["seller"], "quote.request", "Quoting"⟩,
      ⟨"Quoting", "seller", ["buyer"], "quote", "Done"⟩],
    bufferSize := 4, timeoutMs := 1000 }

def demoBindings : List (Role × Did) :=
  [("buyer", "did:example:buyer"), ("seller", "did:example:seller")]

/-- The buyer's genesis. -/
def demoGenesis : Msg :=
  { hash := "g", sigOk := true, protocol := demo.id, inst := none, sender := "did:example:buyer",
    role := "buyer", seq := 1, deps := [], recipients := ["seller"], kind := genesisKind,
    payloadEmpty := false, shapeOk := true }

/-- The seller's join. -/
def demoJoin : Msg :=
  { hash := "j", sigOk := true, protocol := demo.id, inst := some "g",
    sender := "did:example:seller", role := "seller", seq := 1, deps := ["g"],
    recipients := ["buyer"], kind := joinKind, payloadEmpty := true, shapeOk := true }

/-- A request for the buyer's role, validly signed by a DID bound to nothing. -/
def thief : Msg :=
  { hash := "x", sigOk := true, protocol := demo.id, inst := some "g",
    sender := "did:example:thief", role := "buyer", seq := 2, deps := ["j"],
    recipients := ["seller"], kind := "quote.request", payloadEmpty := false, shapeOk := true }

theorem demoInit_some : (init demo demoBindings "did:example:seller" demoGenesis).isSome = true := by
  decide

/-- The seller's state after the genesis. -/
def demoStart : St := (init demo demoBindings "did:example:seller" demoGenesis).get demoInit_some

/-- The seller's state once it has joined: the instance is open. -/
def demoOpen : St := apply demo demoStart demoJoin

theorem demoJoin_accepted : accept demo demoStart demoJoin = .accepted := by decide

theorem demoOpen_open : demoOpen.status demo = .opened := by decide

theorem thief_acceptedU : acceptU demo demoOpen thief = .accepted := by decide

/-- The state the weakened seller reaches. -/
def demoState : St := apply demo demoOpen thief

theorem demoState_reachableU : ReachableU demo demoState :=
  .apply (.apply (.init (Option.some_get demoInit_some).symm)
    (by rw [show acceptU demo demoStart demoJoin = accept demo demoStart demoJoin by decide];
        exact demoJoin_accepted))
    thief_acceptedU

/-- **Without the role binding, a DID bound to nothing speaks for a role.** -/
theorem role_check_needed : ∃ s, ReachableU demo s ∧ UnboundApplied s :=
  ⟨demoState, demoState_reachableU, by decide⟩

/-- The model refuses the same message: the role is bound to another DID. -/
theorem demo_refused : accept demo demoOpen thief = .refused .roleNotPermitted := by decide

/-- In the model, no reachable state has an entry from a DID not bound to its role. -/
theorem no_unbound_applied {p : Protocol} {s : St} (hr : Reachable p s) : ¬ UnboundApplied s := by
  rintro ⟨e, he, hne⟩
  exact hne (log_bound hr e he)

end Vertep.Kolyada.Unsafe
