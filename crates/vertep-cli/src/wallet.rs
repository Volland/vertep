//! `vt wallet`: create a wallet, issue, accept, list, show, present and verify credentials,
//! revoke what was issued, run OpenID4VCI offers, and run the conformance kit.
//!
//! A wallet is a directory: `keys/` (a file key manager), `records/` (the sealed credential
//! store, encrypted under a key only the key manager unwraps) and `wallet.json` (the DID, the
//! trusted issuers, the consent policy and the status lists of issued credentials; no private
//! key and no claim value). Credentials, requests and presentations travel as JSON files or as
//! OpenID4VC URIs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vertep_federation::{Clock, FileKeyManager, KeyManager, KeyType, SystemClock, Timestamp};
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::credential::{
    Credential, DidKeyOnly, IssueRequest, Presentation, PresentationRequest, StatusRef,
};
use vertep_wallet::history::Claims;
use vertep_wallet::status::{Bitstring, StatusList};
use vertep_wallet::store::{DirRecords, SealedStore};
use vertep_wallet::vocab;
use vertep_wallet::{Wallet, WalletConfig, WalletError, WalletRegistry};

use crate::commands::Outcome;

/// What a wallet directory keeps beside its keys and records.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct WalletFile {
    did: String,
    kid: String,
    key: String,
    #[serde(default)]
    trusted: Vec<String>,
    #[serde(default)]
    policy: RulePolicy,
    /// Issued credentials with a status entry: digest to entry.
    #[serde(default)]
    issued: std::collections::BTreeMap<String, StatusRef>,
}

