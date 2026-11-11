//! Named types in RDF and their content-addressed IRIs.
//!
//! # Definition graph
//!
//! A named type `N = T` is a node `<B>` with `a vt:NamedType`, `vt:typeName "N"` and
//! `vt:definition` pointing at the term of `T`:
//!
//! | type | term |
//! |---|---|
//! | primitive, `value` | `vt:BoolType`, `vt:IntType`, ..., `vt:ValueType` |
//! | named type `M` | the type IRI of `M` |
//! | record | `<B#p>` `a vt:RecordType`, one `vt:typeField <B#p.f.name>` per field with `vt:fieldName` and `vt:fieldType` |
//! | union | `<B#p>` `a vt:UnionType`, one `vt:typeVariant <B#p.v.tag>` per variant with `vt:variantTag` and optional `vt:payloadType` |
//! | list | `<B#p>` `a vt:ListType`, `vt:itemType` |
//! | optional | `<B#p>` `a vt:OptionalType`, `vt:someType` |
//!
//! where `p` is the path from the definition root (`t`), extended with `.f.<name>`,
//! `.v.<tag>`, `.l` and `.o` (names percent-encoded). The graph has no blank nodes and fields
//! and variants are keyed by name, so source order is not part of a type.
//!
//! # Type IRIs
//!
//! The IRI of a named type is `urn:vertep:type:sha256:` followed by the hex SHA-256 of the
//! RDFC-1.0 canonical N-Quads of its definition minted under the placeholder base
//! `urn:vertep:type:self`, in which references to other named types are their type IRIs. A
//! type's IRI therefore covers its name, its structure and, transitively, every type it uses.
//! Types that refer to each other (a strongly connected group, possible through lists and
//! optionals) are hashed together: the graph of a member holds the definitions of the whole
//! group, the member itself under `urn:vertep:type:self` and every other member `M` under
//! `urn:vertep:type:self:M`, so each member gets its own IRI and a self-reference is the
//! placeholder itself.

use std::collections::BTreeMap;
use std::fmt;

use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, Term, TripleRef};
use sha2::{Digest, Sha256};
use vertep_core::types::{Declarations, Primitive, Type};

use crate::canon;
use crate::iri::{node_iri, percent_encode_into};
use crate::vocab as vt;

/// The prefix of every type IRI.
pub const TYPE_PREFIX: &str = "urn:vertep:type:sha256:";

/// The base a definition is minted under before its IRI is known.
pub const TYPE_PLACEHOLDER: &str = "urn:vertep:type:self";

/// The IRI of a named type: `urn:vertep:type:sha256:` and the lowercase hex SHA-256 of its
/// canonical definition.
// @tg: implements:: [[openspec:type-system#Named Content-Addressed Types]]
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeIri {
    iri: String,
}

impl TypeIri {
    /// The type IRI for a SHA-256 digest.
    pub fn from_digest(digest: &[u8; 32]) -> Self {
        let mut iri = String::with_capacity(TYPE_PREFIX.len() + 64);
        iri.push_str(TYPE_PREFIX);
        for b in digest {
            iri.push_str(&format!("{b:02x}"));
        }
        TypeIri { iri }
    }

    /// Parse a type IRI; `None` unless it is the prefix and 64 lowercase hex digits.
    pub fn parse(iri: &str) -> Option<Self> {
        let hex = iri.strip_prefix(TYPE_PREFIX)?;
        (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(
            || TypeIri {
                iri: iri.to_owned(),
            },
        )
    }

    /// The full IRI.
    pub fn as_str(&self) -> &str {
        &self.iri
    }

    /// The lowercase hex digest.
    pub fn hex(&self) -> &str {
        &self.iri[TYPE_PREFIX.len()..]
    }

    /// The IRI as an RDF named node.
    pub fn named_node(&self) -> NamedNode {
        NamedNode::new_unchecked(self.iri.clone())
    }

    /// The IRI as a borrowed RDF named node.
    pub fn as_ref(&self) -> NamedNodeRef<'_> {
        NamedNodeRef::new_unchecked(&self.iri)
    }
}

impl fmt::Display for TypeIri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.iri)
    }
}

/// Why a named type has no IRI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeIriError {
    /// The type uses, directly or through other types, a name that is not declared.
    UnknownType {
        /// The type that has no IRI.
        ty: String,
        /// The undeclared name.
        missing: String,
    },
}

impl fmt::Display for TypeIriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeIriError::UnknownType { ty, missing } => {
                write!(
                    f,
                    "type `{ty}` has no IRI: it uses undeclared type `{missing}`"
                )
            }
        }
    }
}

impl std::error::Error for TypeIriError {}

/// The IRIs of the named types of a declaration block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeIris {
    /// Named types with an IRI, in declaration order.
    order: Vec<(String, TypeIri)>,
    /// Types without an IRI and why.
    errors: Vec<TypeIriError>,
}

