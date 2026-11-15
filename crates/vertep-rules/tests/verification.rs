//! Vectors for `rule-verification`. They need the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo test -p vertep-rules --features native`.
#![cfg(feature = "native")]

use std::sync::Arc;

use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_rules::{RuleBundle, RuleDecider, RulePolicy, Totality, put_program, totality};
use vertep_tiramemsu::TiramemsuShard;

const PROGRAM: &str = "urn:vt:program:p";

const POLICY: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:refer")) [X, Eo] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:refer") Eo.
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:refuse")) [X] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X.
"#;

fn ctx(amount: f64) -> Vec<(String, Value)> {
    vec![("amount".to_owned(), Value::Number(amount))]
}

fn candidate(event: &str) -> Candidate {
    Candidate {
        event: event.to_owned(),
        schema: serde_json::json!({}),
        schema_hash: String::new(),
        decision_guard: None,
        source: "s".to_owned(),
        index: 0,
    }
}

fn decide(d: &RuleDecider, id: &str, amount: f64, offered: &[&str]) -> DecisionOutcome {
    let request = DecisionRequest {
        state: "s".to_owned(),
        candidates: offered.iter().map(|e| candidate(e)).collect(),
        context: ContextView::new(Value::Object(ctx(amount))),
        actor: "https://example.org/a".to_owned(),
        correlation: id.to_owned(),
        binding: "b".to_owned(),
        fallback: false,
    };
    futures::executor::block_on(d.decide(request))
}

fn shard(program: &str) -> (tempfile::TempDir, Arc<TiramemsuShard>) {
    let dir = tempfile::tempdir().unwrap();
    let s = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    put_program(&s, PROGRAM, program).unwrap();
    (dir, s)
}

// @tg: verifies:: [[openspec:rule-verification#Rules Profile#In profile]] {level: integration}
// @tg: verifies:: [[openspec:rule-verification#Rules Profile#Out of profile]] {level: integration}
#[test]
fn decisions_from_a_program_outside_the_profile_are_marked_unchecked() {
    let (_d, s) = shard(POLICY);
    let d = RuleDecider::new(s, PROGRAM, RulePolicy::default()).unwrap();
    decide(&d, "a", 1.0, &["approve"]);
    assert!(d.derivations()[0].profile_checked);
    // a program with a cut still runs, and its decision is unchecked
    let cutting = "tm.conclude (triple C (iri \"urn:vt:decision\") (iri \"urn:vt:event:approve\")) [X] :- tm.triple (iri \"urn:vt:ctx\") (iri \"urn:vt:deciding\") C X, !.";
    let (_d2, s2) = shard(cutting);
    let d2 = RuleDecider::new(s2, PROGRAM, RulePolicy::default()).unwrap();
    assert!(matches!(
        decide(&d2, "b", 1.0, &["approve"]),
        DecisionOutcome::Decision(_)
    ));
    assert!(!d2.derivations()[0].profile_checked);
}

// @tg: verifies:: [[openspec:rule-verification#Support Is Necessary#Padded support]] {level: integration}
// @tg: verifies:: [[openspec:rule-verification#Support Is Necessary#Minimal support]] {level: integration}
#[test]
fn padded_support_is_reported_and_minimal_support_passes() {
    // `Eo` (the offer) is listed as support but the amount alone decides here
    let padded = r#"tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
        tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
        tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000,
        tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo."#;
    let (_d, s) = shard(padded);
    let d = RuleDecider::new(s.clone(), PROGRAM, RulePolicy::default()).unwrap();
    decide(&d, "p", 10.0, &["approve"]);
    let bundle = RuleBundle::export(&s, &d.derivations()[0]).unwrap();
    // every statement here is needed: removing any one stops the rule concluding
    assert_eq!(bundle.unnecessary_support().unwrap(), Vec::<String>::new());
    // a bundle whose support lists an extra statement the program never read is padded
    let mut padded_bundle = bundle.clone();
    let extra = vertep_rules::Stmt {
        eid: "e99999".to_owned(),
        s: vertep_rules::Term::Iri("urn:vt:other".to_owned()),
        p: vertep_rules::Term::Iri("urn:vt:p".to_owned()),
        o: vertep_rules::Term::Str("x".to_owned()),
    };
    padded_bundle.statements.push(extra.clone());
    padded_bundle.derivation.support.push(extra.eid.clone());
    assert_eq!(
        padded_bundle.unnecessary_support().unwrap(),
        vec![extra.eid]
    );
}

// @tg: verifies:: [[openspec:rule-verification#Totality Check#Covered]] {level: integration}
// @tg: verifies:: [[openspec:rule-verification#Totality Check#A gap]] {level: integration}
#[test]
fn totality_finds_a_gap_and_accepts_an_explicit_refusal() {
    let events = ["approve", "refer"];
    let contexts = vec![ctx(10.0), ctx(80_000.0)];
    // with the explicit refusal clause, every offer is answered
    match totality(POLICY, &events, &contexts, &RulePolicy::default()).unwrap() {
        Totality::Total { checked } => assert_eq!(checked, 3 * 2),
        other => panic!("{other:?}"),
    }
    // without it, the offer {refer} alone is covered but {approve} at a high amount is not
    let no_refuse: String = POLICY.lines().take(8).collect::<Vec<_>>().join("\n");
    match totality(&no_refuse, &events, &contexts, &RulePolicy::default()).unwrap() {
        Totality::Gap { offer, context } => {
            assert_eq!(offer, vec!["approve".to_owned()]);
            assert_eq!(context, ctx(80_000.0));
        }
        other => panic!("{other:?}"),
    }
}

// @tg: verifies:: [[openspec:rule-verification#Totality Check#A gap]] {level: unit}
#[test]
fn totality_says_unknown_instead_of_guessing() {
    let many: Vec<String> = (0..9).map(|i| format!("e{i}")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(matches!(
        totality(POLICY, &refs, &[ctx(1.0)], &RulePolicy::default()).unwrap(),
        Totality::Unknown(_)
    ));
}
