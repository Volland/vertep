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
        outcome: None,
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
        outcome: None,
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

// ----- replay after erasure -------------------------------------------------------------

use vertep_agent::host::AgentHost;

/// A guard reads the sealed `amount` of `applied`, and the branch it chose stores the amount,
/// which a later guard reads again.
const GUARDED: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="cel" initial="idle">
  <datamodel><data id="amount" expr="0"/></datamodel>
  <state id="idle">
    <transition event="applied" cond="ev.data.amount &gt; 1000" target="review">
      <assign location="amount" expr="ev.data.amount"/>
    </transition>
    <transition event="applied" target="auto"/>
  </state>
  <state id="review">
    <transition event="next" cond="ctx.amount &gt; 5000" target="senior"/>
    <transition event="next" target="junior"/>
  </state>
  <state id="auto"/><state id="senior"/><state id="junior"/>
</scxml>"#;

/// Marks `amount` of `applied` sensitive, with `who` as the subject.
fn amount_policy() -> SealPolicy {
    SealPolicy {
        fields: vec![SensitiveField {
            event: "applied".into(),
            field: "/amount".into(),
            subject: "/who".into(),
            retention_ms: None,
        }],
    }
}

fn host_on(journal: Arc<dyn AgentJournal>, chart: &str, now: i64) -> AgentHost {
    let mut host = AgentHost::builder(Arc::new(vertep_runtime::VirtualClock::new(now)))
        .journal(journal)
        .build();
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    host.register(vertep_scxml::read(chart, &opts).unwrap().chart)
        .unwrap();
    host
}

