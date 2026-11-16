//! Vectors for `journal-redaction#Erasure Retracts Derived Facts`. The vectors that
//! materialize through a rule program need the native engine library (see `tests/invariants.rs`);
//! the one that writes derived facts by hand runs without it.

use std::sync::Arc;

use vertep_agent::journal::{AgentInput, AgentJournal, JournalEntry, MemoryJournal};
use vertep_agent::redaction::{MemoryKeyStore, SealPolicy, SealingJournal, SensitiveField};
use vertep_core::datamodel::Event;
use vertep_rules::{RetractOnErasure, retract_erased};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{Eid, TxOptions, Valid, Value};

const ANN: &str = "https://example.org/people/ann";
const BOB: &str = "https://example.org/people/bob";

fn int(n: i64) -> Value {
    Value::literal(
        &n.to_string(),
        Some("http://www.w3.org/2001/XMLSchema#integer"),
        None,
    )
}

/// The live statements with predicate `p`, as `(eid, subject)`.
fn live(s: &TiramemsuShard, p: &str) -> Vec<(String, Value)> {
    let v = s.db().now();
    let Some(pid) = v.encode(&Value::iri(p)).unwrap() else {
        return Vec::new();
    };
    v.triples(None, Some(pid), None)
        .unwrap()
        .into_iter()
        .map(|t| (t.eid.to_string(), v.decode(t.s).unwrap()))
        .collect()
}

fn subjects(s: &TiramemsuShard, p: &str) -> Vec<Value> {
    live(s, p).into_iter().map(|(_, s)| s).collect()
}

