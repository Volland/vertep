//! Property tests of the runtime's journal protocol with the Lean model as oracle
//! (verification-backlog 5.6).
//!
//! A generated schedule submits and redelivers messages to a toggle actor through two nodes that
//! share one journal store, each with its own durable mailbox store, pumps either node,
//! activates the actor on both (concurrent activations), and crashes nodes before or after an
//! append. After the schedule
//! both nodes restart and run until idle; the observation (the toggle's journal with the
//! configuration after each entry, the messages submitted, the sends its receiver accepted,
//! the messages left waiting) goes to `vertep-journal`, which accepts exactly the observations
//! the Lean journal protocol model admits (`Vertep.Journal.accepts_iff`,
//! `Vertep.Journal.reachable_admissible`): replay reproduces every entry, no message id is
//! journaled twice, exactly the recorded sends were delivered, and no message was lost.

use std::path::Path;
use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use vertep_conformance::LeanTools;
use vertep_core::datamodel::{Event, EventType};
use vertep_core::ir::Chart;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, CrashPoint, Envelope, Node, NodeConfig, StoreMailboxes};
use vertep_store::MemoryStore;

const REQUIRED: bool = cfg!(feature = "differential");

/// 2026-11-11T00:00:00Z.
const T0: i64 = 1_794_355_200_000;

fn ty(name: &str) -> String {
    format!("https://example.org/charts/{name}")
}

fn chart(initial: &str, body: &str) -> Chart {
    let xml = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="{initial}">{body}</scxml>"#
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts).expect("chart reads").chart
}

fn toggle() -> Address {
    Address::virtual_actor(&ty("toggle"), "k").unwrap()
}

fn sink() -> Address {
    Address::virtual_actor(&ty("sink"), "log").unwrap()
}

/// A node on the shared journal store with its mailboxes in `inbox`; `crash` arms a crash point
/// for its next turn.
fn node(
    store: &Arc<MemoryStore>,
    inbox: &Arc<MemoryStore>,
    clock: &vertep_runtime::VirtualClock,
    crash: Option<CrashPoint>,
) -> Node {
    let shell = vertep_runtime::Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock.clone()),
    };
    let mut n = Node::builder(store.clone(), shell)
        .mailboxes(Arc::new(StoreMailboxes::new(inbox.clone())))
        .config(NodeConfig {
            crash_at: crash,
            ..NodeConfig::default()
        })
        .build();
    n.register_type(
        &ty("sink"),
        chart("s", r#"<state id="s"><transition event="*"/></state>"#),
    )
    .unwrap();
    // `go` toggles and sends one event to the sink; anything else changes nothing (the
    // Lean `toggle` machine).
    let send = format!(r#"<send event="ping" target="{}"/>"#, sink());
    n.register_type(
        &ty("toggle"),
        chart(
            "idle",
            &format!(
                r#"<state id="idle"><transition event="go" target="busy">{send}</transition></state>
                   <state id="busy"><transition event="go" target="idle">{send}</transition></state>"#
            ),
        ),
    )
    .unwrap();
    n
}

/// One move of a schedule.
#[derive(Clone, Debug)]
enum Op {
    /// Submit a new message (`go` or `noop`) through node `via`.
    Submit { via: usize, go: bool },
    /// Submit an earlier message again (picked by index) through node `via`.
    Redeliver {
        via: usize,
        pick: prop::sample::Index,
    },
    /// One scheduling round on a node.
    Pump(usize),
    /// Activate the toggle on a node (with the other's, two concurrent activations).
    Activate(usize),
    /// Restart a node, arming a crash point for its next turn.
    Crash(usize, bool),
    /// Restart a node.
    Restart(usize),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..2usize, prop::bool::weighted(0.8)).prop_map(|(via, go)| Op::Submit { via, go }),
        2 => (0..2usize, any::<prop::sample::Index>()).prop_map(|(via, pick)| Op::Redeliver { via, pick }),
        6 => (0..2usize).prop_map(Op::Pump),
        2 => (0..2usize).prop_map(Op::Activate),
        2 => (0..2usize, any::<bool>()).prop_map(|(n, before)| Op::Crash(n, before)),
        1 => (0..2usize).prop_map(Op::Restart),
    ]
}