fn run(f: impl FnOnce() -> Result<String, String>) -> Outcome {
    match f() {
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

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn read_file(dir: &Path) -> Result<WalletFile, String> {
    let text = std::fs::read_to_string(dir.join("wallet.json"))
        .map_err(|e| format!("no wallet in {}: {e}", dir.display()))?;
    serde_json::from_str(&text).map_err(s)
}

fn write_file(dir: &Path, f: &WalletFile) -> Result<(), String> {
    std::fs::write(dir.join("wallet.json"), serde_json::to_string_pretty(f).map_err(s)? + "\n").map_err(s)
}

fn now() -> i64 {
    SystemClock.now().0
}

/// The wallet of `dir`, with the built-in plugins, its sealed store and its consent policy.
fn open(dir: &Path) -> Result<(Wallet, WalletFile), String> {
    let f = read_file(dir)?;
    let km = Arc::new(FileKeyManager::open(dir.join("keys")).map_err(s)?);
    let registry = Arc::new(WalletRegistry::with_builtins());
    let records = DirRecords::open(dir.join("records")).map_err(s)?;
    let store = SealedStore::open(km.as_ref(), &f.key, Arc::new(records)).map_err(s)?;
    registry.register_store(vocab::STORE_SEALED, Arc::new(store));
    registry.register_consent(vocab::CONSENT_RULES, Arc::new(f.policy.clone()));
    let mut config = WalletConfig::did_key(km.as_ref(), &f.key).map_err(s)?;
    config.trusted_issuers = f.trusted.clone();
    config.status = None;
    let km: Arc<dyn KeyManager> = km;
    let w = Wallet::new(config, km, registry, Arc::new(DidKeyOnly), Arc::new(SystemClock));
    Ok((w, f))
}

/// `vt wallet create`: a new did:key wallet in `dir`; prints its DID.
// @tg: implements:: [[openspec:agent-wallet#Wallet Command]]
pub fn create(dir: &Path) -> Outcome {
    run(|| {
        if dir.join("wallet.json").exists() {
            return Err(format!("a wallet already exists in {}", dir.display()));
        }
        std::fs::create_dir_all(dir).map_err(s)?;
        let km = FileKeyManager::open(dir.join("keys")).map_err(s)?;
        let key = km.generate(KeyType::Ed25519).map_err(s)?;
        let config = WalletConfig::did_key(&km, &key).map_err(s)?;
        let f = WalletFile {
            did: config.did.clone(),
            kid: config.kid,
            key,
            ..WalletFile::default()
        };
        write_file(dir, &f)?;
        Ok(format!("did: {}\n", f.did))
    })
}

/// `vt wallet trust`: trust `issuer` when verifying.
pub fn trust(dir: &Path, issuer: &str) -> Outcome {
    run(|| {
        let mut f = read_file(dir)?;
        if !f.trusted.iter().any(|t| t == issuer) {
            f.trusted.push(issuer.to_owned());
        }
        write_file(dir, &f)?;
        Ok(format!("trusted: {issuer}\n"))
    })
}

/// `vt wallet allow`: let `verifier` (or `*`) receive `claims`.
pub fn allow(dir: &Path, verifier: &str, claims: &[String]) -> Outcome {
    run(|| {
        let mut f = read_file(dir)?;
        let names: Vec<&str> = claims.iter().map(String::as_str).collect();
        f.policy = f.policy.clone().allow(verifier, &names);
        write_file(dir, &f)?;
        Ok(format!("allowed: {verifier} {}\n", claims.join(",")))
    })
}

/// `vt wallet list`: one line per credential.
pub fn list(dir: &Path) -> Outcome {
    run(|| {
        let (w, _) = open(dir)?;
        let mut out = String::new();
        for m in w.list().map_err(s)? {
            let until = m.valid_until.map(|u| Timestamp(u).to_rfc3339()).unwrap_or_else(|| "-".into());
            out.push_str(&format!("{}  {}  {}  {}\n", m.digest, m.types.join(","), m.issuer, until));
        }
        Ok(out)
    })
}

/// `vt wallet show`: a credential's metadata and claims.
pub fn show(dir: &Path, digest: &str) -> Outcome {
    run(|| {
        let (w, _) = open(dir)?;
        let (meta, claims) = w.show(digest).map_err(s)?;
        Ok(serde_json::to_string_pretty(&json!({"metadata": meta, "claims": claims})).map_err(s)? + "\n")
    })
}

fn format_iri(name: &str) -> Result<&'static str, String> {
    Ok(match name {
        "sd-jwt" => vocab::FORMAT_SD_JWT,
        "di" | "di-jcs" => vocab::FORMAT_DI_JCS,
        "di-rdfc" => vocab::FORMAT_DI_RDFC,
        other => return Err(format!("unknown format {other} (sd-jwt, di-jcs, di-rdfc)")),
    })
}

/// Options of `vt wallet issue`.
#[derive(Clone, Debug)]
pub struct IssueArgs {
    /// The issuer's wallet.
    pub dir: PathBuf,
    /// The holder's DID (a did:key: the holder key).
    pub to: String,
    /// The credential type.
    pub type_: String,
    /// The claims (a JSON object).
    pub claims: String,
    /// `sd-jwt`, `di-jcs` or `di-rdfc`.
    pub format: String,
    /// Validity in days.
    pub valid_days: Option<i64>,
    /// A status list URL to allocate an entry from.
    pub status_list: Option<String>,
    /// Where to write the credential.
    pub out: PathBuf,
}

fn list_path(dir: &Path, url: &str) -> PathBuf {
    let name = vertep_wallet::credential::digest(url.as_bytes())[7..23].to_owned();
    dir.join("status").join(format!("{name}.json"))
}

#[derive(Serialize, Deserialize)]
struct ListFile {
    url: String,
    purpose: String,
    next: u64,
    encoded: String,
}

fn load_list(dir: &Path, url: &str) -> Result<(StatusList, u64), String> {
    let p = list_path(dir, url);
    let mut l = StatusList::new(url, "revocation");
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Ok((l, 0));
    };
    let f: ListFile = serde_json::from_str(&text).map_err(s)?;
    let bits = Bitstring::decode(&f.encoded).map_err(s)?;
    for i in 0..(vertep_wallet::status::MIN_ENTRIES as u64).max(f.next) {
        if bits.get(i) {
            l.revoke(i);
        }
    }
    for _ in 0..f.next {
        l.allocate();
    }
    Ok((l, f.next))
}

fn save_list(dir: &Path, w: &Wallet, l: &StatusList, next: u64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir.join("status")).map_err(s)?;
    let mut bits = Bitstring::default();
    for i in 0..next {
        if l.is_set(i) {
            bits.set(i);
        }
    }
    let f = ListFile {
        url: l.url.clone(),
        purpose: l.purpose.clone(),
        next,
        encoded: bits.encode().map_err(s)?,
    };
    std::fs::write(list_path(dir, &l.url), serde_json::to_string(&f).map_err(s)?).map_err(s)?;
    // the published list credential, ready for any static host
    let cred = l.publish(w.key_manager().as_ref(), &w.signing().map_err(s)?, now(), 86_400_000).map_err(s)?;
    vertep_wallet::status::DirectoryStatusPublisher(dir.join("public")).publish(&l.url, &cred).map_err(s)
}

