//! Caller-supplied resolution of `src` attributes. The reader never touches the network or the
//! file system itself.

use std::collections::BTreeMap;

use vertep_core::ir::Span;

/// The element whose `src` is being resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// `<script src>`; inlined content becomes the script body.
    Script,
    /// `<data src>`; inlined content becomes the data item's inline content.
    Data,
    /// `<content src>` (not in SCXML 1.0 but accepted); the content must be inlined because
    /// the IR has no field for the reference.
    Content,
    /// `<invoke src>`; inlined content becomes the invoke's `<content>` body.
    Invoke,
}

/// One `src` the reader needs resolved.
#[derive(Clone, Copy, Debug)]
pub struct SourceRequest<'a> {
    /// The element carrying the attribute.
    pub kind: SourceKind,
    /// The attribute value, unchanged.
    pub src: &'a str,
    /// Where the element is.
    pub span: Span,
}

/// A resolver's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The content behind the reference. The chart keeps both the `src` and this content; the
    /// writer emits only the `src`.
    Inline(String),
    /// Keep the reference unresolved (for example in a format converter, or for an invoke the
    /// runtime will load). Not allowed for [`SourceKind::Content`].
    Defer,
}

/// A resolver's failure; the reader reports it as an unresolved-source error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    /// Why the source could not be resolved.
    pub message: String,
}

impl ResolveError {
    /// An error with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        ResolveError {
            message: message.into(),
        }
    }
}

/// Resolves `src` attributes of `script`, `data`, `content` and `invoke`
/// ([[openspec:scxml-format#External Content Resolution]]).
///
/// Without a resolver every `src` is an [`crate::ErrorKind::UnresolvedSource`] error and the
/// reader performs no I/O. Closures `Fn(&SourceRequest) -> Result<Resolution, ResolveError>`
/// implement this trait.
// @tg: implements:: [[openspec:scxml-format#External Content Resolution]]
pub trait SourceResolver {
    /// Resolve one reference.
    fn resolve(&self, request: &SourceRequest<'_>) -> Result<Resolution, ResolveError>;
}

impl<F> SourceResolver for F
where
    F: Fn(&SourceRequest<'_>) -> Result<Resolution, ResolveError>,
{
    fn resolve(&self, request: &SourceRequest<'_>) -> Result<Resolution, ResolveError> {
        self(request)
    }
}

/// An in-memory resolver: maps `src` values to content, fails on anything else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapResolver {
    entries: BTreeMap<String, String>,
}

impl MapResolver {
    /// An empty resolver.
    pub fn new() -> Self {
        MapResolver::default()
    }

    /// Add an entry.
    pub fn with(mut self, src: impl Into<String>, content: impl Into<String>) -> Self {
        self.entries.insert(src.into(), content.into());
        self
    }
}

impl SourceResolver for MapResolver {
    fn resolve(&self, request: &SourceRequest<'_>) -> Result<Resolution, ResolveError> {
        self.entries
            .get(request.src)
            .map(|c| Resolution::Inline(c.clone()))
            .ok_or_else(|| ResolveError::new(format!("no entry for `{}`", request.src)))
    }
}

/// A resolver that defers every reference except `<content src>`, which it cannot keep. Use it
/// to convert or round-trip documents without loading anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeferResolver;

impl SourceResolver for DeferResolver {
    fn resolve(&self, request: &SourceRequest<'_>) -> Result<Resolution, ResolveError> {
        match request.kind {
            SourceKind::Content => Err(ResolveError::new(
                "a <content src> must be inlined; the chart IR cannot keep the reference",
            )),
            _ => Ok(Resolution::Defer),
        }
    }
}
