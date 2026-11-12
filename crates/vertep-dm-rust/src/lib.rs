//! The `rust` datamodel for Vertep charts, and the runtime side of the `statechart!` and
//! `include_chart!` macros.
//!
//! A chart on the `rust` datamodel keeps its context in a Rust struct and evaluates guards and
//! actions by calling Rust functions bound to IRIs:
//!
//! - [`RustDatamodel`] implements the core [`vertep_core::datamodel::Datamodel`] contract over a
//!   [`Context`] (the generated `Ctx` struct, or [`DynContext`]), with typed system variables in
//!   [`Sys`].
//! - [`Registry`] maps guard and action IRIs to functions; it rejects duplicates and makes
//!   instantiation fail for unregistered IRIs.
//! - Guards are `fn(&Ctx, &P) -> bool` ([`GuardFn`]): no mutable context and no emitter.
//!   Actions are `fn(&mut Ctx, &P, &mut Emit<E>)`, `fn(&mut Ctx, &P)` or
//!   `fn(&mut Ctx, &P) -> Vec<E>` ([`IntoAction`]); their only effects are context changes and
//!   emitted events, which become `<raise>` and `<send>` in the step.
//! - [`lowering`] fixes how guards, actions and context appear in the IR, so SCXML, YAML and RDF
//!   charts on the `rust` datamodel bind to the same functions.
//! - [`Machine`] is a typed session over a generated chart ([`ChartSpec`]).
//!
//! The macros are re-exported here, so a crate needs only this dependency:
//!
//! ```
//! use vertep_dm_rust::{Decimal, statechart};
//!
//! statechart! {
//!     name: order,
//!     types { Pay = { amount: decimal } }
//!     context { total: decimal = 0 }
//!     events { start, pay: Pay }
//!     initial idle;
//!     state idle { on start => cart; }
//!     state cart { on pay if positive => paid / record; }
//!     final paid;
//! }
//!
//! fn positive(_: &Ctx, ev: &Pay) -> bool { ev.amount > Decimal::ZERO }
//! fn record(ctx: &mut Ctx, ev: &Pay, _: &mut Emit) { ctx.total = ev.amount; }
//!
//! let (mut m, _) = Machine::start("s1").unwrap();
//! m.send(Event::Start).unwrap();
//! m.send(Event::Pay { amount: "12.5".parse().unwrap() }).unwrap();
//! // `paid` is a top-level final state: the session has ended.
//! assert!(m.is_terminated());
//! assert_eq!(m.context().total.to_string(), "12.5");
//! ```
//!
//! @tg: implements:: [[decisions#Pluggable datamodel]]
//! @tg: implements:: [[decisions#Datamodel typers plug into one core checker]]

extern crate self as vertep_dm_rust;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod datamodel;
pub mod decimal;
pub mod emit;
pub mod lowering;
pub mod machine;
pub mod registry;
pub mod value;
pub mod version;

pub use datamodel::{Context, DynContext, RUST_DATAMODEL, RustDatamodel, RustRegistry, RustTyper};
pub use decimal::Decimal;
pub use emit::{ChartEvent, Emit, Emitted, SendOptions, Sys};
pub use machine::{ChartSpec, Machine, MachineError, StateEnum};
pub use registry::{GuardFn, IntoAction, Registry, RegistryError, chart_functions};
pub use value::{Datetime, FromEvent, FromValue, Iri, ToValue};
pub use vertep_core::datamodel::{EvalError, Event as RawEvent, Value};
pub use vertep_macros::{include_chart, statechart};

/// Paths used by generated code. Not a stable API.
#[doc(hidden)]
pub mod __private {
    pub use std::sync::LazyLock;
    pub use vertep_core;
}
