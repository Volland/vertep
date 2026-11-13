## Context

See proposal.md for the motivation. Where things stand:

- **Resolution:** `vertep-federation` resolves `did:webvh` through `WebVhResolver` (feature `webvh`), which fetches the log with an `HttpFetcher` and verifies it with the DIF `didwebvh-rs` 0.8.2.
- **The library:** `didwebvh-rs` also creates, updates, rotates and deactivates logs and handles witnesses. Signatures go through the `affinidi-data-integrity` `Signer` trait: a key type, a verification method and an async `sign(&[u8])`. The library is async; federation already drives it with `futures::executor`.
- **Custody:** custody (`vertep-objects`) declares its custodians in a `VertepCustody` service. A `ReconfigurationRequest` already carries an optional `did_log_entry`, and `DocumentPublisher` is the publishing hook, so far with only `RecordingPublisher`.
- **Mediator:** `vt-mediator` supports `did = "peer"` and `did = "web:<host>"`, and serves `/.well-known/did.json` from its axum router.
- **Spec:** `did:webvh` 1.0 places the log at `/.well-known/did.jsonl` (no path) or `/<path>/did.jsonl`, with `did-witness.json` beside it. Witnesses are `did:key` DIDs with a threshold, and update keys may be pre-rotated through hashes the previous entry announces.

## Goals / Non-Goals

**Goals:**

- Every `did:webvh` operation an operator needs, signed through `KeyManager` and verified before publishing.
- A durable object's DID log can change only with its custodians' consent, as witnesses.
- Hosting on whatever the operator already has: a directory behind a web server, a git repository behind Pages, a bucket.

**Non-Goals:**

- Migrating DIDs between domains.
- Watchers.
- A hosted multi-tenant service.
- TLS termination: `vt webvh serve` sits behind a proxy or a CDN.

## Decisions

### D1. A new crate `vertep-webvh` on `didwebvh-rs`
Authoring, witnessing, publishing and serving live in `vertep-webvh`, which depends on `vertep-federation` and `vertep-objects`. Log mechanics (SCID, entry hashes, proofs, pre-rotation, witness proof format) stay in `didwebvh-rs`.

