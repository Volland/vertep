//! `vt webvh`: create, update, rotate, deactivate, verify, witness, publish and serve
//! `did:webvh` DIDs, and create durable objects and their custody changes.
//!
//! Keys live in a file key manager directory (`--keys`). Beside the key files, `dids/<did>/`
//! holds what the controller needs and no private key: `controller.json` (update key names,
//! the identity's key names, the object controller's key name) and the working log
//! (`did.jsonl`, `did-witness.json`), which `publish` puts on a site once it verifies.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vertep_federation::{
    ActorIdentity, FileKeyManager, KeyManager, KeyType, PublicKey, Service, ServiceEndpoint,
};
use vertep_objects::KeyManagerSigner;
use vertep_objects::custody::{Custodian, CustodyConfig, StaticKeyDirectory};
use vertep_webvh::document::{
    bind, identity_document, object_document, provision, replace_service,
};
use vertep_webvh::witness::{CustodianWitness, custody_change, custody_witnessing};
use vertep_webvh::{
    Author, CommandPublisher, CreateOptions, DidLog, DirectoryPublisher, GitPublisher,
    LogPublisher, UpdateKeys, WebvhError, Witnessing,
};

use crate::commands::Outcome;

fn ok(stdout: String) -> Outcome {
    Outcome {
        stdout,
        ..Outcome::default()
    }
}

fn fail(code: i32, msg: impl std::fmt::Display) -> Outcome {
    Outcome {
        stderr: format!("{msg}\n"),
        code,
        ..Outcome::default()
    }
}

/// Runs `f`; an `Err` becomes exit status 1 with the message on standard error.
fn run(f: impl FnOnce() -> Result<String, String>) -> Outcome {
    match f() {
        Ok(out) => ok(out),
        Err(e) => fail(1, e),
    }
}

/// What the controller of a DID keeps beside its key files (no private key).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ControllerFile {
    did: String,
    update: UpdateKeys,
    identity: ActorIdentity,
    #[serde(default)]
    controller_key: Option<String>,
}

fn keys_at(dir: &Path) -> Result<Arc<dyn KeyManager>, String> {
    Ok(Arc::new(
        FileKeyManager::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?,
    ))
}

fn did_dir(keys: &Path, did: &str) -> PathBuf {
    keys.join("dids").join(did.replace([':', '/', '\\'], "_"))
}

fn save(keys: &Path, c: &ControllerFile, log: &DidLog) -> Result<PathBuf, String> {
    let dir = did_dir(keys, &c.did);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let w = |name: &str, text: &str| {
        let p = dir.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
    };
    w(
        "controller.json",
        &serde_json::to_string_pretty(c).map_err(|e| e.to_string())?,
    )?;
    w("did.jsonl", &log.jsonl())?;
    match log.witness_json() {
        Some(j) => w("did-witness.json", &j)?,
        None => {
            let _ = std::fs::remove_file(dir.join("did-witness.json"));
        }
    }
    Ok(dir.join("did.jsonl"))
}

fn read_log(path: &Path) -> Result<DidLog, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let wpath = path.with_file_name("did-witness.json");
    let witness = std::fs::read_to_string(&wpath).ok();
    DidLog::from_files(&text, witness.as_deref()).map_err(|e| e.to_string())
}

fn load(keys: &Path, did: &str) -> Result<(ControllerFile, DidLog), String> {
    let dir = did_dir(keys, did);
    let c: ControllerFile = serde_json::from_str(
        &std::fs::read_to_string(dir.join("controller.json"))
            .map_err(|e| format!("{did} is not controlled from {}: {e}", keys.display()))?,
    )
    .map_err(|e| format!("controller.json: {e}"))?;
    Ok((c, read_log(&dir.join("did.jsonl"))?))
}

/// Parses `TYPE=URL` into a service with id `#<type in lower case>`.
fn service(spec: &str) -> Result<Service, String> {
    let (ty, url) = spec
        .split_once('=')
        .ok_or_else(|| format!("--service {spec}: expected TYPE=URL"))?;
    Ok(Service {
        id: format!("#{}", ty.to_ascii_lowercase()),
        types: vec![ty.to_owned()],
        endpoints: vec![ServiceEndpoint::uri(url)],
    })
}

