//! The SHACL subset the checker interprets directly agrees with full SHACL validation in
//! `vertep-rdf`: the checker's verdict on a payload equals the verdict of `vertep-rdf`'s SHACL
//! Core validator over `Shape::to_shacl` and `payload_graph`, for generated shapes with valid and
//! near-miss payloads, for boundary cases and for every payload of the conformance vectors.
#![allow(missing_docs)]

use proptest::prelude::*;
use serde_json::{Map, Value, json};
use vertep_kolyada::Protocol;
use vertep_kolyada::shape::{FieldDecl, Shape, ShapeDecl, payload_graph};
use vertep_rdf::shacl::Shapes;

/// The two verdicts on one payload: the checker's and full SHACL's.
fn verdicts(shape: &Shape, payload: &Value) -> (bool, bool) {
    let checker = shape.validate(payload).is_ok();
    let shapes = Shapes::from_graph(&shape.to_shacl()).expect("the subset is SHACL Core");
    let shacl = payload_graph(payload).is_some_and(|g| shapes.validate(&g).conforms());
    (checker, shacl)
}

/// A small deterministic generator, so a shrunk case names a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[usize::try_from(self.below(items.len() as u64)).unwrap_or(0)]
    }
}

const NAMES: [&str; 6] = ["item", "qty", "price", "a b", "é", "n"];
const DATATYPES: [&str; 4] = ["string", "integer", "number", "boolean"];

/// A value of `datatype` (any scalar when `None`), varied by `k`.
fn scalar(datatype: Option<&str>, r: &mut Rng) -> Value {
    let k = r.below(5);
    match datatype {
        Some("string") => json!(["a", "b", "EUR", "", "ü"][k as usize]),
        Some("integer") => json!([0_i64, 1, -7, 42, i64::MAX][k as usize]),
        Some("number") => {
            [json!(1.5), json!(3), json!(-0.25), json!(1e300), json!(0)][k as usize].clone()
        }
        Some("boolean") => json!(k.is_multiple_of(2)),
        _ => {
            let dt = *r.pick(&DATATYPES);
            scalar(Some(dt), r)
        }
    }
}

fn arb_decl(r: &mut Rng, depth: u32) -> ShapeDecl {
    let n = 1 + r.below(3);
    let mut fields = Vec::new();
    let mut names: Vec<&str> = NAMES.to_vec();
    for _ in 0..n {
        let i = usize::try_from(r.below(names.len() as u64)).unwrap_or(0);
        let name = names.remove(i);
        let nested = depth < 2 && r.chance(25);
        let datatype = if nested || r.chance(20) {
            None
        } else {
            Some((*r.pick(&DATATYPES)).to_owned())
        };
        let allowed: Option<Vec<Value>> = (!nested && r.chance(30)).then(|| {
            let mut list = Vec::new();
            for _ in 0..1 + r.below(3) {
                let v = scalar(datatype.as_deref(), r);
                if !list.contains(&v) {
                    list.push(v);
                }
            }
            list
        });
        // A satisfiable shape: no more required values than distinct values exist.
        let distinct = match (&allowed, datatype.as_deref()) {
            (Some(list), _) => list.len() as u64,
            (None, Some("boolean")) => 2,
            _ => 3,
        };
        let min = u32::try_from(r.below(3).min(distinct)).unwrap_or(0);
        let max = r
            .chance(70)
            .then(|| min.max(1) + u32::try_from(r.below(2)).unwrap_or(0));
        fields.push(FieldDecl {
            name: name.to_owned(),
            datatype,
            min: Some(min),
            max,
            allowed,
            shape: nested.then(|| arb_decl(r, depth + 1)),
        });
    }
    ShapeDecl { fields }
}

/// A payload the checker accepts: for each field, between `min` and `max` distinct values that
/// match its datatype, its `in` list and its nested shape.
fn valid(shape: &Shape, r: &mut Rng) -> Value {
    let mut obj = Map::new();
    for f in &shape.fields {
        let hi = f.max.unwrap_or(f.min + 2);
        let count = f.min + u32::try_from(r.below(u64::from(hi - f.min) + 1)).unwrap_or(0);
        let mut values: Vec<Value> = Vec::new();
        for _ in 0..count * 16 + 16 {
            if values.len() as u32 == count {
                break;
            }
            let v = if let Some(nested) = &f.shape {
                // Shapes are open, so a member no shape names keeps nested values distinct.
                let mut o = valid(nested, r);
                if let Some(m) = o.as_object_mut() {
                    m.insert("_k".into(), json!(values.len()));
                }
                o
            } else if let Some(list) = &f.allowed {
                r.pick(list).clone()
            } else {
                let dt = f.datatype.map(|d| match d {
                    vertep_kolyada::shape::Datatype::String => "string",
                    vertep_kolyada::shape::Datatype::Integer => "integer",
                    vertep_kolyada::shape::Datatype::Number => "number",
                    vertep_kolyada::shape::Datatype::Boolean => "boolean",
                });
                scalar(dt, r)
            };
            if !values.contains(&v) {
                values.push(v);
            }
        }
        match values.len() {
            0 if r.chance(50) => {}
            0 => {
                obj.insert(f.name.clone(), Value::Null);
            }
            1 if r.chance(70) => {
                obj.insert(f.name.clone(), values.remove(0));
            }
            _ => {
                obj.insert(f.name.clone(), Value::Array(values));
            }
        }
    }
    if r.chance(30) {
        // Shapes are open: a member no shape names is not checked.
        obj.insert("extra".into(), json!({ "free": [1, "x"] }));
    }
    Value::Object(obj)
}

