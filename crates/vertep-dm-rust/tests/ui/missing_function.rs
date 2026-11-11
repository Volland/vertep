use vertep_dm_rust::statechart;

statechart! {
    name: orders,
    events { pay }
    initial open;
    state open { on pay => paid / notify; }
    final paid;
}

fn main() {}
