//! Conversions between the datamodel-independent [`Value`] and JSON.

use serde_json::{Map, Number, Value as Json};
use vertep_core::datamodel::Value;

/// `v` as JSON: `undefined` becomes `null`, whole numbers become JSON integers.
pub fn to_json(v: &Value) -> Json {
    match v {
        Value::Undefined | Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => number(*n),
        Value::String(s) => Json::String(s.clone()),
        Value::Array(a) => Json::Array(a.iter().map(to_json).collect()),
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, v) in o {
                m.insert(k.clone(), to_json(v));
            }
            Json::Object(m)
        }
    }
}

fn number(n: f64) -> Json {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Json::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(Json::Null, Json::Number)
    }
}

/// `j` as a [`Value`], in the key order of the JSON map.
pub fn from_json(j: &Json) -> Value {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(a) => Value::Array(a.iter().map(from_json).collect()),
        Json::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect())
        }
    }
}

/// The field `key` of an object value.
pub fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

/// The value at a dotted `path` (`a.b.0.c`) inside `v`.
pub fn at_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(_) => field(cur, seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// An object value from `(key, value)` pairs.
pub fn obj<I: IntoIterator<Item = (K, Value)>, K: Into<String>>(fields: I) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

/// A string value.
pub fn s(text: impl Into<String>) -> Value {
    Value::String(text.into())
}

/// Canonical JSON text of `v` (object keys sorted), for hashing.
pub fn canonical(v: &Json) -> String {
    match v {
        Json::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Json::String(k.clone()), canonical(&m[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Json::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// The SHA-256 of `bytes` as `sha256:<hex>`.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in d {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_integers() {
        let v = obj([
            ("a", Value::Array(vec![s("x"), Value::Bool(true)])),
            ("b", Value::Number(2.0)),
        ]);
        let j = to_json(&v);
        assert_eq!(j.to_string(), r#"{"a":["x",true],"b":2}"#);
        assert_eq!(from_json(&j), v);
        assert_eq!(canonical(&j), r#"{"a":["x",true],"b":2}"#);
        assert_eq!(at_path(&v, "a.0"), Some(&s("x")));
    }
}
