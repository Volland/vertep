//! Sealed sensitive fields, erasure by key destruction and retention
//! (`journal-redaction`).

use std::sync::Arc;

use vertep_agent::journal::{AgentInput, AgentJournal, JournalEntry, MemoryJournal};
use vertep_agent::redaction::{
    KeyStore, MemoryKeyStore, SealPolicy, SealingJournal, SensitiveField,
};
use vertep_core::datamodel::{Event, Value};

fn policy(retention_ms: Option<i64>) -> SealPolicy {
    SealPolicy {
        fields: vec![SensitiveField {
            event: "applied".into(),
            field: "/email".into(),
            subject: "/who".into(),
            retention_ms,
        }],
    }
}

fn applied(position: u64, who: &str, email: &str) -> JournalEntry {
    let data = vertep_agent::json::obj([
        ("who", vertep_agent::json::s(who)),
        ("email", vertep_agent::json::s(email)),
        ("amount", Value::Number(10.0)),
    ]);
    let event = Event {
        name: "applied".into(),
        data,
        ..Event::default()
    };
    JournalEntry {
        position,
        ts: 1_000,
        input: AgentInput::Event {
            event,
            message_id: format!("m{position}"),
        },
        disposition: None,
        offered: None,
    }
}

fn start() -> JournalEntry {
    JournalEntry {
        position: 1,
        ts: 1_000,
        input: AgentInput::Start {
            version: "urn:v".into(),
        },
        disposition: None,
        offered: None,
    }
}

fn email_of(e: &JournalEntry) -> Option<Value> {
    match &e.input {
        AgentInput::Event { event, .. } => vertep_agent::json::field(&event.data, "email").cloned(),
        _ => None,
    }
}

fn setup(retention: Option<i64>) -> (Arc<MemoryJournal>, Arc<MemoryKeyStore>, SealingJournal) {
    let raw = Arc::new(MemoryJournal::new());
    let keys = Arc::new(MemoryKeyStore::new());
    let j = SealingJournal::new(raw.clone(), keys.clone(), policy(retention)).unwrap();
    j.append("urn:a", start()).unwrap();
    j.append("urn:a", applied(2, "ann", "ann@example.org"))
        .unwrap();
    j.append("urn:a", applied(3, "bob", "bob@example.org"))
        .unwrap();
    (raw, keys, j)
}

// @tg: verifies:: [[openspec:journal-redaction#Sensitive Fields#A marked field is sealed]] {level: unit}
#[test]
fn a_marked_field_is_stored_as_ciphertext() {
    let (raw, _, _) = setup(None);
    let stored = raw.read("urn:a");
    let text = serde_json::to_string(&stored).unwrap();
    assert!(!text.contains("ann@example.org") && !text.contains("bob@example.org"));
    assert!(text.contains("$sealed"));
    // unmarked fields stay plain
    assert!(text.contains("\"amount\""));
}

// @tg: verifies:: [[openspec:journal-redaction#Sensitive Fields#An authorised reader sees it]] {level: unit}
#[test]
fn a_reader_with_the_key_store_sees_the_value() {
    let (_, _, j) = setup(None);
    let read = j.read("urn:a");
    assert_eq!(
        email_of(&read[1]),
        Some(vertep_agent::json::s("ann@example.org"))
    );
    assert_eq!(
        email_of(&read[2]),
        Some(vertep_agent::json::s("bob@example.org"))
    );
    assert!(j.erasures("urn:a").is_empty());
}