/// Parses `ROLE,DID,MULTIKEY` (role `host` or `keeper`; the custodian's Ed25519 signing key).
fn custodian(spec: &str) -> Result<(Custodian, PublicKey), String> {
    let mut parts = spec.splitn(3, ',');
    let (Some(role), Some(did), Some(key)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("--custodian {spec}: expected ROLE,DID,MULTIKEY"));
    };
    let c = match role {
        "host" => Custodian::host(did),
        "keeper" => Custodian::keeper(did),
        other => {
            return Err(format!(
                "--custodian {spec}: role {other} is not host or keeper"
            ));
        }
    };
    let pk = PublicKey::from_multikey(key).map_err(|e| format!("--custodian {spec}: {e}"))?;
    Ok((c, pk))
}

fn custody_of(
    specs: &[String],
    quorum: Option<usize>,
    version: u64,
) -> Result<(CustodyConfig, StaticKeyDirectory), String> {
    let directory = StaticKeyDirectory::new();
    let mut custodians = Vec::new();
    for s in specs {
        let (c, pk) = custodian(s)?;
        directory.add_verification(&c.id, pk);
        custodians.push(c);
    }
    let mut config = match quorum {
        Some(q) => CustodyConfig::new(custodians, q),
        None => CustodyConfig::majority(custodians),
    }
    .map_err(|e| e.to_string())?;
    config.version = version;
    Ok((config, directory))
}

/// Publishes the working log to `out` when it is fully witnessed; describes what is missing
/// otherwise.
fn maybe_publish(log: &DidLog, out: Option<&Path>) -> Result<String, String> {
    if let Some((have, need)) = log.last_entry_witnessing().map_err(|e| e.to_string())?
        && have < need
    {
        return Ok(format!(
            "witnesses: {have} of {need} proofs for {}; run `vt webvh witness` then `vt webvh publish`\n",
            log.version_id().unwrap_or("?")
        ));
    }
    let Some(out) = out else {
        return Ok(String::new());
    };
    let p = DirectoryPublisher::new(out)
        .publish(log)
        .map_err(|e| e.to_string())?;
    Ok(p.files
        .iter()
        .map(|f| format!("published: {}\n", f.display()))
        .collect())
}

/// The arguments of `vt webvh create`.
#[derive(Clone, Debug, Default)]
pub struct CreateArgs {
    /// The domain.
    pub domain: String,
    /// The path (`a:b` or `a/b`).
    pub path: Option<String>,
    /// The key manager directory.
    pub keys: PathBuf,
    /// The site to publish to.
    pub out: Option<PathBuf>,
    /// Announce the next update key.
    pub pre_rotation: bool,
    /// The `ttl` parameter.
    pub ttl: Option<u32>,
    /// Services as `TYPE=URL`.
    pub services: Vec<String>,
    /// Witness `did:key` DIDs.
    pub witnesses: Vec<String>,
    /// The witness threshold (default: all).
    pub threshold: Option<u32>,
}

