//! Conversions between Rust values and the datamodel-independent [`Value`], following the
//! canonical value encoding of the type language (`vertep_core::types::check`).
//!
//! The `statechart!` and `include_chart!` macros implement [`FromValue`] and [`ToValue`] for every
//! generated record, union, context and payload type, so typed Rust values cross the
//! `Datamodel` boundary as canonical values and back.

use core::fmt;

use vertep_core::datamodel::{EvalError, Event, Value};
use vertep_core::types::prim::{self, IriError};

use crate::decimal::Decimal;

/// Reads a Rust value from its canonical [`Value`] encoding.
pub trait FromValue: Sized {
    /// The value, or an error naming what does not fit.
    fn from_value(value: &Value) -> Result<Self, EvalError>;

    /// The value of an absent record field or union payload. Only optional types and `value`
    /// accept absence.
    fn absent() -> Result<Self, EvalError> {
        Err(EvalError::new("a required value is missing"))
    }
}

/// Writes a Rust value as its canonical [`Value`] encoding.
pub trait ToValue {
    /// The canonical encoding.
    fn to_value(&self) -> Value;

    /// Whether the value is absent when used as a record field (an empty optional), so the
    /// field is omitted from the encoding.
    fn is_absent(&self) -> bool {
        false
    }
}

fn mismatch(expected: &str, found: &Value) -> EvalError {
    EvalError::new(format!(
        "expected {expected}, found {}",
        vertep_core::types::check::kind_of(found)
    ))
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::Bool(b) => Ok(*b),
            v => Err(mismatch("bool", v)),
        }
    }
}

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::Number(n) => prim::f64_to_i64(*n)
                .ok_or_else(|| EvalError::new(format!("{n} is not a 64-bit integer"))),
            v => Err(mismatch("int", v)),
        }
    }
}

impl ToValue for i64 {
    fn to_value(&self) -> Value {
        Value::Number(*self as f64)
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::String(s) => Ok(s.clone()),
            v => Err(mismatch("string", v)),
        }
    }
}

impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::String(self.clone())
    }
}

impl ToValue for &str {
    fn to_value(&self) -> Value {
        Value::String((*self).to_string())
    }
}

impl FromValue for Decimal {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::String(s) => s.parse().map_err(|e| EvalError::new(format!("{e}"))),
            Value::Number(n) => {
                Decimal::from_f64(*n).ok_or_else(|| EvalError::new(format!("{n} is not a decimal")))
            }
            v => Err(mismatch("decimal", v)),
        }
    }
}

impl ToValue for Decimal {
    fn to_value(&self) -> Value {
        Value::String(self.to_string())
    }
}

impl FromValue for Value {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        Ok(value.clone())
    }

    fn absent() -> Result<Self, EvalError> {
        Ok(Value::Undefined)
    }
}

impl ToValue for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }

    /// A `value` field holding `null` or nothing is omitted, as the canonical encoding omits
    /// absent `value` and optional fields.
    fn is_absent(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    T::from_value(v).map_err(|e| EvalError::new(format!("[{i}]: {}", e.message)))
                })
                .collect(),
            v => Err(mismatch("list", v)),
        }
    }
}

impl<T: ToValue> ToValue for Vec<T> {
    fn to_value(&self) -> Value {
        Value::Array(self.iter().map(ToValue::to_value).collect())
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::Undefined | Value::Null => Ok(None),
            v => T::from_value(v).map(Some),
        }
    }

    fn absent() -> Result<Self, EvalError> {
        Ok(None)
    }
}

impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null,
        }
    }

    fn is_absent(&self) -> bool {
        self.is_none()
    }
}

impl<T: FromValue> FromValue for Box<T> {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        T::from_value(value).map(Box::new)
    }

    fn absent() -> Result<Self, EvalError> {
        T::absent().map(Box::new)
    }
}

impl<T: ToValue> ToValue for Box<T> {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }

    fn is_absent(&self) -> bool {
        (**self).is_absent()
    }
}

/// An RFC 3339 instant, the `datetime` primitive. It keeps the canonical UTC text
/// (`2026-11-11T08:00:00Z`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Datetime(String);

impl Datetime {
    /// Parse an RFC 3339 `date-time` with an offset and normalize it to UTC.
    pub fn parse(text: &str) -> Result<Datetime, EvalError> {
        let dt = prim::parse_datetime(text)
            .map_err(|e| EvalError::new(format!("`{text}` is not an RFC 3339 datetime: {e:?}")))?;
        dt.to_utc_string()
            .map(Datetime)
            .ok_or_else(|| EvalError::new(format!("`{text}` leaves the years 0000 to 9999")))
    }

    /// The canonical UTC text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Datetime {
    /// The Unix epoch.
    fn default() -> Self {
        Datetime("1970-01-01T00:00:00Z".to_string())
    }
}

impl fmt::Display for Datetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromValue for Datetime {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::String(s) => Datetime::parse(s),
            v => Err(mismatch("datetime", v)),
        }
    }
}

impl ToValue for Datetime {
    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

/// An absolute IRI, the `iri` primitive.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Iri(String);

impl Iri {
    /// Check that `text` is an IRI with a scheme.
    pub fn parse(text: &str) -> Result<Iri, EvalError> {
        match prim::check_iri(text) {
            Ok(()) => Ok(Iri(text.to_string())),
            Err(IriError::Relative) => Err(EvalError::new(format!(
                "`{text}` is a relative reference, not an IRI"
            ))),
            Err(IriError::InvalidCharacter) => Err(EvalError::new(format!(
                "`{text}` contains a character IRIs exclude"
            ))),
        }
    }

