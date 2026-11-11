//! Typed coercion at the datamodel boundary, assignment locations, and the snapshot encoding.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use vertep_core::datamodel::Value;

use crate::decimal::Decimal;
use crate::error::{CelError, ErrorKind};
use crate::syntax::{Expr, ExprKind, parse};
use crate::time::{Duration, Timestamp};
use crate::types::{CelType, TypeEnv};
use crate::value::{CelMap, CelValue, Key};

/// Names that are not data items: the CEL bindings and the SCXML system variables.
pub const RESERVED_NAMES: &[&str] = &[
    "ctx",
    "ev",
    "session",
    "In",
    "_event",
    "_sessionid",
    "_name",
    "_ioprocessors",
];

/// Convert `v` to a value of type `ty`, applying the exact widenings of the canonical value
/// encoding (`int` to `decimal` and `double`, decimal and RFC 3339 strings to `decimal` and
/// timestamps) and checking records, unions, lists and maps element by element.
pub fn coerce(v: CelValue, ty: &CelType, path: &str) -> Result<CelValue, CelError> {
    use CelType as T;
    use CelValue as V;
    let mismatch = |v: &V| {
        CelError::new(
            ErrorKind::Conversion,
            format!("{path}: expected {ty}, found {}", v.type_name()),
        )
    };
    Ok(match (ty, v) {
        (T::Dyn, v) => v,
        (T::Optional(_), V::Null) => V::Null,
        (T::Optional(t), v) => coerce(v, t, path)?,
        (T::Null, V::Null) => V::Null,
        (T::Bool, v @ V::Bool(_))
        | (T::Int, v @ V::Int(_))
        | (T::Uint, v @ V::Uint(_))
        | (T::Double, v @ V::Double(_))
        | (T::Decimal, v @ V::Decimal(_))
        | (T::String, v @ V::String(_))
        | (T::Bytes, v @ V::Bytes(_))
        | (T::Timestamp, v @ V::Timestamp(_))
        | (T::Duration, v @ V::Duration(_))
        | (T::Type, v @ V::Type(_)) => v,
        (T::Uint, V::Int(i)) if i >= 0 => V::Uint(i as u64),
        (T::Double, V::Int(i)) if i.unsigned_abs() <= 1 << 53 => V::Double(i as f64),
        (T::Decimal, V::Int(i)) => V::Decimal(Decimal::from_i64(i)),
        (T::Decimal, V::Uint(u)) => V::Decimal(Decimal::from_u64(u)),
        (T::Decimal, v @ V::Double(d)) => {
            V::Decimal(Decimal::from_f64(d).map_err(|_| mismatch(&v))?)
        }
        (T::Decimal, v @ V::String(_)) => {
            let V::String(s) = &v else {
                return Err(mismatch(&v));
            };
            V::Decimal(Decimal::parse(s).map_err(|_| mismatch(&v))?)
        }
        (T::Timestamp, v @ V::String(_)) => {
            let V::String(s) = &v else {
                return Err(mismatch(&v));
            };
            V::Timestamp(Timestamp::parse(s).ok_or_else(|| mismatch(&v))?)
        }
        (T::Duration, v @ V::String(_)) => {
            let V::String(s) = &v else {
                return Err(mismatch(&v));
            };
            V::Duration(Duration::parse(s).ok_or_else(|| mismatch(&v))?)
        }
        (T::List(el), V::List(l)) => {
            let mut out = Vec::with_capacity(l.len());
            for (i, x) in l.iter().enumerate() {
                out.push(coerce(x.clone(), el, &format!("{path}[{i}]"))?);
            }
            V::List(Arc::new(out))
        }
        (T::Map(_, vt), V::Map(m)) => {
            let mut out = CelMap::new();
            for (k, x) in m.iter() {
                out.insert(k.clone(), coerce(x.clone(), vt, &format!("{path}[{k:?}]"))?);
            }
            V::Map(Arc::new(out))
        }
        (T::Record(r), V::Map(m)) => {
            let mut out = CelMap::new();
            for (k, _) in m.iter() {
                let known = matches!(k, Key::String(s) if r.fields.contains_key(&**s));
                if !known {
                    return Err(CelError::new(
                        ErrorKind::Conversion,
                        format!("{path}: unknown field {} for {ty}", key_text(k)),
                    ));
                }
            }
            for (f, ft) in &r.fields {
                let key = Key::str(f);
                match m.get(&key) {
                    Some(x) => {
                        out.insert(key, coerce(x.clone(), ft, &format!("{path}.{f}"))?);
                    }
                    None if matches!(ft, T::Optional(_) | T::Dyn) => {
                        out.insert(key, V::Null);
                    }
                    None => {
                        return Err(CelError::new(
                            ErrorKind::Conversion,
                            format!("{path}: missing field `{f}` of {ty}"),
                        ));
                    }
                }
            }
            V::Map(Arc::new(out))
        }
        (T::Union(u), V::Map(m)) => {
            let tag = match m.get(&Key::str("tag")) {
                Some(V::String(t)) => t.clone(),
                _ => {
                    return Err(CelError::new(
                        ErrorKind::Conversion,
                        format!("{path}: a {ty} value needs a string `tag`"),
                    ));
                }
            };
            let Some(payload) = u.variants.get(&*tag) else {
                return Err(CelError::new(
                    ErrorKind::Conversion,
                    format!("{path}: `{tag}` is not a variant of {ty}"),
                ));
            };
            let mut out = CelMap::new();
            out.insert(Key::str("tag"), V::String(tag.clone()));
            match (payload, m.get(&Key::str("value"))) {
                (Some(pt), Some(x)) => {
                    out.insert(
                        Key::str("value"),
                        coerce(x.clone(), pt, &format!("{path}.value"))?,
                    );
                }
                (Some(T::Optional(_)), None) => {}
                (Some(_), None) => {
                    return Err(CelError::new(
                        ErrorKind::Conversion,
                        format!("{path}: variant `{tag}` needs a `value`"),
                    ));
                }
                (None, None | Some(V::Null)) => {}
                (None, Some(_)) => {
                    return Err(CelError::new(
                        ErrorKind::Conversion,
                        format!("{path}: variant `{tag}` has no payload"),
                    ));
                }
            }
            if m.keys()
                .any(|k| *k != Key::str("tag") && *k != Key::str("value"))
            {
                return Err(CelError::new(
                    ErrorKind::Conversion,
                    format!("{path}: a union value has only `tag` and `value`"),
                ));
            }
            V::Map(Arc::new(out))
        }
        (_, v) => return Err(mismatch(&v)),
    })
}

