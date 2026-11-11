//! One test per scenario of the `chart-rdf` capability
//! (`openspec/changes/m0-scxml-core/specs/chart-rdf/spec.md`).

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, Term, TermRef, TripleRef};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, TestRunner};
use vertep_core::ir::*;
use vertep_rdf::io::{read_dataset, read_graph, write_graph};
use vertep_rdf::vocab::{self as vt, NodeKind};
use vertep_rdf::*;
use vertep_testkit::{ChartConfig, arb_any_chart_with, arb_chart_with};

const OWL_CLASS: &str = "http://www.w3.org/2002/07/owl#Class";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const RDFS_COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";

fn rdf_type() -> NamedNodeRef<'static> {
    NamedNodeRef::new_unchecked(vt::RDF_TYPE)
}

fn node(version: &ChartVersion, fragment: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{version}#{fragment}"))
}

fn objects(g: &Graph, s: &NamedNode, p: NamedNodeRef<'_>) -> Vec<Term> {
    g.objects_for_subject_predicate(s, p)
        .map(TermRef::into_owned)
        .collect()
}

fn one_node(g: &Graph, s: &NamedNode, p: NamedNodeRef<'_>) -> NamedNode {
    match objects(g, s, p).as_slice() {
        [Term::NamedNode(n)] => n.clone(),
        other => panic!("{s} {p}: expected one IRI, got {other:?}"),
    }
}

fn round_trip(c: &Chart) -> Chart {
    let p = project(c);
    reconstruct(&p.graph)
        .unwrap_or_else(|e| panic!("{e}"))
        .chart
}

// ------------------------------------------------------------------ Chart Ontology Module

// @tg: verifies:: [[openspec:chart-rdf#Chart Ontology Module#Every IR node kind has a class]] {level: unit}
#[test]
fn every_ir_node_kind_has_a_class() {
    let onto = chart_ontology();
    let declared: BTreeSet<String> = onto
        .subjects_for_predicate_object(rdf_type(), NamedNodeRef::new_unchecked(OWL_CLASS))
        .map(|s| s.to_string())
        .collect();
    let kinds: BTreeSet<String> = NodeKind::ALL
        .iter()
        .map(|k| k.class().to_string())
        .collect();
    assert_eq!(
        kinds.len(),
        NodeKind::ALL.len(),
        "two node kinds share a class"
    );
    assert_eq!(declared, kinds, "classes and IR node kinds differ");

    let ir_kind = NamedNodeRef::new_unchecked("https://w3id.org/vertep/vt#irNodeKind");
    let mut ir_names = BTreeSet::new();
    for k in NodeKind::ALL {
        let c = k.class().into_owned();
        for p in [RDFS_LABEL, RDFS_COMMENT] {
            assert_eq!(
                objects(&onto, &c, NamedNodeRef::new_unchecked(p)).len(),
                1,
                "{c} {p}"
            );
        }
        let names = objects(&onto, &c, ir_kind);
        assert_eq!(names.len(), 1, "{c} names one IR node kind");
        assert!(
            ir_names.insert(names[0].to_string()),
            "{c}: IR node kind named twice"
        );
    }

    // Every class and property the projection of a chart using every node kind emits is
    // declared with a label and a definition.
    let g = project(&common::kitchen_sink()).graph;
    let mut used_classes = BTreeSet::new();
    let mut used_props = BTreeSet::new();
    for t in g.iter() {
        if t.predicate == rdf_type() {
            used_classes.insert(t.object.to_string());
        } else {
            used_props.insert(t.predicate.into_owned());
        }
    }
    assert_eq!(used_classes, kinds, "the kitchen sink uses every class");
    for p in used_props {
        assert!(p.as_str().starts_with(vt::VT), "{p} is a vt: property");
        for a in [RDFS_LABEL, RDFS_COMMENT] {
            assert_eq!(
                objects(&onto, &p, NamedNodeRef::new_unchecked(a)).len(),
                1,
                "{p} {a}"
            );
        }
    }
}