/// Run a schedule; the observation as the JSON `vertep-journal` reads. With `shared_inbox` both
/// nodes keep their durable mailboxes in the journal store too.
fn run(ops: &[Op], shared_inbox: bool) -> serde_json::Value {
    let store = Arc::new(MemoryStore::new());
    let inbox = if shared_inbox {
        [store.clone(), store.clone()]
    } else {
        [Arc::new(MemoryStore::new()), Arc::new(MemoryStore::new())]
    };
    let clock = vertep_runtime::VirtualClock::new(T0);
    let fresh = |i: usize, crash| node(&store, &inbox[i], &clock, crash);
    let mut nodes = [fresh(0, None), fresh(1, None)];
    let a = toggle();
    let mut submitted: Vec<(String, bool)> = Vec::new();
    for op in ops {
        match op {
            Op::Submit { via, go } => {
                let id = format!("urn:vertep:test:m{}", submitted.len());
                let name = if *go { "go" } else { "noop" };
                let env = Envelope::new(id.clone(), &a, Event::named(name, EventType::External));
                if nodes[*via].send_envelope(env).is_ok() {
                    submitted.push((id, *go));
                }
            }
            Op::Redeliver { via, pick } => {
                if submitted.is_empty() {
                    continue;
                }
                let (id, go) = pick.get(&submitted).clone();
                let name = if go { "go" } else { "noop" };
                let _ = nodes[*via].send_envelope(Envelope::new(
                    id,
                    &a,
                    Event::named(name, EventType::External),
                ));
            }
            Op::Pump(n) => {
                nodes[*n].pump();
            }
            Op::Activate(n) => {
                let _ = nodes[*n].activate_actor(&a);
            }
            Op::Crash(n, before) => {
                let p = if *before {
                    CrashPoint::BeforeAppend
                } else {
                    CrashPoint::AfterAppend
                };
                nodes[*n] = fresh(*n, Some(p));
            }
            Op::Restart(n) => nodes[*n] = fresh(*n, None),
        }
    }
    // Recovery: both nodes restart and run until neither has anything left to do.
    nodes = [fresh(0, None), fresh(1, None)];
    for _ in 0..16 {
        let progress = nodes[0].run_until_idle() + nodes[1].run_until_idle();
        if progress == 0 {
            break;
        }
    }
    observe(&mut nodes[0], &submitted)
}

fn observe(n: &mut Node, submitted: &[(String, bool)]) -> serde_json::Value {
    let a = toggle();
    let journal = n.journal(&a).expect("toggle journal reads");
    let mut entries = Vec::new();
    let mut sends: Vec<(String, (u64, usize))> = Vec::new();
    for e in &journal {
        let EntryBody::Input { input, effects } = &e.body else {
            panic!("only input entries: {:?}", e.body)
        };
        for (k, s) in effects.sends.iter().enumerate() {
            sends.push((s.iri.clone(), (e.position, k)));
        }
        let state = n.state_at(&a, e.position).expect("time travel");
        entries.push(serde_json::json!({
            "id": e.message_id,
            "input": input.event.name,
            "state": state.configuration.join(","),
            "sends": effects.sends.len(),
        }));
    }
    let delivered: Vec<serde_json::Value> = n
        .journal(&sink())
        .expect("sink journal reads")
        .iter()
        .map(|e| {
            let (p, k) = sends
                .iter()
                .find(|(iri, _)| *iri == e.message_id)
                .map_or((0, 0), |(_, x)| *x);
            serde_json::json!([p, k])
        })
        .collect();
    let mailbox: Vec<String> = n.mailbox(&a).into_iter().map(|e| e.message_id).collect();
    serde_json::json!({
        "journal": entries,
        "submitted": submitted.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        "delivered": delivered,
        "mailbox": mailbox,
    })
}

