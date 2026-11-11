// The unreachable-state warning is a `deprecated` lint, denied here to make it visible.
#![deny(deprecated)]

use vertep_dm_rust::statechart;

statechart! {
    name: orders,
    events { ship }
    initial open;
    state open { on ship => shipped; }
    state shipped;
    state archived;
}

fn main() {}
