//! An invariant enforced at commit: no entry may be appended for a blocked actor. The refusal
//! names the constraint and the statements, and nothing is written.
//!
//! Needs the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo run -p vertep-rules --features native --example invariant`

use oxrdf::{Literal, NamedNode, Triple};
use vertep_rules::{enforce, put_program};
use vertep_store::{AppendError, AppendMeta, Entry, JournalStore, Position};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value};

const PROGRAM: &str = "urn:vt:program:no-blocked-actor";
const INVARIANT: &str = r#"
tm.violation-delta "blocked_actor" "the actor is blocked" [E1, E2] :-
  tm.added _ (iri "https://w3id.org/vertep/vt#actor") A E1,
  tm.triple A (iri "urn:vt:status") (str "blocked") E2.
tm.violation-delta "blocked_actor" "the actor is blocked" [E1, E2] :-
  tm.added A (iri "urn:vt:status") (str "blocked") E2,
  tm.triple _ (iri "https://w3id.org/vertep/vt#actor") A E1.
tm.violation error "blocked_actor" "the actor is blocked" [E1, E2] :-
  tm.triple _ (iri "https://w3id.org/vertep/vt#actor") A E1,
  tm.triple A (iri "urn:vt:status") (str "blocked") E2.
"#;

fn entry(actor: &NamedNode, n: u64) -> Entry {
    let e = NamedNode::new(format!("{}/entry/{n}", actor.as_str())).expect("valid");
    let p = NamedNode::new("https://w3id.org/vertep/vt#eventName").expect("valid");
    Entry::new(
        e.clone(),
        [Triple::new(e, p, Literal::from(format!("event {n}")))],
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("vertep-rules-invariant-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let shard = TiramemsuShard::open(dir.join("shard.db"))?;
    put_program(&shard, PROGRAM, INVARIANT)?;
    enforce(&shard, PROGRAM, true)?;
    println!("enforcing {PROGRAM}");

    let fine = NamedNode::new("https://example.org/actors/fine")?;
    let blocked = NamedNode::new("https://example.org/actors/blocked")?;
    shard.db().transact(TxOptions::default(), |t| {
        t.assert(
            Value::iri(blocked.as_str()),
            Value::iri("urn:vt:status"),
            Value::str("blocked"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })?;
    println!("actors/blocked is marked blocked");

    for (name, actor) in [("fine", &fine), ("blocked", &blocked)] {
        match shard.append(
            actor,
            Position::ZERO,
            &[entry(actor, 0)],
            &AppendMeta::none(),
        ) {
            Ok(p) => println!("append for {name}: committed, journal at {}", p.get()),
            Err(AppendError::Refused { violations }) => println!(
                "append for {name}: refused by {} ({} statements); journal still at {}",
                violations[0].constraint,
                violations[0].statements.len(),
                shard.head(actor)?.get()
            ),
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
