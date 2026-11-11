//! JSON Schemas for decisions, tools and structured output: derivation from the IR type
//! language and a validator for the keywords the derivation produces.
//!
//! Schemas come from [`vertep_rdf::types::json_schema`] (the type system's JSON Schema
//! projection). The validator covers that projection's keywords — `type`, `properties`,
//! `required`, `additionalProperties`, `items`, `anyOf`, `oneOf`, `allOf`, `const`, `enum`,
//! `pattern`, `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `$ref` into
//! `#/$defs` — and reports every violation with its JSON Pointer path.

use std::collections::BTreeMap;

use serde_json::{Value as Json, json};
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;
use vertep_core::types::{BasicTyper, ChartTypes, Declarations, Type, check_chart, parse_type};

use crate::json::{canonical, sha256, to_json};

/// One schema violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// The JSON Pointer of the offending value (`""` for the root).
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let p = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        write!(f, "{p}: {}", self.message)
    }
}

/// The schema of the dynamic `value` type: every value conforms.
pub fn dynamic_value_schema() -> Json {
    json!({})
}

/// The content hash of a schema (`sha256:` of its canonical JSON).
pub fn schema_hash(schema: &Json) -> String {
    sha256(canonical(schema).as_bytes())
}

/// Validates `value` against `schema`.
///
/// # Errors
///
/// Every violation, with its path.
pub fn validate(schema: &Json, value: &Value) -> Result<(), Vec<Violation>> {
    validate_json(schema, &to_json(value))
}

/// Validates a JSON document against `schema`.
///
/// # Errors
///
/// Every violation, with its path.
pub fn validate_json(schema: &Json, doc: &Json) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    check(schema, schema, doc, "", &mut out, 0);
    if out.is_empty() { Ok(()) } else { Err(out) }
}

fn push(out: &mut Vec<Violation>, path: &str, message: impl Into<String>) {
    out.push(Violation {
        path: path.to_owned(),
        message: message.into(),
    });
}

fn type_name(v: &Json) -> &'static str {
    match v {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Json::Number(n) if n.as_f64().is_some_and(|f| f.fract() == 0.0) => "integer",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

fn is_type(v: &Json, t: &str) -> bool {
    let actual = type_name(v);
    actual == t || (t == "number" && actual == "integer")
}

fn resolve<'a>(root: &'a Json, r: &str) -> Option<&'a Json> {
    let p = r.strip_prefix('#')?;
    root.pointer(p)
}

