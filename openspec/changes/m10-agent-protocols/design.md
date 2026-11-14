## Context

See proposal.md, Why. What exists today:

- **Grants.** `vertep-federation` checks DID-scoped grants (grantee, target, event descriptors, expiry) after decrypting and verifying the sender, and before any journal append ([[decisions#Grants enforced before the journal]]).
- **Envelope.** One JSON-LD envelope carries an SCXML event over DIDComm and TSP ([[decisions#Protocol-neutral event envelope]]).
- **Relays.** Mailboxes and mediators hold ciphertext, acknowledge after the append and deliver at least once ([[constraints#Relays deliver at least once]]).
- **Content addressing.** A chart's version IRI is the hash of its RDFC-1.0 canonical N-Quads.
- **Keys.** Signing goes through the key manager ([[decisions#Keys stay behind a key manager]]).
- **Lean bridge.** Protocol models with oracle checkers over recorded JSONL histories ([[decisions#Lean verification bridge]]).

## Goals / Non-Goals

**Goals:**

- A protocol is a document a stranger can fetch by hash, verify and run, with no registry and no trusted host.
- Each agent checks only its own role's projection, so verification is local and small.
- Transport-agnostic: the same check runs over TSP, DIDComm and local channels.
- Cooperation without a shared store: relays only ever hold what is sealed to their owner.

**Non-Goals:**

- Cross-role parallel regions, choices decided by several roles and dynamic role sets (format version 2).
- Group encryption, one shared ciphertext per space.
- Replacing DID grants for traffic outside protocols.
- Proofs about the cryptography; the model treats signatures and hashes as perfect.

## Decisions

### D1. A protocol is a role-based choreography; the interface is a projection

One document holds roles, messages and the order of the conversation. "How to talk to me" is the document projected on the roles an agent plays, an interface view derived from it, never a second format.

**Alternative rejected:** per-agent interface lists as in DWN protocol definitions. They cannot express order or multi-party cooperation.

- motivated_by: [[decisions#Grants enforced before the journal]]

### D2. One global chart, projected, in a restricted subset

Projection is mechanical when one role decides at each state, with sequence, choice, loops and explicit timeout and cancel messages. Cross-role parallelism and multi-decider choices are rejected at parse time with the state and the reason.

**Alternative rejected:** per-role charts with a composition check; it needs model checking by every peer.

- motivated_by: [[constraints#Choreography projection needs one deciding role]]

### D3. Identity is the hash of the graph; offers are separate signed statements

`urn:vertep:protocol:sha256:` and the RDFC-1.0 hash (the chart IRI machinery). Offers are signed statements by a DID, discovery is a DID service entry plus an optional query; the document is fetched by hash from anywhere.

**Alternative rejected:** URI identity with a mutable document, and a registry.

- motivated_by: [[decisions#Content-addressed chart versions]]

### D4. A space is logical: pairwise sealed fan-out and a hash-linked history

No shared store and no group key. The sender seals one copy per recipient through the existing relays. Dependency hashes make the history a graph that participants compare, forward to late joiners and keep intact with hash-only entries for hidden messages.

**Alternatives rejected:** a physical shared log with group encryption (key management, selective visibility needs sub-groups).

- motivated_by: [[decisions#Identity, mailbox and transport layers]]
- motivated_by: [[constraints#Relays deliver at least once]]

### D5. Authority comes from the role; the join is countersigned; policy applies at join

A bound DID in an open instance may send exactly what its projection allows. Genesis is signed by the initiator, joins by each participant. The agent's policy runs on the genesis before the journal append and refuses by default.

**Alternative rejected:** grants on top of protocol checks: two sources of truth that can disagree.

- motivated_by: [[decisions#Grants enforced before the journal]]

### D6. Causal delivery with a bounded buffer; refusals come with proof

A message waits for its dependencies in a buffer whose size and timeout the protocol declares; a timeout takes the chart's cancel path. A refusal keeps the signed message, so anyone can re-run the check. Same-sequence double signing is equivocation.

**Alternative rejected:** strict rejection of out-of-order messages; it breaks under normal relay reordering.

- motivated_by: [[constraints#Relays deliver at least once]]
- motivated_by: [[constraints#Authcrypt is not verifiable by a third party]]

### D7. The core is a crate in the workspace with one-way dependencies

`vertep-kolyada` depends on `vertep-rdf` and primitives only. Federation, agent and the CLI depend on it, never the reverse. Vectors come first and Lean second.

**Alternative rejected:** growing `vertep-federation`; it would tie the protocol to Vertep's runtime and block other implementations.

- motivated_by: [[decisions#Lean verification bridge]]

### D8. JSON-LD in a restricted profile, with a SHACL subset for payloads

One frozen, embedded context, never fetched; any other context is rejected. The checker interprets the SHACL subset directly and a test is to tie it to full SHACL in `vertep-rdf`.

**Alternatives rejected:** full JSON-LD 1.1 (remote contexts, processor quirks); plain JSON with an RDF bridge (a second identity scheme).

- motivated_by: [[constraints#JSON-LD remote contexts are not loaded]]

### D9. Sign-then-seal

The body is signed with the sender's DID key before any transport seals it, so authenticity does not depend on the transport and a third party can verify it. The message hash covers the signature.

**Alternative rejected:** transport authentication only; it leaves violation proofs and late-join verification to trust the receiver.

- motivated_by: [[constraints#Authcrypt is not verifiable by a third party]]

### D10. Sub-protocol invocation builds the space as a tree

A transition opens a child instance; bindings come from the parent; the child's completion message closes the parent's step. Each checker sees one protocol.

**Alternative rejected:** several protocols running as peers in one instance; interleaving would break the local projection.

- motivated_by: [[decisions#Event-sourced actors]]

### D11. Binding: chart declaration, check before the journal, check on send, derived state

A chart declares `vt:plays`. The mailbox checks at the grants slot, journals the verdict with the input, and replay never verifies again. The send path checks the actor's own projection. Checker state is derived on replay and never stored.

- motivated_by: [[decisions#Verification outcome is journaled with the input]]
- motivated_by: [[decisions#Grants enforced before the journal]]

### D12. The vectors are the contract; Lean proves the core after them

Identities, projections, envelope hashes and verdict sequences are published as JSON vectors. The Lean model proves that accepted messages are enabled by the projection, that unbound roles are never accepted and that causally consistent orders converge.

- motivated_by: [[decisions#Lean verification bridge]]

## Constraints

- **Multiparty session types and choreography realizability** bind D2: projection is well defined when a single role decides at each branching point.
- **DIDComm v2 authcrypt** authenticates a sender only to the recipient. This binds D6 and D9: a third party cannot verify who sent a message without a signature of its own.
- **JSON-LD 1.1 remote contexts** are a known risk. This binds D8: bundled contexts only.
- **Relays deliver at least once and give no global order.** This binds D4 and D6.

## Risks / Trade-offs

- **Non-repudiation.** Signed messages cannot be denied. → Mitigation: this is the point of verifiable protocols; a deniable profile would be a separate encryption and signing profile.
- **Expressiveness.** No cross-role parallelism. → Mitigation: sub-protocols give parallel work as independent children, and the context version admits more later.
- **Different spellings of one meaning give different protocols if authored outside JSON-LD.** → Mitigation: the identity is the RDF graph's, so JSON-LD spellings agree; other surfaces convert first.
- **Key rotation inside a long instance.** → Mitigation: see Open Questions.
- **Buffer limits trade memory for tolerance.** → Mitigation: the protocol declares them, so both sides know.

## Migration Plan

- Nothing existing changes: grants, mailboxes and envelopes work as before for traffic outside protocols.
- Rollback: the facade feature `protocol` is off by default.

## Open Questions

- **Evolution.** A changed protocol has a new identity. Whether a document may name the protocol it replaces, advisory only, is undecided.
- **Key rotation.** Whether a signature is checked against the DID document version current at signing time or at receipt matters for long-lived instances.
- **Instance end and cleanup.** When a space is finished and which side may drop its history.
- **Private discovery.** The query message's own access control.
- **Mediator-side quotas** per protocol instance cannot read content; whether a sealed outer header could carry the instance id is open.
