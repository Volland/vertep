//! A ReAct support agent driven by an LLM decider, with a mock model and two tools.
//!
//! Run with `cargo run -p vertep-agent --example react_support_agent`.

#[path = "support/kit.rs"]
mod kit;

fn main() {
    let (mut host, _clock) = kit::host(kit::script(), kit::triage());
    let version = host.register(kit::chart()).expect("an agent chart");
    host.spawn(kit::AGENT, &version).expect("the agent starts");
    host.send(kit::AGENT, kit::task());
    host.run_until_idle();

    let state = host.state(kit::AGENT).expect("the agent has a state");
    println!("configuration: {:?}", state.configuration);
    println!("done data: {:?}", state.done);
    for e in host.journal(kit::AGENT) {
        println!("{:>3} {}", e.position, e.input.kind());
    }
    let usage = host.usage(kit::AGENT).expect("usage");
    println!(
        "model usage: {} calls, {} prompt + {} completion tokens",
        usage.calls, usage.prompt_tokens, usage.completion_tokens
    );
}
