//! Checking runtime values against types, the canonical value encoding and the checked
//! conversion at the dynamic boundary.
//!
//! Values are [`crate::datamodel::Value`]s. The canonical encoding of a value of type `T` is a
//! JSON-compatible tree in which:
//!
//! - `int` is a number with no fraction in the signed 64-bit range;
//! - `decimal` is a string `-?[0-9]+(\.[0-9]+)?` (a number is accepted on input and encoded as
//!   its shortest decimal string);
//! - `datetime` is an RFC 3339 string with an offset, encoded in UTC with a `Z` suffix;
//! - `iri` is a string holding an IRI with a scheme;
//! - a record is an object with exactly the declared fields, sorted by name, absent optional
//!   fields omitted;
//! - a tagged union is `{"tag": <tag>, "value": <payload>}`, without `value` for a variant
//!   that has no payload;
//! - an optional value is `null` or the value; `undefined` counts as `null`.
//!
//! Named types may recurse through `optional` alone (`type A = optional<A>` is guarded and
//! valid; its only value is `null`). Checking stops when it would expand a named type it is
//! already expanding at the same position of the value, since nothing of the value was consumed
//! in between and the expansion would never end: the value is then a mismatch for that type.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::datamodel::{EvalError, Value};
use crate::types::decl::Declarations;
use crate::types::prim::{self, DatetimeError, IriError};
use crate::types::ty::{Primitive, Type};

/// One step of a path into a value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSeg {
    /// A record field.
    Field(String),
    /// A list index.
    Index(usize),
    /// The payload of a tagged union variant.
    Variant(String),
}

/// The location of a node inside a checked value, printed like `$.items[2].amount`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ValuePath(pub Vec<PathSeg>);

impl fmt::Display for ValuePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("$")?;
        for s in &self.0 {
            match s {
                PathSeg::Field(n) => write!(f, ".{n}")?,
                PathSeg::Index(i) => write!(f, "[{i}]")?,
                PathSeg::Variant(t) => write!(f, "<{t}>")?,
            }
        }
        Ok(())
    }
}

/// What is wrong with a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeErrorKind {
    /// The value has the wrong shape for the expected type.
    Mismatch {
        /// The expected type, printed.
        expected: String,
        /// What was found (`string`, `number`, `object`, ...).
        found: &'static str,
    },
    /// A number expected to be an `int` has a fraction or is outside the signed 64-bit range.
    IntOutOfRange,
    /// A string expected to be a `decimal` is not a decimal literal.
    InvalidDecimal,
    /// A `datetime` has no offset.
    DatetimeOffsetRequired,
    /// A `datetime` is not RFC 3339.
    InvalidDatetime(&'static str),
    /// An `iri` has no scheme.
    RelativeIri,
    /// An `iri` contains a character IRIs exclude.
    InvalidIri,
    /// A required record field is absent.
    MissingField(String),
    /// A record value has a field the record does not declare.
    UnknownField(String),
    /// A union value has no string `tag`.
    MissingTag,
    /// A union value names a tag the union does not have.
    UnknownTag(String),
    /// The variant needs a payload and the value has none.
    MissingPayload(String),
    /// The variant has no payload and the value carries one.
    UnexpectedPayload(String),
    /// A named type is not declared.
    UnknownType(String),
    /// Named types refer to each other without any structure in between.
    CyclicAlias(String),
}