fn key_text(k: &Key) -> String {
    match k {
        Key::String(s) => format!("`{s}`"),
        other => format!("{other:?}"),
    }
}

/// One step of an assignment path below the context field.
#[derive(Clone, Debug)]
pub enum Seg {
    /// `.field`.
    Field(Arc<str>),
    /// `[index]`, evaluated when assigning.
    Index(Expr),
}

/// An assignable location: a context field (`ctx.name` or the bare `name`) and a path below
/// it.
#[derive(Clone, Debug)]
pub struct Location {
    /// The context field.
    pub root: Arc<str>,
    /// The path below the field.
    pub path: Vec<Seg>,
}

fn not_location(src: &str) -> CelError {
    CelError::new(
        ErrorKind::Location,
        format!(
            "`{src}` is not an assignable location: use `ctx.<field>` with `.field` or `[index]` steps"
        ),
    )
}

/// Parse a location such as `ctx.order.total`, `ctx["a b"]`, `ctx.items[0]` or `count`.
pub fn parse_location(src: &str, max_depth: u32) -> Result<Location, CelError> {
    let e = parse(src, max_depth).map_err(|e| CelError::new(ErrorKind::Syntax, e.message))?;
    let mut path = Vec::new();
    let mut cur = e;
    loop {
        match cur.kind {
            ExprKind::Ident(name) => {
                if RESERVED_NAMES.contains(&&*name) {
                    return Err(CelError::new(
                        ErrorKind::Location,
                        format!("cannot assign to `{name}`: it is a read-only system variable"),
                    ));
                }
                path.reverse();
                return Ok(Location { root: name, path });
            }
            ExprKind::Select {
                operand,
                field,
                test: false,
            } => {
                if let ExprKind::Ident(n) = &operand.kind
                    && &**n == "ctx"
                {
                    path.reverse();
                    return Ok(Location { root: field, path });
                }
                if let ExprKind::Ident(n) = &operand.kind
                    && RESERVED_NAMES.contains(&&**n)
                {
                    return Err(CelError::new(
                        ErrorKind::Location,
                        format!("cannot assign to `{src}`: `{n}` is read-only"),
                    ));
                }
                path.push(Seg::Field(field));
                cur = *operand;
            }
            ExprKind::Index { operand, index } => {
                if let ExprKind::Ident(n) = &operand.kind
                    && &**n == "ctx"
                {
                    let ExprKind::Lit(CelValue::String(root)) = &index.kind else {
                        return Err(not_location(src));
                    };
                    let root = root.clone();
                    path.reverse();
                    return Ok(Location { root, path });
                }
                path.push(Seg::Index(*index));
                cur = *operand;
            }
            _ => return Err(not_location(src)),
        }
    }
}

