//! The SHACL Core projection of types.
//!
//! A type projects to a node shape that the [RDF encoding](super::value) of a value conforms
//! to exactly when the core checker ([`Declarations::check`]) accepts the value. The node shape
//! of a named type is its type IRI; shapes nested in it are minted under that IRI
//! (`<type IRI>#s...`), so the projection has no blank nodes. Every `sh:path` is a predicate
//! (an IRI, which SHACL §2.3.1 reads as a predicate path): the members of a list are reached
//! through the `vt:listMember` triples of the [encoding](super::value), not `rdf:rest*/rdf:first`,
//! whose path nodes would have to be blank nodes.
//!
//! | type | constraints on the value node |
//! |---|---|
//! | `bool`, `int`, `string` | `sh:datatype` `xsd:boolean`, `xsd:long`, `xsd:string` |
//! | `decimal` | `sh:or` of `xsd:long`, `xsd:decimal` and an `xsd:string` matching the decimal pattern |
//! | `datetime`, `iri` | `xsd:string` with the RFC 3339 (offset required) or IRI `sh:pattern` (XPath regular expression syntax) |
//! | `value` | none |
//! | named type | `sh:node` its type IRI |
//! | `optional<T>` | `sh:or` of `sh:hasValue vt:Null` and the shape of `T` |
//! | `list<T>` | `rdf:nil`, or a closed collection cell with one `rdf:first`, one `rdf:rest`, and every `vt:listMember` conforming to `T` |
//! | record | a closed blank node: one property shape per field with `sh:maxCount 1`, `sh:minCount 1` unless the field is optional or `value`, and `sh:node` the field type |
//! | union | a closed blank node with exactly one string `tag`, at most one `value`, and `sh:or` of one shape per variant fixing the tag and the payload |

use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, Term, TripleRef};
use vertep_core::datamodel::Value;
use vertep_core::types::{Declarations, Primitive, Type};

use super::ident::{TypeIris, segment};
use super::value::{VALUE_ROOT, field_predicate, value_graph};
use crate::iri::node_iri;
use crate::shacl::{Shapes, ShapesError, ValidationReport};
use crate::vocab as vt;

/// The pattern of `decimal` strings: `-?[0-9]+(\.[0-9]+)?`.
pub const DECIMAL_PATTERN: &str = r"^-?[0-9]+(\.[0-9]+)?$";

/// The pattern of RFC 3339 `date-time` strings with a required offset, including the day of
/// month (leap years) and second 60.
pub const DATETIME_PATTERN: &str = concat!(
    r"^(?:[0-9]{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12][0-9]|3[01])",
    r"|(?:0[469]|11)-(?:0[1-9]|[12][0-9]|30)|02-(?:0[1-9]|1[0-9]|2[0-8]))",
    r"|(?:[0-9]{2}(?:0[48]|[2468][048]|[13579][26])|(?:[02468][048]|[13579][26])00)-02-29)",
    r"[Tt](?:[01][0-9]|2[0-3]):[0-5][0-9]:(?:[0-5][0-9]|60)(?:\.[0-9]+)?",
    r"(?:[Zz]|[+-](?:[01][0-9]|2[0-3]):[0-5][0-9])$"
);

/// The pattern of IRIs with a scheme: no spaces, control characters or `<>"{}|\^` and
/// backquote, and well-formed percent-encodings.
pub const IRI_PATTERN: &str =
    r#"^[A-Za-z][A-Za-z0-9+.\-]*:(?:[^\x00-\x20\x7F<>"{}|\\^`%]|%[0-9A-Fa-f]{2})*$"#;

/// [`IRI_PATTERN`] in the regular expression syntax of `sh:pattern` (SHACL §4.6.1: XPath
/// `fn:matches`, XML Schema regular expressions), which has no `\x` escapes: the control
/// characters are `\p{Cc}` (U+0000 to U+001F and U+007F to U+009F) and the C1 controls U+0080
/// to U+009F, which an IRI may hold, are allowed again by a second branch. The same language
/// as [`IRI_PATTERN`], which stays the JSON Schema (ECMA-262) form.
pub const SHACL_IRI_PATTERN: &str =
    "^[A-Za-z][A-Za-z0-9+.\\-]*:(?:[^\\p{Cc} <>\"{}|\\\\^`%]|[\u{80}-\u{9F}]|%[0-9A-Fa-f]{2})*$";

fn sh(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{local}", vt::SH))
}

fn xsd(dt: &str) -> NamedNode {
    NamedNode::new_unchecked(dt)
}