/// A value that does not have the expected type, with its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError {
    /// Where in the value.
    pub path: ValuePath,
    /// What is wrong.
    pub kind: TypeErrorKind,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = &self.path;
        match &self.kind {
            TypeErrorKind::Mismatch { expected, found } => {
                write!(f, "at {p}: expected {expected}, found {found}")
            }
            TypeErrorKind::IntOutOfRange => write!(
                f,
                "at {p}: expected int, found a number that is not a signed 64-bit integer"
            ),
            TypeErrorKind::InvalidDecimal => {
                write!(
                    f,
                    "at {p}: expected decimal, found a string that is not a decimal"
                )
            }
            TypeErrorKind::DatetimeOffsetRequired => write!(
                f,
                "at {p}: expected datetime, an offset (`Z` or `+hh:mm`) is required"
            ),
            TypeErrorKind::InvalidDatetime(part) => {
                write!(f, "at {p}: expected an RFC 3339 datetime, bad {part}")
            }
            TypeErrorKind::RelativeIri => {
                write!(
                    f,
                    "at {p}: expected iri, the IRI is not absolute (no scheme)"
                )
            }
            TypeErrorKind::InvalidIri => {
                write!(
                    f,
                    "at {p}: expected iri, the IRI contains an invalid character"
                )
            }
            TypeErrorKind::MissingField(n) => write!(f, "at {p}: missing required field `{n}`"),
            TypeErrorKind::UnknownField(n) => write!(f, "at {p}: unknown field `{n}`"),
            TypeErrorKind::MissingTag => write!(f, "at {p}: tagged union value has no `tag`"),
            TypeErrorKind::UnknownTag(t) => write!(f, "at {p}: unknown variant `{t}`"),
            TypeErrorKind::MissingPayload(t) => {
                write!(f, "at {p}: variant `{t}` requires a payload (`value`)")
            }
            TypeErrorKind::UnexpectedPayload(t) => {
                write!(f, "at {p}: variant `{t}` takes no payload")
            }
            TypeErrorKind::UnknownType(n) => write!(f, "at {p}: unknown type `{n}`"),
            TypeErrorKind::CyclicAlias(n) => {
                write!(f, "at {p}: type `{n}` is an unguarded cycle of names")
            }
        }
    }
}

/// The kind of a value, for messages.
pub fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Undefined => "undefined",
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

/// Check `value` against `ty` with no named types in scope, reporting every error.
// @tg: implements:: [[openspec:type-system#Primitive Types]]
// @tg: implements:: [[openspec:type-system#Composite Types]]
pub fn check_value(ty: &Type, value: &Value) -> Result<(), Vec<TypeError>> {
    Declarations::default().check(ty, value)
}

/// The canonical encoding of `value` as a value of `ty` (see the module docs), with no named
/// types in scope.
pub fn encode(ty: &Type, value: &Value) -> Result<Value, Vec<TypeError>> {
    Declarations::default().encode(ty, value)
}

/// The checked conversion from the dynamic type `value` to `ty`, with no named types in scope.
pub fn convert(ty: &Type, value: &Value) -> Result<Value, EvalError> {
    Declarations::default().convert(ty, value)
}

impl Declarations {
    /// Check `value` against `ty`, resolving named types in this block, and report every error.
    // @tg: implements:: [[openspec:type-system#Composite Types]]
    pub fn check(&self, ty: &Type, value: &Value) -> Result<(), Vec<TypeError>> {
        let mut c = Walker {
            decls: self,
            errors: Vec::new(),
            path: Vec::new(),
            unfolding: Vec::new(),
        };
        c.walk(ty, value);
        if c.errors.is_empty() {
            Ok(())
        } else {
            Err(c.errors)
        }
    }

    /// Check `value` against `ty` and return its canonical encoding.
    pub fn encode(&self, ty: &Type, value: &Value) -> Result<Value, Vec<TypeError>> {
        let mut c = Walker {
            decls: self,
            errors: Vec::new(),
            path: Vec::new(),
            unfolding: Vec::new(),
        };
        let out = c.walk(ty, value);
        if c.errors.is_empty() {
            Ok(out.unwrap_or(Value::Null))
        } else {
            Err(c.errors)
        }
    }

