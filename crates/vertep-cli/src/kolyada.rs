//! `vt kolyada`: agent protocol documents (Kolyada, `vertep-kolyada`) from the command line.
//!
//! - `validate DOC` checks the profile, the payload shapes and well-formedness;
//! - `hash DOC` prints the protocol identity;
//! - `project DOC [--role R] [--json]` prints each role's projection and interface view;
//! - `offer DOC --role R --keys DIR [--key NAME]` signs an offer by the `did:key` of a key;
//! - `check DOC FILE` verifies a signed offer, or replays a message history and prints each
//!   verdict;
//! - `vectors DIR` checks a directory of published test vectors against this implementation.
//!
//! Signatures are verified offline: `did:key` and `did:peer` documents come from the DID itself,
//! and `--key DID=PUBLIC` adds an Ed25519 public key (base64url) for any other DID.
//!
//! Exit status: 0 on success, 1 when the document, offer, history or vectors fail, 2 on usage
//! and I/O errors.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use vertep_federation::keys::{FileKeyManager, InMemoryKeyManager, KeyManager, KeyType, PublicKey};
use vertep_federation::resolver::{KeyResolver, PeerResolver};
use vertep_federation::{Did, DidResolver, ResolverVerifier};
use vertep_kolyada::envelope::{SignError, Signer, Verifier, VerifyError, control};
use vertep_kolyada::offer::{Offer, OfferBody, ServiceProtocol, service_entry};
use vertep_kolyada::projection::{Action, Projection};
use vertep_kolyada::{Event, Instance, Message, Protocol, Verdict};

use crate::commands::Outcome;

/// The outcome of a command: `Ok(stdout)`, or the exit code with the message for stderr.
type Run = Result<String, (i32, String)>;

