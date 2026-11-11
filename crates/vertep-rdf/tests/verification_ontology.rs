//! The verification module declares every term the Lean `vt-certify` checker emits.

use oxrdf::{NamedNodeRef, vocab::rdf};
use vertep_rdf::io::{RdfFormat, read_graph};

#[test]
fn verification_module_declares_certificate_terms() {
    let g = read_graph(
        vertep_rdf::VERIFICATION_ONTOLOGY_TTL.as_bytes(),
        RdfFormat::Turtle,
    )
    .expect("module parses");
    let owl = |t: &str| format!("http://www.w3.org/2002/07/owl#{t}");
    let class = owl("Class");
    let obj = owl("ObjectProperty");
    let data = owl("DatatypeProperty");
    for (term, kind) in [
        ("Certificate", &class),
        ("certifiedBy", &obj),
        ("theorem", &data),
        ("leanToolchain", &data),
        ("leanRdfRevision", &data),
        ("checkerHash", &data),
    ] {
        let iri = format!("https://w3id.org/vertep/vt#{term}");
        assert!(
            g.contains(oxrdf::TripleRef::new(
                NamedNodeRef::new(&iri).unwrap(),
                rdf::TYPE,
                NamedNodeRef::new(kind).unwrap()
            )),
            "vt:{term} is declared as {kind}"
        );
    }
}
