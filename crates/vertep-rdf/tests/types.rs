//! Type projections and content-addressed type IRIs (`type-system` capability, tasks 2.1 to
//! 2.6 of `openspec/changes/m1-typed-authoring`).

use std::collections::BTreeSet;

use oxrdf::{Graph, NamedNode, NamedNodeRef, Term, TermRef};
use proptest::prelude::*;
use serde_json::{Value as Json, json};
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;
use vertep_core::types::{Declarations, Field, Primitive, Type, Variant, parse_declarations};
use vertep_rdf::types::shapes::validate_value;
use vertep_rdf::types::{TypeIri, TypeIris, json_schema, types_ontology};
use vertep_rdf::vocab as vt;
use vertep_rdf::{chart_version, project, reconstruct};

fn decls(src: &str) -> Declarations {
    parse_declarations(src).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn iri_of(src: &str, name: &str) -> TypeIri {
    let iris = TypeIris::of(&decls(src));
    assert!(iris.errors().is_empty(), "{:?}", iris.errors());
    iris.get(name)
        .unwrap_or_else(|| panic!("{name} has no IRI"))
        .clone()
}

fn chart(types: &str, body: &str) -> Chart {
    let doc = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" initial="a">
  <vt:types><![CDATA[{types}]]></vt:types>
  <state id="a">{body}</state>
</scxml>"#
    );
    vertep_scxml::read_str(&doc)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .chart
}

fn rdf_type() -> NamedNodeRef<'static> {
    NamedNodeRef::new_unchecked(vt::RDF_TYPE)
}

// ------------------------------------------------------- Named Content-Addressed Types

// @tg: verifies:: [[openspec:type-system#Named Content-Addressed Types#Identical definitions share an IRI]] {level: unit}
#[test]
fn identical_definitions_share_an_iri() {
    // Two charts declare `Order` identically among different other declarations.
    let a = chart(
        "type Order = { id: string, total: decimal }; event pay: Order;",
        "",
    );
    let b = chart(
        "type Item = { sku: string }; type Order = { id: string, total: decimal };",
        r#"<transition event="go" target="a"/>"#,
    );
    let ia = iri_of("type Order = { id: string, total: decimal };", "Order");
    let pa = project(&a);
    let pb = project(&b);
    assert_ne!(pa.version, pb.version, "the charts differ");
    for p in [&pa, &pb] {
        let declared: Vec<Term> = p
            .graph
            .objects_for_subject_predicate(&p.version.chart_node(), vt::DECLARES_TYPE)
            .map(TermRef::into_owned)
            .collect();
        assert!(
            declared.contains(&ia.named_node().into()),
            "{} declares {declared:?}",
            p.version
        );
    }
    assert!(ia.as_str().starts_with("urn:vertep:type:sha256:"));
    assert_eq!(ia.hex().len(), 64);
    assert_eq!(TypeIri::parse(ia.as_str()), Some(ia));
}

// @tg: verifies:: [[openspec:type-system#Named Content-Addressed Types#Changed definition gets a new IRI]] {level: unit}
#[test]
fn changed_definition_gets_a_new_iri() {
    let old_src = "type Order = { id: string, total: decimal };";
    let new_src = "type Order = { id: string, total: decimal, currency: string };";
    let old = iri_of(old_src, "Order");
    let new = iri_of(new_src, "Order");
    assert_ne!(old, new);
    // A type that uses Order changes with it (its IRI covers Order's).
    assert_ne!(
        iri_of(&format!("{old_src} type Batch = list<Order>;"), "Batch"),
        iri_of(&format!("{new_src} type Batch = list<Order>;"), "Batch"),
    );
    // A chart written against the old definition keeps referring to the old IRI, and its
    // graph still holds the old definition under it.
    let c = chart(old_src, "");
    let p = project(&c);
    let defs: Vec<Term> = p
        .graph
        .objects_for_subject_predicate(&old.named_node(), vt::TYPE_NAME)
        .map(TermRef::into_owned)
        .collect();
    assert_eq!(defs.len(), 1);
    assert!(
        p.graph
            .subjects_for_predicate_object(
                vt::TYPE_NAME,
                oxrdf::LiteralRef::new_simple_literal("Order")
            )
            .all(|s| s.to_string() == old.named_node().to_string()),
        "the chart does not mention the new IRI"
    );
    // The name is part of the identity.
    assert_ne!(
        old,
        iri_of("type Invoice = { id: string, total: decimal };", "Invoice")
    );
}

