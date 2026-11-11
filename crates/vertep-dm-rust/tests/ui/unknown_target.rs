use vertep_dm_rust::statechart;

statechart! {
    name: order,
    events { pay }
    initial cart;
    state cart { on pay => paymnt; }
    state payment;
}

fn main() {}
