## Why

Up to M4, every Vertep actor lives inside one trusted cluster: addresses are cheap `vt:` IRIs, the directory is cluster membership, and `_event.origin` is whatever the sender claims. Agents and workflows that span organisations need to send messages to actors they do not operate, through relays they do not trust, and need charts to know *who* really sent an event. M5 adds that cross-trust layer without changing the identity / mailbox / transport split decided earlier, so a chart's `<send>` is the same whether the target is local, in the cluster or behind a DIDComm or TSP mediator.

## What Changes

- Actors may opt in, one by one, to a DID identity (`did:peer`, `did:webvh`, `did:key` through a pluggable resolver). Actors without one keep cheap `vt:` IRIs and pay no cryptographic cost.
- A DID resolver becomes a `Directory` implementation: resolving a DID yields the DID document's service endpoints, which name the actor's mailboxes and transports.
- A key management trait holds the keys of DID actors, signs and decrypts on their behalf, and supports rotation without changing the actor's address.
- Messages crossing a trust boundary are end-to-end encrypted and sender-authenticated. `_event.origin` carries the verified sender DID, and unauthenticated events are explicitly marked so guards can reject them.
- Capability grants scoped to DIDs decide which senders may reach which events of a DID actor. Ungranted sends fail with `error.communication`, and so do ungranted inbound messages.
- DID documents, keys and grants are linked into the runtime ontology graph, so "who may talk to this agent" is a query.
- New `Mailbox` and `Transport` implementations for DIDComm v2: store-and-forward mediators, forward routing and message pickup.
- New `Mailbox` and `Transport` implementations for the ToIP Trust Spanning Protocol: direct mode, and routed mode through intermediaries.
- Relays receive only encrypted envelopes. Delivery is at least once, and the receiving journal deduplicates by message id.
- SCXML events map onto relay envelopes losslessly (`name`, `data`, `sendid`, `origin`, `origintype`). A mediator can hold one mailbox per actor or a shared mailbox for many actors. A message arriving at a deactivated virtual actor's mailbox activates it.
- Undeliverable messages raise `error.communication` in the sending chart.

## Capabilities

### New Capabilities
- `did-identity`: Opt-in DID identities for actors, DID resolution as address resolution, key management and rotation, authenticated `_event.origin`, DID-scoped capability grants, and DID documents linked into the runtime graph.
- `relay-mailboxes`: DIDComm v2 and TSP mailboxes and transports covering store-and-forward mediators, routing through intermediaries, end-to-end encryption, envelope mapping of SCXML events, at-least-once delivery with deduplication, activation on delivery, and failure reporting.

### Modified Capabilities
<!-- none: the runtime traits from M2 and M4 are implemented, not changed -->

## Impact

- **Code:** a new crate `vertep-federation` that depends on `vertep-runtime` and implements its `Directory`, `Mailbox` and `Transport` traits. It adds no changes to `vertep-core`; the pure `step` sees only an extra verified-origin field in `_event`.
- **Dependencies:** a DIDComm v2 library and a TSP library (Rust implementations exist for both), plus a DID resolution library for `did:key`, `did:peer` and `did:webvh`. All of them sit behind cargo features.
- **tg sections touched:** [[capabilities#DID identity]], [[capabilities#Relay mailboxes]], [[components#vertep-federation]], [[roadmap#M5 Federation]], [[glossary#Mailbox]].
- **Binding decisions and constraints:**
  - [[decisions#Identity, mailbox and transport layers]]: DIDs only at trust boundaries; mailboxes outlive activations.
  - [[decisions#IRI identity and lifecycle classes]]: a DID is one IRI scheme, and virtual actors are reached only by messages.
  - [[decisions#Event-sourced actors]]: deduplication by message id, inbound messages journaled as inputs.
  - [[decisions#Modular vt ontology on RDF 1.2]]: the `vt-id:` identity module aligned with DID Core.
  - [[decisions#Non-blocking request and deferred events]]: `error.communication` and timeouts instead of blocking.
  - [[constraints#Relays deliver at least once]]: exactly-once delivery across relays is impossible.
- **Earlier milestones relied on:** `actor-model`, `messaging`, `event-journal`, `runtime-ontology` (M2) and `cluster-transport` (M4) are used as they are, and none of their requirements change.
