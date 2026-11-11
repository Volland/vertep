//! The Rust projection of the type language (type-system spec), as the macros generate it.

use vertep_core::datamodel::Value;
use vertep_core::types::{Declarations, Primitive, Type};
use vertep_dm_rust::{FromValue, ToValue, statechart};

statechart! {
    #[allow(unhandled_events, unreachable_states)]
    name: types_demo,
    types {
        Payment = Card { last4: string } | Cash,
        Line = { sku: string, qty: int, price: decimal, note: optional<string> },
        Order = {
            id: iri,
            at: datetime,
            lines: list<Line>,
            payment: Payment,
            paid: bool,
            extra: value,
            parent: optional<Order>,
            "unit price": optional<Money>,
            type: optional<string>,
        },
        Tree = Leaf | Node { children: list<Tree> } | Labeled string,
        Money = decimal,
    }
    events { place: Order }
    initial open;
    state open { on place => open; }
}

fn label(p: &Payment) -> String {
    // Exhaustive: the enum has exactly these two variants.
    match p {
        Payment::Card { last4 } => format!("card {last4}"),
        Payment::Cash => "cash".to_string(),
    }
}

// @tg: verifies:: [[openspec:type-system#Rust Projection#Union projects to an enum]] {level: integration}
#[test]
fn union_projects_to_an_enum() {
    let card = Payment::Card {
        last4: String::from("4242"),
    };
    assert_eq!(label(&card), "card 4242");
    assert_eq!(label(&Payment::Cash), "cash");
    assert_eq!(
        card.to_value(),
        Value::Object(vec![
            ("tag".into(), Value::String("Card".into())),
            (
                "value".into(),
                Value::Object(vec![("last4".into(), Value::String("4242".into()))])
            ),
        ])
    );
    assert_eq!(
        Payment::Cash.to_value(),
        Value::Object(vec![("tag".into(), Value::String("Cash".into()))])
    );
    // Records are structs, lists Vec, optionals Option, recursion through optional is boxed.
    let order = Order::default();
    let _: &Vec<Line> = &order.lines;
    let _: &Option<Box<Order>> = &order.parent;
    let _: &Option<Money> = &order.unit_price;
    let _: &Option<String> = &order.r#type;
    let _: vertep_dm_rust::Decimal = Money::ZERO;
}

/// A small deterministic generator of canonical values of a type.
struct Gen {
    state: u64,
}

