use vertep_dm_rust::statechart;

statechart! {
    name: orders,
    events { ship }
    initial open;
    state open { on ship => shipped; }
    state shipped;
    state archived;
}

fn main() {
    // Compilation succeeds (with a warning) and the chart runs.
    let (mut m, _) = Machine::start("s").unwrap();
    m.send(Event::Ship).unwrap();
    assert!(m.is_in(State::Shipped));
}
