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
//! - `vertep-wasm-native conformance-input`: the committed W3C IRP suite's non-manual variants,
//!   their documents and the allow-list, as `conformanceRun` in WebAssembly takes them.
//! - `vertep-wasm-native conformance-native`: the native harness's verdicts on those variants.
//! - `vertep-wasm-native ws-server`: a native actor system (tokio shell, memory store) with the
//!   WebSocket endpoint on 127.0.0.1, hosting the chart type
//!   `https://server.example/charts/server`; it accepts the credential `secret` only. It prints
//!   `{"port": n}`, then reads one JSON command per line from stdin and prints one JSON line per
//!   command: a bridge command `{op, args}` (`snapshot`, `journal`, ...), or the controls
//!   `{"ctl": "hold" | "release" | "dup" | "nodup" | "drop" | "pending"}` (fault injection for
//!   the end-to-end tests).

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

/// The chart of the server actors in `ws-server`: counts `ping`s, remembers the last origin and
/// origin type, and answers `pong` to the origin; counts `hit`s.
const SERVER_CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s" datamodel="cel">
  <datamodel><data id="n" expr="0"/><data id="hits" expr="0"/><data id="ot" expr="''"/><data id="from" expr="''"/></datamodel>
  <state id="s">
    <transition event="ping">
      <assign location="ctx.n" expr="ctx.n + 1"/>
      <assign location="ctx.ot" expr="ev.origintype"/>
      <assign location="ctx.from" expr="ev.origin"/>
      <send event="pong" targetexpr="ev.origin"><content expr="{'n': ctx.n}"/></send>
    </transition>
    <transition event="hit"><assign location="ctx.hits" expr="ctx.hits + 1"/></transition>
  </state>
</scxml>"#;

fn ws_server() -> Result<String, String> {
    use std::io::{BufRead, Write};
    use vertep_runtime::websocket::server::{ServerOptions, WebSocketServer};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let opts =
            SystemOptions::from_json(&json!({ "storage": "memory" })).map_err(|e| e.to_string())?;
        let store = Store::open(&opts.store, None).map_err(|e| e.to_string())?;
        let mut system =
            System::new(&opts, store, Shell::tokio(), None).map_err(|e| e.to_string())?;
        system
            .command(
                "register",
                &json!({ "source": SERVER_CHART, "type": "https://server.example/charts/server" }),
            )
            .map_err(|e| e.to_string())?;
        let server = WebSocketServer::new(
            system.handle(),
            ServerOptions::new("urn:vertep:test-server")
                .hosts(vec!["https://server.example/".into()])
                .credentials(|c| match c {
                    Some("secret") => Ok(()),
                    _ => Err("unknown credential".into()),
                }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        tokio::spawn({
            let s = server.clone();
            async move { s.serve(listener).await }
        });
        let out = |j: J| {
            let mut o = std::io::stdout().lock();
            let _ = writeln!(o, "{j}");
            let _ = o.flush();
        };
        out(json!({ "port": port }));
        loop {
            system.pump();
            let wake = system.handle().wakeup();
            let wait = match system.next_deadline() {
                Some(t) => {
                    let ms = u64::try_from(t - system.handle().now_ms()).unwrap_or(0);
                    Duration::from_millis(ms.min(1_000))
                }
                None => Duration::from_millis(1_000),
            };
            tokio::select! {
                _ = wake => {}
                _ = tokio::time::sleep(wait) => {}
                line = rx.recv() => {
                    let Some(line) = line else { return Ok(String::new()) };
                    let cmd: J = serde_json::from_str(&line).unwrap_or(J::Null);
                    let reply = match cmd.get("ctl").and_then(J::as_str) {
                        Some("hold") => { server.set_hold_acks(true); json!({ "ok": true }) }
                        Some("release") => { server.set_hold_acks(false); json!({ "ok": true }) }
                        Some("dup") => { server.set_duplicate(true); json!({ "ok": true }) }
                        Some("nodup") => { server.set_duplicate(false); json!({ "ok": true }) }
                        Some("drop") => { server.disconnect_all(); json!({ "ok": true }) }
                        Some("pending") => json!({ "ok": server.pending() }),
                        Some(other) => json!({ "err": format!("unknown control {other}") }),
                        None => {
                            let op = cmd.get("op").and_then(J::as_str).unwrap_or_default();
                            let args = cmd.get("args").cloned().unwrap_or(json!({}));
                            match system.command(op, &args) {
                                Ok(v) => json!({ "ok": v }),
                                Err(e) => json!({ "err": e.to_json() }),
                            }
                        }
                    };
                    out(reply);
                }
            }
        }
    })
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
        Some("ws-server") => ws_server(),
        Some("conformance-input") => vertep_wasm::conformance::input()
            .and_then(|i| serde_json::to_string(&i).map_err(|e| e.to_string())),
        Some("conformance-native") => vertep_wasm::conformance::native_verdicts()
            .and_then(|v| serde_json::to_string(&v).map_err(|e| e.to_string())),
        _ => Err(
            "usage: vertep-wasm-native trace < cases.json | script <script.json> | gen-diff <n> <seed> | diff < cases.json | conformance-input | conformance-native | ws-server"
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
