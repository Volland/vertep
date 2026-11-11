//! The actor system behind the JavaScript API, run natively on a virtual clock. The same
//! commands run in WebAssembly under Node.js in `bindings/js/packages/web/test`.

use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value as J, json};
use vertep_runtime::{Shell, VirtualClock};
use vertep_wasm::functions::ActionOutcome;
use vertep_wasm::{
    Code, Implementations, StorageKind, Store, StoreOptions, System, SystemOptions, event_from_json,
};

const ORDER_TYPE: &str = "https://example.org/charts/order";

const ORDER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="2s"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"#;

const COUNTER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="cel">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="idle">
    <transition event="inc"><assign location="ctx.n" expr="ctx.n + 1"/></transition>
    <transition event="go" target="busy"/>
  </state>
  <state id="busy"><transition event="done" target="idle"/></state>
</scxml>"#;

const PRICER: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s" datamodel="cel">
  <state id="s">
    <transition event="price.get">
      <send event="vt:reply" targetexpr="ev.origin"><content expr="{'price': 42}"/></send>
    </transition>
    <transition event="price.slow">
      <send event="vt:reply" targetexpr="ev.origin" delay="300ms"><content expr="{'price': 7}"/></send>
    </transition>
  </state>
</scxml>"#;

fn options(kind: StorageKind, path: &str, journal: bool) -> SystemOptions {
    SystemOptions {
        store: StoreOptions {
            kind,
            path: path.into(),
            journal,
        },
        dev: false,
        idle_timeout_ms: None,
        guards: Vec::new(),
        actions: Vec::new(),
    }
}

fn memory(at: i64) -> (System, VirtualClock) {
    let opts = options(StorageKind::Memory, "m.db", false);
    let store = Store::open(&opts.store, None).unwrap();
    let (shell, clock) = Shell::virtual_time(at);
    (System::new(&opts, store, shell, None).unwrap(), clock)
}

fn file_system(path: &std::path::Path, at: i64) -> (System, VirtualClock) {
    let opts = options(StorageKind::File, path.to_str().unwrap(), true);
    let store = Store::open(&opts.store, None).unwrap();
    let (shell, clock) = Shell::virtual_time(at);
    (System::new(&opts, store, shell, None).unwrap(), clock)
}

fn cmd(s: &mut System, op: &str, args: J) -> J {
    s.command(op, &args)
        .unwrap_or_else(|e| panic!("{op} {args}: {e}"))
}

fn spawn(s: &mut System, source: &str, key: &str) -> String {
    let r = cmd(
        s,
        "spawn",
        json!({ "source": source, "type": ORDER_TYPE, "key": key }),
    );
    r["address"].as_str().unwrap().to_owned()
}

fn config(s: &mut System, a: &str) -> Vec<String> {
    let snap = cmd(s, "snapshot", json!({ "address": a }));
    serde_json::from_value(snap["configuration"].clone()).unwrap()
}