fn check(root: &Json, schema: &Json, v: &Json, path: &str, out: &mut Vec<Violation>, depth: u32) {
    if depth > 64 {
        push(out, path, "schema nesting too deep");
        return;
    }
    let Json::Object(s) = schema else {
        if schema == &Json::Bool(false) {
            push(out, path, "no value is allowed here");
        }
        return;
    };
    if let Some(Json::String(r)) = s.get("$ref") {
        match resolve(root, r) {
            Some(t) => check(root, t, v, path, out, depth + 1),
            None => push(out, path, format!("unresolvable $ref {r}")),
        }
    }
    if let Some(t) = s.get("type") {
        let ok = match t {
            Json::String(t) => is_type(v, t),
            Json::Array(ts) => ts.iter().filter_map(Json::as_str).any(|t| is_type(v, t)),
            _ => true,
        };
        if !ok {
            push(
                out,
                path,
                format!("expected {}, found {}", t_display(t), type_name(v)),
            );
            return;
        }
    }
    if let Some(c) = s.get("const")
        && !json_eq(c, v)
    {
        push(out, path, format!("expected the constant {c}"));
    }
    if let Some(Json::Array(e)) = s.get("enum")
        && !e.iter().any(|x| json_eq(x, v))
    {
        push(
            out,
            path,
            format!("expected one of {}", Json::Array(e.clone())),
        );
    }
    if let (Some(Json::String(p)), Json::String(text)) = (s.get("pattern"), v) {
        match regex::Regex::new(p) {
            Ok(re) if re.is_match(text) => {}
            Ok(_) => push(out, path, format!("does not match the pattern {p}")),
            Err(_) => push(out, path, format!("invalid pattern {p}")),
        }
    }
    if let Some(n) = v.as_f64() {
        let bound = |k: &str| s.get(k).and_then(Json::as_f64);
        if bound("minimum").is_some_and(|m| n < m) {
            push(out, path, "below the minimum");
        }
        if bound("maximum").is_some_and(|m| n > m) {
            push(out, path, "above the maximum");
        }
        if bound("exclusiveMinimum").is_some_and(|m| n <= m) {
            push(out, path, "not above the exclusive minimum");
        }
        if bound("exclusiveMaximum").is_some_and(|m| n >= m) {
            push(out, path, "not below the exclusive maximum");
        }
    }
    if let Json::Object(obj) = v {
        if let Some(Json::Array(req)) = s.get("required") {
            for r in req.iter().filter_map(Json::as_str) {
                if !obj.contains_key(r) {
                    push(out, &format!("{path}/{r}"), "required field is missing");
                }
            }
        }
        let props = s.get("properties").and_then(Json::as_object);
        for (k, x) in obj {
            let p = format!("{path}/{k}");
            match props.and_then(|m| m.get(k)) {
                Some(ps) => check(root, ps, x, &p, out, depth + 1),
                None => match s.get("additionalProperties") {
                    Some(Json::Bool(false)) => push(out, &p, "field is not allowed"),
                    Some(a @ Json::Object(_)) => check(root, a, x, &p, out, depth + 1),
                    _ => {}
                },
            }
        }
    }
    if let (Some(items), Json::Array(a)) = (s.get("items"), v) {
        for (i, x) in a.iter().enumerate() {
            check(root, items, x, &format!("{path}/{i}"), out, depth + 1);
        }
    }
    if let Some(Json::Array(all)) = s.get("allOf") {
        for a in all {
            check(root, a, v, path, out, depth + 1);
        }
    }
    for (key, exactly_one) in [("anyOf", false), ("oneOf", true)] {
        if let Some(Json::Array(alts)) = s.get(key) {
            let mut best: Option<Vec<Violation>> = None;
            let mut matched = 0;
            for a in alts {
                let mut sub = Vec::new();
                check(root, a, v, path, &mut sub, depth + 1);
                if sub.is_empty() {
                    matched += 1;
                } else if best.as_ref().is_none_or(|b| sub.len() < b.len()) {
                    best = Some(sub);
                }
            }
            if matched == 0 {
                match best {
                    Some(b) if alts.len() == 1 || deepest(&b) > path.len() => out.extend(b),
                    _ => push(out, path, format!("matches none of the {key} alternatives")),
                }
            } else if exactly_one && matched > 1 {
                push(out, path, "matches more than one oneOf alternative");
            }
        }
    }
}

fn deepest(v: &[Violation]) -> usize {
    v.iter().map(|x| x.path.len()).max().unwrap_or(0)
}

