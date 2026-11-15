//! Vectors for `rule-invariants`. They need the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo test -p vertep-rules --features native`.
#![cfg(feature = "native")]

use oxrdf::{NamedNode, Triple};
use vertep_rules::{RulesError, enforce, put_program};
use vertep_store::{AppendError, AppendMeta, Entry, JournalStore, Position};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{Db, ElpiOptions, ElpiProgram, TxOptions, Valid, Value};

const PROGRAM: &str = "urn:vt:program:no-blocked-actor";
const STATUS: &str = "urn:vt:status";

/// No entry may be appended for a blocked actor. The delta form is what runs at commit; the
/// full form is what runs when enforcement is turned on. Both say the same thing.
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

fn actor(i: u32) -> NamedNode {
    NamedNode::new(format!("https://example.org/actors/{i}")).unwrap()
}

fn entry(a: &NamedNode, n: u64) -> Entry {
    let e = NamedNode::new(format!("{}/entry/{n}", a.as_str())).unwrap();
    let p = NamedNode::new("https://w3id.org/vertep/vt#eventName").unwrap();
    Entry::new(
        e.clone(),
        [Triple::new(e, p, oxrdf::Literal::from(format!("ev{n}")))],
    )
}

fn append(s: &TiramemsuShard, a: &NamedNode, n: u64) -> Result<Position, AppendError> {
    s.append(a, Position::new(n), &[entry(a, n)], &AppendMeta::none())
}

fn set_status(
    s: &TiramemsuShard,
    a: &NamedNode,
    blocked: bool,
) -> Result<(), vertep_tiramemsu::tiramemsu::Error> {
    let (subject, status) = (Value::iri(a.as_str()), Value::iri(STATUS));
    let live: Vec<_> = {
        let view = s.db().now();
        match (view.encode(&subject)?, view.encode(&status)?) {
            (Some(sid), Some(pid)) => view
                .triples(Some(sid), Some(pid), None)?
                .into_iter()
                .map(|t| t.eid)
                .collect(),
            _ => Vec::new(),
        }
    };
    s.db()
        .transact(TxOptions::default(), |t| {
            if blocked {
                t.assert(
                    subject.clone(),
                    status.clone(),
                    Value::str("blocked"),
                    Valid::ALWAYS,
                )?;
            } else {
                for e in &live {
                    t.retract(*e)?;
                }
            }
            Ok(())
        })
        .map(|_| ())
}

fn shard() -> (tempfile::TempDir, TiramemsuShard) {
    let dir = tempfile::tempdir().unwrap();
    let s = TiramemsuShard::open(dir.path().join("s.db")).unwrap();
    (dir, s)
}

// @tg: verifies:: [[openspec:rule-invariants#Enforced Program Refuses A Violating Append#Approval without review]] {level: integration}
// @tg: verifies:: [[openspec:rule-invariants#Enforced Program Refuses A Violating Append#A conforming append]] {level: integration}
#[test]
fn an_enforced_program_refuses_a_violating_append_and_writes_nothing() {
    let (_d, s) = shard();
    put_program(&s, PROGRAM, INVARIANT).unwrap();
    enforce(&s, PROGRAM, true).unwrap();
    let ok = actor(1);
    let blocked = actor(2);
    set_status(&s, &blocked, true).unwrap();
    // a conforming append commits
    assert_eq!(append(&s, &ok, 0).unwrap(), Position::new(1));
    // a violating one is refused with the violation named, and nothing changes
    let before = s.read(&blocked, Position::ZERO).unwrap().len();
    let err = append(&s, &blocked, 0).unwrap_err();
    match &err {
        AppendError::Refused { violations } => {
            assert_eq!(violations[0].constraint, "blocked_actor");
            assert!(!violations[0].statements.is_empty());
        }
        e => panic!("{e:?}"),
    }
    assert!(err.is_refused());
    assert_eq!(s.read(&blocked, Position::ZERO).unwrap().len(), before);
    assert_eq!(s.head(&blocked).unwrap(), Position::ZERO);
    // once unblocked, the same append goes through at the same position
    set_status(&s, &blocked, false).unwrap();
    assert_eq!(append(&s, &blocked, 0).unwrap(), Position::new(1));
}