    /// The IRI text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Iri {
    /// `about:blank`.
    fn default() -> Self {
        Iri("about:blank".to_string())
    }
}

impl fmt::Display for Iri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromValue for Iri {
    fn from_value(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::String(s) => Iri::parse(s),
            v => Err(mismatch("iri", v)),
        }
    }
}

impl ToValue for Iri {
    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

/// The type a guard or action receives as its event argument: `()` for eventless transitions and
/// events without payload, the payload type of the one declared event a transition names, or the
/// whole [`Event`] when a transition matches several events or undeclared ones.
pub trait FromEvent: Sized {
    /// Read the argument from the current event.
    fn from_event(event: &Event) -> Result<Self, EvalError>;
}

impl FromEvent for () {
    fn from_event(_event: &Event) -> Result<Self, EvalError> {
        Ok(())
    }
}

impl FromEvent for Event {
    fn from_event(event: &Event) -> Result<Self, EvalError> {
        Ok(event.clone())
    }
}

impl<T: FromValue> FromEvent for T {
    fn from_event(event: &Event) -> Result<Self, EvalError> {
        T::from_value(&event.data).map_err(|e| {
            EvalError::new(format!(
                "the payload of event `{}` does not fit: {}",
                event.name, e.message
            ))
        })
    }
}

// ---- helpers used by generated code ------------------------------------------------------

/// Read field `name` of a record encoding. Used by generated [`FromValue`] impls.
pub fn field<T: FromValue>(entries: &[(String, Value)], name: &str) -> Result<T, EvalError> {
    match entries
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
        .filter(|v| !matches!(v, Value::Undefined))
    {
        Some(v) => T::from_value(v).map_err(|e| EvalError::new(format!(".{name}: {}", e.message))),
        None => T::absent().map_err(|_| EvalError::new(format!("missing required field `{name}`"))),
    }
}

/// The entries of a record encoding, rejecting fields outside `known`.
pub fn record<'v>(value: &'v Value, known: &[&str]) -> Result<&'v [(String, Value)], EvalError> {
    match value {
        Value::Object(entries) => {
            if let Some((k, _)) = entries.iter().find(|(k, _)| !known.contains(&k.as_str())) {
                return Err(EvalError::new(format!("unknown field `{k}`")));
            }
            Ok(entries)
        }
        v => Err(mismatch("record", v)),
    }
}

/// Build a record encoding from `(name, value)` pairs: sorted by name, absent optionals omitted.
pub fn make_record(fields: Vec<(&str, &dyn ToValue)>) -> Value {
    let mut out: Vec<(String, Value)> = fields
        .into_iter()
        .filter(|(_, v)| !v.is_absent())
        .map(|(k, v)| (k.to_string(), v.to_value()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Object(out)
}

/// The tag and payload of a tagged union encoding `{"tag": ..., "value": ...}`.
pub fn union_parts(value: &Value) -> Result<(&str, Option<&Value>), EvalError> {
    let entries = record(value, &["tag", "value"])?;
    let tag = match entries.iter().find(|(k, _)| k == "tag") {
        Some((_, Value::String(t))) => t.as_str(),
        _ => return Err(EvalError::new("tagged union value has no `tag`")),
    };
    let payload = entries
        .iter()
        .find(|(k, _)| k == "value")
        .map(|(_, v)| v)
        .filter(|v| !matches!(v, Value::Undefined));
    Ok((tag, payload))
}

/// Read a union variant's payload.
pub fn payload<T: FromValue>(tag: &str, payload: Option<&Value>) -> Result<T, EvalError> {
    match payload {
        Some(v) => T::from_value(v).map_err(|e| EvalError::new(format!("<{tag}>: {}", e.message))),
        None => T::absent()
            .map_err(|_| EvalError::new(format!("variant `{tag}` requires a payload (`value`)"))),
    }
}

/// Build a union encoding.
pub fn make_union(tag: &str, payload: Option<Value>) -> Value {
    let mut out = vec![("tag".to_string(), Value::String(tag.to_string()))];
    if let Some(p) = payload {
        out.push(("value".to_string(), p));
    }
    Value::Object(out)
}

/// The error for an unknown union tag.
pub fn unknown_tag(tag: &str) -> EvalError {
    EvalError::new(format!("unknown variant `{tag}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_round_trip() {
        assert_eq!(i64::from_value(&7i64.to_value()), Ok(7));
        assert!(i64::from_value(&Value::Number(1.5)).is_err());
        let d: Decimal = "12.50".parse().unwrap();
        assert_eq!(d.to_value(), Value::String("12.5".into()));
        assert_eq!(
            Decimal::from_value(&Value::Number(0.25)),
            Ok("0.25".parse().unwrap())
        );
        assert_eq!(
            Datetime::parse("2026-11-11T10:00:00+02:00")
                .unwrap()
                .as_str(),
            "2026-11-11T08:00:00Z"
        );
        assert!(Iri::parse("relative/path").is_err());
        assert_eq!(Option::<i64>::from_value(&Value::Null), Ok(None));
        assert_eq!(
            Vec::<bool>::from_value(&Value::Array(vec![Value::Bool(true)])),
            Ok(vec![true])
        );
    }

    #[test]
    fn records_and_unions() {
        let v = make_record(vec![("b", &1i64), ("a", &None::<i64>)]);
        assert_eq!(v, Value::Object(vec![("b".into(), Value::Number(1.0))]));
        let entries = record(&v, &["a", "b"]).unwrap();
        assert_eq!(field::<Option<i64>>(entries, "a"), Ok(None));
        assert!(field::<i64>(entries, "c").is_err());
        assert!(record(&v, &["a"]).is_err());
        let u = make_union("Cash", None);
        assert_eq!(union_parts(&u).unwrap(), ("Cash", None));
    }
}
