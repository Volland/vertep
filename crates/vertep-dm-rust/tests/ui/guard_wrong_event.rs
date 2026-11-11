use vertep_dm_rust::{Decimal, statechart};

statechart! {
    name: orders,
    types { Pay = { amount: decimal }, Cancel = { reason: string } }
    events { pay: Pay, cancel: Cancel }
    initial open;
    state open { on pay if is_large => review; on cancel => closed; }
    state review;
    final closed;
}

fn is_large(_: &Ctx, ev: &Cancel) -> bool {
    ev.reason.is_empty()
}

fn main() {
    let _ = Decimal::ZERO;
}