// @tg: verifies:: [[openspec:chart-rdf#Chart Ontology Module#Ontology parses]] {level: unit}
#[test]
fn ontology_parses() {
    let g = read_graph(shapes::CHART_ONTOLOGY_TTL.as_bytes(), RdfFormat::Turtle)
        .expect("the ontology is valid RDF 1.2 Turtle");
    assert!(g.len() > 300, "{} triples", g.len());
    let shapes_graph = read_graph(shapes::CHART_SHAPES_TTL.as_bytes(), RdfFormat::Turtle)
        .expect("the shapes are valid RDF 1.2 Turtle");
    assert!(!shapes_graph.is_empty());
    assert!(chart_shapes().targeted_shapes().len() > 20);
    // The module imports nothing.
    let imports = NamedNodeRef::new_unchecked("http://www.w3.org/2002/07/owl#imports");
    assert_eq!(g.triples_for_predicate(imports).count(), 0);
}

// ------------------------------------------------------------------ IR To RDF Projection

// @tg: verifies:: [[openspec:chart-rdf#IR To RDF Projection#State and transition triples]] {level: unit}
#[test]
fn state_and_transition_triples() {
    let mut a = common::atomic("a");
    a.transitions.push(common::transition(&["go"], &["b"]));
    let c = common::chart("null", vec![a, common::atomic("b")]);
    let p = project(&c);
    let g = &p.graph;
    let (a, b, t) = (
        node(&p.version, "s.a"),
        node(&p.version, "s.b"),
        node(&p.version, "s.a:t0"),
    );
    assert!(g.contains(TripleRef::new(&a, rdf_type(), vt::STATE)));
    assert!(g.contains(TripleRef::new(&a, rdf_type(), vt::STATE_NODE)));
    assert!(g.contains(TripleRef::new(&a, vt::TRANSITION_P, &t)));
    assert!(g.contains(TripleRef::new(&t, rdf_type(), vt::TRANSITION)));
    assert!(g.contains(TripleRef::new(&t, vt::SOURCE, &a)));
    let target = one_node(g, &t, vt::TARGET);
    assert_eq!(one_node(g, &target, vt::STATE_P), b);
    let event = one_node(g, &t, vt::EVENT);
    assert_eq!(
        objects(g, &event, vt::VALUE),
        vec![Term::from(Literal::new_simple_literal("go"))]
    );
    // No blank nodes anywhere.
    assert!(
        g.iter()
            .all(|t| !t.subject.is_blank_node() && !t.object.is_blank_node())
    );
}

// @tg: verifies:: [[openspec:chart-rdf#IR To RDF Projection#Deterministic node IRIs]] {level: unit}
#[test]
fn deterministic_node_iris() {
    let c = common::kitchen_sink();
    let (p1, p2) = (project(&c), project(&c.clone()));
    assert_eq!(p1.version, p2.version);
    assert_eq!(p1.graph, p2.graph);
    let iris =
        |g: &Graph| -> BTreeSet<String> { g.iter().map(|t| t.subject.to_string()).collect() };
    assert_eq!(iris(&p1.graph), iris(&p2.graph));
    assert_eq!(
        write_chart(&p1, RdfFormat::NTriples),
        write_chart(&p2, RdfFormat::NTriples)
    );
}

// ------------------------------------------------------------------ RDF To IR Reconstruction

// @tg: verifies:: [[openspec:chart-rdf#RDF To IR Reconstruction#Conforming graph reconstructed]] {level: unit}
#[test]
fn conforming_graph_reconstructed() {
    for c in [common::door(), common::kitchen_sink()] {
        let p = project(&c);
        let r = reconstruct(&p.graph).expect("a projection reconstructs");
        assert_eq!(r.chart, c);
        assert_eq!(r.base, p.version.as_str());
    }
}

// @tg: verifies:: [[openspec:chart-rdf#RDF To IR Reconstruction#Missing required triple]] {level: unit}
#[test]
fn missing_required_triple() {
    let p = project(&common::door());
    let t = node(&p.version, "s.closed:t0");
    let source = node(&p.version, "s.closed");
    let mut g = p.graph.clone();
    assert!(g.remove(TripleRef::new(&t, vt::SOURCE, &source)));
    let Err(ReconstructError::Invalid(report)) = reconstruct(&g) else {
        panic!("a graph without the transition source must not reconstruct");
    };
    assert!(!report.conforms());
    assert!(
        report
            .results
            .iter()
            .any(|r| r.focus_node == Term::from(t.clone())
                && r.path
                    .as_ref()
                    .is_some_and(|p| p.to_string() == "vt:source")
                && r.component == "MinCountConstraintComponent"),
        "the report names the transition and vt:source: {report}"
    );
}

// ------------------------------------------------------------------ Round Trip Identity