// @tg: verifies:: [[openspec:wasm-bindings#Core Builds For WebAssembly Unchanged#Core compiles for wasm32 without changes]] {level: unit}
#[test]
fn core_has_no_target_specific_code() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vertep-core/src");
    let mut stack = vec![dir];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            files += 1;
            for bad in [
                "target_arch",
                "target_family",
                "target_os",
                "wasm_bindgen",
                "js_sys",
                "web_sys",
            ] {
                assert!(
                    !text.contains(bad),
                    "{} mentions {bad}: the core has no target-specific code paths",
                    p.display()
                );
            }
        }
    }
    assert!(files > 10);
    // the core's own dependencies are only `serde` (optional): no std-only or browser crate
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vertep-core/Cargo.toml"),
    )
    .unwrap();
    let deps = manifest.split("[dependencies]").nth(1).unwrap();
    let deps = deps.split("\n[").next().unwrap();
    let names: Vec<&str> = deps
        .lines()
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .filter(|n| !n.is_empty() && !n.starts_with('#'))
        .collect();
    assert_eq!(names, vec!["serde"]);
}

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Create from SCXML text]] {level: integration}
#[test]
fn create_from_scxml_text() {
    let (mut s, _) = memory(1_000);
    let r = cmd(
        &mut s,
        "spawn",
        json!({ "source": ORDER, "type": ORDER_TYPE, "key": "1" }),
    );
    let snap = &r["snapshot"];
    assert_eq!(snap["configuration"], json!(["open"]));
    assert_eq!(snap["position"], json!(0));
    let version = snap["version"].as_str().unwrap();
    assert!(version.starts_with("urn:"), "{version}");
    assert_eq!(r["version"].as_str(), Some(version));
    // nothing is journaled until the first message
    assert_eq!(
        cmd(&mut s, "journal", json!({ "address": r["address"] })),
        json!([])
    );
}

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Same chart, same version IRI]] {level: integration}
#[test]
fn same_chart_same_version_iri() {
    let (mut s, _) = memory(0);
    let chart = vertep_wasm::read_chart(ORDER, vertep_wasm::ChartFormat::Scxml).unwrap();
    let turtle = vertep_wasm::charts::to_turtle(&chart);
    let a = cmd(&mut s, "spawn", json!({ "source": ORDER, "key": "a" }));
    let b = cmd(
        &mut s,
        "spawn",
        json!({ "source": turtle, "format": "rdf", "key": "b" }),
    );
    assert_eq!(a["snapshot"]["version"], b["snapshot"]["version"]);
    assert_eq!(a["type"], b["type"], "default types are version IRIs");
}

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Invalid chart is rejected]] {level: integration}
#[test]
fn invalid_chart_is_rejected() {
    let (mut s, _) = memory(0);
    let bad = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="nope">
  <state id="a"><transition event="x" target="missing"/></state>
  <state id="a"/>
</scxml>"#;
    let e = s
        .command("spawn", &json!({ "source": bad, "key": "k" }))
        .unwrap_err();
    assert_eq!(e.code, Code::ChartInvalid);
    assert!(e.violations.len() >= 2, "{:?}", e.violations);
    assert!(
        e.violations.iter().all(|v| v.line > 0),
        "{:?}",
        e.violations
    );
    assert_eq!(cmd(&mut s, "actors", json!({})), json!([]));
    let e = s
        .command("spawn", &json!({ "source": "states: [", "format": "yaml" }))
        .unwrap_err();
    assert_eq!(e.code, Code::ChartInvalid);
}

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Virtual actor addressed by type and key]] {level: integration}
#[test]
fn virtual_actor_addressed_by_type_and_key() {
    let (mut s, _) = memory(0);
    cmd(
        &mut s,
        "register",
        json!({ "source": ORDER, "type": ORDER_TYPE }),
    );
    let a = cmd(
        &mut s,
        "virtual",
        json!({ "type": ORDER_TYPE, "key": "k1" }),
    )["address"]
        .clone();
    assert_eq!(
        cmd(&mut s, "actors", json!({})),
        json!([]),
        "not active yet"
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "pay" } }),
    );
    assert_eq!(cmd(&mut s, "actors", json!({})), json!([a]));
    let again = cmd(
        &mut s,
        "virtual",
        json!({ "type": ORDER_TYPE, "key": "k1" }),
    )["address"]
        .clone();
    assert_eq!(again, a);
    assert_eq!(config(&mut s, a.as_str().unwrap()), vec!["paid"]);
}

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Send is processed in one macrostep]] {level: integration}
#[test]
fn send_is_processed_in_one_macrostep() {
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, COUNTER, "c");
    // queue two events without running the node, then run once
    let h = s.handle();
    let addr = vertep_runtime::Address::parse(&a).unwrap();
    let inc = event_from_json(&json!({ "type": "inc" })).unwrap();
    h.send(&addr, inc.clone()).unwrap();
    h.send(&addr, inc).unwrap();
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" } }),
    );
    let j = cmd(&mut s, "journal", json!({ "address": a }));
    let names: Vec<&str> = j
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["inc", "inc", "go"],
        "one entry per event, in order"
    );
    let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(snap["configuration"], json!(["busy"]));
    assert_eq!(snap["position"], json!(3));
}

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Duplicate send is deduplicated]] {level: integration}
#[test]
fn duplicate_send_is_deduplicated() {
    let (mut s, _) = memory(0);
    cmd(
        &mut s,
        "register",
        json!({ "source": COUNTER, "type": ORDER_TYPE }),
    );
    let a = cmd(&mut s, "virtual", json!({ "type": ORDER_TYPE, "key": "d" }))["address"].clone();
    for _ in 0..2 {
        let r = cmd(
            &mut s,
            "send",
            json!({ "to": a, "event": { "type": "inc" }, "messageId": "urn:msg:1" }),
        );
        assert_eq!(r["messageId"], json!("urn:msg:1"));
    }
    let j = cmd(&mut s, "journal", json!({ "address": a }));
    assert_eq!(j.as_array().unwrap().len(), 1);
    let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(snap["context"]["n"], json!(1.0));
}

