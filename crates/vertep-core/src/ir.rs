//! The chart IR: the canonical, surface-independent representation of a statechart.
//!
//! Every surface (SCXML, RDF, and later YAML and the Rust DSL) reads into and writes from
//! these types. The IR keeps expression text uninterpreted; it is evaluated only by a
//! [`crate::datamodel::Datamodel`]. Source positions are carried in [`Span`] values, which
//! compare equal to every other span so that derived equality is structural
//! ([[openspec:chart-model#Chart Equality]]).

use alloc::string::String;
use alloc::vec::Vec;

/// Uninterpreted expression text, evaluated by the chart's datamodel.
pub type Expr = String;

/// A source location in the document a node was read from.
///
/// Spans never take part in equality or hashing: two spans always compare equal.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    /// 1-based line, or 0 when unknown.
    pub line: u32,
    /// 1-based column, or 0 when unknown.
    pub column: u32,
}

impl Span {
    /// The span of a node that was not read from a document.
    pub const NONE: Span = Span { line: 0, column: 0 };

    /// A span at the given line and column.
    pub const fn new(line: u32, column: u32) -> Self {
        Span { line, column }
    }

    /// Whether this span carries a known position.
    pub const fn is_known(&self) -> bool {
        self.line != 0
    }
}

impl PartialEq for Span {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for Span {}
impl core::hash::Hash for Span {
    fn hash<H: core::hash::Hasher>(&self, _state: &mut H) {}
}

/// The Vertep extension namespace, used in SCXML documents and as the ontology namespace.
pub const VT_NS: &str = "https://w3id.org/vertep/vt#";
/// The SCXML namespace.
pub const SCXML_NS: &str = "http://www.w3.org/2005/07/scxml";

/// An attribute or element from the `vt:` namespace or a foreign namespace, kept on its owner.
// @tg: implements:: [[openspec:chart-model#Extension Nodes]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Extension {
    /// Namespace IRI.
    pub ns: String,
    /// Local name.
    pub local: String,
    /// Attribute value or element content.
    pub value: ExtensionValue,
    /// For elements: the index among the owner's element children at which it appeared.
    /// For attributes: 0.
    pub position: u32,
    /// Source location.
    pub span: Span,
}

/// The content of an extension node.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExtensionValue {
    /// An attribute and its value.
    Attribute(String),
    /// An element, kept as its serialized XML (including its own start and end tags).
    Element(String),
}

/// Binding mode for data items (W3C SCXML 5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Binding {
    /// All data items are created and initialized at session start.
    #[default]
    Early,
    /// Data items are created at session start and initialized on first entry of their state.
    Late,
}

/// The root of a chart: the `<scxml>` element.
// @tg: implements:: [[openspec:chart-model#Chart Document Structure]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Chart {
    /// The `name` attribute.
    pub name: Option<String>,
    /// The datamodel name; `"null"` when the document names none.
    pub datamodel: String,
    /// Whether the document stated the `datamodel` attribute explicitly.
    pub datamodel_explicit: bool,
    /// Binding mode.
    pub binding: Binding,
    /// Whether the document stated the `binding` attribute explicitly.
    pub binding_explicit: bool,
    /// The `initial` attribute: state identifiers, possibly empty.
    pub initial: Vec<String>,
    /// The top-level `<datamodel>` block.
    pub data: Vec<Data>,
    /// The optional top-level `<script>`.
    pub script: Option<Script>,
    /// Child states in document order.
    pub states: Vec<State>,
    /// Extension attributes and elements on `<scxml>`.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

impl Default for Chart {
    fn default() -> Self {
        Chart {
            name: None,
            datamodel: String::from("null"),
            datamodel_explicit: false,
            binding: Binding::Early,
            binding_explicit: false,
            initial: Vec::new(),
            data: Vec::new(),
            script: None,
            states: Vec::new(),
            extensions: Vec::new(),
            span: Span::NONE,
        }
    }
}

/// History depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HistoryType {
    /// Remembers the active children of the parent.
    #[default]
    Shallow,
    /// Remembers the active atomic descendants of the parent.
    Deep,
}

/// The element a state was written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StateElement {
    /// `<state>`; atomic or compound depending on its children.
    State,
    /// `<parallel>`.
    Parallel,
    /// `<final>`.
    Final,
    /// `<history>` with its type.
    History(HistoryType),
}