// @tg: verifies:: [[openspec:chart-rdf#Round Trip Identity#Conformance corpus round trip]] {level: unit}
#[test]
fn conformance_corpus_round_trip() {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vertep-conformance/suite");
    let options = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    let mut files = Vec::new();
    for dm in ["ecmascript", "null"] {
        for test in std::fs::read_dir(suite.join(dm)).expect("corpus present") {
            for f in std::fs::read_dir(test.unwrap().path()).unwrap() {
                let f = f.unwrap().path();
                if f.extension().is_some_and(|e| e == "scxml") {
                    files.push(f);
                }
            }
        }
    }
    files.sort();
    let (mut checked, mut unread) = (0, Vec::new());
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let Ok(parsed) = vertep_scxml::read(&text, &options) else {
            unread.push(f.display().to_string());
            continue;
        };
        let c = parsed.chart;
        let p = project(&c);
        if vertep_core::validate::validate(&c).is_ok() {
            assert!(validate(&p.graph).conforms(), "{}", f.display());
        }
        // Every corpus chart round-trips, well-formed or not: reconstruction is checked here
        // on the projection directly, without the shapes gate for the few deliberately invalid
        // charts of the IRP suite.
        match reconstruct(&p.graph) {
            Ok(r) => assert_eq!(r.chart, c, "{}", f.display()),
            Err(ReconstructError::Invalid(_)) => {
                assert!(
                    vertep_core::validate::validate(&c).is_err(),
                    "{}",
                    f.display()
                );
            }
            Err(e) => panic!("{}: {e}", f.display()),
        }
        checked += 1;
    }
    println!(
        "corpus: {checked} charts round-tripped, {} not readable: {unread:?}",
        unread.len()
    );
    assert!(
        checked >= 200,
        "only {checked} of {} corpus charts read",
        files.len()
    );
}

// @tg: verifies:: [[openspec:chart-rdf#Round Trip Identity#Extensions survive the round trip]] {level: unit}
#[test]
fn extensions_survive_the_round_trip() {
    let mut c = common::door();
    c.extensions = vec![
        common::ext_attr("owner", "ops"),
        common::ext_elem(
            "urn:foreign",
            "doc",
            "<f:doc xmlns:f=\"urn:foreign\">x</f:doc>",
            0,
        ),
        common::ext_elem(
            VT_NS,
            "meta",
            "<vt:meta xmlns:vt=\"https://w3id.org/vertep/vt#\"/>",
            4,
        ),
    ];
    let closed = &mut c.states[0];
    closed.extensions.push(common::ext_attr("color", "red"));
    closed.extensions.push(Extension {
        ns: "http://example.org/ns#".into(),
        local: "weight".into(),
        value: ExtensionValue::Attribute("3".into()),
        position: 0,
        span: Span::new(4, 9),
    });
    let t = &mut closed.transitions[0];
    t.extensions.push(common::ext_elem(
        "urn:foreign",
        "note",
        "<f:note xmlns:f=\"urn:foreign\"/>",
        2,
    ));
    t.body
        .push(Exec::new(ExecKind::Raise { event: "a".into() }));
    t.body.push(Exec::new(ExecKind::Extension(common::ext_elem(
        "urn:foreign",
        "audit",
        "<f:audit xmlns:f=\"urn:foreign\"/>",
        1,
    ))));
    let back = round_trip(&c);
    assert_eq!(back, c);
    assert_eq!(back.extensions, c.extensions);
    assert_eq!(back.states[0].extensions, c.states[0].extensions);
    assert_eq!(
        back.states[0].transitions[0].extensions,
        c.states[0].transitions[0].extensions
    );
    assert!(matches!(
        &back.states[0].transitions[0].body[1].kind,
        ExecKind::Extension(x) if x.local == "audit" && x.position == 1
    ));
}

// ------------------------------------------------------------------ Explicit Ordering

/// A deterministic shuffle (xorshift) of N-Triples lines.
fn shuffled_ntriples(p: &ProjectedChart, seed: u64) -> String {
    let nt = write_chart(p, RdfFormat::NTriples);
    let mut lines: Vec<&str> = nt.lines().collect();
    let mut x = seed;
    for i in (1..lines.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        lines.swap(i, (x % (i as u64 + 1)) as usize);
    }
    lines.join("\n")
}