async fn yields(n: usize) {
    for _ in 0..n {
        tokio::task::yield_now().await;
    }
}

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Request resolves with the reply]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn request_resolves_with_the_reply() {
    let (mut s, _clock) = memory(0);
    let a = spawn(&mut s, PRICER, "p");
    let h = s.handle();
    let to = vertep_runtime::Address::parse(&a).unwrap();
    let ev = event_from_json(&json!({ "type": "price.get" })).unwrap();
    let local = tokio::task::LocalSet::new();
    let reply = local
        .run_until(async move {
            let ask = tokio::task::spawn_local(vertep_wasm::request(h, to, ev, 1_000));
            yields(5).await;
            s.pump();
            yields(5).await;
            ask.await.unwrap()
        })
        .await
        .unwrap();
    assert_eq!(reply["name"], json!("vt:reply"));
    assert_eq!(reply["data"]["price"], json!(42.0));
}

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Request times out]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn request_times_out() {
    let (mut s, clock) = memory(0);
    let a = spawn(&mut s, PRICER, "p");
    let h = s.handle();
    let to = vertep_runtime::Address::parse(&a).unwrap();
    let ev = event_from_json(&json!({ "type": "price.slow" })).unwrap();
    let local = tokio::task::LocalSet::new();
    let (r, s) = local
        .run_until(async move {
            let ask = tokio::task::spawn_local(vertep_wasm::request(h, to, ev, 100));
            yields(5).await;
            s.pump();
            clock.advance(Duration::from_millis(100));
            yields(5).await;
            let r = ask.await.unwrap();
            // the reply is sent later and must not reach anyone
            clock.advance(Duration::from_millis(300));
            s.pump();
            (r, s)
        })
        .await;
    let e = r.unwrap_err();
    assert_eq!(e.code, Code::RequestTimeout);
    drop(s);
}

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Snapshot after each macrostep]] {level: integration}
#[test]
fn snapshot_after_each_macrostep() {
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, COUNTER, "c");
    let sub = cmd(
        &mut s,
        "subscribe",
        json!({ "address": a, "kind": "snapshot" }),
    )["id"]
        .clone();
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    // an event that changes nothing produces no snapshot
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "done" } }),
    );
    let n = s.take_notifications();
    assert_eq!(n.len(), 2, "{n:?}");
    for (i, x) in n.iter().enumerate() {
        assert_eq!(x["kind"], json!("snapshot"));
        assert_eq!(x["sub"], sub);
        assert_eq!(x["snapshot"]["position"], json!(i + 1));
        assert_eq!(x["snapshot"]["context"]["n"], json!((i + 1) as f64));
        assert!(
            x["snapshot"]["version"]
                .as_str()
                .unwrap()
                .starts_with("urn:")
        );
    }
}

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Transition notification lists taken transitions]] {level: integration}
#[test]
fn transition_notification_lists_taken_transitions() {
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, ORDER, "o");
    cmd(
        &mut s,
        "subscribe",
        json!({ "address": a, "kind": "transitions" }),
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "pay" } }),
    );
    let n = s.take_notifications();
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0]["event"]["name"], json!("pay"));
    assert_eq!(n[0]["transitions"][0]["source"], json!("open"));
    assert_eq!(n[0]["transitions"][0]["targets"], json!(["paid"]));
}

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Unsubscribe stops notifications]] {level: integration}
#[test]
fn unsubscribe_stops_notifications() {
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, COUNTER, "c");
    let id = cmd(&mut s, "subscribe", json!({ "address": a }))["id"].clone();
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    assert_eq!(s.take_notifications().len(), 1);
    assert_eq!(
        cmd(&mut s, "unsubscribe", json!({ "id": id })),
        json!({ "removed": true })
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    assert!(s.take_notifications().is_empty());
    let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(snap["context"]["n"], json!(2.0), "processing is unaffected");
}

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Read journal entries]] {level: integration}
#[test]
fn read_journal_entries() {
    let (mut s, clock) = memory(5_000);
    let a = spawn(&mut s, COUNTER, "c");
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" }, "messageId": "urn:m:1" }),
    );
    clock.advance(Duration::from_millis(10));
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" }, "messageId": "urn:m:2" }),
    );
    let j = cmd(&mut s, "journal", json!({ "address": a, "from": 2 }));
    let j = j.as_array().unwrap();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0]["position"], json!(2));
    assert_eq!(j[0]["messageId"], json!("urn:m:2"));
    assert_eq!(j[0]["timestamp"], json!(5_010));
    assert_eq!(j[0]["event"]["name"], json!("go"));
    assert!(j[0]["version"].as_str().unwrap().starts_with("urn:"));
}

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Snapshot as of a past position]] {level: integration}
#[test]
fn snapshot_as_of_a_past_position() {
    let (mut s, clock) = memory(1_000);
    let a = spawn(&mut s, COUNTER, "c");
    for _ in 0..3 {
        clock.advance(Duration::from_millis(100));
        cmd(
            &mut s,
            "send",
            json!({ "to": a, "event": { "type": "inc" } }),
        );
    }
    let then = cmd(&mut s, "asOf", json!({ "address": a, "position": 1 }));
    assert_eq!(then["context"]["n"], json!(1.0));
    assert_eq!(then["position"], json!(1));
    let by_time = cmd(&mut s, "asOf", json!({ "address": a, "time": 1_250 }));
    assert_eq!(by_time["context"]["n"], json!(2.0));
    let now = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(
        now["context"]["n"],
        json!(3.0),
        "the live actor is unchanged"
    );
}

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Graph query over history]] {level: integration}
#[test]
fn graph_query_over_history() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, clock) = file_system(&dir.path().join("h.db"), 0);
    let a = spawn(&mut s, ORDER, "o");
    // tiramemsu stamps transactions with its own wall clock, not the shell's
    let before = now_ms();
    std::thread::sleep(Duration::from_millis(20));
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "pay" } }),
    );
    let between = now_ms();
    std::thread::sleep(Duration::from_millis(20));
    clock.advance(Duration::from_secs(2));
    s.pump();
    let q = format!(
        "SELECT ?s WHERE {{ GRAPH <{a}> {{ <{a}> <{p}> ?s }} }}",
        p = vertep_runtime::vocab::IN_STATE
    );
    let states = |s: &mut System, time: Option<i64>| -> Vec<String> {
        let mut args = json!({ "sparql": q });
        if let Some(t) = time {
            args["time"] = json!(t);
        }
        let r = cmd(s, "query", args);
        r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row["s"]
                    .as_str()
                    .unwrap()
                    .rsplit('.')
                    .next()
                    .unwrap()
                    .trim_end_matches('>')
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(states(&mut s, None), vec!["shipped"]);
    assert_eq!(states(&mut s, Some(between)), vec!["paid"]);
    assert!(states(&mut s, Some(before)).is_empty());
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Timer fires through the event loop]] {level: integration}
#[test]
fn timer_fires_through_the_event_loop() {
    let (mut s, clock) = memory(10_000);
    let a = spawn(&mut s, ORDER, "o");
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "pay" } }),
    );
    let timers = cmd(&mut s, "pendingTimers", json!({}));
    assert_eq!(timers[0]["due"], json!(12_000), "the timer is journaled");
    assert_eq!(s.next_deadline(), Some(12_000));
    clock.advance(Duration::from_millis(1_999));
    s.pump();
    assert_eq!(config(&mut s, &a), vec!["paid"], "not before two seconds");
    clock.advance(Duration::from_millis(1));
    s.pump();
    assert_eq!(config(&mut s, &a), vec!["shipped"]);
    let j = cmd(&mut s, "journal", json!({ "address": a }));
    let tick = &j[1];
    assert_eq!(tick["event"]["name"], json!("ship"));
    assert_eq!(tick["timestamp"], json!(12_000), "the journaled timestamp");
    assert!(tick["timer"].is_string());
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Due timers fire after reopening]] {level: integration}
#[test]
fn due_timers_fire_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let a = {
        let (mut s, _) = file_system(&path, 1_000);
        let a = spawn(&mut s, ORDER, "o");
        cmd(
            &mut s,
            "send",
            json!({ "to": a, "event": { "type": "pay" } }),
        );
        a
    };
    // reopened long after the timer was due
    let (mut s, _) = file_system(&path, 60_000);
    cmd(
        &mut s,
        "register",
        json!({ "source": ORDER, "type": ORDER_TYPE }),
    );
    s.pump();
    s.pump();
    let j = cmd(&mut s, "journal", json!({ "address": a }));
    let ships = j
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["name"] == json!("ship"))
        .count();
    assert_eq!(ships, 1, "fired exactly once: {j}");
    assert_eq!(config(&mut s, &a), vec!["shipped"]);
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Step never reads the clock]] {level: integration}
#[test]
fn step_never_reads_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.db");
    let (a, before) = {
        let (mut s, clock) = file_system(&path, 1_000);
        let a = spawn(&mut s, COUNTER, "c");
        for e in ["inc", "inc", "go"] {
            clock.advance(Duration::from_millis(7));
            cmd(&mut s, "send", json!({ "to": a, "event": { "type": e } }));
        }
        let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
        (a, snap)
    };
    // replayed years later on another wall clock
    let (mut s, _) = file_system(&path, 4_000_000_000_000);
    cmd(
        &mut s,
        "register",
        json!({ "source": COUNTER, "type": ORDER_TYPE }),
    );
    let after = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(before, after);
    for p in 1..=3 {
        let x = cmd(&mut s, "asOf", json!({ "address": a, "position": p }));
        assert_eq!(x["position"], json!(p));
    }
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Second writer is refused]] {level: integration}
#[test]
fn second_writer_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("l.db");
    let (mut first, _) = file_system(&path, 0);
    let a = spawn(&mut first, COUNTER, "c");
    cmd(
        &mut first,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    let opts = options(StorageKind::File, path.to_str().unwrap(), true);
    let e = Store::open(&opts.store, None).unwrap_err();
    assert_eq!(e.code, Code::StoreLocked);
    cmd(
        &mut first,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    let snap = cmd(&mut first, "snapshot", json!({ "address": a }));
    assert_eq!(
        snap["context"]["n"],
        json!(2.0),
        "the first holder is unaffected"
    );
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Durable journal survives reload]] {level: integration}
#[test]
fn durable_journal_survives_reload() {
    // natively the durable store is a file; in a browser it is the same tiramemsu file in OPFS
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.db");
    let (a, before) = {
        let (mut s, _) = file_system(&path, 0);
        let a = spawn(&mut s, COUNTER, "c");
        cmd(
            &mut s,
            "send",
            json!({ "to": a, "event": { "type": "inc" } }),
        );
        cmd(
            &mut s,
            "send",
            json!({ "to": a, "event": { "type": "go" } }),
        );
        let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
        (a, snap)
    };
    let (mut s, _) = file_system(&path, 0);
    cmd(
        &mut s,
        "register",
        json!({ "source": COUNTER, "type": ORDER_TYPE }),
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "done" } }),
    );
    let after = cmd(&mut s, "asOf", json!({ "address": a, "position": 2 }));
    assert_eq!(before, after);
}

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Unavailable OPFS is an error]] {level: unit}
#[test]
fn unavailable_opfs_is_an_error() {
    let opts = options(StorageKind::Opfs, "/vertep/app.db", false);
    let e = Store::open(&opts.store, None).unwrap_err();
    assert_eq!(e.code, Code::StorageUnavailable);
    let e = SystemOptions::from_json(&json!({ "path": "x.db" })).unwrap_err();
    assert_eq!(e.code, Code::InvalidArgument, "there is no default storage");
}

