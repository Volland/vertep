//! Parse errors and the reader's failure type.

use core::fmt;

use vertep_core::ir::Span;

use crate::loss::LossReport;

/// The class of a [`ParseError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input is not well-formed XML or violates the XML namespaces rules. Reading stops at
    /// the first such error.
    Xml,
    /// The root element is not `scxml`.
    NotScxml,
    /// The root element is `scxml` in a namespace other than the SCXML namespace.
    WrongNamespace,
    /// The `version` attribute is not `1.0`.
    UnsupportedVersion,
    /// An element in the SCXML namespace that SCXML 1.0 does not define.
    UnknownElement,
    /// An SCXML element in a place where SCXML does not allow it.
    UnexpectedElement,
    /// Non-whitespace text where SCXML allows only elements.
    UnexpectedText,
    /// A required attribute is absent.
    MissingAttribute,
    /// A required child element is absent.
    MissingElement,
    /// Two attributes (or an attribute and a child) that exclude each other are both present.
    ExclusiveAttributes,
    /// An attribute value outside its allowed set (for example `type="sideways"`).
    InvalidValue,
    /// A child element that may occur at most once occurs again.
    DuplicateElement,
    /// A `src` attribute that no resolver resolved ([[openspec:scxml-format#External Content Resolution]]).
    UnresolvedSource,
    /// The `datamodel` attribute names a datamodel the caller did not register.
    UnsupportedDatamodel,
    /// Two states declare the same `id`.
    DuplicateStateId,
    /// A transition target or `initial` identifier names no state.
    UnknownTarget,
    /// A `vt:type` annotation or the `vt:types` block does not parse, or names a type that is
    /// not declared ([[openspec:type-system#Typed SCXML Attributes]]).
    Type,
}

/// One problem found while reading an SCXML document, with the location of the element (or,
/// for [`ErrorKind::Xml`], of the offending byte).
// @tg: implements:: [[openspec:scxml-format#Parse Error Reporting]]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// What went wrong.
    pub kind: ErrorKind,
    /// Where it went wrong. Unlike IR spans, compare this field explicitly with
    /// `span.line` and `span.column`: [`Span`] equality is always true.
    pub span: Span,
    /// A human-readable explanation naming the element, attribute or value involved.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Why [`crate::read`] returned no chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The document has errors. Every error the reader could recover from is listed, in
    /// document order, together with the losses seen up to the point reading stopped.
    Invalid {
        /// The errors.
        errors: Vec<ParseError>,
        /// The losses found while reading.
        losses: LossReport,
    },
    /// Strict mode was requested and the document would lose content
    /// ([[openspec:scxml-format#Declared Loss Report]]).
    Lossy(LossReport),
}

impl ReadError {
    /// The parse errors; empty for [`ReadError::Lossy`].
    pub fn errors(&self) -> &[ParseError] {
        match self {
            ReadError::Invalid { errors, .. } => errors,
            ReadError::Lossy(_) => &[],
        }
    }

    /// The loss report gathered while reading.
    pub fn losses(&self) -> &LossReport {
        match self {
            ReadError::Invalid { losses, .. } => losses,
            ReadError::Lossy(losses) => losses,
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Invalid { errors, .. } => {
                write!(f, "{} error(s) in SCXML document", errors.len())?;
                for e in errors {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            ReadError::Lossy(report) => {
                write!(
                    f,
                    "strict mode: {} construct(s) would be lost",
                    report.items.len()
                )?;
                for l in &report.items {
                    write!(f, "\n  {l}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ReadError {}