fn int(n: usize) -> Literal {
    Literal::new_typed_literal(n.to_string(), NamedNodeRef::new_unchecked(vt::XSD_INTEGER))
}

/// Whether a field or payload of this type may be absent: it is `optional<T>` or `value`,
/// through named types.
pub(crate) fn may_be_absent(decls: &Declarations, ty: &Type) -> bool {
    let mut t = ty;
    for _ in 0..=decls.types.len() {
        match t {
            Type::Named(n) => match decls.lookup(n) {
                Some(d) => t = d,
                None => return false,
            },
            Type::Optional(_) | Type::Value => return true,
            _ => return false,
        }
    }
    false
}

/// A shapes graph under construction for the types of one declaration block.
pub struct TypeShapes<'a> {
    decls: &'a Declarations,
    iris: &'a TypeIris,
    graph: Graph,
}

impl<'a> TypeShapes<'a> {
    /// An empty shapes graph for types of `decls`, whose named types have `iris`.
    pub fn new(decls: &'a Declarations, iris: &'a TypeIris) -> Self {
        TypeShapes {
            decls,
            iris,
            graph: Graph::new(),
        }
    }

    /// Add the node shape of every named type that has an IRI, at its IRI.
    // @tg: implements:: [[openspec:type-system#SHACL Projection]]
    pub fn named_types(&mut self) {
        let iris = self.iris;
        for (name, iri) in iris.iter() {
            let Some(ty) = self.decls.lookup(name) else {
                continue;
            };
            let node = iri.named_node();
            self.add(
                &node,
                NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                sh("NodeShape"),
            );
            self.add(
                &node,
                NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                vt::NAMED_TYPE,
            );
            self.constrain(&node, iri.as_str(), "s", ty);
        }
    }

    /// Add a node shape for `ty` at `<base>#<fragment>` and return it. Named types `ty` uses
    /// must have IRIs; add their shapes with [`TypeShapes::named_types`].
    pub fn shape_at(&mut self, base: &str, fragment: &str, ty: &Type) -> NamedNode {
        let node = NamedNode::new_unchecked(node_iri(base, fragment));
        self.add(
            &node,
            NamedNodeRef::new_unchecked(vt::RDF_TYPE),
            sh("NodeShape"),
        );
        self.constrain(&node, base, fragment, ty);
        node
    }

    /// The shapes graph.
    pub fn into_graph(self) -> Graph {
        self.graph
    }

    fn add(&mut self, s: &NamedNode, p: impl Into<NamedNode>, o: impl Into<Term>) {
        let (p, o) = (p.into(), o.into());
        self.graph.insert(TripleRef::new(s, &p, &o));
    }

    /// A shape for `ty`: a named type's IRI, or a node shape minted at `<base>#<path>`.
    fn shape_ref(&mut self, base: &str, path: &str, ty: &Type) -> Term {
        if let Type::Named(n) = ty
            && let Some(iri) = self.iris.get(n)
        {
            return iri.named_node().into();
        }
        self.shape_at(base, path, ty).into()
    }

    /// An RDF collection of `items` with cells minted at `<base>#<path>.<i>`.
    fn list(&mut self, base: &str, path: &str, items: Vec<Term>) -> Term {
        let mut rest: Term = NamedNode::new_unchecked(vt::RDF_NIL).into();
        for (i, item) in items.into_iter().enumerate().rev() {
            let cell = NamedNode::new_unchecked(node_iri(base, &format!("{path}.{i}")));
            self.add(&cell, NamedNodeRef::new_unchecked(vt::RDF_FIRST), item);
            self.add(&cell, NamedNodeRef::new_unchecked(vt::RDF_REST), rest);
            rest = cell.into();
        }
        rest
    }

    /// A property shape at `<base>#<path>` on `predicate` with optional counts.
    fn property(
        &mut self,
        owner: &NamedNode,
        base: &str,
        path: &str,
        predicate: Term,
        min: Option<usize>,
        max: Option<usize>,
    ) -> NamedNode {
        let p = NamedNode::new_unchecked(node_iri(base, path));
        self.add(owner, sh("property"), p.clone());
        self.add(
            &p,
            NamedNodeRef::new_unchecked(vt::RDF_TYPE),
            sh("PropertyShape"),
        );
        self.add(&p, sh("path"), predicate);
        if let Some(n) = min {
            self.add(&p, sh("minCount"), int(n));
        }
        if let Some(n) = max {
            self.add(&p, sh("maxCount"), int(n));
        }
        p
    }

