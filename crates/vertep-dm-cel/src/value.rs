//! CEL runtime values, their equality and ordering, and their mapping to the
//! datamodel-independent [`Value`].

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Ordering;

use vertep_core::datamodel::Value;

use crate::decimal::Decimal;
use crate::time::{Duration, Timestamp};

/// A CEL map. Keys are kept in [`Key`] order, so iteration never depends on insertion
/// history.
// @tg: implements:: [[openspec:cel-datamodel#Deterministic Evaluation]]
pub type CelMap = BTreeMap<Key, CelValue>;

/// A CEL runtime value. Aggregates are shared and immutable; updating one copies the path to
/// the change.
#[derive(Clone, Debug)]
pub enum CelValue {
    /// `null`.
    Null,
    /// `bool`.
    Bool(bool),
    /// `int`, a signed 64-bit integer.
    Int(i64),
    /// `uint`, an unsigned 64-bit integer.
    Uint(u64),
    /// `double`, an IEEE 754 binary64 number.
    Double(f64),
    /// `decimal`, the exact decimal extension type.
    Decimal(Decimal),
    /// `string`.
    String(Arc<str>),
    /// `bytes`.
    Bytes(Arc<[u8]>),
    /// `list`.
    List(Arc<Vec<CelValue>>),
    /// `map`.
    Map(Arc<CelMap>),
    /// `google.protobuf.Timestamp`.
    Timestamp(Timestamp),
    /// `google.protobuf.Duration`.
    Duration(Duration),
    /// A type value such as `int`, as returned by `type(x)`.
    Type(Arc<str>),
}

/// A CEL map key: `bool`, `int`, `uint` or `string`. Keys order as all `false`/`true`, then
/// ints ascending, then uints ascending, then strings by code point.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A boolean key.
    Bool(bool),
    /// An int key.
    Int(i64),
    /// A uint key.
    Uint(u64),
    /// A string key.
    String(Arc<str>),
}

impl Key {
    /// The key as a value.
    pub fn to_value(&self) -> CelValue {
        match self {
            Key::Bool(b) => CelValue::Bool(*b),
            Key::Int(i) => CelValue::Int(*i),
            Key::Uint(u) => CelValue::Uint(*u),
            Key::String(s) => CelValue::String(s.clone()),
        }
    }

    /// The key for a value, when the value's type can be a key.
    pub fn from_value(v: &CelValue) -> Option<Key> {
        match v {
            CelValue::Bool(b) => Some(Key::Bool(*b)),
            CelValue::Int(i) => Some(Key::Int(*i)),
            CelValue::Uint(u) => Some(Key::Uint(*u)),
            CelValue::String(s) => Some(Key::String(s.clone())),
            _ => None,
        }
    }

    /// A string key.
    pub fn str(s: &str) -> Key {
        Key::String(Arc::from(s))
    }

    fn text(&self) -> String {
        match self {
            Key::Bool(b) => b.to_string(),
            Key::Int(i) => i.to_string(),
            Key::Uint(u) => u.to_string(),
            Key::String(s) => s.to_string(),
        }
    }
}

/// Look up `key` in `map`, treating numerically equal int and uint keys as the same key.
pub fn map_get<'m>(map: &'m CelMap, key: &Key) -> Option<&'m CelValue> {
    map.get(key).or_else(|| match key {
        Key::Int(i) => u64::try_from(*i).ok().and_then(|u| map.get(&Key::Uint(u))),
        Key::Uint(u) => i64::try_from(*u).ok().and_then(|i| map.get(&Key::Int(i))),
        _ => None,
    })
}

impl CelValue {
    /// A string value.
    pub fn str(s: &str) -> CelValue {
        CelValue::String(Arc::from(s))
    }

