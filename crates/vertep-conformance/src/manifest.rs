//! The committed IRP suite: `manifest.xml` and the generated datamodel variants.
//!
//! The import script (`scripts/import-irp.sh`) transforms the pinned `.txml` templates once
//! and commits the result; this module reads what was committed. Every manifest `<test>` is
//! classified as the IRP classifies it: `manual="true"` makes a test manual whatever its
//! conformance level, otherwise it is mandatory or optional. A test is offered for each
//! datamodel directory (`ecmascript/<id>/`, `null/<id>/`) that holds all its start documents.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::Event as XmlEvent;
use serde::Serialize;

/// The datamodels the IRP suite is generated for.
pub const DATAMODELS: [Datamodel; 2] = [Datamodel::Ecmascript, Datamodel::Null];

/// The datamodel of a generated test variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Datamodel {
    /// `ecmascript/<id>/`.
    Ecmascript,
    /// `null/<id>/`.
    Null,
}

impl Datamodel {
    /// The directory name, which is also the `datamodel` attribute value.
    pub fn as_str(self) -> &'static str {
        match self {
            Datamodel::Ecmascript => "ecmascript",
            Datamodel::Null => "null",
        }
    }

    /// Parse a directory or attribute name.
    pub fn parse(name: &str) -> Option<Self> {
        DATAMODELS.into_iter().find(|d| d.as_str() == name)
    }
}

impl fmt::Display for Datamodel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the IRP classifies a test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    /// `conformance="mandatory"`, not manual: part of the build gate.
    Mandatory,
    /// `conformance="optional"`, not manual: run and reported, never gating.
    Optional,
    /// `manual="true"`: listed as not run.
    Manual,
}

impl fmt::Display for Classification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Classification::Mandatory => "mandatory",
            Classification::Optional => "optional",
            Classification::Manual => "manual",
        })
    }
}

/// One `<test>` of the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestTest {
    /// The test identifier, such as `"144"`.
    pub id: String,
    /// The IRP conformance level (`mandatory` or `optional`).
    pub mandatory: bool,
    /// `manual="true"`.
    pub manual: bool,
    /// Start documents, as manifest URIs (`403/test403a.txml`).
    pub starts: Vec<String>,
    /// Dependency files, as manifest URIs.
    pub deps: Vec<String>,
}

impl ManifestTest {
    /// The classification the gate uses.
    pub fn classification(&self) -> Classification {
        if self.manual {
            Classification::Manual
        } else if self.mandatory {
            Classification::Mandatory
        } else {
            Classification::Optional
        }
    }
}

/// A test as offered for one datamodel: the documents to run, in manifest order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCase {
    /// The test identifier.
    pub id: String,
    /// The variant's datamodel.
    pub datamodel: Datamodel,
    /// The IRP classification.
    pub classification: Classification,
    /// The directory holding the variant's documents and dependencies.
    pub dir: PathBuf,
    /// The start documents; the test passes when every one passes.
    pub starts: Vec<PathBuf>,
}

/// The committed suite.
#[derive(Clone, Debug)]
pub struct Suite {
    /// The suite directory.
    pub root: PathBuf,
    /// Every manifest test, in manifest order.
    pub tests: Vec<ManifestTest>,
}

/// A problem reading the suite.
#[derive(Debug)]
pub struct SuiteError(pub String);

impl fmt::Display for SuiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SuiteError {}

/// The suite directory committed with this crate.
pub fn default_suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("suite")
}

