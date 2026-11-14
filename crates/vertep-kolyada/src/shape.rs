//! Payload shapes: the subset of SHACL that a protocol uses to type a message payload.
//!
//! A shape lists properties. A property has a name, an optional datatype (`string`, `integer`,
//! `number`, `boolean`), a `min` and `max` count, an `in` list of allowed values and an optional
//! nested shape. Shapes are open, as SHACL's are: members a shape does not name are not checked.
//! Everything else in SHACL (`sh:sparql`, `sh:pattern`, paths) is outside the subset and is
//! rejected when the document is parsed.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A shape as written in a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeDecl {
    /// The properties of the payload object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDecl>,
}

/// A property of a shape as written in a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDecl {
    /// The member name.
    pub name: String,
    /// `string`, `integer`, `number` or `boolean`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datatype: Option<String>,
    /// The least number of values; zero when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    /// The greatest number of values; unbounded when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    /// The allowed values.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<Value>>,
    /// A nested shape for object values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeDecl>,
}

/// The datatypes of the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Datatype {
    /// A JSON string.
    String,
    /// A JSON number without a fraction.
    Integer,
    /// Any JSON number.
    Number,
    /// A JSON boolean.
    Boolean,
}

impl Datatype {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "string" => Self::String,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            _ => return None,
        })
    }

    fn accepts(self, v: &Value) -> bool {
        match self {
            Self::String => v.is_string(),
            Self::Integer => v.is_i64() || v.is_u64(),
            Self::Number => v.is_number(),
            Self::Boolean => v.is_boolean(),
        }
    }
}

/// A property of a checked shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The member name.
    pub name: String,
    /// The datatype, if the property has one.
    pub datatype: Option<Datatype>,
    /// The least number of values.
    pub min: u32,
    /// The greatest number of values.
    pub max: Option<u32>,
    /// The allowed values.
    pub allowed: Option<Vec<Value>>,
    /// The nested shape.
    pub shape: Option<Shape>,
}

/// A checked shape.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    /// The properties.
    pub fields: Vec<Field>,
}

/// Why a payload does not match its shape, or why a shape is outside the subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError {
    /// The member path, with `.` between nested names.
    pub path: String,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for ShapeError {}

fn err(path: &str, reason: impl Into<String>) -> ShapeError {
    ShapeError {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

impl Shape {
    /// Checks a declaration against the subset.
    ///
    /// # Errors
    ///
    /// [`ShapeError`] for an unknown datatype, `min` above `max`, an empty `in` list or a value
    /// in `in` that does not match the datatype.
    // @tg: implements:: [[openspec:agent-protocols#Payload Shapes]]
    pub fn from_decl(decl: &ShapeDecl) -> Result<Self, ShapeError> {
        Self::build(decl, "")
    }

    fn build(decl: &ShapeDecl, prefix: &str) -> Result<Self, ShapeError> {
        let mut fields = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for f in &decl.fields {
            let path = if prefix.is_empty() {
                f.name.clone()
            } else {
                format!("{prefix}.{}", f.name)
            };
            if !seen.insert(f.name.clone()) {
                return Err(err(&path, "declared twice"));
            }
            let datatype =
                match &f.datatype {
                    None => None,
                    Some(n) => Some(Datatype::parse(n).ok_or_else(|| {
                        err(&path, format!("datatype `{n}` is outside the subset"))
                    })?),
                };
            let min = f.min.unwrap_or(0);
            if f.max.is_some_and(|m| m < min) {
                return Err(err(&path, "min is above max"));
            }
            if let Some(list) = &f.allowed {
                if list.is_empty() {
                    return Err(err(&path, "in is empty"));
                }
                if let Some(dt) = datatype
                    && let Some(bad) = list.iter().find(|v| !dt.accepts(v))
                {
                    return Err(err(
                        &path,
                        format!("in value {bad} does not match the datatype"),
                    ));
                }
            }
            let shape = f
                .shape
                .as_ref()
                .map(|s| Self::build(s, &path))
                .transpose()?;
            fields.push(Field {
                name: f.name.clone(),
                datatype,
                min,
                max: f.max,
                allowed: f.allowed.clone(),
                shape,
            });
        }
        Ok(Self { fields })
    }

    /// Checks a payload against the shape.
    ///
    /// # Errors
    ///
    /// The first [`ShapeError`], naming the member.
    // @tg: implements:: [[openspec:agent-protocols#Payload Shapes]]
    pub fn validate(&self, payload: &Value) -> Result<(), ShapeError> {
        self.validate_at(payload, "")
    }

    fn validate_at(&self, value: &Value, prefix: &str) -> Result<(), ShapeError> {
        let Some(obj) = value.as_object() else {
            return Err(err(
                if prefix.is_empty() { "payload" } else { prefix },
                "not an object",
            ));
        };
        for f in &self.fields {
            let path = if prefix.is_empty() {
                f.name.clone()
            } else {
                format!("{prefix}.{}", f.name)
            };
            let values: Vec<&Value> = match obj.get(&f.name) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => items.iter().collect(),
                Some(v) => vec![v],
            };
            let n = u32::try_from(values.len()).unwrap_or(u32::MAX);
            if n < f.min {
                return Err(err(&path, format!("needs at least {} value(s)", f.min)));
            }
            if f.max.is_some_and(|m| n > m) {
                return Err(err(
                    &path,
                    format!("allows at most {} value(s)", f.max.unwrap_or(0)),
                ));
            }
            for v in values {
                if let Some(dt) = f.datatype
                    && !dt.accepts(v)
                {
                    return Err(err(&path, "wrong datatype"));
                }
                if let Some(list) = &f.allowed
                    && !list.contains(v)
                {
                    return Err(err(&path, "value not allowed"));
                }
                if let Some(nested) = &f.shape {
                    nested.validate_at(v, &path)?;
                }
            }
        }
        Ok(())
    }

    /// Whether the shape has no properties.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}
