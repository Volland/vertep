//! Types on chart IR nodes and the chart type checker.
//!
//! The IR carries types only through [`Extension`] nodes in the `vt:` namespace (see the table
//! in [`crate::types`]); the functions here read and write them. [`check_chart`] checks a typed
//! chart against its declarations and reports every error; [`compile_checked`] refuses to
//! compile, and so to instantiate, a chart with type errors.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::compile::CompiledChart;
use crate::datamodel::{Event, Value};
use crate::ir::{
    Block, Chart, Data, Exec, ExecKind, Extension, ExtensionValue, Invoke, Param, Span, State,
    Transition, VT_NS,
};
use crate::types::check::{TypeError, TypeErrorKind, ValuePath, is_assignable};
use crate::types::decl::{DeclError, Declarations};
use crate::types::events::EventTable;
use crate::types::syntax::{SyntaxError, parse_declarations, parse_type};
use crate::types::ty::{Field, Type};
use crate::validate::Violation;

/// Local name of the `vt:type` attribute on `<data>` and `<send>`.
pub const TYPE_ATTR: &str = "type";
/// Local name of the `vt:paramTypes` attribute that carries `<param>` types on the owner.
pub const PARAM_TYPES_ATTR: &str = "paramTypes";
/// Local name of the `vt:types` element on `<scxml>`.
pub const TYPES_ELEMENT: &str = "types";

// ===================================================================== extension accessors

fn find_attr<'e>(exts: &'e [Extension], local: &str) -> Option<&'e str> {
    exts.iter().find_map(|e| match &e.value {
        ExtensionValue::Attribute(v) if e.ns == VT_NS && e.local == local => Some(v.as_str()),
        _ => None,
    })
}

/// Set or remove the `vt:<local>` attribute, keeping attributes first and sorted by
/// `(namespace, local name)` as the SCXML reader produces them.
fn set_attr(exts: &mut Vec<Extension>, local: &str, value: Option<String>) {
    exts.retain(|e| {
        !(e.ns == VT_NS && e.local == local && matches!(e.value, ExtensionValue::Attribute(_)))
    });
    let Some(value) = value else { return };
    let at = exts
        .iter()
        .position(|e| {
            matches!(e.value, ExtensionValue::Element(_))
                || (e.ns.as_str(), e.local.as_str()) > (VT_NS, local)
        })
        .unwrap_or(exts.len());
    exts.insert(
        at,
        Extension {
            ns: VT_NS.to_string(),
            local: local.to_string(),
            value: ExtensionValue::Attribute(value),
            position: 0,
            span: Span::NONE,
        },
    );
}

/// The text of the `vt:type` annotation among `exts`, if any.
pub fn type_annotation(exts: &[Extension]) -> Option<&str> {
    find_attr(exts, TYPE_ATTR)
}

/// Set (or with `None` remove) the `vt:type` annotation among `exts`.
pub fn set_type_annotation(exts: &mut Vec<Extension>, ty: Option<&Type>) {
    set_attr(exts, TYPE_ATTR, ty.map(|t| t.to_string()));
}

/// The declared type of a `<data>` item (`vt:type`), parsed.
// @tg: implements:: [[openspec:type-system#Typed SCXML Attributes]] {confidence: partial, note: "IR carrier and accessors; the SCXML reader and writer live in vertep-scxml"}
pub fn data_type(data: &Data) -> Option<Result<Type, SyntaxError>> {
    type_annotation(&data.extensions).map(parse_type)
}

/// Set the declared type of a `<data>` item.
pub fn set_data_type(data: &mut Data, ty: Option<&Type>) {
    set_type_annotation(&mut data.extensions, ty);
}

/// The payload type declared on a `<send>` element (`vt:type` on its [`Exec`]), parsed.
pub fn send_type(exec: &Exec) -> Option<Result<Type, SyntaxError>> {
    type_annotation(&exec.extensions).map(parse_type)
}

/// Set the payload type declared on a `<send>` element.
pub fn set_send_type(exec: &mut Exec, ty: Option<&Type>) {
    set_type_annotation(&mut exec.extensions, ty);
}

/// The `<param>` types declared on an owner (a send's [`Exec`], an [`Invoke`] or a final
/// [`State`] for `<donedata>`): the fields of its `vt:paramTypes` record.
pub fn param_types(owner: &[Extension]) -> Option<Result<Vec<Field>, SyntaxError>> {
    find_attr(owner, PARAM_TYPES_ATTR).map(|s| match parse_type(s)? {
        Type::Record(fields) => Ok(fields),
        _ => Err(SyntaxError {
            offset: 0,
            message: "vt:paramTypes must be a record of parameter types".to_string(),
        }),
    })
}

/// Set (or with `None` remove) the declared type of parameter `name` on its owner.
pub fn set_param_type(owner: &mut Vec<Extension>, name: &str, ty: Option<&Type>) {
    let mut fields = param_types(owner).and_then(Result::ok).unwrap_or_default();
    match (fields.iter().position(|f| f.name == name), ty) {
        (Some(i), Some(t)) => fields[i].ty = t.clone(),
        (Some(i), None) => {
            fields.remove(i);
        }
        (None, Some(t)) => fields.push(Field::new(name, t.clone())),
        (None, None) => {}
    }
    let value = (!fields.is_empty()).then(|| Type::Record(fields).to_string());
    set_attr(owner, PARAM_TYPES_ATTR, value);
}

