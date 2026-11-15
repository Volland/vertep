//! Vectors for derivation certificates. The generation needs the native engine; the kernel check
//! also needs a built `lean/vertep-lean` (`lake build VertepLean`) and is skipped without it.
#![cfg(feature = "native")]

use std::path::PathBuf;
use std::sync::Arc;

use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_rules::{
    RuleBundle, RuleDecider, RulePolicy, RulesError, check_certificate, lean_certificate,
    put_program,
};
use vertep_tiramemsu::TiramemsuShard;

const PROGRAM: &str = "urn:vt:program:p";
const POLICY: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.
"#;

fn lean_dir() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean");
    (d.join(".lake/build/lib/lean/VertepLean/RuleCert.olean")
        .exists()
        && std::process::Command::new("lake")
            .arg("--version")
            .output()
            .is_ok())
    .then_some(d)
}

fn bundle(program: &str) -> RuleBundle {
    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    put_program(&shard, PROGRAM, program).unwrap();
    let d = RuleDecider::new(shard.clone(), PROGRAM, RulePolicy::default()).unwrap();
    let request = DecisionRequest {
        state: "s".to_owned(),
        candidates: vec![Candidate {
            event: "approve".to_owned(),
            schema: serde_json::json!({}),
            schema_hash: String::new(),
            decision_guard: None,
            source: "s".to_owned(),
            index: 0,
        }],
        context: ContextView::new(Value::Object(vec![(
            "amount".to_owned(),
            Value::Number(20_000.0),
        )])),
        actor: "https://example.org/a".to_owned(),
        correlation: "k1".to_owned(),
        binding: "b".to_owned(),
        fallback: false,
    };
    assert!(matches!(
        futures::executor::block_on(d.decide(request)),
        DecisionOutcome::Decision(_)
    ));
    RuleBundle::export(&shard, &d.derivations()[0]).unwrap()
}

#[test]
fn a_certifiable_decision_becomes_lean_source_with_the_instance() {
    let src = lean_certificate(&bundle(POLICY)).unwrap();
    assert!(src.contains("import VertepLean.RuleCert"));
    assert!(src.contains("theorem cert : check tape clause binding"));
    assert!(src.contains("(\"A\", .int (20000))"), "{src}");
    assert!(src.contains("#print axioms cert"));
}

#[test]
fn a_program_outside_the_fragment_is_reported_not_guessed() {
    let negated = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  not (tm.triple C (iri "urn:vt:blocked") _ _).
"#;
    let e = lean_certificate(&bundle(negated)).unwrap_err();
    assert!(
        e.to_string().contains("outside the certifiable fragment"),
        "{e}"
    );
}

// @tg: verifies:: [[openspec:rule-verification#Derivation Certificate#A valid certificate]] {level: integration}
// @tg: verifies:: [[openspec:rule-verification#Derivation Certificate#A forged conclusion]] {level: integration}
#[test]
fn the_lean_kernel_accepts_a_valid_certificate_and_rejects_a_forged_conclusion() {
    let Some(dir) = lean_dir() else {
        eprintln!("skipped: lean/vertep-lean is not built (lake build VertepLean)");
        return;
    };
    let src = lean_certificate(&bundle(POLICY)).unwrap();
    let report = check_certificate(&dir, &src).unwrap();
    assert!(
        report.axioms.contains("propext") || report.axioms.contains("does not depend"),
        "{}",
        report.axioms
    );
    // the same instance claims a different conclusion: the kernel refuses
    let forged = src.replace(
        "theorem cert : check tape clause binding (.iri \"urn:vt:decision:k1\") (.iri \"urn:vt:decision\") (.iri \"urn:vt:event:approve\")",
        "theorem cert : check tape clause binding (.iri \"urn:vt:decision:k1\") (.iri \"urn:vt:decision\") (.iri \"urn:vt:event:refer\")",
    );
    assert_ne!(forged, src, "the theorem line was found");
    assert!(matches!(
        check_certificate(&dir, &forged),
        Err(RulesError::Diverged(_))
    ));
    // and an instance whose comparison fails (amount over the ceiling) is refused too
    let over = src.replace(".int (20000)", ".int (90000)");
    assert!(matches!(
        check_certificate(&dir, &over),
        Err(RulesError::Diverged(_))
    ));
}

// @tg: verifies:: [[openspec:rule-verification#Derivation Certificate#Engine version mismatch]] {level: unit}
#[test]
fn a_certificate_for_another_engine_version_is_not_made() {
    let mut b = bundle(POLICY);
    b.derivation.engine = "tiramemsu-9.9/leanelpi-9.9".to_owned();
    assert!(matches!(
        b.verify(&RulePolicy::default()),
        Err(RulesError::EngineMismatch { .. })
    ));
}
