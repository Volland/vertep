//! The streaming SCXML reader: a recursive-descent pull parser over `quick-xml` events.

use std::collections::VecDeque;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{PrefixDeclaration, ResolveResult};
use quick_xml::{NsReader, XmlVersion};
use vertep_core::ir::{
    Binding, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, IfBranch, Initial, Invoke, Param, SCXML_NS, Script, Send, Span, State,
    StateElement, Transition, TransitionType,
};

use crate::ReadOptions;
use crate::error::{ErrorKind, ParseError};
use crate::loss::{ExtensionOrigin, LossKind, LossReport};
use crate::resolve::{Resolution, SourceKind, SourceRequest};
use crate::text::{
    LineIndex, escape_attr, escape_text, is_ws, is_xml_char, looks_like_xml, predefined_entity,
    split_ws,
};

/// Every element name SCXML 1.0 defines.
const SCXML_ELEMENTS: &[&str] = &[
    "scxml",
    "state",
    "parallel",
    "transition",
    "initial",
    "final",
    "onentry",
    "onexit",
    "history",
    "raise",
    "if",
    "elseif",
    "else",
    "foreach",
    "log",
    "datamodel",
    "data",
    "assign",
    "donedata",
    "content",
    "param",
    "script",
    "send",
    "cancel",
    "invoke",
    "finalize",
];

/// Maximum nesting of states and executable content before the reader gives up on a subtree.
const MAX_DEPTH: usize = 256;

/// An attribute as read, with its namespace resolved.
#[derive(Debug)]
struct Attr {
    /// Resolved namespace; `None` for unprefixed attributes and namespace declarations.
    ns: Option<String>,
    prefix: Option<String>,
    local: String,
    qname: String,
    value: String,
    /// For namespace declarations: the declared prefix (`None` for the default namespace).
    decl: Option<Option<String>>,
}

/// A start tag as read.
#[derive(Debug)]
struct Elem {
    ns: Option<String>,
    prefix: Option<String>,
    local: String,
    qname: String,
    attrs: Vec<Attr>,
    span: Span,
    /// Written as `<x/>`; the reader emits a synthetic end node right after it.
    empty: bool,
}

impl Elem {
    fn is_scxml(&self) -> bool {
        self.ns.as_deref() == Some(SCXML_NS)
    }
}

#[derive(Debug)]
enum Node {
    Start(Elem),
    End,
    /// Merged character data (text, CDATA and references), with the span of its start.
    Text(String, Span),
    Eof,
}

/// Attribute values of one SCXML element, by local name.
#[derive(Default)]
struct Attrs(Vec<(String, String)>);

impl Attrs {
    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    fn owned(&self, name: &str) -> Option<String> {
        self.get(name).map(str::to_owned)
    }
    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

pub(crate) struct Parser<'i, 'o> {
    reader: NsReader<&'i [u8]>,
    index: LineIndex<'i>,
    opts: &'o ReadOptions<'o>,
    pending: VecDeque<Node>,
    errors: Vec<ParseError>,
    losses: LossReport,
    ws_count: u32,
    ws_first: Span,
    fatal: bool,
    null_dm: bool,
    depth: usize,
}

impl<'i, 'o> Parser<'i, 'o> {
    pub(crate) fn new(input: &'i str, opts: &'o ReadOptions<'o>) -> Self {
        let mut reader = NsReader::from_str(input);
        let cfg = reader.config_mut();
        cfg.expand_empty_elements = false;
        cfg.check_end_names = true;
        cfg.trim_text(false);
        Parser {
            reader,
            index: LineIndex::new(input),
            opts,
            pending: VecDeque::new(),
            errors: Vec::new(),
            losses: LossReport::default(),
            ws_count: 0,
            ws_first: Span::NONE,
            fatal: false,
            null_dm: true,
            depth: 0,
        }
    }

    /// Errors and losses gathered so far, with the whitespace summary appended.
    pub(crate) fn finish(mut self) -> (Vec<ParseError>, LossReport) {
        if self.ws_count > 0 {
            let n = self.ws_count;
            self.losses.push(
                LossKind::Whitespace,
                self.ws_first,
                format!("{n} whitespace-only text node(s) not kept"),
            );
        }
        (self.errors, self.losses)
    }

    // ---------------------------------------------------------------- diagnostics

    fn error(&mut self, kind: ErrorKind, span: Span, message: impl Into<String>) {
        self.errors.push(ParseError {
            kind,
            span,
            message: message.into(),
        });
    }

    fn loss(&mut self, kind: LossKind, span: Span, detail: impl Into<String>) {
        self.losses.push(kind, span, detail);
    }

    fn ws(&mut self, span: Span) {
        if self.ws_count == 0 {
            self.ws_first = span;
        }
        self.ws_count += 1;
    }

    // ---------------------------------------------------------------- event layer

    fn next(&mut self) -> Node {
        if self.pending.is_empty() {
            self.fill();
        }
        self.pending.pop_front().unwrap_or(Node::Eof)
    }

