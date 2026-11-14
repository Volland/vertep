# did-identity Specification

## Purpose
Gives actors that cross a trust boundary a DID identity: their address resolves through DID documents to mailboxes, their messages are signed and verified, and DID-scoped grants decide which senders may reach which of their events.

## Requirements

### Requirement: Opt-In DID Identity Per Actor
The runtime SHALL let each actor be given a DID identity individually, and SHALL NOT require a DID, keys or cryptographic operations for actors without one. An actor with a DID SHALL use the DID as its address IRI, so the DID is the RDF subject of its state and history.

#### Scenario: Actor without a DID sends inside the cluster
- **WHEN** an actor addressed by a `vt:` IRI sends an event to another `vt:` actor in the same trusted cluster
- **THEN** the message is delivered without resolving any DID, signing or encrypting

#### Scenario: Actor configured with a DID
- **WHEN** a virtual actor type is configured so that each instance gets a `did:peer` identity
- **THEN** a new instance's address is its DID, and its journal, snapshots and current-state graph use that DID as subject

#### Scenario: Mixed addressing in one chart
- **WHEN** a chart sends one event to a `vt:` IRI and another to a DID in the same macrostep
- **THEN** both sends are journaled in the outbox and each is resolved through the directory that matches its address scheme

### Requirement: Supported DID Methods
The federation layer SHALL support the `did:key`, `did:peer` and `did:webvh` methods, and SHALL report an unsupported method as a resolution failure instead of guessing.

#### Scenario: did:key resolves offline
- **WHEN** a `did:key` address is resolved
- **THEN** its DID document is derived from the key itself, with no network access

#### Scenario: did:webvh resolves with history verification
- **WHEN** a `did:webvh` address is resolved
- **THEN** the resolver fetches its log, verifies the chain of entries, and returns the current DID document only if verification succeeds

#### Scenario: Unsupported method
- **WHEN** a chart sends to `did:example:123` and no resolver handles `did:example`
- **THEN** the send fails with `error.communication` naming the unsupported method

### Requirement: Pluggable DID Resolver
The federation layer SHALL resolve DIDs through a pluggable resolver interface, so applications can add DID methods or replace a method's resolver without changing the runtime.

#### Scenario: Application registers a custom method
- **WHEN** an application registers a resolver for a DID method that Vertep does not ship
- **THEN** sends to DIDs of that method are resolved by the registered resolver

#### Scenario: Application overrides a built-in method
- **WHEN** an application registers its own `did:webvh` resolver
- **THEN** the runtime uses it instead of the built-in one for every `did:webvh` address

### Requirement: DID Resolution As Address Resolution
A DID resolver SHALL act as a directory: resolving a DID address SHALL yield its mailbox and transport endpoints from the DID document's service entries, plus the keys used to encrypt to the actor and verify its signatures.

#### Scenario: Service endpoint becomes a mailbox
- **WHEN** an actor's DID document lists a DIDComm messaging service whose endpoint is a mediator
- **THEN** the resolved route delivers to that mediator's mailbox for the actor

#### Scenario: No usable service entry
- **WHEN** a DID document has no service entry for any transport the runtime supports
- **THEN** the send fails with `error.communication` stating that no compatible endpoint exists

#### Scenario: Resolution failure is reported, not retried forever
- **WHEN** resolving a DID fails because of a network error past the configured resolution timeout
- **THEN** the send fails with `error.communication`, and the failure is journaled in the sender's journal

### Requirement: Resolution Caching
The federation layer SHALL cache resolved DID documents with a bounded lifetime, and SHALL re-resolve on expiry or when a message to a cached endpoint fails with a key or endpoint error.

#### Scenario: Cached document is reused
- **WHEN** two sends to the same DID happen within the cache lifetime
- **THEN** the DID is resolved once

#### Scenario: Stale endpoint triggers re-resolution
- **WHEN** delivery to a cached endpoint fails because the endpoint no longer exists
- **THEN** the DID is resolved again and delivery is retried once with the fresh document before failing

### Requirement: Key Management Interface
The federation layer SHALL keep the private keys of DID actors behind a key management interface that signs, decrypts and derives keys on the actor's behalf. Private key material SHALL NOT appear in journals, snapshots, the graph store or chart datamodels.

#### Scenario: Signing without exposing keys
- **WHEN** a DID actor sends an authenticated message
- **THEN** the envelope is produced through the key management interface, and the chart's datamodel never holds key material

#### Scenario: Pluggable key store
- **WHEN** an application supplies a key management implementation backed by a hardware module or a remote key service
- **THEN** all signing and decryption for that application's DID actors go through it

#### Scenario: Graph contains only public keys
- **WHEN** the runtime graph of a DID actor is exported
- **THEN** it contains the actor's verification methods and public keys, and no private key

### Requirement: Key Rotation Without Address Change
A DID actor SHALL be able to rotate its keys without changing its DID address. Messages encrypted to a superseded key SHALL remain decryptable for a configured grace period.

#### Scenario: Rotation keeps the address
- **WHEN** a `did:webvh` actor rotates its keys
- **THEN** its DID stays the same, its updated DID document lists the new keys, and senders that re-resolve encrypt to the new keys

#### Scenario: Message in flight during rotation
- **WHEN** a message encrypted to the previous key arrives within the grace period
- **THEN** it is decrypted and delivered

#### Scenario: Message after the grace period
- **WHEN** a message encrypted to a retired key arrives after the grace period
- **THEN** it is not delivered, and it is recorded as an undecryptable message with its sender if known

