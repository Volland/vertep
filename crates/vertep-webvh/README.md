<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-webvh</h1>

<p align="center">did:webvh for Vertep: create, update, witness, publish and serve DID logs with key-manager keys</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-webvh"><img src="https://img.shields.io/crates/v/vertep-webvh.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-webvh"><img src="https://img.shields.io/docsrs/vertep-webvh" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-webvh.svg" alt="license"></a>
</p>

`did:webvh` is `did:web` with a verifiable history: a hash-chained, signed DID log on any web
host, with key pre-rotation and witnesses. `vertep-federation` resolves it. This crate writes and
hosts it, with keys that never leave Vertep's `KeyManager`. Log mechanics (SCIDs, entry hashes,
Data Integrity proofs, pre-rotation, witness proofs) are the DIF
[`didwebvh-rs`](https://crates.io/crates/didwebvh-rs).

## What it does

- **Authoring.** `Author` creates, updates, rotates and deactivates a DID and knows its update
  keys only by name. `KmSigner` signs through the key manager. With pre-rotation, every entry
  reveals the key the previous one announced by hash.
- **Documents.** `document` builds DID documents from Vertep identities: verification and
  key-agreement methods, `TSPTransport`, `DIDCommMessaging` and `VertepCustody` services.
- **Verification.** `DidLog::verify` checks the whole log with its witness proofs. Errors name
  the first failing entry, or say how many witness proofs are missing.
- **Durable objects** (feature `custody`, default):
  - an object's custodians are its witnesses, at the custody quorum (`custody_witnessing`);
  - a custodian witnesses an entry only if it is the valid next entry, signed by a current
    update key, and declares a custodian set the custodian accepted (`CustodianWitness`, given
    to each replica);
  - custody changes carry their log entry (`custody_change`), and the writer publishes it after
    the switch (`WebvhPublisher`).
- **Publishing.** All three publishers write the `did:webvh` 1.0 layout (`did.jsonl`,
  `did-witness.json`, `did.json`) and verify first:
  - `DirectoryPublisher`;
  - `GitPublisher` (commit and push, for GitHub or GitLab Pages);
  - `CommandPublisher` (then run `aws s3 sync`, `wrangler pages deploy` or anything else).
- **Serving** (feature `server`). `server` is the log server behind `vt webvh serve`: the right
  content types, CORS, a cache lifetime capped by the DID's `ttl`, GET and HEAD only, and no
  path outside its root.

## Install

```toml
[dependencies]
vertep-webvh = "0.1"
```

| Feature | Turns on |
| --- | --- |
| `custody` (default) | custodians as witnesses, `WebvhPublisher` and `custody_change` (adds `vertep-objects`) |
| `server` | the axum log server (adds axum and tokio) |

## Example

Create a DID with pre-rotation and publish it to a directory:

```rust
use std::sync::Arc;
use vertep_federation::{InMemoryKeyManager, KeyManager};
use vertep_webvh::{Author, CreateOptions, DirectoryPublisher, LogPublisher, document};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let keys: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
let options = CreateOptions::new("example.org")
    .path("objects:ticket-8812")
    .pre_rotation(true);

// An identity (signing and key-agreement keys) under the DID-to-be.
let identity = document::provision(keys.as_ref(), &options, vec![])?;
let doc = document::identity_document(&identity, keys.as_ref())?;

let mut author = Author::generate(keys.clone(), true)?;
let log = author.create(&options, &doc)?;
assert!(log.did().ends_with(":example.org:objects:ticket-8812"));

let site = std::env::temp_dir().join(format!("vertep-webvh-readme-{}", std::process::id()));
let published = DirectoryPublisher::new(&site).publish(&log)?;
assert!(published.files[0].ends_with("example.org/objects/ticket-8812/did.jsonl"));
# std::fs::remove_dir_all(&site)?;
# Ok(())
# }
```

The same with the command line, then serving the site:

```text
vt webvh create --domain example.org --keys ./keys --out ./site --pre-rotation \
    --service TSPTransport=https://example.org/tsp
vt webvh verify ./site/example.org/.well-known/did.jsonl
vt webvh serve --root ./site --addr 127.0.0.1:8080
```

A durable object whose custodians witness its log:

```text
vt webvh object create --domain example.org --path objects:ticket-8812 --keys ./keys \
    --custodian host,did:web:a.example,z6Mk... --custodian host,did:web:b.example,z6Mk... \
    --custodian keeper,did:web:k.example,z6Mk... --quorum 2
vt webvh witness ./keys/dids/<did>/did.jsonl --keys ./custodian-keys --key <key name>   # by each custodian
vt webvh publish <did> --keys ./keys --git ./pages --subdir docs --branch main
```

## Testing

- `tests/identity.rs`: create, update, rotation with pre-rotation, deactivation, and keys that
  stay in the key manager.
- `tests/witness.rs`: custodian witnessing, and a keeper replaced end to end, in process and
  over TSP.
- `tests/hosting.rs`: the layout, git publishing into a bare remote, command failures, and the
  log server over a socket.
- `tests/interop.rs`: the DIF test-suite vectors (`tests/vectors`), and Vertep logs resolved
  by `didwebvh-rs` alone.

The design is in `tg.md/views/durable-objects.md`. The requirements are in the OpenSpec change
`m8-webvh`.

## License

MIT OR Apache-2.0.