fn done(r: Run) -> Outcome {
    match r {
        Ok(stdout) => Outcome {
            stdout,
            ..Outcome::default()
        },
        Err((code, e)) => Outcome {
            stderr: format!("{e}\n"),
            code,
            ..Outcome::default()
        },
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> (i32, String) + '_ {
    move |e| (2, format!("{}: {e}", path.display()))
}

fn read(path: &Path) -> Result<Vec<u8>, (i32, String)> {
    std::fs::read(path).map_err(io(path))
}

fn json_file(path: &Path) -> Result<Value, (i32, String)> {
    serde_json::from_slice(&read(path)?).map_err(|e| (2, format!("{}: {e}", path.display())))
}

fn protocol(path: &Path) -> Result<Protocol, (i32, String)> {
    Protocol::parse(&read(path)?).map_err(|e| (1, format!("{}: {e}", path.display())))
}

/// `vt kolyada validate DOC`.
// @tg: implements:: [[openspec:agent-protocols#Protocol Document]] {note: "vt kolyada validate"}
pub fn validate(doc: &Path) -> Outcome {
    done((|| {
        let p = protocol(doc)?;
        Ok(format!(
            "valid {}\nroles: {}\nmessages: {}\ntransitions: {}\n",
            p.id(),
            p.roles().join(", "),
            p.messages().keys().cloned().collect::<Vec<_>>().join(", "),
            p.transitions().len()
        ))
    })())
}

/// `vt kolyada hash DOC`.
// @tg: implements:: [[openspec:agent-protocols#Protocol Identity]] {note: "vt kolyada hash"}
pub fn hash(doc: &Path) -> Outcome {
    done((|| Ok(format!("{}\n", protocol(doc)?.id())))())
}

fn action_json(a: &Action) -> Value {
    match a {
        Action::Send { message, receivers } => json!({ "send": message, "to": receivers }),
        Action::Receive { message, sender } => json!({ "receive": message, "from": sender }),
        Action::Silent => json!("silent"),
    }
}

/// A projection in the JSON form of the published vectors (`projection.json`).
#[must_use]
pub fn projection_json(p: &Projection) -> Value {
    let steps: Vec<Value> = p
        .steps
        .iter()
        .map(|s| json!({ "from": s.from, "to": s.to, "action": action_json(&s.action) }))
        .collect();
    json!({ "role": p.role, "initial": p.initial, "final": p.finals, "steps": steps })
}

/// `vt kolyada project DOC [--role R] [--json]`.
// @tg: implements:: [[openspec:agent-protocols#Role Projection]] {note: "vt kolyada project"}
pub fn project(doc: &Path, role: Option<&str>, as_json: bool) -> Outcome {
    done((|| {
        let p = protocol(doc)?;
        let roles: Vec<String> = match role {
            Some(r) if p.has_role(r) => vec![r.to_owned()],
            Some(r) => return Err((1, format!("the protocol declares no role {r}"))),
            None => p.roles().to_vec(),
        };
        let mut out = String::new();
        let mut list = Vec::new();
        for r in &roles {
            let (Some(proj), Some(iface)) = (p.project(r), p.interface(r)) else {
                continue;
            };
            if as_json {
                let mut v = projection_json(&proj);
                v["accepts"] = json!(iface.accepts);
                list.push(v);
                continue;
            }
            out.push_str(&format!("role {r}\n"));
            for s in &proj.steps {
                let what = match &s.action {
                    Action::Send { message, receivers } => format!(
                        "send {message} to {}",
                        receivers.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    Action::Receive { message, sender } => {
                        format!("receive {message} from {sender}")
                    }
                    Action::Silent => "silent".into(),
                };
                out.push_str(&format!("  {} -> {}: {what}\n", s.from, s.to));
            }
            for (state, accepts) in &iface.accepts {
                let items: Vec<String> = accepts
                    .iter()
                    .map(|(m, s)| format!("{m} from {s}"))
                    .collect();
                out.push_str(&format!("  accepts in {state}: {}\n", items.join(", ")));
            }
        }
        if as_json {
            out = serde_json::to_string_pretty(
                &json!({ "protocol": p.id().as_str(), "projections": list }),
            )
            .unwrap_or_default()
                + "\n";
        }
        Ok(out)
    })())
}

/// Signs with one key of a key manager, as the `did:key` of that key.
struct KmSigner {
    km: Arc<dyn KeyManager>,
    name: String,
    did: String,
    kid: String,
}

impl Signer for KmSigner {
    fn did(&self) -> &str {
        &self.did
    }
    fn key_id(&self) -> &str {
        &self.kid
    }
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignError> {
        self.km
            .sign(&self.name, data)
            .map_err(|e| SignError(e.to_string()))
    }
}

/// The arguments of `vt kolyada offer`.
#[derive(Clone, Debug, Default)]
pub struct OfferArgs {
    /// The protocol document.
    pub doc: std::path::PathBuf,
    /// The role offered.
    pub role: String,
    /// The key directory.
    pub keys: std::path::PathBuf,
    /// The key name; a new Ed25519 key when absent.
    pub key: Option<String>,
    /// When the offer lapses, in milliseconds since the epoch.
    pub expires_ms: Option<u64>,
    /// The initiators accepted; anyone when empty.
    pub initiators: Vec<String>,
}

/// `vt kolyada offer DOC --role R --keys DIR [--key NAME] [--expires-ms N] [--initiator DID]...`:
/// prints `{offer, service}`, the signed offer and the `VertepProtocol` service entry.
// @tg: implements:: [[openspec:agent-protocols#Protocol Offers]] {note: "vt kolyada offer"}
pub fn offer(a: &OfferArgs) -> Outcome {
    done((|| {
        let p = protocol(&a.doc)?;
        if !p.has_role(&a.role) {
            return Err((1, format!("the protocol declares no role {}", a.role)));
        }
        let km: Arc<dyn KeyManager> = Arc::new(
            FileKeyManager::open(&a.keys).map_err(|e| (2, format!("{}: {e}", a.keys.display())))?,
        );
        let name = match &a.key {
            Some(n) => n.clone(),
            None => km
                .generate(KeyType::Ed25519)
                .map_err(|e| (2, e.to_string()))?,
        };
        let pk = km
            .public_key(&name)
            .map_err(|e| (2, format!("key {name}: {e}")))?;
        if pk.key_type != KeyType::Ed25519 {
            return Err((2, format!("key {name} is not an Ed25519 key")));
        }
        let did = vertep_federation::resolver::key::did_key(&pk);
        let signer = KmSigner {
            km,
            name: name.clone(),
            kid: format!("{did}#{}", pk.multikey()),
            did: did.clone(),
        };
        let body = OfferBody {
            protocol: p.id().as_str().to_owned(),
            role: a.role.clone(),
            did: did.clone(),
            expires_ms: a.expires_ms,
            initiators: (!a.initiators.is_empty()).then(|| a.initiators.clone()),
        };
        let offer = Offer::sign(body, &signer).map_err(|e| (2, e.to_string()))?;
        let service = service_entry(
            &did,
            &[ServiceProtocol {
                protocol: p.id().as_str().to_owned(),
                roles: vec![a.role.clone()],
            }],
        );
        Ok(serde_json::to_string_pretty(
            &json!({ "key": name, "offer": offer, "service": service }),
        )
        .unwrap_or_default()
            + "\n")
    })())
}

/// Verifies offline: given keys first, then `did:key` and `did:peer` documents.
struct OfflineVerifier {
    keys: BTreeMap<String, PublicKey>,
}

impl OfflineVerifier {
    fn new(given: &[String]) -> Result<Self, (i32, String)> {
        let mut keys = BTreeMap::new();
        for g in given {
            let (did, b64) = g
                .split_once('=')
                .ok_or_else(|| (2, format!("--key {g}: expected DID=PUBLIC")))?;
            let bytes: [u8; 32] = URL_SAFE_NO_PAD
                .decode(b64)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| (2, format!("--key {g}: not a base64url Ed25519 key")))?;
            keys.insert(did.to_owned(), PublicKey::ed25519(bytes));
        }
        Ok(OfflineVerifier { keys })
    }
}

struct Offline;

impl DidResolver for Offline {
    fn resolve(
        &self,
        did: &Did,
    ) -> Result<vertep_federation::Resolution, vertep_federation::ResolutionError> {
        match did.method() {
            "peer" => PeerResolver.resolve(did),
            _ => KeyResolver.resolve(did),
        }
    }
}

impl Verifier for OfflineVerifier {
    fn verify(&self, did: &str, key_id: &str, data: &[u8], sig: &[u8]) -> Result<(), VerifyError> {
        match self.keys.get(did) {
            Some(k) if k.verify(data, sig) => Ok(()),
            Some(_) => Err(VerifyError("signature mismatch".into())),
            None => ResolverVerifier::new(&Offline).verify(did, key_id, data, sig),
        }
    }
}

fn verdict_text(v: &Verdict) -> String {
    match v {
        Verdict::Accepted => "accepted".into(),
        Verdict::Duplicate => "duplicate".into(),
        Verdict::Refused(r) => format!("refused: {r}"),
    }
}

/// `vt kolyada check DOC FILE [--as DID] [--key DID=PUBLIC]... [--now-ms N]`: FILE is a signed
/// offer, or a JSON array of signed messages starting with the genesis, replayed for the
/// participant `--as` (the initiator by default).
// @tg: implements:: [[openspec:protocol-conformance#Conformance Check]] {note: "vt kolyada check"}
pub fn check(
    doc: &Path,
    file: &Path,
    as_did: Option<&str>,
    keys: &[String],
    now_ms: u64,
) -> Outcome {
    done((|| {
        let p = Arc::new(protocol(doc)?);
        let verifier = OfflineVerifier::new(keys)?;
        let value = json_file(file)?;
        if value.is_object() {
            let offer: Offer = serde_json::from_value(value)
                .map_err(|e| (1, format!("{}: not a signed offer: {e}", file.display())))?;
            offer
                .verify(&verifier)
                .map_err(|e| (1, format!("offer by {}: {e}", offer.body.did)))?;
            if offer.body.protocol != p.id().as_str() {
                return Err((1, format!("the offer is for {}", offer.body.protocol)));
            }
            if !p.has_role(&offer.body.role) {
                return Err((
                    1,
                    format!("the protocol declares no role {}", offer.body.role),
                ));
            }
            if !offer.is_valid_at(now_ms) {
                return Err((1, format!("the offer by {} has expired", offer.body.did)));
            }
            return Ok(format!(
                "valid offer: {} plays {} of {}\n",
                offer.body.did,
                offer.body.role,
                p.id()
            ));
        }
        let messages: Vec<Message> = serde_json::from_value(value).map_err(|e| {
            (
                1,
                format!("{}: not a list of signed messages: {e}", file.display()),
            )
        })?;
        let Some(genesis) = messages.first() else {
            return Err((1, "the history is empty".into()));
        };
        if genesis.body.kind != control::GENESIS {
            return Err((1, "the history does not start with a genesis".into()));
        }
        let me = as_did.unwrap_or(&genesis.body.sender);
        let mut inst = Instance::from_genesis(p, genesis, me, &verifier, None)
            .map_err(|r| (1, format!("0 genesis: refused: {r}")))?;
        let mut out = format!("0 genesis from {}: accepted\n", genesis.body.role);
        let mut failed = false;
        for (i, m) in messages.iter().enumerate().skip(1) {
            let verdict = inst.check(m, &verifier);
            let events = inst.receive(m, &verifier, 0);
            let held = events.iter().any(|e| matches!(e, Event::Held(_)));
            let text = if held {
                "held for its dependencies".into()
            } else {
                verdict_text(&verdict)
            };
            failed |= matches!(verdict, Verdict::Refused(_)) && !held;
            out.push_str(&format!(
                "{i} {} from {}: {text}\n",
                m.body.kind, m.body.role
            ));
        }
        out.push_str(&format!(
            "as {} ({}): {:?}, in {}\n",
            inst.local_role(),
            me,
            inst.status(),
            inst.knowledge()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ));
        if failed {
            Err((1, out.trim_end().to_owned()))
        } else {
            Ok(out)
        }
    })())
}

fn public_from_seed(seed: &str) -> Result<PublicKey, String> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(seed)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("a seed is not 32 bytes of base64url")?;
    let km = InMemoryKeyManager::new();
    let name = km
        .import_seed(KeyType::Ed25519, bytes)
        .map_err(|e| e.to_string())?;
    km.public_key(&name).map_err(|e| e.to_string())
}

