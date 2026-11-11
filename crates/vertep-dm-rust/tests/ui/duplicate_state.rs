use vertep_dm_rust::statechart;

statechart! {
    name: order,
    events { finish }
    initial work;
    state work { on finish => done; }
    final done;
    final done;
}

fn main() {}