// @tg: verifies:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript#Can reflects enabled transitions]] {level: integration}
#[test]
fn can_reflects_enabled_transitions() {
    let deferring = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" initial="idle" datamodel="cel">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="idle">
    <transition event="go" target="busy"/>
    <transition event="big" cond="ctx.n &gt; 1" target="busy"/>
    <transition event="inc"><assign location="ctx.n" expr="ctx.n + 1"/></transition>
  </state>
  <state id="busy" vt:defer="order.*"><transition event="done" target="idle"/></state>
</scxml>"#;
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, deferring, "c");
    let can = |s: &mut System, e: &str| {
        cmd(s, "can", json!({ "address": a, "event": { "type": e } }))["can"].clone()
    };
    assert_eq!(
        can(&mut s, "go"),
        json!(false),
        "no configuration before the first entry"
    );
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    assert_eq!(can(&mut s, "go"), json!(true));
    assert_eq!(can(&mut s, "big"), json!(false), "guard false");
    assert_eq!(can(&mut s, "order.new"), json!(false));
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "inc" } }),
    );
    assert_eq!(can(&mut s, "big"), json!(true), "guard true");
    let journal_len = cmd(&mut s, "journal", json!({ "address": a }))
        .as_array()
        .unwrap()
        .len();
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" } }),
    );
    assert_eq!(can(&mut s, "order.new"), json!(true), "deferred");
    assert_eq!(can(&mut s, "go"), json!(false));
    assert_eq!(
        cmd(&mut s, "journal", json!({ "address": a }))
            .as_array()
            .unwrap()
            .len(),
        journal_len + 1,
        "can journals nothing"
    );
    let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(snap["context"]["n"], json!(2.0));
}

