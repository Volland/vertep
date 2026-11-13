## Why

Durable objects (M7) and the TSP mediator are addressed by `did:webvh` DIDs, but Vertep can only *resolve* `did:webvh`. Creating, updating, witnessing or hosting a DID log is left to outside tools. Custody's `DocumentPublisher` hook has only a recording stub, and `vt-mediator` cannot be a `did:webvh` at all.

So today an operator has to:

- hand-craft a DID log with another tool;
- sign it with keys outside Vertep's key manager;
- copy files to a web server by hand.

Nothing ties a custodian-set change to the DID document that announces it. `did:webvh` 1.0 is built for exactly this: a hash-chained, signed log on any web host, with key pre-rotation and witnesses. The DIF `didwebvh-rs` library Vertep already pins implements the log. This change connects it to Vertep's keys, custody and tooling.

## What Changes

- **New crate `vertep-webvh`:**
  - Creates `did:webvh` DIDs from a Vertep identity and its services (TSP, DIDComm, `VertepCustody`).
  - Updates documents, rotates update keys with pre-rotation, and deactivates DIDs.
  - Signs every log entry through `KeyManager`, so no private key leaves it.
  - Verifies every log with the resolver before publishing.
- **Custodians witness their object's DID log.**
  - The witness set is the custodians' `did:key` identities, with the custody quorum as the threshold.
  - A custodian witnesses an entry only if it is a valid next entry signed by the controller's update key. Its `VertepCustody` declaration must match a configuration record the custodian has accepted.
  - A DID document can therefore never announce a custodian set the custodians did not accept.
- **Custody changes publish themselves.** After the joint-consensus switch, the writer collects witness proofs and publishes the controller-signed log entry through a log publisher. This fills custody's `DocumentPublisher` hook.
- **Publishers:**
  - **Directory:** writes the `did:webvh` 1.0 layout: `did.jsonl`, `did-witness.json`, and a `did.json` for `did:web` compatibility.
  - **Git:** commits and pushes, for example to GitHub Pages.
  - **Command:** runs a configured hook such as `aws s3 sync` or `wrangler pages deploy`.
- **Serving:** `vt webvh serve` serves a directory of logs with the right content types, CORS and caching for a reverse proxy or a small host.
- **`vt webvh` commands:** `create`, `update`, `rotate`, `deactivate`, `verify`, `witness`, `publish`, `serve` and `object create`. `object create` makes a durable object's DID with its custodians and witnesses.
- **`vt-mediator` gets `did = "webvh:<host>[:<path>]"`.** It creates its log on first start, keeps it in its data directory, serves it at the matching path, and keeps its DID across restarts.

## Capabilities

### New Capabilities

- `webvh-identity`:
  - creating `did:webvh` DIDs with Vertep keys and identity documents;
  - updates, key rotation with pre-rotation, and deactivation;
  - custodians as witnesses of their object's log, and the rules a custodian follows before witnessing;
  - custody changes published as witnessed log entries;
  - verification before publishing.
- `webvh-hosting`:
  - the published file layout;
  - the directory, git and command publishers;
  - serving logs over HTTP;
  - the `vt webvh` commands;
  - `did:webvh` for `vt-mediator`;
  - interoperability with logs from other `did:webvh` implementations.

### Modified Capabilities

<!-- None. `did-identity` already requires resolving did:webvh; its requirements stay as they are. The M7 capabilities (object-custody, tsp-mediator) are not archived yet, so their extensions live in the two new capabilities. -->

## Impact

- **Code:**
  - New crate `vertep-webvh`: authoring, witnesses, publishers, and the server behind feature `server`.
  - `vertep-cli`: `vt webvh`.
  - `vertep-objects`: a webvh `DocumentPublisher`, and a custody `witness` request answered by replicas.
  - `vertep-tsp-mediator`: `webvh:` DIDs.
  - No change to `vertep-core` or the runtime.
- **Dependencies:**
  - `didwebvh-rs` 0.8.2, already pinned in `vertep-federation` and now also used for authoring with `default-features = false`;
  - `affinidi-data-integrity` for its `Signer` trait;
  - axum, already used by the mediator, for the server.
- **tg sections touched:**
  - [[constraints#did:webvh updates need the web host]];
  - [[decisions#Custodian set changes use joint consensus]];
  - [[decisions#Pinned relay protocol libraries]];
  - [[decisions#Keys stay behind a key manager]];
  - [[components#vertep-objects]], [[components#vertep-tsp-mediator]], [[components#vertep-federation]], [[components#vertep-cli]];
  - [[views/durable-objects#Durable Objects#Custodian set changes]];
  - a new roadmap milestone `M8 did:webvh hosting`.
- **Binding decisions and constraints:**
  - [[decisions#Keys stay behind a key manager]]: entries and witness proofs are signed through `KeyManager`.
  - [[decisions#Custodian set changes use joint consensus]]: a custody change is published only after the switch.
  - [[constraints#did:webvh updates need the web host]]: the change is visible only once a host serves it.
  - [[decisions#Method resolvers behind one trait]]: verification before publishing uses the same resolver path as resolution.
- **Requirements added:** every requirement of `webvh-identity` and `webvh-hosting`.
- **Not in scope:**
  - migrating a DID to another domain (`didwebvh-rs` supports it; it can follow later);
  - watchers;
  - a hosted service.