/// An applicant with an amount of 80 000 and an open flag, plus a loan that names them.
/// Returns the eids of the amount and the flag.
fn applicant(s: &TiramemsuShard, who: &str, loan: &str) -> [Eid; 2] {
    let mut out = Vec::new();
    s.db()
        .transact(TxOptions::default(), |t| {
            let p = Value::iri(who);
            out.push(
                t.assert(
                    p.clone(),
                    Value::iri("urn:vt:amount"),
                    int(80_000),
                    Valid::ALWAYS,
                )?
                .eid(),
            );
            out.push(
                t.assert(
                    p.clone(),
                    Value::iri("urn:vt:flag"),
                    Value::str("open"),
                    Valid::ALWAYS,
                )?
                .eid(),
            );
            t.assert(
                Value::iri(loan),
                Value::iri("urn:vt:applicant"),
                p,
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    [out[0], out[1]]
}

/// A journal that seals the applicant's email, with the hook over `shard`.
fn journal(shard: &Arc<TiramemsuShard>) -> (Arc<MemoryJournal>, SealingJournal) {
    let raw = Arc::new(MemoryJournal::new());
    let policy = SealPolicy {
        fields: vec![SensitiveField {
            event: "applied".into(),
            field: "/email".into(),
            subject: "/who".into(),
            retention_ms: None,
        }],
    };
    let j = SealingJournal::new(raw.clone(), Arc::new(MemoryKeyStore::new()), policy)
        .unwrap()
        .with_erasure_hook(Arc::new(RetractOnErasure::new(shard.clone())));
    (raw, j)
}

fn applied(who: &str) -> JournalEntry {
    let data = vertep_agent::json::obj([
        ("who", vertep_agent::json::s(who)),
        ("email", vertep_agent::json::s("someone@example.org")),
    ]);
    JournalEntry {
        position: 1,
        ts: 1_000,
        input: AgentInput::Event {
            event: Event {
                name: "applied".into(),
                data,
                ..Event::default()
            },
            message_id: "m1".into(),
        },
        disposition: None,
        outcome: None,
        offered: None,
    }
}

fn erasure_of(raw: &MemoryJournal, actor: &str) -> (String, Vec<String>) {
    match &raw.read(actor).last().unwrap().input {
        AgentInput::Erasure {
            subject, retracted, ..
        } => (subject.clone(), retracted.clone()),
        other => panic!("not an erasure: {other:?}"),
    }
}

/// Without the engine: facts derived by hand with stated support fall with the erased subject's
/// statements, transitively, the erasure entry lists them, and facts resting on another
/// subject's statements stay, even one about the erased subject.
#[test]
fn hand_derived_facts_fall_with_the_erased_subjects_statements() {
    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    let ann = applicant(&shard, ANN, "https://example.org/loans/1");
    let bob = applicant(&shard, BOB, "https://example.org/loans/2");
    let (risk, review, about_ann) = {
        let (mut risk, mut review, mut about_ann) = (Vec::new(), Vec::new(), None);
        shard
            .db()
            .transact(TxOptions::default(), |t| {
                for (who, support) in [(ANN, ann), (BOB, bob)] {
                    let r = t
                        .derive(
                            Value::iri(who),
                            Value::iri("urn:vt:risk"),
                            Value::iri("urn:vt:high"),
                            Valid::ALWAYS,
                            &support,
                        )?
                        .eid();
                    // a second layer: a fact resting on a derived fact
                    review.push(
                        t.derive(
                            Value::iri(who),
                            Value::iri("urn:vt:review"),
                            Value::iri("urn:vt:yes"),
                            Valid::ALWAYS,
                            &[r],
                        )?
                        .eid(),
                    );
                    risk.push(r);
                }
                // a fact about ann that rests only on bob's statements
                about_ann = Some(
                    t.derive(
                        Value::iri(ANN),
                        Value::iri("urn:vt:peer"),
                        Value::iri(BOB),
                        Valid::ALWAYS,
                        &[bob[0]],
                    )?
                    .eid(),
                );
                Ok(())
            })
            .unwrap();
        (risk, review, about_ann.unwrap())
    };

    let (raw, j) = journal(&shard);
    j.append("urn:actor:ann", applied(ANN)).unwrap();
    j.append("urn:actor:bob", applied(BOB)).unwrap();
    assert_eq!(j.erase(ANN, "request", 2_000).unwrap(), ["urn:actor:ann"]);

    let (subject, retracted) = erasure_of(&raw, "urn:actor:ann");
    assert_eq!(subject, ANN);
    // in id order: the risk fact, then the review resting on it
    assert_eq!(retracted, [risk[0].to_string(), review[0].to_string()]);
    assert_eq!(subjects(&shard, "urn:vt:risk"), [Value::iri(BOB)]);
    assert_eq!(subjects(&shard, "urn:vt:review"), [Value::iri(BOB)]);
    assert_eq!(
        live(&shard, "urn:vt:peer"),
        [(about_ann.to_string(), Value::iri(ANN))]
    );
    // the subject's own statements, and the loan that names them, are retracted too
    assert_eq!(subjects(&shard, "urn:vt:amount"), [Value::iri(BOB)]);
    assert_eq!(
        subjects(&shard, "urn:vt:applicant"),
        [Value::iri("https://example.org/loans/2")]
    );
    // bob's journal got no entry; a subject the shard does not know retracts nothing
    assert_eq!(raw.read("urn:actor:bob").len(), 1);
    assert_eq!(
        retract_erased(&shard, "https://example.org/people/nobody").unwrap(),
        Default::default()
    );
}

#[cfg(feature = "native")]
mod engine {
    use super::*;
    use vertep_rules::{materialize, put_program};

    const RISK: &str = "urn:vt:program:risk";
    const REVIEW: &str = "urn:vt:program:review";

    /// A person with an amount over 50 000 and an open flag is high risk.
    const RISK_RULES: &str = r#"
tm.conclude (triple C (iri "urn:vt:risk") (iri "urn:vt:high")) [E1, E2] :-
  tm.triple C (iri "urn:vt:amount") (num A) E1, A > 50000,
  tm.triple C (iri "urn:vt:flag") (str "open") E2.
"#;

    /// A high-risk person is reviewed: a derived fact resting on a derived fact.
    const REVIEW_RULES: &str = r#"
tm.conclude (triple C (iri "urn:vt:review") (iri "urn:vt:yes")) [E] :-
  tm.triple C (iri "urn:vt:risk") (iri "urn:vt:high") E.
"#;

    fn setup() -> (tempfile::TempDir, Arc<TiramemsuShard>) {
        let dir = tempfile::tempdir().unwrap();
        let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
        put_program(&shard, RISK, RISK_RULES).unwrap();
        put_program(&shard, REVIEW, REVIEW_RULES).unwrap();
        applicant(&shard, ANN, "https://example.org/loans/1");
        applicant(&shard, BOB, "https://example.org/loans/2");
        assert_eq!(materialize(&shard, RISK, 100_000).unwrap().derived.len(), 2);
        assert_eq!(
            materialize(&shard, REVIEW, 100_000).unwrap().derived.len(),
            2
        );
        (dir, shard)
    }

    fn eid_of(s: &TiramemsuShard, p: &str, who: &str) -> String {
        live(s, p)
            .into_iter()
            .find(|(_, s)| *s == Value::iri(who))
            .map(|(e, _)| e)
            .unwrap()
    }

    // @tg: verifies:: [[openspec:journal-redaction#Erasure Retracts Derived Facts#Erasing a subject]] {level: integration}
    #[test]
    fn erasing_a_subject_retracts_the_facts_on_their_statements_and_lists_them() {
        let (_dir, shard) = setup();
        // in id order: the risk fact, then the review resting on it
        let want = [
            eid_of(&shard, "urn:vt:risk", ANN),
            eid_of(&shard, "urn:vt:review", ANN),
        ];
        let (raw, j) = journal(&shard);
        j.append("urn:actor:ann", applied(ANN)).unwrap();
        j.erase(ANN, "request", 2_000).unwrap();

        let (subject, retracted) = erasure_of(&raw, "urn:actor:ann");
        assert_eq!(subject, ANN);
        assert_eq!(retracted, want, "the erasure entry lists both layers");
        assert!(!subjects(&shard, "urn:vt:risk").contains(&Value::iri(ANN)));
        assert!(!subjects(&shard, "urn:vt:review").contains(&Value::iri(ANN)));

        // the retraction is a recorded change: the store's history still holds the fact
        let history = shard.db().history();
        let p = history.encode(&Value::iri("urn:vt:risk")).unwrap().unwrap();
        assert_eq!(history.triples(None, Some(p), None).unwrap().len(), 2);

        // and the premises are gone, so materializing again does not bring it back
        let again = materialize(&shard, RISK, 100_000).unwrap();
        assert!(again.derived.is_empty(), "{again:?}");
        assert!(!subjects(&shard, "urn:vt:risk").contains(&Value::iri(ANN)));
    }

    // @tg: verifies:: [[openspec:journal-redaction#Erasure Retracts Derived Facts#Unrelated facts survive]] {level: integration}
    #[test]
    fn facts_on_other_subjects_statements_survive_an_erasure() {
        let (_dir, shard) = setup();
        let bob_risk = eid_of(&shard, "urn:vt:risk", BOB);
        let bob_review = eid_of(&shard, "urn:vt:review", BOB);
        let (raw, j) = journal(&shard);
        j.append("urn:actor:ann", applied(ANN)).unwrap();
        j.append("urn:actor:bob", applied(BOB)).unwrap();
        j.erase(ANN, "request", 2_000).unwrap();

        let (_, retracted) = erasure_of(&raw, "urn:actor:ann");
        assert!(!retracted.contains(&bob_risk) && !retracted.contains(&bob_review));
        assert_eq!(live(&shard, "urn:vt:risk"), [(bob_risk, Value::iri(BOB))]);
        assert_eq!(
            live(&shard, "urn:vt:review"),
            [(bob_review, Value::iri(BOB))]
        );
        let again = materialize(&shard, RISK, 100_000).unwrap();
        assert!(again.derived.is_empty() && again.existing == 1, "{again:?}");
    }
}