/// `vt webvh create`: a new DID from a new identity; prints the DID first.
// @tg: implements:: [[openspec:webvh-hosting#vt webvh Commands]]
pub fn create(a: &CreateArgs) -> Outcome {
    run(|| {
        let keys = keys_at(&a.keys)?;
        let mut options = CreateOptions::new(&a.domain).pre_rotation(a.pre_rotation);
        if let Some(p) = &a.path {
            options = options.path(p);
        }
        if let Some(t) = a.ttl {
            options = options.ttl(t);
        }
        if !a.witnesses.is_empty() {
            options = options.witnesses(Witnessing {
                witnesses: a.witnesses.clone(),
                threshold: a
                    .threshold
                    .unwrap_or(u32::try_from(a.witnesses.len()).unwrap_or(u32::MAX)),
            });
        }
        let services = a
            .services
            .iter()
            .map(|s| service(s))
            .collect::<Result<Vec<_>, _>>()?;
        let mut identity =
            provision(keys.as_ref(), &options, services).map_err(|e| e.to_string())?;
        let doc = identity_document(&identity, keys.as_ref()).map_err(|e| e.to_string())?;
        let mut author =
            Author::generate(keys.clone(), a.pre_rotation).map_err(|e| e.to_string())?;
        let log = author.create(&options, &doc).map_err(|e| e.to_string())?;
        bind(&mut identity, log.did());
        let c = ControllerFile {
            did: log.did().to_owned(),
            update: author.update_keys().clone(),
            identity,
            controller_key: None,
        };
        let working = save(&a.keys, &c, &log)?;
        let mut out = format!("{}\nlog: {}\n", log.did(), working.display());
        out.push_str(&maybe_publish(&log, a.out.as_deref())?);
        Ok(out)
    })
}

/// The arguments of `vt webvh object create`.
#[derive(Clone, Debug, Default)]
pub struct ObjectArgs {
    /// The domain.
    pub domain: String,
    /// The path.
    pub path: String,
    /// The key manager directory.
    pub keys: PathBuf,
    /// Custodians as `ROLE,DID,MULTIKEY`.
    pub custodians: Vec<String>,
    /// The quorum (default: the smallest majority).
    pub quorum: Option<usize>,
    /// Services as `TYPE=URL`.
    pub services: Vec<String>,
}

/// `vt webvh object create`: a durable object's DID, declaring its custodians, naming them as
/// witnesses at the quorum, with pre-rotation and a controller key.
pub fn object_create(a: &ObjectArgs) -> Outcome {
    run(|| {
        let keys = keys_at(&a.keys)?;
        let (config, directory) = custody_of(&a.custodians, a.quorum, 1)?;
        let witnessing = custody_witnessing(&config, &directory).map_err(|e| e.to_string())?;
        let options = CreateOptions::new(&a.domain)
            .path(&a.path)
            .pre_rotation(true)
            .witnesses(witnessing);
        let services = a
            .services
            .iter()
            .map(|s| service(s))
            .collect::<Result<Vec<_>, _>>()?;
        let mut identity =
            provision(keys.as_ref(), &options, services).map_err(|e| e.to_string())?;
        let controller = keys.generate(KeyType::Ed25519).map_err(|e| e.to_string())?;
        let controller_pk = keys.public_key(&controller).map_err(|e| e.to_string())?;
        let doc = object_document(&identity, keys.as_ref(), &config, &controller_pk)
            .map_err(|e| e.to_string())?;
        let mut author = Author::generate(keys.clone(), true).map_err(|e| e.to_string())?;
        let log = author.create(&options, &doc).map_err(|e| e.to_string())?;
        bind(&mut identity, log.did());
        let c = ControllerFile {
            did: log.did().to_owned(),
            update: author.update_keys().clone(),
            identity,
            controller_key: Some(controller),
        };
        let working = save(&a.keys, &c, &log)?;
        let mut out = format!("{}\nlog: {}\n", log.did(), working.display());
        out.push_str(&format!(
            "custodians: {} (quorum {})\n",
            config.custodians.len(),
            config.quorum
        ));
        out.push_str(&maybe_publish(&log, None)?);
        Ok(out)
    })
}

/// What `update` changes.
#[derive(Clone, Debug, Default)]
pub struct UpdateArgs {
    /// The DID.
    pub did: String,
    /// The key manager directory.
    pub keys: PathBuf,
    /// The site to publish to.
    pub out: Option<PathBuf>,
    /// Services to add or replace (by type), as `TYPE=URL`.
    pub services: Vec<String>,
    /// A whole new document (JSON).
    pub document: Option<PathBuf>,
}

