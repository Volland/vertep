//! The durable agent journal: agents journaled in a `JournalStore` survive restarts and fork
//! from storage.

mod common;

use std::sync::Arc;

use common::{AGENT, T0, cel, ev};
use vertep_agent::decider::{HumanDecider, Inbox};
use vertep_agent::host::AgentHost;
use vertep_agent::llm::ScriptedProvider;
use vertep_agent::{AgentInput, AgentJournal, StoreAgentJournal};
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;
use vertep_runtime::VirtualClock;
use vertep_store::JournalStore;
use vertep_tiramemsu::TiramemsuShard;

const HEAD: &str =
    r#"<datamodel><data id="summary" expr="''"/><data id="n" expr="0"/></datamodel>"#;

/// `start` runs one model call, then waits in the decide state `review` for a human.
fn agent_chart() -> Chart {
    cel(
        "idle",
        HEAD,
        r#"<state id="idle">
    <transition event="start" target="summarising"/>
    <transition event="tick"><assign location="n" expr="ctx.n + 1"/></transition>
  </state>
  <state id="summarising">
    <invoke type="vt:llm" id="summarise" vt:provider="p" vt:model="m1" vt:template="Summarise."/>
    <transition event="done.invoke.summarise" target="review"><assign location="summary" expr="ev.data.text"/></transition>
  </state>
  <state id="review" vt:decide="human" vt:candidates="approve reject">
    <transition event="approve" target="approved"/>
    <transition event="reject" target="rejected"/>
  </state>
  <state id="approved"/><state id="rejected"/>"#,
    )
}

fn host_on(
    store: Arc<dyn JournalStore>,
    provider: Arc<ScriptedProvider>,
    inbox: Inbox,
) -> AgentHost {
    let clock = VirtualClock::new(T0);
    AgentHost::builder(Arc::new(clock))
        .journal(Arc::new(StoreAgentJournal::new(store)))
        .provider("p", provider)
        .decider("human", Arc::new(HumanDecider::new(inbox)))
        .build()
}

// @tg: verifies:: [[openspec:durable-objects#Durable Agent Journal#Agent survives a restart]] {level: integration}
#[test]
fn agent_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agents.db");
    let correlation;
    {
        let store: Arc<dyn JournalStore> = Arc::new(TiramemsuShard::open(&path).unwrap());
        let provider = Arc::new(ScriptedProvider::new());
        provider.push_text("a short summary", 10, 4);
        let inbox = Inbox::default();
        let mut host = host_on(store, provider.clone(), inbox.clone());
        let v = host.register(agent_chart()).unwrap();
        host.spawn(AGENT, &v).unwrap();
        host.send(AGENT, ev("start"));
        host.run_until_idle();
        assert_eq!(provider.calls(), 1);
        let pending = inbox.pending();
        assert_eq!(pending.len(), 1, "the human was asked once");
        correlation = pending[0].correlation.clone();
    } // the process "restarts": host, provider and store handle are dropped

    let store: Arc<dyn JournalStore> = Arc::new(TiramemsuShard::open(&path).unwrap());
    let provider = Arc::new(ScriptedProvider::new());
    let inbox = Inbox::default();
    let mut host = host_on(store, provider.clone(), inbox.clone());
    host.register(agent_chart()).unwrap();
    let state = host.state(AGENT).unwrap();
    assert_eq!(provider.calls(), 0, "replay never calls a provider");
    assert!(state.configuration.iter().any(|s| s == "review"));
    assert_eq!(
        vertep_agent::ContextView::new(state.datamodel.clone()).get("summary"),
        Some(&Value::String("a short summary".into()))
    );
    let offers = host.offers(AGENT);
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].correlation, correlation, "the same open offer");
    // The decision answered after the restart is accepted against that offer.
    host.run_until_idle();
    assert!(inbox.answer(
        &correlation,
        vertep_agent::DecisionOutcome::Decision(vertep_agent::Decision::new(
            "approve",
            Value::Null
        ))
    ));
    host.run_until_idle();
    let state = host.state(AGENT).unwrap();
    assert!(state.configuration.iter().any(|s| s == "approved"));
}

// @tg: verifies:: [[openspec:durable-objects#Durable Agent Journal#Fork from stored journal]] {level: integration}
#[test]
fn fork_from_stored_journal() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn JournalStore> =
        Arc::new(TiramemsuShard::open(dir.path().join("agents.db")).unwrap());
    let provider = Arc::new(ScriptedProvider::new());
    let mut host = host_on(store.clone(), provider, Inbox::default());
    let v = host.register(agent_chart()).unwrap();
    host.spawn(AGENT, &v).unwrap();
    for _ in 0..14 {
        host.send(AGENT, ev("tick"));
        host.run_until_idle();
    }
    let at = 12;
    let original = host.state_at(AGENT, at).unwrap();
    let forked = "https://example.org/agents/a1-fork";
    host.fork(AGENT, at, forked, Vec::new()).unwrap();
    let fork = host.state(forked).unwrap();
    assert_eq!(fork.configuration, original.configuration);
    // Equal contexts; only the session id names the fork.
    let fork_ctx = vertep_agent::ContextView::new(fork.datamodel.clone());
    let original_ctx = vertep_agent::ContextView::new(original.datamodel.clone());
    assert_eq!(fork_ctx.data(), original_ctx.data());
    assert!(format!("{:?}", fork_ctx.get("n")).contains("11"));
    // The fork is in the store too.
    let journal = StoreAgentJournal::new(store);
    assert_eq!(journal.try_read(forked).unwrap().len() as u64, at);
    assert!(matches!(
        journal.try_read(forked).unwrap()[0].input,
        AgentInput::Start { .. }
    ));
}

#[test]
fn unreadable_store_is_an_error_not_an_empty_journal() {
    let store: Arc<dyn JournalStore> = Arc::new(vertep_store::MemoryStore::new());
    let journal = StoreAgentJournal::new(store);
    assert!(journal.try_read("not an iri").is_err());
}