/// The chart's `vt:types` element, if any.
pub fn declarations_element(chart: &Chart) -> Option<&Extension> {
    chart.extensions.iter().find(|e| {
        e.ns == VT_NS && e.local == TYPES_ELEMENT && matches!(e.value, ExtensionValue::Element(_))
    })
}

/// The chart's declaration block (`vt:types`), parsed.
pub fn declarations(chart: &Chart) -> Option<Result<Declarations, SyntaxError>> {
    declarations_element(chart).map(|e| match &e.value {
        ExtensionValue::Element(xml) => parse_types_element(xml).map(|mut d| {
            for t in &mut d.types {
                t.span = e.span;
            }
            for ev in &mut d.events {
                ev.span = e.span;
            }
            d
        }),
        ExtensionValue::Attribute(_) => Ok(Declarations::default()),
    })
}

/// Set (or with `None` remove) the chart's declaration block. The element goes after the
/// extension attributes, before other extension elements, at child position 0.
pub fn set_declarations(chart: &mut Chart, decls: Option<&Declarations>) {
    chart.extensions.retain(|e| {
        !(e.ns == VT_NS
            && e.local == TYPES_ELEMENT
            && matches!(e.value, ExtensionValue::Element(_)))
    });
    let Some(decls) = decls else { return };
    let at = chart
        .extensions
        .iter()
        .position(|e| matches!(e.value, ExtensionValue::Element(_)))
        .unwrap_or(chart.extensions.len());
    chart.extensions.insert(
        at,
        Extension {
            ns: VT_NS.to_string(),
            local: TYPES_ELEMENT.to_string(),
            value: ExtensionValue::Element(types_element_xml(decls)),
            position: 0,
            span: Span::NONE,
        },
    );
}

/// The canonical XML of a `vt:types` element: the declaration block as escaped text in a
/// self-contained element, `<vt:types xmlns:vt="...">type A = ...;\n</vt:types>`.
pub fn types_element_xml(decls: &Declarations) -> String {
    let mut out = format!("<vt:types xmlns:vt=\"{VT_NS}\">");
    for c in decls.to_text().chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out.push_str("</vt:types>");
    out
}

/// Read the declaration block from the serialized XML of a `vt:types` element: its text
/// content (entities and CDATA sections decoded) parsed with [`parse_declarations`].
pub fn parse_types_element(xml: &str) -> Result<Declarations, SyntaxError> {
    let bad = |m: &str| SyntaxError {
        offset: 0,
        message: m.to_string(),
    };
    let xml = xml.trim();
    // End of the start tag, skipping quoted attribute values.
    let mut quote = None;
    let mut end = None;
    for (i, c) in xml.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| bad("vt:types: unterminated start tag"))?;
    if xml[..end].ends_with('/') {
        return Ok(Declarations::default());
    }
    let close = xml
        .rfind("</")
        .ok_or_else(|| bad("vt:types: missing end tag"))?;
    if close <= end {
        return Err(bad("vt:types: missing end tag"));
    }
    let inner = &xml[end + 1..close];
    let mut text = String::new();
    let mut rest = inner;
    while let Some(i) = rest.find(['&', '<']) {
        text.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r
                .find("]]>")
                .ok_or_else(|| bad("vt:types: unterminated CDATA"))?;
            text.push_str(&r[..e]);
            rest = &r[e + 3..];
        } else if rest.starts_with('<') {
            return Err(bad("vt:types: only text content is allowed"));
        } else {
            let e = rest.find(';').ok_or_else(|| bad("vt:types: bad entity"))?;
            let name = &rest[1..e];
            let c = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| name.strip_prefix('#').map(str::parse))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            };
            text.push(c.ok_or_else(|| bad("vt:types: bad entity"))?);
            rest = &rest[e + 1..];
        }
    }
    text.push_str(rest);
    parse_declarations(&text)
}

/// Whether the chart is typed: it has a `vt:types` block or a `vt:type` or `vt:paramTypes`
/// annotation on any node ([[openspec:type-system#Opt-In Chart Typing]]).
// @tg: implements:: [[openspec:type-system#Opt-In Chart Typing]]
pub fn is_typed(chart: &Chart) -> bool {
    fn annotated(exts: &[Extension]) -> bool {
        find_attr(exts, TYPE_ATTR).is_some() || find_attr(exts, PARAM_TYPES_ATTR).is_some()
    }
    fn block(b: &Block) -> bool {
        b.iter().any(|e| {
            annotated(&e.extensions)
                || match &e.kind {
                    ExecKind::If { branches } => branches.iter().any(|br| block(&br.body)),
                    ExecKind::Foreach { body, .. } => block(body),
                    _ => false,
                }
        })
    }
    fn state(s: &State) -> bool {
        annotated(&s.extensions)
            || s.data.iter().any(|d| annotated(&d.extensions))
            || s.onentry.iter().chain(&s.onexit).any(block)
            || s.transitions.iter().any(|t| block(&t.body))
            || s.initial_element
                .as_ref()
                .is_some_and(|i| block(&i.transition.body))
            || s.invokes
                .iter()
                .any(|i| annotated(&i.extensions) || block(&i.finalize))
            || s.children.iter().any(state)
    }
    declarations_element(chart).is_some()
        || chart.data.iter().any(|d| annotated(&d.extensions))
        || chart.states.iter().any(state)
}

