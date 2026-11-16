//! Rule programs beside the journal ([[openspec:rule-programs#Rule Programs Live In The Shard]],
//! [[openspec:rule-decider#Rule Decider]]).
//!
//! A rule program is an ELPI program stored as a node in the shard's tiramemsu file. A
//! [`RuleDecider`] writes the offer and the actor's context as statements, runs the program
//! under a step budget and answers with the event its conclusion names. The statements the
//! conclusion rests on are the rationale: a [`Derivation`], kept apart from a decider's claim.
//!
//! Everything that needs the engine is behind the `native` feature (off by default, because the
//! native library is large and exists for four platforms). Without it every constructor fails
//! with [`RulesError::Unavailable`] and nothing is linked. [`retract_erased`] and
//! [`RetractOnErasure`] need only the store, so they work either way.
//!
//! @tg: implements:: [[decisions#Rules live per shard, in the journal's file]] {confidence: partial}
//! @tg: implements:: [[decisions#The native rules engine is optional behind a feature]]
//! @tg: implements:: [[decisions#Erasure retracts what depends on the erased]]

mod bundle;
mod certificate;
mod erasure;
mod error;
mod profile;
mod types;

#[cfg(feature = "native")]
pub mod authoring;
mod authoring_types;
#[cfg(not(feature = "native"))]
pub mod authoring {
    //! Without the engine only the types exist; every operation returns an `unavailable` error.
    pub use crate::authoring_types::*;
    use crate::{CounterfactualReport, Derivation, RulePolicy};
    use vertep_tiramemsu::TiramemsuShard;

    fn unavailable() -> GateError {
        GateError {
            code: "unavailable",
            clause: None,
            reason: crate::RulesError::Unavailable.to_string(),
        }
    }

    /// Always unavailable without the `native` feature.
    pub fn propose(
        _: &TiramemsuShard,
        _: &str,
        _: &str,
        _: &Actor<'_>,
        _: &str,
        _: &[String],
        _: &GateContext<'_>,
    ) -> Result<String, GateError> {
        Err(unavailable())
    }

    /// Always unavailable without the `native` feature.
    pub fn shadow(
        _: &TiramemsuShard,
        _: &str,
        _: &[Derivation],
        _: &RulePolicy,
    ) -> Result<CounterfactualReport, GateError> {
        Err(unavailable())
    }

    /// Always unavailable without the `native` feature.
    pub fn promote(
        _: &TiramemsuShard,
        _: &str,
        _: Stage,
        _: &Actor<'_>,
        _: Option<&str>,
        _: &GateContext<'_>,
    ) -> Result<(), GateError> {
        Err(unavailable())
    }
}
#[cfg(feature = "native")]
mod decider;
#[cfg(feature = "native")]
mod replay;
#[cfg(feature = "native")]
pub mod tools;
#[cfg(feature = "native")]
mod totality;

pub use bundle::{
    BUNDLE_FORMAT, CounterfactualReport, Divergence, ProgramSource, RuleBundle, Stmt, Term,
};
pub use certificate::{CertReport, check_certificate, lean_certificate};
pub use erasure::{ErasureRetraction, RetractOnErasure, retract_erased};
pub use error::RulesError;
pub use profile::{HOST_BUILTINS, ProfileViolation, check_profile, clauses};
pub use types::{DERIVED_GRAPH_SUFFIX, Derivation, ENGINE, RulePolicy, Totality};

#[cfg(feature = "native")]
pub use decider::{
    CheckReport, Materialized, RuleDecider, check_program, enforce, materialize, program_text,
    put_program, verify,
};
#[cfg(feature = "native")]
pub use replay::counterfactual;
#[cfg(feature = "native")]
pub use totality::{MAX_CANDIDATES, totality};

#[cfg(not(feature = "native"))]
mod unavailable;
#[cfg(not(feature = "native"))]
pub use unavailable::{
    CheckReport, Materialized, RuleDecider, check_program, counterfactual, enforce, materialize,
    program_text, put_program, totality, verify,
};

/// The IRIs a rule program reads. Programs are written against these names.
pub mod vocab {
    /// `(node urn:vt:offered urn:vt:event:<name>)`: an offered candidate.
    pub const OFFERED: &str = "urn:vt:offered";
    /// `(urn:vt:ctx urn:vt:deciding <node>)`: the decision being made; every rule starts here.
    pub const CTX: &str = "urn:vt:ctx";
    /// The predicate of [`CTX`].
    pub const DECIDING: &str = "urn:vt:deciding";
    /// `(node urn:vt:decision urn:vt:event:<name>)`: a conclusion.
    pub const DECISION: &str = "urn:vt:decision";
    /// `(node urn:vt:actor "<iri>")`.
    pub const ACTOR: &str = "urn:vt:actor";
    /// `(node urn:vt:state "<id>")`.
    pub const STATE: &str = "urn:vt:state";
    /// The prefix of an event IRI.
    /// The event name a program concludes to refuse on purpose.
    pub const REFUSE: &str = "refuse";
    /// The prefix of an event IRI.
    pub const EVENT_PREFIX: &str = "urn:vt:event:";
    /// The prefix of a context field predicate, `urn:vt:ctx:<field>`.
    pub const CTX_FIELD_PREFIX: &str = "urn:vt:ctx:";
    /// The prefix of a decision node, `urn:vt:decision:<correlation>`.
    pub const NODE_PREFIX: &str = "urn:vt:decision:";
}
