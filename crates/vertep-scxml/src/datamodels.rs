//! The set of datamodel names the caller has registered with the reader.

use std::collections::BTreeSet;

use vertep_core::datamodel::DatamodelRegistry;

/// The datamodels a document may name in its `datamodel` attribute
/// ([[openspec:scxml-format#Datamodel Attribute Handling]]).
///
/// The reader asks this set about every explicit `datamodel` value other than `null`, which
/// the core always provides. A document naming an unregistered datamodel is rejected with
/// [`crate::ErrorKind::UnsupportedDatamodel`]. Implemented for arrays of names
/// (`&["null", "ecmascript"]`), `Vec<String>`, `BTreeSet<String>`, [`AnyDatamodel`] (format
/// conversion) and [`Registered`] (a host's [`DatamodelRegistry`]).
// @tg: implements:: [[openspec:scxml-format#Datamodel Attribute Handling]]
pub trait DatamodelSet {
    /// Whether a datamodel with this name is available.
    fn is_registered(&self, name: &str) -> bool;
}

impl<const N: usize> DatamodelSet for [&str; N] {
    fn is_registered(&self, name: &str) -> bool {
        self.contains(&name)
    }
}

impl DatamodelSet for Vec<String> {
    fn is_registered(&self, name: &str) -> bool {
        self.iter().any(|n| n == name)
    }
}

impl DatamodelSet for BTreeSet<String> {
    fn is_registered(&self, name: &str) -> bool {
        self.contains(name)
    }
}

/// Accepts every datamodel name. For tools that convert charts between formats without
/// running them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnyDatamodel;

impl DatamodelSet for AnyDatamodel {
    fn is_registered(&self, _name: &str) -> bool {
        true
    }
}

/// Asks a host's datamodel registry. Probing creates (and drops) one datamodel instance.
pub struct Registered<'a>(pub &'a dyn DatamodelRegistry);

impl DatamodelSet for Registered<'_> {
    fn is_registered(&self, name: &str) -> bool {
        self.0.create(name).is_some()
    }
}

/// The default set: only `null`.
pub(crate) static NULL_ONLY: [&str; 1] = ["null"];
