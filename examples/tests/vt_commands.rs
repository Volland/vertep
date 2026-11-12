//! The `vt` commands shown in examples/README.md, run through the `vertep-cli` library (the
//! `vt` binary prints exactly these outcomes) and compared with the outputs recorded under
//! each example's `vt/` directory.
//!
//! Chart paths are given relative to the repository root, as in the README, so the paths inside
//! error messages match what a reader sees. That needs the working directory to be the root:
//! this file holds a single test, so changing it does not race with other tests.
//!
//! Re-record after an intended change with `VERTEP_BLESS=1 cargo test -p vertep-examples`.

mod support;

use std::path::Path;

use support::{check_recorded, examples_dir};
use vertep_cli::{Outcome, SchemaFormat, Surface, commands};

/// Asserts the exit code and an empty standard error, and returns standard output.
fn ok(o: Outcome) -> String {
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stderr.is_empty(), "{}", o.stderr);
    o.stdout
}

#[test]
fn vt_commands() {
    std::env::set_current_dir(examples_dir().join("..")).expect("the repository root");

    // traffic-light: validate, hash and convert an SCXML chart.
    let light = Path::new("examples/traffic-light/traffic-light.scxml");
    let validate = ok(commands::validate(light));
    check_recorded("traffic-light/vt/validate.txt", &validate);
    let hash = ok(commands::hash(light));
    check_recorded("traffic-light/vt/hash.txt", &hash);
    let yaml = ok(commands::convert(light, Surface::Yaml, None));
    check_recorded("traffic-light/vt/traffic-light.yaml", &yaml);
    let turtle = ok(commands::convert(light, Surface::Turtle, None));
    check_recorded("traffic-light/vt/traffic-light.ttl", &turtle);
    // Every surface of the chart has the same content-addressed version.
    for converted in ["traffic-light.yaml", "traffic-light.ttl"] {
        let p = format!("examples/traffic-light/vt/{converted}");
        assert_eq!(ok(commands::hash(Path::new(&p))), hash, "{converted}");
    }

    // typed-order: type-check, project the types, generate TypeScript.
    let order = Path::new("examples/typed-order/order.yaml");
    check_recorded(
        "typed-order/vt/typecheck.txt",
        &ok(commands::typecheck(order)),
    );
    let schemas = [
        (SchemaFormat::JsonSchema, "typed-order/vt/schema.json"),
        (SchemaFormat::Cel, "typed-order/vt/schema-cel.txt"),
        (SchemaFormat::Shacl, "typed-order/vt/schema-shacl.ttl"),
    ];
    for (format, rel) in schemas {
        check_recorded(rel, &ok(commands::schema(order, format)));
    }
    check_recorded(
        "typed-order/vt/order.ts",
        &ok(commands::gen_ts(order, None)),
    );

    // The ill-typed variant fails with exit code 1 and a located error on standard error.
    let bad = commands::typecheck(Path::new("examples/typed-order/order-ill-typed.yaml"));
    assert_eq!(bad.code, 1);
    assert!(bad.stdout.is_empty());
    assert!(
        bad.stderr.contains("order-ill-typed.yaml:44:19:"),
        "{}",
        bad.stderr
    );
    check_recorded("typed-order/vt/typecheck-ill-typed.txt", &bad.stderr);
}