// ======================================================================= expression typing

/// The static type of an expression, as far as an [`ExprTyper`] can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprType {
    /// The expression has this type.
    Known(Type),
    /// The `null` literal: assignable to optional types and `value`.
    Null,
    /// The typer cannot tell; no check is made.
    Unknown,
}

/// An error a typer found inside an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    /// Byte offset inside the expression text, when known.
    pub offset: Option<usize>,
    /// What is wrong.
    pub message: String,
}

/// What an expression can see.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    /// Named types.
    pub declarations: &'a Declarations,
    /// The context fields (every `<data>` item; untyped ones have type `value`).
    pub context: &'a [Field],
    /// The payload type of the current event; `None` when the event carries no payload.
    pub event: Option<&'a Type>,
}

/// Types expressions of one datamodel for the chart checker. The core ships [`BasicTyper`];
/// a datamodel crate (CEL, `rust`) provides a complete typer.
pub trait ExprTyper {
    /// The static type of a value expression, or the errors found in it.
    fn type_of(&self, expr: &str, scope: &Scope<'_>) -> Result<ExprType, Vec<ExprError>>;

    /// The static type of an assignable location (`<assign location>`).
    fn location_type(&self, location: &str, scope: &Scope<'_>) -> Result<ExprType, Vec<ExprError>> {
        self.type_of(location, scope)
    }
}

/// A datamodel-neutral typer: literals (`true`, `false`, `null`, integers, decimals, quoted
/// strings), and, when roots are configured, paths into the context (`ctx.total`) and the event
/// payload (`ev.data.amount`). In any other expression it checks that every context
/// reference names a declared field and otherwise answers [`ExprType::Unknown`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BasicTyper {
    /// The variable that holds the context (`ctx` in CEL and the `rust` datamodel).
    pub context_root: Option<String>,
    /// The variable that holds the current event (`ev` in CEL); its payload is `<root>.data`.
    pub event_root: Option<String>,
}