/// Ask `vertep-journal`; `Err` with its reason when the model does not admit the run.
fn judge(tools: &LeanTools, obs: &serde_json::Value) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "vertep-journal-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("obs.json");
    std::fs::write(&path, obs.to_string()).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(&tools.journal)
        .arg("--obs")
        .arg(&path)
        .output()
        .map_err(|e| format!("{}: {e}", tools.journal.display()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    match out.status.code() {
        Some(0) => Ok(()),
        _ => Err(format!(
            "{}\nobservation: {obs}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

fn tools() -> Option<LeanTools> {
    let t = LeanTools::for_test(REQUIRED)?;
    if Path::new(&t.journal).is_file() {
        Some(t)
    } else if REQUIRED {
        panic!(
            "{} is not built: run `cd lean/vertep-lean && lake build vertep-journal`",
            t.journal.display()
        )
    } else {
        eprintln!("skipped: {} is not built", t.journal.display());
        None
    }
}

fn check_schedules(tools: &LeanTools, cases: u32, shared_inbox: bool) {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases,
            failure_persistence: None,
            max_shrink_iters: 512,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[7u8; 32]),
    );
    let result = runner.run(&prop::collection::vec(arb_op(), 0..24), |ops| {
        judge(tools, &run(&ops, shared_inbox)).map_err(TestCaseError::fail)
    });
    if let Err(e) = result {
        panic!("the runtime's journal protocol disagrees with the Lean model: {e:?}");
    }
}

/// A schedule with every hazard, judged by the model.
// @tg: verifies:: [[openspec:event-journal#Transactional Outbox#Crash after append, before dispatch]] {level: integration}
#[test]
fn journal_protocol_crash_redelivery_concurrency_admitted() {
    let Some(tools) = tools() else { return };
    let ops = vec![
        Op::Submit { via: 0, go: true },
        Op::Activate(0),
        Op::Activate(1),
        Op::Pump(0),
        Op::Submit { via: 1, go: true },
        Op::Pump(1),
        Op::Crash(0, false),
        Op::Submit { via: 0, go: true },
        Op::Pump(0),
        Op::Crash(1, true),
        Op::Submit { via: 1, go: false },
        Op::Pump(1),
        Op::Pump(0),
    ];
    let obs = run(&ops, false);
    judge(&tools, &obs).unwrap();
    assert_eq!(obs["submitted"].as_array().unwrap().len(), 4);
}

/// The oracle rejects a run that loses a recorded send.
#[test]
fn journal_protocol_oracle_rejects_a_lost_send() {
    let Some(tools) = tools() else { return };
    let mut obs = run(&[Op::Submit { via: 0, go: true }, Op::Pump(0)], false);
    obs["delivered"] = serde_json::json!([]);
    let e = judge(&tools, &obs).expect_err("a lost send");
    assert!(e.contains("never delivered"), "{e}");
}

/// Generated schedules of submissions, redeliveries, pumps, concurrent activations and crashes
/// on two nodes: every run is one the Lean model admits. `VT_JOURNAL_CASES` sets the count.
// @tg: verifies:: [[openspec:event-journal#Message Deduplication#Redelivered message dropped]] {level: integration}
// @tg: verifies:: [[openspec:event-journal#Compare-And-Set Append#Conflicting append rejected]] {level: integration}
// @tg: verifies:: [[openspec:event-journal#Deterministic Replay#Replay equals recorded state]] {level: integration}
#[test]
fn journal_protocol_generated_schedules_admitted() {
    let Some(tools) = tools() else { return };
    let cases = std::env::var("VT_JOURNAL_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    check_schedules(&tools, cases, false);
}

/// Generated schedules where both nodes keep their durable mailboxes in the shared journal
/// store. Found by these schedules and fixed: `StoreMailbox` numbered its row IRIs by its own
/// sequence only, so two nodes wrote rows with the same IRI for different messages and
/// acknowledging one removed the other's `rdf:type` and recipient triples; after a restart the
/// other message was lost (the Lean model reported `a submitted message was lost` on
/// `[Submit { via: 1, go: false }, Redeliver { via: 0, .. }, Submit { via: 1, go: true },
/// Crash(1, true), Pump(0), Pump(1)]`). Row IRIs now carry the envelope's digest.
// @tg: verifies:: [[openspec:messaging#Mailboxes Outlive Activations#Durable mailbox survives a restart]] {level: integration}
#[test]
fn journal_protocol_shared_durable_mailboxes_lose_no_message() {
    let Some(tools) = tools() else { return };
    check_schedules(&tools, 48, true);
}
