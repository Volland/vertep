//! `vt rules`. The full flow needs the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo test -p vertep-cli --features rules --test rules`.

use std::process::{Command, Output};

const VT: &str = env!("CARGO_BIN_EXE_vt");

fn vt(args: &[&str]) -> Output {
    Command::new(VT).args(args).output().expect("vt runs")
}

#[cfg(not(feature = "rules"))]
mod without_engine {
    use super::*;

    // @tg: verifies:: [[openspec:rule-programs#Native Library Is Optional#Feature off]] {level: e2e}
    #[test]
    fn rules_commands_say_unavailable() {
        let out = vt(&["rules", "show", "/nonexistent.db", "urn:vt:program:p"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).starts_with("rules-unavailable"));
    }
}

#[cfg(feature = "rules")]
mod with_engine {
    use super::*;
    use std::sync::Arc;
    use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
    use vertep_core::datamodel::Value;
    use vertep_rules::{RuleDecider, RulePolicy, put_program};
    use vertep_tiramemsu::TiramemsuShard;

    const PROGRAM: &str = "urn:vt:program:loan-policy";
    const POLICY: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.
"#;

    fn ok(args: &[&str]) -> String {
        let out = vt(args);
        assert!(
            out.status.success(),
            "vt {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    // @tg: verifies:: [[openspec:rule-decider#Bundle Includes Rule Closure#Offline re-run]] {level: e2e}
    // @tg: verifies:: [[openspec:rule-decider#Rule Version Counterfactual#New policy]] {level: e2e}
    #[test]
    fn put_check_show_diff_bundle_verify_and_counterfactual() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("shard.db");
        let (db_s, db_p) = (db.to_str().unwrap(), dir.path());
        std::fs::write(db_p.join("v1.elpi"), POLICY).unwrap();
        std::fs::write(db_p.join("v2.elpi"), POLICY.replace("50000", "30000")).unwrap();

        // the CLI puts the program; the decider (a host) makes a decision in the same file
        {
            let shard = Arc::new(TiramemsuShard::open(&db).unwrap());
            drop(shard);
        }
        ok(&[
            "rules",
            "put",
            db_s,
            PROGRAM,
            db_p.join("v1.elpi").to_str().unwrap(),
        ]);
        let tx_v1 = TiramemsuShard::open(&db).unwrap().last_tx().unwrap();
        assert!(ok(&["rules", "check", db_s, PROGRAM]).starts_with("ok: 0 conclusions"));
        assert!(ok(&["rules", "show", db_s, PROGRAM]).contains("A =< 50000"));

        let tx_v2;
        let derivations = {
            let shard = Arc::new(TiramemsuShard::open(&db).unwrap());
            let d = RuleDecider::new(shard.clone(), PROGRAM, RulePolicy::default()).unwrap();
            let req = DecisionRequest {
                state: "underwriting".to_owned(),
                candidates: vec![Candidate {
                    event: "approve".to_owned(),
                    schema: serde_json::json!({}),
                    schema_hash: String::new(),
                    decision_guard: None,
                    source: "underwriting".to_owned(),
                    index: 0,
                }],
                context: ContextView::new(Value::Object(vec![(
                    "amount".to_owned(),
                    Value::Number(40_000.0),
                )])),
                actor: "https://example.org/loans/1".to_owned(),
                correlation: "cli1".to_owned(),
                binding: "underwriter".to_owned(),
                fallback: false,
            };
            assert!(matches!(
                futures::executor::block_on(d.decide(req)),
                DecisionOutcome::Decision(_)
            ));
            let json = serde_json::to_string(&d.derivations()).unwrap();
            // the host stores the program change after the decision
            put_program(&shard, PROGRAM, &POLICY.replace("50000", "30000")).unwrap();
            tx_v2 = shard.last_tx().unwrap();
            json
        };
        std::fs::write(db_p.join("derivations.json"), derivations).unwrap();

        // the earlier version is still readable and the diff names the changed line
        let (from, to) = (tx_v1.to_string(), tx_v2.to_string());
        let d = ok(&["rules", "diff", db_s, PROGRAM, "--from", &from, "--to", &to]);
        assert!(
            d.contains("- ")
                && d.contains("A =< 50000")
                && d.contains("+ ")
                && d.contains("A =< 30000"),
            "{d}"
        );

        // the bundle verifies from its file alone
        let bundle = db_p.join("bundle.json");
        ok(&[
            "rules",
            "bundle",
            db_s,
            db_p.join("derivations.json").to_str().unwrap(),
            "urn:vt:decision:cli1",
            "--out",
            bundle.to_str().unwrap(),
        ]);
        let v = ok(&["rules", "verify-bundle", bundle.to_str().unwrap()]);
        assert!(
            v.starts_with("verified: urn:vt:decision:cli1 concluded approve"),
            "{v}"
        );
        // an altered bundle is refused
        let altered = std::fs::read_to_string(&bundle)
            .unwrap()
            .replace("50000", "90000");
        std::fs::write(db_p.join("altered.json"), altered).unwrap();
        let bad = vt(&[
            "rules",
            "verify-bundle",
            db_p.join("altered.json").to_str().unwrap(),
        ]);
        assert_eq!(bad.status.code(), Some(1));

        // the next version would not approve this case
        let c = ok(&[
            "rules",
            "counterfactual",
            db_s,
            db_p.join("derivations.json").to_str().unwrap(),
            db_p.join("v2.elpi").to_str().unwrap(),
        ]);
        assert!(c.starts_with("0 unchanged, 1 differ"), "{c}");
        assert!(c.contains("approve -> (none)"), "{c}");
    }

    // @tg: verifies:: [[openspec:rule-invariants#Enabling Checks Existing Data#Live data violates]] {level: e2e}
    #[test]
    fn enforce_checks_existing_data_and_materialize_writes_derived_facts() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("shard.db");
        let db_s = db.to_str().unwrap();
        let prog = dir.path().join("p.elpi");
        std::fs::write(
            &prog,
            r#"tm.violation error "never" "always violated" [E] :- tm.triple _ (iri "urn:vt:p") _ E.
tm.conclude (triple S (iri "urn:vt:q") (iri "urn:vt:yes")) [E] :- tm.triple S (iri "urn:vt:p") _ E.
"#,
        )
        .unwrap();
        ok(&["rules", "put", db_s, PROGRAM, prog.to_str().unwrap()]);
        // no data yet: enabling works, and materialize has nothing to derive
        ok(&["rules", "enforce", db_s, PROGRAM]);
        assert!(ok(&["rules", "materialize", db_s, PROGRAM]).starts_with("derived 0"));
        ok(&["rules", "unenforce", db_s, PROGRAM]);
        // with a violating statement present, enabling is refused and says why
        {
            use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value};
            let shard = TiramemsuShard::open(&db).unwrap();
            shard
                .db()
                .transact(TxOptions::default(), |t| {
                    t.assert(
                        Value::iri("urn:vt:a"),
                        Value::iri("urn:vt:p"),
                        Value::str("x"),
                        Valid::ALWAYS,
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        let out = vt(&["rules", "enforce", db_s, PROGRAM]);
        assert_eq!(out.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("never"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(ok(&["rules", "materialize", db_s, PROGRAM]).starts_with("derived 1"));
    }

    // @tg: verifies:: [[openspec:rule-authoring#Promotion Is Staged And Journaled#A step is journaled]] {level: e2e}
    // @tg: verifies:: [[openspec:rule-verification#Rules Profile#Out of profile]] {level: e2e}
    #[test]
    fn propose_shadow_promote_and_profile_from_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("shard.db");
        let (db_s, d) = (db.to_str().unwrap(), dir.path());
        let live = r#"tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.
"#;
        std::fs::write(d.join("live.elpi"), live).unwrap();
        std::fs::write(d.join("next.elpi"), live.replace("50000", "30000")).unwrap();
        std::fs::write(d.join("cut.elpi"), "p X :- q X, !.").unwrap();
        std::fs::write(
            d.join("grants.json"),
            r#"{"propose":["agent:a"],"promote":["reviewer:r"]}"#,
        )
        .unwrap();
        ok(&[
            "rules",
            "put",
            db_s,
            PROGRAM,
            d.join("live.elpi").to_str().unwrap(),
        ]);

        // the profile check
        assert!(
            ok(&["rules", "profile", d.join("live.elpi").to_str().unwrap()]).starts_with("inside")
        );
        let bad = vt(&["rules", "profile", d.join("cut.elpi").to_str().unwrap()]);
        assert_eq!(bad.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&bad.stderr).contains("cut"));

        // one recorded decision to try proposals on
        {
            let shard = Arc::new(TiramemsuShard::open(&db).unwrap());
            let dec = RuleDecider::new(shard, PROGRAM, RulePolicy::default()).unwrap();
            let req = DecisionRequest {
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
                    Value::Number(40_000.0),
                )])),
                actor: "https://example.org/a".to_owned(),
                correlation: "k1".to_owned(),
                binding: "b".to_owned(),
                fallback: false,
            };
            assert!(matches!(
                futures::executor::block_on(dec.decide(req)),
                DecisionOutcome::Decision(_)
            ));
            std::fs::write(
                d.join("cases.json"),
                serde_json::to_string(&dec.derivations()).unwrap(),
            )
            .unwrap();
        }
        let (grants, cases, next) = (
            d.join("grants.json"),
            d.join("cases.json"),
            d.join("next.elpi"),
        );