    fn fill(&mut self) {
        let mut text: Option<(String, Span)> = None;
        loop {
            if self.fatal {
                self.flush(&mut text);
                self.pending.push_back(Node::Eof);
                return;
            }
            let pos = usize::try_from(self.reader.buffer_position()).unwrap_or(usize::MAX);
            let span = self.index.span(pos);
            let ev = self.reader.read_event();
            match ev {
                Err(e) => {
                    let at = usize::try_from(self.reader.error_position()).unwrap_or(pos);
                    let at = self.index.span(at);
                    self.error(ErrorKind::Xml, at, format!("malformed XML: {e}"));
                    self.fatal = true;
                }
                Ok(Event::Text(t)) => push_text(&mut text, &t.xml10_content(), span),
                Ok(Event::CData(c)) => push_text(&mut text, &c.xml10_content(), span),
                Ok(Event::GeneralRef(r)) => {
                    let resolved = if r.is_char_ref() {
                        match r.resolve_char_ref() {
                            Ok(Some(c)) if is_xml_char(c) => Some(c),
                            _ => None,
                        }
                    } else {
                        predefined_entity(&r)
                    };
                    match resolved {
                        Some(c) => push_text(&mut text, c.encode_utf8(&mut [0; 4]), span),
                        None => self.error(
                            ErrorKind::Xml,
                            span,
                            format!("unknown or illegal reference `&{};`", &*r),
                        ),
                    }
                }
                Ok(Event::Comment(_)) => self.loss(LossKind::Comment, span, "comment"),
                Ok(Event::PI(p)) => self.loss(
                    LossKind::ProcessingInstruction,
                    span,
                    format!("processing instruction `{}`", p.target()),
                ),
                Ok(Event::Decl(_)) => self.loss(LossKind::XmlDeclaration, span, "XML declaration"),
                Ok(Event::DocType(_)) => {
                    self.loss(LossKind::Doctype, span, "document type declaration")
                }
                Ok(Event::Start(e)) => {
                    self.flush(&mut text);
                    let el = self.elem(&e, span, false);
                    self.pending.push_back(Node::Start(el));
                    return;
                }
                Ok(Event::Empty(e)) => {
                    self.flush(&mut text);
                    let el = self.elem(&e, span, true);
                    self.pending.push_back(Node::Start(el));
                    self.pending.push_back(Node::End);
                    return;
                }
                Ok(Event::End(_)) => {
                    self.flush(&mut text);
                    self.pending.push_back(Node::End);
                    return;
                }
                Ok(Event::Eof) => {
                    self.flush(&mut text);
                    self.pending.push_back(Node::Eof);
                    return;
                }
            }
        }
    }

    fn flush(&mut self, text: &mut Option<(String, Span)>) {
        if let Some((t, s)) = text.take() {
            self.pending.push_back(Node::Text(t, s));
        }
    }

    fn elem(&mut self, e: &BytesStart<'_>, span: Span, empty: bool) -> Elem {
        let name = e.name();
        let qname = name.as_ref().to_owned();
        let prefix = name.prefix().map(|p| p.as_ref().to_owned());
        let (res, local) = self.reader.resolver().resolve_element(name);
        let local = local.as_ref().to_owned();
        let ns = owned_ns(res);
        let ns = match ns {
            Ok(ns) => ns,
            Err(p) => {
                self.error(
                    ErrorKind::Xml,
                    span,
                    format!("namespace prefix `{p}` of <{qname}> is not declared"),
                );
                None
            }
        };
        let mut attrs = Vec::new();
        for a in e.attributes() {
            let a = match a {
                Ok(a) => a,
                Err(err) => {
                    self.error(
                        ErrorKind::Xml,
                        span,
                        format!("malformed attribute on <{qname}>: {err}"),
                    );
                    self.fatal = true;
                    break;
                }
            };
            let aq = a.key.as_ref().to_owned();
            let value = match a.normalized_value(XmlVersion::Implicit1_0) {
                Ok(v) => v.into_owned(),
                Err(err) => {
                    self.error(
                        ErrorKind::Xml,
                        span,
                        format!("bad value of attribute `{aq}` on <{qname}>: {err}"),
                    );
                    String::new()
                }
            };
            if let Some(d) = a.key.as_namespace_binding() {
                let decl = match d {
                    PrefixDeclaration::Default => None,
                    PrefixDeclaration::Named(p) => Some(p.to_owned()),
                };
                attrs.push(Attr {
                    ns: None,
                    prefix: None,
                    local: String::new(),
                    qname: aq,
                    value,
                    decl: Some(decl),
                });
                continue;
            }
            let aprefix = a.key.prefix().map(|p| p.as_ref().to_owned());
            let (res, alocal) = self.reader.resolver().resolve_attribute(a.key);
            let alocal = alocal.as_ref().to_owned();
            let ans = match owned_ns(res) {
                Ok(ns) => ns,
                Err(p) => {
                    self.error(
                        ErrorKind::Xml,
                        span,
                        format!("namespace prefix `{p}` of attribute `{aq}` is not declared"),
                    );
                    None
                }
            };
            attrs.push(Attr {
                ns: ans,
                prefix: aprefix,
                local: alocal,
                qname: aq,
                value,
                decl: None,
            });
        }
        Elem {
            ns,
            prefix,
            local,
            qname,
            attrs,
            span,
            empty,
        }
    }

    // ---------------------------------------------------------------- structure helpers

    /// The next child element of `parent`, or `None` at its end tag. Whitespace is counted,
    /// other text is an error.
    fn child(&mut self, parent: &Elem) -> Option<Elem> {
        loop {
            match self.next() {
                Node::Start(e) => return Some(e),
                Node::End => return None,
                Node::Text(t, span) => {
                    if is_ws(&t) {
                        self.ws(span);
                    } else {
                        self.error(
                            ErrorKind::UnexpectedText,
                            span,
                            format!("text is not allowed in <{}>", parent.local),
                        );
                    }
                }
                Node::Eof => {
                    self.eof(parent);
                    return None;
                }
            }
        }
    }

    fn eof(&mut self, parent: &Elem) {
        if !self.fatal {
            self.error(
                ErrorKind::Xml,
                parent.span,
                format!("<{}> is not closed", parent.qname),
            );
            self.fatal = true;
        }
    }

    /// Consume the rest of `el` (whose start has been read).
    fn skip(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.next() {
                Node::Start(_) => depth += 1,
                Node::End => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                Node::Text(..) => {}
                Node::Eof => return,
            }
        }
    }

    fn unexpected(&mut self, c: Elem, parent: &str) {
        if c.is_scxml() && !SCXML_ELEMENTS.contains(&c.local.as_str()) {
            self.error(
                ErrorKind::UnknownElement,
                c.span,
                format!("<{}> is not an SCXML 1.0 element", c.local),
            );
        } else {
            self.error(
                ErrorKind::UnexpectedElement,
                c.span,
                format!("<{}> is not allowed in <{parent}>", c.qname),
            );
        }
        self.skip();
    }

