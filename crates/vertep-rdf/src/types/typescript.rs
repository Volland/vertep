//! The TypeScript projection of types, for the `@vertep/web` API (`vt gen ts`).
//!
//! A chart becomes a module with the union of its events (typed payloads), its context
//! interface, the union of its state identifiers and a typed `createActor`. The projection
//! agrees with the JSON Schema projection on the structure of values: records are closed
//! object types (TypeScript rejects excess properties of object literals), fields that may be
//! absent are optional, unions are tagged objects, lists are arrays and `value` is any JSON
//! value. Scalar refinements that TypeScript's type system cannot state are checked when the
//! event reaches the chart: integer range, and the decimal, date-time and IRI patterns.
//!
//! | type | TypeScript |
//! |---|---|
//! | `bool`, `string` | `boolean`, `string` |
//! | `int` | `number` |
//! | `decimal` | `number \| string` |
//! | `datetime`, `iri` | `string` |
//! | `value` | `JsonValue` |
//! | named type `N` | `N` (declared as `export type N = ...`) |
//! | `optional<T>` | `T \| null` (and an optional field) |
//! | `list<T>` | `T[]` |
//! | record | `{ f: T; g?: U \| null }` |
//! | union | `{ tag: "a"; value: T } \| { tag: "b" }` |
//!
//! An event is `{ type: "name" }` for an event without payload, `{ type: "name" } & R` for a
//! record payload `R` (fields beside `type`, as `@vertep/web` sends them), and
//! `{ type: "name"; data: T }` for any other payload, or a record with a field named `type`
//! or only a field named `data`.

use std::fmt::Write as _;

use vertep_core::ir::{Chart, State, StateKind};
use vertep_core::types::{ChartTypes, Declarations, Primitive, Type};

use super::shapes::may_be_absent;

/// Whether `name` is a TypeScript identifier that needs no quotes as a property name.
fn ident(name: &str) -> bool {
    let mut cs = name.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A string literal type.
fn lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

fn prop(name: &str) -> String {
    if ident(name) {
        name.to_owned()
    } else {
        lit(name)
    }
}

/// The TypeScript type of `ty`; named types are referenced by name.
// @tg: implements:: [[openspec:wasm-bindings#Generated TypeScript Types]]
pub fn ts_type(decls: &Declarations, ty: &Type) -> String {
    match ty {
        Type::Value => "JsonValue".into(),
        Type::Named(n) => n.clone(),
        Type::Primitive(p) => match p {
            Primitive::Bool => "boolean".into(),
            Primitive::String | Primitive::Datetime | Primitive::Iri => "string".into(),
            Primitive::Int => "number".into(),
            Primitive::Decimal => "number | string".into(),
        },
        Type::Optional(inner) => format!("{} | null", ts_type(decls, inner)),
        Type::List(item) => {
            let t = ts_type(decls, item);
            if t.contains(['|', '&']) {
                format!("({t})[]")
            } else {
                format!("{t}[]")
            }
        }
        Type::Record(fields) => {
            if fields.is_empty() {
                return "{ [field: string]: never }".into();
            }
            let parts: Vec<String> = fields
                .iter()
                .map(|f| {
                    let t = ts_type(decls, &f.ty);
                    if may_be_absent(decls, &f.ty) {
                        format!("{}?: {t}", prop(&f.name))
                    } else {
                        format!("{}: {t}", prop(&f.name))
                    }
                })
                .collect();
            format!("{{ {} }}", parts.join("; "))
        }
        Type::Union(variants) => {
            let alts: Vec<String> = variants
                .iter()
                .map(|v| match &v.payload {
                    None => format!("{{ tag: {} }}", lit(&v.tag)),
                    Some(p) => {
                        let opt = if may_be_absent(decls, p) { "?" } else { "" };
                        format!(
                            "{{ tag: {}; value{opt}: {} }}",
                            lit(&v.tag),
                            ts_type(decls, p)
                        )
                    }
                })
                .collect();
            alts.join(" | ")
        }
    }
}

/// The record a payload type resolves to, through named types.
fn record_of<'a>(decls: &'a Declarations, ty: &'a Type) -> Option<&'a [vertep_core::types::Field]> {
    let mut t = ty;
    for _ in 0..=decls.types.len() {
        match t {
            Type::Named(n) => t = decls.lookup(n)?,
            Type::Record(fields) => return Some(fields),
            _ => return None,
        }
    }
    None
}

/// Whether a record payload cannot be spread beside `type`: it has a `type` field, or its only
/// field is `data` (which the client reads as a non-record payload).
fn clashes(fields: &[vertep_core::types::Field]) -> bool {
    fields.iter().any(|f| f.name == "type") || (fields.len() == 1 && fields[0].name == "data")
}

/// The event type of event `name` with `payload`.
pub fn ts_event(decls: &Declarations, name: &str, payload: Option<&Type>) -> String {
    let head = format!("{{ type: {} }}", lit(name));
    let Some(p) = payload else {
        return head;
    };
    match record_of(decls, p) {
        Some(fields) if !clashes(fields) => {
            let t = ts_type(decls, p);
            format!(
                "{head} & {}",
                if t.contains('|') { format!("({t})") } else { t }
            )
        }
        _ => {
            let opt = if may_be_absent(decls, p) { "?" } else { "" };
            format!(
                "{{ type: {}; data{opt}: {} }}",
                lit(name),
                ts_type(decls, p)
            )
        }
    }
}

fn collect_states(states: &[State], out: &mut Vec<String>) {
    for s in states {
        if !matches!(s.kind(), StateKind::History(_)) && !s.id.is_empty() {
            out.push(s.id.clone());
        }
        collect_states(&s.children, out);
    }
}

/// The identifiers of a chart's states (not its history pseudo-states), in document order.
pub fn state_ids(chart: &Chart) -> Vec<String> {
    let mut out = Vec::new();
    collect_states(&chart.states, &mut out);
    out
}

/// A PascalCase prefix from a chart name or file stem.
pub fn pascal(name: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if up {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
            up = false;
        } else {
            up = true;
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "Chart");
    }
    out
}

