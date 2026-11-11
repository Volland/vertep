//! Run IRP tests on the Rust core and print the verdict, the host log and the root session's
//! macrostep trace: `cargo run -p vertep-conformance --example irp -- 215 [null]`.

use vertep_conformance::host::HostLog;
use vertep_conformance::{Datamodel, EngineUnderTest, RustCore, Suite};
use vertep_core::tape::OracleInput;
use vertep_core::trace::TraceInput;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let datamodel = args
        .iter()
        .find_map(|a| Datamodel::parse(a))
        .unwrap_or(Datamodel::Ecmascript);
    let suite = Suite::committed().expect("the committed suite reads");
    let engine = RustCore::default();
    for id in args.iter().filter(|a| Datamodel::parse(a).is_none()) {
        let Some(case) = suite.test(id).and_then(|t| suite.case(t, datamodel)) else {
            println!("{id}/{datamodel}: no such test variant");
            continue;
        };
        let run = engine.run(&case, None);
        println!("== {id}/{datamodel}: {:?}", run.verdict);
        for doc in &run.documents {
            println!("-- {}", doc.path.display());
            for l in &doc.log {
                match l {
                    HostLog::Log {
                        session,
                        label,
                        value,
                    } => println!("   log {session} {label:?} {value:?}"),
                    other => println!("   {other:?}"),
                }
            }
            for r in &doc.records {
                let input = match (&r.input, &r.trace.input) {
                    (OracleInput::Start(_), _) => "start".to_string(),
                    (_, TraceInput::Event(e)) => format!("event {e:?}"),
                    (_, other) => format!("{other:?}"),
                };
                println!("   [{}] {input}", r.trace.index);
                for m in &r.trace.microsteps {
                    println!("      {:?} {:?}", m.event, m.transitions);
                }
                println!("      config {:?}", r.trace.configuration);
                for e in &r.trace.effects {
                    println!("      effect {e:?}");
                }
            }
        }
    }
}
