//! Chart version IRIs and the minting of node IRIs under them.
//!
//! A chart version IRI is `urn:vertep:chart:sha256:<hex>`, and every node of the chart is named
//! `<version>#<fragment>`, where the fragment is derived from the node's state identifier and
//! document position (see [`crate::project`]). Because the node IRIs are part of the hashed
//! graph, the hash is computed over the projection minted under the fixed base
//! [`PLACEHOLDER_BASE`]; the published graph is that projection rebased onto the version IRI
//! ([`rebase_graph`]). Verifying a graph's version therefore means rebasing it back onto the
//! placeholder and hashing again.

use core::fmt;

use oxrdf::{Graph, NamedNode, NamedOrBlankNode, Term, Triple};

/// The prefix of every chart version IRI.
pub const VERSION_PREFIX: &str = "urn:vertep:chart:sha256:";

/// The base the hashed projection is minted under, before the version is known.
pub const PLACEHOLDER_BASE: &str = "urn:vertep:chart:self";

/// The fragment of the chart node.
pub const CHART_FRAGMENT: &str = "chart";

/// The IRI naming one chart version: `urn:vertep:chart:sha256:` followed by the lowercase hex
/// SHA-256 digest of the canonical projection.
// @tg: implements:: [[openspec:chart-rdf#Chart Version IRI]]
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChartVersion {
    iri: String,
}

impl ChartVersion {
    /// The version for a SHA-256 digest.
    pub fn from_digest(digest: &[u8; 32]) -> Self {
        let mut iri = String::with_capacity(VERSION_PREFIX.len() + 64);
        iri.push_str(VERSION_PREFIX);
        for b in digest {
            iri.push(hex_digit(b >> 4));
            iri.push(hex_digit(b & 0xF));
        }
        ChartVersion { iri }
    }

    /// Parse a version IRI; `None` unless it is the prefix followed by 64 lowercase hex digits.
    pub fn parse(iri: &str) -> Option<Self> {
        let hex = iri.strip_prefix(VERSION_PREFIX)?;
        (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(
            || ChartVersion {
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
        &self.iri[VERSION_PREFIX.len()..]
    }

    /// The version IRI as an RDF named node (the TriG graph name of the chart).
    pub fn named_node(&self) -> NamedNode {
        NamedNode::new_unchecked(self.iri.clone())
    }

    /// The IRI of the chart node of this version.
    pub fn chart_node(&self) -> NamedNode {
        NamedNode::new_unchecked(node_iri(&self.iri, CHART_FRAGMENT))
    }
}

impl fmt::Display for ChartVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.iri)
    }
}

fn hex_digit(n: u8) -> char {
    char::from(if n < 10 { b'0' + n } else { b'a' + n - 10 })
}

/// `<base>#<fragment>`.
pub fn node_iri(base: &str, fragment: &str) -> String {
    let mut s = String::with_capacity(base.len() + 1 + fragment.len());
    s.push_str(base);
    s.push('#');
    s.push_str(fragment);
    s
}

/// The base of a chart node IRI (`<base>#chart`), or `None` if it does not have that form.
pub fn base_of_chart_node(iri: &str) -> Option<&str> {
    iri.strip_suffix(CHART_FRAGMENT)?.strip_suffix('#')
}

/// Append `s` with every byte outside the RFC 3986 unreserved set (`A-Z a-z 0-9 - . _ ~`)
/// percent-encoded in uppercase hex. The encoding is injective and never yields `/`, `:` or `#`, so
/// encoded identifiers can be used as IRI fragment segments.
pub fn percent_encode_into(s: &str, out: &mut String) {
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(b"0123456789ABCDEF"[usize::from(b >> 4)]));
            out.push(char::from(b"0123456789ABCDEF"[usize::from(b & 0xF)]));
        }
    }
}

/// Rename every IRI of the form `<from>#...` to `<to>#...`, leaving all other terms unchanged.
pub fn rebase_graph(graph: &Graph, from: &str, to: &str) -> Graph {
    let mut prefix = String::from(from);
    prefix.push('#');
    let mut out = Graph::new();
    for t in graph.iter() {
        out.insert(&rebase_triple(&t.into_owned(), &prefix, to));
    }
    out
}

fn rebase_iri(n: &NamedNode, prefix: &str, to: &str) -> NamedNode {
    match n.as_str().strip_prefix(prefix) {
        Some(rest) => NamedNode::new_unchecked(node_iri(to, rest)),
        None => n.clone(),
    }
}

fn rebase_triple(t: &Triple, prefix: &str, to: &str) -> Triple {
    let subject = match &t.subject {
        NamedOrBlankNode::NamedNode(n) => NamedOrBlankNode::NamedNode(rebase_iri(n, prefix, to)),
        other => other.clone(),
    };
    let object = match &t.object {
        Term::NamedNode(n) => Term::NamedNode(rebase_iri(n, prefix, to)),
        Term::Triple(inner) => Term::Triple(Box::new(rebase_triple(inner, prefix, to))),
        other => other.clone(),
    };
    Triple::new(subject, rebase_iri(&t.predicate, prefix, to), object)
}
