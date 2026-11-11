//! The JSON Schema (draft 2020-12) projection of types.
//!
//! A JSON document validates against the schema of a type exactly when its decoding (JSON
//! numbers as `f64`) is accepted by the core checker. The projection uses the keywords that
//! structured-output interfaces support: `type`, `properties`, `required`,
//! `additionalProperties: false`, `items`, `anyOf`, `const`, `pattern`, `minimum`,
//! `exclusiveMaximum`, `$ref` and `$defs`, plus the annotation `format: date-time`.
//!
//! | type | schema |
//! |---|---|
//! | `bool`, `string` | `{"type": "boolean"}`, `{"type": "string"}` |
//! | `int` | `{"type": "integer"}` in `[-2^63, 2^63)` |
//! | `decimal` | `anyOf` a number or a string matching `-?[0-9]+(\.[0-9]+)?` |
//! | `datetime`, `iri` | a string with the RFC 3339 (offset required) or IRI `pattern` |
//! | `value` | `{}` |
//! | named type `N` | `{"$ref": "#/$defs/N"}` |
//! | `optional<T>` | `anyOf` `{"type": "null"}` and `T` |
//! | `list<T>` | `{"type": "array", "items": T}` |
//! | record | a closed object: `properties`, `required` (fields that are not optional or `value`), `additionalProperties: false` |
//! | union | `anyOf` one closed object per variant with `tag` `const` and its `value` |

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json, json};
use vertep_core::types::{ChartTypes, Declarations, Primitive, Type};

use super::shapes::{DATETIME_PATTERN, DECIMAL_PATTERN, IRI_PATTERN, may_be_absent};

/// The `$schema` of every document.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The schema of `ty` as a standalone JSON Schema 2020-12 document: the named types it uses
/// (transitively) are in `$defs`. Fails with the names `ty` uses that `decls` does not declare.
// @tg: implements:: [[openspec:type-system#JSON Schema Projection]]
pub fn json_schema(decls: &Declarations, ty: &Type) -> Result<Json, Vec<String>> {
    let mut defs: BTreeMap<String, Json> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut pending: Vec<String> = ty.references().into_iter().map(str::to_owned).collect();
    while let Some(n) = pending.pop() {
        if defs.contains_key(&n) || missing.contains(&n) {
            continue;
        }
        match decls.lookup(&n) {
            Some(d) => {
                pending.extend(d.references().into_iter().map(str::to_owned));
                let mut s = schema(decls, d);
                if let Json::Object(m) = &mut s {
                    m.insert("title".into(), Json::String(n.clone()));
                }
                defs.insert(n, s);
            }
            None => missing.push(n),
        }
    }
    if !missing.is_empty() {
        missing.sort();
        return Err(missing);
    }
    let mut doc = Map::new();
    doc.insert("$schema".into(), Json::String(DIALECT.into()));
    match schema(decls, ty) {
        Json::Object(m) => doc.extend(m),
        other => {
            doc.insert("allOf".into(), json!([other]));
        }
    }
    if !defs.is_empty() {
        doc.insert("$defs".into(), Json::Object(defs.into_iter().collect()));
    }
    Ok(Json::Object(doc))
}

/// The schema of one type expression; named types are `$ref`s into `$defs`.
pub fn schema(decls: &Declarations, ty: &Type) -> Json {
    match ty {
        Type::Value => json!({}),
        Type::Named(n) => json!({ "$ref": format!("#/$defs/{n}") }),
        Type::Primitive(p) => match p {
            Primitive::Bool => json!({ "type": "boolean" }),
            Primitive::String => json!({ "type": "string" }),
            Primitive::Int => json!({
                "type": "integer",
                "minimum": i64::MIN,
                "exclusiveMaximum": 9_223_372_036_854_775_808_u64,
            }),
            Primitive::Decimal => json!({
                "anyOf": [
                    { "type": "number" },
                    { "type": "string", "pattern": DECIMAL_PATTERN },
                ]
            }),
            Primitive::Datetime => json!({
                "type": "string",
                "format": "date-time",
                "pattern": DATETIME_PATTERN,
            }),
            Primitive::Iri => json!({ "type": "string", "pattern": IRI_PATTERN }),
        },
        Type::Optional(inner) => json!({ "anyOf": [ { "type": "null" }, schema(decls, inner) ] }),
        Type::List(item) => json!({ "type": "array", "items": schema(decls, item) }),
        Type::Record(fields) => {
            let mut props = Map::new();
            let mut required = Vec::new();
            for f in fields {
                props.insert(f.name.clone(), schema(decls, &f.ty));
                if !may_be_absent(decls, &f.ty) {
                    required.push(Json::String(f.name.clone()));
                }
            }
            json!({
                "type": "object",
                "properties": props,
                "required": required,
                "additionalProperties": false,
            })
        }
        Type::Union(variants) => {
            let alts: Vec<Json> = variants
                .iter()
                .map(|v| {
                    let mut props = Map::new();
                    props.insert("tag".into(), json!({ "const": v.tag }));
                    let mut required = vec![json!("tag")];
                    if let Some(p) = &v.payload {
                        props.insert("value".into(), schema(decls, p));
                        if !may_be_absent(decls, p) {
                            required.push(json!("value"));
                        }
                    }
                    json!({
                        "type": "object",
                        "properties": props,
                        "required": required,
                        "additionalProperties": false,
                    })
                })
                .collect();
            json!({ "anyOf": alts })
        }
    }
}

/// The JSON Schema documents of a chart: one for the context record and one per declared
/// event payload (`{"type": "null"}` for an event declared without payload).
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSchemas {
    /// The context type's schema.
    pub context: Json,
    /// Each declared event's payload schema, by event name.
    pub events: BTreeMap<String, Json>,
}

impl ChartSchemas {
    /// Project the resolved types of a chart that passed the type checker.
    pub fn of(types: &ChartTypes) -> Result<ChartSchemas, Vec<String>> {
        let context = json_schema(&types.declarations, &types.context_type())?;
        let mut events = BTreeMap::new();
        for (name, payload) in types.events.iter() {
            let s = match payload {
                Some(t) => json_schema(&types.declarations, t)?,
                None => json!({ "$schema": DIALECT, "type": "null" }),
            };
            events.insert(name.to_owned(), s);
        }
        Ok(ChartSchemas { context, events })
    }

    /// The documents as one JSON object `{"context": ..., "events": {...}}`.
    pub fn to_json(&self) -> Json {
        json!({
            "context": self.context,
            "events": self.events,
        })
    }
}
