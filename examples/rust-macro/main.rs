//! Rust macro: a chart written with `statechart!` on the `rust` datamodel.
//!
//! The macro builds the chart IR at compile time, validates and type-checks it there (a typo in
//! a target or an action with the wrong payload type is a compile error), and generates a typed
//! API next to it: `Ctx` (the context struct), `Event` (one variant per event, carrying its
//! payload), `State` (one variant per state) and `Machine` (a session you can `send` to).
//! Guards and actions are plain Rust functions, bound by name.
//!
//! Illustrates the `rust-dsl` capability (Statechart Macro, Generated Typed API, Rust Datamodel,
//! Guard and Action Signature Checking).
//!
//! Run with `cargo run -p vertep-examples --bin rust-macro`.

use vertep_dm_rust::{MachineError, statechart};

statechart! {
    name: turnstile,
    types { Coin = { cents: int } }
    context { credit: int = 0, price: int = 50, passes: int = 0 }
    events { coin: Coin, push, shutdown }

    initial service;
    state service {
        initial locked;
        on shutdown => off;
        state locked {
            // Targetless: stay locked and count the coin.
            on coin / add_credit;
            // Eventless: unlock as soon as there is enough credit.
            always if paid => unlocked;
        }
        state unlocked {
            on coin / add_credit;
            on push => locked / let_through;
        }
    }
    final off;
}

/// Guard: guards get the context and the payload read-only, and return a bool.
fn paid(ctx: &Ctx, _: &()) -> bool {
    ctx.credit >= ctx.price
}

/// Action: actions may change the context; `Emit` would let them raise or send events.
fn add_credit(ctx: &mut Ctx, coin: &Coin, _: &mut Emit) {
    ctx.credit += coin.cents;
}

/// Action without an emitter: the shorter signature is accepted too.
fn let_through(ctx: &mut Ctx, _: &()) {
    ctx.credit -= ctx.price;
    ctx.passes += 1;
}

fn main() -> Result<(), MachineError> {
    println!("chart version {}", *VERSION);

    // `Machine::start` runs the start macrostep; the effects (none here) are returned.
    let (mut m, _effects) = Machine::start("turnstile-1")?;
    show("(start)", &m);

    let script = [
        Event::Coin { cents: 20 },
        Event::Push, // nothing happens: the turnstile is locked
        Event::Coin { cents: 50 },
        Event::Push,
        Event::Push,
        Event::Shutdown,
    ];
    for event in script {
        let label = format!("{event:?}");
        m.send(event)?;
        show(&label, &m);
    }
    assert!(m.is_terminated(), "`off` is a top-level final state");
    Ok(())
}

fn show(label: &str, m: &Machine) {
    let ctx = m.context();
    println!(
        "{label:<22} -> {:<20} credit={:<3} passes={}",
        format!("{:?}", m.active()),
        ctx.credit,
        ctx.passes
    );
}
