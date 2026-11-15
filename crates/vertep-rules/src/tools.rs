//! The agent tool that proposes rules ([[openspec:rule-authoring#Proposals Pass A Gate]]): an
//! agent calls it with program text and a source; the gate runs under the agent's own identity
//! and grant, and a refusal comes back as JSON the agent can repair from.

use std::sync::Arc;

use vertep_agent::tool::{Tool, ToolDefError};
use vertep_core::datamodel::Value;
use vertep_tiramemsu::TiramemsuShard;

use crate::authoring::{Action, Actor, GateContext, propose};
use crate::{Derivation, RulePolicy};

/// The IRI of the tool.
pub const PROPOSE_TOOL: &str = "vt:tool/rules/propose";

/// The capability check a host supplies.
pub type Authorize = Arc<dyn Fn(&Actor<'_>, Action) -> bool + Send + Sync>;

/// Everything the tool needs beyond what the agent passes.
pub struct ProposeToolConfig {
    /// The shard the proposals are stored in.
    pub shard: Arc<TiramemsuShard>,
    /// The live program the proposals are for.
    pub program: String,
    /// The calling agent's identity: the tool never takes it from the arguments.
    pub identity: String,
    /// The grant the agent holds.
    pub grant: String,
    /// The event names the chart declares.
    pub events: Vec<String>,
    /// Recorded decisions for the trial run.
    pub cases: Vec<Derivation>,
    /// A waiver of the trial run when there are no cases.
    pub allow_no_cases: bool,
    /// The actors the program would govern.
    pub scope: Vec<String>,
    /// The capability check.
    pub authorize: Authorize,
    /// How the rules run.
    pub policy: RulePolicy,
}

fn text_field(v: &Value, name: &str) -> Option<String> {
    match v {
        Value::Object(fields) => fields.iter().find_map(|(k, x)| match x {
            Value::String(s) if k == name => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }
}

/// A tool an agent calls to propose a rule program. The arguments are `{ text, source }`; the
/// result is `{ proposal }`; a gate refusal is the tool's error text, a JSON object with `code`,
/// `clause` and `reason`.
// @tg: implements:: [[openspec:rule-authoring#Proposals Pass A Gate]] {note: "the agent-facing entry point"}
pub fn propose_tool(cfg: ProposeToolConfig) -> Result<Tool, ToolDefError> {
    let cfg = Arc::new(cfg);
    Ok(Tool::new(
        PROPOSE_TOOL,
        "",
        "{ text: string, source: string }",
        "{ proposal: string }",
        move |args: Value| {
            let cfg = cfg.clone();
            async move {
                let text = text_field(&args, "text").ok_or("missing `text`")?;
                let source = text_field(&args, "source").ok_or("missing `source`")?;
                let by = Actor {
                    identity: &cfg.identity,
                    grant: &cfg.grant,
                };
                let ctx = GateContext {
                    events: &cfg.events,
                    cases: &cfg.cases,
                    allow_no_cases: cfg.allow_no_cases,
                    authorize: &*cfg.authorize,
                    policy: &cfg.policy,
                };
                match propose(&cfg.shard, &cfg.program, &text, &by, &source, &cfg.scope, &ctx) {
                    Ok(id) => Ok(Value::Object(vec![("proposal".to_owned(), Value::String(id))])),
                    Err(e) => Err(e.to_json()),
                }
            }
        },
    )?
    .with_description(
        "Propose a rule program. Returns the proposal id, or an error with a code, the clause and the reason; repair the program from the error and propose again.",
    ))
}