// @tg: verifies:: [[openspec:type-system#Named Content-Addressed Types#Field order does not change identity]] {level: unit}
#[test]
fn field_order_does_not_change_identity() {
    assert_eq!(
        iri_of("type Order = { id: string, total: decimal };", "Order"),
        iri_of("type Order = { total: decimal, id: string };", "Order"),
    );
    assert_eq!(
        iri_of("type P = Card { last4: string } | Cash;", "P"),
        iri_of("type P = Cash | Card { last4: string };", "P"),
    );
    // Declaration order of the block does not matter either.
    assert_eq!(
        iri_of("type A = { b: B }; type B = { x: int };", "A"),
        iri_of("type B = { x: int }; type A = { b: B };", "A"),
    );
    // But a field type does.
    assert_ne!(
        iri_of("type Order = { id: string };", "Order"),
        iri_of("type Order = { id: iri };", "Order"),
    );
}

#[test]
fn recursive_and_mutually_recursive_types_get_distinct_iris() {
    let tree = "type Node = { label: string, children: list<Node> };";
    let n = iri_of(tree, "Node");
    assert_eq!(n, iri_of(tree, "Node"), "deterministic");
    let mutual = "type A = { b: optional<B> }; type B = { a: list<A> };";
    let a = iri_of(mutual, "A");
    let b = iri_of(mutual, "B");
    assert_ne!(a, b);
    assert_eq!(
        a,
        iri_of("type B = { a: list<A> }; type A = { b: optional<B> };", "A")
    );
    assert_ne!(
        a,
        iri_of(
            "type A = { b: optional<B> }; type B = { a: list<A>, n: int };",
            "A"
        )
    );
    // A type using an undeclared name has no IRI, and neither has what depends on it.
    let iris = TypeIris::of(&decls(
        "type A = { x: Ordr }; type B = list<A>; type C = int;",
    ));
    assert!(iris.get("A").is_none() && iris.get("B").is_none());
    assert!(iris.get("C").is_some());
    assert_eq!(iris.errors().len(), 2);
}

// ------------------------------------------------------------------- SHACL Projection

// @tg: verifies:: [[openspec:type-system#SHACL Projection#Shapes are found by type IRI]] {level: unit}
#[test]
fn shapes_are_found_by_type_iri() {
    let src = "type Order = { id: string, total: decimal }; \
               type Payment = Card { last4: string } | Cash; \
               type Node = { label: string, children: list<Node> };";
    let c = chart(src, "");
    let p = project(&c);
    let iris = TypeIris::of(&decls(src));
    let node_shape = NamedNode::new_unchecked("http://www.w3.org/ns/shacl#NodeShape");
    let mut declared = BTreeSet::new();
    for (name, iri) in iris.iter() {
        let n = iri.named_node();
        assert!(
            p.graph
                .contains(oxrdf::TripleRef::new(&n, rdf_type(), &node_shape)),
            "{name} is a node shape at {iri}"
        );
        assert!(
            p.graph
                .contains(oxrdf::TripleRef::new(&n, rdf_type(), vt::NAMED_TYPE))
        );
        declared.insert(n.to_string());
    }
    assert_eq!(declared.len(), 3);
    // The shapes in the chart graph compile and validate values by type IRI.
    let shapes = vertep_rdf::shacl::Shapes::from_graph(&p.graph).expect("shapes compile");
    assert!(!shapes.is_empty());
    // The typed chart still round-trips through RDF with its types and version.
    let back = reconstruct(&p.graph).expect("reconstructs");
    assert_eq!(back.chart, c);
    assert_eq!(chart_version(&back.chart), p.version);
}

/// The fixed named types the generated types may use.
const NAMED: &str = "type Item = { sku: string, qty: int }; \
                     type Node = { label: string, children: list<Node> }; \
                     type Pay = Card { last4: string } | Cash; \
                     type MaybeInt = optional<int>;";

