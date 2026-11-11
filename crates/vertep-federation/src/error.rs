//! The failures a cross-trust send reports to the sending chart as `error.communication`.

use std::fmt;

use vertep_core::datamodel::{Event, EventType, Value};

/// Why a cross-trust send failed. Every variant becomes `error.communication` in the sender.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommunicationFailure {
    /// No resolver handles the DID method.
    UnsupportedMethod(String),
    /// The address is not a valid DID.
    InvalidAddress(String),
    /// Resolution failed: network error, timeout, missing document or failed verification.
    ResolutionFailed(String),
    /// The resolved DID document has no service for any transport the runtime supports.
    NoCompatibleEndpoint,
    /// The sender holds no grant to send this event to the target.
    NotGranted(String),
    /// The message could not be sealed (missing keys, key management failure).
    SealFailed(String),
    /// The recipient refused (relationship refused, intermediary refused to route).
    Refused(String),
    /// Every endpoint stayed unreachable.
    Unreachable(String),
}

impl fmt::Display for CommunicationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommunicationFailure::UnsupportedMethod(m) => {
                write!(f, "unsupported DID method did:{m}")
            }
            CommunicationFailure::InvalidAddress(a) => write!(f, "invalid DID address {a}"),
            CommunicationFailure::ResolutionFailed(m) => write!(f, "DID resolution failed: {m}"),
            CommunicationFailure::NoCompatibleEndpoint => f.write_str(
                "no compatible endpoint: the DID document has no service for a supported transport",
            ),
            CommunicationFailure::NotGranted(m) => write!(f, "not granted: {m}"),
            CommunicationFailure::SealFailed(m) => write!(f, "cannot seal message: {m}"),
            CommunicationFailure::Refused(m) => write!(f, "refused by recipient: {m}"),
            CommunicationFailure::Unreachable(m) => write!(f, "unreachable: {m}"),
        }
    }
}

/// A failed cross-trust send, with the send id the chart used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommunicationError {
    /// The reason.
    pub failure: CommunicationFailure,
    /// The `sendid` of the failed `<send>`, if any.
    pub send_id: Option<String>,
}

impl CommunicationError {
    /// A failure without a send id.
    pub fn new(failure: CommunicationFailure) -> Self {
        CommunicationError {
            failure,
            send_id: None,
        }
    }

    /// This failure for the send `send_id`.
    pub fn for_send(mut self, send_id: impl Into<String>) -> Self {
        self.send_id = Some(send_id.into());
        self
    }

    /// The `error.communication` platform event the sending chart receives: the predefined
    /// error payload `{kind: "communication", message}` and the original send id.
    pub fn to_event(&self) -> Event {
        let mut e = Event::named("error.communication", EventType::Platform);
        e.sendid = self.send_id.clone();
        e.data = Value::Object(vec![
            ("kind".into(), Value::String("communication".into())),
            ("message".into(), Value::String(self.failure.to_string())),
        ]);
        e
    }
}

impl fmt::Display for CommunicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.send_id {
            Some(id) => write!(f, "send {id}: {}", self.failure),
            None => self.failure.fmt(f),
        }
    }
}

impl std::error::Error for CommunicationError {}

impl From<CommunicationFailure> for CommunicationError {
    fn from(failure: CommunicationFailure) -> Self {
        CommunicationError::new(failure)
    }
}