    fn duplicate(&mut self, c: Elem, parent: &str) {
        self.error(
            ErrorKind::DuplicateElement,
            c.span,
            format!("<{parent}> may contain at most one <{}>", c.local),
        );
        self.skip();
    }

    fn too_deep(&mut self, c: &Elem) -> bool {
        if self.depth >= MAX_DEPTH {
            self.error(
                ErrorKind::UnexpectedElement,
                c.span,
                format!("<{}> is nested more than {MAX_DEPTH} levels deep", c.local),
            );
            self.skip();
            true
        } else {
            false
        }
    }

    /// Read the SCXML attributes of `el`: `allowed` unprefixed attributes are returned, other
    /// namespaced attributes become extension attributes in `exts` (sorted by namespace and
    /// name) or, when the IR node has no extension slot, are reported as dropped.
    fn attrs(&mut self, el: &Elem, allowed: &[&str], exts: Option<&mut Vec<Extension>>) -> Attrs {
        let mut out = Attrs::default();
        let mut found = Vec::new();
        for a in &el.attrs {
            if a.decl.is_some() {
                continue;
            }
            match &a.ns {
                None if allowed.contains(&a.local.as_str()) => {
                    out.0.push((a.local.clone(), a.value.clone()));
                }
                None => self.loss(
                    LossKind::UnknownAttribute,
                    el.span,
                    format!("attribute `{}` is not defined on <{}>", a.qname, el.local),
                ),
                Some(ns) if ns == SCXML_NS => self.loss(
                    LossKind::UnknownAttribute,
                    el.span,
                    format!(
                        "attribute `{}` on <{}> is in the SCXML namespace",
                        a.qname, el.local
                    ),
                ),
                Some(ns) => found.push(Extension {
                    ns: ns.clone(),
                    local: a.local.clone(),
                    value: ExtensionValue::Attribute(a.value.clone()),
                    position: 0,
                    span: el.span,
                }),
            }
        }
        found.sort_by(|x, y| (&x.ns, &x.local).cmp(&(&y.ns, &y.local)));
        match exts {
            Some(v) => {
                for x in found {
                    if ExtensionOrigin::of(&x.ns) == ExtensionOrigin::Foreign {
                        self.loss(
                            LossKind::ForeignExtension,
                            el.span,
                            format!("attribute {{{}}}{} kept as opaque extension", x.ns, x.local),
                        );
                    }
                    v.push(x);
                }
            }
            None => {
                for x in found {
                    self.loss(
                        LossKind::DroppedExtension,
                        el.span,
                        format!(
                            "attribute {{{}}}{} on <{}> dropped: no extension slot",
                            x.ns, x.local, el.local
                        ),
                    );
                }
            }
        }
        out
    }

    fn required(&mut self, a: &Attrs, name: &str, el: &Elem) -> Option<String> {
        let v = a.owned(name);
        if v.is_none() {
            self.error(
                ErrorKind::MissingAttribute,
                el.span,
                format!("<{}> requires attribute `{name}`", el.local),
            );
        }
        v
    }

    fn exclusive(&mut self, a: &Attrs, x: &str, y: &str, el: &Elem) {
        if a.has(x) && a.has(y) {
            self.error(
                ErrorKind::ExclusiveAttributes,
                el.span,
                format!("<{}> may not have both `{x}` and `{y}`", el.local),
            );
        }
    }

    fn exclusive_with_content(&mut self, cond: bool, what: &str, el: &Elem) {
        if cond {
            self.error(
                ErrorKind::ExclusiveAttributes,
                el.span,
                format!("<{}> may not have both {what}", el.local),
            );
        }
    }

    fn null_loss(&mut self, el: &Elem) {
        if self.null_dm {
            self.loss(
                LossKind::UnsupportedByDatamodel,
                el.span,
                format!("<{}> cannot be executed by the null datamodel", el.local),
            );
        }
    }

    fn resolve(&mut self, kind: SourceKind, src: &str, span: Span) -> Option<String> {
        let Some(resolver) = self.opts.resolver else {
            self.error(
                ErrorKind::UnresolvedSource,
                span,
                format!("src=\"{src}\" is not resolved: no resolver was supplied"),
            );
            return None;
        };
        match resolver.resolve(&SourceRequest { kind, src, span }) {
            Ok(Resolution::Inline(s)) => Some(s),
            Ok(Resolution::Defer) if kind == SourceKind::Content => {
                self.error(
                    ErrorKind::UnresolvedSource,
                    span,
                    format!("src=\"{src}\" on <content> must be inlined, not deferred"),
                );
                None
            }
            Ok(Resolution::Defer) => None,
            Err(e) => {
                self.error(
                    ErrorKind::UnresolvedSource,
                    span,
                    format!("src=\"{src}\" could not be resolved: {}", e.message),
                );
                None
            }
        }
    }

    /// Read an extension element (start already read) into an opaque extension node.
    fn ext_element(&mut self, c: Elem, position: u32) -> Extension {
        let span = c.span;
        let ns = c.ns.clone().unwrap_or_default();
        let local = c.local.clone();
        if ExtensionOrigin::of(&ns) == ExtensionOrigin::Foreign {
            self.loss(
                LossKind::ForeignExtension,
                span,
                format!("element {{{ns}}}{local} kept as opaque extension"),
            );
        }
        let xml = self.fragment(c);
        Extension {
            ns,
            local,
            value: ExtensionValue::Element(xml),
            position,
            span,
        }
    }

    /// Drop an extension element where the IR has no slot for it.
    fn drop_element(&mut self, c: Elem, owner: &str) {
        self.loss(
            LossKind::DroppedExtension,
            c.span,
            format!(
                "element {{{}}}{} in <{owner}> dropped: no extension slot",
                c.ns.as_deref().unwrap_or(""),
                c.local
            ),
        );
        self.skip();
    }

