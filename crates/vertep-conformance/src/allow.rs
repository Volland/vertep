//! The expected-deviation allow-list (`allow-list.toml`).
//!
//! Every entry names a test, the datamodel of the variant and a non-empty reason:
//!
//! ```toml
//! [[deviation]]
//! test = "201"
//! datamodel = "ecmascript"
//! reason = "needs the Basic HTTP Event I/O Processor, which the in-memory host does not offer"
//! ```
//!
//! The allow-list is rejected when an entry has no reason, names a test or variant the suite
//! does not have, or repeats another entry. After a run, an allow-listed test that passed is a
//! failure of its own, so stale entries are removed.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::manifest::{Datamodel, Suite};

/// One expected deviation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deviation {
    /// The test identifier.
    pub test: String,
    /// The variant's datamodel.
    pub datamodel: Datamodel,
    /// Why the test is expected to fail.
    pub reason: String,
}

/// A validated allow-list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowList {
    /// The entries in file order.
    pub entries: Vec<Deviation>,
}

/// Why an allow-list was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowListError {
    /// The file is not valid TOML of the expected shape.
    Syntax(String),
    /// An entry has an empty or missing reason.
    MissingReason {
        /// The entry's test.
        test: String,
        /// The entry's datamodel.
        datamodel: String,
    },
    /// An entry names a datamodel the suite is not generated for.
    UnknownDatamodel {
        /// The entry's test.
        test: String,
        /// The datamodel named.
        datamodel: String,
    },
    /// An entry names a test (or a test variant) the suite does not have.
    UnknownTest {
        /// The entry's test.
        test: String,
        /// The entry's datamodel.
        datamodel: Datamodel,
    },
    /// Two entries name the same variant.
    Duplicate {
        /// The test.
        test: String,
        /// The datamodel.
        datamodel: Datamodel,
    },
}

impl fmt::Display for AllowListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllowListError::Syntax(m) => write!(f, "allow-list is not valid: {m}"),
            AllowListError::MissingReason { test, datamodel } => {
                write!(f, "allow-list entry {test}/{datamodel} has no reason")
            }
            AllowListError::UnknownDatamodel { test, datamodel } => {
                write!(
                    f,
                    "allow-list entry {test} names unknown datamodel `{datamodel}`"
                )
            }
            AllowListError::UnknownTest { test, datamodel } => {
                write!(
                    f,
                    "allow-list entry {test}/{datamodel} names a test the suite does not have"
                )
            }
            AllowListError::Duplicate { test, datamodel } => {
                write!(f, "allow-list entry {test}/{datamodel} appears twice")
            }
        }
    }
}

impl std::error::Error for AllowListError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    deviation: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    test: String,
    datamodel: String,
    #[serde(default)]
    reason: Option<String>,
}

/// The allow-list committed with this crate.
pub fn default_allow_list_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("allow-list.toml")
}

impl AllowList {
    /// Parse and validate allow-list text against `suite`.
    // @tg: implements:: [[openspec:scxml-conformance#Expected Deviation Allow List]]
    pub fn parse(text: &str, suite: &Suite) -> Result<Self, AllowListError> {
        let raw: RawFile =
            toml::from_str(text).map_err(|e| AllowListError::Syntax(e.to_string()))?;
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        for e in raw.deviation {
            let reason = e.reason.unwrap_or_default();
            if reason.trim().is_empty() {
                return Err(AllowListError::MissingReason {
                    test: e.test,
                    datamodel: e.datamodel,
                });
            }
            let Some(datamodel) = Datamodel::parse(&e.datamodel) else {
                return Err(AllowListError::UnknownDatamodel {
                    test: e.test,
                    datamodel: e.datamodel,
                });
            };
            if !suite.has_variant(&e.test, datamodel) {
                return Err(AllowListError::UnknownTest {
                    test: e.test,
                    datamodel,
                });
            }
            if !seen.insert((e.test.clone(), datamodel)) {
                return Err(AllowListError::Duplicate {
                    test: e.test,
                    datamodel,
                });
            }
            entries.push(Deviation {
                test: e.test,
                datamodel,
                reason: reason.trim().to_string(),
            });
        }
        Ok(AllowList { entries })
    }

    /// Read and validate the allow-list file at `path`.
    pub fn load(path: &Path, suite: &Suite) -> Result<Self, AllowListError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AllowListError::Syntax(format!("{}: {e}", path.display())))?;
        AllowList::parse(&text, suite)
    }

    /// The entry for a variant, if any.
    pub fn get(&self, test: &str, datamodel: Datamodel) -> Option<&Deviation> {
        self.entries
            .iter()
            .find(|e| e.test == test && e.datamodel == datamodel)
    }
}
