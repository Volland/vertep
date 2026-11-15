//! Agent-proposed rules: an ordered gate, inert proposals with their authorship, shadow
//! evaluation, staged and journaled promotion, and a refusal of self-governance
//! ([[openspec:rule-authoring#Proposals Pass A Gate]] and its siblings).
//!
//! A proposal is a program node of its own (`<program>/proposal/<digest>`), so no decider reads it
//! and no enforcement runs it until it is promoted to live, which supersedes the live program.

use sha2::{Digest, Sha256};
use tiramemsu::{ElpiOptions, ElpiProgram, TxElpi, TxOptions, Valid, Value as TmValue};
use vertep_tiramemsu::TiramemsuShard;

use crate::authoring_types::err;
pub use crate::authoring_types::*;
use crate::{
    CounterfactualReport, Derivation, RulePolicy, check_profile, clauses, counterfactual,
    program_text, put_program,
};

fn digest12(text: &str) -> String {
    let h = Sha256::digest(text.as_bytes());
    h.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn iri(s: impl Into<String>) -> TmValue {
    TmValue::iri(s)
}

/// The objects of `(subject, predicate, ?)` as text, oldest statement first.
fn objects(shard: &TiramemsuShard, subject: &str, predicate: &str) -> Vec<String> {
    let v = shard.db().now();
    let (Ok(Some(s)), Ok(Some(p))) = (v.encode(&iri(subject)), v.encode(&iri(predicate))) else {
        return Vec::new();
    };
    let mut rows = v.triples(Some(s), Some(p), None).unwrap_or_default();
    rows.sort_by_key(|t| t.eid);
    rows.iter()
        .filter_map(|t| match v.decode(t.o) {
            Ok(TmValue::Str(x)) | Ok(TmValue::Iri(x)) => Some(x),
            _ => None,
        })
        .collect()
}

/// Reads a proposal's authorship.
pub fn authorship(shard: &TiramemsuShard, proposal: &str) -> Option<Authorship> {
    let one = |p: &str| objects(shard, proposal, p).into_iter().next();
    Some(Authorship {
        program: one(vocab::FOR_PROGRAM)?,
        proposer: one(vocab::PROPOSED_BY)?,
        grant: one(vocab::GRANT)?,
        source: one(vocab::SOURCE)?,
        scope: objects(shard, proposal, vocab::SCOPE),
    })
}

/// The stage of a proposal.
pub fn stage(shard: &TiramemsuShard, proposal: &str) -> Option<Stage> {
    objects(shard, proposal, vocab::STAGE)
        .last()
        .and_then(|s| Stage::parse(s))
}

fn write_stage(shard: &TiramemsuShard, proposal: &str, to: Stage) -> Result<(), GateError> {
    let v = shard.db().now();
    let old: Vec<_> = match (v.encode(&iri(proposal)), v.encode(&iri(vocab::STAGE))) {
        (Ok(Some(s)), Ok(Some(p))) => v
            .triples(Some(s), Some(p), None)
            .map_err(|e| err("store", None, e.to_string()))?
            .into_iter()
            .map(|t| t.eid)
            .collect(),
        _ => Vec::new(),
    };
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            for e in &old {
                t.retract(*e)?;
            }
            t.assert(
                iri(proposal),
                iri(vocab::STAGE),
                TmValue::str(to.name()),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .map(|_| ())
        .map_err(|e| err("store", None, e.to_string()))
}

/// The event names a program concludes, by scanning for event IRIs.
fn concluded_events(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in clauses(text) {
        let mut rest = c.as_str();
        while let Some(i) = rest.find(crate::vocab::EVENT_PREFIX) {
            let after = &rest[i + crate::vocab::EVENT_PREFIX.len()..];
            let name: String = after
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
                .collect();
            out.push((name.clone(), c.clone()));
            rest = &after[name.len()..];
        }
    }
    out
}

/// Puts a rule program forward. The proposal passes, in order: the self-governance limit, the
/// capability check, the event types, the profile, a trial run on recorded cases under fuel and,
/// for programs with constraints, a check against the existing data. The first failure stops with
/// a structured error. A passing proposal is stored inert, with its authorship.
// @tg: implements:: [[openspec:rule-authoring#Proposals Pass A Gate]]
// @tg: implements:: [[openspec:rule-authoring#Gate Errors Are Structured]]
// @tg: implements:: [[openspec:rule-authoring#Authorship Is Recorded]]
// @tg: implements:: [[openspec:rule-authoring#No Self-Governance]]
pub fn propose(
    shard: &TiramemsuShard,
    program: &str,
    text: &str,
    by: &Actor<'_>,
    source: &str,
    scope: &[String],
    ctx: &GateContext<'_>,
) -> Result<String, GateError> {
    // 0. structural: an actor never proposes rules that govern itself, whatever its grants
    if scope.iter().any(|a| a == by.identity) {
        return Err(err(
            "self-governance",
            None,
            format!("{} would govern itself", by.identity),
        ));
    }
    // 1. capability
    if !(ctx.authorize)(by, Action::Propose) {
        return Err(err(
            "capability",
            None,
            format!("{} may not propose", by.identity),
        ));
    }
    // 2. types
    for (event, clause) in concluded_events(text) {
        if event != crate::vocab::REFUSE && !ctx.events.contains(&event) {
            return Err(err(
                "type-error",
                Some(clause.chars().take(120).collect()),
                format!("the chart declares no event `{event}`"),
            ));
        }
    }
    // 3. profile
    if let Some(v) = check_profile(text).into_iter().next() {
        return Err(err("profile", Some(v.clause), v.reason));
    }
    // 4. trial run on recorded cases, under fuel
    if ctx.cases.is_empty() {
        if !ctx.allow_no_cases {
            return Err(err(
                "no-cases",
                None,
                "no recorded cases to try the proposal on; a promote-grant holder may waive this",
            ));
        }
    } else if let Err(e) = counterfactual(shard, ctx.cases, text, ctx.policy) {
        return Err(err("trial-failed", None, e.to_string()));
    }
    // 5. constraints against the existing data
    if text.contains("tm.violation") {
        let opts = ElpiOptions {
            fuel: ctx.policy.fuel,
            ..ElpiOptions::default()
        };
        match shard
            .db()
            .now()
            .elpi_check(&ElpiProgram::Text(text.to_owned()), &opts)
        {
            Ok(r) if !r.violations.is_empty() => {
                return Err(err(
                    "existing-data",
                    None,
                    format!("{} existing violation(s)", r.violations.len()),
                ));
            }
            Ok(_) => {}
            Err(e) => return Err(err("trial-failed", None, e.to_string())),
        }
    }
    // stored inert, under its own node, with its authorship
    let id = format!("{program}/proposal/{}", digest12(text));
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.store_elpi_program(iri(id.clone()), text)?;
            let s = || iri(id.clone());
            t.assert(s(), iri(vocab::FOR_PROGRAM), iri(program), Valid::ALWAYS)?;
            t.assert(
                s(),
                iri(vocab::PROPOSED_BY),
                TmValue::str(by.identity),
                Valid::ALWAYS,
            )?;
            t.assert(
                s(),
                iri(vocab::GRANT),
                TmValue::str(by.grant),
                Valid::ALWAYS,
            )?;
            t.assert(s(), iri(vocab::SOURCE), TmValue::str(source), Valid::ALWAYS)?;
            for a in scope {
                t.assert(s(), iri(vocab::SCOPE), iri(a.clone()), Valid::ALWAYS)?;
            }
            t.assert(
                s(),
                iri(vocab::STAGE),
                TmValue::str(Stage::Proposed.name()),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .map_err(|e| err("store", None, e.to_string()))?;
    Ok(id)
}

/// Runs a proposal in shadow beside the live program on recorded cases: each difference between
/// its conclusion and the recorded decision is written to the proposal, and no decision or append
/// is affected ([[openspec:rule-authoring#Shadow Evaluation]]).
// @tg: implements:: [[openspec:rule-authoring#Shadow Evaluation]]
pub fn shadow(
    shard: &TiramemsuShard,
    proposal: &str,
    cases: &[Derivation],
    policy: &RulePolicy,
) -> Result<CounterfactualReport, GateError> {
    let text =
        program_text(shard, proposal, None).map_err(|e| err("store", None, e.to_string()))?;
    let report = counterfactual(shard, cases, &text, policy)
        .map_err(|e| err("trial-failed", None, e.to_string()))?;
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            for d in &report.differs {
                t.assert(
                    iri(proposal),
                    iri(vocab::SHADOW_DIFFERS),
                    iri(d.node.clone()),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .map_err(|e| err("store", None, e.to_string()))?;
    Ok(report)
}

/// The decision nodes a proposal's shadow runs differed on.
pub fn shadow_differences(shard: &TiramemsuShard, proposal: &str) -> Vec<String> {
    objects(shard, proposal, vocab::SHADOW_DIFFERS)
}

/// Moves a proposal to the next stage as a journaled act by an identity holding the promote grant;
/// a stage may not be skipped, and an actor may not promote rules that govern it. `Scoped` records
/// the scope; `Live` supersedes the live program with the proposal's text
/// ([[openspec:rule-authoring#Promotion Is Staged And Journaled]]).
// @tg: implements:: [[openspec:rule-authoring#Promotion Is Staged And Journaled]]
// @tg: implements:: [[openspec:rule-authoring#No Self-Governance]]
pub fn promote(
    shard: &TiramemsuShard,
    proposal: &str,
    to: Stage,
    by: &Actor<'_>,
    scope_name: Option<&str>,
    ctx: &GateContext<'_>,
) -> Result<(), GateError> {
    let a = authorship(shard, proposal).ok_or_else(|| err("stage", None, "no such proposal"))?;
    if a.scope.iter().any(|s| s == by.identity) {
        return Err(err(
            "self-governance",
            None,
            format!("{} would promote rules that govern it", by.identity),
        ));
    }
    if !(ctx.authorize)(by, Action::Promote) {
        return Err(err(
            "capability",
            None,
            format!("{} may not promote", by.identity),
        ));
    }
    let now =
        stage(shard, proposal).ok_or_else(|| err("stage", None, "the proposal has no stage"))?;
    match now.next() {
        Some(n) if n == to => {}
        Some(n) => {
            return Err(err(
                "skipped-step",
                None,
                format!(
                    "the proposal is {}; the next step is {}, not {}",
                    now.name(),
                    n.name(),
                    to.name()
                ),
            ));
        }
        None => return Err(err("stage", None, "the proposal is already live")),
    }
    let text =
        program_text(shard, proposal, None).map_err(|e| err("store", None, e.to_string()))?;
    let act = format!("{proposal}/act/{}", to.name());
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.assert(
                iri(proposal),
                iri(vocab::ACT),
                iri(act.clone()),
                Valid::ALWAYS,
            )?;
            t.assert(
                iri(act.clone()),
                iri(vocab::ACT_BY),
                TmValue::str(by.identity),
                Valid::ALWAYS,
            )?;
            t.assert(
                iri(act.clone()),
                iri(vocab::ACT_STAGE),
                TmValue::str(to.name()),
                Valid::ALWAYS,
            )?;
            t.assert(
                iri(act.clone()),
                iri(vocab::ACT_DIGEST),
                TmValue::str(digest12(&text)),
                Valid::ALWAYS,
            )?;
            if let Some(s) = scope_name {
                t.assert(
                    iri(act.clone()),
                    iri(vocab::ACT_SCOPE),
                    TmValue::str(s),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .map_err(|e| err("store", None, e.to_string()))?;
    if to == Stage::Live {
        put_program(shard, &a.program, &text).map_err(|e| err("store", None, e.to_string()))?;
    }
    write_stage(shard, proposal, to)
}

/// The act that moved a proposal to `to`, if it happened.
pub fn act(shard: &TiramemsuShard, proposal: &str, to: Stage) -> Option<Act> {
    let node = format!("{proposal}/act/{}", to.name());
    let one = |p: &str| objects(shard, &node, p).into_iter().next();
    Some(Act {
        by: one(vocab::ACT_BY)?,
        stage: Stage::parse(&one(vocab::ACT_STAGE)?)?,
        scope: one(vocab::ACT_SCOPE),
        digest: one(vocab::ACT_DIGEST)?,
    })
}
