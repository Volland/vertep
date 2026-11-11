use vertep_dm_rust::statechart;

statechart! {
    name: order,
    types { Pay = { amount: decimal } }
    events { pay: Pay }
    initial cart;
    state cart { on pay => paid; }
    final paid;
}

fn main() {
    let _ = Event::Pay { amount: "ten" };
}