/// The static type of a location: the context field's declared type, refined along the path.
pub fn location_type(env: &TypeEnv, loc: &Location) -> Result<CelType, String> {
    let mut t = env.context_field(&loc.root)?;
    for s in &loc.path {
        t = match s {
            Seg::Field(f) => t.field(f)?,
            Seg::Index(e) => match t.strip_optional() {
                CelType::Dyn => CelType::Dyn,
                CelType::List(el) => (**el).clone(),
                CelType::Map(_, v) => (**v).clone(),
                CelType::Record(_) | CelType::Union(_) => match &e.kind {
                    ExprKind::Lit(CelValue::String(f)) => t.field(f)?,
                    _ => CelType::Dyn,
                },
                other => return Err(format!("type {other} cannot be indexed")),
            },
        };
    }
    Ok(t)
}

/// Store `value` at `keys` below `target`, copying shared aggregates along the path. Map
/// keys may be created at the last step; list indices must exist.
pub fn put(target: &mut CelValue, keys: &[CelValue], value: CelValue) -> Result<(), CelError> {
    let Some((k, rest)) = keys.split_first() else {
        *target = value;
        return Ok(());
    };
    match target {
        CelValue::Map(m) => {
            let key = Key::from_value(k).ok_or_else(|| {
                CelError::new(
                    ErrorKind::NoMatchingOverload,
                    format!("unsupported map key type {}", k.type_name()),
                )
            })?;
            let m = Arc::make_mut(m);
            if rest.is_empty() {
                m.insert(key, value);
                return Ok(());
            }
            let child = m.get_mut(&key).ok_or_else(|| {
                CelError::new(
                    ErrorKind::NoSuchKey,
                    format!("no such key: {}", key_text(&key)),
                )
            })?;
            put(child, rest, value)
        }
        CelValue::List(l) => {
            let ix = match k {
                CelValue::Int(i) => usize::try_from(*i).ok(),
                CelValue::Uint(u) => usize::try_from(*u).ok(),
                _ => None,
            };
            let len = l.len();
            let Some(ix) = ix.filter(|i| *i < len) else {
                return Err(CelError::new(
                    ErrorKind::IndexOutOfRange,
                    format!("list index out of range for a list of {len} elements"),
                ));
            };
            let l = Arc::make_mut(l);
            put(&mut l[ix], rest, value)
        }
        other => Err(CelError::new(
            ErrorKind::Location,
            format!("cannot assign into a {}", other.type_name()),
        )),
    }
}

