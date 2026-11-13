//! The declared loss report: everything in a document that the chart IR does not keep as
//! first-class structure.

use core::fmt;

use vertep_core::ir::{Span, VT_NS};

/// The kind of construct that was lost or kept only opaquely.
///
/// Layout in the [`VL_NS`](crate::VL_NS) namespace is [`LossKind::Layout`], a formatting kind
/// ([[openspec:scxml-format#Layout Namespace]]).
///
/// Kinds split into two groups. *Content* kinds ([`LossKind::is_formatting`] is false) can
/// carry meaning; they make up [`LossReport::items`], and strict mode rejects a document that
/// has any. *Formatting* kinds carry no meaning for an SCXML processor; they are listed in
/// [`LossReport::formatting`] for completeness and never fail strict mode, so that comments
/// and indentation do not make a document unreadable.
// @tg: implements:: [[openspec:scxml-format#Layout Namespace]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LossKind {
    /// An attribute or element in a foreign (non-SCXML, non-`vt:`) namespace, kept only as an
    /// opaque extension node.
    ForeignExtension,
    /// An extension attribute or element on an SCXML element whose IR node has no extension
    /// slot (`datamodel`, `onentry`, `onexit`, `finalize`, `donedata`, `param`, `content`,
    /// `elseif`, `else`, the top-level `script`). It is dropped.
    DroppedExtension,
    /// An unprefixed attribute SCXML 1.0 does not define on that element, or an attribute
    /// qualified with the SCXML namespace. It is dropped.
    UnknownAttribute,
    /// An element the chart's datamodel cannot execute (for example `<assign>` under the
    /// `null` datamodel). It is kept in the IR.
    UnsupportedByDatamodel,
    /// Text-only inline content whose text is itself XML markup (escaped or in CDATA). The IR
    /// keeps it as a string that a writer emits as markup, so it does not survive as text.
    AmbiguousContent,
    /// A `src` on `<content>` was resolved and inlined; the IR keeps only the content.
    InlinedSource,
    /// A document type declaration (and any internal subset). It is dropped.
    Doctype,
    /// A comment. Formatting.
    Comment,
    /// A processing instruction. Formatting.
    ProcessingInstruction,
    /// The XML declaration; the writer always emits its own. Formatting.
    XmlDeclaration,
    /// Whitespace-only text between elements, or whitespace-only inline content. Reported once
    /// per document with a count. Formatting.
    Whitespace,
    /// An element with nothing in it whose absence means the same (`<datamodel/>`,
    /// `<finalize/>`). Formatting.
    EmptyElement,
    /// An attribute stating its default value where the IR records no "explicit" flag
    /// (`autoforward="false"`, history `type="shallow"`). Formatting.
    RedundantAttribute,
    /// An attribute or element in the layout namespace [`VL_NS`](crate::VL_NS) (`vl:x`, `vl:route`, ...):
    /// visual layout written by a designer. It is dropped, so layout never reaches the IR or
    /// the chart hash. Formatting.
    Layout,
}

impl LossKind {
    /// Whether this kind carries no meaning for an SCXML processor.
    pub fn is_formatting(self) -> bool {
        matches!(
            self,
            LossKind::Comment
                | LossKind::ProcessingInstruction
                | LossKind::XmlDeclaration
                | LossKind::Whitespace
                | LossKind::EmptyElement
                | LossKind::RedundantAttribute
                | LossKind::Layout
        )
    }
}

/// One lost construct with its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// What was lost.
    pub kind: LossKind,
    /// Where it was. Compare `span.line` and `span.column` explicitly: [`Span`] equality is
    /// always true.
    pub span: Span,
    /// A short description naming the construct.
    pub detail: String,
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {:?}: {}",
            self.span.line, self.span.column, self.kind, self.detail
        )
    }
}

/// Everything in a document that the IR keeps only opaquely or not at all
/// ([[openspec:scxml-format#Declared Loss Report]]).
///
/// The report "is empty" exactly when [`LossReport::items`] is empty; strict mode fails on a
/// non-empty report. [`LossReport::formatting`] is informational.
// @tg: implements:: [[openspec:scxml-format#Declared Loss Report]]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LossReport {
    /// Content losses, in document order.
    pub items: Vec<Loss>,
    /// Formatting losses, in document order.
    pub formatting: Vec<Loss>,
}

impl LossReport {
    /// True when no content is lost (formatting losses do not count).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Record a loss in the list its kind belongs to.
    pub fn push(&mut self, kind: LossKind, span: Span, detail: impl Into<String>) {
        let loss = Loss {
            kind,
            span,
            detail: detail.into(),
        };
        if kind.is_formatting() {
            self.formatting.push(loss);
        } else {
            self.items.push(loss);
        }
    }

    /// All losses of one kind, from either list.
    pub fn of_kind(&self, kind: LossKind) -> impl Iterator<Item = &Loss> {
        self.items
            .iter()
            .chain(self.formatting.iter())
            .filter(move |l| l.kind == kind)
    }
}

/// Where an extension node's namespace comes from.
///
/// The reader keeps both kinds as extension nodes; only [`ExtensionOrigin::Foreign`] ones are
/// listed in the loss report, because the `vt:` namespace is Vertep's own and its nodes are
/// carried deliberately ([[openspec:scxml-format#Vertep Extension Namespace]]).
// @tg: implements:: [[openspec:scxml-format#Vertep Extension Namespace]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtensionOrigin {
    /// The Vertep namespace [`VT_NS`], written with the `vt` prefix.
    Vertep,
    /// Any other namespace (including no namespace).
    Foreign,
}

impl ExtensionOrigin {
    /// Classify a namespace IRI.
    pub fn of(ns: &str) -> Self {
        if ns == VT_NS {
            ExtensionOrigin::Vertep
        } else {
            ExtensionOrigin::Foreign
        }
    }
}
