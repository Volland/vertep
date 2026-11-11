//! CEL types and the chart's CEL type environment.
//!
//! [`TypeEnv`] is the target of the type-system projection
//! ([[openspec:type-system#CEL Type Environment Projection]]): records become closed
//! [`CelType::Record`]s, tagged unions [`CelType::Union`]s with `tag` and `value` fields, lists
//! `list(T)`, optional values [`CelType::Optional`] (the value or `null`), `int` stays `int`,
//! `decimal` is the exact `decimal` extension type, `string` and `iri` are `string`,
//! `datetime` is `google.protobuf.Timestamp`, and the dynamic `value` is `dyn`.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

/// A CEL type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CelType {
    /// `dyn`: checked at runtime.
    Dyn,
    /// `null_type`.
    Null,
    /// `bool`.
    Bool,
    /// `int`.
    Int,
    /// `uint`.
    Uint,
    /// `double`.
    Double,
    /// `decimal` (extension).
    Decimal,
    /// `string`.
    String,
    /// `bytes`.
    Bytes,
    /// `google.protobuf.Timestamp`.
    Timestamp,
    /// `google.protobuf.Duration`.
    Duration,
    /// `type`.
    Type,
    /// `list(T)`.
    List(Box<CelType>),
    /// `map(K, V)`.
    Map(Box<CelType>, Box<CelType>),
    /// A closed record (a message type): exactly these fields.
    Record(Arc<RecordType>),
    /// A tagged union: a map with a `tag` naming the variant and the variant's `value`.
    Union(Arc<UnionType>),
    /// The value or `null`.
    Optional(Box<CelType>),
}

/// A record type.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RecordType {
    /// The type's name, if it is a named type.
    pub name: Option<String>,
    /// Field types by name.
    pub fields: BTreeMap<String, CelType>,
}

/// A tagged union type.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UnionType {
    /// The type's name, if it is a named type.
    pub name: Option<String>,
    /// Variant payload types by tag; `None` for a variant without payload.
    pub variants: BTreeMap<String, Option<CelType>>,
}

impl CelType {
    /// `list(elem)`.
    pub fn list(elem: CelType) -> CelType {
        CelType::List(Box::new(elem))
    }

    /// `map(key, value)`.
    pub fn map(key: CelType, value: CelType) -> CelType {
        CelType::Map(Box::new(key), Box::new(value))
    }

    /// The value or `null`.
    pub fn optional(t: CelType) -> CelType {
        CelType::Optional(Box::new(t))
    }

    /// A closed record type.
    pub fn record<'a>(
        name: Option<&str>,
        fields: impl IntoIterator<Item = (&'a str, CelType)>,
    ) -> CelType {
        CelType::Record(Arc::new(RecordType {
            name: name.map(String::from),
            fields: fields
                .into_iter()
                .map(|(k, v)| (String::from(k), v))
                .collect(),
        }))
    }

    /// A tagged union type.
    pub fn union<'a>(
        name: Option<&str>,
        variants: impl IntoIterator<Item = (&'a str, Option<CelType>)>,
    ) -> CelType {
        CelType::Union(Arc::new(UnionType {
            name: name.map(String::from),
            variants: variants
                .into_iter()
                .map(|(k, v)| (String::from(k), v))
                .collect(),
        }))
    }

    /// Whether the type is `int`, `uint`, `double` or `decimal`.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            CelType::Int | CelType::Uint | CelType::Double | CelType::Decimal
        )
    }

    /// The type with any `Optional` wrapper removed.
    pub fn strip_optional(&self) -> &CelType {
        match self {
            CelType::Optional(t) => t.strip_optional(),
            t => t,
        }
    }

    /// The type of field `name` of a record, union or `map(string, V)`; `Ok(None)` when the
    /// type is dynamic, `Err` when the type has no such field.
    pub fn field(&self, name: &str) -> Result<CelType, String> {
        match self.strip_optional() {
            CelType::Dyn => Ok(CelType::Dyn),
            CelType::Record(r) => r
                .fields
                .get(name)
                .cloned()
                .ok_or_else(|| format!("no field `{name}` in {}", CelType::Record(r.clone()))),
            CelType::Union(u) => match name {
                "tag" => Ok(CelType::String),
                "value" => {
                    let mut payloads = u.variants.values().flatten();
                    match payloads.next() {
                        Some(first) if payloads.all(|p| p == first) => {
                            Ok(CelType::optional(first.clone()))
                        }
                        _ => Ok(CelType::Dyn),
                    }
                }
                _ => Err(format!(
                    "no field `{name}` in {}: a union has `tag` and `value`",
                    CelType::Union(u.clone())
                )),
            },
            CelType::Map(k, v) if matches!(**k, CelType::String | CelType::Dyn) => {
                Ok((**v).clone())
            }
            t => Err(format!("type {t} has no fields")),
        }
    }
}