/// Runs the guarded chart for `ann` with an amount of 8000 (review, then senior) on a sealing
/// journal, and returns the raw journal, the key store and the sealing journal.
fn run_guarded() -> (Arc<MemoryJournal>, Arc<MemoryKeyStore>, Arc<SealingJournal>) {
    let raw = Arc::new(MemoryJournal::new());
    let keys = Arc::new(MemoryKeyStore::new());
    let j = Arc::new(SealingJournal::new(raw.clone(), keys.clone(), amount_policy()).unwrap());
    let mut host = host_on(j.clone(), GUARDED, 1_000);
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    let v = host
        .register(vertep_scxml::read(GUARDED, &opts).unwrap().chart)
        .unwrap();
    host.spawn("urn:a", &v).unwrap();
    host.send(
        "urn:a",
        Event {
            name: "applied".into(),
            data: vertep_agent::json::obj([
                ("who", vertep_agent::json::s("ann")),
                ("amount", Value::Number(8_000.0)),
            ]),
            ..Event::default()
        },
    );
    host.run_until_idle();
    host.send("urn:a", Event::named("next", Default::default()));
    host.run_until_idle();
    assert_eq!(host.state("urn:a").unwrap().configuration, vec!["senior"]);
    assert!(
        !serde_json::to_string(&raw.read("urn:a"))
            .unwrap()
            .contains("8000")
    );
    (raw, keys, j)
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Replay completes with a marker]] {level: integration}
#[test]
fn after_erasure_a_fresh_host_reaches_the_recorded_states_marked_recorded() {
    let (raw, _, j) = run_guarded();
    // each entry the host journaled records what it did, and none of it is the sealed value
    let stored = raw.read("urn:a");
    let outcome = stored[1].outcome.as_ref().expect("an outcome is recorded");
    assert_eq!(outcome.configuration, vec!["review"]);
    assert_eq!(outcome.guards, "t");
    assert_eq!(stored[2].outcome.as_ref().unwrap().guards, "t");

    j.erase("ann", "request", 2_000).unwrap();
    let mut audit = host_on(j.clone(), GUARDED, 3_000);
    // reactivation after erasure: the guards now read `{"$erased": "ann"}`, and the replay
    // follows what they answered when the entries were journaled
    let state = audit.state("urn:a").unwrap();
    assert_eq!(state.configuration, vec!["senior"]);
    // the branch's executable content ran: the stored amount is the erased marker
    let ctx = audit.machine("urn:a").unwrap().context();
    assert_eq!(
        ctx.get("amount.$erased"),
        Some(&vertep_agent::json::s("ann"))
    );
    for (position, after) in [(2, "review"), (3, "senior")] {
        let x = audit.explain("urn:a", position).unwrap();
        assert_eq!(
            (x.replay, x.state_after),
            ("recorded", vec![after.to_owned()])
        );
    }
    assert_eq!(audit.explain("urn:a", 1).unwrap().replay, "verified");
    let report = audit.replay_report("urn:a", None).unwrap();
    assert!(report.erased && report.disagreed && !report.forced);
    // entries 2 and 3; the erasure entry changes nothing and has no outcome
    assert_eq!((report.followed, report.unrecorded), (2, 0));
    assert_eq!(report.marker(), "recorded");
    // the actor goes on from the recorded state
    audit.send("urn:a", Event::named("next", Default::default()));
    audit.run_until_idle();
    assert_eq!(audit.state("urn:a").unwrap().configuration, vec!["senior"]);
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Unaffected journals stay verified]] {level: integration}
#[test]
fn a_journal_without_erasure_replays_verified() {
    let (_, _, j) = run_guarded();
    let audit = host_on(j, GUARDED, 3_000);
    for position in 1..=3 {
        assert_eq!(audit.explain("urn:a", position).unwrap().replay, "verified");
    }
    let report = audit.replay_report("urn:a", None).unwrap();
    assert_eq!(report, vertep_agent::host::ReplayReport::default());
    assert_eq!(
        audit.state_at("urn:a", 3).unwrap().configuration,
        vec!["senior"]
    );
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Unaffected journals stay verified]] {level: integration}
#[test]
fn an_erased_field_no_guard_reads_leaves_replay_verified() {
    const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="cel" initial="idle">
      <state id="idle"><transition event="applied" target="done"/></state><state id="done"/></scxml>"#;
    let raw = Arc::new(MemoryJournal::new());
    let keys = Arc::new(MemoryKeyStore::new());
    let j = Arc::new(SealingJournal::new(raw, keys, policy(None)).unwrap());
    let mut host = host_on(j.clone(), CHART, 1_000);
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
    let audit = host_on(j, CHART, 3_000);
    let x = audit.explain("urn:a", 2).unwrap();
    assert_eq!(x.state_after, vec!["done"]);
    // the email is erased, but no condition was evaluated after it, so none could read it
    assert_eq!(x.replay, "verified");
    let report = audit.replay_report("urn:a", None).unwrap();
    assert!(report.erased);
    assert_eq!(report.guards_after_erasure, 0);
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Replay completes with a marker]] {level: integration}
#[test]
fn an_old_entry_without_an_outcome_replays_by_re_evaluation_marked_recorded() {
    let (raw, keys, j) = run_guarded();
    j.erase("ann", "request", 2_000).unwrap();
    // the same journal as written before outcomes were recorded
    let old = Arc::new(MemoryJournal::new());
    for mut e in raw.read("urn:a") {
        e.outcome = None;
        let text = e.to_json();
        assert!(!text.contains("outcome"));
        old.append("urn:a", JournalEntry::from_json(&text).unwrap())
            .unwrap();
    }
    let j = Arc::new(SealingJournal::new(old, keys, amount_policy()).unwrap());
    let mut audit = host_on(j, GUARDED, 3_000);
    // nothing to follow: the guard is re-evaluated on the erased value and takes the other
    // branch, which the marker owns up to; the replay does not fail
    let state = audit.state("urn:a").unwrap();
    assert_eq!(state.configuration, vec!["auto"]);
    let report = audit.replay_report("urn:a", None).unwrap();
    assert_eq!((report.followed, report.marker()), (0, "recorded"));
    assert!(report.unrecorded > 0);
    assert_eq!(audit.explain("urn:a", 2).unwrap().replay, "recorded");
}

// @tg: verifies:: [[openspec:journal-redaction#Replay After Erasure#Replay completes with a marker]] {level: integration}
#[test]
fn a_record_that_does_not_line_up_still_reaches_the_recorded_configuration() {
    let (raw, keys, j) = run_guarded();
    j.erase("ann", "request", 2_000).unwrap();
    // the guard answers of entry 2 are missing: the replay re-evaluates, ends in `auto`, and
    // the configuration is then set to the recorded `review`
    let thin = Arc::new(MemoryJournal::new());
    for mut e in raw.read("urn:a") {
        if e.position == 2
            && let Some(o) = e.outcome.as_mut()
        {
            o.guards.clear();
        }
        thin.append("urn:a", e).unwrap();
    }
    let j = Arc::new(SealingJournal::new(thin, keys, amount_policy()).unwrap());
    let audit = host_on(j, GUARDED, 3_000);
    assert_eq!(
        audit.state_at("urn:a", 2).unwrap().configuration,
        vec!["review"]
    );
    let report = audit.replay_report("urn:a", Some(2)).unwrap();
    assert!(report.forced && report.disagreed);
    assert_eq!(report.marker(), "recorded");
}