/// `vt wallet issue`: issues a credential to a holder key and writes it to a file.
pub fn issue(a: &IssueArgs) -> Outcome {
    run(|| {
        let (w, mut f) = open(&a.dir)?;
        let claims: Value = serde_json::from_str(&a.claims).map_err(|e| format!("--claims: {e}"))?;
        let claims = claims.as_object().cloned().ok_or("--claims must be a JSON object")?;
        let holder_key = DidKeyOnly.key_of(&a.to).map_err(|e| format!("--to: {e}"))?;
        let status = match &a.status_list {
            Some(url) => {
                let (mut l, next) = load_list(&a.dir, url)?;
                let entry = l.allocate();
                save_list(&a.dir, &w, &l, next + 1)?;
                Some(entry)
            }
            None => None,
        };
        let request = IssueRequest {
            types: vec![a.type_.clone()],
            claims,
            holder_key,
            valid_from: now(),
            valid_until: a.valid_days.map(|d| now() + d * 86_400_000),
            status: status.clone(),
        };
        let (meta, cred) = w.issue(format_iri(&a.format)?, &request).map_err(s)?;
        if let Some(st) = status {
            f.issued.insert(meta.digest.clone(), st);
            write_file(&a.dir, &f)?;
        }
        let file = json!({"format": cred.format, "credential": cred.raw, "issuer": meta.issuer});
        std::fs::write(&a.out, serde_json::to_string_pretty(&file).map_err(s)? + "\n").map_err(s)?;
        Ok(format!("issued: {}\nto: {}\n", meta.digest, a.to))
    })
}

/// `vt wallet accept`: stores a credential file, or redeems an OpenID4VCI credential offer.
pub fn accept(dir: &Path, input: &str, tx_code: Option<&str>) -> Outcome {
    run(|| {
        if input.starts_with("openid-credential-offer://") {
            return accept_offer(dir, input, tx_code);
        }
        let (w, _) = open(dir)?;
        let text = std::fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(s)?;
        let cred = Credential {
            format: v["format"].as_str().ok_or("no format")?.to_owned(),
            raw: v["credential"].as_str().ok_or("no credential")?.to_owned(),
        };
        let issuer = v["issuer"].as_str().unwrap_or_default();
        let meta = w.accept(&cred, issuer).map_err(s)?;
        Ok(format!("stored: {}\nissuer: {}\n", meta.digest, meta.issuer))
    })
}

fn engine(w: Wallet, wire: Arc<dyn vertep_wallet::Wire>) -> Result<vertep_wallet::Exchanges, String> {
    vertep_wallet::Exchanges::builder(Arc::new(vertep_runtime_clock()))
        .create(w, wire)
        .map_err(s)
}

fn vertep_runtime_clock() -> SystemRuntimeClock {
    SystemRuntimeClock
}

/// The system clock as the runtime's clock.
#[derive(Clone, Copy, Debug, Default)]
struct SystemRuntimeClock;

impl vertep_runtime::Clock for SystemRuntimeClock {
    fn now_ms(&self) -> i64 {
        now()
    }
}

fn accept_offer(dir: &Path, uri: &str, tx_code: Option<&str>) -> Result<String, String> {
    use vertep_wallet::protocol::oid4vc_http::{CaptureWire, Oid4vcHolder};
    let (w, _) = open(dir)?;
    let capture = CaptureWire::default();
    let mut e = engine(w, Arc::new(capture.clone()))?;
    let meta = Oid4vcHolder::new(capture).accept_offer(&mut e, uri, tx_code).map_err(s)?;
    Ok(format!("stored: {}\nissuer: {}\n", meta.digest, meta.issuer))
}

/// `vt wallet request`: writes a presentation request for `claims`.
pub fn request(dir: &Path, claims: &[String], types: &[String], purpose: Option<&str>, out: &Path) -> Outcome {
    run(|| {
        let (w, _) = open(dir)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(s)?;
        let req = PresentationRequest {
            verifier: w.did().to_owned(),
            nonce: vertep_wallet::jose::b64(&nonce),
            claims: claims.iter().cloned().collect::<Claims>(),
            types: types.to_vec(),
            purpose: purpose.map(str::to_owned),
        };
        std::fs::write(out, serde_json::to_string_pretty(&req).map_err(s)? + "\n").map_err(s)?;
        Ok(format!("request: {}\nnonce: {}\n", out.display(), req.nonce))
    })
}