    fn datatype(&mut self, node: &NamedNode, dt: &str, pattern: Option<&str>) {
        self.add(node, sh("datatype"), xsd(dt));
        if let Some(p) = pattern {
            self.add(node, sh("pattern"), Literal::new_simple_literal(p));
        }
    }

    fn closed_blank(&mut self, node: &NamedNode) {
        self.add(node, sh("nodeKind"), sh("BlankNode"));
        self.add(node, sh("closed"), Literal::from(true));
    }

    /// Attach the constraints of `ty` to the shape `node`; nested shapes go under
    /// `<base>#<path>...`.
    fn constrain(&mut self, node: &NamedNode, base: &str, path: &str, ty: &Type) {
        match ty {
            Type::Value => {}
            Type::Named(n) => {
                if let Some(iri) = self.iris.get(n) {
                    self.add(node, sh("node"), iri.named_node());
                } else {
                    // A reference without an IRI cannot be checked: nothing conforms.
                    let empty = self.shape_at(base, &format!("{path}.unknown"), &Type::Value);
                    self.add(node, sh("not"), empty);
                }
            }
            Type::Primitive(p) => match p {
                Primitive::Bool => self.datatype(node, vt::XSD_BOOLEAN, None),
                Primitive::Int => self.datatype(node, vt::XSD_LONG, None),
                Primitive::String => self.datatype(node, vt::XSD_STRING, None),
                Primitive::Datetime => {
                    self.datatype(node, vt::XSD_STRING, Some(DATETIME_PATTERN));
                }
                Primitive::Iri => self.datatype(node, vt::XSD_STRING, Some(SHACL_IRI_PATTERN)),
                Primitive::Decimal => {
                    let mut alts = Vec::new();
                    for (i, (dt, pat)) in [
                        (vt::XSD_LONG, None),
                        (vt::XSD_DECIMAL, None),
                        (vt::XSD_STRING, Some(DECIMAL_PATTERN)),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let alt = NamedNode::new_unchecked(node_iri(base, &format!("{path}.d{i}")));
                        self.add(
                            &alt,
                            NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                            sh("NodeShape"),
                        );
                        self.datatype(&alt, dt, pat);
                        alts.push(alt.into());
                    }
                    let l = self.list(base, &format!("{path}.dor"), alts);
                    self.add(node, sh("or"), l);
                }
            },
            Type::Optional(inner) => {
                let null = NamedNode::new_unchecked(node_iri(base, &format!("{path}.null")));
                self.add(
                    &null,
                    NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                    sh("NodeShape"),
                );
                self.add(&null, sh("hasValue"), vt::NULL.into_owned());
                let some = self.shape_ref(base, &format!("{path}.o"), inner);
                let l = self.list(base, &format!("{path}.oor"), vec![null.into(), some]);
                self.add(node, sh("or"), l);
            }
            Type::List(item) => {
                let nil = NamedNode::new_unchecked(node_iri(base, &format!("{path}.nil")));
                self.add(
                    &nil,
                    NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                    sh("NodeShape"),
                );
                self.add(&nil, sh("hasValue"), NamedNode::new_unchecked(vt::RDF_NIL));
                let cons = NamedNode::new_unchecked(node_iri(base, &format!("{path}.cons")));
                self.add(
                    &cons,
                    NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                    sh("NodeShape"),
                );
                self.closed_blank(&cons);
                for (step, pred) in [("first", vt::RDF_FIRST), ("rest", vt::RDF_REST)] {
                    self.property(
                        &cons,
                        base,
                        &format!("{path}.cons.{step}"),
                        NamedNode::new_unchecked(pred).into(),
                        Some(1),
                        Some(1),
                    );
                }
                // Every member, by the vt:listMember triples of the encoding.
                let m = self.property(
                    &cons,
                    base,
                    &format!("{path}.cons.member"),
                    vt::LIST_MEMBER.into_owned().into(),
                    None,
                    None,
                );
                let s = self.shape_ref(base, &format!("{path}.l"), item);
                self.add(&m, sh("node"), s);
                let l = self.list(base, &format!("{path}.lor"), vec![nil.into(), cons.into()]);
                self.add(node, sh("or"), l);
            }
            Type::Record(fields) => {
                self.closed_blank(node);
                for f in fields {
                    let fpath = segment(path, "f", &f.name);
                    let min = (!may_be_absent(self.decls, &f.ty)).then_some(1);
                    let p = self.property(
                        node,
                        base,
                        &fpath,
                        field_predicate(&f.name).into(),
                        min,
                        Some(1),
                    );
                    let s = self.shape_ref(base, &format!("{fpath}.t"), &f.ty);
                    self.add(&p, sh("node"), s);
                }
            }
            Type::Union(variants) => {
                self.closed_blank(node);
                let tag: Term = field_predicate("tag").into();
                let value: Term = field_predicate("value").into();
                let t = self.property(
                    node,
                    base,
                    &format!("{path}.tag"),
                    tag.clone(),
                    Some(1),
                    Some(1),
                );
                self.add(&t, sh("datatype"), xsd(vt::XSD_STRING));
                self.property(
                    node,
                    base,
                    &format!("{path}.value"),
                    value.clone(),
                    None,
                    Some(1),
                );
                let mut alts = Vec::new();
                for v in variants {
                    let vpath = segment(path, "v", &v.tag);
                    let alt = NamedNode::new_unchecked(node_iri(base, &vpath));
                    self.add(
                        &alt,
                        NamedNodeRef::new_unchecked(vt::RDF_TYPE),
                        sh("NodeShape"),
                    );
                    let t =
                        self.property(&alt, base, &format!("{vpath}.tag"), tag.clone(), None, None);
                    self.add(&t, sh("hasValue"), Literal::new_simple_literal(&v.tag));
                    match &v.payload {
                        None => {
                            self.property(
                                &alt,
                                base,
                                &format!("{vpath}.value"),
                                value.clone(),
                                None,
                                Some(0),
                            );
                        }
                        Some(pt) => {
                            let min = (!may_be_absent(self.decls, pt)).then_some(1);
                            let p = self.property(
                                &alt,
                                base,
                                &format!("{vpath}.value"),
                                value.clone(),
                                min,
                                None,
                            );
                            let s = self.shape_ref(base, &format!("{vpath}.t"), pt);
                            self.add(&p, sh("node"), s);
                        }
                    }
                    alts.push(alt.into());
                }
                let l = self.list(base, &format!("{path}.vor"), alts);
                self.add(node, sh("or"), l);
            }
        }
    }
}

