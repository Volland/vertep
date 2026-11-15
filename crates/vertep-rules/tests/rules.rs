//! Vectors for `rule-programs` and `rule-decider`. They need the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo test -p vertep-rules --features native`.
//! Without the feature only the unavailable vector runs.

use std::sync::Arc;

#[cfg(feature = "native")]
use vertep_rules::{RuleBundle, counterfactual, put_program, verify};
use vertep_rules::{RuleDecider, RulePolicy, RulesError};
use vertep_tiramemsu::TiramemsuShard;

const PROGRAM: &str = "urn:vt:program:loan-policy";

/// The credit policy of `examples/credit-decision`: approve inside the limits, refer above the
/// ceiling. Every rule starts from the context statement.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
const POLICY: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ed, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:dti") (num D) Ed, D =< 43,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.

tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:refer")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:refer") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A > 50000.
"#;

#[cfg(not(feature = "native"))]
mod without_engine {
    use super::*;

    // @tg: verifies:: [[openspec:rule-programs#Native Library Is Optional#Feature off]] {level: unit}
    #[test]
    fn constructors_fail_with_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
        assert_eq!(
            RuleDecider::new(shard, PROGRAM, RulePolicy::default()).err(),
            Some(RulesError::Unavailable)
        );
    }
}

#[cfg(feature = "native")]
mod with_engine {
    use super::*;
    use vertep_agent::{
        Candidate, ContextView, Decider, DeciderKind, DecisionOutcome, DecisionRequest,
    };
    use vertep_core::datamodel::Value;

    fn candidate(event: &str) -> Candidate {
        Candidate {
            event: event.to_owned(),
            schema: serde_json::json!({}),
            schema_hash: String::new(),
            decision_guard: None,
            source: "underwriting".to_owned(),
            index: 0,
        }
    }

    fn request(correlation: &str, amount: f64, dti: f64, offered: &[&str]) -> DecisionRequest {
        DecisionRequest {
            state: "underwriting".to_owned(),
            candidates: offered.iter().map(|e| candidate(e)).collect(),
            context: ContextView::new(Value::Object(vec![
                ("amount".to_owned(), Value::Number(amount)),
                ("dti".to_owned(), Value::Number(dti)),
            ])),
            actor: "https://example.org/loans/1".to_owned(),
            correlation: correlation.to_owned(),
            binding: "underwriter".to_owned(),
            fallback: false,
        }
    }

    fn answer(d: &RuleDecider, r: DecisionRequest) -> DecisionOutcome {
        futures::executor::block_on(d.decide(r))
    }

    fn reason(o: DecisionOutcome) -> String {
        match o {
            DecisionOutcome::Refusal(r) => r.reason,
            DecisionOutcome::Decision(d) => panic!("expected a refusal, got {}", d.event),
        }
    }

    fn shard() -> (tempfile::TempDir, Arc<TiramemsuShard>) {
        let dir = tempfile::tempdir().unwrap();
        let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
        put_program(&shard, PROGRAM, POLICY).unwrap();
        (dir, shard)
    }

    fn decider(shard: &Arc<TiramemsuShard>) -> RuleDecider {
        RuleDecider::new(shard.clone(), PROGRAM, RulePolicy::default()).unwrap()
    }

