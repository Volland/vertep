//! The YAML surface for Vertep charts: a thin, lossless mirror of the chart IR
//! ([[decisions#IR is the hub]]).
//!
//! [`read`] turns a YAML 1.2 document into a [`vertep_core::ir::Chart`], reporting every error
//! at once with file, line, column and a dotted path such as `states.checkout.on.pay[1].target`.
//! [`write`] turns a chart back into canonical, byte-stable YAML; `read(write(c)) == c` for
//! every chart with unique state identifiers.
//!
//! # Parser
//!
//! The reader is built on the event API of [`saphyr_parser`]: a pure-Rust, fully YAML 1.2
//! compliant parser that reports a source span for every event. Building a small node tree
//! from the events keeps mapping keys in document order (document order is chart semantics:
//! it orders states, regions and transitions) and gives every node its line and column. The
//! `serde`-based YAML crates were rejected because they lose positions or order, or follow
//! YAML 1.1 scalar resolution (`on: yes` as a boolean), which would turn the `on` key of every
//! state into `true`.
//!
//! # Document
//!
//! ```yaml
//! vertep: 1                 # format version, required
//! name: order               # required; null for an unnamed chart
//! datamodel: cel            # optional, default cel
//! binding: late             # optional, default early
//! initial: idle             # optional; a state id or a list of ids
//! types:                    # optional named types
//!   Pay: { amount: decimal }
//!   Payment: { oneOf: { Card: { last4: string }, Cash: null } }
//! events: { start: null, pay: Pay }
//! context:                  # top-level <datamodel>
//!   total: { type: decimal, init: "0" }
//! states:
//!   idle:
//!     on: { start: checkout }
//!   checkout:
//!     initial: cart
//!     entry: [ { log: { expr: "ctx.total" } } ]
//!     states:
//!       cart:
//!         on:
//!           pay:
//!             - { cond: "ev.data.amount > 0", target: payment,
//!                 actions: [ { assign: { location: ctx.total, expr: "ev.data.amount" } } ] }
//!             - { target: cart }
//!       payment: { always: [ { cond: "ctx.total > 100", target: review } ] }
//!       review: { type: final }
//! ```
//!
//! States are keyed by id under `states` (document order is kept) and take `type`
//! (`parallel`, `final`, `history` with `history: shallow|deep`), `initial`, `entry`, `exit`,
//! `on`, `always`, `invoke`, `context`, `donedata` and nested `states`. A transition is a
//! target string, a list of targets, or a mapping with `target`, `cond`, `type` and `actions`;
//! a list of mappings is several transitions in order. Executable content items are mappings
//! with exactly one of `assign`, `raise`, `send`, `log`, `if`, `foreach`, `cancel`, `script`,
//! `call` (a `vt:call` extension element) and `extension` (any other extension element).
//!
//! Every IR distinction has a spelling, so the mirror is lossless: `extensions` lists on every
//! node keep `vt:` and foreign extension nodes; `generated: true` marks generated state ids; a
//! `transitions` list (with `event`) keeps any transition order that `on`/`always` grouping
//! would change; `{ value: X, implicit: true }` spells a `datamodel`, `binding` or transition
//! `type` that the source did not state; `initial_element` keeps an `<initial>` element; and
//! parameters and context entries switch to list form when names repeat. Types ride on
//! `vt:` extension nodes (see [`vertep_core::types`]); `type` on context entries, `payload_type`
//! on `send`, `type` on list-form `params` and the `types`/`events` blocks are their spellings.

mod node;
mod read;
mod types_yaml;
mod write;

use std::fmt;

use vertep_core::ir::Chart;
use vertep_core::types::ExprTyper;

/// The YAML format versions this crate reads and writes.
pub const SUPPORTED_VERSIONS: &[u32] = &[1];

/// The datamodel of a YAML chart that does not name one.
pub const DEFAULT_DATAMODEL: &str = "cel";

/// A syntax error found by a datamodel's expression parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionError {
    /// Byte offset of the error inside the expression.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

/// Parses expression strings of a datamodel so that loading reports syntax errors at their
/// position inside the YAML string. A datamodel crate (CEL, ECMAScript) implements it.
// @tg: implements:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel]] {confidence: partial}
pub trait ExpressionChecker {
    /// Check `expr` in the language of `datamodel`. Return `Ok(())` for datamodels the
    /// checker does not know.
    fn check(&self, datamodel: &str, expr: &str) -> Result<(), ExpressionError>;
}

