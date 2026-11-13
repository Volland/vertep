//! A record of custody decisions for replay through the Lean custody model
//! (`vertep-custody --history HISTORY.jsonl`, strict mode): custodians' promises and accepted
//! batches, elections and commits, as JSON lines.

use std::sync::Mutex;

use serde_json::{Value, json};

use super::config::CustodyConfig;

/// A custody history: the configuration line, then events in the order they happened.
#[derive(Debug)]
pub struct History {
    lines: Mutex<Vec<Value>>,
}

impl History {
    /// An empty history of an object with `config`.
    pub fn new(config: &CustodyConfig) -> std::sync::Arc<Self> {
        let ids: Vec<&str> = config.custodians.iter().map(|c| c.id.as_str()).collect();
        std::sync::Arc::new(History {
            lines: Mutex::new(vec![
                json!({"kind": "config", "custodians": ids, "quorum": config.quorum}),
            ]),
        })
    }

    /// Appends one event.
    pub fn record(&self, event: Value) {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    /// The events so far, configuration first.
    pub fn events(&self) -> Vec<Value> {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The history as JSON lines.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for l in self.events() {
            out.push_str(&l.to_string());
            out.push('\n');
        }
        out
    }
}