/// Snapshot encoding: `null`, booleans, strings and lists map to themselves; every other
/// value becomes a one-key object whose key names its type (`$int`, `$uint`, `$double`,
/// `$decimal`, `$bytes`, `$timestamp`, `$duration`, `$type`, `$map`), so restoring is exact.
pub fn encode(v: &CelValue) -> Value {
    let tagged = |t: &str, x: Value| Value::Object(Vec::from([(String::from(t), x)]));
    match v {
        CelValue::Null => Value::Null,
        CelValue::Bool(b) => Value::Bool(*b),
        CelValue::String(s) => Value::String(s.to_string()),
        CelValue::List(l) => Value::Array(l.iter().map(encode).collect()),
        CelValue::Int(i) => tagged("$int", Value::String(i.to_string())),
        CelValue::Uint(u) => tagged("$uint", Value::String(u.to_string())),
        CelValue::Double(d) => tagged("$double", Value::String(format!("{:016x}", d.to_bits()))),
        CelValue::Decimal(d) => tagged("$decimal", Value::String(d.to_string())),
        CelValue::Bytes(b) => tagged(
            "$bytes",
            Value::Array(b.iter().map(|x| Value::Number(f64::from(*x))).collect()),
        ),
        CelValue::Timestamp(t) => tagged("$timestamp", Value::String(t.nanos().to_string())),
        CelValue::Duration(d) => tagged("$duration", Value::String(d.nanos().to_string())),
        CelValue::Type(t) => tagged("$type", Value::String(t.to_string())),
        CelValue::Map(m) => tagged(
            "$map",
            Value::Array(
                m.iter()
                    .map(|(k, x)| Value::Array(Vec::from([encode(&k.to_value()), encode(x)])))
                    .collect(),
            ),
        ),
    }
}

/// Decode [`encode`]'s output.
pub fn decode(v: &Value) -> Result<CelValue, String> {
    let bad = || String::from("malformed CEL snapshot value");
    Ok(match v {
        Value::Null | Value::Undefined => CelValue::Null,
        Value::Bool(b) => CelValue::Bool(*b),
        Value::String(s) => CelValue::str(s),
        Value::Array(a) => {
            CelValue::List(Arc::new(a.iter().map(decode).collect::<Result<_, _>>()?))
        }
        Value::Number(_) => return Err(bad()),
        Value::Object(o) => {
            let [(tag, x)] = o.as_slice() else {
                return Err(bad());
            };
            let text = || match x {
                Value::String(s) => Ok(s.as_str()),
                _ => Err(bad()),
            };
            match tag.as_str() {
                "$int" => CelValue::Int(text()?.parse().map_err(|_| bad())?),
                "$uint" => CelValue::Uint(text()?.parse().map_err(|_| bad())?),
                "$double" => CelValue::Double(f64::from_bits(
                    u64::from_str_radix(text()?, 16).map_err(|_| bad())?,
                )),
                "$decimal" => CelValue::Decimal(Decimal::parse(text()?).map_err(|_| bad())?),
                "$timestamp" => CelValue::Timestamp(
                    Timestamp::from_nanos(text()?.parse().map_err(|_| bad())?).ok_or_else(bad)?,
                ),
                "$duration" => CelValue::Duration(
                    Duration::from_nanos(text()?.parse().map_err(|_| bad())?).ok_or_else(bad)?,
                ),
                "$type" => CelValue::Type(Arc::from(text()?)),
                "$bytes" => {
                    let Value::Array(a) = x else {
                        return Err(bad());
                    };
                    let mut out = Vec::with_capacity(a.len());
                    for b in a {
                        match b {
                            Value::Number(n)
                                if (0.0..=255.0).contains(n) && crate::value::is_integral(*n) =>
                            {
                                out.push(*n as u8);
                            }
                            _ => return Err(bad()),
                        }
                    }
                    CelValue::Bytes(Arc::from(out))
                }
                "$map" => {
                    let Value::Array(a) = x else {
                        return Err(bad());
                    };
                    let mut m = CelMap::new();
                    for pair in a {
                        let Value::Array(kv) = pair else {
                            return Err(bad());
                        };
                        let [k, val] = kv.as_slice() else {
                            return Err(bad());
                        };
                        let key = Key::from_value(&decode(k)?).ok_or_else(bad)?;
                        m.insert(key, decode(val)?);
                    }
                    CelValue::Map(Arc::new(m))
                }
                _ => return Err(bad()),
            }
        }
    })
}