fn t_display(t: &Json) -> String {
    match t {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn json_eq(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// The resolved types of a chart: the type checker's result for a typed chart, untyped
/// otherwise. A chart with type errors gets the untyped view (it is refused elsewhere).
pub fn chart_types(chart: &Chart) -> ChartTypes {
    check_chart(chart, &BasicTyper::for_datamodel(&chart.datamodel)).unwrap_or_default()
}

/// Payload schemas of a chart's events, derived from the IR type language.
#[derive(Clone, Debug, Default)]
pub struct EventSchemas {
    types: ChartTypes,
    cache: BTreeMap<String, Json>,
}

impl EventSchemas {
    /// The schemas of `chart`.
    pub fn of(chart: &Chart) -> Self {
        EventSchemas::from_types(chart_types(chart))
    }

    /// The schemas of a type-checked chart.
    pub fn from_types(types: ChartTypes) -> Self {
        let mut cache = BTreeMap::new();
        for (name, payload) in types.events.iter() {
            let s = match payload {
                Some(t) => vertep_rdf::types::json_schema(&types.declarations, t)
                    .unwrap_or_else(|_| dynamic_value_schema()),
                None => json!({ "type": "null" }),
            };
            cache.insert(name.to_owned(), s);
        }
        EventSchemas { types, cache }
    }

    /// Whether the chart is typed.
    pub fn typed(&self) -> bool {
        self.types.typed
    }

    /// The declarations of the chart.
    pub fn declarations(&self) -> &Declarations {
        &self.types.declarations
    }

    /// The resolved chart types.
    pub fn types(&self) -> &ChartTypes {
        &self.types
    }

    /// The payload schema of event `name`: the projection of its declared payload type, or
    /// the dynamic value schema for an undeclared event or an untyped chart.
    // @tg: implements:: [[openspec:deciders#Typed Decision Schemas]]
    pub fn payload(&self, name: &str) -> Json {
        self.cache
            .get(name)
            .cloned()
            .unwrap_or_else(dynamic_value_schema)
    }

    /// The schema of a type written in the type language, against this chart's declarations.
    ///
    /// # Errors
    ///
    /// The syntax error or the undeclared names.
    pub fn of_type_text(&self, text: &str) -> Result<(Type, Json), String> {
        type_schema(&self.types.declarations, text)
    }
}

/// The type and schema of `text` (in the IR type language) against `decls`.
///
/// # Errors
///
/// The syntax error or the undeclared names.
pub fn type_schema(decls: &Declarations, text: &str) -> Result<(Type, Json), String> {
    let ty = parse_type(text).map_err(|e| e.message)?;
    let s = vertep_rdf::types::json_schema(decls, &ty)
        .map_err(|missing| format!("undeclared types: {}", missing.join(", ")))?;
    Ok((ty, s))
}

/// A core value path as a JSON Pointer, like the validator's paths (`$.items[2].amount` is
/// `/items/2/amount`; a union variant's payload adds no segment).
fn pointer(path: &vertep_core::types::check::ValuePath) -> String {
    use vertep_core::types::check::PathSeg;
    let mut out = String::new();
    for seg in &path.0 {
        match seg {
            PathSeg::Field(n) => {
                out.push('/');
                out.push_str(&n.replace('~', "~0").replace('/', "~1"));
            }
            PathSeg::Index(i) => out.push_str(&format!("/{i}")),
            PathSeg::Variant(_) => {}
        }
    }
    out
}

/// Checks `value` against `ty` with the core checker, as violations.
///
/// # Errors
///
/// Every type error, with its path.
pub fn check_type(decls: &Declarations, ty: &Type, value: &Value) -> Result<(), Vec<Violation>> {
    decls.check(ty, value).map_err(|errs| {
        errs.into_iter()
            .map(|e| Violation {
                path: pointer(&e.path),
                message: e.to_string(),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_reports_paths() {
        let s = json!({
            "type": "object",
            "properties": { "amount": { "type": "number" }, "reason": { "type": "string" } },
            "required": ["amount", "reason"],
            "additionalProperties": false
        });
        let ok = crate::json::from_json(&json!({"amount": 3, "reason": "x"}));
        assert!(validate(&s, &ok).is_ok());
        let bad = crate::json::from_json(&json!({"amount": "3"}));
        let v = validate(&s, &bad).unwrap_err();
        let paths: Vec<&str> = v.iter().map(|x| x.path.as_str()).collect();
        assert!(paths.contains(&"/reason"), "{v:?}");
        assert!(paths.contains(&"/amount"), "{v:?}");
    }

    #[test]
    fn one_of_and_refs() {
        let s = json!({
            "$defs": { "A": { "type": "object", "properties": {"x": {"const": 1}}, "required": ["x"] } },
            "oneOf": [ { "$ref": "#/$defs/A" }, { "type": "string" } ]
        });
        assert!(validate_json(&s, &json!({"x": 1})).is_ok());
        assert!(validate_json(&s, &json!("y")).is_ok());
        assert!(validate_json(&s, &json!({"x": 2})).is_err());
    }
}
