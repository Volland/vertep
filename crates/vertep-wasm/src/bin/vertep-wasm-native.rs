//! The native side of the native-versus-WebAssembly tests in `bindings/js`: the same trace and
//! command-script functions as the WebAssembly build, run natively.
//!
//! - `vertep-wasm-native trace < cases.json`: step traces of chart cases (see `trace`).
//! - `vertep-wasm-native script <script.json>`: runs a command script on a [`System`] on a
//!   virtual clock and prints `{results, notifications}`. The script is `{options, start,
//!   steps, export?}`; a step is `{op, args}`, `{advance: ms}` or `{pump: true}`. With
//!   `options.storage = "native"` the store is a tiramemsu file opened with tiramemsu's native
//!   host (WAL), as the native runtime opens it, at `options.path`.
//! - `vertep-wasm-native gen-diff <n> <seed>`: `n` generated differential cases (chart IR and
//!   inputs) from the differential test's chart generator.
//! - `vertep-wasm-native diff < cases.json`: their oracle records, as the Lean oracle gets them.

use std::io::Read;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value as J, json};
use vertep_runtime::Shell;
use vertep_wasm::{Datamodels, Store, System, SystemOptions, trace_all};

fn trace() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let cases = serde_json::from_str::<Vec<vertep_wasm::TraceCase>>(&input)
        .map_err(|e| format!("cases: {e}"))?;
    Ok(trace_all(
        &cases,
        &Datamodels::new(std::sync::Arc::new(Default::default())),
    ))
}

fn script(path: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let script: J = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let mut options = script.get("options").cloned().unwrap_or(json!({}));
    let native = options.get("storage").and_then(J::as_str) == Some("native");
    if native {
        options["storage"] = json!("file");
    }
    let opts = SystemOptions::from_json(&options).map_err(|e| e.to_string())?;
    let store = if native {
        Store::Journal(std::sync::Arc::new(
            vertep_wasm::store::journal::JournalStore::native(&opts.store.path)
                .map_err(|e| e.to_string())?,
        ))
    } else {
        Store::open(&opts.store, None).map_err(|e| e.to_string())?
    };
    let start = script.get("start").and_then(J::as_i64).unwrap_or(0);
    let (shell, clock) = Shell::virtual_time(start);
    let mut system = System::new(&opts, store, shell, None).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for step in script
        .get("steps")
        .and_then(J::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Some(ms) = step.get("advance").and_then(J::as_u64) {
            clock.advance(Duration::from_millis(ms));
            system.pump();
            continue;
        }
        if step.get("pump").is_some() {
            system.pump();
            continue;
        }
        let op = step.get("op").and_then(J::as_str).unwrap_or_default();
        let args = step.get("args").cloned().unwrap_or(json!({}));
        results.push(match system.command(op, &args) {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "err": e.to_json() }),
        });
    }
    let notifications = system.take_notifications();
    if let Some(out) = script.get("export").and_then(J::as_str) {
        let bytes = system.store().export().map_err(|e| e.to_string())?;
        std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))?;
    }
    Ok(json!({ "results": results, "notifications": notifications }).to_string())
}

/// `n` generated differential cases from `seed`, with the differential test's chart limits.
fn gen_diff(n: usize, seed: u64) -> Result<String, String> {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
    let cfg = vertep_testkit::ChartConfig {
        max_depth: 3,
        max_top_states: 3,
        max_children: 3,
        max_transitions: 2,
        max_targets: 1,
        max_block: 2,
        exec_depth: 1,
        ..vertep_testkit::ChartConfig::default()
    };
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &bytes),
    );
    let strategy = vertep_testkit::arb_chart_with(cfg);
    let mut cases = Vec::new();
    let mut i = 0u64;
    while cases.len() < n {
        let chart = strategy
            .new_tree(&mut runner)
            .map_err(|e| e.to_string())?
            .current();
        let names = event_names(&chart);
        let inputs: Vec<vertep_wasm::trace::DiffInput> = (0..5)
            .map(|k| {
                i += 1;
                if (i + k).is_multiple_of(13) {
                    vertep_wasm::trace::DiffInput::Cancel
                } else {
                    vertep_wasm::trace::DiffInput::Event(
                        names[((i * 7 + k * 3) as usize) % names.len()].clone(),
                    )
                }
            })
            .collect();
        cases.push(vertep_wasm::trace::DiffCase { chart, inputs });
    }
    serde_json::to_string(&cases).map_err(|e| e.to_string())
}

/// Event names some transition listens for, plus one nothing listens for (as the differential
/// test picks them).
fn event_names(chart: &vertep_core::ir::Chart) -> Vec<String> {
    fn walk(states: &[vertep_core::ir::State], out: &mut Vec<String>) {
        for s in states {
            for t in &s.transitions {
                for d in &t.events {
                    let name = match d.as_str() {
                        "*" => "any".to_string(),
                        d => d.trim_end_matches(".*").trim_end_matches('.').to_string(),
                    };
                    if !name.is_empty() && !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
            walk(&s.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&chart.states, &mut out);
    out.push("unheard".to_string());
    out
}

fn diff() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let cases = serde_json::from_str::<Vec<vertep_wasm::trace::DiffCase>>(&input)
        .map_err(|e| format!("cases: {e}"))?;
    Ok(vertep_wasm::trace::diff_all(
        &cases,
        &Datamodels::new(std::sync::Arc::new(Default::default())),
    ))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("trace") => trace(),
        Some("script") if args.len() == 3 => script(&args[2]),
        Some("gen-diff") if args.len() == 4 => match (args[2].parse(), args[3].parse()) {
            (Ok(n), Ok(seed)) => gen_diff(n, seed),
            _ => Err("gen-diff <count> <seed>".into()),
        },
        Some("diff") => diff(),
        _ => Err(
            "usage: vertep-wasm-native trace < cases.json | script <script.json> | gen-diff <n> <seed> | diff < cases.json"
                .into(),
        ),
    };
    match r {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
