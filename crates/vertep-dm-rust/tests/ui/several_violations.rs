use vertep_dm_rust::statechart;

statechart! {
    name: order,
    events { pay, finish }
    initial cart;
    state cart { on pay => paymnt; on finish => done; }
    state payment;
    final done;
    final done;
}

fn main() {}
