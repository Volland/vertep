/-!
# Kolyada protocols (m10 8.1)

A model of the protocol core of `crates/vertep-kolyada`: a protocol document's roles and
choreography, its projection onto one role, the conformance function `accept` of one
participant of an instance, causal delivery around it, and the states `Reachable` from a
genesis.

**What is abstract.** A message is its signed body with two facts the Rust side decides and the
model takes as given, like the answer tape of the chart model: whether its signature verifies
(`sigOk`) and whether its payload matches the declared shape (`shapeOk`, or `payloadEmpty` for
a control message). Everything else the check reads is modelled: the instance, the role
binding, equivocation, dependencies, sequence numbers, recipients, the instance's life and the
choreography state.

**Participant knowledge.** As in `Instance`, a participant knows the protocol state only through
the messages it sees, so its `knowledge` is the set of states consistent with them: a transition
it takes no part in is silent and adds its target (`closure`), and a message hidden from it
counts as one unknown step (`hide`). A message is enabled when some state in the set enables it.

**State as joins.** Every part of the state except the knowledge only grows: the applied hashes,
the dependencies seen, the `(role, seq)` pairs, the joined roles, and the flags for a decline and
a completion. The instance's status, the last sequence number of a role and the heads are
derived from them (`St.status`, `St.lastSeq`, `St.heads`), and they agree with the fields the
Rust instance keeps in every state the checks accept; `vertep-kolyada --history` compares them
on every recorded line. This is what makes concurrent messages commute
(`VertepLean.ProtocolTheorems`, `causal_orders_converge`).
-/

namespace Vertep.Kolyada

abbrev Role := String
abbrev Did := String
abbrev StateId := String
abbrev Hash := String

/-! ## Protocols and projection -/

/-- A transition of the global choreography: in `src`, `sender` sends `message` to `receivers`
and the protocol moves to `dst`. -/
structure Transition where
  src : StateId
  sender : Role
  receivers : List Role
  message : String
  dst : StateId
  deriving Repr, DecidableEq

/-- A protocol document, as the checker reads it: its identity, roles, declared message types,
choreography and delivery limits. The transitions are in the order `Protocol::transitions`
returns them, which decides the transition `accept` checks the recipients against. -/
structure Protocol where
  id : String
  roles : List Role
  messages : List String
  initial : StateId
  finals : List StateId
  transitions : List Transition
  bufferSize : Nat
  timeoutMs : Nat
  deriving Repr

def genesisKind : String := "kolyada:genesis"
def joinKind : String := "kolyada:join"
def declineKind : String := "kolyada:decline"
def violationKind : String := "kolyada:violation"
def completeKind : String := "kolyada:complete"

/-- The reserved control message types; every other type is a protocol message. -/
def isControl (k : String) : Bool :=
  k == genesisKind || k == joinKind || k == declineKind || k == violationKind || k == completeKind

/-- What a step of a projection does for the projected role. -/
inductive Action where
  | send (message : String) (receivers : List Role)
  | receive (message : String) (sender : Role)
  | silent
  deriving Repr, DecidableEq

/-- One step of a projection. -/
structure Step where
  src : StateId
  dst : StateId
  action : Action
  deriving Repr, DecidableEq

/-- A transition as role `r` sees it: a send, a receive, or silence. -/
def stepOf (r : Role) (t : Transition) : Step :=
  { src := t.src, dst := t.dst,
    action := if t.sender = r then .send t.message t.receivers
      else if r ∈ t.receivers then .receive t.message t.sender else .silent }

