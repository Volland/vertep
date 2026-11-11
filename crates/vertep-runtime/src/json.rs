//! The JSON text encoding of datamodel values in journal entries, snapshots and dead letters.
//!
//! Plain JSON for everything JSON can say, with object keys in insertion order; `undefined` and
//! non-finite numbers, which JSON cannot say, are tagged objects `{"$vt": "undefined"}`,
//! `{"$vt": "NaN"}`, `{"$vt": "Infinity"}` and `{"$vt": "-Infinity"}`. Decoding is the exact
//! inverse, so a payload read back from a journal equals the payload that was recorded.

use core::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use vertep_core::datamodel::Value;

const TAG: &str = "$vt";

/// A [`Value`] that (de)serializes as the Vertep JSON encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct JsonValue(pub Value);

impl Serialize for JsonValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ser(&self.0, s)
    }
}

fn tagged<S: Serializer>(tag: &str, s: S) -> Result<S::Ok, S::Error> {
    let mut m = s.serialize_map(Some(1))?;
    m.serialize_entry(TAG, tag)?;
    m.end()
}

fn ser<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Value::Undefined => tagged("undefined", s),
        Value::Null => s.serialize_unit(),
        Value::Bool(b) => s.serialize_bool(*b),
        Value::Number(n) if n.is_nan() => tagged("NaN", s),
        Value::Number(n) if n.is_infinite() && *n > 0.0 => tagged("Infinity", s),
        Value::Number(n) if n.is_infinite() => tagged("-Infinity", s),
        Value::Number(n) => s.serialize_f64(*n),
        Value::String(t) => s.serialize_str(t),
        Value::Array(items) => {
            let mut q = s.serialize_seq(Some(items.len()))?;
            for i in items {
                q.serialize_element(&JsonValue(i.clone()))?;
            }
            q.end()
        }
        Value::Object(entries) => {
            let mut m = s.serialize_map(Some(entries.len()))?;
            for (k, v) in entries {
                m.serialize_entry(k, &JsonValue(v.clone()))?;
            }
            m.end()
        }
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor).map(JsonValue)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }
    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Value, E> {
        Ok(Value::Number(n as f64))
    }
    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Value, E> {
        Ok(Value::Number(n as f64))
    }
    fn visit_f64<E: de::Error>(self, n: f64) -> Result<Value, E> {
        Ok(Value::Number(n))
    }
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Value, E> {
        Ok(Value::String(s.to_owned()))
    }
    fn visit_string<E: de::Error>(self, s: String) -> Result<Value, E> {
        Ok(Value::String(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(JsonValue(v)) = seq.next_element()? {
            items.push(v);
        }
        Ok(Value::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut entries: Vec<(String, Value)> = Vec::new();
        while let Some((k, JsonValue(v))) = map.next_entry::<String, JsonValue>()? {
            entries.push((k, v));
        }
        if let [(k, Value::String(tag))] = entries.as_slice()
            && k == TAG
        {
            return Ok(match tag.as_str() {
                "undefined" => Value::Undefined,
                "NaN" => Value::Number(f64::NAN),
                "Infinity" => Value::Number(f64::INFINITY),
                "-Infinity" => Value::Number(f64::NEG_INFINITY),
                _ => Value::Object(entries),
            });
        }
        Ok(Value::Object(entries))
    }
}

/// The JSON text of `v`.
pub fn to_text(v: &Value) -> String {
    serde_json::to_string(&JsonValue(v.clone())).unwrap_or_else(|_| "null".into())
}

/// The value encoded by `text`.
///
/// # Errors
///
/// The JSON parse error.
pub fn from_text(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<JsonValue>(text).map(|j| j.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_undefined() {
        let v = Value::Object(vec![
            ("z".into(), Value::Number(1.5)),
            ("a".into(), Value::Undefined),
            (
                "list".into(),
                Value::Array(vec![
                    Value::Null,
                    Value::Bool(true),
                    Value::String("x".into()),
                ]),
            ),
        ]);
        let t = to_text(&v);
        assert_eq!(
            t,
            r#"{"z":1.5,"a":{"$vt":"undefined"},"list":[null,true,"x"]}"#
        );
        assert_eq!(from_text(&t).unwrap(), v);
        assert_eq!(
            from_text(&to_text(&Value::Undefined)).unwrap(),
            Value::Undefined
        );
    }
}
