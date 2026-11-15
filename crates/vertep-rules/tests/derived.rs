//! Vectors for `derived-facts`. They need the native engine library; see `tests/invariants.rs`.
#![cfg(feature = "native")]

use vertep_rules::{materialize, put_program};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value};

const PROGRAM: &str = "urn:vt:program:risk";

/// A case with an amount over 50 000 and an open flag is high risk, and the conclusion states
/// both statements as its support.
const RISK: &str = r#"
tm.conclude (triple C (iri "urn:vt:risk") (iri "urn:vt:high")) [E1, E2] :-
  tm.triple C (iri "urn:vt:amount") (num A) E1, A > 50000,
  tm.triple C (iri "urn:vt:flag") (str "open") E2.
"#;

fn int(n: i64) -> Value {
    Value::literal(
        &n.to_string(),
        Some("http://www.w3.org/2001/XMLSchema#integer"),
        None,
    )
}

fn live(s: &TiramemsuShard, p: &str) -> Vec<(String, String)> {
    let v = s.db().now();
    let Some(pid) = v.encode(&Value::iri(p)).unwrap() else {
        return Vec::new();
    };
    v.triples(None, Some(pid), None)
        .unwrap()
        .into_iter()
        .map(|t| (t.eid.to_string(), format!("{:?}", v.decode(t.o).unwrap())))
        .collect()
}

// @tg: verifies:: [[openspec:derived-facts#Derived Facts Carry Support#A risk flag with its cause]] {level: integration}
// @tg: verifies:: [[openspec:derived-facts#Derived Facts Retract With Support#Support retracted]] {level: integration}
// @tg: verifies:: [[openspec:derived-facts#Derived Facts Retract With Support#Program edited]] {level: integration}
#[test]
fn a_derived_fact_has_its_cause_and_falls_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = TiramemsuShard::open(dir.path().join("s.db")).unwrap();
    put_program(&s, PROGRAM, RISK).unwrap();
    let case = Value::iri("https://example.org/loans/1");
    let mut support = Vec::new();
    s.db()
        .transact(TxOptions::default(), |t| {
            support.push(
                t.assert(
                    case.clone(),
                    Value::iri("urn:vt:amount"),
                    int(80_000),
                    Valid::ALWAYS,
                )?
                .eid(),
            );
            support.push(
                t.assert(
                    case.clone(),
                    Value::iri("urn:vt:flag"),
                    Value::str("open"),
                    Valid::ALWAYS,
                )?
                .eid(),
            );
            Ok(())
        })
        .unwrap();

    // the conclusion appears, and each premise lists it among its dependents
    let m = materialize(&s, PROGRAM, 100_000).unwrap();
    assert_eq!(m.derived.len(), 1, "{m:?}");
    assert_eq!(live(&s, "urn:vt:risk").len(), 1);
    for e in &support {
        let dependents = s.db().now().dependents(*e).unwrap();
        assert!(
            dependents.iter().any(|d| d.to_string() == m.derived[0]),
            "{dependents:?}"
        );
    }

    // running again derives nothing new
    let again = materialize(&s, PROGRAM, 100_000).unwrap();
    assert!(again.derived.is_empty() && again.existing == 1, "{again:?}");

    // retracting a premise takes the conclusion with it
    s.db()
        .transact(TxOptions::default(), |t| {
            t.retract(support[1])?;
            Ok(())
        })
        .unwrap();
    assert!(
        live(&s, "urn:vt:risk").is_empty(),
        "the conclusion fell with its support"
    );

    // restore the premise, derive again, then edit the program so it no longer concludes
    s.db()
        .transact(TxOptions::default(), |t| {
            t.assert(
                case.clone(),
                Value::iri("urn:vt:flag"),
                Value::str("open"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(materialize(&s, PROGRAM, 100_000).unwrap().derived.len(), 1);
    put_program(&s, PROGRAM, &RISK.replace("A > 50000", "A > 90000")).unwrap();
    let after = materialize(&s, PROGRAM, 100_000).unwrap();
    assert!(live(&s, "urn:vt:risk").is_empty(), "{after:?}");
}