fn arb_type() -> impl Strategy<Value = Type> {
    let leaf = prop_oneof![
        prop::sample::select(Primitive::ALL.to_vec()).prop_map(Type::Primitive),
        Just(Type::Value),
        prop::sample::select(vec!["Item", "Node", "Pay", "MaybeInt"]).prop_map(Type::named),
    ];
    leaf.prop_recursive(3, 16, 3, |inner| {
        prop_oneof![
            inner.clone().prop_map(Type::list),
            inner.clone().prop_map(Type::optional),
            prop::collection::btree_map(
                prop::sample::select(vec!["a", "b", "c", "tag"]),
                inner.clone(),
                0..3
            )
            .prop_map(|m| Type::Record(m.into_iter().map(|(n, t)| Field::new(n, t)).collect())),
            prop::collection::btree_map(
                prop::sample::select(vec!["X", "Y", "Z"]),
                prop::option::of(inner),
                1..3
            )
            .prop_map(|m| Type::Union(m.into_iter().map(|(n, t)| Variant::new(n, t)).collect())),
        ]
    })
}

/// A small deterministic generator for values of a type.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<T: Clone>(&mut self, xs: &[T]) -> T {
        xs[(self.next() % xs.len() as u64) as usize].clone()
    }
    fn chance(&mut self, n: u64) -> bool {
        self.next().is_multiple_of(n)
    }
}

fn s(x: &str) -> Value {
    Value::String(x.to_owned())
}

/// A value the checker accepts for `ty`.
fn valid(d: &Declarations, ty: &Type, r: &mut Rng, depth: u32) -> Value {
    match ty {
        Type::Primitive(Primitive::Bool) => Value::Bool(r.chance(2)),
        Type::Primitive(Primitive::Int) => Value::Number(r.pick(&[
            0.0,
            -7.0,
            42.0,
            9_007_199_254_740_992.0,
            -9_223_372_036_854_775_808.0,
        ])),
        Type::Primitive(Primitive::Decimal) => r.pick(&[
            Value::Number(1.5),
            Value::Number(-3.0),
            s("12.50"),
            s("-0"),
            s("7"),
        ]),
        Type::Primitive(Primitive::String) => r.pick(&[s(""), s("a"), s("héllo")]),
        Type::Primitive(Primitive::Datetime) => r.pick(&[
            s("2026-11-11T10:00:00Z"),
            s("2024-02-29T23:59:59.5+05:30"),
            s("2000-02-29t00:00:00z"),
            s("1999-12-31T23:59:59-08:00"),
        ]),
        Type::Primitive(Primitive::Iri) => r.pick(&[
            s("urn:x"),
            s("https://example.org/a?b=c#d"),
            s("a:%20"),
            s("mailto:ü@example.org"),
        ]),
        Type::Value => r.pick(&[
            Value::Null,
            s("x"),
            Value::Number(2.5),
            Value::Array(vec![]),
        ]),
        Type::Named(n) => {
            let t = d.lookup(n).cloned().unwrap_or(Type::Value);
            if depth == 0 && n == "Node" {
                Value::Object(vec![
                    ("label".into(), s("leaf")),
                    ("children".into(), Value::Array(vec![])),
                ])
            } else {
                valid(d, &t, r, depth.saturating_sub(1))
            }
        }
        Type::Optional(inner) => {
            if r.chance(3) {
                Value::Null
            } else {
                valid(d, inner, r, depth)
            }
        }
        Type::List(item) => {
            let n = if depth == 0 { 0 } else { r.next() % 3 };
            Value::Array((0..n).map(|_| valid(d, item, r, depth - 1)).collect())
        }
        Type::Record(fields) => {
            let mut o = Vec::new();
            for f in fields {
                if vertep_rdf_may_be_absent(d, &f.ty) && r.chance(3) {
                    continue;
                }
                o.push((f.name.clone(), valid(d, &f.ty, r, depth.saturating_sub(1))));
            }
            Value::Object(o)
        }
        Type::Union(vs) => {
            let v = r.pick(vs);
            let mut o = vec![("tag".to_owned(), s(&v.tag))];
            if let Some(p) = &v.payload
                && !(vertep_rdf_may_be_absent(d, p) && r.chance(3))
            {
                o.push(("value".to_owned(), valid(d, p, r, depth.saturating_sub(1))));
            }
            Value::Object(o)
        }
    }
}

fn vertep_rdf_may_be_absent(d: &Declarations, t: &Type) -> bool {
    let mut t = t;
    for _ in 0..8 {
        match t {
            Type::Named(n) => match d.lookup(n) {
                Some(x) => t = x,
                None => return false,
            },
            Type::Optional(_) | Type::Value => return true,
            _ => return false,
        }
    }
    false
}

