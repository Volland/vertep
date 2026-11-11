//! The W3C SCXML `ecmascript` datamodel for Vertep, on the boa engine.
//!
//! With the `ecmascript` feature (on by default) this crate provides [`EcmaDatamodel`], an
//! implementation of [`vertep_core::datamodel::Datamodel`], and [`EcmaRegistry`], which a host
//! can use to register it. Without the feature the crate is empty apart from [`create`], which
//! then knows no datamodel, so charts that require `ecmascript` are refused.
//!
//! Choices this implementation makes where W3C SCXML B.2 leaves room:
//!
//! - Inline content that `JSON.parse` accepts becomes that value; anything else becomes a
//!   string whose runs of XML whitespace are collapsed to single spaces and trimmed.
//! - `In(id)` answers for state ids that appear as string literals in chart expressions or
//!   scripts that mention `In`; a computed id never seen as a literal is an evaluation error,
//!   because the trait's [`StateQuery`](vertep_core::datamodel::StateQuery) cannot be kept
//!   past the call that supplies it.
//! - Values leave the engine as `JSON.stringify` would shape them, except that `undefined` is
//!   kept; functions become `undefined`, cycles and symbols are evaluation errors.
//!
//! @tg: implements:: [[decisions#Pluggable datamodel]]

#[cfg(feature = "ecmascript")]
mod ecma;

#[cfg(feature = "ecmascript")]
pub use ecma::{DEFAULT_LOOP_ITERATION_LIMIT, EcmaDatamodel, SNAPSHOT_FORMAT};

use vertep_core::datamodel::{Datamodel, DatamodelRegistry};

/// The datamodel name used in the `datamodel` attribute.
pub const ECMASCRIPT: &str = "ecmascript";

/// A fresh datamodel for `name`: the ECMAScript datamodel for `"ecmascript"` when the
/// `ecmascript` feature is enabled, and `None` otherwise.
#[must_use]
pub fn create(name: &str) -> Option<Box<dyn Datamodel>> {
    #[cfg(feature = "ecmascript")]
    if name == ECMASCRIPT {
        return ecma::EcmaDatamodel::try_new()
            .ok()
            .map(|dm| Box::new(dm) as Box<dyn Datamodel>);
    }
    let _ = name;
    None
}

/// A registry offering exactly the datamodels of this crate (see [`create`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct EcmaRegistry;

impl DatamodelRegistry for EcmaRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        create(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // @tg: verifies:: [[openspec:datamodel#ECMAScript Datamodel#Feature disabled]] {level: unit}
    #[test]
    fn ecmascript_is_registered_only_with_its_feature() {
        let created = EcmaRegistry.create(ECMASCRIPT);
        if cfg!(feature = "ecmascript") {
            let dm = created.expect("the ecmascript feature registers the datamodel");
            assert_eq!(dm.name(), ECMASCRIPT);
        } else {
            assert!(
                created.is_none(),
                "without the feature the datamodel is refused"
            );
        }
        assert!(EcmaRegistry.create("xpath").is_none());
        assert!(EcmaRegistry.create("null").is_none());
    }
}