/// One near miss: a small change that often, not always, breaks the payload.
fn mutate(v: &mut Value, r: &mut Rng) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    let keys: Vec<String> = obj.keys().cloned().collect();
    let key = if keys.is_empty() || r.chance(15) {
        (*r.pick(&NAMES)).to_owned()
    } else {
        r.pick(&keys).clone()
    };
    let current = obj.get(&key).cloned().unwrap_or(Value::Null);
    let replacement = match r.below(11) {
        0 => {
            obj.remove(&key);
            return;
        }
        1 => Value::Null,
        // A second, different value.
        2 => match current {
            Value::Array(mut items) => {
                items.push(scalar(None, r));
                Value::Array(items)
            }
            other => json!([other, scalar(None, r)]),
        },
        // The same value twice: one value in RDF, so one value for the checker.
        3 => json!([current.clone(), current]),
        // An integral number written with a fraction is a double, not an integer.
        4 => json!(1.0),
        5 => json!("1"),
        6 => json!(true),
        7 => json!({ "item": "a" }),
        8 => json!([]),
        9 => json!([[1]]),
        _ => {
            if let Value::Object(mut inner) = current {
                mutate_inner(&mut inner, r);
                Value::Object(inner)
            } else {
                scalar(None, r)
            }
        }
    };
    obj.insert(key, replacement);
}

fn mutate_inner(obj: &mut Map<String, Value>, r: &mut Rng) {
    let mut v = Value::Object(std::mem::take(obj));
    mutate(&mut v, r);
    if let Value::Object(m) = v {
        *obj = m;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // @tg: verifies:: [[openspec:agent-protocols#Payload Shapes]] {level: unit}
    #[test]
    fn subset_and_full_shacl_agree(seed in 1u64..u64::MAX) {
        let mut r = Rng(seed);
        let decl = arb_decl(&mut r, 0);
        let shape = Shape::from_decl(&decl).expect("generated shapes are in the subset");
        let v = valid(&shape, &mut r);
        let (c, s) = verdicts(&shape, &v);
        prop_assert!(c, "generated payload is valid: {} for {:?}", v, decl);
        prop_assert_eq!(c, s, "valid payload {} for {:?}", v, decl);
        for _ in 0..4 {
            let mut m = v.clone();
            mutate(&mut m, &mut r);
            let (c, s) = verdicts(&shape, &m);
            prop_assert_eq!(c, s, "near miss {} for {:?}", m, decl);
        }
    }
}

fn shape(fields: Value) -> Shape {
    let decl: ShapeDecl = serde_json::from_value(json!({ "fields": fields })).expect("decl");
    Shape::from_decl(&decl).expect("in the subset")
}

// @tg: verifies:: [[openspec:agent-protocols#Payload Shapes]] {level: unit}
#[test]
fn agreement_on_the_boundaries() {
    let one_int = shape(json!([{ "name": "n", "datatype": "integer", "min": 1, "max": 1 }]));
    let number = shape(json!([{ "name": "n", "datatype": "number", "in": [1, 2.5] }]));
    let nested = shape(json!([{ "name": "o", "shape": { "fields": [] } }]));
    let flag = shape(json!([{ "name": "f", "datatype": "boolean", "min": 1 }]));
    let cases: Vec<(&Shape, Value, bool)> = vec![
        (&one_int, json!({ "n": 1 }), true),
        (&one_int, json!({ "n": 1.0 }), false),
        (&one_int, json!({ "n": "1" }), false),
        (&one_int, json!({ "n": [1, 1] }), true),
        (&one_int, json!({ "n": [1, 2] }), false),
        (&one_int, json!({ "n": null }), false),
        (&one_int, json!({ "n": [] }), false),
        (&one_int, json!({ "n": [null] }), false),
        (&one_int, json!({ "n": u64::MAX }), true),
        (&number, json!({ "n": 2.5 }), true),
        (&number, json!({ "n": 1 }), true),
        (&number, json!({ "n": 1.0 }), false),
        (&number, json!({ "n": [[1]] }), false),
        (&nested, json!({ "o": {} }), true),
        (&nested, json!({ "o": 3 }), false),
        (&nested, json!({ "o": [{}, {}] }), true),
        (&flag, json!({ "f": false }), true),
        (&flag, json!({ "f": "false" }), false),
        (&flag, json!([true]), false),
    ];
    for (s, v, ok) in cases {
        assert_eq!(verdicts(s, &v), (ok, ok), "{v}");
    }
}

// @tg: verifies:: [[openspec:agent-protocols#Payload Shapes]] {level: unit}
#[test]
fn agreement_on_every_vector_payload() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/verdicts.json");
    let vectors: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vectors")).expect("json");
    let protocol =
        Protocol::parse(vectors["protocol"].to_string().as_bytes()).expect("the protocol parses");
    let mut checked = 0;
    for step in vectors["steps"].as_array().expect("steps") {
        let Some(body) = step.pointer("/message/body") else {
            continue;
        };
        let kind = body["type"].as_str().unwrap_or_default();
        let Some(shape) = protocol.messages().get(kind).and_then(|m| m.shape.as_ref()) else {
            continue;
        };
        let (c, s) = verdicts(shape, &body["payload"]);
        assert_eq!(c, s, "vector payload {} of {kind}", body["payload"]);
        checked += 1;
    }
    assert!(checked >= 2, "the vectors carry shaped payloads");
}

#[test]
fn objects_in_an_in_list_are_outside_the_subset() {
    let decl: ShapeDecl =
        serde_json::from_value(json!({ "fields": [{ "name": "a", "in": [{ "x": 1 }] }] }))
            .expect("decl");
    assert!(Shape::from_decl(&decl).is_err());
}