/// Parse `manifest.xml` text into its tests.
// @tg: implements:: [[openspec:scxml-conformance#IRP Suite Import]] {confidence: partial, note: "classification of the committed suite; the import itself is scripts/import-irp.sh"}
pub fn parse_manifest(xml: &str) -> Result<Vec<ManifestTest>, SuiteError> {
    let mut reader = Reader::from_str(xml);
    let mut tests: Vec<ManifestTest> = Vec::new();
    let mut current: Option<ManifestTest> = None;
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| SuiteError(format!("manifest.xml: {e}")))?;
        match ev {
            XmlEvent::Start(e) | XmlEvent::Empty(e) => {
                let attrs: BTreeMap<String, String> = e
                    .attributes()
                    .filter_map(Result::ok)
                    .map(|a| (a.key.as_ref().to_string(), a.value.to_string()))
                    .collect();
                let get = |k: &str| {
                    attrs
                        .get(k)
                        .cloned()
                        .ok_or_else(|| SuiteError(format!("manifest.xml: a test lacks `{k}`")))
                };
                match e.name().as_ref() {
                    "test" => {
                        if let Some(t) = current.take() {
                            tests.push(t);
                        }
                        let conformance = get("conformance")?;
                        let mandatory = match conformance.as_str() {
                            "mandatory" => true,
                            "optional" => false,
                            other => {
                                return Err(SuiteError(format!(
                                    "manifest.xml: unknown conformance `{other}`"
                                )));
                            }
                        };
                        current = Some(ManifestTest {
                            id: get("id")?,
                            mandatory,
                            manual: attrs.get("manual").is_some_and(|m| m == "true"),
                            starts: Vec::new(),
                            deps: Vec::new(),
                        });
                    }
                    "start" => {
                        if let Some(t) = current.as_mut() {
                            t.starts.push(get("uri")?);
                        }
                    }
                    "dep" => {
                        if let Some(t) = current.as_mut() {
                            t.deps.push(get("uri")?);
                        }
                    }
                    _ => {}
                }
            }
            XmlEvent::End(e) if e.name().as_ref() == "test" => {
                if let Some(t) = current.take() {
                    tests.push(t);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    if let Some(t) = current.take() {
        tests.push(t);
    }
    Ok(tests)
}

impl Suite {
    /// Read the suite at `root` (the directory holding `manifest.xml`).
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, SuiteError> {
        let root = root.into();
        let path = root.join("manifest.xml");
        let xml = fs::read_to_string(&path)
            .map_err(|e| SuiteError(format!("{}: {e}", path.display())))?;
        Ok(Suite {
            root,
            tests: parse_manifest(&xml)?,
        })
    }

    /// The suite committed with this crate.
    pub fn committed() -> Result<Self, SuiteError> {
        Suite::load(default_suite_dir())
    }

    /// The manifest test with this identifier.
    pub fn test(&self, id: &str) -> Option<&ManifestTest> {
        self.tests.iter().find(|t| t.id == id)
    }

    /// The variant of `test` for `datamodel`, when every start document was generated for it.
    pub fn case(&self, test: &ManifestTest, datamodel: Datamodel) -> Option<TestCase> {
        let mut starts = Vec::new();
        let mut dir = None;
        for uri in &test.starts {
            // `.txml` templates were generated into `.scxml` charts; other start files (the
            // manual test 513 is a `.txt` description) were copied as they are.
            let rel = match uri.strip_suffix(".txml") {
                Some(stem) => PathBuf::from(format!("{stem}.scxml")),
                None => PathBuf::from(uri),
            };
            let path = self.root.join(datamodel.as_str()).join(&rel);
            if !path.is_file() {
                return None;
            }
            dir = path.parent().map(Path::to_path_buf);
            starts.push(path);
        }
        Some(TestCase {
            id: test.id.clone(),
            datamodel,
            classification: test.classification(),
            dir: dir?,
            starts,
        })
    }

    /// Every test variant, in manifest order and then datamodel order.
    pub fn cases(&self) -> Vec<TestCase> {
        self.tests
            .iter()
            .flat_map(|t| DATAMODELS.into_iter().filter_map(|d| self.case(t, d)))
            .collect()
    }

    /// Check every file listed in `SHA256SUMS` against its recorded SHA-256, and that no
    /// generated chart is missing from the list, so a suite that drifted from the pinned import
    /// is refused. Returns every mismatch.
    // @tg: implements:: [[openspec:scxml-conformance#IRP Suite Import]] {confidence: partial, note: "content-hash check of the pinned import"}
    pub fn verify_hashes(&self) -> Result<usize, Vec<String>> {
        use sha2::{Digest, Sha256};
        let sums = fs::read_to_string(self.root.join("SHA256SUMS"))
            .map_err(|e| vec![format!("SHA256SUMS: {e}")])?;
        let mut problems = Vec::new();
        let mut listed = std::collections::BTreeSet::new();
        for line in sums.lines().filter(|l| !l.trim().is_empty()) {
            let Some((hash, file)) = line.split_once("  ") else {
                problems.push(format!("malformed SHA256SUMS line `{line}`"));
                continue;
            };
            listed.insert(file.to_string());
            match fs::read(self.root.join(file)) {
                Ok(bytes) => {
                    let actual: String = Sha256::digest(&bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    if actual != hash {
                        problems.push(format!("{file}: content hash differs from SHA256SUMS"));
                    }
                }
                Err(e) => problems.push(format!("{file}: {e}")),
            }
        }
        for case in self.cases() {
            for start in &case.starts {
                let rel = start
                    .strip_prefix(&self.root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if !listed.contains(&rel) {
                    problems.push(format!("{rel}: not listed in SHA256SUMS"));
                }
            }
        }
        if problems.is_empty() {
            Ok(listed.len())
        } else {
            Err(problems)
        }
    }

    /// Whether a variant of the test exists for the datamodel.
    pub fn has_variant(&self, id: &str, datamodel: Datamodel) -> bool {
        self.test(id)
            .and_then(|t| self.case(t, datamodel))
            .is_some()
    }
}
