//! Vectors for `rule-authoring`. They need the native engine library; see `tests/invariants.rs`.
#![cfg(feature = "native")]

use std::sync::Arc;

use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_rules::authoring::{
    Action, Actor, GateContext, Stage, act, authorship, diff, promote, propose, shadow,
    shadow_differences, stage,
};
use vertep_rules::{Derivation, RuleDecider, RulePolicy, program_text, put_program};
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{TxOptions, Valid, Value as TmValue};

const PROGRAM: &str = "urn:vt:program:loan-policy";
const LIVE: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:refer")) [X, Eo] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:refer") Eo.
"#;

fn tighter() -> String {
    LIVE.replace("50000", "30000")
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

fn decide(d: &RuleDecider, id: &str, amount: f64) -> DecisionOutcome {
    let request = DecisionRequest {
        state: "s".to_owned(),
        candidates: ["approve", "refer"].iter().map(|e| candidate(e)).collect(),
        context: ContextView::new(Value::Object(vec![(
            "amount".to_owned(),
            Value::Number(amount),
        )])),
        actor: "https://example.org/a".to_owned(),
        correlation: id.to_owned(),
        binding: "b".to_owned(),
        fallback: false,
    };
    futures::executor::block_on(d.decide(request))
}

struct World {
    _dir: tempfile::TempDir,
    shard: Arc<TiramemsuShard>,
    decider: RuleDecider,
    cases: Vec<Derivation>,
    events: Vec<String>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let shard = Arc::new(TiramemsuShard::open(dir.path().join("s.db")).unwrap());
    put_program(&shard, PROGRAM, LIVE).unwrap();
    let decider = RuleDecider::new(shard.clone(), PROGRAM, RulePolicy::default()).unwrap();
    for (i, amount) in [20_000.0, 40_000.0, 45_000.0].iter().enumerate() {
        assert!(matches!(
            decide(&decider, &format!("c{i}"), *amount),
            DecisionOutcome::Decision(_)
        ));
    }
    let cases = decider.derivations();
    World {
        _dir: dir,
        shard,
        decider,
        cases,
        events: vec!["approve".to_owned(), "refer".to_owned()],
    }
}

fn authorize(a: &Actor<'_>, action: Action) -> bool {
    match action {
        Action::Propose => a.identity.starts_with("agent:") || a.identity.starts_with("reviewer:"),
        Action::Promote => a.identity.starts_with("reviewer:"),
    }
}

fn gate<'a>(w: &'a World, policy: &'a RulePolicy) -> GateContext<'a> {
    GateContext {
        events: &w.events,
        cases: &w.cases,
        allow_no_cases: false,
        authorize: &authorize,
        policy,
    }
}

const AGENT: Actor<'static> = Actor {
    identity: "agent:a",
    grant: "grant:propose",
};
const REVIEWER: Actor<'static> = Actor {
    identity: "reviewer:r",
    grant: "grant:promote",
};

// @tg: verifies:: [[openspec:rule-authoring#Proposals Pass A Gate#A passing proposal]] {level: integration}
// @tg: verifies:: [[openspec:rule-authoring#Proposals Pass A Gate#A proposal is inert]] {level: integration}
// @tg: verifies:: [[openspec:rule-authoring#Authorship Is Recorded#Who proposed it]] {level: integration}
#[test]
fn a_passing_proposal_is_stored_inert_with_its_authorship() {
    let w = world();
    let policy = RulePolicy::default();
    let id = propose(
        &w.shard,
        PROGRAM,
        &tighter(),
        &AGENT,
        "reflection on refused cases",
        &[],
        &gate(&w, &policy),
    )
    .unwrap();
    assert_eq!(stage(&w.shard, &id), Some(Stage::Proposed));
    let a = authorship(&w.shard, &id).unwrap();
    assert_eq!(
        (a.proposer.as_str(), a.grant.as_str(), a.source.as_str()),
        ("agent:a", "grant:propose", "reflection on refused cases")
    );
    assert_eq!(a.program, PROGRAM);
    // the live program still decides: 40 000 is approved, not referred
    match decide(&w.decider, "after", 40_000.0) {
        DecisionOutcome::Decision(d) => assert_eq!(d.event, "approve"),
        o => panic!("{o:?}"),
    }
    assert!(
        program_text(&w.shard, PROGRAM, None)
            .unwrap()
            .contains("50000")
    );
}

