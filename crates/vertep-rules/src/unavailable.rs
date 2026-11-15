//! The crate without the engine: constructors fail with a typed error and never fall back.

use std::sync::Arc;

use vertep_tiramemsu::TiramemsuShard;

use crate::{CounterfactualReport, Derivation, RuleBundle, RulePolicy, RulesError};

/// A rules decider cannot be built here.
#[derive(Debug)]
pub struct RuleDecider {
    _private: (),
}

impl RuleDecider {
    /// Always [`RulesError::Unavailable`] without the `native` feature
    /// ([[openspec:rule-programs#Native Library Is Optional]]).
    // @tg: implements:: [[openspec:rule-programs#Native Library Is Optional]]
    pub fn new(
        _shard: Arc<TiramemsuShard>,
        _program: &str,
        _policy: RulePolicy,
    ) -> Result<Self, RulesError> {
        Err(RulesError::Unavailable)
    }
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn put_program(_shard: &TiramemsuShard, _program: &str, _text: &str) -> Result<(), RulesError> {
    Err(RulesError::Unavailable)
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn verify(
    _shard: &TiramemsuShard,
    _derivation: &Derivation,
    _policy: &RulePolicy,
) -> Result<(), RulesError> {
    Err(RulesError::Unavailable)
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn counterfactual(
    _shard: &TiramemsuShard,
    _derivations: &[Derivation],
    _text: &str,
    _policy: &RulePolicy,
) -> Result<CounterfactualReport, RulesError> {
    Err(RulesError::Unavailable)
}

impl RuleBundle {
    /// Always [`RulesError::Unavailable`] without the `native` feature.
    pub fn export(_shard: &TiramemsuShard, _d: &Derivation) -> Result<RuleBundle, RulesError> {
        Err(RulesError::Unavailable)
    }

    /// Always [`RulesError::Unavailable`] without the `native` feature.
    pub fn verify(&self, _policy: &RulePolicy) -> Result<(), RulesError> {
        Err(RulesError::Unavailable)
    }
}

/// What a trial run of a stored program found (never produced without the engine).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    /// Conclusions over the current data.
    pub conclusions: usize,
    /// Steps used.
    pub fuel_used: u64,
    /// True when the program may be non-monotonic.
    pub non_monotonic: bool,
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn program_text(
    _shard: &TiramemsuShard,
    _program: &str,
    _tx: Option<u64>,
) -> Result<String, RulesError> {
    Err(RulesError::Unavailable)
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn check_program(
    _shard: &TiramemsuShard,
    _program: &str,
    _fuel: u64,
) -> Result<CheckReport, RulesError> {
    Err(RulesError::Unavailable)
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn enforce(_shard: &TiramemsuShard, _program: &str, _on: bool) -> Result<(), RulesError> {
    Err(RulesError::Unavailable)
}

/// What a materialization run did (never produced without the engine).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Materialized {
    /// Statements derived by this run.
    pub derived: Vec<String>,
    /// Conclusions that were already derived from the same support.
    pub existing: usize,
    /// Earlier conclusions this run no longer derives, retracted.
    pub retracted: Vec<String>,
    /// True when the program may be non-monotonic.
    pub non_monotonic: bool,
}

/// Always [`RulesError::Unavailable`] without the `native` feature.
pub fn materialize(
    _shard: &TiramemsuShard,
    _program: &str,
    _fuel: u64,
) -> Result<Materialized, RulesError> {
    Err(RulesError::Unavailable)
}
