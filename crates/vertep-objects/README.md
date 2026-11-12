<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-objects</h1>

<p align="center">Durable objects for Vertep: DID-addressed actors with verifiable journals, kept by custodians in several organisations over TSP</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-objects"><img src="https://img.shields.io/crates/v/vertep-objects.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-objects"><img src="https://img.shields.io/docsrs/vertep-objects" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-objects.svg" alt="license"></a>
</p>

A durable object is a Vertep virtual actor whose journal belongs to no single machine or
organisation. Its address is a `did:webvh` DID. Its journal is kept by the custodians the DID
document names. **Hosts** may serve and write the object; **keepers** only store encrypted
entries. Exactly one host writes at a time, and no coordinator, lease authority or clock
decides who.

## How it works

- **Verifiable entries.** Every journal entry carries its object, position, custody epoch, writer,
  the previous entry's hash, its own content hash and the writer's signature (`chain`). A
  replica is checked from genesis without trusting whoever served it.
- **Custody epochs.** A host writes only after a majority of custodians promised it a new
  epoch. It adopts the chain accepted in the highest epoch, then the longest. This is Paxos
  over the whole log (`custody::CustodyHost`).
- **Commitment.** An entry is committed once a majority accepted it in one epoch. The runtime
  performs a turn's sends and timers only then.
- **Fencing.** A host that lost its epoch is refused on its next append.
- **Encrypted replicas.** Keepers see a clear header and an encrypted body. Content keys are
  wrapped to hosts; a new generation starts when a host leaves.
- **Handoff, failover and reconfiguration.** Objects move by handoff or by failover. A
  controller-signed reconfiguration changes the custodian set and publishes the `did:webvh`
  update.
- **Equivocation.** Two validly signed entries of one epoch at one position become evidence.
- **Transport.** Custody messages travel as TSP application messages, directly or through a
  store-and-forward mediator such as `vt-mediator` (`custody::tsp_link`, feature `tsp`).
- **Runtime integration.** `ObjectStorage` plugs custody into a `vertep-runtime` node. The
  runtime is unchanged: a durable object is an ordinary virtual actor whose journal is a
  custody journal. `serve_object` takes an object over on a host and re-arms its timers.
- **Agents.** `vertep-agent`'s `StoreAgentJournal` over the custody store makes an agent a
  durable object.

## Install

```toml
[dependencies]
vertep-objects = { version = "0.1", features = ["tsp"] }
```

| Feature | Turns on |
| --- | --- |
| `tsp` | `custody::tsp_link`: custody over TSP between host and custodian VIDs, direct or routed through a mediator |
| `testkit` | `testkit`: in-memory hosts and a custody fixture with switchable links |
| `differential` | for the repository's own tests: require the Lean custody model (`vertep-custody`) |

## Example

`cargo run -p vertep-objects --example three_orgs --features testkit,tsp` runs a support agent
kept by three organisations over TSP:

1. Org 1 serves the agent until it waits for a person's approval.
2. Org 1 disappears.
3. Org 2 wins the next epoch from itself and org 3's keeper, which it reaches through the
   keeper's mediator.
4. Org 2 replays the agent without calling the model again, and the person approves the
   decision org 1 had opened.

```text
org 1: agent in ["review"], custody epoch Some(1), 3 entries committed, model calls 1
       keeper K holds 3 encrypted entries
org 2: won custody epoch Some(2), replayed ["review"], model calls 0
       open approval did:webvh:QmScid:support.example:agents:ticket-8812/decision/3.0 (same as before the failover: true)
org 2: agent in ["approved"] after 4 journaled inputs
```

The core of a host's wiring:

```rust,ignore
let host = Arc::new(
    CustodyHost::new(object, signer, keys, agreement_key, own_replica, directory)
        .with_transport(Arc::new(TspCustodyTransport::direct(&org2, session.clone(), net.clone(), resolve.clone())))
        .with_transport(Arc::new(TspCustodyTransport::routed(&keeper, session, net, resolve, vec![mediator], inbox, timeout))),
);
let storage = Arc::new(ObjectStorage::new(inner_storage));
storage.custody().add(host);
let mut node = Node::builder(storage, shell).build();
node.handle().bind_address(&object_address, "https://example.org/charts/ticket", "8812");
serve_object(&mut node, &object_address)?;
```

## Testing

- `tests/custody.rs`: every custody scenario on in-process custodians.
- `tests/custody_tsp.rs`: custody over real TSP, directly and through a store-and-forward
  intermediary.
- `tests/runtime.rs`: durable objects in the actor runtime.
- `tests/faults.rs`: random partitions, colliding elections, crashes and late duplicate
  batches.
  - It checks one writer per epoch and that every committed log survives.
  - It writes each history for the Lean custody model, which replays them in strict mode
    when built.
  - `VERTEP_CUSTODY_SEEDS` sets the number of seeds.

The design is in `tg.md/views/durable-objects.md` and the requirements are in the OpenSpec
change `m7-durable-objects`.

## License

MIT OR Apache-2.0.
