<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-federation</h1>

<p align="center">Vertep actors across trust boundaries: DID identities, grants and relay mailboxes over TSP and DIDComm v2</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-federation"><img src="https://img.shields.io/crates/v/vertep-federation.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-federation"><img src="https://img.shields.io/docsrs/vertep-federation" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-federation.svg" alt="license"></a>
</p>

This crate connects actors of different owners through relays that nobody has to trust. An
actor that crosses a trust boundary can use a DID as its address. A DID resolver acts as the
directory: the DID document's services become routes, and its verification methods become
keys. Messages are encrypted end to end and carried by DIDComm v2 mediators or TSP endpoints,
which also serve as mailboxes, so an actor can receive while it is deactivated or offline.

Before any journal append, an inbound message is decrypted, its sender is verified against its
DID document, a grant for that sender, target and event is looked up, and duplicates are
dropped. An ungranted message never costs the receiver a journal append. Grants are checked on
the sending side too. Private keys stay behind the `KeyManager` trait and never enter journals,
snapshots, graphs or datamodels. Actors on ordinary IRIs never touch any of this.

The crate plugs into `vertep-runtime` as a mailbox factory for `did:` addresses and as SCXML
Event I/O Processors (`<send type="didcomm">`, `<send type="tsp">`). The `vertep` facade's
`federation` feature turns it on with `didcomm` and `tsp` and re-exports it as
`vertep::federation`. Depend on it directly for `did:webvh` (feature `webvh`), which the facade
does not enable, or to pick protocols yourself.

## Install

```toml
[dependencies]
vertep-federation = { version = "0.1", features = ["didcomm", "tsp"] }
```

| Feature | Turns on |
| --- | --- |
| `didcomm` | `didcomm` and `pickup`: DIDComm v2 authcrypt, anoncrypt and forward wrapping (Affinidi crates), Mediator Coordination 2.0 and Message Pickup 3.0 |
| `tsp` | `tsp`: TSP direct and routed mode and relationship setup (OpenWallet Foundation `tsp_sdk`) |
| `webvh` | `resolver::webvh::WebVhResolver`: the `did:webvh` method with log verification |
| `testkit` | `testkit` (with `didcomm` or `tsp`): an in-process DIDComm mediator and TSP intermediary for tests and examples |

With no features you get identities, `did:key`, `did:peer` and `did:web` resolution, key
management, grants, grant credentials, the relay envelope and the identity ontology, but no
relay protocol.

## Example

This example runs with the default features: no network, no relay.

```rust
use std::time::Duration;

use vertep_federation::resolver::key::did_key;
use vertep_federation::{
    Did, DidResolver, Grant, GrantRefusal, GrantSet, InMemoryKeyManager, KeyManager, KeyType,
    ResolverRegistry, Timestamp,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Private keys stay in the key manager; callers see key names and public keys.
    let keys = InMemoryKeyManager::new();
    let buyer_key = keys.generate(KeyType::Ed25519)?;
    let shop_key = keys.generate(KeyType::Ed25519)?;
    let buyer = did_key(&keys.public_key(&buyer_key)?);
    let shop = did_key(&keys.public_key(&shop_key)?);

    // did:key resolves offline. The registry also has did:peer and did:web (with a fetcher).
    let resolver = ResolverRegistry::with_builtin(None, Duration::from_secs(5));
    let document = resolver.resolve(&Did::parse(&buyer)?)?.document;
    let method = document.authentication_methods()[0].id.clone();
    assert!(method.starts_with(&buyer));

    // A signature made through the key manager verifies with the public key.
    let signature = keys.sign(&buyer_key, b"order 42")?;
    assert!(keys.public_key(&buyer_key)?.verify(b"order 42", &signature));

    // Grants: the buyer may send `order.*` events to the shop, nothing else.
    let mut grants = GrantSet::new();
    grants.add(Grant::new(&buyer, &shop, ["order.*"]));
    let now = Timestamp::now();
    assert!(grants.check_inbound(Some(&buyer), &shop, "order.placed", now).is_ok());
    assert!(matches!(
        grants.check_inbound(Some(&buyer), &shop, "admin.reset", now),
        Err(GrantRefusal::EventNotGranted { .. })
    ));
    assert!(matches!(
        grants.check_inbound(None, &shop, "order.placed", now),
        Err(GrantRefusal::Unauthenticated)
    ));
    Ok(())
}
```

