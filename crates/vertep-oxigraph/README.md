<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-oxigraph</h1>

<p align="center">Graph stores for Vertep on embedded Oxigraph and on any SPARQL 1.1 endpoint</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-oxigraph"><img src="https://img.shields.io/crates/v/vertep-oxigraph.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-oxigraph"><img src="https://img.shields.io/docsrs/vertep-oxigraph" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-oxigraph.svg" alt="license"></a>
</p>

This crate implements the `GraphStore` trait of `vertep-store` twice: `OxigraphStore` runs an
embedded [Oxigraph](https://crates.io/crates/oxigraph) store, and `SparqlEndpoint` talks the
SPARQL 1.1 Protocol to a remote endpoint (writes by SPARQL Update, reads by SPARQL Query).
Neither implements `JournalStore`. SPARQL Update has no compare-and-set, so a SPARQL store
cannot hold a journal safely.

Use it to keep a queryable copy of your actors' journals and current state. Journals stay in a
journal store such as `vertep-tiramemsu`, and `vertep_store::Projector` copies them into an
Oxigraph store or a SPARQL endpoint in order and idempotently. The copy may lag the journal;
consistent reads go to the journal store. `vertep-runtime`'s `SplitStorage` can also pair a
journal store with one of these graph stores. The `vertep` facade does not include this crate,
so add it yourself.

## Install

```toml
[dependencies]
vertep-oxigraph = "0.1"
vertep-store = "0.1"   # GraphStore, Projector
```

The crate has no cargo features. It builds Oxigraph with RDF 1.2 support and without its
`rocksdb` feature, so `OxigraphStore::new()` is in memory. For an on-disk store, enable
Oxigraph's `rocksdb` feature in your own manifest, open the store, and pass it to
`OxigraphStore::from_store`.

## Example

```rust
use vertep_oxigraph::OxigraphStore;
use vertep_store::oxrdf::{Literal, NamedNode, Triple};
use vertep_store::{AppendMeta, Entry, GraphStore, JournalStore, MemoryStore, Position, Projector};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The journal: any JournalStore. In production this is usually a tiramemsu backend.
    let journal = MemoryStore::new();
    let actor = NamedNode::new("https://example.org/charts/order/42")?;
    let event_name = NamedNode::new("https://w3id.org/vertep/vt#eventName")?;
    for (n, event) in [(1, "pay"), (2, "ship")] {
        let name = NamedNode::new(format!("{}/entry/{n}", actor.as_str()))?;
        let entry = Entry::new(name.clone(), [Triple::new(name, event_name.clone(), Literal::from(event))]);
        journal.append(&actor, Position::new(n - 1), &[entry], &AppendMeta::none())?;
    }

    // Project the journal into an embedded Oxigraph store.
    let target = OxigraphStore::new()?;
    let projector = Projector::new(&journal, &target);
    let report = projector.project_actor(&actor)?;
    assert_eq!(report.entries, 2);

    // Running again writes nothing: the target records how far it got.
    assert_eq!(projector.project_actor(&actor)?.entries, 0);
    assert_eq!(projector.projected_position(&actor)?, Position::new(2));

    // Query the projection.
    let events = target
        .query("SELECT ?e WHERE { GRAPH ?g { ?g <https://w3id.org/vertep/vt#eventName> ?e } } ORDER BY ?e")?
        .into_solutions()?;
    assert_eq!(events.len(), 2);
    Ok(())
}
```

Two journal entries are projected into Oxigraph. The projector stores
`<actor> vt:projectedPosition 2` in the target, so a second run (or a run after a crash) starts
where the last one stopped. With a tiramemsu shard as the journal, add
`.with_state_source(&shard)` to copy each actor's current-state graph as well.

A remote endpoint is configured the same way and used through the same trait:

```rust,no_run
use vertep_oxigraph::SparqlEndpoint;
use vertep_store::{GraphStore, RdfCapability};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = SparqlEndpoint::new("http://localhost:7878/query")
        .with_update_url("http://localhost:7878/update")
        .with_rdf(RdfCapability::Rdf11); // annotations fall back to RDF 1.1 reification
    let any = endpoint.query("ASK { ?s ?p ?o }")?.into_boolean()?;
    println!("endpoint has data: {any}");
    Ok(())
}
```

## Key types

| Item | What it is |
| --- | --- |
| `OxigraphStore` | embedded Oxigraph as a `GraphStore`: `new()` (in memory), `from_store(store)`, `store()` |
| `SparqlEndpoint` | a SPARQL 1.1 Protocol endpoint as a `GraphStore`: `new(query_url)`, `with_update_url`, `with_rdf`, `with_client` |
| `HttpClient`, `StdHttpClient`, `HttpRequest`, `HttpResponse` | the small HTTP seam `SparqlEndpoint` uses; `StdHttpClient` is a blocking plain-HTTP client on `std::net`; supply your own for HTTPS or authentication |
| `oxigraph` | the Oxigraph crate, re-exported |

`StdHttpClient` speaks plain HTTP only. For an `https://` endpoint, implement `HttpClient`
over the HTTP stack you already use and pass it with `with_client`.

## Documentation

- Website: https://volland.github.io/vertep/ ([RDF and versions, projection](https://volland.github.io/vertep/docs/rdf.html#journal), [The actor runtime, storage](https://volland.github.io/vertep/docs/runtime.html#storage))
- API reference: https://docs.rs/vertep-oxigraph
- Examples: https://volland.github.io/vertep/examples/

## License

MIT OR Apache-2.0, at your option.
