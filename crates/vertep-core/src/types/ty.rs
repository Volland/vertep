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
    /// Data-protection marks written before the field name (`@sensitive`, `@subject`,
    /// `@retain(P30D)`). They do not change which values the field accepts.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "FieldMarks::is_empty")
    )]
    pub marks: FieldMarks,
}

/// The data-protection marks of a record field.
///
/// `@sensitive` marks a field whose value is personal data: a journal seals it under the data
/// key of its subject. `@subject` marks the string field naming that subject; it applies to the
/// sensitive fields of its own record and of the records nested in it. `@retain(<duration>)`
/// gives a sensitive field a retention horizon, an ISO 8601 duration of weeks, days, hours,
/// minutes and seconds (`P30D`, `PT12H`, `P1DT6H`). Marks are annotations: the checker accepts
/// the same values with or without them, and the projections carry them along.
// @tg: implements:: [[openspec:journal-redaction#Sensitive Fields]] {confidence: partial}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldMarks {
    /// `@sensitive`: the field holds personal data and is sealed in journals.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sensitive: bool,
    /// `@subject`: the field names the data subject of the sensitive fields in scope.
    #[cfg_attr(feature = "serde", serde(default))]
    pub subject: bool,
    /// `@retain(...)`: the retention horizon of a sensitive field, in milliseconds.
    #[cfg_attr(feature = "serde", serde(default))]
    pub retain_ms: Option<u64>,
}

impl FieldMarks {
    /// Whether the field carries no mark.
    pub fn is_empty(&self) -> bool {
        !self.sensitive && !self.subject && self.retain_ms.is_none()
    }

    /// The retention horizon as a canonical ISO 8601 duration (`P30D`, `PT12H`, `P1DT6H30M`).
    pub fn retention(&self) -> Option<String> {
        self.retain_ms.map(format_duration)
    }
}

/// Milliseconds per day, hour, minute and second.
const DURATION_UNITS: [(char, u64); 4] = [
    ('D', 86_400_000),
    ('H', 3_600_000),
    ('M', 60_000),
    ('S', 1_000),
];

/// The canonical ISO 8601 form of a whole number of seconds given in milliseconds: days, then
/// `T` and hours, minutes and seconds, omitting zero parts; zero is `PT0S`.
pub fn format_duration(ms: u64) -> String {
    use core::fmt::Write;
    let mut out = String::from("P");
    let mut rest = ms;
    let mut time = false;
    for (i, (unit, size)) in DURATION_UNITS.iter().enumerate() {
        let n = rest / size;
        rest %= size;
        if n == 0 {
            continue;
        }
        if i > 0 && !time {
            out.push('T');
            time = true;
        }
        let _ = write!(out, "{n}{unit}");
    }
    if out.len() == 1 {
        out.push_str("T0S");
    }
    out
}

/// Parse an ISO 8601 duration of fixed-length units: `P[nW][nD][T[nH][nM][nS]]` with whole
/// numbers, to milliseconds. Years and months are refused: their length varies.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let bad = || alloc::format!("`{text}` is not a duration such as P30D, PT12H or P1DT6H");
    let rest = text.strip_prefix('P').ok_or_else(bad)?;
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) if !t.is_empty() => (d, Some(t)),
        Some(_) => return Err(bad()),
        None => (rest, None),
    };
    let mut total: u64 = 0;
    let mut any = false;
    let mut part = |src: &str, units: &[(char, u64)]| -> Result<(), String> {
        let mut next = 0;
        let mut num = String::new();
        for c in src.chars() {
            if c.is_ascii_digit() {
                num.push(c);
                continue;
            }
            if c == 'Y' || (c == 'M' && units[0].0 == 'W') {
                return Err(alloc::format!(
                    "`{text}`: years and months have no fixed length; write days (P30D)"
                ));
            }
            let Some(i) = units[next..].iter().position(|(u, _)| *u == c) else {
                return Err(bad());
            };
            if num.is_empty() {
                return Err(bad());
            }
            let n: u64 = num.parse().map_err(|_| bad())?;
            let add = n.checked_mul(units[next + i].1).ok_or_else(bad)?;
            total = total.checked_add(add).ok_or_else(bad)?;
            any = true;
            next += i + 1;
            num.clear();
        }
        if num.is_empty() { Ok(()) } else { Err(bad()) }
    };
    part(date, &[('W', 7 * 86_400_000), ('D', 86_400_000)])?;
    if let Some(t) = time {
        part(t, &[('H', 3_600_000), ('M', 60_000), ('S', 1_000)])?;
    }
    if any { Ok(total) } else { Err(bad()) }
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
            marks: FieldMarks::default(),
        }
    }

    /// The field marked `@sensitive`.
    pub fn sensitive(mut self) -> Self {
        self.marks.sensitive = true;
        self
    }

    /// The field marked `@subject`.
    pub fn subject(mut self) -> Self {
        self.marks.subject = true;
        self
    }

    /// The field with a retention horizon of `ms` milliseconds (`@retain(...)`).
    pub fn retain_ms(mut self, ms: u64) -> Self {
        self.marks.retain_ms = Some(ms);
        self
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
