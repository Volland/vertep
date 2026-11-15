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
//! with [`RulesError::Unavailable`] and nothing is linked.
//!
//! @tg: implements:: [[decisions#Rules live per shard, in the journal's file]] {confidence: partial}
//! @tg: implements:: [[decisions#The native rules engine is optional behind a feature]]

mod error;
mod types;

#[cfg(feature = "native")]
mod decider;

pub use error::RulesError;
pub use types::{Derivation, ENGINE, RulePolicy};

#[cfg(feature = "native")]
pub use decider::{RuleDecider, put_program, verify};

#[cfg(not(feature = "native"))]
mod unavailable;
#[cfg(not(feature = "native"))]
pub use unavailable::{RuleDecider, put_program, verify};

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
    pub const EVENT_PREFIX: &str = "urn:vt:event:";
    /// The prefix of a context field predicate, `urn:vt:ctx:<field>`.
    pub const CTX_FIELD_PREFIX: &str = "urn:vt:ctx:";
    /// The prefix of a decision node, `urn:vt:decision:<correlation>`.
    pub const NODE_PREFIX: &str = "urn:vt:decision:";
}
