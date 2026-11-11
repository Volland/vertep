//! The SCXML typing extension: `vt:type` on `<data>`, `<send>` and `<param>`, and the
//! `vt:types` element ([[openspec:type-system#Typed SCXML Attributes]]).
//!
//! The reader keeps `vt:type` on `<data>` and `<send>` as extension attributes of their IR
//! nodes, moves `vt:type` on `<param>` into its owner's `vt:paramTypes` record (the IR's
//! `Param` has no extension slot), and keeps `vt:types` as an extension element of the chart,
//! all through the carriers of [`vertep_core::types::chart`]. [`normalize`] then rewrites every
//! annotation and the declaration block in its canonical text, so that the same types read from
//! any document (and from the YAML surface) give the same IR and the same chart version, and
//! reports annotations that do not parse or name undeclared types, suggesting the closest
//! declared name. Other declaration errors (duplicates, cycles, conflicting events) are left to
//! the type checker.

use vertep_core::ir::{
    Block, Chart, Data, ExecKind, Extension, ExtensionValue, Span, State, VT_NS,
};
use vertep_core::types::chart::{
    PARAM_TYPES_ATTR, TYPE_ATTR, TYPES_ELEMENT, parse_types_element, types_element_xml,
};
use vertep_core::types::decl::DeclErrorKind;
use vertep_core::types::syntax::parse_type;
use vertep_core::types::{Declarations, Type};

use crate::error::{ErrorKind, ParseError};

fn error(span: Span, message: String) -> ParseError {
    ParseError {
        kind: ErrorKind::Type,
        span,
        message,
    }
}

struct Normalizer {
    decls: Declarations,
    errors: Vec<ParseError>,
}

/// Canonicalize the chart's type annotations and declaration block and report those that do
/// not parse or use undeclared names.
// @tg: implements:: [[openspec:type-system#Typed SCXML Attributes]]
// @tg: implements:: [[openspec:type-system#Types Survive Every Surface]]
pub(crate) fn normalize(chart: &mut Chart) -> Vec<ParseError> {
    let mut n = Normalizer {
        decls: Declarations::default(),
        errors: Vec::new(),
    };
    let mut block_span = None;
    for x in &mut chart.extensions {
        if x.ns != VT_NS || x.local != TYPES_ELEMENT {
            continue;
        }
        if let ExtensionValue::Element(xml) = &mut x.value {
            block_span = Some(x.span);
            match parse_types_element(xml) {
                Ok(d) => {
                    *xml = types_element_xml(&d);
                    n.decls = d;
                }
                Err(e) => n.errors.push(error(
                    x.span,
                    format!("vt:types does not parse: {}", e.message),
                )),
            }
            break;
        }
    }
    if let Some(span) = block_span {
        for e in n.decls.validate() {
            if let DeclErrorKind::UnknownType { name, suggestion } = &e.kind {
                let msg = unknown(&format!("vt:types, in `{}`", e.decl), name, suggestion);
                n.errors.push(error(span, msg));
            }
        }
    }
    for d in &mut chart.data {
        n.data(d);
    }
    for s in &mut chart.states {
        n.state(s);
    }
    n.errors
}

fn unknown(at: &str, name: &str, suggestion: &Option<String>) -> String {
    let mut m = format!("{at}: unknown type `{name}`");
    if let Some(s) = suggestion {
        m.push_str(&format!("; did you mean `{s}`?"));
    }
    m
}

impl Normalizer {
    /// Report undeclared names in `ty`, annotated at `at`.
    fn check_names(&mut self, ty: &Type, at: &str, span: Span) {
        for e in self.decls.validate_type(ty, at, span) {
            if let DeclErrorKind::UnknownType { name, suggestion } = &e.kind {
                self.errors.push(error(span, unknown(at, name, suggestion)));
            }
        }
    }

    /// Canonicalize and check the `vt:type` attribute among `exts`.
    fn type_attr(&mut self, exts: &mut [Extension], at: &str) {
        let mut found = None;
        for x in exts.iter_mut() {
            if x.ns != VT_NS || x.local != TYPE_ATTR {
                continue;
            }
            if let ExtensionValue::Attribute(text) = &mut x.value {
                match parse_type(text) {
                    Ok(t) => {
                        *text = t.to_string();
                        found = Some((t, x.span));
                    }
                    Err(e) => self.errors.push(error(
                        x.span,
                        format!("vt:type=\"{text}\" on {at} does not parse: {}", e.message),
                    )),
                }
            }
        }
        if let Some((t, span)) = found {
            self.check_names(&t, &format!("vt:type on {at}"), span);
        }
    }

    /// Canonicalize and check the `vt:paramTypes` record among `exts`.
    fn param_types(&mut self, exts: &mut [Extension], at: &str) {
        let mut found = None;
        for x in exts.iter_mut() {
            if x.ns != VT_NS || x.local != PARAM_TYPES_ATTR {
                continue;
            }
            if let ExtensionValue::Attribute(text) = &mut x.value {
                match parse_type(text) {
                    Ok(t @ Type::Record(_)) => {
                        *text = t.to_string();
                        found = Some((t, x.span));
                    }
                    Ok(_) => self.errors.push(error(
                        x.span,
                        format!("vt:paramTypes on {at} must be a record of parameter types"),
                    )),
                    Err(e) => self.errors.push(error(
                        x.span,
                        format!("vt:paramTypes on {at} does not parse: {}", e.message),
                    )),
                }
            }
        }
        if let Some((Type::Record(fields), span)) = found {
            for f in fields {
                self.check_names(
                    &f.ty,
                    &format!("vt:type on <param name=\"{}\">", f.name),
                    span,
                );
            }
        }
    }

    fn data(&mut self, d: &mut Data) {
        let at = format!("<data id=\"{}\">", d.id);
        self.type_attr(&mut d.extensions, &at);
    }

    fn block(&mut self, b: &mut Block) {
        for e in b.iter_mut() {
            match &mut e.kind {
                ExecKind::Send(s) => {
                    let at = match (&s.event, &s.eventexpr) {
                        (Some(ev), _) => format!("<send event=\"{ev}\">"),
                        (None, Some(x)) => format!("<send eventexpr=\"{x}\">"),
                        (None, None) => "<send>".to_owned(),
                    };
                    self.type_attr(&mut e.extensions, &at);
                    self.param_types(&mut e.extensions, &at);
                }
                ExecKind::If { branches } => {
                    for br in branches {
                        self.block(&mut br.body);
                    }
                }
                ExecKind::Foreach { body, .. } => self.block(body),
                _ => {}
            }
        }
    }

    fn state(&mut self, s: &mut State) {
        let at = format!("<{}>", if s.id.is_empty() { "final" } else { &s.id });
        if s.donedata.is_some() {
            self.param_types(&mut s.extensions, &format!("the <donedata> of {at}"));
        }
        for d in &mut s.data {
            self.data(d);
        }
        for b in s.onentry.iter_mut().chain(s.onexit.iter_mut()) {
            self.block(b);
        }
        for t in &mut s.transitions {
            self.block(&mut t.body);
        }
        if let Some(i) = &mut s.initial_element {
            self.block(&mut i.transition.body);
        }
        for inv in &mut s.invokes {
            let at = format!("<invoke> in state `{}`", s.id);
            self.param_types(&mut inv.extensions, &at);
            self.block(&mut inv.finalize);
        }
        for c in &mut s.children {
            self.state(c);
        }
    }
}