fn with_author(
    keys_dir: &Path,
    did: &str,
    out: Option<&Path>,
    f: impl FnOnce(&mut Author, &mut DidLog, &ControllerFile) -> Result<String, WebvhError>,
) -> Outcome {
    run(|| {
        let keys = keys_at(keys_dir)?;
        let (mut c, mut log) = load(keys_dir, did)?;
        let mut author = Author::new(keys, c.update.clone());
        let vid = f(&mut author, &mut log, &c).map_err(|e| e.to_string())?;
        c.update = author.update_keys().clone();
        let working = save(keys_dir, &c, &log)?;
        let mut s = format!(
            "{}\nversion: {vid}\nlog: {}\n",
            log.did(),
            working.display()
        );
        s.push_str(&maybe_publish(&log, out)?);
        Ok(s)
    })
}

/// `vt webvh update`.
pub fn update(a: &UpdateArgs) -> Outcome {
    let services = match a
        .services
        .iter()
        .map(|s| service(s))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(s) => s,
        Err(e) => return fail(2, e),
    };
    let document = match &a.document {
        Some(p) => match std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()))
        {
            Ok(d) => Some(d),
            Err(e) => return fail(2, format!("{}: {e}", p.display())),
        },
        None => None,
    };
    with_author(&a.keys, &a.did, a.out.as_deref(), |author, log, _| {
        let mut doc = document
            .clone()
            .or_else(|| log.document().cloned())
            .ok_or_else(|| WebvhError::Malformed("an empty log".into()))?;
        for s in &services {
            let mut s = s.clone();
            s.id = format!("{}{}", log.did(), s.id);
            let ty = s.types[0].clone();
            doc = replace_service(&doc, &ty, s)?;
        }
        author.update(log, doc)
    })
}

/// `vt webvh rotate`.
pub fn rotate(did: &str, keys: &Path, out: Option<&Path>) -> Outcome {
    with_author(keys, did, out, |author, log, _| author.rotate(log, None))
}

/// `vt webvh deactivate`.
pub fn deactivate(did: &str, keys: &Path, out: Option<&Path>) -> Outcome {
    with_author(keys, did, out, |author, log, _| author.deactivate(log))
}

/// `vt webvh verify`: verifies a `did.jsonl` with the `did-witness.json` beside it.
pub fn verify(log: &Path) -> Outcome {
    let parsed = match read_log(log) {
        Ok(l) => l,
        Err(e) => return fail(2, e),
    };
    match parsed.verify() {
        Ok(v) => ok(format!(
            "valid: {}\nversions: {}\nversion: {}\ndeactivated: {}\n",
            v.did, v.versions, v.version_id, v.deactivated
        )),
        Err(e) => fail(1, format!("invalid: {e}")),
    }
}

/// `vt webvh witness`: adds the witness proof of key `key` for the last entry of the log at
/// `log`, writing `did-witness.json` beside it.
pub fn witness(log_path: &Path, keys: &Path, key: &str) -> Outcome {
    run(|| {
        let km = keys_at(keys)?;
        let mut log = read_log(log_path)?;
        let w = CustodianWitness::new(km, key).map_err(|e| e.to_string())?;
        let vid = log.version_id().ok_or("an empty log")?.to_owned();
        let proof = w.prove(&vid).map_err(|e| e.to_string())?;
        log.add_witness_proof(&vid, &proof)
            .map_err(|e| e.to_string())?;
        let wpath = log_path.with_file_name("did-witness.json");
        std::fs::write(&wpath, log.witness_json().unwrap_or_default())
            .map_err(|e| format!("{}: {e}", wpath.display()))?;
        let status = match log.last_entry_witnessing().map_err(|e| e.to_string())? {
            Some((have, need)) => format!("{have} of {need} proofs"),
            None => "no witnesses required".into(),
        };
        Ok(format!("witnessed {vid} as {}\n{status}\n", w.did_key()))
    })
}

/// Where `publish` puts a log.
#[derive(Clone, Debug)]
pub enum Target {
    /// A directory.
    Dir(PathBuf),
    /// A git working copy.
    Git {
        /// The working copy.
        worktree: PathBuf,
        /// The directory inside it.
        subdir: Option<PathBuf>,
        /// The branch to push to.
        branch: Option<String>,
    },
    /// A directory, then a command.
    Command {
        /// The directory.
        root: PathBuf,
        /// The command and its arguments.
        command: Vec<String>,
    },
}

