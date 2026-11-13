//! Publishing DID logs: the `did:webvh` 1.0 file layout and the publishers that write it.
//!
//! Every publisher writes one layout below a root directory: `<domain>/.well-known/` for a DID
//! without a path, `<domain>/<path...>/` for one with a path, holding `did.jsonl`,
//! `did-witness.json` (when the DID has witness proofs) and `did.json` (the `did:web` form).
//! Every publisher verifies the complete log with its witness proofs before it writes anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{WebvhError, io};
use crate::log::DidLog;

/// The directory of a DID's files, relative to a publishing root.
///
/// # Errors
///
/// [`WebvhError::Malformed`] when `did` is not a `did:webvh` DID with a domain, or its path has
/// empty, `.` or `..` segments.
// @tg: implements:: [[openspec:webvh-hosting#Published File Layout]]
pub fn log_dir(did: &str) -> Result<PathBuf, WebvhError> {
    let rest = did
        .strip_prefix("did:webvh:")
        .ok_or_else(|| WebvhError::Malformed(format!("{did} is not a did:webvh DID")))?;
    let mut parts = rest.split(':');
    let _scid = parts.next();
    let host = parts
        .next()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| WebvhError::Malformed(format!("{did} has no domain")))?
        .replace("%3A", ":")
        .replace("%3a", ":");
    if host.contains('/') || host.contains('\\') || host == "." || host == ".." {
        return Err(WebvhError::Malformed(format!("{did}: bad domain {host}")));
    }
    let path: Vec<&str> = parts.collect();
    if path
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['/', '\\']))
    {
        return Err(WebvhError::Malformed(format!("{did}: bad path")));
    }
    let mut dir = PathBuf::from(host);
    if path.is_empty() {
        dir.push(".well-known");
    } else {
        for p in path {
            dir.push(p);
        }
    }
    Ok(dir)
}

/// The files of a DID as they are published: `(relative path, contents)`. The log is verified
/// first.
///
/// # Errors
///
/// The verification errors of [`DidLog::verify`]; [`WebvhError::Malformed`] for a DID without
/// a layout.
pub fn did_files(log: &DidLog) -> Result<Vec<(PathBuf, Option<String>)>, WebvhError> {
    log.verify()?;
    let dir = log_dir(log.did())?;
    let web = log.did_web_document()?;
    Ok(vec![
        (dir.join("did.jsonl"), Some(log.jsonl())),
        (dir.join("did-witness.json"), log.witness_json()),
        (
            dir.join("did.json"),
            Some(serde_json::to_string_pretty(&web).unwrap_or_default()),
        ),
    ])
}

/// What a publish wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    /// The files written (absolute, below the publisher's root).
    pub files: Vec<PathBuf>,
    /// What the external step reported (git's or the command's output), if any.
    pub detail: String,
}

/// Puts a DID's files where its web host serves them.
pub trait LogPublisher: Send + Sync {
    /// Verifies `log` and publishes its files.
    ///
    /// # Errors
    ///
    /// The verification error, or why writing or the external step failed. A failed publish is
    /// never reported as success.
    fn publish(&self, log: &DidLog) -> Result<Published, WebvhError>;

    /// The log of `did` as this publisher last wrote it, if any.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Io`] or [`WebvhError::Malformed`] when the files cannot be read.
    fn current(&self, did: &str) -> Result<Option<DidLog>, WebvhError>;
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), WebvhError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    fs::write(&tmp, contents).map_err(|e| io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| io(path, e))
}

fn write_layout(root: &Path, log: &DidLog) -> Result<Vec<PathBuf>, WebvhError> {
    let files = did_files(log)?;
    let mut written = Vec::new();
    for (rel, contents) in files {
        let path = root.join(rel);
        match contents {
            Some(c) => {
                write_atomic(&path, &c)?;
                written.push(path);
            }
            None => {
                if path.exists() {
                    fs::remove_file(&path).map_err(|e| io(&path, e))?;
                    written.push(path);
                }
            }
        }
    }
    Ok(written)
}

fn read_layout(root: &Path, did: &str) -> Result<Option<DidLog>, WebvhError> {
    let dir = root.join(log_dir(did)?);
    let log_path = dir.join("did.jsonl");
    if !log_path.exists() {
        return Ok(None);
    }
    let log = fs::read_to_string(&log_path).map_err(|e| io(&log_path, e))?;
    let wpath = dir.join("did-witness.json");
    let witness = if wpath.exists() {
        Some(fs::read_to_string(&wpath).map_err(|e| io(&wpath, e))?)
    } else {
        None
    };
    DidLog::from_files(&log, witness.as_deref()).map(Some)
}

/// Writes the layout below a directory a web server serves.
#[derive(Clone, Debug)]
pub struct DirectoryPublisher {
    root: PathBuf,
}