    /// The CEL name of the value's type.
    pub fn type_name(&self) -> &'static str {
        match self {
            CelValue::Null => "null_type",
            CelValue::Bool(_) => "bool",
            CelValue::Int(_) => "int",
            CelValue::Uint(_) => "uint",
            CelValue::Double(_) => "double",
            CelValue::Decimal(_) => "decimal",
            CelValue::String(_) => "string",
            CelValue::Bytes(_) => "bytes",
            CelValue::List(_) => "list",
            CelValue::Map(_) => "map",
            CelValue::Timestamp(_) => "google.protobuf.Timestamp",
            CelValue::Duration(_) => "google.protobuf.Duration",
            CelValue::Type(_) => "type",
        }
    }

    /// CEL equality: numbers compare by value across `int`, `uint`, `double` and `decimal`,
    /// aggregates compare element-wise, values of different types are unequal.
    pub fn cel_eq(&self, o: &CelValue) -> bool {
        use CelValue as V;
        match (self, o) {
            (V::Null, V::Null) => true,
            (V::Bool(a), V::Bool(b)) => a == b,
            (V::String(a), V::String(b)) => a == b,
            (V::Bytes(a), V::Bytes(b)) => a == b,
            (V::Timestamp(a), V::Timestamp(b)) => a == b,
            (V::Duration(a), V::Duration(b)) => a == b,
            (V::Type(a), V::Type(b)) => a == b,
            (V::List(a), V::List(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.cel_eq(y))
            }
            (V::Map(a), V::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| map_get(b, k).is_some_and(|w| v.cel_eq(w)))
            }
            _ if self.is_number() && o.is_number() => num_cmp(self, o) == Some(Ordering::Equal),
            _ => false,
        }
    }

    /// Whether the value is an `int`, `uint`, `double` or `decimal`.
    pub fn is_number(&self) -> bool {
        matches!(
            self,
            CelValue::Int(_) | CelValue::Uint(_) | CelValue::Double(_) | CelValue::Decimal(_)
        )
    }

    /// The value's size (nodes plus string and bytes lengths) and nesting depth, counted
    /// without recursion and only until the size exceeds `cap`
    /// ([[openspec:cel-datamodel#Guaranteed Termination]]).
    pub fn measure(&self, cap: usize) -> (usize, usize) {
        let mut total = 0usize;
        let mut depth = 0usize;
        let mut stack: Vec<(&CelValue, usize)> = Vec::from([(self, 1)]);
        while let Some((v, d)) = stack.pop() {
            total += 1;
            depth = depth.max(d);
            match v {
                CelValue::String(s) => total += s.len(),
                CelValue::Bytes(b) => total += b.len(),
                CelValue::List(l) => stack.extend(l.iter().map(|x| (x, d + 1))),
                CelValue::Map(m) => {
                    for (k, x) in m.iter() {
                        total += 1;
                        if let Key::String(s) = k {
                            total += s.len();
                        }
                        stack.push((x, d + 1));
                    }
                }
                _ => {}
            }
            if total > cap {
                break;
            }
        }
        (total, depth)
    }

    /// [`CelValue::measure`]'s size.
    pub fn deep_size(&self, cap: usize) -> usize {
        self.measure(cap).0
    }
}

/// Compare two numbers of any numeric type by mathematical value; `None` when either is NaN
/// or the values are not both numbers.
pub fn num_cmp(a: &CelValue, b: &CelValue) -> Option<Ordering> {
    use CelValue as V;
    match (a, b) {
        (V::Int(x), V::Int(y)) => Some(x.cmp(y)),
        (V::Uint(x), V::Uint(y)) => Some(x.cmp(y)),
        (V::Double(x), V::Double(y)) => x.partial_cmp(y),
        (V::Decimal(x), V::Decimal(y)) => Some(x.cmp(y)),
        (V::Int(x), V::Uint(y)) => Some(if *x < 0 {
            Ordering::Less
        } else {
            (*x as u64).cmp(y)
        }),
        (V::Uint(_), V::Int(_)) => num_cmp(b, a).map(Ordering::reverse),
        (V::Int(x), V::Double(y)) => int_double_cmp(i128::from(*x), *y),
        (V::Uint(x), V::Double(y)) => int_double_cmp(i128::from(*x), *y),
        (V::Double(_), V::Int(_) | V::Uint(_)) => num_cmp(b, a).map(Ordering::reverse),
        (V::Int(x), V::Decimal(y)) => Some(Decimal::from_i64(*x).cmp(y)),
        (V::Uint(x), V::Decimal(y)) => Some(Decimal::from_u64(*x).cmp(y)),
        (V::Decimal(_), V::Int(_) | V::Uint(_)) => num_cmp(b, a).map(Ordering::reverse),
        (V::Double(x), V::Decimal(y)) => {
            if x.is_nan() {
                return None;
            }
            match Decimal::from_f64(*x) {
                Ok(d) => Some(d.cmp(y)),
                Err(_) => x.partial_cmp(&y.to_f64()),
            }
        }
        (V::Decimal(_), V::Double(_)) => num_cmp(b, a).map(Ordering::reverse),
        _ => None,
    }
}

