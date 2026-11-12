<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-tiramemsu</h1>

<p align="center">The default Vertep storage backend: journals and current state in tiramemsu, a bitemporal triple store</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-tiramemsu"><img src="https://img.shields.io/crates/v/vertep-tiramemsu.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-tiramemsu"><img src="https://img.shields.io/docsrs/vertep-tiramemsu" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-tiramemsu.svg" alt="license"></a>
</p>

This crate implements the two storage traits of `vertep-store` (`JournalStore` and
`GraphStore`) on [tiramemsu](https://crates.io/crates/tiramemsu), a triple store that never
deletes rows. One `TiramemsuShard` is one file. An append is one transaction: it checks the
journal head, writes the entry, and applies the actor's current-state changes. Each state
statement is stored as a derived fact whose support is the entry that caused it, so you can
ask which entry put an actor in a state. Because nothing is deleted, every past transaction
can be read back with SPARQL, without replaying the journal.

A `TiramemsuBackend` is a directory of shard files, one writer each. Actors are placed in
shards by an FNV-1a hash of their address IRI. `vertep-runtime` implements its `Storage` trait
for both types behind its `tiramemsu` feature, and `vt cluster reshard` moves actors between
shard counts offline. The `vertep` facade turns this crate on with its default `tiramemsu`
feature and re-exports it as `vertep::tiramemsu`. Depend on it directly when you use
`vertep-runtime` without the facade, or when you read shard files yourself.

## Install

```toml
[dependencies]
vertep-tiramemsu = "0.1"
vertep-store = "0.1"   # the traits it implements
```

The crate has no cargo features. With `vertep-runtime`, turn on the runtime's `tiramemsu`
feature so a backend can be passed to `Node::builder`.

## Example

```rust
use vertep_store::oxrdf::{Literal, NamedNode, Triple};
use vertep_store::{AppendMeta, Entry, GraphStore, JournalStore, Position, StateDelta};
use vertep_tiramemsu::TiramemsuShard;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("vertep-tiramemsu-readme-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let shard = TiramemsuShard::open(dir.join("shard-0.db"))?;

    let actor = NamedNode::new("https://example.org/charts/order/42")?;
    let in_state = NamedNode::new("https://w3id.org/vertep/vt#inState")?;
    let event_name = NamedNode::new("https://w3id.org/vertep/vt#eventName")?;
    let state = |s: &str| -> Result<Triple, Box<dyn std::error::Error>> {
        Ok(Triple::new(actor.clone(), in_state.clone(), NamedNode::new(format!("urn:example:{s}"))?))
    };
    let entry = |n: u32, event: &str| -> Result<Entry, Box<dyn std::error::Error>> {
        let name = NamedNode::new(format!("{}/entry/{n}", actor.as_str()))?;
        Ok(Entry::new(name.clone(), [Triple::new(name, event_name.clone(), Literal::from(event))]))
    };

    // Entry 1 enters `open`; entry 2 leaves `open` and enters `paid`. Each append commits the
    // entry and the state change in one transaction.
    let delta1 = StateDelta { retract: vec![], assert: vec![state("open")?] };
    shard.append(&actor, Position::ZERO, &[entry(1, "start")?], &AppendMeta::none().with_state(delta1))?;
    let after_first = shard.last_tx()?;
    let delta2 = StateDelta { retract: vec![state("open")?], assert: vec![state("paid")?] };
    shard.append(&actor, Position::new(1), &[entry(2, "pay")?], &AppendMeta::none().with_state(delta2))?;

    // The current-state graph is named by the actor's address.
    let query = format!("SELECT ?s WHERE {{ GRAPH {actor} {{ {actor} {in_state} ?s }} }}");
    let now = shard.query(&query)?.into_solutions()?;
    assert_eq!(now.values("s"), vec![NamedNode::new("urn:example:paid")?.into()]);

    // Time travel in the store: the same query as of an earlier transaction.
    let then = shard.view_as_of(after_first).query(&query)?.into_solutions()?;
    assert_eq!(then.values("s"), vec![NamedNode::new("urn:example:open")?.into()]);

    // Which journal entry caused the current state?
    let causes = shard.state_causes(&actor, &state("paid")?)?;
    assert_eq!(causes[0].as_str(), "https://example.org/charts/order/42/entry/2");

    drop(shard);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
```

Two appends move an actor from `open` to `paid`. The current-state graph says `paid`, a view
as of the first transaction still says `open`, and `state_causes` follows the support link
from the `paid` statement back to entry 2. The runtime writes the same kind of entries and
state deltas for every actor turn.

## Key types

| Item | What it is |
| --- | --- |
| `TiramemsuShard` | one tiramemsu file; implements `JournalStore` and `GraphStore`. `open`, `open_with`, `from_db` (for a `Db` you opened, such as the WebAssembly host) |
| `TiramemsuShard::view_as_of(t)`, `view_as_of_time(ms)`, `view_now()` | read-only `TiramemsuView`s, themselves graph stores, as of a transaction or a wall-clock instant |
| `TiramemsuShard::state_causes`, `state_history` | the entries that caused a current-state statement; when a statement was added and retracted |
| `TiramemsuShard::last_tx`, `db` | the last transaction number; the underlying tiramemsu database |
| `TiramemsuBackend` | a directory of shards: `open(dir, shard_count)`, `shard_for(actor)`, `shards()`; implements `JournalStore` |
| `shard_of`, `shard_file_name` | the fixed placement function (FNV-1a of the address IRI) and the file name `shard-NNNN-of-NNNN.db` |
| `CAPABILITIES`, `graph_capabilities()` | fencing and state-in-append are supported; RDF 1.2 terms are supported |
| `tiramemsu` | the tiramemsu crate, re-exported |

A shard file may be open only once per process; a second `open` of the same path returns
`StoreError::Unavailable`. `TiramemsuBackend` implements only `JournalStore`: queries across
shards need an external graph store fed by `vertep_store::Projector` (see `vertep-oxigraph`).

## Documentation

- Website: https://volland.github.io/vertep/ ([The actor runtime, storage](https://volland.github.io/vertep/docs/runtime.html#storage), [time travel](https://volland.github.io/vertep/docs/runtime.html#time-travel), [RDF and versions](https://volland.github.io/vertep/docs/rdf.html#journal))
- API reference: https://docs.rs/vertep-tiramemsu
- Examples: https://volland.github.io/vertep/examples/ ([durable actor](https://volland.github.io/vertep/examples/durable-actor.html))

## License

MIT OR Apache-2.0, at your option.
