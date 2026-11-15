//! `vt rules`: rule programs beside a shard's journal, bundles that verify offline, and
//! counterfactual replay under another program ([[openspec:rule-programs#Rule Programs Live In The Shard]]).
//!
//! Needs the native engine library, so the commands work only in a build with the `rules`
//! feature; otherwise they print `rules-unavailable` and exit with 2.

use std::path::Path;

use vertep_rules::{
    Derivation, RuleBundle, RulePolicy, RulesError, check_program, counterfactual, program_text,
    put_program,
};
use vertep_tiramemsu::TiramemsuShard;

use crate::commands::Outcome;

fn done(r: Result<String, String>) -> Outcome {
    match r {
        Ok(stdout) => Outcome {
            stdout,
            ..Outcome::default()
        },
        Err(e) => Outcome {
            stderr: format!("{e}\n"),
            code: if e.starts_with("rules-unavailable") {
                2
            } else {
                1
            },
            ..Outcome::default()
        },
    }
}

/// The commands need the engine library, which only a build with the `rules` feature links.
fn engine() -> Result<(), String> {
    if cfg!(feature = "rules") {
        Ok(())
    } else {
        Err(RulesError::Unavailable.to_string())
    }
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn shard(db: &Path) -> Result<TiramemsuShard, String> {
    engine()?;
    TiramemsuShard::open(db).map_err(s)
}

fn read(path: &Path) -> Result<String, String> {
    engine()?;
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn derivations(path: &Path) -> Result<Vec<Derivation>, String> {
    serde_json::from_str(&read(path)?).map_err(s)
}

/// `vt rules put DB PROGRAM FILE`: store or supersede a program.
pub fn put(db: &Path, program: &str, file: &Path) -> Outcome {
    done((|| {
        let text = read(file)?;
        put_program(&shard(db)?, program, &text).map_err(s)?;
        Ok(format!("stored {program}\n"))
    })())
}

/// `vt rules show DB PROGRAM [--as-of TX]`: print a program's source.
pub fn show(db: &Path, program: &str, as_of: Option<u64>) -> Outcome {
    done((|| program_text(&shard(db)?, program, as_of).map_err(s))())
}

/// `vt rules check DB PROGRAM [--fuel N]`: load a program and run it once over the current data.
pub fn check(db: &Path, program: &str, fuel: u64) -> Outcome {
    done((|| {
        let r = check_program(&shard(db)?, program, fuel).map_err(s)?;
        Ok(format!(
            "ok: {} conclusions, {} steps{}\n",
            r.conclusions,
            r.fuel_used,
            if r.non_monotonic {
                ", non-monotonic"
            } else {
                ""
            }
        ))
    })())
}

/// A line diff by longest common subsequence.
fn line_diff(a: &str, b: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            out.push_str(&format!("- {}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        }
    }
    for l in &a[i..] {
        out.push_str(&format!("- {l}\n"));
    }
    for l in &b[j..] {
        out.push_str(&format!("+ {l}\n"));
    }
    out
}

/// `vt rules diff DB PROGRAM --from TX --to TX`: the lines that changed between two versions.
pub fn diff(db: &Path, program: &str, from: u64, to: u64) -> Outcome {
    done((|| {
        let sh = shard(db)?;
        let (a, b) = (
            program_text(&sh, program, Some(from)).map_err(s)?,
            program_text(&sh, program, Some(to)).map_err(s)?,
        );
        let d = line_diff(&a, &b);
        Ok(if d.is_empty() {
            "no difference\n".to_owned()
        } else {
            d
        })
    })())
}

/// `vt rules bundle DB DERIVATIONS NODE [--out FILE]`: export one decision's bundle.
pub fn bundle(db: &Path, derivations_file: &Path, node: &str, out: Option<&Path>) -> Outcome {
    done((|| {
        let d = derivations(derivations_file)?
            .into_iter()
            .find(|d| d.node == node)
            .ok_or_else(|| format!("no derivation for {node}"))?;
        let json = RuleBundle::export(&shard(db)?, &d).map_err(s)?.to_json();
        match out {
            Some(p) => {
                std::fs::write(p, &json).map_err(s)?;
                Ok(format!("wrote {}\n", p.display()))
            }
            None => Ok(format!("{json}\n")),
        }
    })())
}

/// `vt rules verify-bundle FILE`: re-run a decision offline from its bundle alone.
pub fn verify_bundle(file: &Path) -> Outcome {
    done((|| {
        let b = RuleBundle::from_json(&read(file)?).map_err(s)?;
        b.verify(&RulePolicy::default()).map_err(|e| match e {
            RulesError::Unavailable => s(e),
            e => format!("verification failed: {e}"),
        })?;
        Ok(format!(
            "verified: {} concluded {} from {} statements\n",
            b.derivation.node,
            b.derivation.event,
            b.derivation.support.len()
        ))
    })())
}

/// `vt rules counterfactual DB DERIVATIONS PROGRAM_FILE`: where recorded decisions would differ
/// under another program version. Writes nothing.
pub fn counterfactual_cmd(db: &Path, derivations_file: &Path, program_file: &Path) -> Outcome {
    done((|| {
        let r = counterfactual(
            &shard(db)?,
            &derivations(derivations_file)?,
            &read(program_file)?,
            &RulePolicy::default(),
        )
        .map_err(s)?;
        let mut out = format!("{} unchanged, {} differ\n", r.unchanged, r.differs.len());
        for d in &r.differs {
            out.push_str(&format!(
                "{}: {} -> {}{}\n",
                d.node,
                d.recorded_event,
                d.counterfactual_event.as_deref().unwrap_or("(none)"),
                d.note
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            ));
        }
        Ok(out)
    })())
}