/// `vt webvh publish`: verifies the working log of `did` and publishes it.
pub fn publish(did: &str, keys: &Path, target: &Target) -> Outcome {
    run(|| {
        let (_, log) = load(keys, did)?;
        let publisher: Box<dyn LogPublisher> = match target {
            Target::Dir(d) => Box::new(DirectoryPublisher::new(d)),
            Target::Git {
                worktree,
                subdir,
                branch,
            } => {
                let mut g = GitPublisher::new(worktree);
                if let Some(s) = subdir {
                    g = g.subdir(s);
                }
                if let Some(b) = branch {
                    g = g.branch(b);
                }
                Box::new(g)
            }
            Target::Command { root, command } => {
                let (prog, args) = command.split_first().ok_or("--command needs a program")?;
                Box::new(CommandPublisher::new(root, prog, args.to_vec()))
            }
        };
        let p = publisher.publish(&log).map_err(|e| e.to_string())?;
        let mut s: String = p
            .files
            .iter()
            .map(|f| format!("published: {}\n", f.display()))
            .collect();
        s.push_str(&p.detail);
        Ok(s)
    })
}

/// `vt webvh serve`: serves a site until stopped.
pub fn serve(root: &Path, addr: &str, domain: Option<String>) -> Outcome {
    eprintln!("serving {} on http://{addr}", root.display());
    match vertep_webvh::server::serve_blocking(root, addr, domain) {
        Ok(()) => ok(String::new()),
        Err(e) => fail(2, format!("serve: {e}")),
    }
}

/// The arguments of `vt webvh custody change`.
#[derive(Clone, Debug, Default)]
pub struct ChangeArgs {
    /// The object's DID.
    pub did: String,
    /// The key manager directory.
    pub keys: PathBuf,
    /// The new custodians, as `ROLE,DID,MULTIKEY`.
    pub custodians: Vec<String>,
    /// The new quorum.
    pub quorum: Option<usize>,
    /// Where to write the signed request (standard output when absent).
    pub request_out: Option<PathBuf>,
}

/// `vt webvh custody change`: the next log entry declaring the new custodian set and its
/// witnesses, and the controller-signed reconfiguration request carrying it.
// @tg: implements:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries]] {confidence: partial, note: "vt webvh custody change"}
pub fn custody_change_cmd(a: &ChangeArgs) -> Outcome {
    run(|| {
        let keys = keys_at(&a.keys)?;
        let (mut c, mut log) = load(&a.keys, &a.did)?;
        let controller_key = c.controller_key.clone().ok_or(
            "this DID has no object controller key (create it with `vt webvh object create`)",
        )?;
        let current = log
            .document()
            .and_then(|d| vertep_federation::DidDocument::from_json(d).ok())
            .and_then(|d| CustodyConfig::from_document(&d).ok())
            .ok_or("the current document declares no custodians")?;
        let (config, directory) = custody_of(&a.custodians, a.quorum, current.version + 1)?;
        let mut author = Author::new(keys.clone(), c.update.clone());
        let controller = KeyManagerSigner::new(&c.did, keys, controller_key);
        let request = custody_change(&mut author, &mut log, &config, &directory, &controller)
            .map_err(|e| e.to_string())?;
        c.update = author.update_keys().clone();
        let working = save(&a.keys, &c, &log)?;
        let json = serde_json::to_string_pretty(&request).map_err(|e| e.to_string())?;
        let mut s = format!(
            "{}\nversion: {}\nlog: {}\n",
            log.did(),
            log.version_id().unwrap_or("?"),
            working.display()
        );
        match &a.request_out {
            Some(p) => {
                std::fs::write(p, &json).map_err(|e| format!("{}: {e}", p.display()))?;
                s.push_str(&format!("request: {}\n", p.display()));
            }
            None => {
                s.push_str(&json);
                s.push('\n');
            }
        }
        Ok(s)
    })
}
