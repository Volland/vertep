//! Errors of the rules crate.

use core::fmt;

/// Why a rules operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RulesError {
    /// The crate was built without the `native` feature, or the platform has no engine library.
    /// A chart that declares a rules decider must not fall back to another decider.
    Unavailable,
    /// The store refused or failed a read or write.
    Store(String),
    /// The program text does not load, or a run failed.
    Program(String),
    /// The step budget ran out.
    FuelExhausted {
        /// The budget.
        limit: u64,
    },
    /// The derivation was made by another engine version than the one running.
    EngineMismatch {
        /// Recorded on the entry.
        recorded: String,
        /// Running now.
        running: String,
    },
    /// An enforced program reported violations; nothing was changed.
    Violations(Vec<vertep_store::Violation>),
    /// Replay reached a different conclusion, support or closure than the record.
    Diverged(String),
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RulesError::Unavailable => f.write_str(
                "rules-unavailable: built without the `native` feature or on a platform without the engine library",
            ),
            RulesError::Store(m) => write!(f, "rules store: {m}"),
            RulesError::Program(m) => write!(f, "rules program: {m}"),
            RulesError::FuelExhausted { limit } => {
                write!(f, "fuel-exhausted: the budget of {limit} steps ran out")
            }
            RulesError::EngineMismatch { recorded, running } => {
                write!(f, "engine version differs: recorded {recorded}, running {running}")
            }
            RulesError::Violations(v) => {
                write!(f, "violations: ")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{} ({})", x.constraint, x.statements.join(" "))?;
                }
                Ok(())
            }
            RulesError::Diverged(m) => write!(f, "replay diverged: {m}"),
        }
    }
}

impl std::error::Error for RulesError {}