// @tg: verifies:: [[openspec:journal-redaction#Erasure By Key Destruction#Erased data is unreadable]] {level: unit}
// @tg: verifies:: [[openspec:journal-redaction#Erasure By Key Destruction#Erasure is recorded]] {level: unit}
// @tg: verifies:: [[openspec:journal-redaction#Erasure By Key Destruction#Other subjects are unaffected]] {level: unit}
#[test]
fn erasing_a_subject_destroys_their_key_and_records_it() {
    let (raw, keys, j) = setup(None);
    let before = raw.read("urn:a");
    let touched = j.erase("ann", "request", 5_000).unwrap();
    assert_eq!(touched, vec!["urn:a".to_owned()]);
    assert!(keys.key("ann").is_none());

    let read = j.read("urn:a");
    // ann's field is unreadable, bob's is untouched
    let ann = email_of(&read[1]).unwrap();
    assert_eq!(
        vertep_agent::json::field(&ann, "$erased"),
        Some(&vertep_agent::json::s("ann"))
    );
    assert_eq!(
        email_of(&read[2]),
        Some(vertep_agent::json::s("bob@example.org"))
    );
    // an erasure entry names the subject and the time, appended; nothing was rewritten
    let AgentInput::Erasure {
        subject, reason, ..
    } = &read[3].input
    else {
        panic!("{:?}", read[3].input)
    };
    assert_eq!(
        (subject.as_str(), reason.as_str(), read[3].ts),
        ("ann", "request", 5_000)
    );
    assert_eq!(&raw.read("urn:a")[..3], &before[..]);
    assert_eq!(
        j.erasures("urn:a").subjects.into_iter().collect::<Vec<_>>(),
        vec!["ann"]
    );

    // erasing again is a no-op on the journal
    j.erase("ann", "request", 6_000).unwrap();
    assert_eq!(raw.read("urn:a").len(), 4);
    // a new event for the erased subject cannot be sealed under a new key
    assert!(
        j.append("urn:a", applied(5, "ann", "new@example.org"))
            .is_err()
    );
}

// @tg: verifies:: [[openspec:journal-redaction#Retention Policy#A horizon expires]] {level: unit}
// @tg: verifies:: [[openspec:journal-redaction#Retention Policy#No policy, no expiry]] {level: unit}
#[test]
fn a_passed_horizon_erases_with_the_reason_retention() {
    let (_, _, j) = setup(Some(10_000));
    assert!(j.expire(10_999).unwrap().is_empty());
    assert_eq!(j.expire(11_000).unwrap(), vec!["ann", "bob"]);
    let read = j.read("urn:a");
    let erasures: Vec<_> = read
        .iter()
        .filter_map(|e| match &e.input {
            AgentInput::Erasure { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(erasures, vec!["retention", "retention"]);

    let (_, _, none) = setup(None);
    assert!(none.expire(i64::MAX).unwrap().is_empty());
}

#[test]
fn a_key_store_that_cannot_guarantee_destruction_is_refused() {
    struct Weak;
    impl KeyStore for Weak {
        fn key(&self, _: &str) -> Option<[u8; 32]> {
            None
        }
        fn obtain(&self, _: &str) -> Result<[u8; 32], String> {
            Ok([0; 32])
        }
        fn destroy(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn destruction_guaranteed(&self) -> bool {
            false
        }
    }
    let r = SealingJournal::new(Arc::new(MemoryJournal::new()), Arc::new(Weak), policy(None));
    assert!(r.is_err());
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Replay completes with a marker]] {level: integration}
// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Unaffected journals stay verified]] {level: integration}
#[test]
fn explaining_after_erasure_is_marked_recorded_and_otherwise_verified() {
    use vertep_agent::host::AgentHost;
    const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="cel" initial="idle">
      <state id="idle"><transition event="applied" target="done"/></state><state id="done"/></scxml>"#;
    let raw = Arc::new(MemoryJournal::new());
    let keys = Arc::new(MemoryKeyStore::new());
    let j = Arc::new(SealingJournal::new(raw, keys, policy(None)).unwrap());
    let mut host = AgentHost::builder(Arc::new(vertep_runtime::VirtualClock::new(1_000)))
        .journal(j.clone())
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let v = host
        .register(vertep_scxml::read(CHART, &opts).unwrap().chart)
        .unwrap();
    host.spawn("urn:a", &v).unwrap();
    let e = Event {
        name: "applied".into(),
        data: vertep_agent::json::obj([
            ("who", vertep_agent::json::s("ann")),
            ("email", vertep_agent::json::s("ann@example.org")),
        ]),
        ..Event::default()
    };
    host.send("urn:a", e);
    host.run_until_idle();
    assert_eq!(host.explain("urn:a", 2).unwrap().replay, "verified");

    j.erase("ann", "request", 2_000).unwrap();
    let mut audit = AgentHost::builder(Arc::new(vertep_runtime::VirtualClock::new(3_000)))
        .journal(j)
        .build();
    audit
        .register(vertep_scxml::read(CHART, &opts).unwrap().chart)
        .unwrap();
    let x = audit.explain("urn:a", 2).unwrap();
    assert_eq!(x.replay, "recorded");
    assert_eq!(x.state_after, vec!["done"]);
    // an entry before the erased value is still fully verified
    assert_eq!(audit.explain("urn:a", 1).unwrap().replay, "verified");
}
