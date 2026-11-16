//! Payload shapes: the subset of SHACL that a protocol uses to type a message payload.
//!
//! A shape lists properties. A property has a name, an optional datatype (`string`, `integer`,
//! `number`, `boolean`), a `min` and `max` count, an `in` list of allowed values and an optional
//! nested shape. Shapes are open, as SHACL's are: members a shape does not name are not checked.
//! Everything else in SHACL (`sh:sparql`, `sh:pattern`, paths) is outside the subset and is
//! rejected when the document is parsed.
//!
//! The checker interprets the subset directly. [`Shape::to_shacl`] writes the same shape as a
//! SHACL Core shapes graph and [`payload_graph`] maps a payload to RDF, so a full SHACL validator
//! (`vertep-rdf`'s) can judge the payload too; the crate's tests hold the two verdicts equal.
//!
//! The mapping: the payload is the node [`PAYLOAD_NODE`], a member is the predicate
//! [`FIELD_NS`] plus its percent-encoded name, an array gives one value per distinct member, a
//! `null` member is absent, a nested object is a blank node, a string is an `xsd:string`, an
//! integral number an `xsd:integer`, any other number an `xsd:double`, a boolean an
//! `xsd:boolean`, and a nested array or a `null` inside an array an `rdf:JSON` literal.

use std::fmt;

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{BlankNode, Graph, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The IRI of the payload node in [`payload_graph`], and the target of [`Shape::to_shacl`].
pub const PAYLOAD_NODE: &str = "urn:vertep:kolyada:payload";

/// The namespace of payload member predicates.
pub const FIELD_NS: &str = "https://w3id.org/vertep/kolyada/payload#";

const SH: &str = "http://www.w3.org/ns/shacl#";

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
                // `sh:in` lists RDF terms: a value of `in` is a literal, never an object.
                if let Some(bad) = list
                    .iter()
                    .find(|v| !(v.is_string() || v.is_number() || v.is_boolean()))
                {
                    return Err(err(
                        &path,
                        format!("in value {bad} is not a string, number or boolean"),
                    ));
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
            let members: Vec<&Value> = match obj.get(&f.name) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => items.iter().collect(),
                Some(v) => vec![v],
            };
            // SHACL counts value nodes, and an RDF graph holds a literal once: `["a", "a"]` is
            // one value, as it is after the payload's mapping to RDF ([`payload_graph`]). Each
            // object is its own node, so two equal objects are two values.
            let mut values: Vec<&Value> = Vec::new();
            for v in members {
                if v.is_object() || !values.contains(&v) {
                    values.push(v);
                }
            }
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

fn sh(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{SH}{local}"))
}

/// The predicate of the payload member `name`: [`FIELD_NS`] and the name, with every byte
/// outside the IRI-unreserved characters percent-encoded.
#[must_use]
pub fn field_predicate(name: &str) -> NamedNode {
    let mut iri = String::from(FIELD_NS);
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            iri.push(char::from(b));
        } else {
            iri.push_str(&format!("%{b:02X}"));
        }
    }
    NamedNode::new_unchecked(iri)
}

/// The RDF term of a JSON value that is not an object. Objects become blank nodes in
/// [`payload_graph`]; arrays and `null` reach here only inside an array.
fn literal(v: &Value) -> Literal {
    match v {
        Value::String(s) => Literal::new_simple_literal(s),
        Value::Bool(b) => Literal::new_typed_literal(b.to_string(), xsd::BOOLEAN),
        Value::Number(n) if n.is_i64() || n.is_u64() => {
            Literal::new_typed_literal(n.to_string(), xsd::INTEGER)
        }
        Value::Number(n) => {
            // JSON has no NaN or infinity; -0 and 0 are one JSON value, so one literal.
            let f = n.as_f64().unwrap_or_default();
            let f = if f == 0.0 { 0.0 } else { f };
            Literal::new_typed_literal(format!("{f:E}"), xsd::DOUBLE)
        }
        Value::Null | Value::Array(_) | Value::Object(_) => Literal::new_typed_literal(
            String::from_utf8(serde_json_canonicalizer::to_vec(v).unwrap_or_default())
                .unwrap_or_default(),
            rdf::JSON,
        ),
    }
}

fn rdf_list(g: &mut Graph, items: Vec<Term>) -> Term {
    let mut head: Term = rdf::NIL.into_owned().into();
    for item in items.into_iter().rev() {
        let node = BlankNode::default();
        g.insert(&Triple::new(node.clone(), rdf::FIRST, item));
        g.insert(&Triple::new(node.clone(), rdf::REST, head));
        head = node.into();
    }
    head
}

