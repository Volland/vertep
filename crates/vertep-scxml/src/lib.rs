//! SCXML 1.0 reader and writer for Vertep charts.
//!
//! [`read`] turns an SCXML document into a [`vertep_core::ir::Chart`] with the source location
//! of every element, a [`LossReport`] of what the IR does not keep, and every recoverable
//! error at once. [`write`] turns a chart back into SCXML. The two are inverses on the IR:
//! `read(write(c)) == c` for every chart the writer can represent, and
//! `read(write(read(d))) == read(d)` for every document ([`check_round_trip`],
//! [`check_chart_round_trip`]).
//!
//! # Mapping
//!
//! - Elements and attributes in the SCXML namespace map to IR nodes. The reader accepts every
//!   child the IR can hold on every state element (it is structurally permissive); semantic
//!   rules such as "no transitions out of a final state" belong to the core validator. It does
//!   check identifiers itself: duplicate `id`s and references to unknown states are errors
//!   with the location of the offending element (see [`ReadOptions::check_references`]).
//! - Attributes and elements in the Vertep namespace [`VT_NS`] and in any foreign namespace
//!   become [`vertep_core::ir::Extension`] nodes on their owner. Extension attributes are
//!   kept sorted by namespace and local name, so attribute order does not matter; extension
//!   elements keep their index among the owner's element children (counting only children
//!   the IR represents) and their XML, serialized self-contained: namespace bindings inherited
//!   from outside are declared on the element, relative to a context whose default namespace
//!   is SCXML. Inside executable content an extension element is an
//!   [`vertep_core::ir::ExecKind::Extension`] whose position is its index in the block.
//! - Inline content of `<data>`, `<assign>` and `<content>` is kept as text when it has no
//!   child elements, and as serialized XML (elements as above, text escaped, whitespace-only
//!   text between elements dropped) when it does. Whitespace-only content is absent.
//! - `src` on `<script>`, `<data>`, `<content>` and `<invoke>` goes through the caller's
//!   [`SourceResolver`]; there is no default I/O.
//! - The `datamodel` attribute is checked against the caller's [`DatamodelSet`]; `null` is
//!   always accepted and is the default.
//! - Anonymous states get identifiers from
//!   [`vertep_core::identity::assign_generated_ids`]. A declared identifier that is exactly
//!   the generated identifier of its own position is read as generated, so that a written
//!   generated identifier reads back as generated.
//! - Types ride on `vt:` extensions ([[decisions#Types ride on vt extension nodes]]):
//!   `vt:type` on `<data>` and `<send>` stays on the node, `vt:type` on `<param>` goes to the
//!   owner's `vt:paramTypes` record (and the writer puts it back on each `<param>`), and the
//!   `vt:types` element stays on the chart. Annotations and the block are read in canonical
//!   form; one that does not parse or names an undeclared type is an [`ErrorKind::Type`] error
//!   suggesting the closest declared name. A processor that ignores the `vt:` namespace runs
//!   the same document untyped.
//!
//! @tg: implements:: [[decisions#IR is the hub]]
//! @tg: implements:: [[decisions#Full SCXML first]]

pub mod datamodels;
pub mod error;
mod ids;
pub mod loss;
mod reader;
pub mod resolve;
mod text;
mod typing;
mod writer;

use vertep_core::identity::assign_generated_ids;
use vertep_core::ir::Chart;
pub use vertep_core::ir::{SCXML_NS, VT_NS};

pub use datamodels::{AnyDatamodel, DatamodelSet, Registered};
pub use error::{ErrorKind, ParseError, ReadError};
pub use loss::{ExtensionOrigin, Loss, LossKind, LossReport};
pub use resolve::{
    DeferResolver, MapResolver, Resolution, ResolveError, SourceKind, SourceRequest, SourceResolver,
};
pub use text::LineIndex;
pub use writer::{WriteError, write};

/// How [`read`] treats datamodels, external sources, losses and references.
#[derive(Clone, Copy)]
pub struct ReadOptions<'a> {
    /// The datamodels a document may name. Default: only `null`.
    pub datamodels: &'a dyn DatamodelSet,
    /// Resolves `src` attributes. Default: none, so every `src` is an error.
    pub resolver: Option<&'a dyn SourceResolver>,
    /// Fail with [`ReadError::Lossy`] when the loss report is not empty. Default: false.
    pub strict: bool,
    /// Report duplicate state identifiers and unknown transition targets and `initial`
    /// identifiers as errors. Default: true.
    pub check_references: bool,
}

impl Default for ReadOptions<'static> {
    fn default() -> Self {
        ReadOptions {
            datamodels: &datamodels::NULL_ONLY,
            resolver: None,
            strict: false,
            check_references: true,
        }
    }
}