/// What a generated module embeds about the chart source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TsSource<'a> {
    /// The PascalCase prefix of the generated names.
    pub name: &'a str,
    /// The chart source text.
    pub text: &'a str,
    /// `scxml`, `yaml` or `rdf`.
    pub format: &'a str,
    /// The chart version IRI.
    pub version: &'a str,
}

/// The TypeScript module for a chart: named types, `<Name>Event`, `<Name>Context`,
/// `<Name>StateId`, the chart spec `<name>Chart` and a typed `create<Name>Actor`. An untyped
/// chart's events and context are dynamic JSON values.
// @tg: implements:: [[openspec:wasm-bindings#Generated TypeScript Types]]
pub fn ts_module(chart: &Chart, types: &ChartTypes, src: &TsSource<'_>) -> String {
    let n = src.name;
    let decls = &types.declarations;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "// Generated by `vt gen ts` from chart {}. Do not edit.",
        src.version
    );
    let _ = writeln!(
        o,
        "import type {{ ActorRef, ActorSystem, ChartSpec, JsonValue }} from \"@vertep/web\";\n"
    );
    for d in &decls.types {
        let _ = writeln!(o, "export type {} = {};", d.name, ts_type(decls, &d.ty));
    }
    if !decls.types.is_empty() {
        o.push('\n');
    }
    let events: Vec<String> = types
        .events
        .iter()
        .map(|(name, p)| ts_event(decls, name, p))
        .collect();
    if types.typed && !events.is_empty() {
        let _ = writeln!(o, "export type {n}Event =");
        for e in &events {
            let _ = writeln!(o, "  | {e}");
        }
        o.pop();
        o.push_str(";\n");
    } else {
        let _ = writeln!(
            o,
            "export type {n}Event = {{ type: string; [field: string]: JsonValue }};"
        );
    }
    if types.typed {
        let fields: Vec<String> = types
            .context
            .iter()
            .map(|f| {
                let t = ts_type(decls, &f.ty);
                if may_be_absent(decls, &f.ty) {
                    format!("  {}?: {t};", prop(&f.name))
                } else {
                    format!("  {}: {t};", prop(&f.name))
                }
            })
            .collect();
        if fields.is_empty() {
            let _ = writeln!(o, "export type {n}Context = Record<string, never>;");
        } else {
            let _ = writeln!(
                o,
                "export interface {n}Context {{\n{}\n}}",
                fields.join("\n")
            );
        }
    } else {
        let _ = writeln!(o, "export type {n}Context = JsonValue;");
    }
    let ids: Vec<String> = state_ids(chart).iter().map(|s| lit(s)).collect();
    if ids.is_empty() {
        let _ = writeln!(o, "export type {n}StateId = never;");
    } else {
        let _ = writeln!(o, "export type {n}StateId = {};", ids.join(" | "));
    }
    let lower = {
        let mut c = n.chars();
        c.next()
            .map(|f| f.to_ascii_lowercase().to_string() + c.as_str())
            .unwrap_or_default()
    };
    let _ = writeln!(
        o,
        "\n/** The chart, typed: pass it to `spawn` or `virtual`. */\nexport const {lower}Chart: ChartSpec<{n}Event, {n}Context, {n}StateId> = {{\n  source: {},\n  format: {},\n  type: {},\n}};",
        lit(src.text),
        lit(src.format),
        lit(src.version)
    );
    let _ = writeln!(
        o,
        "\n/** The virtual actor of this chart with `key`, typed. */\nexport function create{n}Actor(system: ActorSystem, key: string): ActorRef<{n}Event, {n}Context, {n}StateId> {{\n  return system.virtual({lower}Chart, key);\n}}"
    );
    o
}
