//! Hash-chained, signed agent journals ([[openspec:decision-records#Agent Journal Integrity]]).

use std::sync::Arc;

use oxrdf::NamedNode;
use vertep_agent::decider::GuardDecider;
use vertep_agent::host::AgentHost;
use vertep_agent::journal::StoreAgentJournal;
use vertep_core::datamodel::{Event, EventType};
use vertep_objects::testkit::{StaticHostKeys, TestHost, turn};
use vertep_objects::{
    AgentWriter, ChainHead, ChainLink, ChainedAgentSealer, FailureReason, read_link, seal,
    verify_agent_journal, verify_chain,
};
use vertep_runtime::VirtualClock;
use vertep_store::{AppendMeta, JournalRecord, JournalStore, MemoryStore, Position};

const ACTOR: &str = "https://example.org/agents/a1";
const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
  version="1.0" datamodel="cel" initial="idle">
  <datamodel><data id="level" expr="1"/></datamodel>
  <state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.level > 3" target="approved"/>
    <transition event="reject" target="rejected"/>
  </state>
  <state id="approved"/><state id="rejected"/>
</scxml>"#;

fn actor() -> NamedNode {
    NamedNode::new(ACTOR).unwrap()
}

/// An agent that decides once, on a journal sealed by `host`; returns the store.
fn run(store: Arc<MemoryStore>, host: &TestHost) -> AgentHost {
    let journal = Arc::new(StoreAgentJournal::with_sealer(
        store,
        Arc::new(ChainedAgentSealer::new(Arc::new(host.signer.clone()))),
    ));
    let mut agent = AgentHost::builder(Arc::new(VirtualClock::new(1_794_355_200_000)))
        .journal(journal)
        .decider("d", Arc::new(GuardDecider::first_enabled(["reject"])))
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let chart = vertep_scxml::read(CHART, &opts).unwrap().chart;
    let v = agent.register(chart).unwrap();
    agent.spawn(ACTOR, &v).unwrap();
    agent.send(ACTOR, Event::named("start", EventType::External));
    agent.run_until_idle();
    agent
}

fn keys(host: &TestHost) -> AgentWriter {
    AgentWriter {
        did: host.did.clone(),
        keys: vec![host.public],
    }
}

// @tg: verifies:: [[openspec:decision-records#Agent Journal Integrity#A chained journal verifies]] {level: integration}
#[test]
fn a_chained_agent_journal_verifies_and_the_agent_still_works() {
    let host = TestHost::new("did:key:z6MkAgentHost");
    let store = Arc::new(MemoryStore::new());
    let mut agent = run(store.clone(), &host);
    let head = verify_agent_journal(&*store, &actor(), &keys(&host)).unwrap();
    assert_eq!(head.position, Position::new(3), "start, event, decision");
    assert!(head.hash.is_some());
    // the explanation view reads the same journal through the sealed entries
    assert_eq!(agent.state(ACTOR).unwrap().configuration, vec!["rejected"]);
    let x = agent.explain(ACTOR, 3).unwrap();
    assert_eq!(x.decision.unwrap().disposition, "accepted");
    // a second host on the same store reads the sealed entries back after a restart
    let journal = Arc::new(StoreAgentJournal::with_sealer(
        store.clone(),
        Arc::new(ChainedAgentSealer::new(Arc::new(host.signer.clone()))),
    ));
    let mut again = AgentHost::builder(Arc::new(VirtualClock::new(1_794_355_200_000)))
        .journal(journal)
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    again
        .register(vertep_scxml::read(CHART, &opts).unwrap().chart)
        .unwrap();
    assert_eq!(again.state(ACTOR).unwrap().configuration, vec!["rejected"]);
    assert!(verify_agent_journal(&*store, &actor(), &keys(&host)).is_ok());
}

