use vertep_dm_rust::statechart;

statechart! {
    #[allow(unhandled_events)]
    name: orders,
    events { pay, paid_out }
    initial open;
    state open { on pay if announce => paid; }
    final paid;
}

fn announce(_: &Ctx, _: &(), emit: &mut Emit) -> bool {
    emit.raise(Event::PaidOut);
    true
}

fn main() {}
