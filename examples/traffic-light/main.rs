//! Traffic light: load an SCXML chart and drive it through the pure core interpreter.
//!
//! The core never reads a clock or sleeps. `session::start` and `session::step` take the
//! session as a value plus one input and return the next session and a list of *effects*:
//! "deliver `tick` in 30 s", "cancel `light-timer`". This program is the smallest possible host
//! for those effects: a virtual clock, a list of pending timers and a script of external events.
//! It prints the configuration (the active states) after every macrostep.
//!
//! Illustrates the `scxml-interpretation` capability (Pure Step Function, Macrostep Execution,
//! Send And Cancel Effects) on a chart read by `scxml-format`.
//!
//! Run with `cargo run -p vertep-examples --bin traffic-light`.

use vertep::core::compile::CompiledChart;
use vertep::core::datamodel::{Environment, Event, EventType, SessionInfo};
use vertep::core::null::NullDatamodel;
use vertep::core::session::{self, Effect, Input, Outcome, StartInput};

/// The chart, next to this file.
const CHART: &str = include_str!("traffic-light.scxml");

/// External events, by the virtual time (in seconds) at which they arrive.
const SCRIPT: &[(u64, &str)] = &[(40, "button"), (50, "fault"), (60, "reset")];

/// The virtual time at which the program stops.
const END_S: u64 = 120;

/// A delayed `<send>` the host must deliver later.
struct Timer {
    due_ms: u64,
    sendid: String,
    event: Event,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Read the SCXML document into the chart IR, then validate and compile it.
    let parsed = vertep::scxml::read_str(CHART)?;
    let chart = CompiledChart::new(parsed.chart).map_err(|v| format!("invalid chart: {v:?}"))?;

    // 2. Start a session. The chart names the `null` datamodel (no data, only `In()` guards).
    let mut dm = NullDatamodel::new();
    let mut timers: Vec<Timer> = Vec::new();
    let mut now_ms = 0;
    let start = StartInput {
        info: SessionInfo {
            sessionid: "light-1".into(),
            ..SessionInfo::default()
        },
        ..StartInput::default()
    };
    let outcome = session::start(&chart, start, &mut dm).map_err(|e| format!("{e:?}"))?;
    let mut session = report(now_ms, "(start)", outcome, &mut timers);

    // 3. The event loop: deliver whichever comes first, a due timer or a scripted event.
    let mut script = SCRIPT.iter().peekable();
    loop {
        let next_timer = timers.iter().map(|t| t.due_ms).min();
        let next_script = script.peek().map(|(at, _)| at * 1000);
        let timer_first = match (next_timer, next_script) {
            (Some(t), Some(s)) => t <= s,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let at = if timer_first { next_timer } else { next_script }.expect("a next input");
        if at > END_S * 1000 {
            break;
        }
        let event = if timer_first {
            let i = timers
                .iter()
                .position(|t| t.due_ms == at)
                .expect("a due timer");
            timers.remove(i).event
        } else {
            let (_, name) = script.next().expect("a scripted event");
            Event::named(*name, EventType::External)
        };
        now_ms = at;
        let name = event.name.clone();
        // The time travels inside the input: the core sees it, but never asks for it.
        let env = Environment {
            now_ms: now_ms as i64,
            seed: 0,
        };
        let outcome = session::step(&chart, session, Input::event(event, env), &mut dm)
            .map_err(|e| format!("{e:?}"))?;
        session = report(now_ms, &name, outcome, &mut timers);
    }

    for t in &timers {
        println!(
            "stopped at {END_S}s; still pending: {} due at {}s",
            t.event.name,
            t.due_ms / 1000
        );
    }
    Ok(())
}

/// Prints one macrostep and applies its effects to the timer list; returns the new session.
fn report(now_ms: u64, input: &str, outcome: Outcome, timers: &mut Vec<Timer>) -> session::Session {
    println!(
        "[{:>4}s] {:<8} -> {}",
        now_ms / 1000,
        input,
        outcome.trace.configuration.join(" ")
    );
    for effect in outcome.effects {
        match effect {
            // No target: the event goes back to this session's external queue after the delay.
            Effect::Send(send) if send.target.is_none() => {
                println!(
                    "          schedule {} in {}s",
                    send.event.name,
                    send.delay_ms / 1000
                );
                timers.push(Timer {
                    due_ms: now_ms + send.delay_ms,
                    sendid: send.sendid,
                    event: send.event,
                });
            }
            Effect::Cancel { sendid } => {
                let before = timers.len();
                timers.retain(|t| t.sendid != sendid);
                if timers.len() < before {
                    println!("          cancel {sendid}");
                }
            }
            other => println!("          (ignored effect {other:?})"),
        }
    }
    outcome.session
}