// @tg: verifies:: [[openspec:chart-rdf#Explicit Ordering#Transition priority recoverable]] {level: unit}
#[test]
fn transition_priority_recoverable() {
    let mut s = common::atomic("s");
    for (i, e) in ["e5", "e1", "e4", "e2", "e3", "e0"].iter().enumerate() {
        let mut t = common::transition(&[e], &["u"]);
        t.cond = Some(format!("c{i}"));
        s.transitions.push(t);
    }
    let c = common::chart("ecmascript", vec![s, common::atomic("u")]);
    let p = project(&c);
    for seed in [1, 7, 42, 0xDEAD_BEEF] {
        let text = shuffled_ntriples(&p, seed);
        assert_ne!(text, write_chart(&p, RdfFormat::NTriples));
        let back = read_chart(text.as_bytes(), RdfFormat::NTriples)
            .unwrap()
            .chart;
        let events: Vec<&str> = back.states[0]
            .transitions
            .iter()
            .map(|t| t.events[0].as_str())
            .collect();
        assert_eq!(events, ["e5", "e1", "e4", "e2", "e3", "e0"]);
        assert_eq!(back, c);
    }
}

// @tg: verifies:: [[openspec:chart-rdf#Explicit Ordering#Target order recoverable]] {level: unit}
#[test]
fn target_order_recoverable() {
    let mut par = State::new("p", StateElement::Parallel);
    par.children = vec![common::atomic("p.a"), common::atomic("p.b")];
    let mut s = common::atomic("s");
    s.transitions.push(common::transition(&[], &["p.b", "p.a"]));
    let c = common::chart("null", vec![s, par]);
    let p = project(&c);
    let t = node(&p.version, "s.s:t0");
    let mut by_order: Vec<(String, String)> = objects(&p.graph, &t, vt::TARGET)
        .into_iter()
        .map(|r| {
            let Term::NamedNode(r) = r else { panic!() };
            let Term::Literal(o) = &objects(&p.graph, &r, vt::ORDER)[0] else {
                panic!()
            };
            let state = one_node(&p.graph, &r, vt::STATE_P);
            let Term::Literal(id) = &objects(&p.graph, &state, vt::ID)[0] else {
                panic!()
            };
            (o.value().to_owned(), id.value().to_owned())
        })
        .collect();
    by_order.sort();
    assert_eq!(
        by_order,
        [("0".into(), "p.b".into()), ("1".into(), "p.a".into())]
    );
    let back = read_chart(shuffled_ntriples(&p, 3).as_bytes(), RdfFormat::NTriples).unwrap();
    assert_eq!(back.chart.states[0].transitions[0].targets, ["p.b", "p.a"]);
}

// ------------------------------------------------------------------ Executable Content As Literals

// @tg: verifies:: [[openspec:chart-rdf#Executable Content As Literals#Condition literal typed by datamodel]] {level: unit}
#[test]
fn condition_literal_typed_by_datamodel() {
    let mut s = common::atomic("s");
    let mut t = common::transition(&["e"], &["s"]);
    t.cond = Some("x > 1".into());
    s.transitions.push(t);
    let c = common::chart("ecmascript", vec![s]);
    let p = project(&c);
    let cond = objects(&p.graph, &node(&p.version, "s.s:t0"), vt::COND);
    let expected = Literal::new_typed_literal(
        "x > 1",
        NamedNodeRef::new_unchecked("https://w3id.org/vertep/vt#ecmascriptExpr"),
    );
    assert_eq!(cond, vec![Term::from(expected)]);

    // The same text under another datamodel is a different literal.
    let mut other = c.clone();
    other.datamodel = "null".into();
    let q = project(&other);
    let cond = objects(&q.graph, &node(&q.version, "s.s:t0"), vt::COND);
    let Term::Literal(l) = &cond[0] else { panic!() };
    assert_eq!(l.datatype().as_str(), "https://w3id.org/vertep/vt#nullExpr");
    assert_ne!(p.version, q.version);
}