// @tg: verifies:: [[openspec:rule-authoring#Gate Errors Are Structured#A type error]] {level: integration}
// @tg: verifies:: [[openspec:rule-authoring#Gate Errors Are Structured#Repair]] {level: integration}
#[test]
fn a_type_error_names_the_clause_and_a_repair_passes() {
    let w = world();
    let policy = RulePolicy::default();
    let bad = LIVE.replace("event:refer", "event:reject");
    let e = propose(
        &w.shard,
        PROGRAM,
        &bad,
        &AGENT,
        "s",
        &[],
        &gate(&w, &policy),
    )
    .unwrap_err();
    assert_eq!(e.code, "type-error");
    assert!(e.reason.contains("reject") && e.clause.is_some(), "{e}");
    // the agent repairs from the error and resubmits
    assert!(
        propose(
            &w.shard,
            PROGRAM,
            &LIVE.replace("event:refer", "event:refuse"),
            &AGENT,
            "s",
            &[],
            &gate(&w, &policy)
        )
        .is_ok()
    );
}

#[test]
fn capability_profile_trial_and_existing_data_stop_in_order() {
    let w = world();
    let policy = RulePolicy::default();
    let stranger = Actor {
        identity: "stranger",
        grant: "g",
    };
    assert_eq!(
        propose(
            &w.shard,
            PROGRAM,
            &tighter(),
            &stranger,
            "s",
            &[],
            &gate(&w, &policy)
        )
        .unwrap_err()
        .code,
        "capability"
    );
    let cut = format!("{LIVE}\nq X :- r X, !.");
    assert_eq!(
        propose(
            &w.shard,
            PROGRAM,
            &cut,
            &AGENT,
            "s",
            &[],
            &gate(&w, &policy)
        )
        .unwrap_err()
        .code,
        "profile"
    );
    // no recorded cases: refused unless a promote-grant holder waives it
    let mut ctx = gate(&w, &policy);
    let none: Vec<Derivation> = Vec::new();
    ctx.cases = &none;
    assert_eq!(
        propose(&w.shard, PROGRAM, &tighter(), &AGENT, "s", &[], &ctx)
            .unwrap_err()
            .code,
        "no-cases"
    );
    ctx.allow_no_cases = true;
    assert!(propose(&w.shard, PROGRAM, &tighter(), &AGENT, "s", &[], &ctx).is_ok());
    // a constraint the data already violates
    w.shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.assert(
                TmValue::iri("urn:vt:a"),
                TmValue::iri("urn:vt:p"),
                TmValue::str("x"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let invariant =
        r#"tm.violation error "never" "always" [E] :- tm.triple _ (iri "urn:vt:p") _ E."#;
    assert_eq!(
        propose(
            &w.shard,
            PROGRAM,
            invariant,
            &AGENT,
            "s",
            &[],
            &gate(&w, &policy)
        )
        .unwrap_err()
        .code,
        "existing-data"
    );
}

// @tg: verifies:: [[openspec:rule-authoring#Shadow Evaluation#Shadow difference]] {level: integration}
#[test]
fn a_shadow_run_records_differences_and_decides_nothing() {
    let w = world();
    let policy = RulePolicy::default();
    let id = propose(
        &w.shard,
        PROGRAM,
        &tighter(),
        &AGENT,
        "s",
        &[],
        &gate(&w, &policy),
    )
    .unwrap();
    let report = shadow(&w.shard, &id, &w.cases, &policy).unwrap();
    // 40 000 and 45 000 would be referred under the tighter ceiling
    assert_eq!(report.differs.len(), 2);
    assert_eq!(shadow_differences(&w.shard, &id).len(), 2);
    assert_eq!(
        stage(&w.shard, &id),
        Some(Stage::Proposed),
        "shadow evaluation is not a promotion"
    );
    match decide(&w.decider, "live", 45_000.0) {
        DecisionOutcome::Decision(d) => assert_eq!(d.event, "approve"),
        o => panic!("{o:?}"),
    }
}

// @tg: verifies:: [[openspec:rule-authoring#Promotion Is Staged And Journaled#A skipped step]] {level: integration}
// @tg: verifies:: [[openspec:rule-authoring#Promotion Is Staged And Journaled#A step is journaled]] {level: integration}
#[test]
fn promotion_goes_stage_by_stage_and_each_step_is_journaled() {
    let w = world();
    let policy = RulePolicy::default();
    let id = propose(
        &w.shard,
        PROGRAM,
        &tighter(),
        &AGENT,
        "s",
        &[],
        &gate(&w, &policy),
    )
    .unwrap();
    let e = promote(
        &w.shard,
        &id,
        Stage::Live,
        &REVIEWER,
        None,
        &gate(&w, &policy),
    )
    .unwrap_err();
    assert_eq!(e.code, "skipped-step");
    assert!(e.reason.contains("shadow"), "{e}");
    // the proposer cannot promote: no promote grant
    assert_eq!(
        promote(
            &w.shard,
            &id,
            Stage::Shadow,
            &AGENT,
            None,
            &gate(&w, &policy)
        )
        .unwrap_err()
        .code,
        "capability"
    );
    promote(
        &w.shard,
        &id,
        Stage::Shadow,
        &REVIEWER,
        None,
        &gate(&w, &policy),
    )
    .unwrap();
    promote(
        &w.shard,
        &id,
        Stage::Scoped,
        &REVIEWER,
        Some("team-a"),
        &gate(&w, &policy),
    )
    .unwrap();
    let a = act(&w.shard, &id, Stage::Scoped).unwrap();
    assert_eq!(
        (a.by.as_str(), a.scope.as_deref()),
        ("reviewer:r", Some("team-a"))
    );
    assert!(
        program_text(&w.shard, PROGRAM, None)
            .unwrap()
            .contains("50000"),
        "scoped is not live"
    );
    promote(
        &w.shard,
        &id,
        Stage::Live,
        &REVIEWER,
        None,
        &gate(&w, &policy),
    )
    .unwrap();
    assert_eq!(stage(&w.shard, &id), Some(Stage::Live));
    assert_eq!(act(&w.shard, &id, Stage::Live).unwrap().digest, a.digest);
    // the live program is now the proposal, and a new decision follows it
    match decide(&w.decider, "new", 40_000.0) {
        DecisionOutcome::Decision(d) => assert_eq!(d.event, "refer"),
        o => panic!("{o:?}"),
    }
}

// @tg: verifies:: [[openspec:rule-authoring#No Self-Governance#An agent edits its own rules]] {level: integration}
// @tg: verifies:: [[openspec:rule-authoring#No Self-Governance#A reviewer's grant]] {level: integration}
#[test]
fn an_actor_cannot_govern_itself_but_a_reviewer_can_promote_for_it() {
    let w = world();
    let policy = RulePolicy::default();
    let own = ["agent:a".to_owned()];
    let e = propose(
        &w.shard,
        PROGRAM,
        &tighter(),
        &AGENT,
        "s",
        &own,
        &gate(&w, &policy),
    )
    .unwrap_err();
    assert_eq!(e.code, "self-governance");
    // a proposal that governs the agent, made by another identity
    let id = propose(
        &w.shard,
        PROGRAM,
        &tighter(),
        &Actor {
            identity: "reviewer:other",
            grant: "g",
        },
        "s",
        &own,
        &gate(&w, &policy),
    )
    .unwrap();
    // the governed actor holding the promote grant still cannot promote it
    let governed = Actor {
        identity: "agent:a",
        grant: "grant:promote",
    };
    let ctx = GateContext {
        authorize: &|_, _| true,
        ..gate(&w, &policy)
    };
    assert_eq!(
        promote(&w.shard, &id, Stage::Shadow, &governed, None, &ctx)
            .unwrap_err()
            .code,
        "self-governance"
    );
    // a different reviewer can
    promote(
        &w.shard,
        &id,
        Stage::Shadow,
        &REVIEWER,
        None,
        &gate(&w, &policy),
    )
    .unwrap();
}

// @tg: verifies:: [[openspec:rule-authoring#Rule Diff#A readable diff]] {level: unit}
#[test]
fn the_diff_lists_added_and_removed_clauses() {
    let d = diff(LIVE, &tighter());
    assert_eq!(d.added.len(), 1);
    assert_eq!(d.removed.len(), 1);
    assert!(d.added[0].contains("30000") && d.removed[0].contains("50000"));
}