    /// The explicit checked conversion of a dynamic `value` to `ty`
    /// ([[openspec:type-system#Dynamic Value Boundary]]). Converting to `value` itself performs
    /// no check and returns the value unchanged. On mismatch the error describes every type
    /// error; the interpreter raises it as `error.execution`.
    // @tg: implements:: [[openspec:type-system#Dynamic Value Boundary]]
    pub fn convert(&self, ty: &Type, value: &Value) -> Result<Value, EvalError> {
        if *ty == Type::Value {
            return Ok(value.clone());
        }
        self.encode(ty, value).map_err(|errs| {
            let mut msg = format!("type mismatch converting to {ty}");
            for e in errs {
                msg.push_str("; ");
                msg.push_str(&e.to_string());
            }
            EvalError::new(msg)
        })
    }
}

struct Walker<'a> {
    decls: &'a Declarations,
    errors: Vec<TypeError>,
    path: Vec<PathSeg>,
    /// The named types being expanded at the current position of the value.
    unfolding: Vec<String>,
}

fn is_absent(v: &Value) -> bool {
    matches!(v, Value::Undefined | Value::Null)
}

impl<'a> Walker<'a> {
    fn err(&mut self, kind: TypeErrorKind) {
        self.errors.push(TypeError {
            path: ValuePath(self.path.clone()),
            kind,
        });
    }

    fn mismatch(&mut self, ty: &Type, v: &Value) {
        self.err(TypeErrorKind::Mismatch {
            expected: ty.to_string(),
            found: kind_of(v),
        });
    }

