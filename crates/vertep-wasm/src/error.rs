//! Errors of the command bridge, each with a stable code that the TypeScript client turns
//! into a typed error.

use core::fmt;

use serde::Serialize;
use serde_json::{Value as J, json};

/// The stable error codes of the JavaScript API. `WorkerTerminated` exists only on the client
/// side (the worker cannot report its own death) and is listed here so both sides share one
/// list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Code {
    /// A chart source failed to read or validate.
    ChartInvalid,
    /// The requested storage is not available in this environment.
    StorageUnavailable,
    /// Another worker holds the store open.
    StoreLocked,
    /// No reply arrived within a request's timeout.
    RequestTimeout,
    /// The worker hosting the actor system died.
    WorkerTerminated,
    /// The transport refused the connection credential.
    TransportRefused,
    /// A call named something that does not exist (an actor, a chart type, a subscription).
    NotFound,
    /// A call's arguments are malformed.
    InvalidArgument,
    /// An actor could not be activated or replayed.
    Activation,
    /// A message could not be delivered.
    Delivery,
    /// The store failed.
    Store,
}

impl Code {
    /// The code as the client sees it.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::ChartInvalid => "ChartInvalid",
            Code::StorageUnavailable => "StorageUnavailable",
            Code::StoreLocked => "StoreLocked",
            Code::RequestTimeout => "RequestTimeout",
            Code::WorkerTerminated => "WorkerTerminated",
            Code::TransportRefused => "TransportRefused",
            Code::NotFound => "NotFound",
            Code::InvalidArgument => "InvalidArgument",
            Code::Activation => "Activation",
            Code::Delivery => "Delivery",
            Code::Store => "Store",
        }
    }
}

/// One problem of an invalid chart, with its location in the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// What is wrong.
    pub message: String,
    /// 1-based line, 0 when unknown.
    pub line: u32,
    /// 1-based column, 0 when unknown.
    pub column: u32,
    /// The offending node (a state id, a YAML path), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// An error of a bridge call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// The code.
    pub code: Code,
    /// A human-readable explanation.
    pub message: String,
    /// For [`Code::ChartInvalid`], every violation found.
    pub violations: Vec<Violation>,
}

impl ApiError {
    /// An error with `code` and `message`.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        ApiError {
            code,
            message: message.into(),
            violations: Vec::new(),
        }
    }

    /// An [`Code::InvalidArgument`] error.
    pub fn arg(message: impl Into<String>) -> Self {
        ApiError::new(Code::InvalidArgument, message)
    }

    /// A [`Code::ChartInvalid`] error listing `violations`.
    pub fn chart(violations: Vec<Violation>) -> Self {
        let message = violations
            .iter()
            .map(|v| {
                if v.line > 0 {
                    format!("{}:{}: {}", v.line, v.column, v.message)
                } else {
                    v.message.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        ApiError {
            code: Code::ChartInvalid,
            message: format!("chart is invalid: {message}"),
            violations,
        }
    }

    /// The error as the JSON object the client receives: `{code, message, violations?}`.
    pub fn to_json(&self) -> J {
        let mut o = json!({ "code": self.code.as_str(), "message": self.message });
        if !self.violations.is_empty() {
            o["violations"] = serde_json::to_value(&self.violations).unwrap_or(J::Null);
        }
        o
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ApiError {}

/// Shorthand for bridge results.
pub type Res<T> = Result<T, ApiError>;