impl TypeIris {
    /// Derive the IRI of every named type of `decls` whose references all resolve. For a
    /// name declared twice the first declaration counts, as in [`Declarations::lookup`].
    pub fn of(decls: &Declarations) -> TypeIris {
        let mut names: Vec<&str> = Vec::new();
        for t in &decls.types {
            if !names.contains(&t.name.as_str()) {
                names.push(&t.name);
            }
        }
        let index: BTreeMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
        let defs: Vec<&Type> = names
            .iter()
            .map(|n| decls.lookup(n).unwrap_or(&Type::Value))
            .collect();
        let edges: Vec<Vec<usize>> = defs
            .iter()
            .map(|t| {
                t.references()
                    .iter()
                    .filter_map(|r| index.get(r).copied())
                    .collect()
            })
            .collect();

        // A type is resolvable when every name reachable from it is declared.
        let mut missing: Vec<Option<String>> = vec![None; names.len()];
        for (i, t) in defs.iter().enumerate() {
            missing[i] = t
                .references()
                .into_iter()
                .find(|r| !index.contains_key(r))
                .map(str::to_owned);
        }
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..names.len() {
                if missing[i].is_none()
                    && let Some(m) = edges[i].iter().find_map(|&j| missing[j].clone())
                {
                    missing[i] = Some(m);
                    changed = true;
                }
            }
        }

        let mut iris: Vec<Option<TypeIri>> = vec![None; names.len()];
        for scc in strongly_connected(&edges) {
            if scc.iter().any(|&i| missing[i].is_some()) {
                continue;
            }
            for &root in &scc {
                let mut g = Graph::new();
                for &m in &scc {
                    let base = member_base(names[m], m == root);
                    let resolve = |name: &str| -> Option<NamedNode> {
                        let j = *index.get(name)?;
                        if scc.contains(&j) {
                            Some(NamedNode::new_unchecked(member_base(names[j], j == root)))
                        } else {
                            iris[j].as_ref().map(TypeIri::named_node)
                        }
                    };
                    write_definition(&mut g, &base, names[m], defs[m], &resolve);
                }
                let nquads = match canon::canonical_nquads_of_graph(&g) {
                    Ok(n) => n,
                    Err(e) => unreachable!("type definitions have no blank nodes: {e}"),
                };
                let digest: [u8; 32] = Sha256::digest(nquads.as_bytes()).into();
                iris[root] = Some(TypeIri::from_digest(&digest));
            }
        }
        let mut out = TypeIris::default();
        for (i, n) in names.iter().enumerate() {
            match (&iris[i], &missing[i]) {
                (Some(t), _) => out.order.push(((*n).to_owned(), t.clone())),
                (None, Some(m)) => out.errors.push(TypeIriError::UnknownType {
                    ty: (*n).to_owned(),
                    missing: m.clone(),
                }),
                (None, None) => {}
            }
        }
        out
    }

    /// The IRI of the named type `name`.
    pub fn get(&self, name: &str) -> Option<&TypeIri> {
        self.order.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    /// The named types with an IRI, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &TypeIri)> {
        self.order.iter().map(|(n, t)| (n.as_str(), t))
    }

    /// Whether no named type has an IRI.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The named types that have no IRI, and why.
    pub fn errors(&self) -> &[TypeIriError] {
        &self.errors
    }

    /// The published definitions: each named type with an IRI, minted under that IRI, with
    /// references to other named types as their IRIs.
    pub fn definitions(&self, decls: &Declarations) -> Graph {
        let mut g = Graph::new();
        let resolve = |name: &str| self.get(name).map(TypeIri::named_node);
        for (name, iri) in self.iter() {
            if let Some(ty) = decls.lookup(name) {
                write_definition(&mut g, iri.as_str(), name, ty, &resolve);
            }
        }
        g
    }
}

/// The placeholder base of a member of the group being hashed.
fn member_base(name: &str, is_root: bool) -> String {
    let mut s = String::from(TYPE_PLACEHOLDER);
    if !is_root {
        s.push(':');
        percent_encode_into(name, &mut s);
    }
    s
}

/// Tarjan's algorithm; components come out dependencies first.
fn strongly_connected(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct St<'e> {
        edges: &'e [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(st: &mut St<'_>, v: usize) {
        st.index[v] = Some(st.next);
        st.low[v] = st.next;
        st.next += 1;
        st.stack.push(v);
        st.on_stack[v] = true;
        for &w in &st.edges[v] {
            match st.index[w] {
                None => {
                    visit(st, w);
                    st.low[v] = st.low[v].min(st.low[w]);
                }
                Some(iw) if st.on_stack[w] => st.low[v] = st.low[v].min(iw),
                Some(_) => {}
            }
        }
        if Some(st.low[v]) == st.index[v] {
            let mut scc = Vec::new();
            while let Some(w) = st.stack.pop() {
                st.on_stack[w] = false;
                scc.push(w);
                if w == v {
                    break;
                }
            }
            scc.sort_unstable();
            st.out.push(scc);
        }
    }
    let n = edges.len();
    let mut st = St {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if st.index[v].is_none() {
            visit(&mut st, v);
        }
    }
    st.out
}

