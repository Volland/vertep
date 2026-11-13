## 1. Graph first

- [x] 1.1 Add milestone `M8 did:webvh hosting` to `tg.md/roadmap.md` with `precedes::` from M7, and add it to the roadmap diagram
- [x] 1.2 Add a Component section `vertep-webvh` to `tg.md/components.md` with `provides::` (webvh-identity, webvh-hosting), `uses::`, `realizes::` and `scheduled_in::`, before code lands; add the crate to the component diagram
- [x] 1.3 Add Capability sections `webvh identity` and `webvh hosting` to `tg.md/capabilities.md` and list `webvh-identity` and `webvh-hosting` in its `openspec:` frontmatter
- [x] 1.4 Record design decisions D1–D9 as `Decision` sections in `tg.md/decisions.md` with the `motivated_by::` edges design.md names
- [x] 1.5 Record the constraints (witnesses are did:key, versionTime increases, domains over HTTPS, async library) in `tg.md/constraints.md` with `source:` and `constrains::` edges

## 2. Signing and authoring

- [x] 2.1 Create the crate `vertep-webvh` (workspace member and dependency) on `vertep-federation`, `vertep-objects` and `didwebvh-rs` 0.8.2 (`default-features = false`)
- [x] 2.2 Implement `KmSigner`: `didwebvh-rs` `Signer` over `KeyManager` (Ed25519, `did:key` verification method); a test that signatures verify and no secret leaves the key manager
- [x] 2.3 Implement `create` from a Vertep identity and services, with optional pre-rotation and witnesses (`Create With Vertep Keys`, `Document From An Identity`)
- [x] 2.4 Implement `update`, `rotate` (with pre-rotation enforcement) and `deactivate`, with strictly increasing `versionTime` (`Update And Rotate`, `Deactivate`)
- [x] 2.5 Implement in-memory verification of a log plus its witness file through `didwebvh-rs` `resolve_log`, used by every publisher (`Verify Before Publish`)
- [x] 2.6 Annotate with `@tg: implements::`; tests for every scenario of the four requirements with `@tg: verifies::`

## 3. Custodian witnesses

- [x] 3.1 Derive a custodian's witness `did:key` from its Ed25519 signing key; build the witness parameter (custodians, threshold = quorum) for object DIDs (`Custodians Witness Object Logs`)
- [x] 3.2 Add `CustodyRequest::Witness` and its responses to the custody wire format and ontology, and an `EntryWitness` trait in `vertep-objects` that `Replica` consults
- [x] 3.3 Implement the webvh `EntryWitness`: next-entry validity, update-key signature, and `VertepCustody` equal to the accepted effective or pending configuration; proof made through `KeyManager` (`Custodian Witnessing Rules`)
- [x] 3.4 Implement `WebvhPublisher` as custody's `DocumentPublisher`: append the controller-signed entry, collect witness proofs over the custody transports, verify, and publish (`Custody Changes Publish As Log Entries`)
- [x] 3.5 Implement `vt webvh custody change`, which builds the next log entry and the signed `ReconfigurationRequest`
- [x] 3.6 Tests: witnessed by a quorum, too few witnesses, valid entry witnessed, unaccepted set refused, forged entry refused, replace a keeper end to end (in process and over TSP)

## 4. Publishing and serving

- [x] 4.1 Implement the file layout and `did.json` export (`Published File Layout`)
- [x] 4.2 Implement `DirectoryPublisher`, `GitPublisher` and `CommandPublisher` behind `LogPublisher` (`Log Publishers`)
- [x] 4.3 Implement the axum server (feature `server`): content types, CORS, cache by `ttl`, HEAD and GET, 404, no path traversal (`Serving Logs`)
- [x] 4.4 Implement an `HttpFetcher` that maps domains to directories or local servers, for tests and offline verification
- [x] 4.5 Tests: layout, git publish into a temporary bare remote, command failure, resolver resolves from the server, path traversal refused

## 5. Commands and the mediator

- [x] 5.1 Add `vt webvh` to `vertep-cli`: `create`, `update`, `rotate`, `deactivate`, `verify`, `witness`, `publish`, `serve`, `object create`, `custody change`, with a file key manager directory (`vt webvh Commands`)
- [x] 5.2 CLI tests for create-then-verify and object create, with recorded outputs
- [x] 5.3 Support `did = "webvh:<host>[:<path>]"` in `vt-mediator`: create on first start, store under the data directory, serve at the DID's path, keep it across restarts (`Mediator Under did:webvh`), with tests
- [x] 5.4 Interop: publish and resolve the `didwebvh-rs` 1.0 test vectors through Vertep, and verify Vertep-written logs with `didwebvh-rs` alone (`Third-Party Logs Interoperate`)

## 6. Docs and closing checks

- [x] 6.1 Extend `tg.md/views/durable-objects.md` (or a new view) with did:webvh authoring, witnessing and hosting, with `describes::` edges and a Mermaid sequence of a witnessed custody change
- [x] 6.2 Add a `vertep-webvh` README, a docs page section on hosting `did:webvh`, and an `[Unreleased]` entry in `CHANGELOG.md`
- [x] 6.3 `tg check`, `tg ontology check` and `tg trace webvh-identity webvh-hosting --gaps` show no new gaps