impl BasicTyper {
    /// The typer for a datamodel name: `cel` and `rust` use the `ctx` and `ev` roots; other
    /// datamodels only get literal typing.
    pub fn for_datamodel(datamodel: &str) -> Self {
        match datamodel {
            "cel" | "rust" => BasicTyper {
                context_root: Some("ctx".to_string()),
                event_root: Some("ev".to_string()),
            },
            _ => BasicTyper::default(),
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn literal_type(e: &str) -> Option<ExprType> {
    match e {
        "true" | "false" => return Some(ExprType::Known(Type::BOOL)),
        "null" => return Some(ExprType::Null),
        _ => {}
    }
    let digits = e.strip_prefix('-').unwrap_or(e);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return Some(ExprType::Known(if e.parse::<i64>().is_ok() {
            Type::INT
        } else {
            Type::DECIMAL
        }));
    }
    if crate::types::prim::is_decimal(e) {
        return Some(ExprType::Known(Type::DECIMAL));
    }
    for q in ['\'', '"'] {
        if e.len() >= 2 && e.starts_with(q) && e.ends_with(q) {
            let inner = &e[1..e.len() - 1];
            let mut escaped = false;
            let closed_early = inner.chars().any(|c| {
                let hit = c == q && !escaped;
                escaped = c == '\\' && !escaped;
                hit
            });
            if !closed_early {
                return Some(ExprType::Known(Type::STRING));
            }
        }
    }
    None
}

/// Whether `e` is a dotted path of identifiers.
fn path_segments(e: &str) -> Option<Vec<&str>> {
    let segs: Vec<&str> = e.split('.').collect();
    let ok = segs.iter().all(|s| {
        !s.is_empty()
            && s.chars().all(is_ident_char)
            && !s.starts_with(|c: char| c.is_ascii_digit())
    });
    ok.then_some(segs)
}

fn resolve<'t>(decls: &'t Declarations, mut t: &'t Type) -> Option<&'t Type> {
    for _ in 0..=decls.types.len() {
        match t {
            Type::Named(n) => t = decls.lookup(n)?,
            other => return Some(other),
        }
    }
    None
}

impl BasicTyper {
    /// Walk `rest` field by field from `ty`.
    fn walk_fields(
        &self,
        scope: &Scope<'_>,
        ty: &Type,
        rest: &[&str],
        what: &str,
        offset: usize,
    ) -> Result<ExprType, Vec<ExprError>> {
        let mut t = ty.clone();
        let mut at = offset;
        for seg in rest {
            match resolve(scope.declarations, &t) {
                Some(Type::Record(fields)) => match fields.iter().find(|f| f.name == *seg) {
                    Some(f) => t = f.ty.clone(),
                    None => {
                        return Err(Vec::from([ExprError {
                            offset: Some(at),
                            message: format!("unknown field `{seg}` of {what} (type {t})"),
                        }]));
                    }
                },
                Some(Type::Union(_)) => {
                    return Err(Vec::from([ExprError {
                        offset: Some(at),
                        message: format!(
                            "{what} has the union type {t}; discriminate on the variant before \
                             reading `{seg}`"
                        ),
                    }]));
                }
                Some(Type::Value) => return Ok(ExprType::Known(Type::Value)),
                _ => return Ok(ExprType::Unknown),
            }
            at += seg.len() + 1;
        }
        Ok(ExprType::Known(t))
    }

    /// Check every `<root>.<field>` context reference inside an arbitrary expression.
    fn scan_context_refs(&self, e: &str, base: usize, scope: &Scope<'_>) -> Vec<ExprError> {
        let mut errors = Vec::new();
        let Some(root) = &self.context_root else {
            return errors;
        };
        let needle = format!("{root}.");
        let mut quote: Option<char> = None;
        let mut prev: Option<char> = None;
        for (i, c) in e.char_indices() {
            match quote {
                Some(q) => {
                    if c == q && prev != Some('\\') {
                        quote = None;
                    }
                }
                None if c == '\'' || c == '"' => quote = Some(c),
                None => {
                    let boundary = !prev.is_some_and(|p| is_ident_char(p) || p == '.');
                    if boundary && e[i..].starts_with(&needle) {
                        let start = i + needle.len();
                        let len = e[start..]
                            .find(|c: char| !is_ident_char(c))
                            .unwrap_or(e.len() - start);
                        let field = &e[start..start + len];
                        if !field.is_empty() && !scope.context.iter().any(|f| f.name == field) {
                            errors.push(ExprError {
                                offset: Some(base + start),
                                message: format!("unknown context field `{field}`"),
                            });
                        }
                    }
                }
            }
            prev = Some(c);
        }
        errors
    }
}

impl ExprTyper for BasicTyper {
    fn type_of(&self, expr: &str, scope: &Scope<'_>) -> Result<ExprType, Vec<ExprError>> {
        let base = expr.len() - expr.trim_start().len();
        let e = expr.trim();
        if let Some(t) = literal_type(e) {
            return Ok(t);
        }
        if let Some(segs) = path_segments(e) {
            if self.context_root.as_deref() == Some(segs[0]) && segs.len() >= 2 {
                let field = segs[1];
                let Some(f) = scope.context.iter().find(|f| f.name == field) else {
                    return Err(Vec::from([ExprError {
                        offset: Some(base + segs[0].len() + 1),
                        message: format!("unknown context field `{field}`"),
                    }]));
                };
                let at = base + segs[0].len() + field.len() + 2;
                return self.walk_fields(scope, &f.ty, &segs[2..], "the context field", at);
            }
            if self.event_root.as_deref() == Some(segs[0]) && segs.get(1) == Some(&"data") {
                let Some(payload) = scope.event else {
                    return Err(Vec::from([ExprError {
                        offset: Some(base),
                        message: "the event carries no payload".to_string(),
                    }]));
                };
                let at = base + segs[0].len() + 6;
                return self.walk_fields(scope, payload, &segs[2..], "the event payload", at);
            }
        }
        let errors = self.scan_context_refs(e, base, scope);
        if errors.is_empty() {
            Ok(ExprType::Unknown)
        } else {
            Err(errors)
        }
    }
}

// ============================================================================ diagnostics

/// Which part of the chart a diagnostic concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// The `vt:types` block or a `vt:type` annotation does not parse.
    Syntax,
    /// A declaration is invalid (unknown type, duplicate, cycle, conflicting event).
    Declaration,
    /// A `<data>` declaration or initializer.
    Data,
    /// A `<send>` payload.
    Send,
    /// A `<param>`.
    Param,
    /// A transition guard.
    Guard,
    /// An action (assign, raise, if, foreach, log, ...).
    Action,
}

/// One type-checking error with its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// What part of the chart.
    pub kind: DiagnosticKind,
    /// A human-readable location in the IR, such as `state "cart", transition 0, send "pay"`.
    pub location: String,
    /// What is wrong.
    pub message: String,
    /// Source location of the IR node.
    pub span: Span,
    /// Byte offset inside the offending expression or annotation text, when known.
    pub offset: Option<usize>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.span.is_known() {
            write!(f, "{}:{}: ", self.span.line, self.span.column)?;
        }
        write!(f, "{}: {}", self.location, self.message)
    }
}

/// The resolved types of a chart that passed [`check_chart`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartTypes {
    /// Whether the chart is typed at all.
    pub typed: bool,
    /// Its declaration block (empty for untyped charts).
    pub declarations: Declarations,
    /// Its typed events.
    pub events: EventTable,
    /// Its context fields in document order; untyped `<data>` items have type `value`.
    pub context: Vec<Field>,
}

impl ChartTypes {
    /// The context as one record type.
    pub fn context_type(&self) -> Type {
        Type::Record(self.context.clone())
    }