### Requirement: Authenticated Event Origin
For a message received across a trust boundary, the runtime SHALL set `_event.origin` to the sender DID only when the sender's signature or authenticated encryption has been verified against the sender's resolved DID document, and SHALL record that verification in `_event`.

#### Scenario: Verified sender
- **WHEN** a DID actor receives an authenticated message from `did:peer:2...A` whose authentication verifies
- **THEN** `_event.origin` is `did:peer:2...A` and `_event` marks the origin as verified

#### Scenario: Guard accepts only a specific sender
- **WHEN** a transition's guard requires a verified origin equal to an approver's DID, and an `approve` event arrives from another DID
- **THEN** the guard evaluates to false and the transition is not taken

#### Scenario: Forged origin claim
- **WHEN** a message claims an origin DID but its authentication does not verify against that DID's keys
- **THEN** the message is not delivered as coming from that DID, and the rejection is recorded

### Requirement: Unauthenticated Event Marking
Messages received without sender authentication, such as anonymously encrypted messages, SHALL be delivered only if the receiving actor's policy allows anonymous senders, and SHALL be marked unverified so guards can reject them.

#### Scenario: Anonymous message allowed
- **WHEN** an actor that allows anonymous senders receives an anonymously encrypted message
- **THEN** the event is delivered with its origin marked unverified and no sender DID in `_event.origin`

#### Scenario: Anonymous message refused by policy
- **WHEN** an actor whose policy requires authenticated senders receives an anonymously encrypted message
- **THEN** the message is not delivered and is recorded as rejected

### Requirement: Journaled Verification Outcome
The outcome of origin verification SHALL be recorded in the receiving actor's journal with the input event, and replay SHALL use the recorded outcome without re-verifying or re-resolving.

#### Scenario: Replay after key rotation
- **WHEN** an actor's journal is replayed after the sender has rotated its keys
- **THEN** each event's origin and verified flag are those recorded at receipt, and replay produces the same state

#### Scenario: Audit query on verified origins
- **WHEN** an auditor queries the graph for every event an actor accepted from a given DID
- **THEN** the result includes each journaled event with its verification outcome

### Requirement: DID-Scoped Capability Grants
A DID actor SHALL accept a message from another DID only if a grant allows that sender to send that event to it. A grant SHALL name the grantee DID, the target actor and a set of event descriptors, and MAY carry an expiry.

#### Scenario: Granted sender
- **WHEN** an actor holds a grant allowing `did:key:z6Mk...B` to send `order.*` and that DID sends `order.placed`
- **THEN** the event is delivered

#### Scenario: Event outside the grant
- **WHEN** the same sender sends `admin.reset` to that actor
- **THEN** the event is not delivered, the sender receives `error.communication` if a reply path exists, and the refusal is recorded

#### Scenario: Expired grant
- **WHEN** a grant has expired and its grantee sends a granted event
- **THEN** the event is refused as ungranted

### Requirement: Outbound Grant Enforcement
A chart SHALL be able to send across a trust boundary only to targets its actor is granted to reach. Sending to an ungranted cross-trust target SHALL raise `error.communication` in the sending chart without anything leaving the node.

#### Scenario: Ungranted outbound send
- **WHEN** an invoked LLM actor without a grant for an external DID tries to send to it
- **THEN** `error.communication` is raised in the sender and no envelope is produced

#### Scenario: Granted outbound send
- **WHEN** the invoking state granted the actor sends to that DID
- **THEN** the message is encrypted and handed to the resolved transport

### Requirement: Credential-Based Grants
The federation layer SHALL accept grants presented as verifiable credentials issued by a DID the receiving actor trusts, verifying the issuer signature, subject, scope and validity period before honouring them.

#### Scenario: Valid credential grant
- **WHEN** a sender presents a credential, issued by a DID the receiver trusts, granting it `quote.*` on the receiver
- **THEN** the receiver delivers the sender's `quote.request` events

#### Scenario: Credential from an untrusted issuer
- **WHEN** a sender presents a well-formed credential issued by a DID the receiver does not trust
- **THEN** the grant is ignored and the sender's events are refused as ungranted

### Requirement: DID Documents In The Runtime Graph
The runtime graph SHALL link each DID actor to its DID, its verification methods, its service endpoints and the grants it holds and has issued, using the identity ontology module aligned with DID Core, so that reachability and authority are answerable by graph queries.

#### Scenario: Who may talk to this agent
- **WHEN** a SPARQL query asks for every grantee DID allowed to send to an agent and the event descriptors each may send
- **THEN** the graph store returns them from the agent's grant statements

#### Scenario: Document change is history, not overwrite
- **WHEN** an actor's DID document changes after a key rotation
- **THEN** the graph records the new document state and keeps the previous one queryable through time travel

### Requirement: Grant Credentials From A Wallet
A grant credential SHALL be holdable in a wallet like any other credential. Its grantee SHALL be able to present it to the target's verifier over any conformant exchange protocol. A presented grant credential SHALL yield its grant only after it is verified, including its status, and the grant SHALL be enforced before any journal append, exactly like a configured grant.

#### Scenario: Grant presented over an exchange
- **WHEN** an issuer the target trusts issues a grant credential to an agent's wallet, and the agent presents it to the target over the native TSP exchange
- **THEN** the target holds the grant, and the agent's granted events are delivered

#### Scenario: Presented grant enforced before the journal
- **WHEN** the same agent then sends an event outside the presented grant
- **THEN** the event is refused at the mailbox, and nothing is appended to the target's journal

#### Scenario: Revoked grant credential refused
- **WHEN** the issuer revokes the grant credential, and the agent presents it again
- **THEN** verification fails, the target holds no grant from it, and the agent's events are refused as ungranted