/// `vt wallet present`: answers a request file (or an `openid4vp://` request) through the
/// consent policy; writes the presentation.
pub fn present(dir: &Path, input: &str, out: Option<&Path>) -> Outcome {
    run(|| {
        if input.starts_with("openid4vp://") {
            use vertep_wallet::protocol::oid4vc_http::{CaptureWire, Oid4vcHolder};
            let (w, _) = open(dir)?;
            let capture = CaptureWire::default();
            let mut e = engine(w, Arc::new(capture.clone()))?;
            let answered = Oid4vcHolder::new(capture).respond(&mut e, input).map_err(s)?;
            return Ok(format!("{answered:?}\n"));
        }
        let (w, _) = open(dir)?;
        let text = std::fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
        let req: PresentationRequest = serde_json::from_str(&text).map_err(s)?;
        let options = w.options(&req).map_err(s)?;
        let Some(choice) = options.first() else {
            return Err(WalletError::ConsentRefused("no-match: no allowed disclosure satisfies the request".into()).to_string());
        };
        let (meta, p) = w.present(&choice.credential, &choice.claims, &req.verifier, &req.nonce).map_err(s)?;
        let file = json!({"format": p.format, "presentation": p.raw});
        let out = out.ok_or("--out is required for a request file")?;
        std::fs::write(out, serde_json::to_string_pretty(&file).map_err(s)? + "\n").map_err(s)?;
        let names: Vec<&String> = choice.claims.iter().collect();
        Ok(format!("presented: {}\ndisclosed: {}\n", meta.digest, names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(",")))
    })
}

/// `vt wallet verify`: checks a presentation against the request it answers.
pub fn verify(dir: &Path, request: &Path, presentation: &Path, status_dir: Option<&Path>) -> Outcome {
    run(|| {
        let (mut w, _) = open(dir)?;
        let req: PresentationRequest =
            serde_json::from_str(&std::fs::read_to_string(request).map_err(s)?).map_err(s)?;
        let v: Value = serde_json::from_str(&std::fs::read_to_string(presentation).map_err(s)?).map_err(s)?;
        let p = Presentation {
            format: v["format"].as_str().ok_or("no format")?.to_owned(),
            raw: v["presentation"].as_str().ok_or("no presentation")?.to_owned(),
        };
        if let Some(sd) = status_dir {
            let local = vertep_wallet::status::LocalStatus::new(Arc::new(DidKeyOnly));
            for entry in std::fs::read_dir(sd).map_err(s)? {
                let path = entry.map_err(s)?.path();
                let raw = std::fs::read_to_string(&path).map_err(s)?;
                if let Ok(doc) = serde_json::from_str::<Value>(&raw)
                    && let Some(id) = doc.get("id").and_then(Value::as_str)
                {
                    local.publish(id, Credential { format: vocab::FORMAT_DI_JCS.into(), raw });
                }
            }
            w.registry().register_status(vocab::STATUS_BITSTRING, Arc::new(local));
            let mut c = w.config().clone();
            c.status = Some(vocab::STATUS_BITSTRING.into());
            let km = w.key_manager().clone();
            let reg = w.registry().clone();
            w = Wallet::new(c, km, reg, Arc::new(DidKeyOnly), Arc::new(SystemClock));
        }
        match w.verify_presentation(&p, &req) {
            Ok(v) => Ok(format!(
                "valid\nissuer: {}\ncredential: {}\ndisclosed: {}\n",
                v.credential.issuer,
                v.credential.digest,
                serde_json::to_string(&v.disclosed).map_err(s)?
            )),
            Err(e) => Err(format!("invalid: {} check failed: {e}", e.check())),
        }
    })
}

/// `vt wallet revoke`: sets the status entry of an issued credential and republishes its list
/// under `public/`.
pub fn revoke(dir: &Path, digest: &str) -> Outcome {
    run(|| {
        let (w, f) = open(dir)?;
        let entry = f.issued.get(digest).ok_or_else(|| format!("{digest} was not issued with a status entry"))?;
        let (mut l, next) = load_list(dir, &entry.list)?;
        l.revoke(entry.index);
        let path = save_list(dir, &w, &l, next)?;
        Ok(format!("revoked: {digest}\nlist: {}\n", path.display()))
    })
}

