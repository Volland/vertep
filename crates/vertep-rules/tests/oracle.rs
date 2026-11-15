//! The ELPI engine and the Datalog engine agree on a Horn program
//! ([[openspec:rule-verification#Engines Agree On Horn Programs]]).
#![cfg(feature = "native")]

use tiramemsu::{
    DatalogOptions, DatalogProgram, Db, ElpiOptions, ElpiProgram, ElpiTerm, OpenOptions, TxOptions,
    Valid, Value,
};

/// The node name inside a printed IRI, so the two engines' term wrappers do not matter.
fn node(printed: String) -> String {
    printed
        .split("urn:tiramemsu:v:")
        .nth(1)
        .unwrap_or("")
        .split('"')
        .next()
        .unwrap_or("")
        .to_owned()
}

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

// @tg: implements:: [[openspec:rule-verification#Engines Agree On Horn Programs]] {note: "the requirement is a differential test, so the test is the implementation"}
// @tg: verifies:: [[openspec:rule-verification#Engines Agree On Horn Programs#Agreement on random data]] {level: integration}
#[test]
fn the_elpi_engine_and_the_datalog_engine_agree_on_a_horn_join() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("o.db"), OpenOptions::default()).unwrap();
    db.transact(TxOptions::default(), |t| {
        let mut x: u64 = 11;
        for _ in 0..60 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let (a, b) = ((x >> 33) % 15, (x >> 17) % 15);
            t.assert(
                v(&format!("n{a}")),
                v("parent"),
                v(&format!("n{b}")),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    })
    .unwrap();
    let dl = db
        .now()
        .datalog(
            &DatalogProgram::Text("grand(X, Z) :- v:parent(X, Y), v:parent(Y, Z).".into()),
            Some("grand(X, Z)"),
            &DatalogOptions::default(),
        )
        .unwrap();
    let mut want: Vec<(String, String)> = dl
        .rows
        .iter()
        .map(|r| {
            (
                node(format!("{:?}", r.values[0])),
                node(format!("{:?}", r.values[1])),
            )
        })
        .collect();
    want.sort();
    want.dedup();
    let program = r#"
pred grand o:term, o:term.
grand X Z :- tm.triple X (iri "urn:tiramemsu:v:parent") Y _, tm.triple Y (iri "urn:tiramemsu:v:parent") Z _.
"#;
    let r = db
        .now()
        .elpi(
            &ElpiProgram::Text(program.into()),
            "grand X Z",
            &ElpiOptions::default(),
        )
        .unwrap();
    let mut got: Vec<(String, String)> = r
        .rows
        .iter()
        .filter_map(|row| match (row.bindings.get("X"), row.bindings.get("Z")) {
            (Some(ElpiTerm::Value(a)), Some(ElpiTerm::Value(b))) => {
                Some((node(format!("{a:?}")), node(format!("{b:?}"))))
            }
            _ => None,
        })
        .collect();
    got.sort();
    got.dedup();
    assert!(!want.is_empty(), "the graph has two-step paths");
    assert_eq!(got, want);
}