    /// Resolve named types to a structural type. `hops` counts consecutive name expansions.
    fn resolve<'t>(&mut self, ty: &'t Type) -> Option<&'t Type>
    where
        'a: 't,
    {
        let mut t = ty;
        let mut hops = 0;
        while let Type::Named(n) = t {
            if hops > self.decls.types.len() {
                self.err(TypeErrorKind::CyclicAlias(n.clone()));
                return None;
            }
            match self.decls.lookup(n) {
                Some(d) => t = d,
                None => {
                    self.err(TypeErrorKind::UnknownType(n.clone()));
                    return None;
                }
            }
            hops += 1;
        }
        Some(t)
    }

    /// Check `v` against `ty` and build its canonical encoding.
    ///
    /// A named type that is already being expanded at this position of the value is a
    /// non-contractive recursion (through `optional` only): nothing of `v` was consumed since,
    /// so `v` is no value of it.
    // @tg: implements:: [[decisions#Type checking stops on non-contractive recursion]]
    fn walk(&mut self, ty: &Type, v: &Value) -> Option<Value> {
        let Type::Named(n) = ty else {
            return self.walk_structure(ty, v);
        };
        if self.unfolding.contains(n) {
            self.mismatch(ty, v);
            return None;
        }
        self.unfolding.push(n.clone());
        let out = self.walk_structure(ty, v);
        self.unfolding.pop();
        out
    }

    /// Check the part `v` of the value, at `seg` below the current position, against `ty`.
    fn walk_child(&mut self, seg: PathSeg, ty: &Type, v: &Value) -> Option<Value> {
        self.path.push(seg);
        let unfolding = core::mem::take(&mut self.unfolding);
        let r = self.walk(ty, v);
        self.unfolding = unfolding;
        self.path.pop();
        r
    }

    fn walk_structure(&mut self, ty: &Type, v: &Value) -> Option<Value> {
        let ty = self.resolve(ty)?;
        match ty {
            Type::Value => Some(v.clone()),
            Type::Named(_) => None,
            Type::Primitive(p) => self.primitive(*p, ty, v),
            Type::Optional(inner) => {
                if is_absent(v) {
                    Some(Value::Null)
                } else {
                    self.walk(inner, v)
                }
            }
            Type::List(inner) => {
                let Value::Array(items) = v else {
                    self.mismatch(ty, v);
                    return None;
                };
                let mut out = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    if let Some(r) = self.walk_child(PathSeg::Index(i), inner, item) {
                        out.push(r);
                    }
                }
                Some(Value::Array(out))
            }
            Type::Record(fields) => {
                let Value::Object(entries) = v else {
                    self.mismatch(ty, v);
                    return None;
                };
                let mut out: Vec<(String, Value)> = Vec::new();
                for f in fields {
                    let present = entries
                        .iter()
                        .find(|(k, _)| *k == f.name)
                        .map(|(_, v)| v)
                        .filter(|v| !matches!(v, Value::Undefined));
                    let optional = matches!(
                        self.resolve_quiet(&f.ty),
                        Some(Type::Optional(_)) | Some(Type::Value)
                    );
                    match present {
                        None if optional => {}
                        None => self.err(TypeErrorKind::MissingField(f.name.clone())),
                        Some(fv) => {
                            let r = self.walk_child(PathSeg::Field(f.name.clone()), &f.ty, fv);
                            if let Some(r) = r
                                && !(optional && is_absent(&r))
                            {
                                out.push((f.name.clone(), r));
                            }
                        }
                    }
                }
                // An entry holding `undefined` is absent, so it is no unknown field either.
                for (k, fv) in entries {
                    if !matches!(fv, Value::Undefined) && !fields.iter().any(|f| f.name == *k) {
                        self.err(TypeErrorKind::UnknownField(k.clone()));
                    }
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Some(Value::Object(out))
            }
            Type::Union(variants) => {
                let Value::Object(entries) = v else {
                    self.mismatch(ty, v);
                    return None;
                };
                let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v);
                let Some(Value::String(tag)) = get("tag") else {
                    self.err(TypeErrorKind::MissingTag);
                    return None;
                };
                for (k, fv) in entries {
                    if k != "tag" && k != "value" && !matches!(fv, Value::Undefined) {
                        self.err(TypeErrorKind::UnknownField(k.clone()));
                    }
                }
                let Some(variant) = variants.iter().find(|var| var.tag == *tag) else {
                    self.err(TypeErrorKind::UnknownTag(tag.clone()));
                    return None;
                };
                let payload = get("value").filter(|p| !matches!(p, Value::Undefined));
                let mut out = Vec::from([("tag".to_string(), Value::String(tag.clone()))]);
                match (&variant.payload, payload) {
                    (None, None) => {}
                    (None, Some(_)) => self.err(TypeErrorKind::UnexpectedPayload(tag.clone())),
                    (Some(pt), None) => {
                        let optional = matches!(
                            self.resolve_quiet(pt),
                            Some(Type::Optional(_) | Type::Value)
                        );
                        if !optional {
                            self.err(TypeErrorKind::MissingPayload(tag.clone()));
                        }
                    }
                    (Some(pt), Some(pv)) => {
                        let r = self.walk_child(PathSeg::Variant(tag.clone()), pt, pv);
                        if let Some(r) = r {
                            out.push(("value".to_string(), r));
                        }
                    }
                }
                Some(Value::Object(out))
            }
        }
    }

    /// Resolve names without reporting errors (used to ask "is this optional?").
    fn resolve_quiet<'t>(&self, ty: &'t Type) -> Option<&'t Type>
    where
        'a: 't,
    {
        let mut t = ty;
        for _ in 0..=self.decls.types.len() {
            match t {
                Type::Named(n) => t = self.decls.lookup(n)?,
                other => return Some(other),
            }
        }
        None
    }

    fn primitive(&mut self, p: Primitive, ty: &Type, v: &Value) -> Option<Value> {
        match (p, v) {
            (Primitive::Bool, Value::Bool(_)) => Some(v.clone()),
            (Primitive::String, Value::String(_)) => Some(v.clone()),
            (Primitive::Int, Value::Number(n)) => match prim::f64_to_i64(*n) {
                Some(_) => Some(v.clone()),
                None => {
                    self.err(TypeErrorKind::IntOutOfRange);
                    None
                }
            },
            (Primitive::Decimal, Value::Number(n)) if n.is_finite() => {
                Some(Value::String(format!("{n}")))
            }
            (Primitive::Decimal, Value::String(s)) => {
                if prim::is_decimal(s) {
                    Some(v.clone())
                } else {
                    self.err(TypeErrorKind::InvalidDecimal);
                    None
                }
            }
            (Primitive::Datetime, Value::String(s)) => match prim::parse_datetime(s) {
                Ok(dt) => Some(Value::String(
                    dt.to_utc_string().unwrap_or_else(|| s.clone()),
                )),
                Err(DatetimeError::MissingOffset) => {
                    self.err(TypeErrorKind::DatetimeOffsetRequired);
                    None
                }
                Err(DatetimeError::Invalid(part)) => {
                    self.err(TypeErrorKind::InvalidDatetime(part));
                    None
                }
            },
            (Primitive::Iri, Value::String(s)) => match prim::check_iri(s) {
                Ok(()) => Some(v.clone()),
                Err(IriError::Relative) => {
                    self.err(TypeErrorKind::RelativeIri);
                    None
                }
                Err(IriError::InvalidCharacter) => {
                    self.err(TypeErrorKind::InvalidIri);
                    None
                }
            },
            _ => {
                self.mismatch(ty, v);
                None
            }
        }
    }
}