    /// Children of an element that has no SCXML children: extension elements go to `exts`.
    fn leaf_children(&mut self, el: &Elem, mut exts: Option<&mut Vec<Extension>>) {
        let mut idx = 0u32;
        while let Some(c) = self.child(el) {
            if c.is_scxml() {
                self.unexpected(c, &el.local);
                continue;
            }
            match exts.as_deref_mut() {
                Some(v) => {
                    let x = self.ext_element(c, idx);
                    v.push(x);
                    idx += 1;
                }
                None => self.drop_element(c, &el.local),
            }
        }
    }

    // ---------------------------------------------------------------- serialization of XML

    /// Serialize an element subtree (start already read) as a self-contained XML string:
    /// namespace bindings it inherits from outside are declared on its root, interpreting the
    /// string in a context whose default namespace is SCXML. Comments and processing
    /// instructions are dropped (and reported); text is re-escaped canonically.
    fn fragment(&mut self, top: Elem) -> String {
        let mut needed: Vec<(Option<String>, String)> = Vec::new();
        let mut declared: Vec<Vec<Option<String>>> = Vec::new();
        let top_attrs = frag_open(&top, &mut declared, &mut needed);
        let mut inner = String::new();
        if top.empty {
            self.next();
        } else {
            let mut open: Vec<String> = Vec::new();
            loop {
                match self.next() {
                    Node::Start(el) => {
                        let attrs = frag_open(&el, &mut declared, &mut needed);
                        inner.push('<');
                        inner.push_str(&el.qname);
                        inner.push_str(&attrs);
                        if el.empty {
                            inner.push_str("/>");
                            declared.pop();
                            self.next();
                        } else {
                            inner.push('>');
                            open.push(el.qname);
                        }
                    }
                    Node::End => match open.pop() {
                        Some(q) => {
                            inner.push_str("</");
                            inner.push_str(&q);
                            inner.push('>');
                            declared.pop();
                        }
                        None => break,
                    },
                    Node::Text(t, _) => escape_text(&t, &mut inner),
                    Node::Eof => {
                        self.eof(&top);
                        break;
                    }
                }
            }
        }
        let mut out = String::from("<");
        out.push_str(&top.qname);
        for (p, uri) in &needed {
            match p {
                None => out.push_str(" xmlns=\""),
                Some(p) => {
                    out.push_str(" xmlns:");
                    out.push_str(p);
                    out.push_str("=\"");
                }
            }
            escape_attr(uri, &mut out);
            out.push('"');
        }
        out.push_str(&top_attrs);
        if top.empty {
            out.push_str("/>");
        } else {
            out.push('>');
            out.push_str(&inner);
            out.push_str("</");
            out.push_str(&top.qname);
            out.push('>');
        }
        out
    }

    /// Inline content of `<data>`, `<assign>` or `<content>` (start already read).
    ///
    /// With element children the result is XML: the elements serialized by
    /// [`Parser::fragment`], non-whitespace text escaped, whitespace-only text between them
    /// dropped. Without element children it is the text itself, unescaped. Whitespace-only
    /// or empty content is `None`.
    fn inline(&mut self, el: &Elem) -> Option<String> {
        let mut items: Vec<(bool, String, Span)> = Vec::new();
        loop {
            match self.next() {
                Node::Text(t, span) => items.push((false, t, span)),
                Node::Start(c) => {
                    let span = c.span;
                    let x = self.fragment(c);
                    items.push((true, x, span));
                }
                Node::End => break,
                Node::Eof => {
                    self.eof(el);
                    break;
                }
            }
        }
        if items.iter().any(|i| i.0) {
            let mut out = String::new();
            for (is_el, s, span) in items {
                if is_el {
                    out.push_str(&s);
                } else if is_ws(&s) {
                    self.ws(span);
                } else {
                    escape_text(&s, &mut out);
                }
            }
            Some(out)
        } else {
            let text: String = items.into_iter().map(|i| i.1).collect();
            if is_ws(&text) {
                if !text.is_empty() {
                    self.ws(el.span);
                }
                None
            } else {
                if looks_like_xml(&text) {
                    self.loss(
                        LossKind::AmbiguousContent,
                        el.span,
                        format!(
                            "text content of <{}> is XML markup and will be written as markup",
                            el.local
                        ),
                    );
                }
                Some(text)
            }
        }
    }

    // ---------------------------------------------------------------- document

    pub(crate) fn document(&mut self) -> Option<Chart> {
        loop {
            match self.next() {
                Node::Text(t, span) => {
                    if is_ws(&t) {
                        self.ws(span);
                    } else {
                        self.error(
                            ErrorKind::UnexpectedText,
                            span,
                            "text outside the root element",
                        );
                    }
                }
                Node::Start(el) => {
                    if el.local != "scxml" {
                        self.error(
                            ErrorKind::NotScxml,
                            el.span,
                            format!("root element is <{}>, expected <scxml>", el.qname),
                        );
                        return None;
                    }
                    if !el.is_scxml() {
                        let ns = el.ns.as_deref().map_or_else(
                            || "no namespace".to_owned(),
                            |n| format!("namespace `{n}`"),
                        );
                        self.error(
                            ErrorKind::WrongNamespace,
                            el.span,
                            format!("root <scxml> is in {ns}; expected namespace `{SCXML_NS}`"),
                        );
                        return None;
                    }
                    let chart = self.scxml(el);
                    loop {
                        match self.next() {
                            Node::Eof => break,
                            Node::Text(t, span) => {
                                if is_ws(&t) {
                                    self.ws(span);
                                } else {
                                    self.error(
                                        ErrorKind::UnexpectedText,
                                        span,
                                        "text after the root element",
                                    );
                                }
                            }
                            Node::Start(c) => {
                                self.error(
                                    ErrorKind::UnexpectedElement,
                                    c.span,
                                    format!("<{}> after the root element", c.qname),
                                );
                                self.skip();
                            }
                            Node::End => {}
                        }
                    }
                    return Some(chart);
                }
                Node::End => {}
                Node::Eof => {
                    if !self.fatal {
                        self.error(
                            ErrorKind::NotScxml,
                            Span::new(1, 1),
                            "document has no root element",
                        );
                    }
                    return None;
                }
            }
        }
    }

