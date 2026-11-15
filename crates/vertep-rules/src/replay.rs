//! Bundles that verify offline and counterfactual replay under another rule version.

use std::collections::BTreeSet;

use tiramemsu::{
    Db, ElpiOptions, ElpiProgram, Error as TmError, OpenOptions, TimeRef, TxElpi, TxOptions, Valid,
    Value as TmValue,
};
use vertep_tiramemsu::TiramemsuShard;

use crate::bundle::{
    BUNDLE_FORMAT, CounterfactualReport, Divergence, ProgramSource, RuleBundle, Stmt, Term,
};
use crate::decider::{closure_sources, digest_of};
use crate::vocab;
use crate::{Derivation, RulePolicy, RulesError};

impl Term {
    fn from_tm(v: &TmValue) -> Result<Term, RulesError> {
        Ok(match v {
            TmValue::Iri(s) => Term::Iri(s.clone()),
            TmValue::Str(s) => Term::Str(s.clone()),
            TmValue::Int(n) => Term::Int(*n),
            TmValue::Bool(b) => Term::Bool(*b),
            TmValue::Double(d) => Term::Double(d.to_string()),
            other => {
                return Err(RulesError::Store(format!(
                    "a statement term of this kind cannot be bundled: {other:?}"
                )));
            }
        })
    }

    fn to_tm(&self) -> Result<TmValue, RulesError> {
        Ok(match self {
            Term::Iri(s) => TmValue::iri(s.clone()),
            Term::Str(s) => TmValue::str(s.clone()),
            Term::Int(n) => TmValue::Int(*n),
            Term::Bool(b) => TmValue::Bool(*b),
            Term::Double(d) => TmValue::Double(
                d.parse()
                    .map_err(|_| RulesError::Store(format!("bad double {d}")))?,
            ),
        })
    }
}

fn tm<E: std::fmt::Display>(e: E) -> RulesError {
    RulesError::Program(e.to_string())
}

fn event_of(v: &TmValue) -> Option<String> {
    match v {
        TmValue::Iri(s) => s.strip_prefix(vocab::EVENT_PREFIX).map(str::to_owned),
        _ => None,
    }
}

impl RuleBundle {
    /// Exports the bundle of `d` from the shard, reading the store as of the decision's
    /// transaction. Refuses a derivation whose closure no longer matches its recorded digest.
    // @tg: implements:: [[openspec:rule-decider#Bundle Includes Rule Closure]]
    pub fn export(shard: &TiramemsuShard, d: &Derivation) -> Result<RuleBundle, RulesError> {
        let view = shard.db().as_of(TimeRef::Tx(d.tx));
        let program = TmValue::iri(d.program.clone());
        let sources = closure_sources(&view, &program)?;
        if digest_of(&sources) != d.closure_digest {
            return Err(RulesError::Diverged(
                "the closure in the store differs from the recorded digest".to_owned(),
            ));
        }
        let wanted: BTreeSet<&String> = d.support.iter().collect();
        let mut statements = Vec::new();
        for t in view.triples(None, None, None).map_err(tm)? {
            let eid = t.eid.to_string();
            if !wanted.contains(&eid) {
                continue;
            }
            statements.push(Stmt {
                eid,
                s: Term::from_tm(&view.decode(t.s).map_err(tm)?)?,
                p: Term::from_tm(&view.decode(t.p).map_err(tm)?)?,
                o: Term::from_tm(&view.decode(t.o).map_err(tm)?)?,
            });
        }
        if statements.len() != d.support.len() {
            return Err(RulesError::Diverged(format!(
                "{} of {} supporting statements are readable as of the decision",
                statements.len(),
                d.support.len()
            )));
        }
        Ok(RuleBundle {
            format: BUNDLE_FORMAT.to_owned(),
            derivation: d.clone(),
            programs: sources
                .into_iter()
                .map(|(iri, text)| ProgramSource { iri, text })
                .collect(),
            statements,
        })
    }