    /// Check an incoming event's payload: a declared event's data must have its declared type
    /// (or be absent for an event declared without payload); an undeclared event is accepted
    /// with payload type `value`.
    // @tg: implements:: [[openspec:type-system#Opt-In Chart Typing]]
    pub fn check_event(&self, event: &Event) -> Result<(), Vec<TypeError>> {
        match self.events.iter().find(|(n, _)| *n == event.name) {
            None => Ok(()),
            Some((_, Some(t))) => self.declarations.check(t, &event.data),
            Some((_, None)) => match event.data {
                Value::Undefined | Value::Null => Ok(()),
                ref other => Err(Vec::from([TypeError {
                    path: ValuePath::default(),
                    kind: TypeErrorKind::Mismatch {
                        expected: "no payload".to_string(),
                        found: crate::types::check::kind_of(other),
                    },
                }])),
            },
        }
    }
}

/// Why a chart could not be compiled for instantiation.
#[derive(Clone, Debug, PartialEq)]
pub enum InstantiateError {
    /// Type errors; no session may be created.
    Type(Vec<Diagnostic>),
    /// Well-formedness violations.
    Invalid(Vec<Violation>),
}

/// Type-check, then compile a chart. A chart with type errors is refused, so no
/// [`CompiledChart`] and therefore no session exists for it.
// @tg: implements:: [[openspec:type-system#Chart Type Checking]]
pub fn compile_checked(
    chart: Chart,
    typer: &dyn ExprTyper,
) -> Result<(CompiledChart, ChartTypes), InstantiateError> {
    let types = check_chart(&chart, typer).map_err(InstantiateError::Type)?;
    let compiled = CompiledChart::new(chart).map_err(InstantiateError::Invalid)?;
    Ok((compiled, types))
}

/// Type-check a chart and report every error. An untyped chart passes without checks and
/// gets `value` everywhere. For a typed chart the checker validates the declaration block and
/// every `vt:type` annotation, then checks `<data>` initializers, `<send>` payloads against
/// the event's declared type, `<param>`s, guards (which must be `bool` and read only declared
/// context fields) and actions, using `typer` for expressions.
// @tg: implements:: [[openspec:type-system#Chart Type Checking]]
// @tg: implements:: [[openspec:type-system#Opt-In Chart Typing]]
pub fn check_chart(chart: &Chart, typer: &dyn ExprTyper) -> Result<ChartTypes, Vec<Diagnostic>> {
    if !is_typed(chart) {
        let mut context = Vec::new();
        collect_data(&chart.data, &chart.states, &mut |d| {
            if !context.iter().any(|f: &Field| f.name == d.id) {
                context.push(Field::new(d.id.clone(), Type::Value));
            }
        });
        return Ok(ChartTypes {
            typed: false,
            context,
            ..ChartTypes::default()
        });
    }
    let mut diags = Vec::new();
    let declarations = match declarations(chart) {
        None => Declarations::default(),
        Some(Ok(d)) => d,
        Some(Err(e)) => {
            diags.push(Diagnostic {
                kind: DiagnosticKind::Syntax,
                location: "vt:types".to_string(),
                message: e.message.clone(),
                span: declarations_element(chart).map_or(Span::NONE, |e| e.span),
                offset: Some(e.offset),
            });
            Declarations::default()
        }
    };
    for e in declarations.validate() {
        diags.push(decl_diag(&e, "vt:types"));
    }
    let events = EventTable::new(&declarations);

    let mut ck = Checker {
        decls: &declarations,
        events: &events,
        typer,
        context: Vec::new(),
        diags,
    };
    // Context: every <data> item, in document order.
    let mut data_items: Vec<(&Data, String)> = Vec::new();
    collect_data_located(chart, &mut data_items);
    for (d, loc) in &data_items {
        let ty = ck.annotation(
            type_annotation(&d.extensions),
            loc,
            d.span,
            DiagnosticKind::Data,
        );
        if !ck.context.iter().any(|f| f.name == d.id) {
            ck.context
                .push(Field::new(d.id.clone(), ty.unwrap_or(Type::Value)));
        }
    }
    for (d, loc) in &data_items {
        ck.data_init(d, loc);
    }
    for s in &chart.states {
        ck.state(s);
    }
    let Checker { context, diags, .. } = ck;
    if diags.is_empty() {
        Ok(ChartTypes {
            typed: true,
            declarations,
            events,
            context,
        })
    } else {
        Err(diags)
    }
}

fn decl_diag(e: &DeclError, location: &str) -> Diagnostic {
    Diagnostic {
        kind: DiagnosticKind::Declaration,
        location: location.to_string(),
        message: e.to_string(),
        span: e.span,
        offset: None,
    }
}

fn collect_data<'c>(top: &'c [Data], states: &'c [State], f: &mut dyn FnMut(&'c Data)) {
    fn walk<'c>(s: &'c State, f: &mut dyn FnMut(&'c Data)) {
        s.data.iter().for_each(&mut *f);
        s.children.iter().for_each(|c| walk(c, f));
    }
    top.iter().for_each(&mut *f);
    states.iter().for_each(|s| walk(s, f));
}

fn collect_data_located<'c>(chart: &'c Chart, out: &mut Vec<(&'c Data, String)>) {
    fn walk<'c>(s: &'c State, out: &mut Vec<(&'c Data, String)>) {
        for d in &s.data {
            out.push((d, format!("state \"{}\", data \"{}\"", s.id, d.id)));
        }
        s.children.iter().for_each(|c| walk(c, out));
    }
    for d in &chart.data {
        out.push((d, format!("data \"{}\"", d.id)));
    }
    chart.states.iter().for_each(|s| walk(s, out));
}

