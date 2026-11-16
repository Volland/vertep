//! Seal policies derived from a chart's sensitive-field marks (`journal-redaction`, Sensitive
//! Fields): `@sensitive`, `@subject` and `@retain` in the `vt:types` block.

use std::sync::Arc;

use vertep_agent::journal::{AgentInput, AgentJournal, JournalEntry, MemoryJournal};
use vertep_agent::redaction::{MemoryKeyStore, SealPolicy, SealingJournal, SensitiveField};
use vertep_core::datamodel::{Event, Value};
use vertep_core::ir::Chart;

fn chart(types: &str) -> Chart {
    let doc = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="idle">
  <vt:types><![CDATA[{types}]]></vt:types>
  <state id="idle"><transition event="applied" target="done"/></state>
  <final id="done"/>
</scxml>"#
    );
    let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
    vertep_scxml::read(&doc, &opts)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .chart
}

const MARKED: &str = "type Contact = { @sensitive phone: string, city: string }; \
     event applied: { @subject who: string, @sensitive @retain(P30D) email: string, \
                      amount: decimal, contact: optional<Contact> }; \
     event reviewed: { case: { @subject holder: iri, @sensitive note: string }, by: string }; \
     event plain: { amount: decimal }; \
     event start;";

fn field(event: &str, field: &str, subject: &str, retention_ms: Option<i64>) -> SensitiveField {
    SensitiveField {
        event: event.into(),
        field: field.into(),
        subject: subject.into(),
        retention_ms,
    }
}

#[test]
fn a_marked_chart_declares_its_seal_policy() {
    let policy = SealPolicy::from_chart(&chart(MARKED)).unwrap();
    assert_eq!(
        policy.fields,
        vec![
            field("applied", "/email", "/who", Some(30 * 86_400_000)),
            field("applied", "/contact/phone", "/who", None),
            field("reviewed", "/case/note", "/case/holder", None),
        ]
    );
    // An untyped chart, or one without marks, declares an empty policy.
    let untyped = vertep_scxml::read_str(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"/></scxml>"#,
    )
    .unwrap()
    .chart;
    assert_eq!(SealPolicy::from_chart(&untyped), Ok(SealPolicy::default()));
    assert_eq!(
        SealPolicy::from_chart(&chart("event applied: { who: string };")),
        Ok(SealPolicy::default())
    );
}

#[test]
fn marks_a_pointer_cannot_reach_are_refused() {
    for (types, needle) in [
        (
            "event applied: { @sensitive email: string };",
            "has no `@subject`",
        ),
        (
            "event applied: { @subject who: string, items: list<{ @sensitive e: string }> };",
            "inside a list",
        ),
        (
            "event applied: { @subject who: string, pay: Card { @sensitive pan: string } | Cash };",
            "inside a union variant",
        ),
        (
            "type N = { @subject id: string, @sensitive e: string, next: optional<N> }; event applied: N;",
            "recursive type `N`",
        ),
        (
            "event applied: { @subject who: int, @sensitive e: string };",
            "a `string` or an `iri`",
        ),
    ] {
        let errs = SealPolicy::from_chart(&chart(types)).expect_err(types);
        assert!(errs.iter().any(|e| e.contains(needle)), "{types}: {errs:?}");
    }
    // Marking the whole list sensitive is the way out.
    let ok = SealPolicy::from_chart(&chart(
        "event applied: { @subject who: string, @sensitive items: list<{ e: string }> };",
    ))
    .unwrap();
    assert_eq!(ok.fields, vec![field("applied", "/items", "/who", None)]);
}

fn applied(position: u64) -> JournalEntry {
    use vertep_agent::json::{obj, s};
    let data = obj([
        ("who", s("ann")),
        ("email", s("ann@example.org")),
        ("amount", Value::Number(10.0)),
        (
            "contact",
            obj([("phone", s("+43 1 234")), ("city", s("Vienna"))]),
        ),
    ]);
    JournalEntry {
        position,
        ts: 1_000,
        input: AgentInput::Event {
            event: Event {
                name: "applied".into(),
                data,
                ..Event::default()
            },
            message_id: format!("m{position}"),
        },
        disposition: None,
        offered: None,
    }
}

// @tg: verifies:: [[openspec:journal-redaction#Sensitive Fields#A marked field is sealed]] {level: integration}
#[test]
fn a_journal_sealing_by_the_chart_policy_seals_the_marked_fields() {
    let policy = SealPolicy::from_chart(&chart(MARKED)).unwrap();
    let raw = Arc::new(MemoryJournal::new());
    let keys = Arc::new(MemoryKeyStore::new());
    let j = SealingJournal::new(raw.clone(), keys, policy).unwrap();
    j.append("urn:a", applied(1)).unwrap();

    let stored = serde_json::to_string(&raw.read("urn:a")).unwrap();
    assert!(!stored.contains("ann@example.org") && !stored.contains("+43 1 234"));
    assert!(stored.contains("Vienna"), "unmarked fields stay plain");
    assert_eq!(stored.matches("$sealed").count(), 2);

    let read = j.read("urn:a");
    let AgentInput::Event { event, .. } = &read[0].input else {
        panic!()
    };
    assert_eq!(
        vertep_agent::json::field(&event.data, "email"),
        Some(&vertep_agent::json::s("ann@example.org"))
    );
    // The e-mail's horizon is 30 days after the entry; when it passes the subject is erased.
    let horizon = 1_000 + 30 * 86_400_000;
    assert!(j.expire(horizon - 1).unwrap().is_empty());
    assert_eq!(j.expire(horizon).unwrap(), vec!["ann"]);
}
