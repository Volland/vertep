#![allow(missing_docs)] // criterion_group! generates undocumented items
//! Pure `vertep_core` step throughput on three chart sizes, with the null datamodel so the
//! core, not a datamodel engine, is measured.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vertep_bench::{ev, scxml};
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Environment, SessionInfo, Value};
use vertep_core::null::NullDatamodel;
use vertep_core::session::{Input, Options, Session, StartInput, start, step};

/// A ring of `n` states `{p}0 .. {p}{n-1}` moving on `next`.
fn ring(p: &str, n: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                r#"<state id="{p}{i}"><transition event="next" target="{p}{}"/></state>"#,
                (i + 1) % n
            )
        })
        .collect()
}

/// `regions` parallel regions, each a compound state holding a ring of `per` states.
fn parallel(regions: usize, per: usize) -> String {
    let body: String = (0..regions)
        .map(|r| {
            format!(
                r#"<state id="r{r}" initial="r{r}s0">{}</state>"#,
                ring(&format!("r{r}s"), per)
            )
        })
        .collect();
    format!(r#"<parallel id="p">{body}</parallel>"#)
}

fn compiled(initial: &str, body: &str) -> CompiledChart {
    CompiledChart::new(scxml("null", initial, body)).expect("chart compiles")
}

fn start_input() -> StartInput {
    StartInput {
        info: SessionInfo {
            sessionid: "bench".to_owned(),
            name: None,
            ioprocessors: Value::Undefined,
        },
        env: Environment::default(),
        options: Options::default(),
        data: Vec::new(),
        sessions: None,
    }
}

// @tg: implements:: [[openspec:production-readiness#Benchmark Suite]]
fn macrostep(c: &mut Criterion) {
    let mut g = c.benchmark_group("step");
    g.throughput(Throughput::Elements(1));
    let charts = [
        ("small", compiled("s0", &ring("s", 3))),
        ("medium", compiled("p", &parallel(3, 10))),
        ("large", compiled("p", &parallel(10, 30))),
    ];
    for (size, cc) in &charts {
        let mut session: Option<Session> = Some(
            start(cc, start_input(), &mut NullDatamodel)
                .expect("start")
                .session,
        );
        g.bench_function(BenchmarkId::new("macrostep", size), |b| {
            b.iter(|| {
                let s = session.take().expect("session");
                let out = step(
                    cc,
                    s,
                    Input::event(ev("next"), Environment::default()),
                    &mut NullDatamodel,
                )
                .expect("step");
                session = Some(out.session);
            });
        });
    }
    g.finish();
}

criterion_group!(benches, macrostep);
criterion_main!(benches);