// @tg: verifies:: [[openspec:decision-records#Agent Journal Integrity#A reordered entry is detected]] {level: integration}
#[test]
fn a_reordered_or_altered_entry_is_detected_at_the_first_affected_position() {
    let host = TestHost::new("did:key:z6MkAgentHost");
    let store = Arc::new(MemoryStore::new());
    let _ = run(store.clone(), &host);
    let mut records = store.read(&actor(), Position::ZERO).unwrap();
    assert_eq!(records.len(), 3);
    // swap the entries at positions 2 and 3, leaving the positions where they were
    let (a, b) = (records[1].entry.clone(), records[2].entry.clone());
    records[1] = JournalRecord {
        position: Position::new(2),
        entry: b,
    };
    records[2] = JournalRecord {
        position: Position::new(3),
        entry: a,
    };
    let failure = verify_chain(
        &actor(),
        ChainHead::EMPTY,
        &records,
        &keys(&TestHost::new("x")).into_keys(&host),
    )
    .unwrap_err();
    assert_eq!(failure.position, Position::new(2));
    assert!(
        matches!(failure.reason, FailureReason::Malformed(_)),
        "{failure}"
    );

    // an entry whose content was changed: the hash no longer matches
    let honest = store.read(&actor(), Position::ZERO).unwrap();
    let mut changed = honest.clone();
    let name = changed[2].entry.name().clone();
    let tampered = oxrdf::Triple::new(
        name.clone(),
        NamedNode::new_unchecked(vertep_agent::journal::AGENT_INPUT),
        oxrdf::Literal::new_simple_literal(
            "{\"position\":3,\"ts\":1,\"input\":{\"kind\":\"timer\",\"iri\":\"x\"}}",
        ),
    );
    let triples = changed[2]
        .entry
        .triples()
        .iter()
        .filter(|t| t.predicate.as_str() != vertep_agent::journal::AGENT_INPUT)
        .map(|t| t.into_owned())
        .chain([tampered]);
    changed[2].entry = vertep_store::Entry::new(name, triples);
    let failure = verify_chain(
        &actor(),
        ChainHead::EMPTY,
        &changed,
        &keys(&host).into_keys(&host),
    )
    .unwrap_err();
    assert_eq!(failure.position, Position::new(3));
    assert_eq!(failure.reason, FailureReason::HashMismatch);
}

// @tg: verifies:: [[openspec:decision-records#Agent Journal Integrity#Promotion keeps history]] {level: integration}
#[test]
fn promotion_to_an_object_keeps_the_history_verifying_under_the_objects_verifier() {
    let agent_host = TestHost::new("did:key:z6MkAgentHost");
    let object_host = TestHost::new("did:key:z6MkObjectHost");
    let store = Arc::new(MemoryStore::new());
    let _ = run(store.clone(), &agent_host);
    let before = store.read(&actor(), Position::ZERO).unwrap();
    let head = verify_agent_journal(&*store, &actor(), &keys(&agent_host)).unwrap();

    // the object's own writer continues the same chain in a later epoch
    let next = seal(
        &turn(&actor(), 4, "promoted"),
        &ChainLink {
            object: actor(),
            position: Position::new(4),
            epoch: 2,
            writer: object_host.did.clone(),
            prev: head.hash,
        },
        &object_host.signer,
    )
    .unwrap();
    store
        .append(&actor(), Position::new(3), &[next], &AppendMeta::none())
        .unwrap();

    let all = store.read(&actor(), Position::ZERO).unwrap();
    let object_keys = StaticHostKeys::new()
        .with(&agent_host, 1)
        .with(&object_host, 2);
    let end = verify_chain(&actor(), ChainHead::EMPTY, &all, &object_keys).unwrap();
    assert_eq!((end.position, end.epoch), (Position::new(4), 2));
    // the agent's entries are the same bytes as before the promotion
    assert_eq!(&all[..3], &before[..]);
    assert_eq!(read_link(&all[3].entry).unwrap().link.prev, head.hash);
}

