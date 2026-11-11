## 1. Crate setup

- [ ] 1.1 Create the `vertep-federation` crate, depending on `vertep-runtime`, with cargo features `didcomm`, `tsp` and `webvh`
- [ ] 1.2 Choose and pin the DIDComm v2, TSP and DID resolution libraries after a short interop spike, and record the choice as a Decision section in `tg.md/decisions.md`
- [ ] 1.3 Add a test mediator and a test TSP intermediary to the CI test fixtures

## 2. DID resolution and directory

- [ ] 2.1 Define the method resolver trait and the registry that lets applications add or override methods
- [ ] 2.2 Implement the `did:key` resolver (offline)
- [ ] 2.3 Implement the `did:peer` resolver
- [ ] 2.4 Implement the `did:webvh` resolver with log verification
- [ ] 2.5 Implement `DidDirectory`, which maps DID document service entries and keys to mailbox and transport routes, selected by the `did:` address scheme
- [ ] 2.6 Add resolution caching with bounded lifetime and re-resolution on endpoint or key failure
- [ ] 2.7 Raise `error.communication` for unsupported methods, missing compatible services and resolution timeouts, journaled in the sender

## 3. Keys and identities

- [ ] 3.1 Define the `KeyManager` trait (sign, decrypt, key agreement) and an in-memory and file-backed implementation
- [ ] 3.2 Add per-actor-type DID configuration so virtual actor instances get a DID as their address
- [ ] 3.3 Implement key rotation with a grace period for retired keys and the recording of undecryptable messages
- [ ] 3.4 Add a test that no private key material reaches journals, snapshots, datamodels or the graph store

## 4. Authenticated origin and grants

- [ ] 4.1 Verify sender authentication in the shell and set `_event.origin` plus the verified flag, and add the verified-origin field to `_event` in `vertep-core`
- [ ] 4.2 Journal the verification outcome with the input, and make replay use it without re-verifying
- [ ] 4.3 Apply the anonymous-sender policy per actor, and mark unverified events
- [ ] 4.4 Implement grant statements (grantee, target, event descriptors, expiry) and inbound enforcement before the journal append
- [ ] 4.5 Implement outbound grant enforcement that raises `error.communication` without producing an envelope
- [ ] 4.6 Implement credential-based grants that verify issuer trust, subject, scope and validity

## 5. Identity ontology and graph

- [ ] 5.1 Write the `vt-id:` ontology module aligned with DID Core (DID, verification method, service, grant), plus its SHACL shapes
- [ ] 5.2 Project DID documents, public keys, services and grants into the runtime graph, keeping history across rotations
- [ ] 5.3 Add SPARQL examples as tests: who may talk to an agent, and which events an agent accepted from a DID

## 6. Relay envelope

- [ ] 6.1 Define the protocol-neutral event envelope (name, sendid, typed data, sender chart version, message id) and its JSON-LD shape
- [ ] 6.2 Derive `origin` and `origintype` at the receiver from the verified envelope and the I/O processor, never from the body
- [ ] 6.3 Type-check received payloads against the receiving chart's declared event types, and record rejections
- [ ] 6.4 Ignore unknown extension headers

## 7. DIDComm v2 mailbox and transport

- [ ] 7.1 Implement the DIDComm transport: authcrypt or anoncrypt to the recipient, and forward wrapping per routing key
- [ ] 7.2 Implement the DIDComm mediator mailbox with batch pickup and a live pickup session
- [ ] 7.3 Acknowledge to the mediator only after the journal append or a recorded rejection
- [ ] 7.4 Implement shared per-shard mailboxes with demultiplexing by recipient, and dedicated per-actor mailboxes
- [ ] 7.5 Activate deactivated virtual actors on pickup, leaving messages unacknowledged if activation fails

## 8. TSP transport

- [ ] 8.1 Map actor DIDs to TSP VIDs and implement direct mode, including relationship setup before the first event
- [ ] 8.2 Implement routed mode through intermediaries with a nested payload
- [ ] 8.3 Implement the TSP intermediary mailbox where the deployment needs store-and-forward

## 9. Delivery semantics

- [ ] 9.1 Wire relay sends through the outbox with retry policy, background retries and `error.communication` on exhaustion, carrying the send id
- [ ] 9.2 Implement transport selection from DID document services with configurable preference and recorded fallback
- [ ] 9.3 Add deduplication tests for relay redelivery, lost acknowledgements and sender retries
- [ ] 9.4 Add tests that per-mailbox FIFO order is preserved

## 10. Interop and end-to-end tests

- [ ] 10.1 Interop test against an existing third-party DIDComm v2 mediator: forward, pickup, acknowledgement
- [ ] 10.2 Interop test against the TSP reference implementation: direct and routed modes
- [ ] 10.3 End-to-end test across two clusters through a mediator: verified origin, guard on a sender DID, grant refusal
- [ ] 10.4 Crash-recovery test: kill the receiving node between journal append and acknowledgement, and assert a single processing

## 11. Traceability and docs

- [ ] 11.1 Annotate the code that implements each `did-identity` requirement with `// @tg: implements:: [[openspec:did-identity#<Requirement>]]` above the function, type or file, adding `{confidence: partial}` where it applies
- [ ] 11.2 Annotate the code that implements each `relay-mailboxes` requirement with `// @tg: implements:: [[openspec:relay-mailboxes#<Requirement>]]`
- [ ] 11.3 Annotate each test that proves a scenario with `// @tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the test
- [ ] 11.4 Record the design decisions D1 to D9 that are new as `Decision` sections in `tg.md/decisions.md`, with `motivated_by` edges to [[decisions#Identity, mailbox and transport layers]] and [[constraints#Relays deliver at least once]], and add `constrains` edges from constraints to the new requirements
- [ ] 11.5 Update [[capabilities#DID identity]] and [[capabilities#Relay mailboxes]] with `specified_by` edges to their requirements, and keep both capabilities in the file's `openspec:` frontmatter
- [ ] 11.6 Update [[components#vertep-federation]] with `provides` edges to the implemented requirements
- [ ] 11.7 Run `tg check`, `tg ontology check`, `tg trace did-identity --gaps` and `tg trace relay-mailboxes --gaps`, and close every gap they report