// @tg: verifies:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript#Document order decides conflicts]] {level: integration}
#[test]
fn document_order_decides_conflicts() {
    let chart = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
  <parallel id="p">
    <state id="a"><transition event="e" target="x"/></state>
    <state id="b"><transition event="e" target="y"/></state>
  </parallel>
  <state id="x"/><state id="y"/>
</scxml>"#;
    let (mut s, _) = memory(0);
    let a = spawn(&mut s, chart, "c");
    cmd(&mut s, "send", json!({ "to": a, "event": { "type": "e" } }));
    // both transitions exit the parallel state: they conflict and the first in document order
    // wins, exactly as the core trace (the native runtime) says
    assert_eq!(config(&mut s, &a), vec!["x"]);
    let case = vertep_wasm::TraceCase {
        name: "conflict".into(),
        chart: chart.into(),
        format: None,
        start_ms: 0,
        inputs: vec![vertep_wasm::TraceInput {
            name: "e".into(),
            data: J::Null,
            now_ms: 0,
            seed: 0,
        }],
    };
    let t = vertep_wasm::trace(&case, &vertep_core::null::NullRegistry);
    assert_eq!(t.steps.last().unwrap().configuration, vec!["x"]);
}

struct Impure {
    calls: std::cell::Cell<u32>,
}

