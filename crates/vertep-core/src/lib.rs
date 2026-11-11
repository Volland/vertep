//! Chart IR, validation and the pure SCXML step kernel for Vertep.
//!
//! `vertep-core` is `no_std` with `alloc` and has no I/O, no async and no clock: every
//! interaction with the outside world is an effect returned by the step function.
//!
//! @tg: implements:: [[decisions#IR is the hub]]
//! @tg: implements:: [[decisions#SCXML semantics are normative]]
//! @tg: implements:: [[decisions#No-std core and tokio shell]]
//! @tg: implements:: [[decisions#Pluggable datamodel]]

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod datamodel;
pub mod identity;
pub mod ir;