Two `did:key` identities are made from keys held by an `InMemoryKeyManager`. The buyer's DID
resolves offline to a document whose verification method is its key. A grant lets the buyer
send `order.*` events to the shop: an `order.placed` passes, an `admin.reset` is refused, and a
message with no verified sender is refused before any grant is looked up. In a running node
`Federation` performs these checks on every inbound and outbound message.

The full flow, two organisations' nodes exchanging an event through an untrusted in-process
DIDComm mediator, is the crate's `two_orgs` example. It needs the `didcomm` and `testkit`
features:

```sh
cargo run -p vertep-federation --example two_orgs --features didcomm,testkit
```

```text
shop journaled order.placed from did:peer:2.Vz6Mk... via https://w3id.org/vertep/vt-id#DIDCommEventProcessor (send id Some("o-1"))
mediator still holds 0 message(s)
```

The DID differs on every run because the example generates fresh keys. The mediator holds
nothing at the end because the shop's node acknowledges a message only after its turn is
durable.

## Key types

| Item | What it is |
| --- | --- |
| `Federation`, `FederationBuilder` | wires a runtime node: `mailboxes()` for `did:` addresses, `transport(kind)`, `attach`, `provision` (keys and a DID for an actor), `grant_inbound`, `grant_outbound`, `accept_credential`, `pump` |
| `Did`, `DidDocument`, `Service`, `ServiceEndpoint`, `VerificationMethod` | DIDs and DID documents |
| `DidResolver`, `ResolverRegistry`, `CachingResolver`, `resolver::{key, peer, web, webvh}` | DID resolution by method, extensible and with a bounded cache |
| `DidDirectory`, `Route`, `TransportKind` | a DID address resolved to routes and keys |
| `KeyManager`, `InMemoryKeyManager`, `FileKeyManager`, `PublicKey`, `KeyType` | the key seam: generate, sign, key agreement, decrypt; private keys never leave it |
| `ActorIdentity`, `IdentityConfig`, `DidMethodChoice` | per-actor DIDs, provisioning per actor type, rotation with a grace period |
| `Grant`, `GrantSet`, `GrantRefusal` | DID-scoped grants by SCXML event descriptor, with optional expiry |
| `issue_grant_credential`, `verify_grant_credential` | grants as verifiable credentials (`eddsa-jcs-2022` proofs) |
| `admit`, `InboundPolicy`, `AnonymousPolicy`, `VerifiedOrigin` | the inbound admission pipeline run before a journal append |
| `RelayEnvelope`, `RelayProtocol`, `RelayOutbox`, `RelayInbox`, `RetryPolicy` | the protocol-neutral envelope, the protocol seam, outbound transport choice with fallback and retries, inbound relay mailboxes |
| `ontology` | the `vt-id:` identity ontology module and its SHACL shapes |

## Known gaps

- Live delivery over WebSocket from a remote mediator is not bound. Pickup from a remote
  mediator works in batches; `Federation::go_live` opens live sessions only on the in-process
  relays.
- Interop with a third-party mediator (Hyperledger Identus) is tested over HTTP in Docker only
  when `VERTEP_MEDIATOR_INTEROP=1` is set.
- The `vertep` facade's `federation` feature does not enable `did:webvh`.
- No Lean proofs cover federation; it is checked by interop and integration tests.

## Documentation

- Website: https://volland.github.io/vertep/ ([Cluster and federation](https://volland.github.io/vertep/docs/distribution.html#federation))
- API reference: https://docs.rs/vertep-federation
- Examples: https://volland.github.io/vertep/examples/

## License

MIT OR Apache-2.0, at your option.
