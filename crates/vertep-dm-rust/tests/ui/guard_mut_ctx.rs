use vertep_dm_rust::statechart;

statechart! {
    name: orders,
    context { count: int = 0 }
    events { pay }
    initial open;
    state open { on pay if counted => paid; }
    final paid;
}

fn counted(ctx: &mut Ctx, _: &()) -> bool {
    ctx.count += 1;
    true
}

fn main() {}