/// Whether a value of static type `from` can be used where `to` is expected without a
/// conversion: `to` is `value`, the types are equal, `int` widens to `decimal`, `T` to
/// `optional<T>`, records with the same fields, unions whose variants are all in the target,
/// and lists and optionals of assignable types. `value` is never assignable to a static type:
/// that takes the explicit checked conversion [`Declarations::convert`].
// @tg: implements:: [[openspec:type-system#Dynamic Value Boundary]]
pub fn is_assignable(decls: &Declarations, from: &Type, to: &Type) -> bool {
    let mut assumed: Vec<(String, String)> = Vec::new();
    assignable(decls, from, to, &mut assumed, 0)
}

fn assignable(
    decls: &Declarations,
    from: &Type,
    to: &Type,
    assumed: &mut Vec<(String, String)>,
    depth: usize,
) -> bool {
    if depth > 64 {
        return false;
    }
    if from == to || *to == Type::Value {
        return true;
    }
    if let (Type::Named(a), Type::Named(b)) = (from, to) {
        let key = (a.clone(), b.clone());
        if assumed.contains(&key) {
            return true;
        }
        assumed.push(key);
    }
    let resolve = |t: &Type| -> Option<Type> {
        let mut t = t.clone();
        for _ in 0..=decls.types.len() {
            match t {
                Type::Named(n) => t = decls.lookup(&n)?.clone(),
                other => return Some(other),
            }
        }
        None
    };
    let (Some(f), Some(t)) = (resolve(from), resolve(to)) else {
        return false;
    };
    let rec = |a: &Type, b: &Type, assumed: &mut Vec<(String, String)>| {
        assignable(decls, a, b, assumed, depth + 1)
    };
    match (&f, &t) {
        (_, Type::Value) => true,
        (Type::Value, _) => false,
        (Type::Primitive(Primitive::Int), Type::Primitive(Primitive::Decimal)) => true,
        (Type::Primitive(a), Type::Primitive(b)) => a == b,
        (Type::Optional(a), Type::Optional(b)) => rec(a, b, assumed),
        (_, Type::Optional(b)) => rec(&f, b, assumed),
        (Type::List(a), Type::List(b)) => rec(a, b, assumed),
        (Type::Record(fa), Type::Record(fb)) => {
            fa.iter().all(|x| {
                fb.iter()
                    .find(|y| y.name == x.name)
                    .is_some_and(|y| rec(&x.ty, &y.ty, assumed))
            }) && fb.iter().all(|y| {
                fa.iter().any(|x| x.name == y.name)
                    || matches!(resolve(&y.ty), Some(Type::Optional(_) | Type::Value))
            })
        }
        (Type::Union(va), Type::Union(vb)) => va.iter().all(|x| {
            vb.iter()
                .find(|y| y.tag == x.tag)
                .is_some_and(|y| match (&x.payload, &y.payload) {
                    (None, None) => true,
                    (Some(a), Some(b)) => rec(a, b, assumed),
                    _ => false,
                })
        }),
        _ => false,
    }
}
