//! Regression tests for CEL datamodel gaps found by running the W3C SCXML IRP suite under CEL
//! (`vertep-conformance`, M1 task 6.10). Each test names the IRP tests that exposed the gap.

use vertep_core::datamodel::{Datamodel, SessionInfo, StateQuery, Value};
use vertep_dm_cel::CelDatamodel;

struct NoStates;

impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

fn fresh() -> CelDatamodel {
    let mut dm = CelDatamodel::default();
    dm.init_session(&SessionInfo {
        sessionid: "s1".into(),
        name: None,
        ioprocessors: Value::Undefined,
    })
    .unwrap();
    dm
}

/// IRP 240, 241, 244, 245, 354: the interpreter reads `namelist` names (and `<param
/// location>`) with `eval_value`; a bare location naming a data item reads it.
#[test]
fn bare_data_item_location_reads_its_value() {
    let mut dm = fresh();
    dm.declare("Var1").unwrap();
    dm.assign_expr("Var1", "{'a': 2}", &NoStates).unwrap();
    assert_eq!(
        dm.eval_value("Var1", &NoStates).unwrap(),
        Value::Object(vec![("a".into(), Value::Number(2.0))])
    );
    assert_eq!(
        dm.eval_value("Var1.a", &NoStates).unwrap(),
        Value::Number(2.0)
    );
    assert_eq!(
        dm.eval_value("ctx.Var1.a", &NoStates).unwrap(),
        Value::Number(2.0)
    );
    // Names that are not data items stay undeclared references.
    assert!(dm.eval_value("Var2", &NoStates).is_err());
    assert!(dm.eval_value("Var1.b", &NoStates).is_err());
    // Conditions are not locations: a bare name is still an error there.
    assert!(dm.eval_cond("Var1 != null", &NoStates).is_err());
}
