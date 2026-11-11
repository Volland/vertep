//! Quickstart: one virtual actor on tiramemsu, a durable delayed send, and a time-travel query.
//!
//! An order actor is addressed by its chart type and key. Paying it schedules a delayed `ship`
//! event, which is written to the journal as a durable timer and fires 200 ms later as an
//! ordinary journaled input. Afterwards we look back in time twice: by replaying the journal to
//! position 1, and by asking tiramemsu what the current-state graph said after each of its
//! transactions.
//!
//! Run with `cargo run -p vertep --example quickstart`.

use std::sync::Arc;
use std::time::Duration;

use vertep::prelude::*;

const ORDER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="200ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"#;

const ORDER_TYPE: &str = "https://example.org/charts/order";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("vertep-quickstart-{}", std::process::id()));
    let backend = Arc::new(TiramemsuBackend::open(&dir, 1)?);

    let mut node = Node::builder(backend.clone(), Shell::tokio()).build();
    let chart = vertep::scxml::read_str(ORDER)
        .map_err(|e| format!("{e:?}"))?
        .chart;
    let version = node.register_type(ORDER_TYPE, chart)?;
    println!("chart version {}", version.as_str());

    // A virtual actor exists for every key: no create call, the first message activates it.
    let order = Address::virtual_actor(ORDER_TYPE, "42")?;
    node.send(&order, Event::named("pay", EventType::External))?;

    // The node runs on one thread (datamodels need not be Send); the handle works from anywhere.
    let handle = node.handle();
    let local = tokio::task::LocalSet::new();
    let mut node = local
        .run_until(async move {
            let runner = tokio::task::spawn_local(async move {
                node.run().await;
                node
            });
            tokio::time::sleep(Duration::from_millis(400)).await;
            handle.shutdown();
            runner.await
        })
        .await?;

    let now = node.state(&order)?;
    println!(
        "now: {:?} after {} entries",
        now.configuration, now.position
    );
    for e in node.journal(&order)? {
        if let vertep::runtime::EntryBody::Input { input, effects } = &e.body {
            println!(
                "  entry {}: {} (timers scheduled: {})",
                e.position,
                input.event.name,
                effects.timers.len()
            );
        }
    }

    // Time travel by replay: the state after entry 1, rebuilt without touching the live actor.
    let then = node.state_at(&order, 1)?;
    println!("as of entry 1: {:?}", then.configuration);

    // Time travel in the store: tiramemsu never deletes, so each past transaction is queryable.
    let shard = backend.shard_for(order.node());
    let query = format!(
        "SELECT ?s WHERE {{ GRAPH {o} {{ {o} <{p}> ?s }} }}",
        o = order.node(),
        p = vertep::runtime::vocab::IN_STATE
    );
    for t in 1..=shard.last_tx()? {
        let states = shard.view_as_of(t).query(&query)?.into_solutions()?;
        let names: Vec<String> = states
            .values("s")
            .iter()
            .map(|s| {
                s.to_string()
                    .rsplit("#s.")
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('>')
                    .to_owned()
            })
            .collect();
        if !names.is_empty() {
            println!("  tiramemsu tx {t}: in {names:?}");
        }
    }

    drop(node);
    drop(backend);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(now.configuration, vec!["shipped".to_owned()]);
    assert_eq!(then.configuration, vec!["paid".to_owned()]);
    Ok(())
}