impl Implementations for Impure {
    fn guard(&self, _iri: &str, _c: &J, _e: &J) -> Result<bool, String> {
        // true on the first call only: replay disagrees with the live step
        let n = self.calls.get();
        self.calls.set(n + 1);
        Ok(n == 0)
    }
    fn action(&self, _iri: &str, c: &J, _e: &J) -> Result<ActionOutcome, String> {
        let n = c["n"].as_f64().unwrap_or(0.0);
        ActionOutcome::from_json(&json!({ "context": { "n": n + 1.0 } }))
    }
}

const RUST_CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="rust">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="idle">
    <transition event="go" cond="vt:guard/test/lucky" target="done">
      <foreach array="vt:action/test/bump" item="_vt_emit">
        <send eventexpr="_vt_emit.event" targetexpr="_vt_emit.target" typeexpr="_vt_emit.type" delayexpr="_vt_emit.delay"><content expr="_vt_emit.data"/></send>
      </foreach>
    </transition>
  </state>
  <state id="done"/>
</scxml>"#;

fn with_impls(dev: bool) -> System {
    let mut opts = options(StorageKind::Memory, "m.db", false);
    opts.dev = dev;
    opts.guards = vec!["vt:guard/test/lucky".into()];
    opts.actions = vec!["vt:action/test/bump".into()];
    let store = Store::open(&opts.store, None).unwrap();
    let (shell, _) = Shell::virtual_time(0);
    let imp: Rc<dyn Implementations> = Rc::new(Impure {
        calls: std::cell::Cell::new(0),
    });
    System::new(&opts, store, shell, Some(imp)).unwrap()
}

