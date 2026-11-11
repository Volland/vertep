//! The type language data structures.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// The primitive types. `int` is a signed 64-bit integer, `decimal` an arbitrary-precision
/// decimal, `datetime` an RFC 3339 instant with an explicit offset and `iri` an IRI with a
/// scheme (RFC 3987).
// @tg: implements:: [[openspec:type-system#Primitive Types]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Primitive {
    /// `bool`.
    Bool,
    /// `int`: a signed 64-bit integer.
    Int,
    /// `decimal`: an arbitrary-precision decimal.
    Decimal,
    /// `string`.
    String,
    /// `datetime`: an RFC 3339 instant with an explicit offset.
    Datetime,
    /// `iri`: an absolute IRI (RFC 3987).
    Iri,
}

impl Primitive {
    /// Every primitive, in declaration order.
    pub const ALL: [Primitive; 6] = [
        Primitive::Bool,
        Primitive::Int,
        Primitive::Decimal,
        Primitive::String,
        Primitive::Datetime,
        Primitive::Iri,
    ];

    /// The primitive's name in the textual syntax.
    pub const fn name(self) -> &'static str {
        match self {
            Primitive::Bool => "bool",
            Primitive::Int => "int",
            Primitive::Decimal => "decimal",
            Primitive::String => "string",
            Primitive::Datetime => "datetime",
            Primitive::Iri => "iri",
        }
    }

    /// The primitive with this name.
    pub fn from_name(name: &str) -> Option<Primitive> {
        Primitive::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// A type of the chart type language.
///
/// Constructors nest to any depth. A [`Type::Named`] refers to a declaration in a
/// [`crate::types::Declarations`] block by name.
// @tg: implements:: [[openspec:type-system#Composite Types]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Type {
    /// A primitive.
    Primitive(Primitive),
    /// The dynamic type `value`: any JSON-compatible or RDF-literal value.
    Value,
    /// A closed record with named fields in declaration order. A field whose type is
    /// [`Type::Optional`] may be absent.
    Record(Vec<Field>),
    /// A tagged union; each variant has a tag and an optional payload type.
    Union(Vec<Variant>),
    /// A homogeneous list.
    List(Box<Type>),
    /// An optional value.
    Optional(Box<Type>),
    /// A reference to a named type.
    Named(String),
}

/// A record field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: Type,
}

/// A tagged union variant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Variant {
    /// The tag.
    pub tag: String,
    /// The payload type; `None` for a variant without payload.
    pub payload: Option<Type>,
}

impl Field {
    /// A field.
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Field {
            name: name.into(),
            ty,
        }
    }
}

impl Variant {
    /// A variant.
    pub fn new(tag: impl Into<String>, payload: Option<Type>) -> Self {
        Variant {
            tag: tag.into(),
            payload,
        }
    }
}

impl Type {
    /// `bool`.
    pub const BOOL: Type = Type::Primitive(Primitive::Bool);
    /// `int`.
    pub const INT: Type = Type::Primitive(Primitive::Int);
    /// `decimal`.
    pub const DECIMAL: Type = Type::Primitive(Primitive::Decimal);
    /// `string`.
    pub const STRING: Type = Type::Primitive(Primitive::String);
    /// `datetime`.
    pub const DATETIME: Type = Type::Primitive(Primitive::Datetime);
    /// `iri`.
    pub const IRI: Type = Type::Primitive(Primitive::Iri);

    /// `list<t>`.
    pub fn list(t: Type) -> Type {
        Type::List(Box::new(t))
    }

    /// `optional<t>`.
    pub fn optional(t: Type) -> Type {
        Type::Optional(Box::new(t))
    }

    /// A reference to the named type `name`.
    pub fn named(name: impl Into<String>) -> Type {
        Type::Named(name.into())
    }

    /// A record from `(name, type)` pairs.
    pub fn record<N: Into<String>>(fields: impl IntoIterator<Item = (N, Type)>) -> Type {
        Type::Record(fields.into_iter().map(|(n, t)| Field::new(n, t)).collect())
    }

    /// A tagged union from `(tag, payload)` pairs.
    pub fn union<N: Into<String>>(variants: impl IntoIterator<Item = (N, Option<Type>)>) -> Type {
        Type::Union(
            variants
                .into_iter()
                .map(|(n, t)| Variant::new(n, t))
                .collect(),
        )
    }

    /// The names of the named types this type refers to, in order of first appearance.
    pub fn references(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_refs(&mut out);
        out
    }

    fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Type::Primitive(_) | Type::Value => {}
            Type::Named(n) => {
                if !out.contains(&n.as_str()) {
                    out.push(n);
                }
            }
            Type::Record(fields) => fields.iter().for_each(|f| f.ty.collect_refs(out)),
            Type::Union(vs) => vs
                .iter()
                .filter_map(|v| v.payload.as_ref())
                .for_each(|t| t.collect_refs(out)),
            Type::List(t) | Type::Optional(t) => t.collect_refs(out),
        }
    }
}

impl core::fmt::Display for Type {
    /// The textual syntax; [`crate::types::parse_type`] reads it back to an equal type.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        crate::types::syntax::write_type(self, f)
    }
}