/// How [`read`] treats a document.
#[derive(Clone, Copy)]
pub struct ReadOptions<'a> {
    /// The file path shown in diagnostics.
    pub file: Option<&'a str>,
    /// Parses expression strings in the chart's datamodel. Default: none.
    pub expressions: Option<&'a dyn ExpressionChecker>,
    /// Check state identifiers (duplicates, unknown targets) and run the core
    /// well-formedness validator. Default: true.
    pub validate: bool,
    /// Run the core type checker on typed charts. Default: true.
    pub typecheck: bool,
    /// The expression typer for type checking. Default:
    /// [`vertep_core::types::BasicTyper::for_datamodel`] of the chart's datamodel.
    pub typer: Option<&'a dyn ExprTyper>,
}

impl Default for ReadOptions<'_> {
    fn default() -> Self {
        ReadOptions {
            file: None,
            expressions: None,
            validate: true,
            typecheck: true,
            typer: None,
        }
    }
}

impl<'a> ReadOptions<'a> {
    /// Options that only lower the document: no validation and no type checking.
    pub fn lower_only() -> Self {
        ReadOptions {
            validate: false,
            typecheck: false,
            ..ReadOptions::default()
        }
    }

    /// Name the file in diagnostics.
    pub fn with_file(self, file: &'a str) -> Self {
        ReadOptions {
            file: Some(file),
            ..self
        }
    }

    /// Parse expressions with `checker`.
    pub fn with_expressions(self, checker: &'a dyn ExpressionChecker) -> Self {
        ReadOptions {
            expressions: Some(checker),
            ..self
        }
    }

    /// Type-check expressions with `typer`.
    pub fn with_typer(self, typer: &'a dyn ExprTyper) -> Self {
        ReadOptions {
            typer: Some(typer),
            ..self
        }
    }
}

/// What kind of problem a diagnostic reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// YAML syntax: the document is not well-formed YAML.
    Syntax,
    /// A key, value or shape the chart format does not allow.
    Schema,
    /// The format version is missing or unsupported.
    Version,
    /// A state identifier is duplicated or a target names no state.
    Reference,
    /// The datamodel's parser rejected an expression.
    Expression,
    /// A core well-formedness violation.
    Validation,
    /// A type error.
    Type,
}

/// One located problem in a YAML document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// What kind of problem.
    pub kind: ErrorKind,
    /// The file, when the caller named it.
    pub file: Option<String>,
    /// 1-based line, 0 when unknown.
    pub line: u32,
    /// 1-based column, 0 when unknown.
    pub column: u32,
    /// The dotted path to the offending node, such as `states.checkout.on.pay[1].target`.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.file.as_deref().unwrap_or("<yaml>"),
            self.line,
            self.column
        )?;
        if !self.path.is_empty() {
            write!(f, ": {}", self.path)?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A document that could not be loaded, with every problem found. No IR is produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    /// The problems, sorted by position.
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadError {}

/// Read a YAML chart document into the chart IR.
///
/// Returns the chart or every problem found: YAML syntax (which stops reading), schema errors,
/// version errors, duplicate state ids and unknown targets, expression syntax (with
/// [`ReadOptions::expressions`]), core validation and type errors, each with its location.
// @tg: implements:: [[openspec:yaml-format#YAML Chart Document]]
// @tg: implements:: [[openspec:yaml-format#Source-Located Errors]]
pub fn read(input: &str, options: &ReadOptions<'_>) -> Result<Chart, LoadError> {
    read::read(input, options)
}

/// [`read`] with default options.
pub fn read_str(input: &str) -> Result<Chart, LoadError> {
    read(input, &ReadOptions::default())
}

/// Why a chart cannot be written as YAML.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Two states share an identifier; `states` mappings need unique keys.
    DuplicateStateId(String),
    /// The written document would not read back to the same chart (a bug in the writer).
    NotLossless,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::DuplicateStateId(id) => {
                write!(
                    f,
                    "state id `{id}` is used more than once; YAML needs unique ids"
                )
            }
            WriteError::NotLossless => f.write_str("the YAML form would not read back losslessly"),
        }
    }
}

impl std::error::Error for WriteError {}

/// Write a chart as canonical YAML: block style, two-space indentation, keys in a fixed order,
/// states and transitions in IR order, strings quoted only when needed. The output is
/// byte-stable, and reading it gives back an equal chart (checked before returning).
// @tg: implements:: [[openspec:yaml-format#Lossless YAML Round Trip]]
pub fn write(chart: &Chart) -> Result<String, WriteError> {
    write::write(chart)
}