    /// Re-runs the decision in a fresh store with no shard, no decider and no network: the
    /// closure digest must match, and the rule must conclude the recorded event from exactly the
    /// recorded statements.
    // @tg: implements:: [[openspec:rule-decider#Bundle Includes Rule Closure]]
    pub fn verify(&self, policy: &RulePolicy) -> Result<(), RulesError> {
        let d = &self.derivation;
        if d.engine != policy.engine {
            return Err(RulesError::EngineMismatch {
                recorded: d.engine.clone(),
                running: policy.engine.clone(),
            });
        }
        let sources: Vec<(String, String)> = self
            .programs
            .iter()
            .map(|p| (p.iri.clone(), p.text.clone()))
            .collect();
        if digest_of(&sources) != d.closure_digest {
            return Err(RulesError::Diverged(
                "the program closure does not match the recorded digest".to_owned(),
            ));
        }
        let ids: BTreeSet<&String> = self.statements.iter().map(|s| &s.eid).collect();
        if d.support.iter().any(|e| !ids.contains(e)) {
            return Err(RulesError::Diverged(
                "a supporting statement is missing from the bundle".to_owned(),
            ));
        }

        let dir = tempfile::tempdir().map_err(|e| RulesError::Store(e.to_string()))?;
        let db = Db::open(dir.path().join("verify.db"), OpenOptions::default()).map_err(tm)?;
        // Includes must exist before the program that accumulates them: store until done.
        let mut pending: Vec<&ProgramSource> = self.programs.iter().collect();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|p| {
                db.transact(TxOptions::default(), |t| {
                    t.store_elpi_program(TmValue::iri(p.iri.clone()), &p.text)?;
                    Ok(())
                })
                .is_err()
            });
            if pending.len() == before {
                return Err(RulesError::Program(
                    "the bundled programs do not load".to_owned(),
                ));
            }
        }
        db.transact(TxOptions::default(), |t| {
            for s in &self.statements {
                t.assert(
                    s.s.to_tm().map_err(|e| TmError::custom(e.to_string()))?,
                    s.p.to_tm().map_err(|e| TmError::custom(e.to_string()))?,
                    s.o.to_tm().map_err(|e| TmError::custom(e.to_string()))?,
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .map_err(tm)?;

        let view = db.now();
        let opts = ElpiOptions {
            fuel: d.fuel_limit,
            ..ElpiOptions::default()
        };
        let r = view
            .elpi_conclude(&ElpiProgram::Stored(TmValue::iri(d.program.clone())), &opts)
            .map_err(|e| match e {
                TmError::ElpiOutOfFuel { .. } => RulesError::FuelExhausted {
                    limit: d.fuel_limit,
                },
                e => tm(e),
            })?;
        let node = TmValue::iri(d.node.clone());
        let decision = TmValue::iri(vocab::DECISION);
        let c = r
            .conclusions
            .iter()
            .find(|c| c.s == node && c.p == decision && event_of(&c.o).as_deref() == Some(&d.event))
            .ok_or_else(|| {
                RulesError::Diverged(format!(
                    "the rule does not conclude {} for {}",
                    d.event, d.node
                ))
            })?;

        // Compare supports by content: statement ids differ between stores.
        let fresh: std::collections::BTreeMap<String, (Term, Term, Term)> = view
            .triples(None, None, None)
            .map_err(tm)?
            .into_iter()
            .map(|t| {
                Ok((
                    t.eid.to_string(),
                    (
                        Term::from_tm(&view.decode(t.s).map_err(tm)?)?,
                        Term::from_tm(&view.decode(t.p).map_err(tm)?)?,
                        Term::from_tm(&view.decode(t.o).map_err(tm)?)?,
                    ),
                ))
            })
            .collect::<Result<_, RulesError>>()?;
        let got: BTreeSet<(Term, Term, Term)> = c
            .support
            .iter()
            .filter_map(|e| fresh.get(&e.to_string()).cloned())
            .collect();
        let want: BTreeSet<(Term, Term, Term)> = self
            .statements
            .iter()
            .filter(|s| d.support.contains(&s.eid))
            .map(|s| (s.s.clone(), s.p.clone(), s.o.clone()))
            .collect();
        if got != want {
            return Err(RulesError::Diverged(
                "the rule rests on different statements than recorded".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Replays `derivations` under the program text `text`, each against the store as of its own
/// transaction, and reports where the decision would differ. Nothing is written: the original
/// journal and rules are untouched.
// @tg: implements:: [[openspec:rule-decider#Rule Version Counterfactual]]
pub fn counterfactual(
    shard: &TiramemsuShard,
    derivations: &[Derivation],
    text: &str,
    policy: &RulePolicy,
) -> Result<CounterfactualReport, RulesError> {
    let mut report = CounterfactualReport::default();
    for d in derivations {
        let view = shard.db().as_of(TimeRef::Tx(d.tx));
        let node = TmValue::iri(d.node.clone());
        // The offer in offer order: the statements were written in that order.
        let mut offered: Vec<(tiramemsu::Eid, String)> = Vec::new();
        if let (Some(n), Some(p)) = (
            view.encode(&node).map_err(tm)?,
            view.encode(&TmValue::iri(vocab::OFFERED)).map_err(tm)?,
        ) {
            for t in view.triples(Some(n), Some(p), None).map_err(tm)? {
                if let Some(e) = event_of(&view.decode(t.o).map_err(tm)?) {
                    offered.push((t.eid, e));
                }
            }
        }
        offered.sort_by_key(|(e, _)| *e);
        let opts = ElpiOptions {
            fuel: policy.fuel,
            ..ElpiOptions::default()
        };
        let outcome = view.elpi_conclude(&ElpiProgram::Text(text.to_owned()), &opts);
        let (event, support, note) = match outcome {
            Ok(r) => {
                let mine: Vec<(String, Vec<String>)> = r
                    .conclusions
                    .iter()
                    .filter(|c| c.s == node && c.p == TmValue::iri(vocab::DECISION))
                    .filter_map(|c| {
                        event_of(&c.o)
                            .map(|e| (e, c.support.iter().map(|s| s.to_string()).collect()))
                    })
                    .collect();
                match offered
                    .iter()
                    .find_map(|(_, e)| mine.iter().find(|(m, _)| m == e))
                {
                    Some((e, s)) => (Some(e.clone()), s.clone(), None),
                    None if mine.is_empty() => {
                        (None, Vec::new(), Some("no-applicable-rule".to_owned()))
                    }
                    None => (None, Vec::new(), Some("not-offered".to_owned())),
                }
            }
            Err(TmError::ElpiOutOfFuel { .. }) => {
                (None, Vec::new(), Some("fuel-exhausted".to_owned()))
            }
            Err(e) => return Err(tm(e)),
        };
        if event.as_deref() == Some(&d.event) {
            report.unchanged += 1;
        } else {
            report.differs.push(Divergence {
                node: d.node.clone(),
                recorded_event: d.event.clone(),
                counterfactual_event: event,
                counterfactual_support: support,
                note,
            });
        }
    }
    Ok(report)
}
