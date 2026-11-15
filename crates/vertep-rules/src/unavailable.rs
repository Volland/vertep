//! The crate without the engine: constructors fail with a typed error and never fall back.

use std::sync::Arc;

use vertep_tiramemsu::TiramemsuShard;

use crate::{Derivation, RulePolicy, RulesError};

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