impl fmt::Display for CelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CelType::Dyn => f.write_str("dyn"),
            CelType::Null => f.write_str("null_type"),
            CelType::Bool => f.write_str("bool"),
            CelType::Int => f.write_str("int"),
            CelType::Uint => f.write_str("uint"),
            CelType::Double => f.write_str("double"),
            CelType::Decimal => f.write_str("decimal"),
            CelType::String => f.write_str("string"),
            CelType::Bytes => f.write_str("bytes"),
            CelType::Timestamp => f.write_str("google.protobuf.Timestamp"),
            CelType::Duration => f.write_str("google.protobuf.Duration"),
            CelType::Type => f.write_str("type"),
            CelType::List(e) => write!(f, "list({e})"),
            CelType::Map(k, v) => write!(f, "map({k}, {v})"),
            CelType::Optional(t) => write!(f, "optional({t})"),
            CelType::Record(r) => match &r.name {
                Some(n) => f.write_str(n),
                None => {
                    let fields: Vec<String> =
                        r.fields.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    write!(f, "{{{}}}", fields.join(", "))
                }
            },
            CelType::Union(u) => match &u.name {
                Some(n) => f.write_str(n),
                None => {
                    let vs: Vec<String> = u.variants.keys().map(ToString::to_string).collect();
                    write!(f, "union({})", vs.join(" | "))
                }
            },
        }
    }
}

/// Whether a value of type `from` may be stored where `to` is expected. `dyn` on either
/// side defers the check to runtime; `int` and `uint` widen exactly to `decimal`; `null`
/// fits an optional; maps with string keys may be checked against records at runtime.
pub fn assignable(to: &CelType, from: &CelType) -> bool {
    use CelType as T;
    match (to, from) {
        (T::Dyn, _) | (_, T::Dyn) => true,
        (T::Optional(_), T::Null) => true,
        (T::Optional(a), T::Optional(b)) => assignable(a, b),
        (T::Optional(a), b) => assignable(a, b),
        (a, T::Optional(b)) => assignable(a, b),
        (T::Decimal, T::Int | T::Uint) => true,
        (T::List(a), T::List(b)) => assignable(a, b),
        (T::Map(ka, va), T::Map(kb, vb)) => assignable(ka, kb) && assignable(va, vb),
        (T::Record(r), T::Map(k, v)) => {
            assignable(&T::String, k) && r.fields.values().all(|f| assignable(f, v))
        }
        (T::Record(a), T::Record(b)) => {
            a == b
                || (a.fields.len() == b.fields.len()
                    && a.fields
                        .iter()
                        .all(|(n, t)| b.fields.get(n).is_some_and(|u| assignable(t, u))))
        }
        (T::Union(_), T::Map(..)) => true,
        (a, b) => a == b,
    }
}

/// The CEL type environment of a chart: the context record bound to `ctx` and the payload
/// type of each declared event bound to `ev.data`. An untyped environment checks names and
/// syntax only and types everything as `dyn`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeEnv {
    context: Option<BTreeMap<String, CelType>>,
    events: BTreeMap<String, CelType>,
}

impl TypeEnv {
    /// An untyped environment: `ctx` is a dynamic map, `ev.data` is `dyn`.
    pub fn untyped() -> Self {
        TypeEnv::default()
    }

    /// A typed environment with an empty context record.
    pub fn typed() -> Self {
        TypeEnv {
            context: Some(BTreeMap::new()),
            events: BTreeMap::new(),
        }
    }

    /// Declare a context field (making the environment typed).
    pub fn with_context_field(mut self, name: &str, ty: CelType) -> Self {
        self.context
            .get_or_insert_with(BTreeMap::new)
            .insert(String::from(name), ty);
        self
    }

    /// Declare the payload type of event `name` (`null` payloads use [`CelType::Null`]).
    pub fn with_event(mut self, name: &str, payload: CelType) -> Self {
        self.events.insert(String::from(name), payload);
        self
    }

    /// Whether the context is typed.
    pub fn is_typed(&self) -> bool {
        self.context.is_some()
    }

    /// The type of `ctx`.
    pub fn context_type(&self) -> CelType {
        match &self.context {
            None => CelType::map(CelType::String, CelType::Dyn),
            Some(fields) => CelType::Record(Arc::new(RecordType {
                name: Some(String::from("ctx")),
                fields: fields.clone(),
            })),
        }
    }

    /// The declared type of context field `name`: `Ok(Dyn)` when untyped, `Err` when typed
    /// and not declared.
    pub fn context_field(&self, name: &str) -> Result<CelType, String> {
        match &self.context {
            None => Ok(CelType::Dyn),
            Some(fields) => fields
                .get(name)
                .cloned()
                .ok_or_else(|| format!("no field `{name}` in the chart context")),
        }
    }

    /// The payload type of event `name`, when declared.
    pub fn event_payload(&self, name: &str) -> Option<&CelType> {
        self.events.get(name)
    }

    /// The type of `ev.data` for a transition on the given event descriptors: the common
    /// declared payload type when every descriptor names a declared event with the same
    /// payload, `dyn` otherwise.
    pub fn event_data_for(&self, descriptors: &[String]) -> CelType {
        let mut found: Option<&CelType> = None;
        for d in descriptors {
            let Some(t) = self.events.get(d.as_str()) else {
                return CelType::Dyn;
            };
            match found {
                None => found = Some(t),
                Some(f) if f == t => {}
                Some(_) => return CelType::Dyn,
            }
        }
        found.cloned().unwrap_or(CelType::Dyn)
    }
}
