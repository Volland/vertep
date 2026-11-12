## 1. Crate setup

- [x] 1.1 Create the `vertep-federation` crate, depending on `vertep-runtime`, with cargo features `didcomm`, `tsp` and `webvh`
- [x] 1.2 Choose and pin the DIDComm v2, TSP and DID resolution libraries after a short interop spike, and record the choice as a Decision section in `tg.md/decisions.md` (Decision `Pinned relay protocol libraries`: Affinidi DIDComm 2.1 and JOSE crates, OpenWallet `tsp_sdk` 0.16, DIF `didwebvh-rs` 0.8; wire compatibility checked against Affinidi's decrypt and tsp_sdk's open)
- [x] 1.3 Add a test mediator and a test TSP intermediary to the CI test fixtures (feature `testkit`: `TestMediator` and `TestIntermediary` on an in-process `LocalNetwork`)

## 2. DID resolution and directory

- [x] 2.1 Define the method resolver trait and the registry that lets applications add or override methods
- [x] 2.2 Implement the `did:key` resolver (offline)
- [x] 2.3 Implement the `did:peer` resolver
- [x] 2.4 Implement the `did:webvh` resolver with log verification
- [x] 2.5 Implement `DidDirectory`, which maps DID document service entries and keys to mailbox and transport routes, selected by the `did:` address scheme
- [x] 2.6 Add resolution caching with bounded lifetime and re-resolution on endpoint or key failure
- [x] 2.7 Raise `error.communication` for unsupported methods, missing compatible services and resolution timeouts, journaled in the sender

## 3. Keys and identities

- [x] 3.1 Define the `KeyManager` trait (sign, decrypt, key agreement) and an in-memory and file-backed implementation
- [x] 3.2 Add per-actor-type DID configuration so virtual actor instances get a DID as their address
- [x] 3.3 Implement key rotation with a grace period for retired keys and the recording of undecryptable messages
- [x] 3.4 Add a test that no private key material reaches journals, snapshots, datamodels or the graph store

## 4. Authenticated origin and grants

- [x] 4.1 Verify sender authentication in the shell and set `_event.origin` plus the verified flag, and add the verified-origin field to `_event` in `vertep-core` (`Event::originverified`, false by default so `..Event::default()` literals compile, set by `VerifiedOrigin::apply`; exposed as `_event.originverified` in ECMAScript and `rust`, `ev.originverified` in CEL, `Sys::verified_origin`; journaled as `vt:originVerified`; carried by the Lean `Event` and oracle JSON, with every third differential input from a verified sender; Decision `Verified origin flag in the event` supersedes `Verified origin carried by origin presence`)
- [x] 4.2 Journal the verification outcome with the input, and make replay use it without re-verifying (the outcome is the input's `origin`, set only for a verified sender, and `origintype`, both journaled; replay after the sender's key rotation is tested; the full outcome with the sender key is in graph `<actor>/inbound`)
- [x] 4.3 Apply the anonymous-sender policy per actor, and mark unverified events
- [x] 4.4 Implement grant statements (grantee, target, event descriptors, expiry) and inbound enforcement before the journal append
- [x] 4.5 Implement outbound grant enforcement that raises `error.communication` without producing an envelope
- [x] 4.6 Implement credential-based grants that verify issuer trust, subject, scope and validity

## 5. Identity ontology and graph

- [x] 5.1 Write the `vt-id:` ontology module aligned with DID Core (DID, verification method, service, grant), plus its SHACL shapes
- [x] 5.2 Project DID documents, public keys, services and grants into the runtime graph, keeping history across rotations
- [x] 5.3 Add SPARQL examples as tests: who may talk to an agent, and which events an agent accepted from a DID

## 6. Relay envelope

- [x] 6.1 Define the protocol-neutral event envelope (name, sendid, typed data, sender chart version, message id) and its JSON-LD shape
- [x] 6.2 Derive `origin` and `origintype` at the receiver from the verified envelope and the I/O processor, never from the body
- [x] 6.3 Type-check received payloads against the receiving chart's declared event types, and record rejections
- [x] 6.4 Ignore unknown extension headers

## 7. DIDComm v2 mailbox and transport