// @tg: verifies:: [[openspec:chart-rdf#Executable Content As Literals#Inline content preserved exactly]] {level: unit}
#[test]
fn inline_content_preserved_exactly() {
    let xml = "\n  <list xmlns=\"urn:x\" a='1'>\r\n\t<i>&amp;&lt;</i><!-- c --><![CDATA[ ]]>\u{0}é中😀\"\\</list>  ";
    let mut c = common::door();
    c.data.push(Data {
        id: "d".into(),
        src: None,
        expr: None,
        content: Some(xml.into()),
        extensions: vec![],
        span: Span::NONE,
    });
    let p = project(&c);
    let d = node(&p.version, "chart:d0");
    assert_eq!(
        objects(&p.graph, &d, vt::INLINE_CONTENT),
        vec![Term::from(Literal::new_simple_literal(xml))]
    );
    for format in RdfFormat::ALL {
        let text = write_chart(&p, format);
        let back =
            read_chart(text.as_bytes(), format).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(
            back.chart.data[0].content.as_deref(),
            Some(xml),
            "{format:?}"
        );
    }
}

// ------------------------------------------------------------------ SHACL Shapes For Charts

// @tg: verifies:: [[openspec:chart-rdf#SHACL Shapes For Charts#Well-formed chart conforms]] {level: unit}
#[test]
fn well_formed_chart_conforms() {
    for c in [common::door(), common::kitchen_sink()] {
        assert_eq!(vertep_core::validate::validate(&c), Ok(()));
        let report = validate(&project(&c).graph);
        assert!(report.conforms(), "{report}");
    }
    // Generated well-formed charts. The generator does not keep data identifiers unique,
    // which the chart model requires, so charts the IR validator rejects are skipped.
    let mut runner = TestRunner::new(Config::with_cases(1));
    let strategy = arb_chart_with(small());
    let mut checked = 0;
    for _ in 0..80 {
        let c = strategy.new_tree(&mut runner).unwrap().current();
        if vertep_core::validate::validate(&c).is_err() {
            continue;
        }
        let report = validate(&project(&c).graph);
        assert!(report.conforms(), "{report}\n{c:#?}");
        checked += 1;
    }
    assert!(
        checked >= 40,
        "only {checked} generated charts were well-formed"
    );
}

/// Generator limits that keep debug-build property tests fast.
fn small() -> ChartConfig {
    ChartConfig {
        max_depth: 2,
        max_top_states: 3,
        max_children: 3,
        max_transitions: 2,
        max_block: 2,
        exec_depth: 1,
        ..ChartConfig::default()
    }
}

// @tg: verifies:: [[openspec:chart-rdf#SHACL Shapes For Charts#Ill-formed graph rejected]] {level: unit}
#[test]
fn ill_formed_graph_rejected() {
    let p = project(&common::door());
    let t = node(&p.version, "s.closed:t0");
    let r = one_node(&p.graph, &t, vt::TARGET);
    let open = node(&p.version, "s.open");
    for not_a_state in [
        node(&p.version, "chart"),
        node(&p.version, "s.closed:t0:ev0"),
        NamedNode::new_unchecked("urn:example:elsewhere"),
    ] {
        let mut g = p.graph.clone();
        g.remove(TripleRef::new(&r, vt::STATE_P, &open));
        g.insert(TripleRef::new(&r, vt::STATE_P, &not_a_state));
        let report = validate(&g);
        assert!(!report.conforms());
        assert!(
            report
                .results
                .iter()
                .any(|res| res.focus_node == Term::from(t.clone())
                    && res.value == Some(Term::from(r.clone()))),
            "the report names the transition: {report}"
        );
        assert!(matches!(reconstruct(&g), Err(ReconstructError::Invalid(_))));
    }
}

// @tg: verifies:: [[openspec:chart-rdf#SHACL Shapes For Charts#Shapes and IR validation agree]] {level: unit}
#[test]
fn shapes_and_ir_validation_agree() {
    let mut runner = TestRunner::new(Config::with_cases(1));
    let strategies = [
        arb_any_chart_with(small()),
        arb_chart_with(small()),
        arb_chart_with(ChartConfig {
            max_targets: 3,
            ..small()
        }),
    ];
    let (mut good, mut bad) = (0, 0);
    for strategy in &strategies {
        for _ in 0..150 {
            let c = strategy.new_tree(&mut runner).unwrap().current();
            let ir = vertep_core::validate::validate(&c);
            let report = validate(&project(&c).graph);
            assert_eq!(
                ir.is_ok(),
                report.conforms(),
                "IR: {ir:?}\nshapes: {report}\nchart: {c:#?}"
            );
            if ir.is_ok() {
                good += 1;
            } else {
                bad += 1;
            }
        }
    }
    assert!(
        good > 100 && bad > 100,
        "{good} well-formed, {bad} ill-formed"
    );
}

