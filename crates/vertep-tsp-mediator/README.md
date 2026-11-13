# vertep-tsp-mediator

A mailbox mediator for the [ToIP Trust Spanning Protocol](https://trustoverip.github.io/tswg-tsp-specification/) (TSP 1.0).

TSP routes messages through intermediaries, but it defines no store-and-forward and "provides no delivery guarantee". This mediator adds a mailbox. While a VID is offline, its messages wait at the mediator, still sealed to the recipient. The mediator never sees an event:

- **Routing.** It is a TSP 1.0 routed-mode intermediary. It opens only its own layer and forwards middle hops. At the last hop, it pushes the nested message on a live session, delivers it to the recipient's own endpoint, or stores it.
- **Mailboxes.** Each recipient VID has a durable, event-sourced journal in tiramemsu shard files, one writer per file. The mediator accepts a message from the previous hop only after appending it. It deletes a message only when the recipient acknowledges it. Quotas and retention apply.
- **Pickup.** A versioned mailbox protocol travels inside TSP over the relationship between the VID and the mediator: `register`, `status`, `deliver`, `received`, `unregister`. A WebSocket live session pushes messages as they arrive.

The client side is `vertep_federation::tsp_mailbox::TspMailboxClient`, a pickup source for the relay inbox.

## Run

```sh
cargo run -p vertep-tsp-mediator --bin vt-mediator -- serve --config mediator.toml
```

```toml
listen = "0.0.0.0:8700"
public_url = "https://mediator.example/tsp"   # published in the DID document
data_dir = "/var/lib/vt-mediator"
did = "web:mediator.example"                  # or "peer", or "webvh:mediator.example[:path]"
shards = 4

[limits]
max_messages = 10000
max_bytes = 67108864
retention_secs = 1209600
```

The mediator serves these routes:

- `POST /tsp` for TSP messages;
- `GET /tsp/live` for the WebSocket live session;
- `GET /health` and `GET /metrics` (Prometheus);
- `GET /.well-known/did.json`.

`vt-mediator compact` rewrites the store with live messages only, so acknowledged ciphertext leaves the journal history. `docker/` has an image and a compose file that run two mediators in a route.

## Use as a library

```rust,no_run
use std::sync::Arc;
use vertep_tsp_mediator::{MailboxStore, Mediator, MediatorOptions};
# fn demo(
#     identity: vertep_federation::ActorIdentity,
#     km: Arc<dyn vertep_federation::KeyManager>,
#     resolver: Arc<dyn vertep_federation::DidResolver>,
#     net: Arc<dyn vertep_federation::RelayNetwork>,
# ) -> Result<(), Box<dyn std::error::Error>> {
let store = MailboxStore::open(Arc::new(vertep_store::MemoryStore::new()))?;
let mediator = Mediator::new(
    identity, km, resolver, net, store,
    Arc::new(vertep_federation::SystemClock),
    MediatorOptions::default(),
)?;
// Serve it on any RelayEndpoint / LiveEndpoint transport, or with `server::router`.
# let _ = mediator; Ok(()) }
```