    fn scxml(&mut self, el: Elem) -> Chart {
        let mut chart = Chart {
            span: el.span,
            ..Chart::default()
        };
        let a = self.attrs(
            &el,
            &["version", "name", "initial", "datamodel", "binding"],
            Some(&mut chart.extensions),
        );
        match a.get("version") {
            None => self.error(
                ErrorKind::MissingAttribute,
                el.span,
                "<scxml> requires attribute `version`",
            ),
            Some("1.0") => {}
            Some(v) => {
                let msg = format!("SCXML version `{v}` is not supported; expected `1.0`");
                self.error(ErrorKind::UnsupportedVersion, el.span, msg);
            }
        }
        chart.name = a.owned("name");
        if let Some(i) = a.get("initial") {
            chart.initial = split_ws(i);
        }
        if let Some(dm) = a.get("datamodel") {
            chart.datamodel = dm.to_owned();
            chart.datamodel_explicit = true;
            if dm != "null" && !self.opts.datamodels.is_registered(dm) {
                self.error(
                    ErrorKind::UnsupportedDatamodel,
                    el.span,
                    format!("datamodel `{dm}` is not registered with the reader"),
                );
            }
        }
        self.null_dm = chart.datamodel == "null";
        if let Some(b) = a.get("binding") {
            chart.binding_explicit = true;
            match b {
                "early" => chart.binding = Binding::Early,
                "late" => chart.binding = Binding::Late,
                other => {
                    let msg = format!("binding `{other}` is not `early` or `late`");
                    self.error(ErrorKind::InvalidValue, el.span, msg);
                }
            }
        }
        let mut idx = 0u32;
        let mut seen_dm = false;
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                let x = self.ext_element(c, idx);
                chart.extensions.push(x);
                idx += 1;
                continue;
            }
            match c.local.as_str() {
                "state" | "parallel" | "final" | "history" => {
                    if let Some(s) = self.state(c) {
                        chart.states.push(s);
                        idx += 1;
                    }
                }
                "datamodel" if seen_dm => self.duplicate(c, "scxml"),
                "datamodel" => {
                    seen_dm = true;
                    let d = self.datamodel(c);
                    if !d.is_empty() {
                        chart.data = d;
                        idx += 1;
                    }
                }
                "script" if chart.script.is_some() => self.duplicate(c, "scxml"),
                "script" => {
                    self.null_loss(&c);
                    chart.script = Some(self.script(c, None));
                    idx += 1;
                }
                _ => self.unexpected(c, "scxml"),
            }
        }
        chart
    }

    fn state(&mut self, el: Elem) -> Option<State> {
        if self.too_deep(&el) {
            return None;
        }
        self.depth += 1;
        let history = el.local == "history";
        let element = match el.local.as_str() {
            "parallel" => StateElement::Parallel,
            "final" => StateElement::Final,
            "history" => StateElement::History(HistoryType::Shallow),
            _ => StateElement::State,
        };
        let mut s = State::new("", element);
        s.span = el.span;
        let allowed: &[&str] = if history {
            &["id", "initial", "type"]
        } else {
            &["id", "initial"]
        };
        let a = self.attrs(&el, allowed, Some(&mut s.extensions));
        if let Some(id) = a.get("id") {
            s.id = id.to_owned();
        }
        if let Some(i) = a.get("initial") {
            s.initial = split_ws(i);
        }
        if history {
            match a.get("type") {
                None => {}
                Some("shallow") => self.loss(
                    LossKind::RedundantAttribute,
                    el.span,
                    "history type=\"shallow\" is the default",
                ),
                Some("deep") => s.element = StateElement::History(HistoryType::Deep),
                Some(v) => {
                    let msg = format!("history type `{v}` is not `shallow` or `deep`");
                    self.error(ErrorKind::InvalidValue, el.span, msg);
                }
            }
        }
        let mut idx = 0u32;
        let mut seen_dm = false;
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                let x = self.ext_element(c, idx);
                s.extensions.push(x);
                idx += 1;
                continue;
            }
            match c.local.as_str() {
                "onentry" => {
                    let b = self.block_owner(c);
                    s.onentry.push(b);
                    idx += 1;
                }
                "onexit" => {
                    let b = self.block_owner(c);
                    s.onexit.push(b);
                    idx += 1;
                }
                "transition" => {
                    let t = self.transition(c);
                    s.transitions.push(t);
                    idx += 1;
                }
                "initial" if s.initial_element.is_some() => self.duplicate(c, &el.local),
                "initial" => {
                    s.initial_element = Some(self.initial(c));
                    idx += 1;
                }
                "state" | "parallel" | "final" | "history" => {
                    if let Some(child) = self.state(c) {
                        s.children.push(child);
                        idx += 1;
                    }
                }
                "datamodel" if seen_dm => self.duplicate(c, &el.local),
                "datamodel" => {
                    seen_dm = true;
                    let d = self.datamodel(c);
                    if !d.is_empty() {
                        s.data = d;
                        idx += 1;
                    }
                }
                "invoke" => {
                    let i = self.invoke(c);
                    s.invokes.push(i);
                    idx += 1;
                }
                "donedata" if s.donedata.is_some() => self.duplicate(c, &el.local),
                "donedata" => {
                    s.donedata = Some(self.donedata(c));
                    idx += 1;
                }
                _ => self.unexpected(c, &el.local),
            }
        }
        if !s.initial.is_empty() && s.initial_element.is_some() {
            self.error(
                ErrorKind::ExclusiveAttributes,
                el.span,
                format!(
                    "<{}> may not have both an `initial` attribute and an <initial> child",
                    el.local
                ),
            );
        }
        self.depth -= 1;
        Some(s)
    }

    fn datamodel(&mut self, el: Elem) -> Vec<Data> {
        self.attrs(&el, &[], None);
        let mut v = Vec::new();
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                self.drop_element(c, "datamodel");
                continue;
            }
            match c.local.as_str() {
                "data" => {
                    let d = self.data(c);
                    v.push(d);
                }
                _ => self.unexpected(c, "datamodel"),
            }
        }
        if v.is_empty() {
            self.loss(
                LossKind::EmptyElement,
                el.span,
                "<datamodel> without <data> not kept",
            );
        }
        v
    }

    fn data(&mut self, el: Elem) -> Data {
        let mut d = Data {
            id: String::new(),
            src: None,
            expr: None,
            content: None,
            extensions: Vec::new(),
            span: el.span,
        };
        let a = self.attrs(&el, &["id", "src", "expr"], Some(&mut d.extensions));
        d.id = self.required(&a, "id", &el).unwrap_or_default();
        d.src = a.owned("src");
        d.expr = a.owned("expr");
        self.exclusive(&a, "src", "expr", &el);
        self.null_loss(&el);
        d.content = self.inline(&el);
        self.exclusive_with_content(
            d.content.is_some() && (d.src.is_some() || d.expr.is_some()),
            "`src` or `expr` and inline content",
            &el,
        );
        if let Some(src) = d.src.clone()
            && d.content.is_none()
            && d.expr.is_none()
        {
            d.content = self.resolve(SourceKind::Data, &src, el.span);
        }
        d
    }

    fn script(&mut self, el: Elem, exts: Option<&mut Vec<Extension>>) -> Script {
        let a = self.attrs(&el, &["src"], exts);
        let mut text = String::new();
        loop {
            match self.next() {
                Node::Text(t, _) => text.push_str(&t),
                Node::Start(c) => self.unexpected(c, "script"),
                Node::End => break,
                Node::Eof => {
                    self.eof(&el);
                    break;
                }
            }
        }
        let mut body = if is_ws(&text) {
            if !text.is_empty() {
                self.ws(el.span);
            }
            None
        } else {
            Some(text)
        };
        let src = a.owned("src");
        if let Some(src) = &src {
            if body.is_some() {
                self.exclusive_with_content(true, "`src` and inline script", &el);
            } else {
                body = self.resolve(SourceKind::Script, src, el.span);
            }
        }
        Script {
            src,
            body,
            span: el.span,
        }
    }

    fn block_owner(&mut self, el: Elem) -> Vec<Exec> {
        self.attrs(&el, &[], None);
        self.block(&el)
    }

    /// Executable content children of `el` (start already read).
    fn block(&mut self, el: &Elem) -> Vec<Exec> {
        let mut b = Vec::new();
        while let Some(c) = self.child(el) {
            self.block_item(c, &mut b, &el.local);
        }
        b
    }

    fn block_item(&mut self, c: Elem, b: &mut Vec<Exec>, parent: &str) {
        if !c.is_scxml() {
            let span = c.span;
            let pos = u32::try_from(b.len()).unwrap_or(u32::MAX);
            let x = self.ext_element(c, pos);
            b.push(Exec {
                kind: ExecKind::Extension(x),
                extensions: Vec::new(),
                span,
            });
            return;
        }
        if let Some(x) = self.exec(c, parent) {
            b.push(x);
        }
    }

    fn exec(&mut self, el: Elem, parent: &str) -> Option<Exec> {
        let span = el.span;
        let mut exts = Vec::new();
        let kind = match el.local.as_str() {
            "raise" => {
                let a = self.attrs(&el, &["event"], Some(&mut exts));
                let event = self.required(&a, "event", &el).unwrap_or_default();
                self.leaf_children(&el, Some(&mut exts));
                ExecKind::Raise { event }
            }
            "send" => return Some(self.send(el)),
            "cancel" => {
                let a = self.attrs(&el, &["sendid", "sendidexpr"], Some(&mut exts));
                self.exclusive(&a, "sendid", "sendidexpr", &el);
                if !a.has("sendid") && !a.has("sendidexpr") {
                    self.error(
                        ErrorKind::MissingAttribute,
                        span,
                        "<cancel> requires `sendid` or `sendidexpr`",
                    );
                }
                self.leaf_children(&el, Some(&mut exts));
                ExecKind::Cancel {
                    sendid: a.owned("sendid"),
                    sendidexpr: a.owned("sendidexpr"),
                }
            }
            "log" => {
                let a = self.attrs(&el, &["label", "expr"], Some(&mut exts));
                self.leaf_children(&el, Some(&mut exts));
                ExecKind::Log {
                    label: a.owned("label"),
                    expr: a.owned("expr"),
                }
            }
            "assign" => {
                let a = self.attrs(&el, &["location", "expr"], Some(&mut exts));
                let location = self.required(&a, "location", &el).unwrap_or_default();
                self.null_loss(&el);
                let content = self.inline(&el);
                let expr = a.owned("expr");
                self.exclusive_with_content(
                    expr.is_some() && content.is_some(),
                    "`expr` and inline content",
                    &el,
                );
                ExecKind::Assign {
                    location,
                    expr,
                    content,
                }
            }
            "script" => {
                self.null_loss(&el);
                ExecKind::Script(self.script(el, Some(&mut exts)))
            }
            "if" => return self.if_(el),
            "foreach" => {
                if self.too_deep(&el) {
                    return None;
                }
                self.depth += 1;
                let a = self.attrs(&el, &["array", "item", "index"], Some(&mut exts));
                let array = self.required(&a, "array", &el).unwrap_or_default();
                let item = self.required(&a, "item", &el).unwrap_or_default();
                self.null_loss(&el);
                let body = self.block(&el);
                self.depth -= 1;
                ExecKind::Foreach {
                    array,
                    item,
                    index: a.owned("index"),
                    body,
                }
            }
            _ => {
                self.unexpected(el, parent);
                return None;
            }
        };
        Some(Exec {
            kind,
            extensions: exts,
            span,
        })
    }

    fn send(&mut self, el: Elem) -> Exec {
        let mut exts = Vec::new();
        let a = self.attrs(
            &el,
            &[
                "event",
                "eventexpr",
                "target",
                "targetexpr",
                "type",
                "typeexpr",
                "id",
                "idlocation",
                "delay",
                "delayexpr",
                "namelist",
            ],
            Some(&mut exts),
        );
        for (x, y) in [
            ("event", "eventexpr"),
            ("target", "targetexpr"),
            ("type", "typeexpr"),
            ("id", "idlocation"),
            ("delay", "delayexpr"),
        ] {
            self.exclusive(&a, x, y, &el);
        }
        let mut send = Send {
            event: a.owned("event"),
            eventexpr: a.owned("eventexpr"),
            target: a.owned("target"),
            targetexpr: a.owned("targetexpr"),
            type_: a.owned("type"),
            typeexpr: a.owned("typeexpr"),
            id: a.owned("id"),
            idlocation: a.owned("idlocation"),
            delay: a.owned("delay"),
            delayexpr: a.owned("delayexpr"),
            namelist: a.get("namelist").map(split_ws).unwrap_or_default(),
            params: Vec::new(),
            content: None,
        };
        let mut idx = 0u32;
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                let x = self.ext_element(c, idx);
                exts.push(x);
                idx += 1;
                continue;
            }
            match c.local.as_str() {
                "param" => {
                    let p = self.param(c);
                    send.params.push(p);
                    idx += 1;
                }
                "content" if send.content.is_some() => self.duplicate(c, "send"),
                "content" => {
                    send.content = Some(self.content(c));
                    idx += 1;
                }
                _ => self.unexpected(c, "send"),
            }
        }
        self.exclusive_with_content(
            send.content.is_some() && (!send.namelist.is_empty() || !send.params.is_empty()),
            "<content> and `namelist` or <param>",
            &el,
        );
        Exec {
            kind: ExecKind::Send(Box::new(send)),
            extensions: exts,
            span: el.span,
        }
    }

    fn if_(&mut self, el: Elem) -> Option<Exec> {
        if self.too_deep(&el) {
            return None;
        }
        self.depth += 1;
        let mut exts = Vec::new();
        let a = self.attrs(&el, &["cond"], Some(&mut exts));
        let cond = self.required(&a, "cond", &el).unwrap_or_default();
        let mut branches = vec![IfBranch {
            cond: Some(cond),
            body: Vec::new(),
            span: el.span,
        }];
        let mut seen_else = false;
        while let Some(c) = self.child(&el) {
            if c.is_scxml() && (c.local == "elseif" || c.local == "else") {
                let is_else = c.local == "else";
                if seen_else {
                    let msg = format!("<{}> after <else>", c.local);
                    self.error(ErrorKind::UnexpectedElement, c.span, msg);
                    self.skip();
                    continue;
                }
                let span = c.span;
                let cond = if is_else {
                    seen_else = true;
                    self.attrs(&c, &[], None);
                    None
                } else {
                    let a = self.attrs(&c, &["cond"], None);
                    Some(self.required(&a, "cond", &c).unwrap_or_default())
                };
                self.leaf_children(&c, None);
                branches.push(IfBranch {
                    cond,
                    body: Vec::new(),
                    span,
                });
                continue;
            }
            let mut body = branches.pop().unwrap_or(IfBranch {
                cond: None,
                body: Vec::new(),
                span: el.span,
            });
            self.block_item(c, &mut body.body, "if");
            branches.push(body);
        }
        self.depth -= 1;
        Some(Exec {
            kind: ExecKind::If { branches },
            extensions: exts,
            span: el.span,
        })
    }

    fn transition(&mut self, el: Elem) -> Transition {
        let mut t = Transition {
            span: el.span,
            ..Transition::default()
        };
        let a = self.attrs(
            &el,
            &["event", "cond", "target", "type"],
            Some(&mut t.extensions),
        );
        t.events = a.get("event").map(split_ws).unwrap_or_default();
        t.cond = a.owned("cond");
        t.targets = a.get("target").map(split_ws).unwrap_or_default();
        if let Some(ty) = a.get("type") {
            t.kind_explicit = true;
            match ty {
                "internal" => t.kind = TransitionType::Internal,
                "external" => t.kind = TransitionType::External,
                other => {
                    let msg = format!("transition type `{other}` is not `internal` or `external`");
                    self.error(ErrorKind::InvalidValue, el.span, msg);
                }
            }
        }
        t.body = self.block(&el);
        t
    }

    fn initial(&mut self, el: Elem) -> Initial {
        let mut exts = Vec::new();
        self.attrs(&el, &[], Some(&mut exts));
        let mut tr: Option<Transition> = None;
        let mut idx = 0u32;
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                let x = self.ext_element(c, idx);
                exts.push(x);
                idx += 1;
                continue;
            }
            match c.local.as_str() {
                "transition" if tr.is_some() => self.duplicate(c, "initial"),
                "transition" => {
                    tr = Some(self.transition(c));
                    idx += 1;
                }
                _ => self.unexpected(c, "initial"),
            }
        }
        if tr.is_none() {
            self.error(
                ErrorKind::MissingElement,
                el.span,
                "<initial> requires a <transition>",
            );
        }
        Initial {
            transition: tr.unwrap_or_default(),
            extensions: exts,
            span: el.span,
        }
    }

    fn invoke(&mut self, el: Elem) -> Invoke {
        let mut inv = Invoke {
            span: el.span,
            ..Invoke::default()
        };
        let a = self.attrs(
            &el,
            &[
                "type",
                "typeexpr",
                "src",
                "srcexpr",
                "id",
                "idlocation",
                "namelist",
                "autoforward",
            ],
            Some(&mut inv.extensions),
        );
        for (x, y) in [
            ("type", "typeexpr"),
            ("src", "srcexpr"),
            ("id", "idlocation"),
        ] {
            self.exclusive(&a, x, y, &el);
        }
        inv.type_ = a.owned("type");
        inv.typeexpr = a.owned("typeexpr");
        inv.src = a.owned("src");
        inv.srcexpr = a.owned("srcexpr");
        inv.id = a.owned("id");
        inv.idlocation = a.owned("idlocation");
        inv.namelist = a.get("namelist").map(split_ws).unwrap_or_default();
        match a.get("autoforward") {
            None => {}
            Some("true") => inv.autoforward = true,
            Some("false") => self.loss(
                LossKind::RedundantAttribute,
                el.span,
                "autoforward=\"false\" is the default",
            ),
            Some(v) => {
                let msg = format!("autoforward `{v}` is not `true` or `false`");
                self.error(ErrorKind::InvalidValue, el.span, msg);
            }
        }
        let mut idx = 0u32;
        let mut seen_finalize = false;
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                let x = self.ext_element(c, idx);
                inv.extensions.push(x);
                idx += 1;
                continue;
            }
            match c.local.as_str() {
                "param" => {
                    let p = self.param(c);
                    inv.params.push(p);
                    idx += 1;
                }
                "content" if inv.content.is_some() => self.duplicate(c, "invoke"),
                "content" => {
                    inv.content = Some(self.content(c));
                    idx += 1;
                }
                "finalize" if seen_finalize => self.duplicate(c, "invoke"),
                "finalize" => {
                    seen_finalize = true;
                    let span = c.span;
                    let b = self.block_owner(c);
                    if b.is_empty() {
                        self.loss(LossKind::EmptyElement, span, "empty <finalize> not kept");
                    } else {
                        inv.finalize = b;
                        idx += 1;
                    }
                }
                _ => self.unexpected(c, "invoke"),
            }
        }
        self.exclusive_with_content(
            inv.content.is_some() && (inv.src.is_some() || inv.srcexpr.is_some()),
            "`src` or `srcexpr` and <content>",
            &el,
        );
        if let Some(src) = inv.src.clone()
            && inv.content.is_none()
            && let Some(body) = self.resolve(SourceKind::Invoke, &src, el.span)
        {
            inv.content = Some(Content {
                expr: None,
                body: Some(body),
                span: el.span,
            });
        }
        inv
    }

    fn param(&mut self, el: Elem) -> Param {
        let a = self.attrs(&el, &["name", "expr", "location"], None);
        let name = self.required(&a, "name", &el).unwrap_or_default();
        self.exclusive(&a, "expr", "location", &el);
        self.leaf_children(&el, None);
        Param {
            name,
            expr: a.owned("expr"),
            location: a.owned("location"),
            span: el.span,
        }
    }

    fn content(&mut self, el: Elem) -> Content {
        let a = self.attrs(&el, &["expr", "src"], None);
        let expr = a.owned("expr");
        let mut body = self.inline(&el);
        self.exclusive_with_content(
            expr.is_some() && body.is_some(),
            "`expr` and inline content",
            &el,
        );
        if let Some(src) = a.get("src") {
            if expr.is_some() || body.is_some() {
                self.exclusive_with_content(true, "`src` and `expr` or inline content", &el);
            } else {
                body = self.resolve(SourceKind::Content, src, el.span);
                if body.is_some() {
                    self.loss(
                        LossKind::InlinedSource,
                        el.span,
                        format!("<content src=\"{src}\"> inlined; the reference is not kept"),
                    );
                }
            }
        }
        Content {
            expr,
            body,
            span: el.span,
        }
    }

    fn donedata(&mut self, el: Elem) -> DoneData {
        self.attrs(&el, &[], None);
        let mut d = DoneData {
            content: None,
            params: Vec::new(),
            span: el.span,
        };
        while let Some(c) = self.child(&el) {
            if !c.is_scxml() {
                self.drop_element(c, "donedata");
                continue;
            }
            match c.local.as_str() {
                "param" => {
                    let p = self.param(c);
                    d.params.push(p);
                }
                "content" if d.content.is_some() => self.duplicate(c, "donedata"),
                "content" => d.content = Some(self.content(c)),
                _ => self.unexpected(c, "donedata"),
            }
        }
        self.exclusive_with_content(
            d.content.is_some() && !d.params.is_empty(),
            "<content> and <param>",
            &el,
        );
        d
    }
}