/// The classification of a state (W3C SCXML 3.1).
// @tg: implements:: [[openspec:chart-model#State Kinds]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StateKind {
    /// A `<state>` without child states.
    Atomic,
    /// A `<state>` with child states.
    Compound,
    /// A `<parallel>`.
    Parallel,
    /// A `<final>`.
    Final,
    /// A `<history>`.
    History(HistoryType),
}

/// A state node of any kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct State {
    /// The state identifier, declared or generated.
    pub id: String,
    /// True when `id` was generated because the document declared none.
    pub id_generated: bool,
    /// The element the state was written as.
    pub element: StateElement,
    /// The `initial` attribute (compound states only), possibly empty.
    pub initial: Vec<String>,
    /// The `<initial>` child element, if any.
    pub initial_element: Option<Initial>,
    /// Child states in document order (excluding the `<initial>` pseudo-state).
    pub children: Vec<State>,
    /// Outgoing transitions in document order. A history state has at most one: its default.
    pub transitions: Vec<Transition>,
    /// `<onentry>` handlers in document order.
    pub onentry: Vec<Block>,
    /// `<onexit>` handlers in document order.
    pub onexit: Vec<Block>,
    /// The state's `<datamodel>` block.
    pub data: Vec<Data>,
    /// `<invoke>` elements in document order.
    pub invokes: Vec<Invoke>,
    /// `<donedata>` (final states only).
    pub donedata: Option<DoneData>,
    /// Extension attributes and elements.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

impl State {
    /// A new state with the given identifier and element and nothing else.
    pub fn new(id: impl Into<String>, element: StateElement) -> Self {
        State {
            id: id.into(),
            id_generated: false,
            element,
            initial: Vec::new(),
            initial_element: None,
            children: Vec::new(),
            transitions: Vec::new(),
            onentry: Vec::new(),
            onexit: Vec::new(),
            data: Vec::new(),
            invokes: Vec::new(),
            donedata: None,
            extensions: Vec::new(),
            span: Span::NONE,
        }
    }

    /// The state's kind, derived from its element and children.
    pub fn kind(&self) -> StateKind {
        match self.element {
            StateElement::State if self.children.is_empty() => StateKind::Atomic,
            StateElement::State => StateKind::Compound,
            StateElement::Parallel => StateKind::Parallel,
            StateElement::Final => StateKind::Final,
            StateElement::History(h) => StateKind::History(h),
        }
    }
}

/// The `<initial>` pseudo-state of a compound state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Initial {
    /// Its single transition (no event, no condition).
    pub transition: Transition,
    /// Extension attributes and elements.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

/// Transition type (W3C SCXML 3.13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransitionType {
    /// Exits the source state when targets are its descendants (the default).
    #[default]
    External,
    /// Does not exit a compound source when all targets are its descendants.
    Internal,
}

/// A transition.
// @tg: implements:: [[openspec:chart-model#Transition Structure]]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transition {
    /// Event descriptors, possibly empty (an eventless transition).
    pub events: Vec<String>,
    /// The guard.
    pub cond: Option<Expr>,
    /// Target state identifiers, possibly empty (a targetless transition).
    pub targets: Vec<String>,
    /// Internal or external.
    pub kind: TransitionType,
    /// Whether the document stated `type` explicitly.
    pub kind_explicit: bool,
    /// The transition's executable content.
    pub body: Block,
    /// Extension attributes and elements.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

/// An ordered block of executable content.
pub type Block = Vec<Exec>;

/// One executable content element.
// @tg: implements:: [[openspec:chart-model#Executable Content Model]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Exec {
    /// The element.
    pub kind: ExecKind,
    /// Extension attributes on the element.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

impl Exec {
    /// An element without extensions or span.
    pub fn new(kind: ExecKind) -> Self {
        Exec {
            kind,
            extensions: Vec::new(),
            span: Span::NONE,
        }
    }
}