impl Gen {
    fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn value(&mut self, ty: &Type, decls: &Declarations, depth: u32) -> Value {
        match ty {
            Type::Primitive(p) => match p {
                Primitive::Bool => Value::Bool(self.below(2) == 1),
                Primitive::Int => Value::Number(self.below(20_001) as f64 - 10_000.0),
                Primitive::Decimal => {
                    let m = self.below(2_000_001) as i64 - 1_000_000;
                    let d = vertep_dm_rust::Decimal::new(m, self.below(5) as u32);
                    Value::String(d.to_string())
                }
                Primitive::String => {
                    let words = ["", "a", "naïve \"quoted\"", "line\nbreak", "4242", "日本"];
                    Value::String(words[self.below(words.len() as u64) as usize].to_string())
                }
                Primitive::Datetime => Value::String(format!(
                    "20{:02}-0{}-1{}T0{}:3{}:0{}{}Z",
                    self.below(100),
                    1 + self.below(9),
                    self.below(9),
                    self.below(10),
                    self.below(10),
                    self.below(10),
                    if self.below(2) == 0 { "" } else { ".25" }
                )),
                Primitive::Iri => {
                    Value::String(format!("https://example.org/o/{}", self.below(99)))
                }
            },
            Type::Value => match self.below(5) {
                0 => Value::Null,
                1 => Value::Number(self.below(100) as f64 / 4.0),
                2 => Value::String("any".into()),
                3 => Value::Array(vec![Value::Bool(true), Value::Number(2.0)]),
                _ => Value::Object(vec![
                    ("z".into(), Value::Bool(false)),
                    ("a".into(), Value::Null),
                ]),
            },
            Type::List(inner) => {
                let n = if depth == 0 { 0 } else { self.below(3) };
                Value::Array(
                    (0..n)
                        .map(|_| self.value(inner, decls, depth - 1))
                        .collect(),
                )
            }
            Type::Optional(inner) => {
                if depth == 0 || self.below(2) == 0 {
                    Value::Null
                } else {
                    self.value(inner, decls, depth - 1)
                }
            }
            Type::Named(n) => self.value(decls.lookup(n).unwrap(), decls, depth),
            Type::Record(fields) => {
                let mut out = Vec::new();
                for f in fields {
                    let v = self.value(&f.ty, decls, depth);
                    if !matches!(v, Value::Null) || !is_optional(&f.ty, decls) {
                        out.push((f.name.clone(), v));
                    }
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Object(out)
            }
            Type::Union(vs) => {
                let v = &vs[self.below(vs.len() as u64) as usize];
                let mut out = vec![("tag".to_string(), Value::String(v.tag.clone()))];
                if let Some(p) = &v.payload {
                    out.push(("value".to_string(), self.value(p, decls, depth)));
                }
                Value::Object(out)
            }
        }
    }
}

fn is_optional(t: &Type, decls: &Declarations) -> bool {
    match t {
        Type::Optional(_) | Type::Value => true,
        Type::Named(n) => decls.lookup(n).is_some_and(|t| is_optional(t, decls)),
        _ => false,
    }
}

fn round_trip<T: FromValue + ToValue>(name: &str, decls: &Declarations, g: &mut Gen) {
    let ty = Type::named(name);
    for _ in 0..300 {
        let v = g.value(&ty, decls, 3);
        // The generator writes canonical encodings; the core checker agrees.
        let canonical = decls
            .encode(&ty, &v)
            .unwrap_or_else(|e| panic!("{v:?}: {e:?}"));
        assert_eq!(canonical, v);
        let rust = T::from_value(&v).unwrap_or_else(|e| panic!("{name} {v:?}: {e}"));
        assert_eq!(rust.to_value(), v, "{name}");
    }
}

// @tg: verifies:: [[openspec:type-system#Rust Projection#Round trip through Rust]] {level: integration}
#[test]
fn round_trip_through_rust() {
    let decls = vertep_core::types::chart::declarations(&CHART)
        .unwrap()
        .unwrap();
    let mut g = Gen { state: 7 };
    round_trip::<Payment>("Payment", &decls, &mut g);
    round_trip::<Line>("Line", &decls, &mut g);
    round_trip::<Order>("Order", &decls, &mut g);
    round_trip::<Tree>("Tree", &decls, &mut g);
    round_trip::<Money>("Money", &decls, &mut g);
    // A value of the wrong type is refused with the path of the problem.
    let bad = Value::Object(vec![("tag".into(), Value::String("Wire".into()))]);
    assert!(
        Payment::from_value(&bad)
            .unwrap_err()
            .message
            .contains("Wire")
    );
    // Event payloads use the same projection.
    let ev = Event::Place {
        id: vertep_dm_rust::Iri::parse("urn:o:1").unwrap(),
        at: vertep_dm_rust::Datetime::parse("2026-11-11T10:00:00+02:00").unwrap(),
        lines: vec![],
        payment: Payment::Cash,
        paid: false,
        extra: Value::Bool(true),
        parent: None,
        unit_price: None,
        r#type: Some("rush".into()),
    };
    use vertep_dm_rust::ChartEvent;
    let raw = ev.to_event();
    assert_eq!(Event::from_event(&raw).unwrap(), ev);
    assert!(decls.check(&Type::named("Order"), &raw.data).is_ok());
    // In the canonical encoding a `value` field holding null is the same as an absent one.
    let mut o = Order {
        extra: Value::Null,
        ..Order::default()
    };
    o = Order::from_value(&o.to_value()).unwrap();
    assert_eq!(o.extra, Value::Undefined);
}