    // @tg: verifies:: [[openspec:rule-decider#Rule Decider#A rule chooses an offered event]] {level: integration}
    // @tg: verifies:: [[openspec:rule-decider#Derivation Is The Rationale#Support is listed]] {level: integration}
    // @tg: verifies:: [[openspec:rule-decider#Offer And State Are Statements#The offer is readable by the rule]] {level: integration}
    // @tg: verifies:: [[openspec:rule-programs#Evaluation Is Bounded#Fuel used is recorded]] {level: integration}
    #[test]
    fn a_rule_chooses_an_offered_event_and_lists_its_support() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        assert_eq!(d.kind(), DeciderKind::Rules);
        match answer(
            &d,
            request("c1", 20_000.0, 31.0, &["approve", "decline", "refer"]),
        ) {
            DecisionOutcome::Decision(dec) => {
                assert_eq!(dec.event, "approve");
                assert!(
                    dec.annotations
                        .rationale
                        .unwrap()
                        .starts_with("derivation: approve")
                );
                assert_eq!(dec.annotations.decider_kind.as_deref(), Some("rules"));
            }
            o => panic!("{o:?}"),
        }
        let log = d.derivations();
        assert_eq!(log.len(), 1);
        // the context statement, the offer, the ratio and the amount
        assert_eq!(log[0].support.len(), 4, "{:?}", log[0].support);
        assert_eq!(log[0].event, "approve");
        assert!(log[0].fuel_used > 0 && log[0].fuel_used <= log[0].fuel_limit);
    }

    // @tg: verifies:: [[openspec:rule-decider#Rule Decider#A rule chooses an offered event]] {level: integration}
    #[test]
    fn a_rule_refers_above_the_ceiling() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        match answer(&d, request("c2", 80_000.0, 38.0, &["decline", "refer"])) {
            DecisionOutcome::Decision(dec) => assert_eq!(dec.event, "refer"),
            o => panic!("{o:?}"),
        }
    }

    // @tg: verifies:: [[openspec:rule-decider#No Applicable Rule Is A Refusal#Nothing applies]] {level: integration}
    #[test]
    fn nothing_applies_is_a_refusal() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        assert_eq!(
            reason(answer(&d, request("c3", 20_000.0, 31.0, &["decline"]))),
            "no-applicable-rule"
        );
        assert!(d.derivations().is_empty());
    }

    // @tg: verifies:: [[openspec:rule-decider#Rule Decider#A rule names an event that was not offered]] {level: integration}
    #[test]
    fn an_event_that_was_not_offered_is_a_refusal() {
        let (_d, shard) = shard();
        // a program that concludes `approve` without reading the offer
        let careless = r#"tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X] :-
            tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X."#;
        put_program(&shard, PROGRAM, careless).unwrap();
        let d = decider(&shard);
        assert_eq!(
            reason(answer(&d, request("c4", 1.0, 1.0, &["refer", "decline"]))),
            "not-offered"
        );
    }

    // @tg: verifies:: [[openspec:rule-programs#Evaluation Is Bounded#A runaway rule ends]] {level: integration}
    #[test]
    fn a_runaway_rule_ends_with_a_fuel_refusal() {
        let (_d, shard) = shard();
        let looping = "pred spin.\nspin :- spin.\ntm.conclude (triple C (iri \"urn:vt:decision\") (iri \"urn:vt:event:approve\")) [X] :- tm.triple (iri \"urn:vt:ctx\") (iri \"urn:vt:deciding\") C X, spin.";
        put_program(&shard, PROGRAM, looping).unwrap();
        let policy = RulePolicy {
            fuel: 10_000,
            ..RulePolicy::default()
        };
        let d = RuleDecider::new(shard.clone(), PROGRAM, policy).unwrap();
        let r = reason(answer(&d, request("c5", 1.0, 1.0, &["approve"])));
        assert!(r.starts_with("fuel-exhausted"), "{r}");
    }

    // @tg: verifies:: [[openspec:rule-programs#Rule Version Is Pinned By The Entry#Replay uses the rules of the day]] {level: integration}
    #[test]
    fn replay_uses_the_rules_of_the_day() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("c6", 40_000.0, 31.0, &["approve", "refer"]));
        let first = d.derivations().remove(0);
        assert_eq!(first.event, "approve");
        // the ceiling drops to 30 000 after the decision
        put_program(
            &shard,
            PROGRAM,
            &POLICY
                .replace("A =< 50000", "A =< 30000")
                .replace("A > 50000", "A > 30000"),
        )
        .unwrap();
        // the recorded decision still replays against the rules of its day
        verify(&shard, &first, &RulePolicy::default()).unwrap();
        // a new decision under the new rules differs
        match answer(&d, request("c7", 40_000.0, 31.0, &["approve", "refer"])) {
            DecisionOutcome::Decision(dec) => assert_eq!(dec.event, "refer"),
            o => panic!("{o:?}"),
        }
    }

    // @tg: verifies:: [[openspec:rule-programs#Rule Version Is Pinned By The Entry#Engine version differs]] {level: unit}
    #[test]
    fn a_different_engine_version_is_reported() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("c8", 20_000.0, 31.0, &["approve"]));
        let rec = d.derivations().remove(0);
        let other = RulePolicy {
            engine: "tiramemsu-9.9/leanelpi-9.9".to_owned(),
            ..RulePolicy::default()
        };
        assert!(matches!(
            verify(&shard, &rec, &other),
            Err(RulesError::EngineMismatch { .. })
        ));
    }

    // @tg: verifies:: [[openspec:rule-programs#Rule Programs Live In The Shard#An edit is a supersede]] {level: integration}
    #[test]
    fn an_altered_closure_is_detected() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("c9", 20_000.0, 31.0, &["approve"]));
        let mut rec = d.derivations().remove(0);
        rec.closure_digest = "00".repeat(32);
        assert!(matches!(
            verify(&shard, &rec, &RulePolicy::default()),
            Err(RulesError::Diverged(_))
        ));
    }

    // @tg: verifies:: [[openspec:rule-programs#Rule Programs Live In The Shard#A program is stored with the journal]] {level: integration}
    // @tg: verifies:: [[openspec:rule-programs#Rule Programs Live In The Shard#An edit is a supersede]] {level: integration}
    #[test]
    fn a_program_is_a_node_in_the_shard_and_an_edit_supersedes() {
        use vertep_tiramemsu::tiramemsu::{TimeRef, Value as TmValue};
        let (_d, shard) = shard();
        let node = TmValue::iri(PROGRAM);
        let t1 = shard.last_tx().unwrap();
        let before = shard.db().now().elpi_program_info(&node).unwrap();
        put_program(&shard, PROGRAM, &POLICY.replace("50000", "40000")).unwrap();
        assert!(
            shard.last_tx().unwrap() > t1,
            "one transaction in the shard's file"
        );
        let after = shard.db().now().elpi_program_info(&node).unwrap();
        assert_ne!(before.statement, after.statement);
        let old = shard
            .db()
            .as_of(TimeRef::Tx(t1))
            .elpi_program_info(&node)
            .unwrap();
        assert_eq!(old.text, before.text, "the earlier source stays readable");
    }

    // @tg: verifies:: [[openspec:rule-programs#Rule Programs Live In The Shard#A program includes another]] {level: integration}
    #[test]
    fn a_program_includes_another_and_pins_its_version() {
        let (_d, shard) = shard();
        put_program(
            &shard,
            "urn:vt:program:ceiling",
            "pred ceiling o:int.\nceiling 50000.",
        )
        .unwrap();
        let main = POLICY.replace("A =< 50000", "ceiling Max, A =< Max");
        put_program(
            &shard,
            PROGRAM,
            &format!("accumulate <urn:vt:program:ceiling>.\n{main}"),
        )
        .unwrap();
        let d = decider(&shard);
        answer(&d, request("i1", 20_000.0, 31.0, &["approve"]));
        let rec = d.derivations().remove(0);
        assert_eq!(rec.event, "approve");
        // editing the library after the decision does not break the replay of the record
        put_program(
            &shard,
            "urn:vt:program:ceiling",
            "pred ceiling o:int.\nceiling 10.",
        )
        .unwrap();
        verify(&shard, &rec, &RulePolicy::default()).unwrap();
        // and the new closure decides differently
        assert_eq!(
            reason(answer(&d, request("i2", 20_000.0, 31.0, &["approve"]))),
            "no-applicable-rule"
        );
    }

    // @tg: verifies:: [[openspec:rule-decider#Bundle Includes Rule Closure#Offline re-run]] {level: integration}
    #[test]
    fn a_bundle_re_runs_offline_without_the_shard() {
        let (dir, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("b1", 20_000.0, 31.0, &["approve", "refer"]));
        let rec = d.derivations().remove(0);
        let json = RuleBundle::export(&shard, &rec).unwrap().to_json();
        // the shard and its file are gone: verification needs only the bundle
        drop(d);
        drop(shard);
        drop(dir);
        let bundle = RuleBundle::from_json(&json).unwrap();
        assert_eq!(bundle.programs.len(), 1);
        assert_eq!(bundle.statements.len(), rec.support.len());
        bundle.verify(&RulePolicy::default()).unwrap();
    }

    // @tg: verifies:: [[openspec:rule-decider#Bundle Includes Rule Closure#Tampered program]] {level: integration}
    #[test]
    fn a_tampered_bundle_fails() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("b2", 20_000.0, 31.0, &["approve"]));
        let bundle = RuleBundle::export(&shard, &d.derivations().remove(0)).unwrap();
        // the program source is altered
        let mut p = bundle.clone();
        p.programs[0].text = p.programs[0].text.replace("50000", "90000");
        assert!(matches!(
            p.verify(&RulePolicy::default()),
            Err(RulesError::Diverged(_))
        ));
        // a supporting statement is altered: the rule no longer concludes from it
        let mut st = bundle.clone();
        for s in &mut st.statements {
            if s.p == vertep_rules::Term::Iri("urn:vt:ctx:amount".to_owned()) {
                s.o = vertep_rules::Term::Int(99_999);
            }
        }
        assert!(matches!(
            st.verify(&RulePolicy::default()),
            Err(RulesError::Diverged(_))
        ));
        // the unchanged bundle still verifies
        bundle.verify(&RulePolicy::default()).unwrap();
    }

    // @tg: verifies:: [[openspec:rule-decider#Rule Version Counterfactual#New policy]] {level: integration}
    #[test]
    fn a_new_policy_is_tried_on_old_decisions_without_changing_the_journal() {
        let (_d, shard) = shard();
        let d = decider(&shard);
        answer(&d, request("k1", 20_000.0, 31.0, &["approve", "refer"]));
        answer(&d, request("k2", 40_000.0, 31.0, &["approve", "refer"]));
        answer(&d, request("k3", 80_000.0, 31.0, &["approve", "refer"]));
        let recorded = d.derivations();
        assert_eq!(
            recorded
                .iter()
                .map(|r| r.event.as_str())
                .collect::<Vec<_>>(),
            ["approve", "approve", "refer"]
        );
        let tx_before = shard.last_tx().unwrap();
        let next = POLICY
            .replace("A =< 50000", "A =< 30000")
            .replace("A > 50000", "A > 30000");
        let report = counterfactual(&shard, &recorded, &next, &RulePolicy::default()).unwrap();
        assert_eq!(report.unchanged, 2);
        assert_eq!(report.differs.len(), 1);
        let diff = &report.differs[0];
        assert_eq!(diff.recorded_event, "approve");
        assert_eq!(diff.counterfactual_event.as_deref(), Some("refer"));
        assert!(!diff.counterfactual_support.is_empty());
        assert_eq!(
            shard.last_tx().unwrap(),
            tx_before,
            "the journal is untouched"
        );
        // the live program is untouched too: a new decision still follows the original
        match answer(&d, request("k4", 40_000.0, 31.0, &["approve", "refer"])) {
            DecisionOutcome::Decision(dec) => assert_eq!(dec.event, "approve"),
            o => panic!("{o:?}"),
        }
    }
}