impl<'a> ReadOptions<'a> {
    /// Default options.
    pub fn new() -> ReadOptions<'static> {
        ReadOptions::default()
    }

    /// Accept the datamodels in `set`.
    pub fn with_datamodels<'b>(self, set: &'b dyn DatamodelSet) -> ReadOptions<'b>
    where
        'a: 'b,
    {
        ReadOptions {
            datamodels: set,
            ..self
        }
    }

    /// Resolve `src` attributes with `resolver`.
    pub fn with_resolver<'b>(self, resolver: &'b dyn SourceResolver) -> ReadOptions<'b>
    where
        'a: 'b,
    {
        ReadOptions {
            resolver: Some(resolver),
            ..self
        }
    }

    /// Set strict mode.
    pub fn strict(self, strict: bool) -> Self {
        ReadOptions { strict, ..self }
    }

    /// Enable or disable the reference checks.
    pub fn check_references(self, check: bool) -> Self {
        ReadOptions {
            check_references: check,
            ..self
        }
    }
}

/// A successfully read document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    /// The chart.
    pub chart: Chart,
    /// What the chart does not keep.
    pub losses: LossReport,
}

/// Read an SCXML 1.0 document into the chart IR
/// ([[openspec:scxml-format#SCXML Document Parsing]]).
///
/// Returns the chart and its loss report, or every error found. Malformed XML stops reading
/// at the first XML error; all SCXML-level errors (unknown elements, missing or exclusive
/// attributes, bad values, unresolved sources, unregistered datamodels, unknown targets) are
/// collected. Errors are sorted by position.
// @tg: implements:: [[openspec:scxml-format#SCXML Document Parsing]]
pub fn read(input: &str, options: &ReadOptions<'_>) -> Result<Parsed, ReadError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut parser = reader::Parser::new(input, options);
    let chart = parser.document();
    let (mut errors, losses) = parser.finish();
    let Some(mut chart) = chart else {
        return Err(ReadError::Invalid { errors, losses });
    };
    assign_generated_ids(&mut chart);
    ids::recognize_generated(&mut chart);
    let xml_ok = !errors.iter().any(|e| e.kind == ErrorKind::Xml);
    if xml_ok {
        errors.extend(typing::normalize(&mut chart));
    }
    if options.check_references && xml_ok {
        errors.extend(ids::check_references(&chart));
    }
    errors.sort_by_key(|e| (e.span.line, e.span.column));
    if !errors.is_empty() {
        Err(ReadError::Invalid { errors, losses })
    } else if options.strict && !losses.is_empty() {
        Err(ReadError::Lossy(losses))
    } else {
        Ok(Parsed { chart, losses })
    }
}

/// [`read`] with default options.
pub fn read_str(input: &str) -> Result<Parsed, ReadError> {
    read(input, &ReadOptions::default())
}

/// A failed round-trip check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundTripError {
    /// The input document could not be read.
    Read(ReadError),
    /// The chart could not be written.
    Write(WriteError),
    /// The written document could not be read back.
    Reread {
        /// The written document.
        written: String,
        /// Why reading it failed.
        error: ReadError,
    },
    /// Reading the written document gave a different chart.
    Mismatch {
        /// The chart before writing.
        expected: Box<Chart>,
        /// The chart read back.
        actual: Box<Chart>,
        /// The written document.
        written: String,
    },
    /// Writing the chart read back gave a different document.
    Unstable {
        /// The first serialization.
        first: String,
        /// The second serialization.
        second: String,
    },
}

impl core::fmt::Display for RoundTripError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RoundTripError::Read(e) => write!(f, "input not readable: {e}"),
            RoundTripError::Write(e) => write!(f, "{e}"),
            RoundTripError::Reread { written, error } => {
                write!(f, "written document not readable: {error}\n{written}")
            }
            RoundTripError::Mismatch { written, .. } => {
                write!(f, "chart read back differs from the original\n{written}")
            }
            RoundTripError::Unstable { first, second } => {
                write!(f, "serialization is not stable\n{first}\n---\n{second}")
            }
        }
    }
}

impl std::error::Error for RoundTripError {}

/// Check that `chart` survives SCXML: `read(write(chart)) == chart` and writing the result
/// again gives the same text ([[openspec:scxml-format#SCXML Round Trip]]).
///
/// `options` must accept the chart's datamodel and resolve (or defer) its `src` attributes;
/// with a resolver that resolves, the chart's resolved content must match what the resolver
/// returns. Returns the written document.
// @tg: implements:: [[openspec:scxml-format#SCXML Round Trip]]
pub fn check_chart_round_trip(
    chart: &Chart,
    options: &ReadOptions<'_>,
) -> Result<String, RoundTripError> {
    let written = write(chart).map_err(RoundTripError::Write)?;
    let back = read(&written, options)
        .map_err(|error| RoundTripError::Reread {
            written: written.clone(),
            error,
        })?
        .chart;
    if back != *chart {
        return Err(RoundTripError::Mismatch {
            expected: Box::new(chart.clone()),
            actual: Box::new(back),
            written,
        });
    }
    let again = write(&back).map_err(RoundTripError::Write)?;
    if again != written {
        return Err(RoundTripError::Unstable {
            first: written,
            second: again,
        });
    }
    Ok(written)
}

/// Check that a document's chart survives SCXML: read it, then apply
/// [`check_chart_round_trip`]. Returns the chart read from `input`.
pub fn check_round_trip(input: &str, options: &ReadOptions<'_>) -> Result<Chart, RoundTripError> {
    let chart = read(input, options).map_err(RoundTripError::Read)?.chart;
    check_chart_round_trip(&chart, options)?;
    Ok(chart)
}