        // a stranger is refused with a code; the agent passes
        let refused = vt(&[
            "rules",
            "propose",
            db_s,
            PROGRAM,
            next.to_str().unwrap(),
            "--by",
            "stranger",
            "--events",
            "approve",
            "--cases",
            cases.to_str().unwrap(),
            "--grants",
            grants.to_str().unwrap(),
        ]);
        assert_eq!(refused.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&refused.stderr).contains("capability"));
        let out = ok(&[
            "rules",
            "propose",
            db_s,
            PROGRAM,
            next.to_str().unwrap(),
            "--by",
            "agent:a",
            "--events",
            "approve",
            "--cases",
            cases.to_str().unwrap(),
            "--grants",
            grants.to_str().unwrap(),
        ]);
        let id = out.trim().strip_prefix("proposed ").unwrap().to_owned();

        assert!(
            ok(&["rules", "shadow", db_s, &id, cases.to_str().unwrap()])
                .starts_with("0 unchanged, 1 differ")
        );
        // a step may not be skipped
        let skip = vt(&[
            "rules",
            "promote",
            db_s,
            &id,
            "--to",
            "live",
            "--by",
            "reviewer:r",
            "--grants",
            grants.to_str().unwrap(),
        ]);
        assert_eq!(skip.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&skip.stderr).contains("skipped-step"));
        for (to, extra) in [("shadow", None), ("scoped", Some("team-a")), ("live", None)] {
            let mut args = vec![
                "rules",
                "promote",
                db_s,
                id.as_str(),
                "--to",
                to,
                "--by",
                "reviewer:r",
                "--grants",
                grants.to_str().unwrap(),
            ];
            if let Some(sc) = extra {
                args.extend(["--scope", sc]);
            }
            assert!(ok(&args).contains(to));
        }
        assert!(ok(&["rules", "show", db_s, PROGRAM]).contains("A =< 30000"));
    }
}