- motivated_by: [[decisions#Pinned relay protocol libraries]].

Alternatives rejected:

- **Writing our own log code:** a second implementation of a security-critical format.
- **Putting authoring in `vertep-federation`:** that crate is the client side used by every node. Authoring brings git, HTTP serving and files, which nodes don't need.

### D2. `KeyManager` behind `didwebvh-rs`'s `Signer`
`KmSigner` implements `Signer` with key type Ed25519 and verification method `did:key:<multikey>#<multikey>` of the key manager's public key. `sign` calls `KeyManager::sign`. Update keys, witness keys and the object's keys are all key-manager keys.

- motivated_by: [[decisions#Keys stay behind a key manager]].

Rejected: `didwebvh-rs`'s `Secret`, which holds raw private keys.

### D3. Custodians are the object's witnesses, at the custody quorum
An object DID's witness parameter lists every custodian's `did:key`, derived from its Ed25519 signing key, with threshold = quorum.

- motivated_by: [[decisions#Custody epochs are won by quorum promises]].

A log entry, and so the published DID document, then needs the same majority as a custody commit. A web host that serves an unwitnessed entry is caught by every resolver. A custodian-set change rewrites the witness list with the new custodians and quorum.

Rejected:

- **No witnesses:** the controller alone could publish a set the custodians never accepted.
- **Separate witness services:** a second trust set to operate and keep in step.

### D4. Witnessing is a custody request, judged by a policy the replica is given
The custody protocol gains `Witness { object, log, entry }` → `WitnessProof | Refused`. Each custodian answers it with an `EntryWitness` policy given to its `Replica`. The trait lives in `vertep-objects`; `vertep-webvh` implements it, so the crates don't depend on each other in a cycle. The policy checks three things:

- the entry is the valid next entry of the object's log (`didwebvh-rs` verification, in memory);
- the entry is signed by a current update key;
- its `VertepCustody` declaration equals the effective or pending configuration of the replica's accepted record.

Only then does it make the witness proof with the custodian's `did:key` key, through `KeyManager`.

- motivated_by: [[decisions#Custodian set changes use joint consensus]].

The request travels over the existing custody transports (in process or TSP), so witnessing needs no new network path. Rejected: a separate witness HTTP API.

### D5. A custody change carries its log entry; the writer publishes after the switch
`vt webvh custody change` builds the next log entry with the controller's update key. That entry declares the new `VertepCustody` set and the new witness list. The same command builds the `ReconfigurationRequest` with that entry in `did_log_entry`.

The writer runs the joint-consensus change, then calls the `DocumentPublisher`. `WebvhPublisher` implements it in four steps:

1. append the entry to the object's log;
2. collect witness proofs from the custodians until the old threshold is met;
3. verify the whole log;
4. publish through the configured log publisher.

The pending configuration is already accepted when the proofs are collected, so custodians can witness it.

- motivated_by: [[constraints#did:webvh updates need the web host]].

### D6. Publishers write one layout and verify first
`LogPublisher::publish(did, files)` has three implementations:

- `DirectoryPublisher` writes the D-layout below a root, `<domain>/<path or .well-known>/`.
- `GitPublisher` writes into a working copy, then runs `git add`, `git commit` and `git push` as processes.
- `CommandPublisher` writes into a directory, then runs a configured command with its arguments.

Each publish first verifies the full log with its witness file through the resolver (D2 of the spec, *Verify Before Publish*). `did.json` is the `didwebvh-rs` export of the current document to `did:web`.

Rejected: provider SDKs for S3 or Cloudflare. Heavy dependencies, and every provider already has a CLI the command publisher can run.

### D7. `vt webvh serve` is an axum file server with webvh content types
It runs behind feature `server` of `vertep-webvh`:

- `text/jsonl` for logs, and `application/json` for the witness file and `did.json`;
- `Access-Control-Allow-Origin: *`;
- `Cache-Control: max-age=min(ttl, 300)`;
- HEAD and GET only;
- paths canonicalised under the root, so `..` cannot escape it.

TLS is the proxy's job. axum is already in the workspace for the mediator.

### D8. `vt-mediator` creates and serves its own `did:webvh`
`did = "webvh:<host>[:<path>]"` makes the mediator do three things:

- on first start, create the DID from its identity, with its `TSPTransport` at `public_url`;
- keep the log under `<data_dir>/webvh/`;
- serve it from its router at the path the DID determines, beside `/tsp`.

Its update key is a file key manager key in the data directory. Restarts load the log.

### D9. Tests route domains to local servers through the fetcher
`did:webvh` requires domain names over HTTPS, and `didwebvh-rs` rejects IP addresses. So tests resolve through an `HttpFetcher` that maps `https://<domain>/...` onto a directory or a local server. The production resolver path is unchanged.

## Constraints

- **`did:webvh` 1.0 witnesses are `did:key`.** Binds D3: a custodian witnesses as the `did:key` of its signing key, not as its own DID (must). Source: DIF did:webvh v1.0, witnesses.
- **`versionTime` strictly increases and is never in the future**, at second granularity. Binds D5 and the `update` commands: entries made within one second need a backdated or waited timestamp (must). Source: DIF did:webvh v1.0, log entry.
- **Domains, not IP addresses, over HTTPS.** Binds D7 and D9: local tests need a fetcher mapping (must). Source: DIF did:webvh v1.0, DID-to-HTTPS transformation; `didwebvh-rs` URL validation.
- **The library is async.** Binds D2 and D4: synchronous custody code drives it with `futures::executor::block_on` and never inside a tokio worker (should). Source: `didwebvh-rs` 0.8 API.

## Risks / Trade-offs

- [Witness collection needs a quorum of reachable custodians] → it fails with the number of missing proofs. The custody change itself has already taken effect, and only discovery waits.
- [The controller's update key is a single point] → pre-rotation is on by default for objects, so a stolen current key cannot rotate to an attacker's key. Deactivation needs a current key too.
- [`git push` and commands are external processes] → their exit status and stderr are reported. Nothing is retried silently.
- [Library churn] → `didwebvh-rs` stays pinned. The interop test against its own vectors catches format drift.

## Migration Plan

- **Additive.** Objects created before this change have no witnesses. `vt webvh object witnesses` adds them with one controller-signed update, and nothing else changes.
- **Mediators:** existing ones keep `peer` or `web:` DIDs. Switching to `webvh:` gives the mediator a new DID, which its users must re-register with.

## Open Questions

- Whether to offer a `did:webvh` watcher integration once the DIF watcher API settles. It changes nothing here.