- [x] 7.1 Implement the DIDComm transport: authcrypt or anoncrypt to the recipient, and forward wrapping per routing key
- [x] 7.2 Implement the DIDComm mediator mailbox with batch pickup and a live pickup session (pickup operations behind `PickupSource`: delivery batch, messages-received acknowledgement, live session; their DIDComm Pickup 3.0 wire binding to a remote mediator belongs to 10.1)
- [x] 7.3 Acknowledge to the mediator only after the journal append or a recorded rejection
- [x] 7.4 Implement shared per-shard mailboxes with demultiplexing by recipient, and dedicated per-actor mailboxes
- [x] 7.5 Activate deactivated virtual actors on pickup, leaving messages unacknowledged if activation fails

## 8. TSP transport

- [x] 8.1 Map actor DIDs to TSP VIDs and implement direct mode, including relationship setup before the first event
- [x] 8.2 Implement routed mode through intermediaries with a nested payload
- [x] 8.3 Implement the TSP intermediary mailbox where the deployment needs store-and-forward (the intermediary holds routed messages for pickup; the recipient's relay mailbox picks up from it like from a mediator)

## 9. Delivery semantics

- [x] 9.1 Wire relay sends through the outbox with retry policy, background retries and `error.communication` on exhaustion, carrying the send id
- [x] 9.2 Implement transport selection from DID document services with configurable preference and recorded fallback
- [x] 9.3 Add deduplication tests for relay redelivery, lost acknowledgements and sender retries
- [x] 9.4 Add tests that per-mailbox FIFO order is preserved

## 10. Interop and end-to-end tests

- [x] 10.1 Interop test against an existing third-party DIDComm v2 mediator: forward, pickup, acknowledgement (`pickup::MediatorClient` binds Mediator Coordination 2.0 and Message Pickup 3.0; `tests/mediator_interop.rs` runs against the Hyperledger Identus mediator `ghcr.io/hyperledger/identus-mediator:1.0.0` with `mongo:6.0.19` in Docker when `VERTEP_MEDIATOR_INTEROP=1`, or a running one at `VERTEP_MEDIATOR_URL`, and skips otherwise; the same wire path runs in CI against the in-process test mediator, which answers those messages; live delivery over WebSocket is not bound)
- [x] 10.2 Interop test against the TSP reference implementation: direct and routed modes
- [x] 10.3 End-to-end test across two clusters through a mediator: verified origin, guard on a sender DID, grant refusal (`two_clusters_through_a_mediator` in `tests/identity_runtime.rs`)
- [x] 10.4 Crash-recovery test: kill the receiving node between journal append and acknowledgement, and assert a single processing (`crash_before_acknowledgement` in `tests/relay_didcomm.rs`, with the runtime's `CrashPoint::AfterAppend`)

## 11. Traceability and docs

- [x] 11.1 Annotate the code that implements each `did-identity` requirement with `// @tg: implements:: [[openspec:did-identity#<Requirement>]]` above the function, type or file, adding `{confidence: partial}` where it applies
- [x] 11.2 Annotate the code that implements each `relay-mailboxes` requirement with `// @tg: implements:: [[openspec:relay-mailboxes#<Requirement>]]`
- [x] 11.3 Annotate each test that proves a scenario with `// @tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the test
- [x] 11.4 Record the design decisions D1 to D9 that are new as `Decision` sections in `tg.md/decisions.md`, with `motivated_by` edges to [[decisions#Identity, mailbox and transport layers]] and [[constraints#Relays deliver at least once]], and add `constrains` edges from constraints to the new requirements (Decision sections added; the `constrains` edges to the new requirements DONE AS FRONTMATTER: tg 0.22 does not resolve doc links to openspec requirements, so constraints constrain the new decisions and code carries @tg annotations)
- [x] 11.5 DONE AS FRONTMATTER (tg 0.22 does not resolve doc links to openspec requirements; capabilities.md lists both capabilities in its openspec frontmatter and code carries @tg annotations), was: Update [[capabilities#DID identity]] and [[capabilities#Relay mailboxes]] with `specified_by` edges to their requirements, and keep both capabilities in the file's `openspec:` frontmatter
- [x] 11.6 DONE AS FRONTMATTER (tg 0.22 does not resolve doc links to openspec requirements; the component's `provides` edges name both capabilities and code carries @tg implements annotations), was: Update [[components#vertep-federation]] with `provides` edges to the implemented requirements
- [x] 11.7 Run `tg check`, `tg ontology check`, `tg trace did-identity --gaps` and `tg trace relay-mailboxes --gaps`, and close every gap they report
