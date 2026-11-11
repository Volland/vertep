//! Per-macrostep trace records.
//!
//! Every [`crate::session::start`] and [`crate::session::step`] call returns one
//! [`MacrostepTrace`]: the input it processed, each event it consumed with the transitions that
//! event (or an eventless selection) fired, the configuration afterwards and the effects it
//! emitted. Rust and the Lean oracle produce these records independently and the differential
//! tests compare them for equality. See [`crate::tape`] for the JSON Lines exchange format.

use alloc::string::String;
use alloc::vec::Vec;

use crate::datamodel::Event;
use crate::session::Effect;

/// A declared transition, identified by its source state and its index in that state's
/// transition list in the IR.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransitionRef {
    /// The source state identifier.
    pub source: String,
    /// Index in the source's `transitions`.
    pub index: usize,
}

/// What a macrostep was started by.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TraceInput {
    /// Session start, with the session identifier.
    Start {
        /// `_sessionid`.
        sessionid: String,
    },
    /// An external event.
    Event(Event),
    /// A cancel request from the host or parent session.
    Cancel,
}

/// One selection round inside a macrostep.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MicrostepTrace {
    /// The event consumed (`None` for an eventless selection).
    pub event: Option<String>,
    /// The transitions fired, in the order they were selected; empty when the event enabled
    /// nothing.
    pub transitions: Vec<TransitionRef>,
}

/// The record of one macrostep (one `start` or `step` call).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MacrostepTrace {
    /// 0 for the start macrostep, then 1, 2, ...
    pub index: u64,
    /// What started the macrostep.
    pub input: TraceInput,
    /// Events consumed and transitions fired, in order.
    pub microsteps: Vec<MicrostepTrace>,
    /// Active state identifiers after the macrostep, in document order.
    pub configuration: Vec<String>,
    /// Effects emitted, in order.
    pub effects: Vec<Effect>,
    /// Whether the session terminated in this macrostep.
    pub terminated: bool,
}