/// Why a value could not be validated against the SHACL projection of a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The type uses a named type that has no IRI (it is not declared, or uses an undeclared
    /// type).
    UnknownType(String),
    /// The generated shapes did not compile (a bug).
    Shapes(String),
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::UnknownType(n) => write!(f, "type `{n}` has no IRI"),
            ShapeError::Shapes(e) => write!(f, "generated shapes do not compile: {e}"),
        }
    }
}

impl std::error::Error for ShapeError {}

impl From<ShapesError> for ShapeError {
    fn from(e: ShapesError) -> Self {
        ShapeError::Shapes(e.0)
    }
}

/// The shapes graph checking that the value under [`VALUE_ROOT`] has type `ty`: the shapes of
/// every named type of `decls`, and a shape targeting the root.
pub fn value_shapes(decls: &Declarations, ty: &Type) -> Result<Graph, ShapeError> {
    let iris = TypeIris::of(decls);
    if let Some(r) = ty.references().into_iter().find(|r| iris.get(r).is_none()) {
        return Err(ShapeError::UnknownType(r.to_owned()));
    }
    let mut shapes = TypeShapes::new(decls, &iris);
    shapes.named_types();
    let base = "urn:vertep:value-shape";
    let root = NamedNode::new_unchecked(base);
    shapes.add(
        &root,
        NamedNodeRef::new_unchecked(vt::RDF_TYPE),
        sh("NodeShape"),
    );
    shapes.add(
        &root,
        sh("targetNode"),
        NamedNode::new_unchecked(VALUE_ROOT),
    );
    let p = shapes.property(
        &root,
        base,
        "p",
        NamedNode::new_unchecked(vt::RDF_VALUE).into(),
        Some(1),
        Some(1),
    );
    let s = shapes.shape_ref(base, "t", ty);
    shapes.add(&p, sh("node"), s);
    Ok(shapes.into_graph())
}

/// Validate the [RDF encoding](super::value) of `value` against the SHACL projection of `ty`
/// (named types resolved in `decls`). The report conforms exactly when
/// [`Declarations::check`] accepts the value.
// @tg: implements:: [[openspec:type-system#SHACL Projection]]
pub fn validate_value(
    decls: &Declarations,
    ty: &Type,
    value: &Value,
) -> Result<ValidationReport, ShapeError> {
    let shapes = Shapes::from_graph(&value_shapes(decls, ty)?)?;
    Ok(shapes.validate(&value_graph(value)))
}