fn parse_doc(doc: &Value) -> Result<Protocol, String> {
    Protocol::parse(doc.to_string().as_bytes()).map_err(|e| e.to_string())
}

/// `vt kolyada vectors DIR`: checks `identity.json`, `projection.json`, `envelope.json` and
/// `verdicts.json` in DIR and prints one line per case.
// @tg: implements:: [[openspec:protocol-conformance#Test Vectors]] {note: "vt kolyada vectors"}
pub fn vectors(dir: &Path) -> Outcome {
    done((|| {
        let mut out = String::new();
        let mut failures = 0;
        let mut line = |ok: bool, what: String| {
            if !ok {
                failures += 1;
            }
            out.push_str(&format!("{} {what}\n", if ok { "ok  " } else { "FAIL" }));
        };
        let file = |name: &str| json_file(&dir.join(name));

        let identity = file("identity.json")?;
        let mut docs = BTreeMap::new();
        for c in identity["cases"].as_array().into_iter().flatten() {
            let name = c["name"].as_str().unwrap_or("?");
            match parse_doc(&c["document"]) {
                Ok(p) => {
                    line(p.id().as_str() == c["id"], format!("identity: {name}"));
                    docs.insert(p.id().as_str().to_owned(), p);
                }
                Err(e) => line(false, format!("identity: {name}: {e}")),
            }
        }

        let projection = file("projection.json")?;
        let pid = projection["protocol"].as_str().unwrap_or_default();
        match docs.get(pid) {
            Some(p) => {
                for v in projection["projections"].as_array().into_iter().flatten() {
                    let role = v["role"].as_str().unwrap_or_default();
                    let ok = p.project(role).is_some_and(|pr| projection_json(&pr) == *v);
                    line(ok, format!("projection: {role}"));
                }
            }
            None => line(false, format!("projection: no identity case for {pid}")),
        }

        let envelope = file("envelope.json")?;
        let ok = (|| -> Result<bool, String> {
            let m: Message =
                serde_json::from_value(envelope["message"].clone()).map_err(|e| e.to_string())?;
            let pk = public_from_seed(envelope["seed"].as_str().unwrap_or_default())?;
            let did = envelope["did"].as_str().unwrap_or_default();
            let v = OfflineVerifier {
                keys: BTreeMap::from([(did.to_owned(), pk)]),
            };
            Ok(URL_SAFE_NO_PAD.encode(pk.bytes) == envelope["publicKey"]
                && envelope["signingBytes"] == *String::from_utf8_lossy(&m.body.signing_bytes())
                && m.hash() == envelope["hash"]
                && m.verify(&v).is_ok())
        })();
        line(
            ok == Ok(true),
            format!(
                "envelope: {}",
                ok.err()
                    .unwrap_or_else(|| "signature, signing bytes and hash".into())
            ),
        );

        let verdicts = file("verdicts.json")?;
        let result = (|| -> Result<(), String> {
            let p = Arc::new(parse_doc(&verdicts["protocol"])?);
            let mut keys = BTreeMap::new();
            for (role, seed) in verdicts["seeds"].as_object().into_iter().flatten() {
                let did = verdicts["participants"][role].as_str().unwrap_or_default();
                keys.insert(
                    did.to_owned(),
                    public_from_seed(seed.as_str().unwrap_or_default())?,
                );
            }
            let v = OfflineVerifier { keys };
            let genesis: Message =
                serde_json::from_value(verdicts["genesis"].clone()).map_err(|e| e.to_string())?;
            let receiver = verdicts["receiver"].as_str().unwrap_or_default();
            let me = verdicts["participants"][receiver]
                .as_str()
                .unwrap_or_default();
            let mut inst =
                Instance::from_genesis(p, &genesis, me, &v, None).map_err(|r| r.to_string())?;
            for s in verdicts["setup"].as_array().into_iter().flatten() {
                let m: Message = serde_json::from_value(s.clone()).map_err(|e| e.to_string())?;
                if m.body.kind != control::GENESIS {
                    inst.receive(&m, &v, 0);
                }
            }
            for s in verdicts["steps"].as_array().into_iter().flatten() {
                let m: Message =
                    serde_json::from_value(s["message"].clone()).map_err(|e| e.to_string())?;
                let got = match inst.check(&m, &v) {
                    Verdict::Accepted => json!("accepted"),
                    Verdict::Duplicate => json!("duplicate"),
                    Verdict::Refused(r) => json!({ "refused": r.as_str() }),
                };
                line(
                    got == s["verdict"],
                    format!("verdict: {}", s["name"].as_str().unwrap_or("?")),
                );
            }
            Ok(())
        })();
        if let Err(e) = result {
            line(false, format!("verdicts: {e}"));
        }
        if failures == 0 {
            Ok(out)
        } else {
            Err((1, format!("{}{failures} vector(s) failed", out)))
        }
    })())
}
