//! The CEL datamodel for Vertep: a deterministic, side-effect-free, always-terminating and
//! statically checked datamodel for YAML charts and charts written by untrusted authors.
//!
//! [`CelDatamodel`] implements [`vertep_core::datamodel::Datamodel`] under the name
//! [`CEL`] (`datamodel="cel"`). [`load`] parses and type-checks every expression of a chart
//! against its [`TypeEnv`] and applies a loading [`Policy`]; [`CelRegistry`] lets a host
//! register the datamodel; [`YAML_DEFAULT_DATAMODEL`] is the datamodel a YAML chart that
//! names none uses.
//!
//! The interpreter is this crate's own (task 6.1 of the M1 change): no existing Rust CEL
//! implementation had exact decimals, evaluation limits, defined map order together with
//! `no_std`. It is `no_std` with `alloc`, uses integer arithmetic for `decimal`, timestamps
//! and durations, keeps maps in key order, and depends only on `vertep-core` and
//! `regex-automata` (a linear-time `matches`).
//!
//! @tg: implements:: [[decisions#Pluggable datamodel]]

#![no_std]

extern crate alloc;

mod check;
mod convert;
mod datamodel;
mod decimal;
mod error;
mod eval;
mod funcs;
mod json;
mod load;
mod policy;
mod syntax;
mod time;
mod types;
mod value;

use alloc::boxed::Box;

use vertep_core::datamodel::{Datamodel, DatamodelRegistry};

pub use check::{CheckError, event_type, session_type};
pub use datamodel::{CEL, CelDatamodel, SNAPSHOT_FORMAT};
pub use decimal::{Decimal, DecimalError};
pub use error::{CelError, ErrorKind, Limits};
pub use load::{LoadError, LoadErrors, LoadOptions, load};
pub use policy::{Policy, check as check_policy, function_iris};
pub use time::{Duration, Timestamp};
pub use types::{CelType, RecordType, TypeEnv, UnionType, assignable};
pub use value::{CelMap, CelValue, Key, from_value, to_value};

/// The datamodel of a YAML chart that declares none.
// @tg: implements:: [[openspec:cel-datamodel#CEL Datamodel Selection]]
pub const YAML_DEFAULT_DATAMODEL: &str = CEL;

/// A fresh, untyped CEL datamodel for `"cel"`, and `None` for every other name.
#[must_use]
pub fn create(name: &str) -> Option<Box<dyn Datamodel>> {
    (name == CEL).then(|| Box::new(CelDatamodel::default()) as Box<dyn Datamodel>)
}

/// A registry offering exactly the CEL datamodel (see [`create`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct CelRegistry;

impl DatamodelRegistry for CelRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        create(name)
    }
}