// @tg: verifies:: [[openspec:rule-invariants#Enabling Checks Existing Data#Live data violates]] {level: integration}
#[test]
fn enabling_enforcement_checks_existing_data() {
    let (_d, s) = shard();
    let a = actor(3);
    append(&s, &a, 0).unwrap();
    set_status(&s, &a, true).unwrap(); // no program yet, so nothing refuses this
    put_program(&s, PROGRAM, INVARIANT).unwrap();
    match enforce(&s, PROGRAM, true) {
        Err(RulesError::Violations(v)) => assert_eq!(v[0].constraint, "blocked_actor"),
        other => panic!("{other:?}"),
    }
    // enforcement stayed off: appends still go through
    assert!(append(&s, &a, 1).is_ok());
    // after the data is repaired, enabling works
    set_status(&s, &a, false).unwrap();
    enforce(&s, PROGRAM, true).unwrap();
}

/// A small deterministic generator: the test must repeat exactly.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

// @tg: verifies:: [[openspec:rule-invariants#Delta Check Agrees With Full Check#Random transactions]] {level: integration}
#[test]
fn the_delta_check_agrees_with_the_full_check_on_random_transactions() {
    // `live` enforces the program. For each operation a `twin` is rebuilt without enforcement
    // from the operations `live` accepted, the operation is applied to it, and its full check
    // says whether the operation adds a violation. `live` must refuse exactly then.
    let dir = tempfile::tempdir().unwrap();
    let live = TiramemsuShard::open(dir.path().join("live.db")).unwrap();
    put_program(&live, PROGRAM, INVARIANT).unwrap();
    enforce(&live, PROGRAM, true).unwrap();
    let mut accepted_ops: Vec<(u64, usize, u64)> = Vec::new(); // (op, actor, position)
    let mut heads = [0u64; 4];
    let mut rng = Lcg(7);
    let (mut refused, mut accepted) = (0, 0);
    for step in 0..60 {
        let i = rng.next(4) as usize;
        let a = actor(i as u32);
        let op = rng.next(3);
        let pos = heads[i];
        let twin = TiramemsuShard::open(dir.path().join(format!("twin{step}.db"))).unwrap();
        put_program(&twin, PROGRAM, INVARIANT).unwrap();
        let run = |s: &TiramemsuShard, op: u64, i: usize, pos: u64| -> Result<(), AppendError> {
            let a = actor(i as u32);
            match op {
                0 => append(s, &a, pos).map(|_| ()),
                1 => set_status(s, &a, true).map_err(into_append_error),
                _ => set_status(s, &a, false).map_err(into_append_error),
            }
        };
        for (o, ai, p) in &accepted_ops {
            run(&twin, *o, *ai, *p).unwrap();
        }
        let before = full_violations(twin.db());
        run(&twin, op, i, pos).unwrap();
        let would_violate = full_violations(twin.db()) > before;
        match (would_violate, run(&live, op, i, pos)) {
            (true, Err(AppendError::Refused { .. })) => refused += 1,
            (false, Ok(())) => {
                accepted += 1;
                accepted_ops.push((op, i, pos));
                if op == 0 {
                    heads[i] += 1;
                }
            }
            (v, r) => panic!(
                "step {step}: the full check says violate={v}, live said {r:?} (op {op}, actor {})",
                a.as_str()
            ),
        }
    }
    assert!(
        refused > 0 && accepted > 0,
        "the run exercised both outcomes: {refused} refused, {accepted} accepted"
    );
}

fn into_append_error(e: vertep_tiramemsu::tiramemsu::Error) -> AppendError {
    match vertep_tiramemsu::violations(&e) {
        Some(violations) => AppendError::Refused { violations },
        None => AppendError::Invalid(e.to_string()),
    }
}

fn full_violations(db: &Db) -> usize {
    db.now()
        .elpi_check(
            &ElpiProgram::Stored(Value::iri(PROGRAM)),
            &ElpiOptions::default(),
        )
        .unwrap()
        .violations
        .len()
}
