//! Runs every example and compares what it prints with the `expected-output.txt` recorded next
//! to it, so the examples (and the docs that quote them) cannot drift from the code.
//!
//! Re-record after an intended change with `VERTEP_BLESS=1 cargo test -p vertep-examples`.

mod support;

use std::process::Command;

use support::{check_recorded, examples_dir};

/// Runs a binary of this package and returns its standard output; fails on a non-zero exit.
fn run(bin: &str) -> String {
    let out = Command::new(bin).output().expect("the example starts");
    assert!(
        out.status.success(),
        "{bin} failed with {}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("UTF-8 output")
}

// @tg: verifies:: [[openspec:scxml-interpretation#Send And Cancel Effects#Cancel by send identifier]] {level: e2e}
#[test]
fn traffic_light() {
    let out = run(env!("CARGO_BIN_EXE_traffic-light"));
    assert!(out.contains("[  40s] button   -> operating light yellow crossing dont_walk"));
    assert!(out.contains("[  50s] fault    -> flashing\n          cancel light-timer"));
    check_recorded("traffic-light/expected-output.txt", &out);
}

#[test]
fn rust_macro() {
    let out = run(env!("CARGO_BIN_EXE_rust-macro"));
    assert!(out.contains("Coin { cents: 50 }     -> [Service, Unlocked]"));
    check_recorded("rust-macro/expected-output.txt", &out);
}

// @tg: verifies:: [[openspec:durable-timers#Delayed Send Is Journaled#Timer survives a crash]] {level: e2e}
#[test]
fn durable_actor() {
    let out = run(env!("CARGO_BIN_EXE_durable-actor"));
    assert!(
        out.contains(r#"process 2: ["awaiting_payment"] context={"items":2} (replayed 3 entries)"#)
    );
    assert!(out.contains("entry 4 at +900s: payment.timeout"));
    check_recorded("durable-actor/expected-output.txt", &out);
}

// @tg: verifies:: [[openspec:deciders#Enabled Event Offer#Only enabled transitions are offered]] {level: e2e}
// @tg: verifies:: [[openspec:deciders#Decisions Are Journaled Inputs#Replay does not call the decider]] {level: e2e}
#[test]
fn decider_agent() {
    let out = run(env!("CARGO_BIN_EXE_decider-agent"));
    assert!(out.contains(r#"offered ["reject", "escalate"] -> escalate"#));
    assert!(out.contains("entries replayed: 4, decider calls: 0"));
    check_recorded("decider-agent/expected-output.txt", &out);
}

/// The Node.js example, when Node.js and the example's `node_modules` (the link to the built
/// `@vertep/web`) are present; skipped otherwise, since building the WebAssembly packages needs
/// the wasm32 target and the wasm-bindgen CLI.
#[test]
fn browser_js() {
    let dir = examples_dir().join("browser-js");
    if !dir.join("node_modules/@vertep/web/dist/index.js").exists() {
        eprintln!("skipped: run `npm install` in examples/browser-js after building bindings/js");
        return;
    }
    let out = match Command::new("node")
        .arg("main.mjs")
        .current_dir(&dir)
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!("skipped: node is not available ({e})");
            return;
        }
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = String::from_utf8(out.stdout).expect("UTF-8 output");
    check_recorded("browser-js/expected-output.txt", &out);
}

/// The typed-order chart is not only well-typed: it runs. A short session through the core
/// interpreter on the CEL datamodel.
///
/// It needs a *typed* CEL datamodel, built from the chart's type environment: the untyped one
/// that `CelRegistry` creates (and that the actor runtime, the agent host and the WebAssembly
/// worker use) initializes `total: decimal = "0"` as an int and leaves decimal payload fields
/// as strings, so `ctx.total + ev.data.price * ...` raises `error.execution` there. The type
/// environment projection lives in `vertep-cli` (`vertep_cli::cel::type_env`).
#[test]
fn typed_order_runs() {
    use vertep::core::compile::CompiledChart;
    use vertep::core::datamodel::{Environment, Event, SessionInfo, Value};
    use vertep::core::session::{Input, StartInput, start, step};
    use vertep::core::types::{BasicTyper, check_chart};

    let text = std::fs::read_to_string(examples_dir().join("typed-order/order.yaml")).unwrap();
    let chart = CompiledChart::new(vertep_yaml::read_str(&text).unwrap()).unwrap();
    let start_input = StartInput {
        info: SessionInfo {
            sessionid: "order-1".into(),
            ..SessionInfo::default()
        },
        ..StartInput::default()
    };
    let types = check_chart(chart.chart(), &BasicTyper::for_datamodel("cel")).unwrap();
    let options = vertep_dm_cel::LoadOptions {
        env: vertep_cli::cel::type_env(&types),
        ..vertep_dm_cel::LoadOptions::default()
    };
    let mut dm = vertep_dm_cel::load(chart.chart(), &options).unwrap();
    let outcome = start(&chart, start_input, &mut dm).unwrap();
    let mut session = outcome.session;
    let obj = |fields: &[(&str, Value)]| {
        Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    };
    let item = |qty: f64| {
        obj(&[
            ("sku", Value::String("BOOK-1".into())),
            ("qty", Value::Number(qty)),
            ("price", Value::String("12.50".into())),
        ])
    };
    let card = obj(&[
        ("tag", Value::String("Card".into())),
        ("value", obj(&[("last4", Value::String("4242".into()))])),
    ]);
    let pay = |amount: &str, method: &Value| {
        obj(&[
            ("amount", Value::String(amount.into())),
            ("method", method.clone()),
        ])
    };
    let events = [
        ("add", item(2.0), vec!["shopping", "cart"]),
        ("add", item(0.0), vec!["shopping", "cart"]), // guard false: qty must be positive
        ("checkout", Value::Null, vec!["shopping", "payment"]),
        // 20 is less than the total of 2 x 12.50: stay in payment
        ("pay", pay("20", &card), vec!["shopping", "payment"]),
        ("pay", pay("25", &card), vec!["fulfilment"]),
    ];
    for (name, data, want) in events {
        let event = Event {
            name: name.into(),
            data,
            ..Event::default()
        };
        let input = Input::event(event, Environment::default());
        let outcome = step(&chart, session, input, &mut dm).unwrap();
        let failed = (outcome.trace.microsteps.iter())
            .any(|m| m.event.as_deref() == Some("error.execution"));
        assert!(!failed, "{name} raised error.execution");
        assert_eq!(outcome.trace.configuration, want, "after {name}");
        session = outcome.session;
    }
}
