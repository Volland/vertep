use vertep_dm_rust::statechart;

statechart! {
    #[deny(unhandled_events)]
    name: orders,
    events { pay, refund }
    initial open;
    state open { on pay => paid; }
    final paid;
}

fn main() {}