fn int_double_cmp(i: i128, d: f64) -> Option<Ordering> {
    if d.is_nan() {
        return None;
    }
    // Every i64 and u64 lies strictly inside ±2^64.
    if d >= 18_446_744_073_709_551_616.0 {
        return Some(Ordering::Less);
    }
    if d <= -18_446_744_073_709_551_616.0 {
        return Some(Ordering::Greater);
    }
    let t = trunc(d);
    let ti = t as i128;
    match i.cmp(&ti) {
        Ordering::Equal => {
            let frac = d - t;
            Some(if frac > 0.0 {
                Ordering::Less
            } else if frac < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        o => Some(o),
    }
}

/// `d` rounded toward zero (`f64::trunc` is not in `core`).
pub fn trunc(d: f64) -> f64 {
    // Every finite double of magnitude 2^52 or more is already integral.
    if !d.is_finite() || d.abs() >= 4_503_599_627_370_496.0 {
        d
    } else {
        (d as i64) as f64
    }
}

/// Whether `d` is finite and has no fractional part.
pub fn is_integral(d: f64) -> bool {
    d.is_finite() && trunc(d) == d
}

const MAX_SAFE_INT: i64 = 9_007_199_254_740_992;

/// Map a datamodel-independent value into CEL: `undefined` and `null` become `null`,
/// integral numbers within ±2^53 become `int`, other numbers `double`, arrays lists and
/// objects maps with string keys (the last duplicate key wins).
pub fn from_value(v: &Value) -> CelValue {
    match v {
        Value::Undefined | Value::Null => CelValue::Null,
        Value::Bool(b) => CelValue::Bool(*b),
        Value::Number(n) => {
            if is_integral(*n) && n.abs() <= MAX_SAFE_INT as f64 {
                CelValue::Int(*n as i64)
            } else {
                CelValue::Double(*n)
            }
        }
        Value::String(s) => CelValue::str(s),
        Value::Array(a) => CelValue::List(Arc::new(a.iter().map(from_value).collect())),
        Value::Object(o) => {
            let mut m = CelMap::new();
            for (k, x) in o {
                m.insert(Key::str(k), from_value(x));
            }
            CelValue::Map(Arc::new(m))
        }
    }
}

/// Map a CEL value to the datamodel-independent value, following the canonical value
/// encoding: `int` and `uint` become numbers when exactly representable and decimal strings
/// otherwise, `decimal` a canonical decimal string, `bytes` base64, timestamps RFC 3339,
/// durations seconds strings such as `1.5s`, maps objects with stringified keys in key order.
pub fn to_value(v: &CelValue) -> Value {
    match v {
        CelValue::Null => Value::Null,
        CelValue::Bool(b) => Value::Bool(*b),
        CelValue::Int(i) => {
            if i.unsigned_abs() <= MAX_SAFE_INT as u64 {
                Value::Number(*i as f64)
            } else {
                Value::String(i.to_string())
            }
        }
        CelValue::Uint(u) => {
            if *u <= MAX_SAFE_INT as u64 {
                Value::Number(*u as f64)
            } else {
                Value::String(u.to_string())
            }
        }
        CelValue::Double(d) => Value::Number(*d),
        CelValue::Decimal(d) => Value::String(d.to_string()),
        CelValue::String(s) => Value::String(s.to_string()),
        CelValue::Bytes(b) => Value::String(base64(b)),
        CelValue::List(l) => Value::Array(l.iter().map(to_value).collect()),
        CelValue::Map(m) => Value::Object(m.iter().map(|(k, x)| (k.text(), to_value(x))).collect()),
        CelValue::Timestamp(t) => Value::String(t.format()),
        CelValue::Duration(d) => Value::String(d.format()),
        CelValue::Type(t) => Value::String(t.to_string()),
    }
}

/// Standard base64 with padding.
pub fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_across_types() {
        let d = |s: &str| CelValue::Decimal(Decimal::parse(s).unwrap());
        assert!(CelValue::Int(1).cel_eq(&CelValue::Double(1.0)));
        assert!(CelValue::Int(1).cel_eq(&CelValue::Uint(1)));
        assert!(CelValue::Int(2).cel_eq(&d("2.0")));
        assert!(!CelValue::Double(f64::NAN).cel_eq(&CelValue::Double(f64::NAN)));
        assert_eq!(
            num_cmp(&CelValue::Int(-1), &CelValue::Uint(0)),
            Some(Ordering::Less)
        );
        assert_eq!(
            num_cmp(&CelValue::Int(3), &CelValue::Double(2.5)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            num_cmp(&CelValue::Double(0.1), &d("0.1")),
            Some(Ordering::Equal)
        );
        assert!(!CelValue::Int(1).cel_eq(&CelValue::str("1")));
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"a"), "YQ==");
    }
}