/// The `vt:` individual of a primitive type.
pub(crate) fn primitive_term(p: Primitive) -> NamedNodeRef<'static> {
    match p {
        Primitive::Bool => vt::BOOL_TYPE,
        Primitive::Int => vt::INT_TYPE,
        Primitive::Decimal => vt::DECIMAL_TYPE,
        Primitive::String => vt::STRING_TYPE,
        Primitive::Datetime => vt::DATETIME_TYPE,
        Primitive::Iri => vt::IRI_TYPE,
    }
}

/// Write the definition of the named type `name = ty` under `base`. `resolve` names the
/// node of a referenced named type; an unresolved reference is written as `vt:ValueType`,
/// which never happens for types that have an IRI.
pub(crate) fn write_definition(
    g: &mut Graph,
    base: &str,
    name: &str,
    ty: &Type,
    resolve: &dyn Fn(&str) -> Option<NamedNode>,
) {
    let node = NamedNode::new_unchecked(base);
    g.insert(TripleRef::new(&node, super::rdf_type(), vt::NAMED_TYPE));
    g.insert(TripleRef::new(
        &node,
        vt::TYPE_NAME,
        &Literal::new_simple_literal(name),
    ));
    let def = type_term(g, base, "t", ty, resolve);
    g.insert(TripleRef::new(&node, vt::DEFINITION, &def));
}

fn type_term(
    g: &mut Graph,
    base: &str,
    path: &str,
    ty: &Type,
    resolve: &dyn Fn(&str) -> Option<NamedNode>,
) -> Term {
    let mint = |g: &mut Graph, class: NamedNodeRef<'_>| {
        let n = NamedNode::new_unchecked(node_iri(base, path));
        g.insert(TripleRef::new(&n, super::rdf_type(), class));
        n
    };
    match ty {
        Type::Primitive(p) => primitive_term(*p).into_owned().into(),
        Type::Value => vt::VALUE_TYPE.into_owned().into(),
        Type::Named(n) => resolve(n)
            .unwrap_or_else(|| vt::VALUE_TYPE.into_owned())
            .into(),
        Type::Record(fields) => {
            let n = mint(g, vt::RECORD_TYPE);
            for f in fields {
                let fpath = segment(path, "f", &f.name);
                let fnode = NamedNode::new_unchecked(node_iri(base, &fpath));
                g.insert(TripleRef::new(&n, vt::TYPE_FIELD, &fnode));
                g.insert(TripleRef::new(
                    &fnode,
                    super::rdf_type(),
                    vt::TYPE_FIELD_NODE,
                ));
                g.insert(TripleRef::new(
                    &fnode,
                    vt::FIELD_NAME,
                    &Literal::new_simple_literal(&f.name),
                ));
                let t = type_term(g, base, &format!("{fpath}.t"), &f.ty, resolve);
                g.insert(TripleRef::new(&fnode, vt::FIELD_TYPE, &t));
            }
            n.into()
        }
        Type::Union(variants) => {
            let n = mint(g, vt::UNION_TYPE);
            for v in variants {
                let vpath = segment(path, "v", &v.tag);
                let vnode = NamedNode::new_unchecked(node_iri(base, &vpath));
                g.insert(TripleRef::new(&n, vt::TYPE_VARIANT, &vnode));
                g.insert(TripleRef::new(
                    &vnode,
                    super::rdf_type(),
                    vt::TYPE_VARIANT_NODE,
                ));
                g.insert(TripleRef::new(
                    &vnode,
                    vt::VARIANT_TAG,
                    &Literal::new_simple_literal(&v.tag),
                ));
                if let Some(p) = &v.payload {
                    let t = type_term(g, base, &format!("{vpath}.t"), p, resolve);
                    g.insert(TripleRef::new(&vnode, vt::PAYLOAD_TYPE, &t));
                }
            }
            n.into()
        }
        Type::List(item) => {
            let n = mint(g, vt::LIST_TYPE);
            let t = type_term(g, base, &format!("{path}.l"), item, resolve);
            g.insert(TripleRef::new(&n, vt::ITEM_TYPE, &t));
            n.into()
        }
        Type::Optional(inner) => {
            let n = mint(g, vt::OPTIONAL_TYPE);
            let t = type_term(g, base, &format!("{path}.o"), inner, resolve);
            g.insert(TripleRef::new(&n, vt::SOME_TYPE, &t));
            n.into()
        }
    }
}

/// `<path>.<kind>.<percent-encoded name>`.
pub(crate) fn segment(path: &str, kind: &str, name: &str) -> String {
    let mut s = format!("{path}.{kind}.");
    percent_encode_into(name, &mut s);
    s
}
