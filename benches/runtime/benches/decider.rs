#![allow(missing_docs)] // criterion_group! generates undocumented items
//! Scripted decider round trip: an event enters a decide state, the decider is offered the
//! enabled events, its answer is journaled and processed, and the chart returns to `idle`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vertep_agent::{
    AgentHost, Decider, DeciderKind, Decision, DecisionOutcome, DecisionRequest, MemoryJournal,
};
use vertep_bench::{T0, ev, scxml};
use vertep_core::datamodel::Value;
use vertep_runtime::VirtualClock;

const AGENT: &str = "https://example.org/bench/agents/decider";

/// Answers every offer with `done`, at once.
struct Immediate;

impl Decider for Immediate {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }

    fn decide(
        &self,
        _request: DecisionRequest,
    ) -> Pin<Box<dyn Future<Output = DecisionOutcome> + Send>> {
        Box::pin(async { DecisionOutcome::Decision(Decision::new("done", Value::Undefined)) })
    }
}

// @tg: implements:: [[openspec:production-readiness#Benchmark Suite]]
fn round_trip(c: &mut Criterion) {
    let mut g = c.benchmark_group("decider");
    g.throughput(Throughput::Elements(1));
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .journal(Arc::new(MemoryJournal::new()))
        .decider("policy", Arc::new(Immediate))
        .build();
    let chart = scxml(
        "null",
        "idle",
        r#"<state id="idle"><transition event="go" target="ask"/></state>
<state id="ask" vt:decide="policy" vt:candidates="done"><transition event="done" target="idle"/></state>"#,
    );
    let version = host.register(chart).expect("chart registers");
    host.spawn(AGENT, &version).expect("spawn");
    host.run_until_idle();
    g.bench_function(BenchmarkId::new("round_trip", "scripted"), |b| {
        b.iter(|| {
            host.send(AGENT, ev("go"));
            host.run_until_idle();
        });
    });
    g.finish();
    let state = host.state(AGENT).expect("state");
    assert!(
        state.configuration.iter().any(|s| s == "idle"),
        "every round trip came back to idle: {:?}",
        state.configuration
    );
}

criterion_group!(benches, round_trip);
criterion_main!(benches);