/// Replace one node of `v` (chosen by `r`) with a near-miss: a value of another kind, a
/// boundary or malformed primitive, or an object with a field removed or added.
fn mutate(v: &mut Value, r: &mut Rng) {
    let children = match v {
        Value::Array(items) if !items.is_empty() => items.len(),
        Value::Object(es) if !es.is_empty() => es.len(),
        _ => 0,
    };
    if children > 0 && !r.chance(3) {
        let i = (r.next() % children as u64) as usize;
        match v {
            Value::Array(items) => return mutate(&mut items[i], r),
            Value::Object(es) => return mutate(&mut es[i].1, r),
            _ => {}
        }
    }
    if let Value::Object(es) = v {
        match r.next() % 3 {
            0 if !es.is_empty() => {
                let i = (r.next() % es.len() as u64) as usize;
                es.remove(i);
                return;
            }
            1 => {
                es.push(("note".into(), s("extra")));
                return;
            }
            _ => {}
        }
    }
    *v = r.pick(&[
        Value::Null,
        Value::Bool(true),
        Value::Number(1.5),
        Value::Number(9_223_372_036_854_775_808.0),
        Value::Number(-3.0),
        s("orders/42"),
        s("2026-11-11T10:00:00"),
        s("2023-02-29T00:00:00Z"),
        s("2026-13-01T00:00:00Z"),
        s("1."),
        s("12.5"),
        s("a:%zz"),
        s("http://x y"),
        Value::Array(vec![]),
        Value::Array(vec![Value::Number(1.0)]),
        Value::Object(vec![]),
        Value::Object(vec![("tag".into(), s("Cash"))]),
        Value::Object(vec![("tag".into(), s("Card"))]),
        Value::Object(vec![("tag".into(), s("X"))]),
        Value::Object(vec![("tag".into(), Value::Number(1.0))]),
    ]);
}

