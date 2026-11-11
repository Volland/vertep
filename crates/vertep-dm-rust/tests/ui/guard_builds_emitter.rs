use vertep_dm_rust::statechart;

statechart! {
    #[allow(unhandled_events)]
    name: orders,
    events { pay, paid_out }
    initial open;
    state open { on pay if announce => paid; }
    final paid;
}

fn announce(_: &Ctx, _: &()) -> bool {
    let mut emit = vertep_dm_rust::Emit::<Event>::new(Default::default());
    emit.raise(Event::PaidOut);
    true
}

fn main() {}
