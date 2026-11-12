<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-store</h1>

<p align="center">Storage contracts for Vertep: a compare-and-set journal store and a SPARQL graph store</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-store"><img src="https://img.shields.io/crates/v/vertep-store.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-store"><img src="https://img.shields.io/docsrs/vertep-store" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-store.svg" alt="license"></a>
</p>

Vertep runs W3C SCXML statecharts as durable actors. Each actor turn is one journal entry, and
the journal is the only source of truth. This crate defines where those entries go. It has two
traits, so that no backend is asked for a guarantee it cannot give:

- `JournalStore`: per actor address, a linearizable, all-or-nothing append at an expected
  position (compare-and-set), reads from a position, and an optional fencing token checked on
  append.
- `GraphStore`: RDF quads in, SPARQL 1.1 out, with a declared RDF 1.2 or RDF 1.1 capability.
  No compare-and-set is required.

Journal entries are RDF datasets with one named graph per entry. The crate also ships
`MemoryStore` (the in-memory reference implementation of both traits), the `Projector` that
copies journals into another graph store, and a shared test kit every backend must pass.
`vertep-tiramemsu` (the default backend) and `vertep-oxigraph` implement these traits;
`vertep-runtime` and `vertep-cluster` use them. The `vertep` facade always pulls this crate in
and re-exports it as `vertep::store`. Depend on it directly when you write a storage backend,
read journals outside the runtime, or want the in-memory store in tests.

## Install

```toml
[dependencies]
vertep-store = "0.1"

[dev-dependencies]
# The shared backend test kit: `journal_store_kit!`, `graph_store_kit!`, `combined_store_kit!`.
vertep-store = { version = "0.1", features = ["testkit"] }
```

## Example

```rust
use vertep_store::oxrdf::{Literal, NamedNode, Triple};
use vertep_store::{AppendMeta, Entry, GraphStore, JournalStore, MemoryStore, Position};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = MemoryStore::new();
    let actor = NamedNode::new("https://example.org/charts/order/42")?;
    let event_name = NamedNode::new("https://w3id.org/vertep/vt#eventName")?;

    // One entry is one named graph, named by an IRI the caller chooses.
    let entry = |n: u32, event: &str| -> Result<Entry, Box<dyn std::error::Error>> {
        let name = NamedNode::new(format!("{}/entry/{n}", actor.as_str()))?;
        Ok(Entry::new(
            name.clone(),
            [Triple::new(name, event_name.clone(), Literal::from(event))],
        ))
    };

    // Append at the expected head: 0 is the empty journal.
    let head = store.append(&actor, Position::ZERO, &[entry(1, "pay")?], &AppendMeta::none())?;
    assert_eq!(head, Position::new(1));

    // A second writer that still expects the empty journal gets a conflict and writes nothing.
    let err = store
        .append(&actor, Position::ZERO, &[entry(2, "cancel")?], &AppendMeta::none())
        .unwrap_err();
    assert!(err.is_conflict());

    let head = store.append(&actor, head, &[entry(2, "ship")?], &AppendMeta::none())?;
    assert_eq!(head, Position::new(2));

    // Entries come back in order, stamped with vt:actor and vt:position.
    let records = store.read(&actor, Position::ZERO)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].entry.position(), Some(Position::new(2)));

    // The graph side answers SPARQL over the entry graphs.
    let events = store
        .query(
            "SELECT ?e WHERE { GRAPH ?g { ?g <https://w3id.org/vertep/vt#eventName> ?e } } ORDER BY ?e",
        )?
        .into_solutions()?;
    assert_eq!(events.len(), 2);
    Ok(())
}
```

The example appends two entries to one actor's journal. The append in the middle expects the
journal to be empty, but it already holds one entry, so it fails with `AppendError::Conflict`.
This check is what stops two activations of one actor from both extending its history. The
last step reads the entries back with SPARQL, because `MemoryStore` is a graph store too.

## Key types

| Item | What it is |
| --- | --- |
| `JournalStore` | `append(actor, expected, entries, meta)`, `read`, `head`, `actors`, `capabilities` |
| `GraphStore` | `insert_quads`, `remove_quads`, `clear_graph`, `query` (SPARQL 1.1), `capabilities` |
| `Entry`, `JournalRecord`, `Position` | a journal entry (one named graph), an entry read back with its position, a position (0 is empty) |
| `AppendMeta`, `FencingToken`, `StateDelta` | optional append metadata: a fencing epoch per scope, and current-state changes committed with the entries |
| `JournalCapabilities`, `GraphCapabilities`, `RdfCapability` | what a store supports: fencing, state in the append, RDF 1.2 or RDF 1.1 |
| `AppendError`, `StoreError` | `Conflict`, `Fenced`, `Unsupported`, `Invalid`, `Storage`; and `Unavailable`, `Invalid`, `Unsupported`, `Backend` |
| `MemoryStore` | the in-memory reference implementation of both traits; `with_fencing`, `with_state_in_append`, `with_rdf` turn capabilities off |
| `Projector` | copies journal entries into a graph store in order, idempotently, and records `vt:projectedPosition` per actor |
| `reification`, `sparql`, `vocab` | the RDF 1.1 reification fallback for annotations, SPARQL text helpers, the `vt:` IRIs the stores write |
| `testkit` (feature) | `journal_store_kit!`, `graph_store_kit!`, `combined_store_kit!`: the contract tests every backend runs |
| `oxrdf` | the RDF model the traits use, re-exported |

## Documentation

- Website: https://volland.github.io/vertep/ (storage is described in [The actor runtime](https://volland.github.io/vertep/docs/runtime.html#storage) and [RDF and versions](https://volland.github.io/vertep/docs/rdf.html))
- API reference: https://docs.rs/vertep-store
- Examples: https://volland.github.io/vertep/examples/ ([durable actor](https://volland.github.io/vertep/examples/durable-actor.html))

## License

MIT OR Apache-2.0, at your option.