/-- The projection of the protocol onto `r` (`Protocol::project`); `none` for an unknown role. -/
-- @tg: implements:: [[openspec:agent-protocols#Role Projection]]
def project (p : Protocol) (r : Role) : Option (List Step) :=
  if r ∈ p.roles then some (p.transitions.map (stepOf r)) else none

/-! ## Knowledge -/

/-- Add `x` to a set kept as a list. -/
def addNew (K : List StateId) (x : StateId) : List StateId :=
  if x ∈ K then K else K ++ [x]

/-- The targets of the transitions out of `K` that `r` takes no part in. -/
def silentFrom (p : Protocol) (r : Role) (K : List StateId) : List StateId :=
  (p.transitions.filter (fun t => decide (t.src ∈ K ∧ t.sender ≠ r ∧ r ∉ t.receivers))).map
    (·.dst)

/-- Add the targets of silent steps until nothing new appears (at most one round per
transition). -/
def closureN (p : Protocol) (r : Role) : Nat → List StateId → List StateId
  | 0, K => K
  | n + 1, K =>
    let K' := (silentFrom p r K).foldl addNew K
    if K'.length = K.length then K else closureN p r n K'

/-- The states `r` may be in once it may be in `K`. -/
def closure (p : Protocol) (r : Role) (K : List StateId) : List StateId :=
  closureN p r (p.transitions.length + 1) K

/-- The transitions out of `K` by which `r` sends `kind`. -/
def matching (p : Protocol) (K : List StateId) (r : Role) (kind : String) : List Transition :=
  p.transitions.filter (fun t => decide (t.src ∈ K ∧ t.message = kind ∧ t.sender = r))

/-- Knowledge after the steps whose targets are `next`: unchanged if there are none. -/
def advanceTo (p : Protocol) (me : Role) (K next : List StateId) : List StateId :=
  if next.isEmpty then K else closure p me (next.foldl addNew [])

/-! ## Messages and verdicts -/

/-- A signed message, with the two facts the Rust side decides (`sigOk`, `shapeOk`). -/
structure Msg where
  hash : Hash
  sigOk : Bool
  protocol : String
  inst : Option Hash
  sender : Did
  role : Role
  seq : Nat
  deps : List Hash
  recipients : List Role
  kind : String
  payloadEmpty : Bool
  shapeOk : Bool
  deriving Repr, DecidableEq

/-- An input of a participant: a message, or the hash of a message hidden from it. -/
inductive Input where
  | msg (m : Msg)
  | hidden (h : Hash)
  deriving Repr, DecidableEq

/-- Whether an input moves the choreography (a protocol message, or a hidden message). -/
def Input.advances : Input → Bool
  | .msg m => !isControl m.kind
  | .hidden _ => true

/-- The knowledge of `me` after an input; control messages leave it unchanged. -/
def kstep (p : Protocol) (me : Role) (K : List StateId) : Input → List StateId
  | .msg m =>
    if isControl m.kind then K
    else if m.role = me ∨ me ∈ m.recipients then
      advanceTo p me K ((matching p K m.role m.kind).map (·.dst))
    else advanceTo p me K ((p.transitions.filter (fun t => decide (t.src ∈ K))).map (·.dst))
  | .hidden _ => advanceTo p me K ((p.transitions.filter (fun t => decide (t.src ∈ K))).map (·.dst))

/-- Why a message is refused (`Reason`). -/
inductive Reason where
  | signature | wrongInstance | roleNotPermitted | notOpen | missingDependencies
  | notEnabledInState | unknownMessage | payloadShape | recipients | sequence | equivocation
  | bufferFull | badGenesis | alreadyJoined
  deriving Repr, DecidableEq

/-- The fixed text of a reason (`Reason::as_str`). -/
def Reason.text : Reason → String
  | .signature => "signature"
  | .wrongInstance => "wrong instance"
  | .roleNotPermitted => "role not permitted"
  | .notOpen => "not open"
  | .missingDependencies => "missing dependencies"
  | .notEnabledInState => "not enabled in state"
  | .unknownMessage => "unknown message"
  | .payloadShape => "payload shape"
  | .recipients => "recipients"
  | .sequence => "sequence"
  | .equivocation => "equivocation"
  | .bufferFull => "buffer full"
  | .badGenesis => "bad genesis"
  | .alreadyJoined => "already joined"

/-- The verdict of the conformance check. -/
inductive Verdict where
  | accepted
  | duplicate
  | refused (r : Reason)
  deriving Repr, DecidableEq

/-- Where an instance is in its life. -/
inductive Status where
  | pending | opened | declined | completed
  deriving Repr, DecidableEq

def Status.text : Status → String
  | .pending => "pending"
  | .opened => "open"
  | .declined => "declined"
  | .completed => "completed"

/-! ## A participant's state -/

/-- One participant's view of an instance. `log` is the history in the order it was applied,
each message with the knowledge it was judged in. -/
structure St where
  id : Hash
  bindings : List (Role × Did)
  me : Role
  applied : List Hash
  deps : List Hash
  seqs : List (Role × Nat)
  seen : List (Role × Nat × Hash)
  joined : List Role
  declined : Bool
  completed : Bool
  knowledge : List StateId
  log : List (Msg × List StateId)

/-- The highest sequence number applied for `r` (0 if none). -/
def St.lastSeq (s : St) (r : Role) : Nat :=
  s.seqs.foldr (fun x acc => if x.1 = r then max x.2 acc else acc) 0

/-- Whether another message with role `r` and number `q` was applied. -/
def St.Equivocates (s : St) (r : Role) (q : Nat) (h : Hash) : Prop :=
  ∃ x ∈ s.seen, x.1 = r ∧ x.2.1 = q ∧ x.2.2 ≠ h

instance (s : St) (r : Role) (q : Nat) (h : Hash) : Decidable (s.Equivocates r q h) := by
  unfold St.Equivocates; infer_instance

/-- The other message with role `r` and number `q`, for the equivocation proof. -/
def St.equivocal (s : St) (r : Role) (q : Nat) (h : Hash) : Option Hash :=
  (s.seen.find? (fun x => decide (x.1 = r ∧ x.2.1 = q ∧ x.2.2 ≠ h))).map (·.2.2)

/-- The roles bound to `d`. -/
def rolesOf (bindings : List (Role × Did)) (d : Did) : List Role :=
  (bindings.filter (fun b => decide (b.2 = d))).map (·.1)

/-- Every role of the protocol has joined. -/
def St.AllJoined (p : Protocol) (s : St) : Prop := ∀ r ∈ p.roles, r ∈ s.joined

instance (p : Protocol) (s : St) : Decidable (s.AllJoined p) := by
  unfold St.AllJoined; infer_instance

/-- The status, from the joins, a decline and a completion. -/
def St.status (p : Protocol) (s : St) : Status :=
  if s.declined then .declined else if s.completed then .completed
  else if s.AllJoined p then .opened else .pending

/-- The applied hashes no applied message depends on. -/
def St.heads (s : St) : List Hash := s.applied.filter (fun h => decide (h ∉ s.deps))

/-- Every state the participant may be in is final. -/
def St.Complete (p : Protocol) (s : St) : Prop := s.knowledge ≠ [] ∧ ∀ x ∈ s.knowledge, x ∈ p.finals

instance (p : Protocol) (s : St) : Decidable (s.Complete p) := by
  unfold St.Complete; infer_instance

/-! ## The conformance check -/

/-- The check of a protocol message's type in the participant's knowledge. -/
def judgeProtocol (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if s.status p ≠ .opened then .refused .notOpen
  else if m.kind ∉ p.messages then .refused .unknownMessage
  else match matching p s.knowledge m.role m.kind with
    | [] =>
      if ∃ t ∈ p.transitions, t.src ∈ s.knowledge ∧ t.message = m.kind ∧ t.sender ≠ m.role
      then .refused .roleNotPermitted else .refused .notEnabledInState
    | t :: _ =>
      if ¬ ∀ r ∈ t.receivers, r ∈ m.recipients then .refused .recipients
      else if m.shapeOk then .accepted else .refused .payloadShape

/-- The check of the message type, after the checks common to every message. -/
def judgeKind (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if m.kind = genesisKind then .refused .badGenesis
  else if m.kind = joinKind ∨ m.kind = declineKind then
    if s.status p ≠ .pending then .refused .notOpen
    else if m.role ∈ s.joined then .refused .alreadyJoined
    else if m.payloadEmpty then .accepted else .refused .payloadShape
  else if m.kind = violationKind then .accepted
  else if m.kind = completeKind then
    if s.status p = .opened ∧ s.Complete p then .accepted else .refused .notEnabledInState
  else judgeProtocol p s m

/-- The conformance check without the signature, in the order of `Instance::judge`. -/
def judge (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if m.hash ∈ s.applied then .duplicate
  else if m.protocol ≠ p.id ∨ m.inst ≠ some s.id then .refused .wrongInstance
  else if s.bindings.lookup m.role ≠ some m.sender then .refused .roleNotPermitted
  else if s.Equivocates m.role m.seq m.hash then .refused .equivocation
  else if ¬ ∀ d ∈ m.deps, d ∈ s.applied then .refused .missingDependencies
  else if m.seq ≤ s.lastSeq m.role then .refused .sequence
  else if m.recipients = [] ∨ ¬ ∀ r ∈ m.recipients, r ∈ p.roles ∧ r ≠ m.role then
    .refused .recipients
  else judgeKind p s m

/-- **The conformance check** (`Instance::check`): a pure function of the participant's state and
a message. -/
-- @tg: implements:: [[openspec:protocol-conformance#Conformance Check]]
def accept (p : Protocol) (s : St) (m : Msg) : Verdict :=
  if m.sigOk then judge p s m else .refused .signature

/-- Apply an accepted message. -/
def apply (p : Protocol) (s : St) (m : Msg) : St :=
  { s with
    applied := m.hash :: s.applied
    deps := m.deps ++ s.deps
    seqs := (m.role, m.seq) :: s.seqs
    seen := (m.role, m.seq, m.hash) :: s.seen
    joined := if m.kind = joinKind then rolesOf s.bindings m.sender ++ s.joined else s.joined
    declined := s.declined || decide (m.kind = declineKind)
    completed := s.completed || decide (m.kind = completeKind)
    knowledge := kstep p s.me s.knowledge (.msg m)
    log := s.log ++ [(m, s.knowledge)] }

/-- Record a message hidden from the participant (`Instance::observe_hidden`). -/
def hide (p : Protocol) (s : St) (h : Hash) : St :=
  { s with
    applied := h :: s.applied
    knowledge := kstep p s.me s.knowledge (.hidden h) }

/-- Start an instance from a genesis for the participant `localDid` (`Instance::from_genesis`,
without sub-protocols); `none` for a genesis the Rust side refuses. -/
def init (p : Protocol) (bindings : List (Role × Did)) (localDid : Did) (g : Msg) : Option St :=
  if g.sigOk ∧ g.kind = genesisKind ∧ g.inst = none ∧ g.protocol = p.id ∧
      (∀ r ∈ p.roles, r ∈ bindings.map (·.1)) ∧ (∀ b ∈ bindings, b.1 ∈ p.roles) ∧
      bindings.lookup g.role = some g.sender ∧
      (∀ r ∈ g.recipients, r ∈ p.roles ∧ r ≠ g.role) ∧
      (∀ r ∈ p.roles, r ≠ g.role → r ∈ g.recipients) then
    match bindings.find? (fun b => decide (b.2 = localDid)) with
    | none => none
    | some (me, _) =>
      let K := closure p me [p.initial]
      some { id := g.hash, bindings, me, applied := [g.hash], deps := [],
             seqs := [(g.role, g.seq)], seen := [(g.role, g.seq, g.hash)],
             joined := rolesOf bindings g.sender, declined := false, completed := false,
             knowledge := K, log := [(g, K)] }
  else none

/-- The states of a participant reachable from a genesis: applying accepted messages and taking
in hidden ones. -/
inductive Reachable (p : Protocol) : St → Prop
  | init {b : List (Role × Did)} {d : Did} {g : Msg} {s : St} :
      init p b d g = some s → Reachable p s
  | apply {s : St} {m : Msg} : Reachable p s → accept p s m = .accepted → Reachable p (apply p s m)
  | hide {s : St} {h : Hash} : Reachable p s → h ∉ s.applied → Reachable p (hide p s h)

/-! ## Runs -/

/-- One input, if the participant takes it. -/
def step (p : Protocol) (s : St) : Input → Option St
  | .msg m => if accept p s m = .accepted then some (apply p s m) else none
  | .hidden h => if h ∈ s.applied then none else some (hide p s h)

/-- Take inputs in order; `none` if one is not taken. -/
def run (p : Protocol) : St → List Input → Option St
  | s, [] => some s
  | s, i :: is =>
    match step p s i with
    | some t => run p t is
    | none => none

/-! ## Causal delivery -/

/-- What the receiver did with a message (`Event`). -/
inductive Ev where
  | applied (h : Hash)
  | duplicate (h : Hash)
  | held (h : Hash)
  | refused (h : Hash) (reason : String)
  | stalled (h : Hash) (missing : List Hash)
  | equivocation (h : Hash) (other : Hash)
  deriving Repr, DecidableEq

/-- A receiver: the participant's state and the messages held for their dependencies, with
their arrival time. -/
structure Rx where
  st : St
  held : List (Msg × Nat)

/-- One pass over the held messages, in order: apply what is accepted, drop what is refused or
duplicate, keep what still misses dependencies; and whether anything was applied. -/
def drainPass (p : Protocol) : St → List (Msg × Nat) → St × List (Msg × Nat) × List Ev × Bool
  | s, [] => (s, [], [], false)
  | s, (m, t) :: rest =>
    match accept p s m with
    | .refused .missingDependencies =>
      let r := drainPass p s rest
      (r.1, (m, t) :: r.2.1, r.2.2.1, r.2.2.2)
    | .accepted =>
      let r := drainPass p (apply p s m) rest
      (r.1, r.2.1, .applied m.hash :: r.2.2.1, true)
    | .duplicate =>
      let r := drainPass p s rest
      (r.1, r.2.1, .duplicate m.hash :: r.2.2.1, r.2.2.2)
    | .refused why =>
      let r := drainPass p s rest
      (r.1, r.2.1, .refused m.hash why.text :: r.2.2.1, r.2.2.2)

/-- Passes until one applies nothing. -/
def drainN (p : Protocol) : Nat → St → List (Msg × Nat) → St × List (Msg × Nat) × List Ev
  | 0, s, hs => (s, hs, [])
  | n + 1, s, hs =>
    let r := drainPass p s hs
    if r.2.2.2 then
      let r' := drainN p n r.1 r.2.1
      (r'.1, r'.2.1, r.2.2.1 ++ r'.2.2)
    else (r.1, r.2.1, r.2.2.1)

/-- Apply every held message whose dependencies are now in (`Instance::drain`). -/
def drain (p : Protocol) (s : St) (hs : List (Msg × Nat)) : St × List (Msg × Nat) × List Ev :=
  drainN p (hs.length + 1) s hs

/-- Take in a message from the network at time `now` (`Instance::receive`): apply it and what it
unblocks, hold it for its dependencies in the bounded buffer, or refuse it. -/
-- @tg: implements:: [[openspec:protocol-conformance#Causal Delivery]]
def receive (p : Protocol) (x : Rx) (m : Msg) (now : Nat) : Rx × List Ev :=
  match accept p x.st m with
  | .duplicate => (x, [.duplicate m.hash])
  | .accepted =>
    let r := drain p (apply p x.st m) x.held
    (⟨r.1, r.2.1⟩, .applied m.hash :: r.2.2)
  | .refused .missingDependencies =>
    if x.held.any (fun e => e.1.hash == m.hash) then (x, [.held m.hash])
    else if p.bufferSize ≤ x.held.length then
      match x.held with
      | (e, _) :: rest => (⟨x.st, rest ++ [(m, now)]⟩, [.refused e.hash Reason.bufferFull.text, .held m.hash])
      | [] => (⟨x.st, [(m, now)]⟩, [.held m.hash])
    else (⟨x.st, x.held ++ [(m, now)]⟩, [.held m.hash])
  | .refused why =>
    let proof := if why = .equivocation then
        match x.st.equivocal m.role m.seq m.hash with
        | some o => [Ev.equivocation m.hash o]
        | none => []
      else []
    (x, proof ++ [.refused m.hash why.text])

/-- Take in the hash of a message hidden from the participant, then drain. -/
def observeHidden (p : Protocol) (x : Rx) (h : Hash) : Rx × List Ev :=
  if h ∈ x.st.applied then (x, [])
  else
    let r := drain p (hide p x.st h) x.held
    (⟨r.1, r.2.1⟩, r.2.2)

/-- Drop held messages that waited `timeoutMs` or longer, reporting each as a stall. -/
def expire (p : Protocol) (x : Rx) (now : Nat) : Rx × List Ev :=
  let gone := x.held.filter (fun e => decide (p.timeoutMs ≤ now - e.2))
  let keep := x.held.filter (fun e => !decide (p.timeoutMs ≤ now - e.2))
  (⟨x.st, keep⟩,
    gone.map (fun e => .stalled e.1.hash (e.1.deps.filter (fun d => decide (d ∉ x.st.applied)))))

/-- The participant's own message (`Instance::send`): checked on its own projection, then
applied; nothing is drained. -/
def send (p : Protocol) (x : Rx) (m : Msg) : Option Rx :=
  if accept p x.st m = .accepted then some ⟨apply p x.st m, x.held⟩ else none

end Vertep.Kolyada