// ------------------------------------------------------------------ Canonical Chart Hash

// @tg: verifies:: [[openspec:chart-rdf#Canonical Chart Hash#Same chart from two surfaces]] {level: unit}
#[test]
fn same_chart_from_two_surfaces() {
    let a = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a" name="m">
  <state id="a"><transition event="go" cond="In('a')" target="b"/></state>
  <final id="b"/>
</scxml>"#;
    let b = "<?xml version=\"1.0\"?>\n<scxml name='m'\n       initial='a' version='1.0'\n       \
             xmlns='http://www.w3.org/2005/07/scxml'>\n\n\n    <state\n        id='a'>\n        \
             <transition target='b'   cond=\"In('a')\" event='go'>\n        </transition>\n    \
             </state>\n    <final id='b'></final></scxml>\n";
    let ca = vertep_scxml::read_str(a).unwrap().chart;
    let cb = vertep_scxml::read_str(b).unwrap().chart;
    assert_ne!(ca.states[0].span.line, cb.states[0].span.line);
    assert_eq!(chart_hash(&ca), chart_hash(&cb));
    assert_eq!(chart_version(&ca), chart_version(&cb));
    assert_eq!(canonical_nquads(&ca), canonical_nquads(&cb));
    // Spans never matter, whatever the surface.
    let k = common::kitchen_sink();
    assert_eq!(
        chart_hash(&k),
        chart_hash(&common::with_spans(k.clone(), 40))
    );
}

// @tg: verifies:: [[openspec:chart-rdf#Canonical Chart Hash#Any semantic change changes the hash]] {level: unit}
#[test]
fn any_semantic_change_changes_the_hash() {
    let c = common::kitchen_sink();
    let mut changed = c.clone();
    changed.states[0].transitions[0].cond = Some("x > 2 && !locked".into());
    assert_ne!(chart_hash(&c), chart_hash(&changed));
    let mut removed = c.clone();
    removed.states[0].transitions[0].cond = None;
    assert_ne!(chart_hash(&c), chart_hash(&removed));
    assert_ne!(chart_hash(&changed), chart_hash(&removed));
    // The canonical form is RDFC-1.0: sorted N-Quads lines.
    let nq = canonical_nquads(&c);
    let lines: Vec<&str> = nq.lines().collect();
    assert!(lines.windows(2).all(|w| w[0] < w[1]));
}

// ------------------------------------------------------------------ Chart Version IRI

// @tg: verifies:: [[openspec:chart-rdf#Chart Version IRI#Version IRI from hash]] {level: unit}
#[test]
fn version_iri_from_hash() {
    let c = common::kitchen_sink();
    let hex: String = chart_hash(&c).iter().map(|b| format!("{b:02x}")).collect();
    let v = chart_version(&c);
    assert_eq!(v.as_str(), format!("urn:vertep:chart:sha256:{hex}"));
    assert_eq!(v.hex(), hex);
    assert_eq!(ChartVersion::parse(v.as_str()), Some(v.clone()));
    assert_eq!(project(&c).version, v);
    assert_eq!(graph_version(&project(&c).graph), Ok(v));
}

// @tg: verifies:: [[openspec:chart-rdf#Chart Version IRI#Node IRIs scoped by version]] {level: unit}
#[test]
fn node_iris_scoped_by_version() {
    let one = common::chart("null", vec![common::atomic("a")]);
    let two = common::chart("null", vec![common::atomic("a"), common::atomic("b")]);
    let (p1, p2) = (project(&one), project(&two));
    assert_ne!(p1.version, p2.version);
    let (a1, a2) = (node(&p1.version, "s.a"), node(&p2.version, "s.a"));
    assert_ne!(a1, a2);
    assert!(
        p1.graph
            .contains(TripleRef::new(&a1, rdf_type(), vt::STATE_NODE))
    );
    assert!(
        p2.graph
            .contains(TripleRef::new(&a2, rdf_type(), vt::STATE_NODE))
    );
    // Every node IRI is minted under the version IRI.
    for p in [&p1, &p2] {
        let prefix = format!("{}#", p.version);
        assert!(
            p.graph
                .iter()
                .all(|t| t.subject.to_string()[1..].starts_with(&prefix))
        );
    }
}

// ------------------------------------------------------------------ Annotations Outside The Projection