fn push_text(text: &mut Option<(String, Span)>, s: &str, span: Span) {
    match text {
        Some((t, _)) => t.push_str(s),
        None => *text = Some((s.to_owned(), span)),
    }
}

fn owned_ns(res: ResolveResult<'_>) -> Result<Option<String>, String> {
    match res {
        ResolveResult::Bound(ns) => Ok(Some(ns.as_ref().to_owned())),
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Unknown(p) => Err(p),
    }
}

/// Start-tag attributes of `el` inside a fragment, and the bindings it needs from outside.
fn frag_open(
    el: &Elem,
    declared: &mut Vec<Vec<Option<String>>>,
    needed: &mut Vec<(Option<String>, String)>,
) -> String {
    declared.push(el.attrs.iter().filter_map(|a| a.decl.clone()).collect());
    let is_declared = |declared: &[Vec<Option<String>>], p: &Option<String>| {
        declared.iter().any(|level| level.contains(p))
    };
    let mut need = |p: Option<String>, uri: String| {
        if !needed.iter().any(|(q, _)| *q == p) {
            needed.push((p, uri));
        }
    };
    match &el.prefix {
        None => {
            if !is_declared(declared, &None) && el.ns.as_deref() != Some(SCXML_NS) {
                need(None, el.ns.clone().unwrap_or_default());
            }
        }
        Some(p) if p == "xml" => {}
        Some(p) => {
            let p = Some(p.clone());
            if !is_declared(declared, &p) {
                need(p, el.ns.clone().unwrap_or_default());
            }
        }
    }
    for a in &el.attrs {
        if a.decl.is_some() {
            continue;
        }
        if let Some(p) = &a.prefix
            && p != "xml"
        {
            let p = Some(p.clone());
            if !is_declared(declared, &p) {
                need(p, a.ns.clone().unwrap_or_default());
            }
        }
    }
    let mut s = String::new();
    for a in &el.attrs {
        s.push(' ');
        s.push_str(&a.qname);
        s.push_str("=\"");
        escape_attr(&a.value, &mut s);
        s.push('"');
    }
    s
}
