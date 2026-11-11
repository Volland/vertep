//! Conversion between `oxrdf` terms and tiramemsu values, and tiramemsu errors.

use std::collections::HashMap;

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, TermRef, Triple};
use tiramemsu::rdf::{RdfTerm, RdfTriple};
use tiramemsu::{Error, IntoObject, Obj, Tx, Value};
use vertep_store::StoreError;

/// Blank node labels of one write, mapped to the fresh tiramemsu blank nodes standing for them.
pub(crate) type BlankMap = HashMap<String, Obj>;

/// The tiramemsu value of an IRI.
pub(crate) fn iri(s: &str) -> Value {
    Value::iri(s)
}

/// The tiramemsu value of a literal.
pub(crate) fn literal(l: &Literal) -> Result<Value, StoreError> {
    if l.direction().is_some() {
        return Err(StoreError::Unsupported(
            "directional language strings on tiramemsu".into(),
        ));
    }
    Ok(Value::literal(
        l.value(),
        Some(l.datatype().as_str()),
        l.language(),
    ))
}

/// The value written for a term in a transaction. Blank nodes get fresh tiramemsu blank nodes,
/// one per label per write; triple terms are refused (tiramemsu cannot hold an unasserted
/// triple term outside SPARQL).
pub(crate) fn term_obj(
    tx: &mut Tx<'_>,
    t: TermRef<'_>,
    blanks: &mut BlankMap,
) -> Result<Obj, StoreError> {
    match t {
        TermRef::NamedNode(n) => Ok(iri(n.as_str()).into_obj()),
        TermRef::Literal(l) => Ok(literal(&l.into_owned())?.into_obj()),
        TermRef::BlankNode(b) => {
            if let Some(o) = blanks.get(b.as_str()) {
                return Ok(o.clone());
            }
            let id = tx.new_bnode().map_err(store_error)?;
            let o = id.into_obj();
            blanks.insert(b.as_str().to_owned(), o.clone());
            Ok(o)
        }
        TermRef::Triple(_) => Err(StoreError::Unsupported(
            "triple terms in journal entries or state statements on tiramemsu".into(),
        )),
    }
}

/// The `oxrdf` term of a stored value. Blank nodes keep a stable label (`b<n>`); anonymous
/// nodes, statements and transactions are tiramemsu's skolem IRIs.
pub(crate) fn value_term(v: &Value) -> Result<Term, StoreError> {
    match v {
        Value::BNode(n) => Ok(BlankNode::new_unchecked(format!("b{n}")).into()),
        other => rdf_term(&tiramemsu::rdf::render(other)),
    }
}

/// The `oxrdf` term of a tiramemsu RDF term.
pub(crate) fn rdf_term(t: &RdfTerm) -> Result<Term, StoreError> {
    Ok(match t {
        RdfTerm::Iri(s) => {
            if let Some(n) = s.strip_prefix(tiramemsu::vocab::SKOLEM_BNODE) {
                BlankNode::new_unchecked(format!("b{n}")).into()
            } else {
                NamedNode::new(s.as_str())
                    .map_err(StoreError::backend)?
                    .into()
            }
        }
        RdfTerm::Blank(b) => BlankNode::new(b.as_str())
            .map_err(StoreError::backend)?
            .into(),
        RdfTerm::Literal {
            lex,
            datatype,
            lang,
        } => match (lang, datatype) {
            (Some(l), _) => Literal::new_language_tagged_literal(lex.as_str(), l.as_str())
                .map_err(StoreError::backend)?
                .into(),
            (None, Some(dt)) => Literal::new_typed_literal(
                lex.as_str(),
                NamedNode::new(dt.as_str()).map_err(StoreError::backend)?,
            )
            .into(),
            (None, None) => Literal::new_simple_literal(lex.as_str()).into(),
        },
        RdfTerm::Triple(tr) => Term::Triple(Box::new(rdf_triple(tr)?)),
    })
}

/// The `oxrdf` triple of a tiramemsu RDF triple.
pub(crate) fn rdf_triple(t: &RdfTriple) -> Result<Triple, StoreError> {
    let s = match rdf_term(&t.s)? {
        Term::NamedNode(n) => NamedOrBlankNode::from(n),
        Term::BlankNode(b) => NamedOrBlankNode::from(b),
        other => {
            return Err(StoreError::Backend(format!(
                "a triple with subject {other}"
            )));
        }
    };
    let Term::NamedNode(p) = rdf_term(&t.p)? else {
        return Err(StoreError::Backend(
            "a triple with a non-IRI predicate".into(),
        ));
    };
    Ok(Triple::new(s, p, rdf_term(&t.o)?))
}

/// The store error of a tiramemsu error.
pub(crate) fn store_error(e: Error) -> StoreError {
    match e {
        Error::Parse { .. } | Error::InvalidQuery { .. } | Error::InvalidTerm { .. } => {
            StoreError::Invalid(e.to_string())
        }
        Error::Unsupported { .. } | Error::MissingCapability { .. } => {
            StoreError::Unsupported(e.to_string())
        }
        Error::PoolTimeout { .. } | Error::ImportInProgress => {
            StoreError::Unavailable(e.to_string())
        }
        other => StoreError::Backend(other.to_string()),
    }
}