// @tg: verifies:: [[openspec:wasm-bindings#Worker-Hosted Actor System#Implementations load inside the worker]] {level: unit}
#[test]
fn implementations_are_called_synchronously_in_the_step() {
    let mut s = with_impls(false);
    let a = spawn(&mut s, RUST_CHART, "r");
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" } }),
    );
    let snap = cmd(&mut s, "snapshot", json!({ "address": a }));
    assert_eq!(snap["configuration"], json!(["done"]));
    assert_eq!(snap["context"]["n"], json!(1.0));
}

#[test]
fn development_mode_reports_impure_guards() {
    let mut s = with_impls(true);
    let a = spawn(&mut s, RUST_CHART, "r");
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" } }),
    );
    let n = s.take_notifications();
    assert!(
        n.iter().any(|x| x["kind"] == json!("impurity")),
        "expected an impurity report: {n:?}"
    );
    let mut s = with_impls(false);
    let a = spawn(&mut s, RUST_CHART, "r");
    cmd(
        &mut s,
        "send",
        json!({ "to": a, "event": { "type": "go" } }),
    );
    assert!(
        s.take_notifications().is_empty(),
        "off outside development mode"
    );
}

#[test]
fn events_read_flat_or_with_data() {
    let e = event_from_json(&json!({ "type": "item.add", "sku": "A-17", "qty": 2 })).unwrap();
    assert_eq!(e.name, "item.add");
    assert_eq!(
        vertep_wasm::functions::value_to_json(&e.data),
        json!({ "sku": "A-17", "qty": 2.0 })
    );
    let e = event_from_json(&json!({ "type": "n", "data": 5 })).unwrap();
    assert_eq!(vertep_wasm::functions::value_to_json(&e.data), json!(5.0));
    assert!(event_from_json(&json!({ "data": 1 })).is_err());
}