fn add_values(g: &mut Graph, subject: &NamedOrBlankNode, obj: &serde_json::Map<String, Value>) {
    for (name, v) in obj {
        let p = field_predicate(name);
        let members: Vec<&Value> = match v {
            Value::Null => Vec::new(),
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for m in members {
            let o: Term = match m {
                Value::Object(inner) => {
                    let node = BlankNode::default();
                    add_values(g, &node.clone().into(), inner);
                    node.into()
                }
                other => literal(other).into(),
            };
            g.insert(&Triple::new(subject.clone(), p.clone(), o));
        }
    }
}

/// The RDF graph of a payload, with the payload as the node [`PAYLOAD_NODE`]; `None` when the
/// payload is not a JSON object. See the module documentation for the mapping.
#[must_use]
pub fn payload_graph(payload: &Value) -> Option<Graph> {
    let obj = payload.as_object()?;
    let mut g = Graph::new();
    add_values(&mut g, &NamedNode::new_unchecked(PAYLOAD_NODE).into(), obj);
    Some(g)
}

impl Shape {
    /// The shape as a SHACL Core shapes graph whose node shape targets [`PAYLOAD_NODE`]. Over a
    /// payload mapped by [`payload_graph`], a SHACL validator conforms exactly when
    /// [`Shape::validate`] accepts.
    // @tg: implements:: [[openspec:agent-protocols#Payload Shapes]]
    #[must_use]
    pub fn to_shacl(&self) -> Graph {
        let mut g = Graph::new();
        let root = self.node_shape(&mut g);
        g.insert(&Triple::new(
            root,
            sh("targetNode"),
            NamedNode::new_unchecked(PAYLOAD_NODE),
        ));
        g
    }

    fn node_shape(&self, g: &mut Graph) -> BlankNode {
        let node = BlankNode::default();
        g.insert(&Triple::new(node.clone(), rdf::TYPE, sh("NodeShape")));
        for f in &self.fields {
            let p = BlankNode::default();
            g.insert(&Triple::new(node.clone(), sh("property"), p.clone()));
            g.insert(&Triple::new(
                p.clone(),
                sh("path"),
                field_predicate(&f.name),
            ));
            if f.min > 0 {
                g.insert(&Triple::new(
                    p.clone(),
                    sh("minCount"),
                    Literal::new_typed_literal(f.min.to_string(), xsd::INTEGER),
                ));
            }
            if let Some(max) = f.max {
                g.insert(&Triple::new(
                    p.clone(),
                    sh("maxCount"),
                    Literal::new_typed_literal(max.to_string(), xsd::INTEGER),
                ));
            }
            match f.datatype {
                None => {}
                Some(Datatype::Number) => {
                    // Any JSON number: an integral one maps to xsd:integer, others to xsd:double.
                    let alternatives = [xsd::INTEGER, xsd::DOUBLE]
                        .into_iter()
                        .map(|dt| {
                            let s = BlankNode::default();
                            g.insert(&Triple::new(s.clone(), sh("datatype"), dt));
                            Term::from(s)
                        })
                        .collect();
                    let list = rdf_list(g, alternatives);
                    g.insert(&Triple::new(p.clone(), sh("or"), list));
                }
                Some(Datatype::String) => {
                    g.insert(&Triple::new(p.clone(), sh("datatype"), xsd::STRING));
                }
                Some(Datatype::Integer) => {
                    g.insert(&Triple::new(p.clone(), sh("datatype"), xsd::INTEGER));
                }
                Some(Datatype::Boolean) => {
                    g.insert(&Triple::new(p.clone(), sh("datatype"), xsd::BOOLEAN));
                }
            }
            if let Some(list) = &f.allowed {
                let items = list.iter().map(|v| literal(v).into()).collect();
                let head = rdf_list(g, items);
                g.insert(&Triple::new(p.clone(), sh("in"), head));
            }
            if let Some(nested) = &f.shape {
                // The checker wants an object here, which maps to a node, never a literal.
                g.insert(&Triple::new(
                    p.clone(),
                    sh("nodeKind"),
                    sh("BlankNodeOrIRI"),
                ));
                let n = nested.node_shape(g);
                g.insert(&Triple::new(p, sh("node"), n));
            }
        }
        node
    }
}
