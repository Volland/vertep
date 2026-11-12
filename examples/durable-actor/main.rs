//! Durable actor: an order actor on a tiramemsu journal that outlives its process.
//!
//! "Process" 1 opens a store, registers the chart and sends three events to a virtual actor.
//! Each accepted event becomes one journal entry; the checkout entry also records a durable
//! timer (`payment.timeout` in 900 s). Then the node and the store are dropped, as if the
//! process stopped. "Process" 2 opens the same directory ten minutes later: asking for the
//! actor's state replays the journal, and when the clock reaches the timer's due time the timer
//! fires as an ordinary journaled input, although the node that scheduled it is gone.
//!
//! Both nodes run on a virtual clock (`Shell::virtual_time`) and are driven with
//! `run_until_idle`, so the output is the same on every run. A server would use
//! `Shell::tokio()` and `node.run().await` instead (see `crates/vertep/examples/quickstart.rs`).
//!
//! Illustrates the `actor-model` (Virtual Actor Identity, Virtual Actor Activation),
//! `event-journal` (Journal Of Accepted Inputs, Deterministic Replay) and `durable-timers`
//! (Delayed Send Is Journaled, Timer Fires As Input) capabilities.
//!
//! Run with `cargo run -p vertep-examples --bin durable-actor`.

use std::sync::Arc;
use std::time::Duration;

use vertep::prelude::*;
use vertep::runtime::EntryBody;

/// The chart, next to this file.
const ORDER: &str = include_str!("order.scxml");
/// The chart type: every actor of this type is addressed by this IRI plus a key.
const ORDER_TYPE: &str = "https://example.org/charts/order";
/// 2026-11-11T00:00:00Z, the virtual clock's start.
const T0: i64 = 1_794_355_200_000;

type Error = Box<dyn std::error::Error>;

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("vertep-durable-actor-{}", std::process::id()));
    let result = run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run(dir: &std::path::Path) -> Result<(), Error> {
    let (shell, clock) = Shell::virtual_time(T0);
    let order = Address::virtual_actor(ORDER_TYPE, "42")?;
    println!("actor {}", order.as_str());

    // ---- process 1 -------------------------------------------------------------------------
    {
        let backend = Arc::new(TiramemsuBackend::open(dir, 1)?);
        let mut node = node(backend, shell.clone())?;

        // A virtual actor needs no create call: the first message activates it.
        for name in ["add", "add", "checkout"] {
            node.send(&order, Event::named(name, EventType::External))?;
        }
        node.run_until_idle();

        let state = node.state(&order)?;
        println!(
            "process 1: {:?} context={}",
            state.configuration,
            context(&state)
        );
        print_journal(&node, &order)?;
        for (due, timer) in node.pending_timers() {
            println!(
                "  pending timer {} due at +{}s",
                timer.event.name,
                (due - T0) / 1000
            );
        }
        println!("process 1 stops");
    } // the node and the store are dropped here

    // ---- process 2, ten minutes later -------------------------------------------------------
    clock.advance(Duration::from_secs(600));
    let backend = Arc::new(TiramemsuBackend::open(dir, 1)?);
    let mut node = node(backend, shell)?;

    // Reading the state activates the actor: its journal is replayed through the same chart.
    let state = node.state(&order)?;
    println!(
        "process 2: {:?} context={} (replayed {} entries)",
        state.configuration,
        context(&state),
        node.stats().replayed
    );

    // Five more minutes: the timer process 1 scheduled is due and fires here.
    clock.advance(Duration::from_secs(300));
    node.run_until_idle();
    let state = node.state(&order)?;
    println!(
        "process 2 at +900s: {:?} context={}",
        state.configuration,
        context(&state)
    );
    print_journal(&node, &order)?;

    // Time travel: the state after any past entry, rebuilt from the journal.
    let then = node.state_at(&order, 1)?;
    println!(
        "as of entry 1: {:?} context={}",
        then.configuration,
        context(&then)
    );
    Ok(())
}

/// A node on `backend` that runs CEL charts, with the order chart registered.
fn node(backend: Arc<TiramemsuBackend>, shell: Shell) -> Result<Node, Error> {
    let mut node = Node::builder(backend, shell)
        .datamodels(Arc::new(vertep_dm_cel::CelRegistry))
        .build();
    // `read_str` accepts only the `null` datamodel; name the ones the host supports.
    let opts = vertep::scxml::ReadOptions::new().with_datamodels(&vertep::scxml::AnyDatamodel);
    let chart = vertep::scxml::read(ORDER, &opts)?.chart;
    // Registering is idempotent: the same chart gets the same content-addressed version.
    node.register_type(ORDER_TYPE, chart)?;
    Ok(node)
}

fn print_journal(node: &Node, order: &Address) -> Result<(), Error> {
    for entry in node.journal(order)? {
        if let EntryBody::Input { input, effects } = &entry.body {
            let timers: Vec<&str> = effects
                .timers
                .iter()
                .map(|t| t.event.name.as_str())
                .collect();
            println!(
                "  entry {} at +{}s: {:<16} timers scheduled: {:?}",
                entry.position,
                (entry.timestamp_ms - T0) / 1000,
                input.event.name,
                timers
            );
        }
    }
    Ok(())
}

/// The context of an actor state as JSON. `ActorState::datamodel` is the datamodel's own
/// snapshot encoding (for CEL, typed values like `{"$int": "2"}` plus system variables);
/// `ContextView` decodes the data items into plain values.
fn context(state: &ActorState) -> String {
    let view = vertep::agent::ContextView::new(state.datamodel.clone());
    vertep::agent::json::to_json(view.data()).to_string()
}