/// JSON of a value: integral numbers in the 64-bit range as JSON integers, as a JSON parser
/// would read them back.
fn to_json(v: &Value) -> Json {
    match v {
        Value::Undefined | Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Number(n) => match vertep_core::types::prim::f64_to_i64(*n) {
            Some(i) => json!(i),
            None => json!(n),
        },
        Value::String(s) => json!(s),
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        Value::Object(es) => {
            Json::Object(es.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
    }
}

fn json_valid(schema: &Json, doc: &Json) -> bool {
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(false)
        .build(schema)
        .unwrap_or_else(|e| panic!("schema does not compile: {e}\n{schema:#}"));
    validator.is_valid(doc)
}

/// The three verdicts on one value.
fn verdicts(d: &Declarations, ty: &Type, v: &Value) -> (bool, bool, bool) {
    let checker = d.check(ty, v).is_ok();
    let schema = json_schema(d, ty).expect("all names declared");
    let js = json_valid(&schema, &to_json(v));
    let report = validate_value(d, ty, v).expect("shapes");
    (checker, js, report.conforms())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    // @tg: verifies:: [[openspec:type-system#JSON Schema Projection#Schema agrees with the checker]] {level: unit}
    // @tg: verifies:: [[openspec:type-system#SHACL Projection#SHACL agrees with the checker]] {level: unit}
    #[test]
    fn checker_json_schema_and_shacl_agree(ty in arb_type(), seed in 1u64..u64::MAX) {
        let d = decls(NAMED);
        let mut r = Rng(seed);
        let v = valid(&d, &ty, &mut r, 3);
        let (c, j, s) = verdicts(&d, &ty, &v);
        prop_assert!(c, "generated value is valid: {:?} for {}", v, ty);
        prop_assert_eq!((c, c), (j, s), "valid value {:?} for {}", v, ty);
        for _ in 0..3 {
            let mut m = v.clone();
            mutate(&mut m, &mut r);
            let (c, j, s) = verdicts(&d, &ty, &m);
            prop_assert_eq!((c, c), (j, s), "near-miss {:?} for {}", m, ty);
        }
    }
}

#[test]
fn agreement_on_the_spec_boundaries() {
    let d = decls(NAMED);
    let cases: Vec<(Type, Value, bool)> = vec![
        (Type::INT, Value::Number(9_223_372_036_854_775_808.0), false),
        (Type::INT, Value::Number(-9_223_372_036_854_775_808.0), true),
        (Type::INT, Value::Number(1.5), false),
        (Type::DATETIME, s("2026-11-11T10:00:00"), false),
        (Type::DATETIME, s("2026-11-11T10:00:00+02:00"), true),
        (Type::DATETIME, s("2023-02-29T10:00:00Z"), false),
        (Type::IRI, s("orders/42"), false),
        (Type::IRI, s("urn:vertep:order:42"), true),
        (Type::DECIMAL, s("1e5"), false),
        (
            Type::named("Pay"),
            Value::Object(vec![("tag".into(), s("Cash"))]),
            true,
        ),
        (
            Type::named("Pay"),
            Value::Object(vec![("tag".into(), s("Card"))]),
            false,
        ),
        (
            Type::named("Item"),
            Value::Object(vec![("sku".into(), s("a"))]),
            false,
        ),
        (
            Type::named("Item"),
            Value::Object(vec![
                ("sku".into(), s("a")),
                ("qty".into(), Value::Number(1.0)),
                ("note".into(), s("x")),
            ]),
            false,
        ),
        (
            Type::record([("x", Type::named("MaybeInt"))]),
            Value::Object(vec![]),
            true,
        ),
        (Type::list(Type::INT), Value::Object(vec![]), false),
        (Type::record::<&str>([]), Value::Array(vec![]), false),
        (Type::optional(Type::STRING), Value::Null, true),
    ];
    for (ty, v, expected) in cases {
        let (c, j, s) = verdicts(&d, &ty, &v);
        assert_eq!((c, j, s), (expected, expected, expected), "{v:?} : {ty}");
    }
}

// --------------------------------------------------------------- JSON Schema Projection

// @tg: verifies:: [[openspec:type-system#JSON Schema Projection#Tagged union in JSON Schema]] {level: unit}
#[test]
fn tagged_union_in_json_schema() {
    let d = decls("type Payment = Card { last4: string } | Cash;");
    let s = json_schema(&d, &Type::named("Payment")).unwrap();
    let def = &s["$defs"]["Payment"];
    let alts = def["anyOf"].as_array().expect("anyOf");
    assert_eq!(alts.len(), 2);
    assert_eq!(alts[0]["properties"]["tag"], json!({ "const": "Card" }));
    assert_eq!(alts[0]["required"], json!(["tag", "value"]));
    assert_eq!(
        alts[0]["properties"]["value"]["properties"]["last4"],
        json!({ "type": "string" })
    );
    assert_eq!(alts[1]["properties"]["tag"], json!({ "const": "Cash" }));
    assert_eq!(alts[1]["required"], json!(["tag"]));
    for a in alts {
        assert_eq!(a["additionalProperties"], json!(false));
    }
    assert_eq!(s["$ref"], json!("#/$defs/Payment"));
    assert_eq!(
        s["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema")
    );
    assert!(json_valid(&s, &json!({ "tag": "Cash" })));
    assert!(!json_valid(&s, &json!({ "tag": "Card" })));
}

// @tg: verifies:: [[openspec:type-system#JSON Schema Projection#Closed records]] {level: unit}
#[test]
fn closed_records() {
    let d = decls("");
    let ty = Type::record([
        ("id", Type::STRING),
        ("amount", Type::DECIMAL),
        ("note", Type::optional(Type::STRING)),
        ("extra", Type::Value),
    ]);
    let s = json_schema(&d, &ty).unwrap();
    assert_eq!(s["type"], json!("object"));
    assert_eq!(s["additionalProperties"], json!(false));
    assert_eq!(s["required"], json!(["id", "amount"]));
    assert_eq!(
        s["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["amount", "extra", "id", "note"])
    );
    assert!(!json_valid(
        &s,
        &json!({ "id": "1", "amount": "2", "other": 1 })
    ));
    assert!(json_valid(&s, &json!({ "id": "1", "amount": 2 })));
}

#[test]
fn json_schema_reports_unknown_types() {
    let d = decls("type Order = { id: string };");
    assert_eq!(
        json_schema(&d, &Type::list(Type::named("Ordr"))),
        Err(vec!["Ordr".to_owned()])
    );
}

// ------------------------------------------------------------ Types Survive Every Surface

// @tg: verifies:: [[openspec:type-system#Types Survive Every Surface#Type change changes the chart version]] {level: unit}
#[test]
fn type_change_changes_the_chart_version() {
    let body = r#"<transition event="pay" target="a"/>"#;
    let a = chart("event pay: { amount: decimal };", body);
    let b = chart("event pay: { amount: int };", body);
    let a2 = chart("event pay: { amount: decimal };", body);
    assert_ne!(chart_version(&a), chart_version(&b));
    assert_eq!(chart_version(&a), chart_version(&a2));
    // A named type change changes the version too, and the declared type IRIs differ.
    let c = chart("type Pay = { amount: decimal }; event pay: Pay;", body);
    let d = chart(
        "type Pay = { amount: decimal, currency: string }; event pay: Pay;",
        body,
    );
    assert_ne!(chart_version(&c), chart_version(&d));
    let declared = |c: &Chart| -> Vec<Term> {
        let p = project(c);
        p.graph
            .objects_for_subject_predicate(&p.version.chart_node(), vt::DECLARES_TYPE)
            .map(TermRef::into_owned)
            .collect()
    };
    assert_ne!(declared(&c), declared(&d));
    // An untyped chart has no type triples.
    let untyped = vertep_scxml::read_str(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"/></scxml>"#,
    )
    .unwrap()
    .chart;
    assert!(declared(&untyped).is_empty());
}

// ------------------------------------------------------------------------- ontology

#[test]
fn type_terms_are_declared_in_the_type_module() {
    let onto = types_ontology();
    let label = NamedNodeRef::new_unchecked("http://www.w3.org/2000/01/rdf-schema#label");
    let c = chart(
        "type A = { a: list<optional<int>>, b: B, v: value, d: datetime, i: iri, x: bool, s: string, n: decimal }; type B = X { y: A } | Z;",
        "",
    );
    let g: Graph = project(&c).graph;
    let mut used = BTreeSet::new();
    for t in g.iter() {
        if t.predicate.as_str().starts_with(vt::VT) {
            used.insert(t.predicate.into_owned());
        }
        if let TermRef::NamedNode(o) = t.object
            && o.as_str().starts_with(vt::VT)
        {
            used.insert(o.into_owned());
        }
    }
    let chart_onto = vertep_rdf::chart_ontology();
    for term in used {
        let declared = onto.objects_for_subject_predicate(&term, label).count()
            + chart_onto
                .objects_for_subject_predicate(&term, label)
                .count();
        assert_eq!(declared, 1, "{term} is declared once with a label");
    }
}

/// The generated corpus is not one-sided: most near-misses are rejected, and some are
/// accepted (a mutation that lands inside `value` or replaces a value by another valid one).
#[test]
fn near_miss_corpus_has_both_verdicts() {
    let d = decls(NAMED);
    let types = [
        Type::named("Item"),
        Type::named("Node"),
        Type::named("Pay"),
        Type::list(Type::named("MaybeInt")),
        Type::record([
            ("at", Type::DATETIME),
            ("id", Type::IRI),
            ("n", Type::DECIMAL),
        ]),
    ];
    let (mut accepted, mut rejected) = (0, 0);
    for seed in 1..120u64 {
        for ty in &types {
            let mut r = Rng(seed * 7919);
            let mut v = valid(&d, ty, &mut r, 3);
            mutate(&mut v, &mut r);
            let (c, j, s) = verdicts(&d, ty, &v);
            assert_eq!((c, c), (j, s), "{v:?} : {ty}");
            if c {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
    }
    assert!(
        rejected > accepted,
        "{rejected} rejected, {accepted} accepted"
    );
    assert!(accepted > 0, "{rejected} rejected, {accepted} accepted");
}

/// An object entry whose value is `undefined` is absent, for the checker as for the SHACL
/// encoding (which leaves such entries out): a record or union value with an unknown field
/// holding `undefined` has no unknown field. Before verification-backlog 4.2 the checker
/// reported `UnknownField` here while the shapes conformed.
// @tg: verifies:: [[openspec:type-system#SHACL Projection#SHACL agrees with the checker]] {level: unit}
#[test]
fn undefined_entries_are_absent_for_the_checker_and_the_shapes() {
    let d = decls(NAMED);
    let record = Type::record([("a", Type::INT)]);
    let union = Type::Union(vec![
        Variant::new("X", None),
        Variant::new("Y", Some(Type::INT)),
    ]);
    let cases: Vec<(Type, Value, bool)> = vec![
        (
            record.clone(),
            Value::Object(vec![
                ("a".into(), Value::Number(1.0)),
                ("q".into(), Value::Undefined),
            ]),
            true,
        ),
        (
            record.clone(),
            Value::Object(vec![("a".into(), Value::Undefined)]),
            false,
        ),
        (
            union.clone(),
            Value::Object(vec![("tag".into(), s("X")), ("q".into(), Value::Undefined)]),
            true,
        ),
        (
            union.clone(),
            Value::Object(vec![
                ("tag".into(), s("X")),
                ("value".into(), Value::Undefined),
            ]),
            true,
        ),
        (
            union,
            Value::Object(vec![
                ("tag".into(), s("Y")),
                ("value".into(), Value::Undefined),
            ]),
            false,
        ),
    ];
    for (ty, v, expected) in cases {
        let checker = d.check(&ty, &v).is_ok();
        let shapes = validate_value(&d, &ty, &v).expect("shapes").conforms();
        assert_eq!((checker, shapes), (expected, expected), "{v:?} : {ty}");
    }
}

/// The `sh:pattern`s of the type shapes are XPath regular expressions (SHACL §4.6.1), so any
/// SHACL engine compiles them: no `\x` or `\u` escapes, which XPath lacks. Before
/// verification-backlog 4.2 the IRI pattern used `\x00-\x20\x7F`, which LeanRDF's SHACL validator
/// (XPath regular expressions) rejects. The patterns still agree with the checker, control
/// characters included.
// @tg: verifies:: [[openspec:type-system#SHACL Projection#SHACL agrees with the checker]] {level: unit}
#[test]
fn shacl_patterns_use_the_xpath_syntax() {
    use vertep_rdf::types::shapes::{DATETIME_PATTERN, DECIMAL_PATTERN, SHACL_IRI_PATTERN};
    for p in [DATETIME_PATTERN, DECIMAL_PATTERN, SHACL_IRI_PATTERN] {
        assert!(!p.contains("\\x") && !p.contains("\\u"), "{p}");
    }
    let d = decls(NAMED);
    for (v, ok) in [
        ("a:\u{85}", true),
        ("a:\u{9f}", true),
        ("a:\u{a0}", true),
        ("a:\u{7f}", false),
        ("a:\u{1}", false),
        ("a:\u{0}", false),
        ("a:\u{1f}", false),
        ("a: ", false),
        ("a:%41", true),
        ("a:%4", false),
        ("a:x^y", false),
        ("urn:vertep:order:42", true),
    ] {
        let checker = d.check(&Type::IRI, &s(v)).is_ok();
        let shapes = validate_value(&d, &Type::IRI, &s(v))
            .expect("shapes")
            .conforms();
        assert_eq!((checker, shapes), (ok, ok), "{v:?}");
    }
}

/// Every `sh:path` of the type shapes is a well-formed SHACL property path (SHACL §2.3.1): an
/// IRI is a predicate path, so the nodes of sequence and `sh:zeroOrMorePath` paths must be blank
/// nodes. Found by the Lean cross-check (verification backlog 4.2, `VertepTests/Shacl.lean`):
/// the member property of `list<T>` minted `<…#t.rest*>` and `<…#t.members.0>` as IRIs, so a
/// standard SHACL engine read the path as a predicate and never checked the members. Fixed by
/// verification-backlog 4.7: members are reached on `vt:listMember`, a predicate path.
// @tg: verifies:: [[openspec:type-system#SHACL Projection#SHACL agrees with the checker]] {level: unit}
#[test]
fn type_shape_paths_are_well_formed_shacl() {
    let d = decls(NAMED);
    let g = vertep_rdf::types::shapes::value_shapes(&d, &Type::list(Type::INT)).expect("shapes");
    let path = NamedNodeRef::new_unchecked("http://www.w3.org/ns/shacl#path");
    let complex = [
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#first",
        "http://www.w3.org/ns/shacl#zeroOrMorePath",
    ];
    for t in g.triples_for_predicate(path) {
        if let TermRef::NamedNode(n) = t.object {
            for p in complex {
                assert!(
                    g.object_for_subject_predicate(n, NamedNodeRef::new_unchecked(p))
                        .is_none(),
                    "sh:path {n} is an IRI with {p}: read as a predicate path"
                );
            }
        }
    }
}

/// The published shapes of `list<T>` check the members under a standard reading of SHACL paths
/// (an IRI `sh:path` is a predicate): the shapes a chart projection carries for
/// `type L = list<int>`, read by the validator, reject `[1, "2"]` and accept `[1, 2]`; the members
/// are the `vt:listMember` triples of the encoding, next to the collection that keeps the order.
// @tg: verifies:: [[openspec:type-system#SHACL Projection#SHACL agrees with the checker]] {level: unit}
#[test]
fn published_list_shapes_check_every_member() {
    use vertep_rdf::shacl::Shapes;
    use vertep_rdf::types::value::{VALUE_ROOT, value_graph};

    let c = chart("type L = list<int>; type LL = list<L>;", "");
    let g: Graph = project(&c).graph;
    let path = NamedNodeRef::new_unchecked("http://www.w3.org/ns/shacl#path");
    let paths: Vec<_> = g.triples_for_predicate(path).map(|t| t.object).collect();
    assert!(paths.iter().all(|p| matches!(p, TermRef::NamedNode(_))));
    assert!(paths.contains(&TermRef::NamedNode(vt::LIST_MEMBER)));
    let d = decls("type L = list<int>; type LL = list<L>;");
    let iris = TypeIris::of(&d);
    let shapes = g.clone();
    let root = NamedNodeRef::new_unchecked(VALUE_ROOT);
    let value = NamedNodeRef::new_unchecked(vt::RDF_VALUE);
    let sh = |l: &str| NamedNode::new_unchecked(format!("http://www.w3.org/ns/shacl#{l}"));
    let check = |name: &str, v: &Value| {
        let mut sg = shapes.clone();
        let target = NamedNode::new_unchecked("urn:test:target");
        let p = NamedNode::new_unchecked("urn:test:target-p");
        let t = iris.get(name).unwrap().named_node();
        sg.insert(oxrdf::TripleRef::new(&target, rdf_type(), &sh("NodeShape")));
        sg.insert(oxrdf::TripleRef::new(&target, &sh("targetNode"), root));
        sg.insert(oxrdf::TripleRef::new(&target, &sh("property"), &p));
        sg.insert(oxrdf::TripleRef::new(&p, &sh("path"), value));
        sg.insert(oxrdf::TripleRef::new(&p, &sh("node"), &t));
        let report = Shapes::from_graph(&sg)
            .expect("shapes")
            .validate(&value_graph(v));
        (report.conforms(), d.check(&Type::named(name), v).is_ok())
    };
    let n = |x: f64| Value::Number(x);
    let ints = Value::Array(vec![n(1.0), n(2.0), n(1.0)]);
    assert_eq!(check("L", &ints), (true, true));
    assert_eq!(check("L", &Value::Array(vec![])), (true, true));
    assert_eq!(
        check("L", &Value::Array(vec![n(1.0), s("2")])),
        (false, false)
    );
    assert_eq!(
        check("L", &Value::Array(vec![s("2"), n(1.0)])),
        (false, false)
    );
    assert_eq!(check("L", &Value::Object(vec![])), (false, false));
    let nested = |x: Value| Value::Array(vec![Value::Array(vec![n(1.0)]), Value::Array(vec![x])]);
    assert_eq!(check("LL", &nested(n(2.0))), (true, true));
    assert_eq!(check("LL", &nested(s("2"))), (false, false));
    // The encoding: one vt:listMember per distinct item term on the first cell, and the
    // collection with every item in order.
    let eg = value_graph(&ints);
    let head = eg
        .object_for_subject_predicate(root, value)
        .expect("root")
        .into_owned();
    let oxrdf::Term::BlankNode(head) = head else {
        panic!("a non-empty list is a blank node")
    };
    assert_eq!(
        eg.objects_for_subject_predicate(&head, vt::LIST_MEMBER)
            .count(),
        2
    );
    let first = NamedNodeRef::new_unchecked(vt::RDF_FIRST);
    let rest = NamedNodeRef::new_unchecked(vt::RDF_REST);
    let mut cell = oxrdf::Term::BlankNode(head);
    let mut items = Vec::new();
    while let oxrdf::Term::BlankNode(b) = &cell {
        items.push(
            eg.object_for_subject_predicate(b, first)
                .unwrap()
                .to_string(),
        );
        cell = eg
            .object_for_subject_predicate(b, rest)
            .unwrap()
            .into_owned();
    }
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], items[2]);
}
