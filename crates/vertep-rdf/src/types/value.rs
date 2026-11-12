//! The RDF encoding of runtime values, for validation against the SHACL projection of types.
//!
//! The encoding depends only on the value, never on a type, so a value that does not fit a type
//! still has an encoding for the shapes to reject:
//!
//! | value | RDF |
//! |---|---|
//! | `true`, `false` | `xsd:boolean` literal |
//! | a number that is integral and in the signed 64-bit range | `xsd:long` literal |
//! | any other finite number | `xsd:decimal` literal |
//! | a non-finite number | `xsd:double` literal (`NaN`, `INF`, `-INF`) |
//! | a string | `xsd:string` literal |
//! | `null`, `undefined` | `vt:Null` |
//! | an array | an RDF collection (`rdf:nil` when empty) whose first cell also has one `vt:listMember` triple per item |
//! | an object | a blank node with one triple per entry whose value is not `undefined`, predicate `<https://w3id.org/vertep/field#name>` |
//!
//! The `vt:listMember` triples let a SHACL shape reach every member of a list with a predicate
//! path: the path `rdf:rest*/rdf:first` would need blank nodes in the shapes (SHACL §2.3.1 reads
//! an IRI as a predicate path), and the type shapes in a chart projection are blank-node free.
//! The collection stays next to them because the member triples are a set: it keeps the order
//! and the repeated items of the list.
//!
//! [`value_graph`] puts the encoding under the fixed node [`VALUE_ROOT`] with `rdf:value`.

use oxrdf::{BlankNode, Graph, Literal, NamedNode, NamedNodeRef, Term, TripleRef};
use vertep_core::datamodel::Value;
use vertep_core::types::prim::f64_to_i64;

use crate::iri::percent_encode_into;
use crate::vocab as vt;

/// The node whose `rdf:value` is the encoded value in [`value_graph`].
pub const VALUE_ROOT: &str = "urn:vertep:value";

/// The predicate of record field (object key) `name`.
pub fn field_predicate(name: &str) -> NamedNode {
    let mut s = String::from(vt::FIELD_NS);
    percent_encode_into(name, &mut s);
    NamedNode::new_unchecked(s)
}

/// The RDF encoding of `value`: a graph holding `<urn:vertep:value> rdf:value <encoding>` and
/// the triples of the encoding.
// @tg: implements:: [[openspec:type-system#SHACL Projection]]
pub fn value_graph(value: &Value) -> Graph {
    let mut g = Graph::new();
    let root = NamedNode::new_unchecked(VALUE_ROOT);
    let v = encode(&mut g, value);
    g.insert(TripleRef::new(
        &root,
        NamedNodeRef::new_unchecked(vt::RDF_VALUE),
        &v,
    ));
    g
}

/// Encode `value` into `g` and return the term standing for it.
pub fn encode(g: &mut Graph, value: &Value) -> Term {
    let typed =
        |s: String, dt: &str| Literal::new_typed_literal(s, NamedNodeRef::new_unchecked(dt));
    match value {
        Value::Undefined | Value::Null => vt::NULL.into_owned().into(),
        Value::Bool(b) => Literal::from(*b).into(),
        Value::Number(n) if n.is_finite() => match f64_to_i64(*n) {
            Some(i) => typed(i.to_string(), vt::XSD_LONG).into(),
            None => typed(format!("{n}"), vt::XSD_DECIMAL).into(),
        },
        Value::Number(n) => {
            let lex = if n.is_nan() {
                "NaN"
            } else if *n > 0.0 {
                "INF"
            } else {
                "-INF"
            };
            typed(lex.to_owned(), vt::XSD_DOUBLE).into()
        }
        Value::String(s) => Literal::new_simple_literal(s).into(),
        Value::Array(items) => {
            let members: Vec<Term> = items.iter().map(|item| encode(g, item)).collect();
            let mut rest: Term = NamedNode::new_unchecked(vt::RDF_NIL).into();
            for first in members.iter().rev() {
                let cell = BlankNode::default();
                g.insert(TripleRef::new(
                    &cell,
                    NamedNodeRef::new_unchecked(vt::RDF_FIRST),
                    first,
                ));
                g.insert(TripleRef::new(
                    &cell,
                    NamedNodeRef::new_unchecked(vt::RDF_REST),
                    &rest,
                ));
                rest = cell.into();
            }
            if let Term::BlankNode(head) = &rest {
                for member in &members {
                    g.insert(TripleRef::new(head, vt::LIST_MEMBER, member));
                }
            }
            rest
        }
        Value::Object(entries) => {
            let node = BlankNode::default();
            for (k, v) in entries {
                if matches!(v, Value::Undefined) {
                    continue;
                }
                let o = encode(g, v);
                g.insert(TripleRef::new(&node, &field_predicate(k), &o));
            }
            node.into()
        }
    }
}
