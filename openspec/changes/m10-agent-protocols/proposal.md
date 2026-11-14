## Why

Vertep actors can talk across trust boundaries: they have DIDs, sealed mailboxes, TSP and DIDComm relays, and per-DID grants. What they cannot do is say, in a form a stranger can verify, **how to talk to them**:

- which messages an agent accepts, in which order, and from whom;
- what the handshake is before any work starts;
- how several agents cooperate in one conversation without a shared server.

A grant says "this DID may send this event". It says nothing about order, about a second party's replies, or about three parties in one exchange. Decentralised Web Nodes solve a part of this with protocol definitions over a shared record store. Vertep needs the same idea for agents that are statecharts, over mailboxes that only hold ciphertext.

## What Changes

- **New crate `vertep-kolyada`:** the protocol core, small and portable. Kolyada is the name of the whole layer, after the Ukrainian carol that singers perform from a known script, door to door. It depends on `vertep-rdf` (JSON-LD, RDFC-1.0, SHACL) and on no interpreter, runtime, store or transport.
  - A **protocol document**: one JSON-LD document in a restricted profile with a frozen, embedded `vk:` context. It declares roles, message types with payload shapes, and a choreography of states and transitions.
  - **Identity by hash**: `urn:vertep:protocol:sha256:` and the RDFC-1.0 hash of the graph.
  - **Role projection** and an interface view ("what this role accepts here").
  - **Signed envelope**: sign-then-seal, transport-agnostic, with dependency hashes.
  - **`accept(state, message)`**: a pure conformance function with a fixed list of refusal reasons.
  - **Causal delivery** with a protocol-declared bounded buffer, violation records and equivocation proofs.
  - **Test vectors** usable from any language.
- **Spaces in `vertep-federation`:**
  - signed genesis, countersigned joins, and an agent join policy that refuses by default;
  - authority derived from the role: the protocol is the permission document;
  - pairwise sealed fan-out through the existing mailboxes and relays, with hash-linked history, late join and hash-only entries for hidden messages;
  - sub-protocol invocation: a space is a tree of instances with one shared history;
  - protocol offers as signed statements and a `VertepProtocol` DID service entry.
- **Runtime binding:** a chart declares `vt:plays` (protocol, role). The mailbox runs the check where grants run, before the journal; the send path runs it on outbound messages; the checker state is derived on replay.
- **The RDF bridge**, in `vertep-kolyada` (`vertep-rdf` cannot depend on it): protocol graphs and offers as triples, so "which DIDs offer this role" is a SPARQL query.
- **Lean:** a model of projection and `accept` with three theorems, an unsafe variant and a checker `vertep-kolyada`. This is the second stage of the milestone, after the vectors.
- **`vt kolyada`:** `validate`, `hash`, `project`, `offer`, `check` and `vectors`.

**Assumptions** (adjust before applying if they are wrong):

- The existing DID grants stay for traffic outside any protocol; they never widen a protocol role.
- Cross-role parallelism and choices decided by several roles are out of scope for the format version 1, and the `vk:` context version admits them later.
- Group encryption (one ciphertext for a whole space) is out of scope; the message format admits it as an encryption profile later.

## Capabilities

### New Capabilities

- `agent-protocols`: the protocol document and its profile, identity, shapes, well-formedness, projection, offers, discovery and the RDF bridge.
- `protocol-conformance`: the signed envelope, `accept`, causal delivery, violation records, test vectors and the Lean model.
- `protocol-spaces`: instances, joining and join policy, role-derived authority, sealed fan-out, hash-linked history, sub-protocols and the chart and mailbox binding.

### Modified Capabilities

None. `did-identity` grants and `relay-mailboxes` are used as they are.

## Impact

- **Code:**
  - New crate `vertep-kolyada`, with feature `vectors` for the vector generator.
  - `vertep-federation`: module `protocol` (instances, policy, inbound and outbound checks, DID service entry, signer and resolver adapters).
  - `vertep-agent` and `vertep-runtime`: the `vt:plays` declaration and the derived checker state.
  - `vertep-cli`: `vt kolyada`.
  - `lean/vertep-lean`: modules `Protocol`, `ProtocolTheorems`, `ProtocolUnsafe`, `ProtocolCheck`, executable `vertep-kolyada`.
  - Facade `vertep`: feature `protocol`.
- **Dependencies:** none new beyond `vertep-rdf` and the JSON-LD parser already in the workspace; Ed25519 signing goes through the key manager.
- **The graph (`tg.md`):** a component `vertep-kolyada`, milestone `M10 Agent protocols`, a view `tg.md/views/kolyada.md`, three capabilities, decisions D1-D12 and two constraints.
- **Decisions this change builds on:**
  - [[decisions#Grants enforced before the journal]]
  - [[decisions#Keys stay behind a key manager]]
  - [[decisions#Protocol-neutral event envelope]]
  - [[decisions#TSP direct when reachable, routed otherwise]]
  - [[decisions#Lean verification bridge]]
