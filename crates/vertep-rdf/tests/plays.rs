//! `vt:plays`: a chart's protocol roles survive every surface and are projected as `vt:plays`.
#![allow(missing_docs)]

use oxrdf::{NamedNode, NamedNodeRef, TermRef};
use vertep_core::plays::{RolePlay, plays};
use vertep_rdf::vocab as vt;
use vertep_rdf::{chart_ontology, project, reconstruct};

const ID: &str =
    "urn:vertep:protocol:sha256:45790502561699ae0a09e8e6c58096fcf6de7eca7bb242f3c0ca7b329e287bc6";

fn seller_chart() -> String {
    format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" xmlns:vt="https://w3id.org/vertep/vt#"
       initial="idle" vt:plays="{ID}#seller {ID}#auditor">
  <state id="idle"><transition event="quote.request" target="quoting"/></state>
  <state id="quoting"/>
</scxml>"#
    )
}

// @lat: [[views/kolyada#Kolyada#Binding to charts and mailboxes]]
#[test]
fn plays_survives_scxml_yaml_and_rdf() {
    let chart = vertep_scxml::read_str(&seller_chart()).unwrap().chart;
    let expected = vec![
        RolePlay::parse(&format!("{ID}#auditor")).unwrap(),
        RolePlay::parse(&format!("{ID}#seller")).unwrap(),
    ];
    assert_eq!(plays(&chart).unwrap(), expected);

    let yaml = vertep_yaml::write(&chart).unwrap();
    let from_yaml = vertep_yaml::read_str(&yaml).unwrap();
    assert_eq!(plays(&from_yaml).unwrap(), expected);

    let projected = project(&chart);
    let chart_node = NamedNode::new(format!("{}#chart", projected.version)).unwrap();
    let mut roles: Vec<String> = projected
        .graph
        .objects_for_subject_predicate(&chart_node, vt::PLAYS)
        .filter_map(|o| match o {
            TermRef::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .collect();
    roles.sort();
    assert_eq!(roles, vec![format!("{ID}#auditor"), format!("{ID}#seller")]);

    let back = reconstruct(&projected.graph).unwrap().chart;
    assert_eq!(plays(&back).unwrap(), expected);
}

#[test]
fn plays_is_declared_in_the_chart_ontology() {
    let onto = chart_ontology();
    for p in [
        "http://www.w3.org/2000/01/rdf-schema#label",
        "http://www.w3.org/2000/01/rdf-schema#comment",
    ] {
        assert_eq!(
            onto.objects_for_subject_predicate(vt::PLAYS, NamedNodeRef::new_unchecked(p))
                .count(),
            1,
            "vt:plays {p}"
        );
    }
}

#[test]
fn malformed_plays_derives_nothing() {
    let chart = vertep_scxml::read_str(&seller_chart().replace(ID, "urn:x"))
        .unwrap()
        .chart;
    assert!(plays(&chart).is_err());
    let projected = project(&chart);
    assert_eq!(projected.graph.triples_for_predicate(vt::PLAYS).count(), 0);
    // The attribute itself is still kept as an extension node.
    assert!(reconstruct(&projected.graph).is_ok());
}
