//! Erasure retracts what depends on the erased
//! ([[openspec:journal-redaction#Erasure Retracts Derived Facts]]).
//!
//! The journal seals a subject's sensitive fields under the subject's key; rules read statements
//! in the shard, which are not sealed. So in the store, *the subject's statements* are the live,
//! asserted (not derived) statements that name the subject's IRI, as subject or as object, outside
//! the engine's `sys:` namespace. The subject is the string a [`SealPolicy`] reads as the data
//! subject, the same string that names the key, taken as an IRI.
//!
//! Erasing the subject retracts those statements in one recorded transaction. The store's
//! cascade takes with them every derived fact whose support includes one of them, and every
//! derived fact resting on such a fact, transitively. The premises go too, so that the next
//! materialization cannot derive the facts again from data that is meant to be gone. A derived
//! fact whose support is only other subjects' statements stays live, even when it is about the
//! erased subject. History keeps the retracted rows, as for every retraction in tiramemsu.
//!
//! [`SealPolicy`]: vertep_agent::redaction::SealPolicy

use std::collections::BTreeSet;
use std::sync::Arc;

use vertep_agent::redaction::ErasureHook;
use vertep_tiramemsu::TiramemsuShard;
use vertep_tiramemsu::tiramemsu::{Eid, ObjectId, TxOptions, Value, View, vocab};

use crate::RulesError;

/// What erasing a subject retracted from one shard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErasureRetraction {
    /// The subject's own statements, retracted, as the store names them (`e12`).
    pub statements: Vec<String>,
    /// The derived facts that rested on them, directly or through another derived fact,
    /// retracted with them. These are what the `erasure` entry lists.
    pub derived: Vec<String>,
}

fn store(e: impl std::fmt::Display) -> RulesError {
    RulesError::Store(e.to_string())
}

/// True when `eid` was derived: it has a `sys:derivedFrom` link to its support.
fn is_derived(
    view: &View<'_>,
    derived_from: Option<ObjectId>,
    eid: Eid,
) -> Result<bool, RulesError> {
    let Some(p) = derived_from else {
        return Ok(false);
    };
    Ok(!view
        .triples(Some(eid.oid()), Some(p), None)
        .map_err(store)?
        .is_empty())
}

/// The subject's live statements and the derived facts resting on them, in `view`.
fn affected(view: &View<'_>, subject: &str) -> Result<(Vec<Eid>, Vec<Eid>), RulesError> {
    let Some(id) = view.encode(&Value::iri(subject)).map_err(store)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let derived_from = view
        .encode(&Value::iri(vocab::SYS_DERIVED_FROM))
        .map_err(store)?;
    let mut rows = view.triples(Some(id), None, None).map_err(store)?;
    rows.extend(view.triples(None, None, Some(id)).map_err(store)?);
    let mut own = BTreeSet::new();
    for t in rows {
        let reserved = matches!(
            view.decode(t.p).map_err(store)?,
            Value::Iri(p) if p.starts_with(vocab::SYS) || p.starts_with(vocab::TM)
        );
        if !reserved && !is_derived(view, derived_from, t.eid)? {
            own.insert(t.eid);
        }
    }
    let mut derived = BTreeSet::new();
    for e in &own {
        for d in view.dependents(*e).map_err(store)? {
            if !own.contains(&d) && is_derived(view, derived_from, d)? {
                derived.insert(d);
            }
        }
    }
    Ok((own.into_iter().collect(), derived.into_iter().collect()))
}

/// Retracts `subject`'s statements from `shard`, and with them every derived fact whose support
/// includes one of them, transitively, in one transaction. Derived facts resting only on other
/// subjects' statements stay live. Returns what was retracted; nothing is retracted, and
/// nothing returned, for a subject the shard does not know.
///
/// It needs no rules engine: it reads the support the store recorded when the facts were
/// materialized.
// @tg: implements:: [[openspec:journal-redaction#Erasure Retracts Derived Facts]]
pub fn retract_erased(
    shard: &TiramemsuShard,
    subject: &str,
) -> Result<ErasureRetraction, RulesError> {
    let (own, derived) = affected(&shard.db().now(), subject)?;
    if own.is_empty() {
        return Ok(ErasureRetraction::default());
    }
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            for e in own.iter().chain(&derived) {
                // a derived fact is normally already gone with its premise; `false` says so
                t.retract(*e)?;
            }
            Ok(())
        })
        .map_err(store)?;
    Ok(ErasureRetraction {
        statements: own.iter().map(ToString::to_string).collect(),
        derived: derived.iter().map(ToString::to_string).collect(),
    })
}

/// The [`ErasureHook`] that retracts an erased subject's statements and the derived facts
/// resting on them from a shard; install it with
/// [`SealingJournal::with_erasure_hook`](vertep_agent::redaction::SealingJournal::with_erasure_hook).
/// The `erasure` entry then lists the retracted derived facts.
#[derive(Clone)]
pub struct RetractOnErasure {
    shard: Arc<TiramemsuShard>,
}

impl std::fmt::Debug for RetractOnErasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetractOnErasure")
            .field("shard", &self.shard.path())
            .finish()
    }
}

impl RetractOnErasure {
    /// A hook over `shard`, the shard the rules decider and materialization write to.
    pub fn new(shard: Arc<TiramemsuShard>) -> Self {
        RetractOnErasure { shard }
    }
}

impl ErasureHook for RetractOnErasure {
    // @tg: implements:: [[openspec:journal-redaction#Erasure Retracts Derived Facts]]
    fn on_erase(&self, subject: &str) -> Result<Vec<String>, String> {
        retract_erased(&self.shard, subject)
            .map(|r| r.derived)
            .map_err(|e| e.to_string())
    }
}
