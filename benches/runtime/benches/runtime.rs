#![allow(missing_docs)] // criterion_group! generates undocumented items
//! The runtime's hot paths: turns, activation against history, receive under deferral, and
//! journal append and replay per backend.
//!
//! Ids are `group/case/parameter`; `scripts/bench-gate.py` reads them from `target/criterion`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxrdf::{Literal, NamedNode, Triple};
use vertep_bench::{Backend, Store, counter_with_history, ev, node_on, scxml, ty, va};
use vertep_runtime::mailbox::{Mailbox, MailboxFactory, MemoryMailbox};
use vertep_runtime::{Address, Envelope, NodeConfig, Resolver, StoreMailboxes};
use vertep_store::{AppendMeta, Entry, Position};

/// Resolves `bench:out` to an in-memory mailbox: a send that leaves the node.
struct Outside(Arc<MemoryMailbox>);

impl Resolver for Outside {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        (address.as_str() == "bench:out").then(|| self.0.clone() as Arc<dyn Mailbox>)
    }
}

fn quick() -> bool {
    std::env::var_os("VERTEP_BENCH_QUICK").is_some()
}

/// Turns per second: one event, one turn, on a warm actor; with a send, the turn also sends one
/// event out of the node.
fn turns(c: &mut Criterion) {
    let mut g = c.benchmark_group("turns");
    g.throughput(Throughput::Elements(1));
    for backend in Backend::ALL {
        for with_send in [false, true] {
            let store = Store::open(backend);
            let mut node = node_on(store.storage.clone(), NodeConfig::default());
            let outside = Arc::new(MemoryMailbox::default());
            node.handle()
                .register_resolver("bench", Arc::new(Outside(outside.clone())));
            let body = if with_send {
                r#"<state id="s"><transition event="go"><send event="out" target="bench:out"/></transition></state>"#
            } else {
                r#"<state id="s"><transition event="go"/></state>"#
            };
            node.register_type(&ty("turn"), scxml("null", "s", body))
                .expect("chart registers");
            let a = va("turn", "a");
            node.send(&a, ev("go")).expect("send");
            node.run_until_idle();
            let case = if with_send { "send" } else { "no_send" };
            g.bench_function(BenchmarkId::new(backend.name(), case), |b| {
                b.iter(|| {
                    node.send(&a, ev("go")).expect("send");
                    node.run_until_idle();
                    if let Some(out) = outside.receive(&[]) {
                        outside.ack(&out.message_id).expect("ack");
                    }
                });
            });
        }
    }
    g.finish();
}

// @tg: verifies:: [[openspec:runtime-performance#Hot Path Benchmarks#Activation is measured against history]] {level: e2e}
/// Activation of an actor with 10, 1,000 and 100,000 entries, without a snapshot and with one
/// ten entries before the head.
fn activation(c: &mut Criterion) {
    let mut g = c.benchmark_group("activation");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(5));
    let lengths: &[u64] = if quick() {
        &[10, 1_000]
    } else {
        &[10, 1_000, 100_000]
    };
    for &len in lengths {
        for snapshot in [false, true] {
            let interval = if snapshot { len.saturating_sub(10) } else { 0 };
            let store = Store::open(Backend::Memory);
            let (mut node, a) = counter_with_history(
                &store,
                "history",
                len,
                NodeConfig {
                    snapshot_interval: interval,
                    ..NodeConfig::default()
                },
            );
            let case = if snapshot { "snapshot" } else { "no_snapshot" };
            g.bench_function(BenchmarkId::new(case, len), |b| {
                b.iter(|| {
                    node.deactivate(&a);
                    node.activate_actor(&a).expect("activates");
                });
            });
        }
    }
    g.finish();
}

/// Receive with 0, 100 and 10,000 deferred messages ahead of the deliverable one.
fn deferred_receive(c: &mut Criterion) {
    let mut g = c.benchmark_group("receive");
    let deferring = vec!["order".to_owned()];
    for deferred in [0usize, 100, 10_000] {
        for kind in ["memory", "store"] {
            let store = Store::open(Backend::Memory);
            let to = va("busy", "a");
            let mailbox: Arc<dyn Mailbox> = match kind {
                "memory" => Arc::new(MemoryMailbox::default()),
                _ => StoreMailboxes::new(store.storage.clone()).create(&to),
            };
            for i in 0..deferred {
                mailbox
                    .enqueue(Envelope::new(
                        format!("urn:bench:d{i}"),
                        &to,
                        ev("order.placed"),
                    ))
                    .expect("enqueue");
            }
            mailbox
                .enqueue(Envelope::new("urn:bench:go", &to, ev("go")))
                .expect("enqueue");
            g.bench_function(BenchmarkId::new(kind, deferred), |b| {
                b.iter(|| {
                    let env = mailbox.receive(black_box(&deferring)).expect("deliverable");
                    mailbox.release(&env.message_id);
                });
            });
        }
    }
    g.finish();
}

fn entry(actor: &str, n: u64) -> Entry {
    let iri = |s: String| NamedNode::new(s).expect("iri");
    let name = iri(format!("{actor}/entry/{n}"));
    Entry::new(
        name.clone(),
        (0..6).map(|k| {
            Triple::new(
                name.clone(),
                iri(format!("urn:vertep:bench:p{k}")),
                Literal::from(format!("value {n} {k}")),
            )
        }),
    )
}

/// Journal append latency (one six-statement entry) and replay of 1,000 entries, per backend.
fn journal(c: &mut Criterion) {
    let mut g = c.benchmark_group("journal");
    for backend in Backend::ALL {
        let store = Store::open(backend);
        let journal = store.storage.journal();
        let actor = NamedNode::new("urn:vertep:bench:append").expect("iri");
        let mut head = 0u64;
        g.bench_function(BenchmarkId::new("append", backend.name()), |b| {
            b.iter(|| {
                let e = entry(actor.as_str(), head + 1);
                head = journal
                    .append(&actor, Position::new(head), &[e], &AppendMeta::none())
                    .expect("append")
                    .get();
            });
        });

        let replayed = NamedNode::new("urn:vertep:bench:replay").expect("iri");
        let entries: Vec<Entry> = (1..=1000).map(|n| entry(replayed.as_str(), n)).collect();
        journal
            .append(&replayed, Position::new(0), &entries, &AppendMeta::none())
            .expect("history");
        g.throughput(Throughput::Elements(1000));
        g.bench_function(BenchmarkId::new("replay_1000", backend.name()), |b| {
            b.iter(|| black_box(journal.read(&replayed, Position::new(0)).expect("read")));
        });
        g.throughput(Throughput::Elements(1));
    }
    g.finish();
}

criterion_group!(benches, turns, activation, deferred_receive, journal);
criterion_main!(benches);
