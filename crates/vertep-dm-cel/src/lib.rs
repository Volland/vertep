//! The CEL datamodel for Vertep: a deterministic, side-effect-free, always-terminating and
//! statically checked datamodel for YAML charts and charts written by untrusted authors.
//!
//! [`CelDatamodel`] implements [`vertep_core::datamodel::Datamodel`] under the name
//! [`CEL`] (`datamodel="cel"`). [`load`] parses and type-checks every expression of a chart
//! against its [`TypeEnv`] and applies a loading [`Policy`]; [`type_env`] projects a chart's
//! types to that environment and [`instantiate`] is the typed load every host goes through;
//! [`CelRegistry`] lets a host register the datamodel and creates it typed for a typed chart;
//! [`YAML_DEFAULT_DATAMODEL`] is the datamodel a YAML chart that names none uses.
//!
//! The interpreter is this crate's own (task 6.1 of the M1 change): no existing Rust CEL
//! implementation had exact decimals, evaluation limits, defined map order together with
//! `no_std`. It is `no_std` with `alloc`, uses integer arithmetic for `decimal`, timestamps
//! and durations, keeps maps in key order, and depends only on `vertep-core` and
//! `regex-automata` (a linear-time `matches`).
//!
//! @tg: implements:: [[decisions#Pluggable datamodel]]
//! @tg: implements:: [[decisions#Hosts instantiate CEL charts typed]]

#![no_std]

extern crate alloc;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

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
mod typed;
mod types;
mod value;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use vertep_core::datamodel::{Datamodel, DatamodelRegistry};
use vertep_core::ir::Chart;

pub use check::{CheckError, event_type, session_type};
pub use datamodel::{CEL, CelDatamodel, SNAPSHOT_FORMAT};
pub use decimal::{Decimal, DecimalError};
pub use error::{CelError, ErrorKind, Limits};
pub use load::{LoadError, LoadErrors, LoadOptions, load};
pub use policy::{Policy, check as check_policy, function_iris};
pub use time::{Duration, Timestamp};
pub use typed::{cel_type, instantiate, type_env};
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

/// A registry offering exactly the CEL datamodel. By name ([`create`]) it is untyped; for a
/// chart ([`DatamodelRegistry::create_for`], which hosts use) it is [`instantiate`]d: typed
/// and statically checked when the chart declares types, and refused with every type error.
#[derive(Debug, Default, Clone, Copy)]
pub struct CelRegistry;

impl DatamodelRegistry for CelRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        create(name)
    }

    // @tg: implements:: [[openspec:cel-datamodel#Static Type Checking of CEL]]
    fn create_for(&self, chart: &Chart) -> Option<Result<Box<dyn Datamodel>, Vec<String>>> {
        if chart.datamodel != CEL {
            return None;
        }
        Some(
            instantiate(chart, Limits::default())
                .map(|dm| Box::new(dm) as Box<dyn Datamodel>)
                .map_err(|errs| errs.0.iter().map(ToString::to_string).collect()),
        )
    }

    fn check(&self, chart: &Chart) -> Result<(), Vec<String>> {
        if chart.datamodel != CEL {
            return Ok(());
        }
        instantiate(chart, Limits::default())
            .map(|_| ())
            .map_err(|errs| errs.0.iter().map(ToString::to_string).collect())
    }
}
