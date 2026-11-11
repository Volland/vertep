//! CEL errors and evaluation limits.

use alloc::string::String;
use core::fmt;

/// What kind of failure a [`CelError`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The expression does not parse.
    Syntax,
    /// A name or function that the environment does not declare.
    Undeclared,
    /// A static type error.
    Type,
    /// A map key or record field that is absent.
    NoSuchKey,
    /// A list index outside the list.
    IndexOutOfRange,
    /// Division or remainder by zero.
    DivisionByZero,
    /// An arithmetic result outside its type's range.
    Overflow,
    /// A failed conversion (`int('x')`, a runtime type mismatch on assignment).
    Conversion,
    /// No overload of a function or operator accepts the argument types.
    NoMatchingOverload,
    /// An argument outside a function's domain (a bad regular expression or time zone).
    InvalidArgument,
    /// The evaluation step limit was exceeded.
    StepLimit,
    /// The value size limit was exceeded.
    SizeLimit,
    /// A location that cannot be assigned (a system variable, an undeclared item).
    Location,
    /// An operation the CEL datamodel does not have (`<script>`).
    Unsupported,
}

/// A CEL failure. The datamodel reports it as an `EvalError`, which the interpreter turns
/// into `error.execution`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CelError {
    /// The kind of failure.
    pub kind: ErrorKind,
    /// A human-readable reason.
    pub message: String,
}

impl CelError {
    /// An error of the given kind.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        CelError {
            kind,
            message: message.into(),
        }
    }

    /// Whether the error is an exceeded limit, which no operator may absorb.
    pub fn is_limit(&self) -> bool {
        matches!(self.kind, ErrorKind::StepLimit | ErrorKind::SizeLimit)
    }
}

impl fmt::Display for CelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Bounds on evaluation cost ([[openspec:cel-datamodel#Guaranteed Termination]]).
///
/// Every evaluated node costs one step, every comprehension iteration one more, and string,
/// bytes and list operations one step per 64 bytes or elements they touch. The value size of
/// a string or bytes value is its length, of a list or map its element count; the deep size
/// of a result counts nodes plus bytes. Both are counted the same way on every target, so a
/// limit is exceeded deterministically.
// @tg: implements:: [[openspec:cel-datamodel#Guaranteed Termination]]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Evaluation steps allowed for one expression.
    pub max_steps: u64,
    /// The largest value an expression may produce or a location may hold.
    pub max_value_size: usize,
    /// The deepest expression nesting accepted by the parser.
    pub max_depth: u32,
    /// The longest expression source text, in bytes.
    pub max_expr_len: usize,
    /// The deepest nesting of lists and maps in a produced or stored value.
    pub max_value_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_steps: 100_000,
            max_value_size: 1 << 20,
            max_depth: 64,
            max_expr_len: 1 << 16,
            max_value_depth: 128,
        }
    }
}
