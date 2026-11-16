//! `vt explain` and `vt verify-bundle`: read an explanation bundle, verify it offline, and print
//! what it explains ([[openspec:decision-records#Explanation Bundle]]).
//!
//! A bundle is exported by a host (`AgentHost::export_bundle`); these commands need no runtime,
//! decider or network.

use std::path::Path;

use vertep_agent::bundle::Bundle;
use vertep_agent::explain::Explanation;

use crate::commands::Outcome;

fn done(r: Result<String, String>) -> Outcome {
    match r {
        Ok(stdout) => Outcome {
            stdout,
            ..Outcome::default()
        },
        Err(e) => Outcome {
            stderr: format!("{e}\n"),
            code: 1,
            ..Outcome::default()
        },
    }
}

fn load(file: &Path) -> Result<Bundle, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    Bundle::from_json(&text).map_err(|e| e.to_string())
}

/// `vt verify-bundle FILE`: verify a bundle offline.
// @tg: implements:: [[openspec:decision-records#Explanation Bundle]] {note: "the command"}
pub fn verify_bundle(file: &Path) -> Outcome {
    done((|| {
        let b = load(file)?;
        let v = b
            .verify()
            .map_err(|e| format!("verification failed: {e}"))?;
        Ok(format!(
            "verified: {} positions {} to {} ({} explanations recomputed, {} decider calls)\n",
            b.actor,
            b.from,
            b.to,
            v.explanations.len(),
            v.decider_calls
        ))
    })())
}

fn line(x: &Explanation) -> String {
    let mut out = format!(
        "entry {} ({}) {} -> {}",
        x.position,
        x.input,
        render(&x.state_before),
        render(&x.state_after)
    );
    if x.replay == "recorded" {
        out.push_str("\n    replay: recorded (erased data; not a full verification)");
    }
    if let Some(d) = &x.decision {
        out.push_str(&format!(
            "\n    {}: offered {}, chose {}",
            d.disposition,
            if d.offered_known {
                render(&d.offered)
            } else {
                "unknown".to_owned()
            },
            d.chosen.as_deref().unwrap_or("nothing")
        ));
        if let Some(by) = d.decider.as_deref() {
            out.push_str(&format!(", decided by {by}"));
        }
        if let Some(c) = &d.claim {
            out.push_str(&format!(
                "\n    {} ({}): {}",
                c.label,
                c.claimed_by,
                c.rationale.as_deref().unwrap_or("(no text)")
            ));
        }
    }
    for e in &x.effects {
        out.push_str(&format!("\n    then: {e}"));
    }
    out
}

fn render(v: &[String]) -> String {
    format!("[{}]", v.join(", "))
}

/// `vt explain FILE [--position N] [--json]`: verify a bundle, then print its explanations.
pub fn explain(file: &Path, position: Option<u64>, json: bool) -> Outcome {
    done((|| {
        let b = load(file)?;
        let v = b
            .verify()
            .map_err(|e| format!("verification failed: {e}"))?;
        let chosen: Vec<&Explanation> = v
            .explanations
            .iter()
            .filter(|x| position.is_none_or(|p| x.position == p))
            .collect();
        if chosen.is_empty() {
            return Err(format!(
                "no explanation for position {}",
                position.unwrap_or(0)
            ));
        }
        if json {
            let all: Vec<&Explanation> = chosen;
            return Ok(format!(
                "{}\n",
                serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?
            ));
        }
        let mut out = format!("{} (verified offline)\n", b.actor);
        for x in chosen {
            out.push_str(&line(x));
            out.push('\n');
        }
        Ok(out)
    })())
}