fn annotated(p: &ProjectedChart) -> Graph {
    let ttl = format!(
        "{}\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix ex: <urn:example:> .\n\
         c:s.closed vt:transition c:s.closed:t0 ~ ex:r1 {{| rdfs:comment \"the door opens\" ; ex:by ex:alice |}} .\n\
         ex:r2 rdf:reifies <<( c:s.closed:t0 vt:target c:s.closed:t0:tg0 )>> ; ex:confidence 0.9 .\n\
         c:s.closed rdfs:comment \"a comment on a chart node\" .\n\
         <{}> ex:certifiedBy ex:cert1 .\n",
        write_chart(p, RdfFormat::Turtle).replace(
            "@prefix vt:",
            "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n@prefix vt:"
        ),
        p.version
    );
    let g = read_graph(ttl.as_bytes(), RdfFormat::Turtle).expect("RDF 1.2 Turtle parses");
    assert!(g.len() >= p.graph.len() + 6);
    assert!(
        g.iter().any(|t| matches!(t.object, TermRef::Triple(_))),
        "has triple terms"
    );
    g
}

// @tg: verifies:: [[openspec:chart-rdf#Annotations Outside The Projection#Annotation does not change the chart]] {level: unit}
#[test]
fn annotation_does_not_change_the_chart() {
    let c = common::door();
    let p = project(&c);
    let g = annotated(&p);
    let r = reconstruct(&g).expect("the annotated graph reconstructs");
    assert_eq!(r.chart, c);
    assert_eq!(graph_version(&g), Ok(p.version.clone()));
    assert_eq!(chart_hash(&r.chart), chart_hash(&c));
}

// @tg: verifies:: [[openspec:chart-rdf#Annotations Outside The Projection#Annotation does not break validation]] {level: unit}
#[test]
fn annotation_does_not_break_validation() {
    let p = project(&common::door());
    let report = validate(&annotated(&p));
    assert!(report.conforms(), "{report}");
}

// ------------------------------------------------------------------ RDF Serialization Formats

// @tg: verifies:: [[openspec:chart-rdf#RDF Serialization Formats#TriG named graph]] {level: unit}
#[test]
fn trig_named_graph() {
    let p = project(&common::kitchen_sink());
    let trig = write_chart(&p, RdfFormat::TriG);
    let ds = read_dataset(trig.as_bytes(), RdfFormat::TriG).unwrap();
    assert_eq!(ds.len(), p.graph.len());
    let name = p.version.named_node();
    assert!(
        ds.iter()
            .all(|q| q.graph_name == oxrdf::GraphNameRef::NamedNode(name.as_ref()))
    );
    assert!(
        trig.contains(&format!("<{}> {{", p.version)),
        "{}",
        &trig[..300]
    );
    // The same holds for N-Quads.
    let nq = write_chart(&p, RdfFormat::NQuads);
    let ds = read_dataset(nq.as_bytes(), RdfFormat::NQuads).unwrap();
    assert!(
        ds.iter()
            .all(|q| q.graph_name == oxrdf::GraphNameRef::NamedNode(name.as_ref()))
    );
    assert_eq!(
        read_chart(trig.as_bytes(), RdfFormat::TriG).unwrap().chart,
        common::kitchen_sink()
    );
}

// @tg: verifies:: [[openspec:chart-rdf#RDF Serialization Formats#Format independence]] {level: unit}
#[test]
fn format_independence() {
    let c = common::kitchen_sink();
    let p = project(&c);
    let ttl = read_chart(
        write_chart(&p, RdfFormat::Turtle).as_bytes(),
        RdfFormat::Turtle,
    )
    .unwrap();
    let nq = read_chart(
        write_chart(&p, RdfFormat::NQuads).as_bytes(),
        RdfFormat::NQuads,
    )
    .unwrap();
    assert_eq!(ttl.chart, nq.chart);
    assert_eq!(ttl.chart, c);
    for format in RdfFormat::ALL {
        let back = read_chart(write_chart(&p, format).as_bytes(), format).unwrap();
        assert_eq!(back.chart, c, "{format:?}");
        assert_eq!(back.base, p.version.as_str());
    }
    // A plain graph written without a graph name also reads back.
    let nt = write_graph(&p.graph, RdfFormat::NQuads, None, &[]);
    assert_eq!(
        read_chart(nt.as_bytes(), RdfFormat::NQuads).unwrap().chart,
        c
    );
}
