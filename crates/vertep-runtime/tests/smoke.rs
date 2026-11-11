//! A first end-to-end check.
mod common;
use common::*;

#[test]
fn a_virtual_actor_takes_a_turn() {
    let (mut node, _clock, _store) = fresh();
    node.register_type(
        &ty("toggle"),
        scxml(
            "idle",
            r#"<state id="idle"><transition event="go" target="busy"/></state>
               <state id="busy"><transition event="done" target="idle"/></state>"#,
        ),
    )
    .unwrap();
    let a = va("toggle", "k1");
    node.send(&a, ev("go")).unwrap();
    node.run_until_idle();
    let s = node.state(&a).unwrap();
    assert_eq!(s.configuration, vec!["busy"]);
    assert_eq!(s.position, 1);
    node.deactivate(&a);
    let s = node.state(&a).unwrap();
    assert_eq!(s.configuration, vec!["busy"]);
}
