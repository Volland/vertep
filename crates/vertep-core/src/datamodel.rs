//! The datamodel contract shared by every datamodel (`null`, `ecmascript`, later `cel` and
//! `rust`), the datamodel-independent [`Value`], and the host-supplied registry.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A datamodel-independent value, used for event data, snapshots and traces.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// No value (`undefined` in ECMAScript).
    #[default]
    Undefined,
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    String(String),
    /// An ordered array.
    Array(Vec<Value>),
    /// An object with keys in insertion order.
    Object(Vec<(String, Value)>),
}

/// A failed datamodel operation. The interpreter turns it into `error.execution`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EvalError {
    /// A human-readable reason.
    pub message: String,
}

impl EvalError {
    /// An error with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        EvalError {
            message: message.into(),
        }
    }
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The `_event.type` field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EventType {
    /// Raised by the platform (errors, `done.*`).
    Platform,
    /// Raised with `<raise>` or `<send target="#_internal">`.
    Internal,
    /// Received from outside the session.
    #[default]
    External,
}

/// An SCXML event as seen by the datamodel in `_event`.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Event {
    /// `name`.
    pub name: String,
    /// `type`.
    pub type_: EventType,
    /// `sendid`.
    pub sendid: Option<String>,
    /// `origin`.
    pub origin: Option<String>,
    /// `origintype`.
    pub origintype: Option<String>,
    /// `originverified`: true only when the shell verified the sender's authentication against
    /// its resolved DID document before the event entered the session, so `origin` is that
    /// sender's DID. False for every other event, including runtime-assigned origins inside a
    /// trusted cluster and anonymous senders. Absent in older journals and tapes, where it reads
    /// as false.
    #[cfg_attr(feature = "serde", serde(default))]
    pub originverified: bool,
    /// `invokeid`.
    pub invokeid: Option<String>,
    /// `data`.
    pub data: Value,
}

impl Event {
    /// An event with only a name and type.
    pub fn named(name: impl Into<String>, type_: EventType) -> Self {
        Event {
            name: name.into(),
            type_,
            ..Event::default()
        }
    }

    /// This event with `origin` set to a sender DID the shell verified, and the verified flag
    /// set ([[openspec:did-identity#Authenticated Event Origin]]).
    // @tg: implements:: [[openspec:did-identity#Authenticated Event Origin]]
    pub fn with_verified_origin(mut self, sender: impl Into<String>) -> Self {
        self.origin = Some(sender.into());
        self.originverified = true;
        self
    }
}

/// Values for the read-only session system variables.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SessionInfo {
    /// `_sessionid`.
    pub sessionid: String,
    /// `_name`.
    pub name: Option<String>,
    /// `_ioprocessors`.
    pub ioprocessors: Value,
}

/// Host-supplied sources of nondeterminism for one input
/// ([[openspec:datamodel#Deterministic Evaluation]]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Environment {
    /// Current time in milliseconds since the Unix epoch, as recorded in the input.
    pub now_ms: i64,
    /// Seed for the datamodel's random number generator.
    pub seed: u64,
}

/// Answers the `In(stateId)` predicate against the current configuration.
pub trait StateQuery {
    /// True exactly when the state with this identifier is active.
    fn in_state(&self, id: &str) -> bool;

    /// The identifiers of every active state in document order, when the caller can list
    /// them. Datamodels that must answer `In()` from a callback they cannot borrow (such as an
    /// embedded engine's native function) copy this before evaluating; `None` means only
    /// [`StateQuery::in_state`] is available.
    fn active_states(&self) -> Option<Vec<String>> {
        None
    }
}

/// The contract every datamodel implements. No operation may panic on bad input: failures are
/// [`EvalError`]s, which the interpreter turns into `error.execution`.
// @tg: implements:: [[openspec:datamodel#Datamodel Contract]]
pub trait Datamodel {
    /// The datamodel name as used in the `datamodel` attribute.
    fn name(&self) -> &str;

    /// Prepare a fresh session: set the system variables `_sessionid`, `_name` and
    /// `_ioprocessors`. Called once before any other operation.
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError>;

    /// Set the sources of nondeterminism for the input about to be processed.
    fn set_environment(&mut self, env: &Environment);

    /// Create a data item with an undefined value.
    fn declare(&mut self, id: &str) -> Result<(), EvalError>;

    /// Bind `_event` to the event currently being processed.
    fn set_event(&mut self, event: &Event);

    /// Evaluate a condition. Must not change the datamodel ([[openspec:datamodel#Guard Purity]]).
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError>;

    /// Evaluate a value expression.
    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError>;

    /// Interpret inline content (of `<data>`, `<assign>`, `<content>`) as a value.
    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError>;

    /// Assign the value of an expression to a location (`<assign location expr>`).
    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError>;

    /// Assign a value to a location. The location must already exist.
    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError>;

    /// Bind a variable, declaring it when it does not exist (used by `foreach` item and index).
    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError>;

    /// Whether `location` is a legal, existing location (used by `foreach` and `idlocation`).
    fn is_location(&mut self, location: &str) -> bool;

    /// Execute a script.
    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError>;

    /// Evaluate a `foreach` array expression to its items, shallow-copied, in index order.
    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError>;

    /// Serialize the complete datamodel state ([[openspec:datamodel#Datamodel Snapshot]]).
    fn snapshot(&self) -> Value;

    /// Restore a state produced by [`Datamodel::snapshot`].
    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError>;
}

/// Creates datamodels by name; supplied by the host
/// ([[openspec:datamodel#Datamodel Registry]]).
pub trait DatamodelRegistry {
    /// A fresh datamodel instance for `name`, or `None` when it is not registered.
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>>;
}