struct Checker<'a> {
    decls: &'a Declarations,
    events: &'a EventTable,
    typer: &'a dyn ExprTyper,
    context: Vec<Field>,
    diags: Vec<Diagnostic>,
}

/// The event scope of a transition or block: `Some(t)` a payload type, `None` no payload.
type EventScope = Option<Type>;

impl Checker<'_> {
    fn push(&mut self, kind: DiagnosticKind, loc: &str, span: Span, message: String) {
        self.diags.push(Diagnostic {
            kind,
            location: loc.to_string(),
            message,
            span,
            offset: None,
        });
    }

    /// Parse and validate a `vt:type` annotation.
    fn annotation(
        &mut self,
        text: Option<&str>,
        loc: &str,
        span: Span,
        kind: DiagnosticKind,
    ) -> Option<Type> {
        let text = text?;
        match parse_type(text) {
            Ok(t) => {
                let errs = self.decls.validate_type(&t, loc, span);
                if errs.is_empty() {
                    Some(t)
                } else {
                    for e in errs {
                        self.diags.push(Diagnostic {
                            kind,
                            location: loc.to_string(),
                            message: e.to_string(),
                            span,
                            offset: None,
                        });
                    }
                    None
                }
            }
            Err(e) => {
                self.diags.push(Diagnostic {
                    kind: DiagnosticKind::Syntax,
                    location: loc.to_string(),
                    message: format!("bad type `{text}`: {}", e.message),
                    span,
                    offset: Some(e.offset),
                });
                None
            }
        }
    }

    /// Type an expression, turning typer errors into diagnostics.
    fn expr(
        &mut self,
        expr: &str,
        event: &EventScope,
        kind: DiagnosticKind,
        loc: &str,
        span: Span,
        location: bool,
    ) -> ExprType {
        let scope = Scope {
            declarations: self.decls,
            context: &self.context,
            event: event.as_ref(),
        };
        let r = if location {
            self.typer.location_type(expr, &scope)
        } else {
            self.typer.type_of(expr, &scope)
        };
        match r {
            Ok(t) => t,
            Err(errs) => {
                for e in errs {
                    self.diags.push(Diagnostic {
                        kind,
                        location: loc.to_string(),
                        message: e.message,
                        span,
                        offset: e.offset,
                    });
                }
                ExprType::Unknown
            }
        }
    }

    /// Whether an expression of type `got` may be used where `want` is expected; reports a
    /// diagnostic when not.
    fn expect(
        &mut self,
        got: &ExprType,
        want: &Type,
        what: &str,
        kind: DiagnosticKind,
        loc: &str,
        span: Span,
    ) {
        let ok = match got {
            ExprType::Unknown => true,
            ExprType::Null => self.optional_or_value(want),
            ExprType::Known(t) => is_assignable(self.decls, t, want),
        };
        if !ok {
            let shown = match got {
                ExprType::Known(t) => t.to_string(),
                _ => "null".to_string(),
            };
            self.push(
                kind,
                loc,
                span,
                format!("{what} has type {shown}, expected {want}"),
            );
        }
    }

    fn optional_or_value(&self, t: &Type) -> bool {
        matches!(
            resolve(self.decls, t),
            Some(Type::Optional(_) | Type::Value)
        )
    }

    fn data_init(&mut self, d: &Data, loc: &str) {
        let want = self
            .context
            .iter()
            .find(|f| f.name == d.id)
            .map_or(Type::Value, |f| f.ty.clone());
        if let Some(expr) = &d.expr {
            let any: EventScope = Some(Type::Value);
            let got = self.expr(expr, &any, DiagnosticKind::Data, loc, d.span, false);
            if want != Type::Value {
                let what = format!("the initializer of `{}`", d.id);
                self.expect(&got, &want, &what, DiagnosticKind::Data, loc, d.span);
            }
        }
    }

    fn state(&mut self, s: &State) {
        let loc = format!("state \"{}\"", s.id);
        let any: EventScope = Some(Type::Value);
        for b in s.onentry.iter() {
            self.block(b, &any, &format!("{loc}, onentry"));
        }
        for b in s.onexit.iter() {
            self.block(b, &any, &format!("{loc}, onexit"));
        }
        if let Some(i) = &s.initial_element {
            self.transition(&i.transition, &format!("{loc}, initial"));
        }
        for (n, t) in s.transitions.iter().enumerate() {
            let on = if t.events.is_empty() {
                String::from("eventless")
            } else {
                format!("on {}", t.events.join(" "))
            };
            self.transition(t, &format!("{loc}, transition {n} ({on})"));
        }
        for (n, inv) in s.invokes.iter().enumerate() {
            self.invoke(inv, &format!("{loc}, invoke {n}"));
        }
        if let Some(dd) = &s.donedata {
            let ptypes = self.param_types(&s.extensions, &loc, s.span);
            let dl = format!("{loc}, donedata");
            self.params(&dd.params, &ptypes, &any, &dl);
            if let Some(c) = dd.content.as_ref().and_then(|c| c.expr.as_ref()) {
                self.expr(c, &any, DiagnosticKind::Action, &dl, dd.span, false);
            }
        }
        for c in &s.children {
            self.state(c);
        }
    }

    fn transition(&mut self, t: &Transition, loc: &str) {
        let scope = self.events.payload_for_descriptors(&t.events);
        if let Some(cond) = &t.cond {
            let got = self.expr(cond, &scope, DiagnosticKind::Guard, loc, t.span, false);
            self.expect(
                &got,
                &Type::BOOL,
                "the guard",
                DiagnosticKind::Guard,
                loc,
                t.span,
            );
        }
        self.block(&t.body, &scope, loc);
    }

    fn invoke(&mut self, inv: &Invoke, loc: &str) {
        let any: EventScope = Some(Type::Value);
        let ptypes = self.param_types(&inv.extensions, loc, inv.span);
        self.params(&inv.params, &ptypes, &any, loc);
        for e in [&inv.typeexpr, &inv.srcexpr]
            .into_iter()
            .flatten()
            .chain(inv.content.as_ref().and_then(|c| c.expr.as_ref()))
        {
            self.expr(e, &any, DiagnosticKind::Action, loc, inv.span, false);
        }
        self.block(&inv.finalize, &any, &format!("{loc}, finalize"));
    }

    fn param_types(&mut self, owner: &[Extension], loc: &str, span: Span) -> Vec<Field> {
        let Some(text) = find_attr(owner, PARAM_TYPES_ATTR) else {
            return Vec::new();
        };
        match self.annotation(Some(text), loc, span, DiagnosticKind::Param) {
            Some(Type::Record(fields)) => fields,
            Some(_) => {
                self.push(
                    DiagnosticKind::Syntax,
                    loc,
                    span,
                    "vt:paramTypes must be a record of parameter types".to_string(),
                );
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    /// Check params against their declared `vt:paramTypes`.
    fn params(&mut self, params: &[Param], ptypes: &[Field], event: &EventScope, loc: &str) {
        for p in params {
            let ploc = format!("{loc}, param \"{}\"", p.name);
            let got = match (&p.expr, &p.location) {
                (Some(e), _) => self.expr(e, event, DiagnosticKind::Param, &ploc, p.span, false),
                (None, Some(l)) => self.expr(l, event, DiagnosticKind::Param, &ploc, p.span, true),
                (None, None) => ExprType::Unknown,
            };
            if let Some(f) = ptypes.iter().find(|f| f.name == p.name) {
                let what = format!("param `{}`", p.name);
                self.expect(
                    &got,
                    &f.ty.clone(),
                    &what,
                    DiagnosticKind::Param,
                    &ploc,
                    p.span,
                );
            }
        }
    }

    fn block(&mut self, b: &Block, event: &EventScope, loc: &str) {
        for (i, e) in b.iter().enumerate() {
            self.exec(e, event, &format!("{loc}, action {i}"));
        }
    }

    fn exec(&mut self, e: &Exec, event: &EventScope, loc: &str) {
        let span = e.span;
        let act = DiagnosticKind::Action;
        match &e.kind {
            ExecKind::Send(send) => {
                let sloc = match &send.event {
                    Some(n) => format!("{loc} (send \"{n}\")"),
                    None => format!("{loc} (send)"),
                };
                self.send(e, send, event, &sloc);
            }
            ExecKind::Raise { event: name } => {
                if let Some(Some(t)) = self.events.payload_of(name)
                    && !self.optional_or_value(&t)
                {
                    self.push(
                        act,
                        &format!("{loc} (raise \"{name}\")"),
                        span,
                        format!("event `{name}` requires a payload of type {t}; <raise> cannot carry one, use <send>"),
                    );
                }
            }
            ExecKind::Assign { location, expr, .. } => {
                let want = self.expr(location, event, act, loc, span, true);
                if let Some(x) = expr {
                    let got = self.expr(x, event, act, loc, span, false);
                    if let ExprType::Known(w) = want {
                        self.expect(
                            &got,
                            &w,
                            &format!("the value assigned to `{location}`"),
                            act,
                            loc,
                            span,
                        );
                    }
                }
            }
            ExecKind::If { branches } => {
                for (n, br) in branches.iter().enumerate() {
                    let bloc = format!("{loc}, branch {n}");
                    if let Some(c) = &br.cond {
                        let got = self.expr(c, event, act, &bloc, br.span, false);
                        self.expect(&got, &Type::BOOL, "the condition", act, &bloc, br.span);
                    }
                    self.block(&br.body, event, &bloc);
                }
            }
            ExecKind::Foreach { array, body, .. } => {
                let got = self.expr(array, event, act, loc, span, false);
                if let ExprType::Known(t) = &got
                    && !matches!(resolve(self.decls, t), Some(Type::List(_) | Type::Value))
                {
                    self.push(
                        act,
                        loc,
                        span,
                        format!("foreach array has type {t}, expected a list"),
                    );
                }
                self.block(body, event, &format!("{loc}, foreach"));
            }
            ExecKind::Log { expr: Some(x), .. } => {
                self.expr(x, event, act, loc, span, false);
            }
            ExecKind::Cancel {
                sendidexpr: Some(x),
                ..
            } => {
                self.expr(x, event, act, loc, span, false);
            }
            ExecKind::Log { expr: None, .. }
            | ExecKind::Cancel {
                sendidexpr: None, ..
            }
            | ExecKind::Script(_)
            | ExecKind::Extension(_) => {}
        }
    }

    fn send(&mut self, e: &Exec, send: &crate::ir::Send, event: &EventScope, loc: &str) {
        let k = DiagnosticKind::Send;
        let span = e.span;
        let annotated = self.annotation(type_annotation(&e.extensions), loc, span, k);
        let declared = send.event.as_deref().and_then(|n| {
            self.events
                .iter()
                .find(|(m, _)| *m == n)
                .map(|(_, t)| t.cloned())
        });
        let payload: Option<Option<Type>> = match (declared, annotated) {
            (Some(d), Some(a)) => {
                if d.as_ref() != Some(&a) {
                    let shown = d
                        .as_ref()
                        .map_or("no payload".to_string(), |t| t.to_string());
                    self.push(
                        k,
                        loc,
                        span,
                        format!(
                            "vt:type {a} disagrees with the declared payload {shown} of event `{}`",
                            send.event.as_deref().unwrap_or_default()
                        ),
                    );
                }
                Some(d)
            }
            (Some(d), None) => Some(d),
            (None, Some(a)) => Some(Some(a)),
            (None, None) => None,
        };
        let ptypes = self.param_types(&e.extensions, loc, span);
        self.params(&send.params, &ptypes, event, loc);
        for x in [
            &send.eventexpr,
            &send.targetexpr,
            &send.typeexpr,
            &send.delayexpr,
        ]
        .into_iter()
        .flatten()
        {
            self.expr(x, event, k, loc, span, false);
        }
        let content_type = send
            .content
            .as_ref()
            .and_then(|c| c.expr.as_ref())
            .map(|x| self.expr(x, event, k, loc, span, false));
        let name = send.event.as_deref().unwrap_or_default();
        let supplies =
            !send.params.is_empty() || !send.namelist.is_empty() || send.content.is_some();
        match payload {
            None => {}
            Some(None) if supplies => {
                self.push(k, loc, span, format!("event `{name}` takes no payload"));
            }
            Some(None) => {}
            Some(Some(t)) => {
                if let Some(ct) = &content_type {
                    self.expect(ct, &t, "the content", k, loc, span);
                }
                let record = match resolve(self.decls, &t) {
                    Some(Type::Record(fields)) => Some(fields.clone()),
                    _ => None,
                };
                let Some(fields) = record else {
                    if !send.params.is_empty() || !send.namelist.is_empty() {
                        self.push(
                            k,
                            loc,
                            span,
                            format!("the payload type {t} of `{name}` is not a record; send it as content"),
                        );
                    }
                    return;
                };
                let mut supplied: Vec<&str> = Vec::new();
                for p in &send.params {
                    supplied.push(&p.name);
                    let Some(f) = fields.iter().find(|f| f.name == p.name) else {
                        self.push(
                            k,
                            loc,
                            p.span,
                            format!(
                                "unknown field `{}` in the payload of `{name}` (type {t})",
                                p.name
                            ),
                        );
                        continue;
                    };
                    let got = match (&p.expr, &p.location) {
                        (Some(x), _) => self.typer_quiet(x, event, false),
                        (None, Some(l)) => self.typer_quiet(l, event, true),
                        (None, None) => ExprType::Unknown,
                    };
                    self.expect(
                        &got,
                        &f.ty.clone(),
                        &format!("field `{}`", p.name),
                        k,
                        loc,
                        span,
                    );
                }
                for n in &send.namelist {
                    let field = n.rsplit('.').next().unwrap_or(n);
                    supplied.push(field);
                    match fields.iter().find(|f| f.name == field) {
                        None => self.push(
                            k,
                            loc,
                            span,
                            format!(
                                "unknown field `{field}` in the payload of `{name}` (type {t})"
                            ),
                        ),
                        Some(f) => {
                            let got = self.typer_quiet(n, event, true);
                            self.expect(
                                &got,
                                &f.ty.clone(),
                                &format!("field `{field}`"),
                                k,
                                loc,
                                span,
                            );
                        }
                    }
                }
                if send.content.is_none() {
                    for f in &fields {
                        if !supplied.contains(&f.name.as_str()) && !self.optional_or_value(&f.ty) {
                            self.push(
                                k,
                                loc,
                                span,
                                format!(
                                    "missing field `{}` in the payload of `{name}` (type {t})",
                                    f.name
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Type an expression whose errors were already reported by [`Checker::params`].
    fn typer_quiet(&self, expr: &str, event: &EventScope, location: bool) -> ExprType {
        let scope = Scope {
            declarations: self.decls,
            context: &self.context,
            event: event.as_ref(),
        };
        let r = if location {
            self.typer.location_type(expr, &scope)
        } else {
            self.typer.type_of(expr, &scope)
        };
        r.unwrap_or(ExprType::Unknown)
    }
}