/// Options of `vt wallet offer`.
#[derive(Clone, Debug)]
pub struct OfferArgs {
    /// The issuer's wallet.
    pub dir: PathBuf,
    /// Where to listen (`127.0.0.1:8080`).
    pub listen: String,
    /// The public base URL (default `http://<listen>`).
    pub base: Option<String>,
    /// The credential type.
    pub type_: String,
    /// The claims (a JSON object).
    pub claims: String,
    /// A transaction code the holder must enter.
    pub tx_code: Option<String>,
    /// Stop after this many seconds.
    pub timeout: u64,
}

/// `vt wallet offer`: serves an OpenID4VCI issuer, prints the credential offer URI, and stops
/// once the credential is issued (or at the timeout).
pub fn offer(a: &OfferArgs) -> Outcome {
    use vertep_wallet::protocol::oid4vc_http::{CaptureWire, ClientId, Configuration, EngineHandle, Oid4vcServer, ServerConfig};
    run(|| {
        let base = a.base.clone().unwrap_or_else(|| format!("http://{}", a.listen));
        let claims: Value = serde_json::from_str(&a.claims).map_err(|e| format!("--claims: {e}"))?;
        let claims = claims.as_object().cloned().ok_or("--claims must be a JSON object")?;
        let capture = CaptureWire::default();
        let (dir, cap, b) = (a.dir.clone(), capture.clone(), base.clone());
        let handle = EngineHandle::spawn(move || {
            let (w, _) = open(&dir).unwrap_or_else(|e| panic!("{e}"));
            let mut c = w.config().clone();
            c.aliases.push(b);
            let w = Wallet::new(c, w.key_manager().clone(), w.registry().clone(), Arc::new(DidKeyOnly), Arc::new(SystemClock));
            engine(w, Arc::new(cap)).unwrap_or_else(|e| panic!("{e}"))
        });
        let config = ServerConfig {
            base_url: base.clone(),
            configurations: vec![Configuration { id: a.type_.clone(), format: vocab::FORMAT_SD_JWT.into(), vct: a.type_.clone() }],
            client_id: ClientId::Did,
        };
        let server = Oid4vcServer::new(handle.clone(), capture, config).map_err(s)?;
        let uri = server
            .offer(
                vertep_wallet::Offer { format: vocab::FORMAT_SD_JWT.into(), types: vec![a.type_.clone()], claims, valid_until: None, status: None },
                a.tx_code.as_deref(),
            )
            .map_err(s)?;
        println!("{uri}");
        let listener = std::net::TcpListener::bind(&a.listen).map_err(s)?;
        let router = server.router();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(s)?;
        let timeout = a.timeout;
        rt.block_on(async move {
            listener.set_nonblocking(true).map_err(s)?;
            let l = tokio::net::TcpListener::from_std(listener).map_err(s)?;
            let serve = axum::serve(l, router);
            let done = async {
                let start = std::time::Instant::now();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let issued = handle.call(|e| e.exchanges().iter().any(|x| e.issued(&x.id).is_some())).unwrap_or(false);
                    if issued || start.elapsed().as_secs() >= timeout {
                        return issued;
                    }
                }
            };
            tokio::select! {
                r = serve => r.map(|_| String::new()).map_err(s),
                issued = done => Ok(if issued { "issued\n".to_owned() } else { "timed out\n".to_owned() }),
            }
        })
    })
}

/// `vt wallet conformance`: runs the conformance kit for the built-in protocols and checks
/// every history with the Lean checker when it is built.
pub fn conformance(seeds: u64) -> Outcome {
    run(|| {
        let mut out = String::new();
        let mut failed = false;
        for (name, issuing, presenting) in [
            ("vce", vocab::PROTOCOL_VCE, vocab::PROTOCOL_VCE),
            ("oid4vc", vocab::PROTOCOL_OID4VCI, vocab::PROTOCOL_OID4VP),
        ] {
            let r = vertep_wallet::testkit::conformance(issuing, presenting, seeds).map_err(s)?;
            let lean = if r.checked {
                format!("{} of {} accepted by the Lean checker", r.accepted, r.histories)
            } else {
                "not checked: the Lean checker vertep-wallet is not built".to_owned()
            };
            out.push_str(&format!("{name}: {} histories, {} presentations accepted, {lean}\n", r.histories, r.presentations));
            if !r.rejected.is_empty() {
                failed = true;
                for rej in &r.rejected {
                    out.push_str(&format!("  rejected: {}\n", rej.trim()));
                }
            }
        }
        if failed { Err(out) } else { Ok(out) }
    })
}