#[test]
fn a_journal_that_did_not_opt_in_is_not_chained_and_is_unchanged() {
    let store = Arc::new(MemoryStore::new());
    let journal = Arc::new(StoreAgentJournal::new(store.clone()));
    let mut agent = AgentHost::builder(Arc::new(VirtualClock::new(1)))
        .journal(journal)
        .decider("d", Arc::new(GuardDecider::first_enabled(["reject"])))
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let v = agent
        .register(vertep_scxml::read(CHART, &opts).unwrap().chart)
        .unwrap();
    agent.spawn(ACTOR, &v).unwrap();
    agent.send(ACTOR, Event::named("start", EventType::External));
    agent.run_until_idle();
    let host = TestHost::new("did:key:z6MkAgentHost");
    let failure = verify_agent_journal(&*store, &actor(), &keys(&host)).unwrap_err();
    assert!(matches!(failure.reason, FailureReason::Malformed(_)));
    assert_eq!(agent.state(ACTOR).unwrap().configuration, vec!["rejected"]);
}

/// Host keys for a verifier that trusts `host` in every epoch.
trait IntoKeys {
    fn into_keys(self, host: &TestHost) -> StaticHostKeys;
}
impl IntoKeys for AgentWriter {
    fn into_keys(self, host: &TestHost) -> StaticHostKeys {
        StaticHostKeys::new().with(host, 1)
    }
}

// @tg: verifies:: [[openspec:journal-redaction#Erasure By Key Destruction#Erased data is unreadable]] {level: integration}
// @tg: verifies:: [[openspec:journal-redaction#Sensitive Fields#A marked field is sealed]] {level: integration}
#[test]
fn erasing_a_subject_keeps_the_signed_chain_valid() {
    use vertep_agent::journal::{AgentInput, AgentJournal, JournalEntry};
    use vertep_agent::redaction::{MemoryKeyStore, SealPolicy, SealingJournal, SensitiveField};

    let host = TestHost::new("did:key:z6MkAgentHost");
    let store = Arc::new(MemoryStore::new());
    let chained = Arc::new(StoreAgentJournal::with_sealer(
        store.clone(),
        Arc::new(ChainedAgentSealer::new(Arc::new(host.signer.clone()))),
    ));
    let journal = SealingJournal::new(
        chained,
        Arc::new(MemoryKeyStore::new()),
        SealPolicy {
            fields: vec![SensitiveField {
                event: "applied".into(),
                field: "/email".into(),
                subject: "/who".into(),
                retention_ms: None,
            }],
        },
    )
    .unwrap();
    let entry = |position, input| JournalEntry {
        position,
        ts: 1_794_355_200_000,
        input,
        disposition: None,
        offered: None,
        outcome: None,
    };
    journal
        .append(
            ACTOR,
            entry(
                1,
                AgentInput::Start {
                    version: "urn:v".into(),
                },
            ),
        )
        .unwrap();
    let mut applied = Event::named("applied", EventType::External);
    applied.data = vertep_agent::json::obj([
        ("who", vertep_agent::json::s("ann")),
        ("email", vertep_agent::json::s("ann@example.org")),
    ]);
    journal
        .append(
            ACTOR,
            entry(
                2,
                AgentInput::Event {
                    event: applied,
                    message_id: "m1".into(),
                },
            ),
        )
        .unwrap();

    let before = verify_agent_journal(&*store, &actor(), &keys(&host)).unwrap();
    assert_eq!(before.position, Position::new(2));
    let stored = format!("{:?}", store.read(&actor(), Position::ZERO).unwrap());
    assert!(
        !stored.contains("ann@example.org"),
        "only ciphertext is stored"
    );

    journal.erase("ann", "request", 1_794_355_300_000).unwrap();

    // the erasure is one more link; every hash and signature still checks
    let after = verify_agent_journal(&*store, &actor(), &keys(&host)).unwrap();
    assert_eq!(after.position, Position::new(3));
    let read = journal.read(ACTOR);
    assert!(matches!(read[2].input, AgentInput::Erasure { .. }));
    assert!(journal.erasures(ACTOR).subjects.contains("ann"));
}