impl DirectoryPublisher {
    /// A publisher writing below `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DirectoryPublisher { root: root.into() }
    }

    /// The root.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl LogPublisher for DirectoryPublisher {
    // @tg: implements:: [[openspec:webvh-hosting#Log Publishers]] {confidence: partial, note: "the directory publisher"}
    fn publish(&self, log: &DidLog) -> Result<Published, WebvhError> {
        Ok(Published {
            files: write_layout(&self.root, log)?,
            detail: String::new(),
        })
    }

    fn current(&self, did: &str) -> Result<Option<DidLog>, WebvhError> {
        read_layout(&self.root, did)
    }
}

fn run(cmd: &mut Command, what: &str) -> Result<String, WebvhError> {
    let out = cmd
        .output()
        .map_err(|e| WebvhError::Publish(format!("{what}: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(WebvhError::Publish(format!(
            "{what} exited with {}: {}",
            out.status
                .code()
                .map_or_else(|| "a signal".to_owned(), |c| format!("status {c}")),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Writes the layout into a git working copy, then commits and pushes it (for GitHub Pages,
/// GitLab Pages or any host that deploys from a repository).
#[derive(Clone, Debug)]
pub struct GitPublisher {
    worktree: PathBuf,
    subdir: PathBuf,
    remote: String,
    branch: Option<String>,
}

impl GitPublisher {
    /// A publisher committing in the working copy `worktree` and pushing to `origin`.
    pub fn new(worktree: impl Into<PathBuf>) -> Self {
        GitPublisher {
            worktree: worktree.into(),
            subdir: PathBuf::new(),
            remote: "origin".into(),
            branch: None,
        }
    }

    /// Writes the layout below `subdir` of the working copy (for example `docs` for Pages).
    #[must_use]
    pub fn subdir(mut self, subdir: impl Into<PathBuf>) -> Self {
        self.subdir = subdir.into();
        self
    }

    /// Pushes to `remote` instead of `origin`.
    #[must_use]
    pub fn remote(mut self, remote: impl Into<String>) -> Self {
        self.remote = remote.into();
        self
    }

    /// Pushes `HEAD` to `branch` instead of the current branch's upstream.
    #[must_use]
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    fn git(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("-C").arg(&self.worktree);
        c
    }
}

impl LogPublisher for GitPublisher {
    // @tg: implements:: [[openspec:webvh-hosting#Log Publishers]] {confidence: partial, note: "the git publisher"}
    fn publish(&self, log: &DidLog) -> Result<Published, WebvhError> {
        let root = self.worktree.join(&self.subdir);
        let files = write_layout(&root, log)?;
        let mut add = self.git();
        add.arg("add").arg("-A").arg("--");
        for f in &files {
            add.arg(f);
        }
        run(&mut add, "git add")?;
        let staged = self
            .git()
            .args(["diff", "--cached", "--quiet"])
            .status()
            .map_err(|e| WebvhError::Publish(format!("git diff: {e}")))?;
        let mut detail = String::new();
        if !staged.success() {
            let msg = format!(
                "Publish {} version {}",
                log.did(),
                log.version_id().unwrap_or("?")
            );
            detail.push_str(&run(
                self.git().args(["commit", "-q", "-m", &msg]),
                "git commit",
            )?);
        }
        let mut push = self.git();
        push.args(["push", "-q", &self.remote]);
        if let Some(b) = &self.branch {
            push.arg(format!("HEAD:{b}"));
        }
        detail.push_str(&run(&mut push, "git push")?);
        Ok(Published { files, detail })
    }

    fn current(&self, did: &str) -> Result<Option<DidLog>, WebvhError> {
        read_layout(&self.worktree.join(&self.subdir), did)
    }
}

/// Writes the layout below a directory, then runs a command (for example `aws s3 sync` or
/// `wrangler pages deploy`). The command runs in the root with `VT_WEBVH_DID` and
/// `VT_WEBVH_DIR` (the DID's directory) set.
#[derive(Clone, Debug)]
pub struct CommandPublisher {
    root: PathBuf,
    program: String,
    args: Vec<String>,
}

impl CommandPublisher {
    /// A publisher writing below `root` and then running `program` with `args`.
    pub fn new(root: impl Into<PathBuf>, program: impl Into<String>, args: Vec<String>) -> Self {
        CommandPublisher {
            root: root.into(),
            program: program.into(),
            args,
        }
    }
}

impl LogPublisher for CommandPublisher {
    // @tg: implements:: [[openspec:webvh-hosting#Log Publishers]] {confidence: partial, note: "the command publisher"}
    fn publish(&self, log: &DidLog) -> Result<Published, WebvhError> {
        let files = write_layout(&self.root, log)?;
        let dir = self.root.join(log_dir(log.did())?);
        let detail = run(
            Command::new(&self.program)
                .args(&self.args)
                .current_dir(&self.root)
                .env("VT_WEBVH_DID", log.did())
                .env("VT_WEBVH_DIR", &dir),
            &self.program,
        )?;
        Ok(Published { files, detail })
    }

    fn current(&self, did: &str) -> Result<Option<DidLog>, WebvhError> {
        read_layout(&self.root, did)
    }
}
