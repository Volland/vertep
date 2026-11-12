//! Chart IR, validation and the pure SCXML step kernel for Vertep.
//!
//! `vertep-core` is `no_std` with `alloc` and has no I/O, no async and no clock: every
//! interaction with the outside world is an effect returned by the step function.
//!
//! The pipeline is: build or read a [`ir::Chart`], give anonymous states identifiers with
//! [`identity::assign_generated_ids`], compile it with [`compile::CompiledChart::new`] (which runs
//! [`validate::validate`]), pick the datamodel with [`null::create_datamodel`], then call
//! [`session::start`] once and [`session::step`] for every input. Wrap the datamodel in a
//! [`tape::TapeDatamodel`] to record the answer tape for the Lean oracle.
//!
//! @tg: implements:: [[decisions#IR is the hub]]
//! @tg: implements:: [[decisions#SCXML semantics are normative]]
//! @tg: implements:: [[decisions#No-std core and tokio shell]]
//! @tg: implements:: [[decisions#Pluggable datamodel]]

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod bitset;
pub mod compile;
pub mod datamodel;
pub mod identity;
pub mod ir;
pub mod kernel;
pub mod null;
pub mod order;
pub mod session;
pub mod tape;
pub mod trace;
pub mod types;
pub mod validate;

#[cfg(test)]
mod tests;