/// The executable content elements of SCXML.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExecKind {
    /// `<raise event>`.
    Raise {
        /// Event name.
        event: String,
    },
    /// `<send>`.
    Send(Send),
    /// `<cancel>`.
    Cancel {
        /// `sendid`.
        sendid: Option<String>,
        /// `sendidexpr`.
        sendidexpr: Option<Expr>,
    },
    /// `<log>`.
    Log {
        /// `label`.
        label: Option<String>,
        /// `expr`.
        expr: Option<Expr>,
    },
    /// `<assign>`.
    Assign {
        /// `location`.
        location: Expr,
        /// `expr`.
        expr: Option<Expr>,
        /// Inline content (raw text or XML).
        content: Option<String>,
    },
    /// `<script>`.
    Script(Script),
    /// `<if>` with its `<elseif>` and `<else>` branches, in document order. The `else`
    /// branch, if present, is last and has no condition.
    If {
        /// Branches.
        branches: Vec<IfBranch>,
    },
    /// `<foreach>`.
    Foreach {
        /// `array`.
        array: Expr,
        /// `item`.
        item: String,
        /// `index`.
        index: Option<String>,
        /// Body.
        body: Block,
    },
    /// An executable content element from the `vt:` or a foreign namespace.
    Extension(Extension),
}

/// One branch of an `<if>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IfBranch {
    /// The condition; `None` for `<else>`.
    pub cond: Option<Expr>,
    /// The branch body.
    pub body: Block,
    /// Source location of the `<if>`, `<elseif>` or `<else>`.
    pub span: Span,
}

/// `<send>` (W3C SCXML 6.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Send {
    /// `event`.
    pub event: Option<String>,
    /// `eventexpr`.
    pub eventexpr: Option<Expr>,
    /// `target`.
    pub target: Option<String>,
    /// `targetexpr`.
    pub targetexpr: Option<Expr>,
    /// `type`.
    pub type_: Option<String>,
    /// `typeexpr`.
    pub typeexpr: Option<Expr>,
    /// `id`.
    pub id: Option<String>,
    /// `idlocation`.
    pub idlocation: Option<Expr>,
    /// `delay`.
    pub delay: Option<String>,
    /// `delayexpr`.
    pub delayexpr: Option<Expr>,
    /// `namelist`, split on whitespace.
    pub namelist: Vec<String>,
    /// `<param>` children.
    pub params: Vec<Param>,
    /// `<content>` child.
    pub content: Option<Content>,
}

/// `<param>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Param {
    /// `name`.
    pub name: String,
    /// `expr`.
    pub expr: Option<Expr>,
    /// `location`.
    pub location: Option<Expr>,
    /// Source location.
    pub span: Span,
}

/// `<content>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Content {
    /// `expr`.
    pub expr: Option<Expr>,
    /// Inline children as raw text or XML.
    pub body: Option<String>,
    /// Source location.
    pub span: Span,
}

/// `<script>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Script {
    /// `src`, resolved by the reader's resolver into `body` when possible.
    pub src: Option<String>,
    /// Inline script text.
    pub body: Option<String>,
    /// Source location.
    pub span: Span,
}

/// A `<data>` item.
// @tg: implements:: [[openspec:chart-model#Datamodel Declaration]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Data {
    /// `id`.
    pub id: String,
    /// `src`.
    pub src: Option<String>,
    /// `expr`.
    pub expr: Option<Expr>,
    /// Inline content as raw text or XML.
    pub content: Option<String>,
    /// Extension attributes and elements.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}

/// `<donedata>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DoneData {
    /// `<content>`.
    pub content: Option<Content>,
    /// `<param>` children.
    pub params: Vec<Param>,
    /// Source location.
    pub span: Span,
}

/// `<invoke>` (W3C SCXML 6.4).
// @tg: implements:: [[openspec:chart-model#Invoke Declaration]]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Invoke {
    /// `type`.
    pub type_: Option<String>,
    /// `typeexpr`.
    pub typeexpr: Option<Expr>,
    /// `src`.
    pub src: Option<String>,
    /// `srcexpr`.
    pub srcexpr: Option<Expr>,
    /// `id`.
    pub id: Option<String>,
    /// `idlocation`.
    pub idlocation: Option<Expr>,
    /// `namelist`, split on whitespace.
    pub namelist: Vec<String>,
    /// `autoforward`.
    pub autoforward: bool,
    /// `<param>` children.
    pub params: Vec<Param>,
    /// `<content>` child.
    pub content: Option<Content>,
    /// `<finalize>` block.
    pub finalize: Block,
    /// Extension attributes and elements.
    pub extensions: Vec<Extension>,
    /// Source location.
    pub span: Span,
}
