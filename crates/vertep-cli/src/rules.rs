//! `vt rules`: rule programs beside a shard's journal, bundles that verify offline, and
//! counterfactual replay under another program ([[openspec:rule-programs#Rule Programs Live In The Shard]]).
//!
//! Needs the native engine library, so the commands work only in a build with the `rules`
//! feature; otherwise they print `rules-unavailable` and exit with 2.

use std::path::Path;

use vertep_rules::authoring::{self, Action, Actor, GateContext, Stage};
use vertep_rules::{
    Derivation, RuleBundle, RulePolicy, RulesError, check_profile, check_program, counterfactual,
    enforce, materialize, program_text, put_program,
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

/// `vt rules enforce DB PROGRAM` and `vt rules unenforce DB PROGRAM`: turn commit-time
/// enforcement on or off. Turning it on checks existing data first and is refused, with the
/// violations listed, if live data violates the program.
pub fn enforce_cmd(db: &Path, program: &str, on: bool) -> Outcome {
    done((|| {
        enforce(&shard(db)?, program, on).map_err(s)?;
        Ok(format!(
            "{} {program}\n",
            if on {
                "enforcing"
            } else {
                "no longer enforcing"
            }
        ))
    })())
}

/// `vt rules materialize DB PROGRAM [--fuel N]`: write the program's conclusions as derived facts.
pub fn materialize_cmd(db: &Path, program: &str, fuel: u64) -> Outcome {
    done((|| {
        let m = materialize(&shard(db)?, program, fuel).map_err(s)?;
        Ok(format!(
            "derived {}, kept {}, retracted {}{}\n",
            m.derived.len(),
            m.existing,
            m.retracted.len(),
            if m.non_monotonic {
                " (non-monotonic: rerun after writes)"
            } else {
                ""
            }
        ))
    })())
}

/// Who may do what, from a JSON file: `{"propose": ["id", ...], "promote": ["id", ...]}`.
#[derive(serde::Deserialize, Default)]
struct Grants {
    #[serde(default)]
    propose: Vec<String>,
    #[serde(default)]
    promote: Vec<String>,
}

fn grants(path: Option<&Path>) -> Result<Grants, String> {
    match path {
        Some(p) => serde_json::from_str(&read(p)?).map_err(s),
        None => Ok(Grants::default()),
    }
}

/// Arguments of `vt rules propose`.
pub struct ProposeArgs<'a> {
    /// The shard's tiramemsu file.
    pub db: &'a Path,
    /// The live program IRI the proposal is for.
    pub program: &'a str,
    /// The proposed source.
    pub file: &'a Path,
    /// The proposing identity.
    pub by: &'a str,
    /// The grant presented.
    pub grant: &'a str,
    /// Where the proposal came from.
    pub source: &'a str,
    /// The event names the chart declares.
    pub events: Vec<String>,
    /// Recorded decisions for the trial run.
    pub cases: Option<&'a Path>,
    /// Waive the trial run when there are no cases.
    pub allow_no_cases: bool,
    /// The actors the program would govern.
    pub scope: Vec<String>,
    /// The grants file.
    pub grants: Option<&'a Path>,
}

/// `vt rules propose`: put a program forward through the gate; prints the proposal id.
pub fn propose(a: &ProposeArgs<'_>) -> Outcome {
    done((|| {
        let g = grants(a.grants)?;
        let cases = match a.cases {
            Some(p) => derivations(p)?,
            None => Vec::new(),
        };
        let policy = RulePolicy::default();
        let authorize = |who: &Actor<'_>, act: Action| match act {
            Action::Propose => g.propose.iter().any(|i| i == who.identity),
            Action::Promote => g.promote.iter().any(|i| i == who.identity),
        };
        let ctx = GateContext {
            events: &a.events,
            cases: &cases,
            allow_no_cases: a.allow_no_cases,
            authorize: &authorize,
            policy: &policy,
        };
        let by = Actor {
            identity: a.by,
            grant: a.grant,
        };
        let id = authoring::propose(
            &shard(a.db)?,
            a.program,
            &read(a.file)?,
            &by,
            a.source,
            &a.scope,
            &ctx,
        )
        .map_err(|e| format!("refused: {e}"))?;
        Ok(format!("proposed {id}\n"))
    })())
}

/// `vt rules shadow DB PROPOSAL CASES`: run a proposal beside the live program on recorded cases.
pub fn shadow_cmd(db: &Path, proposal: &str, cases: &Path) -> Outcome {
    done((|| {
        let r = authoring::shadow(
            &shard(db)?,
            proposal,
            &derivations(cases)?,
            &RulePolicy::default(),
        )
        .map_err(|e| format!("refused: {e}"))?;
        let mut out = format!("{} unchanged, {} differ\n", r.unchanged, r.differs.len());
        for d in &r.differs {
            out.push_str(&format!(
                "{}: {} -> {}\n",
                d.node,
                d.recorded_event,
                d.counterfactual_event.as_deref().unwrap_or("(none)")
            ));
        }
        Ok(out)
    })())
}

/// `vt rules promote DB PROPOSAL --to STAGE --by ID`: move a proposal one stage along.
pub fn promote_cmd(
    db: &Path,
    proposal: &str,
    to: &str,
    by: &str,
    grant: &str,
    scope: Option<&str>,
    grants_file: Option<&Path>,
) -> Outcome {
    done((|| {
        let to = match to {
            "shadow" => Stage::Shadow,
            "scoped" => Stage::Scoped,
            "live" => Stage::Live,
            other => return Err(format!("unknown stage {other}: shadow, scoped or live")),
        };
        let g = grants(grants_file)?;
        let policy = RulePolicy::default();
        let authorize = |who: &Actor<'_>, act: Action| match act {
            Action::Propose => g.propose.iter().any(|i| i == who.identity),
            Action::Promote => g.promote.iter().any(|i| i == who.identity),
        };
        let ctx = GateContext {
            events: &[],
            cases: &[],
            allow_no_cases: false,
            authorize: &authorize,
            policy: &policy,
        };
        authoring::promote(
            &shard(db)?,
            proposal,
            to,
            &Actor {
                identity: by,
                grant,
            },
            scope,
            &ctx,
        )
        .map_err(|e| format!("refused: {e}"))?;
        Ok(format!("{proposal} is now {}\n", to.name()))
    })())
}

/// `vt rules profile FILE`: whether a program is inside the rules profile, and why not.
pub fn profile_cmd(file: &Path) -> Outcome {
    done((|| {
        let v = check_profile(&read(file)?);
        if v.is_empty() {
            return Ok("inside the rules profile\n".to_owned());
        }
        Err(v
            .iter()
            .map(|x| format!("outside the profile: {} [{}]", x.reason, x.clause))
            .collect::<Vec<_>>()
            .join("\n"))
    })())
}

/// `vt rules unnecessary BUNDLE`: supporting statements the conclusion does not need.
pub fn unnecessary_cmd(file: &Path) -> Outcome {
    done((|| {
        let b = RuleBundle::from_json(&read(file)?).map_err(s)?;
        let u = b.unnecessary_support().map_err(s)?;
        if u.is_empty() {
            Ok("every supporting statement is needed\n".to_owned())
        } else {
            Err(format!("support-not-necessary: {}", u.join(" ")))
        }
    })())
}
