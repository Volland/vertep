//! Operation budgets of [[openspec:runtime-performance#Operation Budgets]], counted through
//! `CountingStorage` so they hold on any machine.
//!
//! The budgets are what M18 promises, not what the runtime does today: each test stays
//! `#[ignore]` until the phase that makes it pass (phase 2 of `m18-runtime-performance`).
//! `cargo test -p vertep-runtime --test budgets -- --ignored --nocapture` prints today's counts.

mod common;

use std::sync::Arc;

use common::*;
use vertep_runtime::counting::CountingStorage;
use vertep_runtime::mailbox::{Mailbox, MemoryMailbox};
use vertep_runtime::{
    Address, Envelope, Node, NodeConfig, Resolver, Shell, StoreMailboxes, VirtualClock,
};
use vertep_store::counting::Counts;
use vertep_tiramemsu::TiramemsuShard;

/// `Err` naming the operation and both counts when `counted` is over `budget`.
fn within_budget(operation: &str, budget: u64, counted: u64) -> Result<(), String> {
    if counted <= budget {
        Ok(())
    } else {
        Err(format!(
            "budget breached: {operation}: budget {budget}, counted {counted}"
        ))
    }
}

fn check(operation: &str, budget: u64, counted: u64) {
    if let Err(e) = within_budget(operation, budget, counted) {
        panic!("{e}");
    }
}

/// A node on one tiramemsu shard behind the counting wrapper, with store mailboxes on it.
fn counted_node(
    dir: &tempfile::TempDir,
    config: NodeConfig,
) -> (Node, Arc<CountingStorage<TiramemsuShard>>) {
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("budget.db")).unwrap());
    let storage = Arc::new(CountingStorage::new(shard));
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock),
    };
    let node = Node::builder(storage.clone(), shell)
        .datamodels(ecma())
        .mailboxes(Arc::new(StoreMailboxes::new(storage.clone())))
        .config(config)
        .build();
    (node, storage)
}

/// Resolves `test:out` to an in-memory mailbox, so a send leaves the node without a store write
/// on the receiving side.
struct Outside(Arc<MemoryMailbox>);

impl Resolver for Outside {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        (address.as_str() == "test:out").then(|| self.0.clone() as Arc<dyn Mailbox>)
    }
}

const COUNTER: &str = r#"<datamodel><data id="n" expr="0"/></datamodel>
<state id="s"><transition event="inc"><assign location="n" expr="n + 1"/></transition></state>"#;

fn report(label: &str, c: &Counts) {
    eprintln!("{label}: {c}");
}

// @tg: verifies:: [[openspec:runtime-performance#Operation Budgets#Budget breach fails the build]] {level: unit}
#[test]
fn breach_names_operation_and_both_counts() {
    assert!(within_budget("turn commits", 1, 1).is_ok());
    assert_eq!(
        within_budget("turn commits", 1, 2).unwrap_err(),
        "budget breached: turn commits: budget 1, counted 2"
    );
}

// @tg: verifies:: [[openspec:runtime-performance#Operation Budgets#A turn with a send commits once]] {level: integration}
#[test]
#[ignore = "M18 phase 2 (one commit per turn) makes this pass"]
fn turn_with_a_send_commits_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, storage) = counted_node(&dir, NodeConfig::default());
    let outside = Arc::new(MemoryMailbox::default());
    node.handle()
        .register_resolver("test", Arc::new(Outside(outside.clone())));
    node.register_type(
        &ty("sender"),
        scxml(
            "s",
            r#"<state id="s"><transition event="go"><send event="out" target="test:out"/></transition></state>"#,
        ),
    )
    .unwrap();
    let a = va("sender", "a");
    // Warm up: activate the actor so that the measured turn is a steady-state one.
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let counters = storage.counters().clone();
    node.send(&a, ev("go")).unwrap();
    counters.reset();
    {
        let _turn = counters.scope("turn");
        node.run_until_idle();
    }
    assert_eq!(outside.pending().len(), 2, "both sends left the node");
    let turn = counters.get("turn");
    report("turn with a send", &turn);
    check("turn commits", 1, turn.commits);
}

// @tg: verifies:: [[openspec:runtime-performance#Operation Budgets#Activation reads only after the snapshot]] {level: integration}
#[test]
#[ignore = "M18 phase 2 (activation reads from the snapshot) makes this pass"]
fn activation_reads_only_after_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, storage) = counted_node(
        &dir,
        NodeConfig {
            snapshot_interval: 990,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("counter"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("counter", "big");
    for _ in 0..1000 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    node.deactivate(&a);
    let counters = storage.counters().clone();
    counters.reset();
    {
        let _s = counters.scope("activation");
        node.activate_actor(&a).unwrap();
    }
    let act = counters.get("activation");
    report("activation, 1000 entries, snapshot at 990", &act);
    check("activation journal reads", 1, act.journal_reads);
    check("activation entries read", 10, act.entries_read);
}

// @tg: verifies:: [[openspec:event-journal#Indexed Deduplication]] {level: integration}
#[test]
#[ignore = "M18 phase 2 (indexed deduplication) makes this pass"]
fn duplicate_is_dropped_without_a_full_read() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, storage) = counted_node(
        &dir,
        NodeConfig {
            snapshot_interval: 990,
            ..NodeConfig::default()
        },
    );
    node.register_type(&ty("counter"), ecma_scxml("s", COUNTER))
        .unwrap();
    let a = va("counter", "dedup");
    let first = "urn:vertep:budget:first";
    node.send_envelope(Envelope::new(first, &a, ev("inc")))
        .unwrap();
    for _ in 1..1000 {
        node.send(&a, ev("inc")).unwrap();
    }
    node.run_until_idle();
    node.deactivate(&a);
    let counters = storage.counters().clone();
    counters.reset();
    {
        let _s = counters.scope("duplicate");
        node.send_envelope(Envelope::new(first, &a, ev("inc")))
            .unwrap();
        node.run_until_idle();
    }
    assert_eq!(
        node.journal(&a).unwrap().len(),
        1000,
        "the duplicate was not processed"
    );
    let dup = counters.get("duplicate");
    report("duplicate after deactivation, 1000 entries", &dup);
    check("duplicate entries read", 10, dup.entries_read);
}
