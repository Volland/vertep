//! OpenID4VCI and OpenID4VP over HTTP (feature `oid4vc`, design D11): the bindings of the
//! role charts.
//!
//! - [`Oid4vcServer`]: an issuer's and verifier's endpoints (axum): credential issuer and
//!   authorisation server metadata, the token endpoint for the pre-authorised code (with
//!   transaction codes), the nonce endpoint, the credential endpoint, and the `direct_post`
//!   response endpoint; plus [`Oid4vcServer::offer`] and [`Oid4vcServer::request`], which
//!   start exchanges and return the credential offer and the authorisation request.
//! - [`Oid4vcHolder`]: a wallet's client (`ureq`): it redeems credential offers and answers
//!   authorisation requests through its consent step.
//!
//! Every request and response becomes an exchange message of the protocol codecs in
//! [`super::oid4vc`], handed to the engine; the engine's answers are captured by
//! [`CaptureWire`]. The engine is not `Send` (datamodels are not), so a server reaches it
//! through an [`EngineHandle`], a thread that owns it.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

use super::oid4vc::{
    DID_CLIENT_PREFIX, Oid4vci, Oid4vp, PRE_AUTHORIZED_GRANT, REDIRECT_CLIENT_PREFIX,
    authorization_request, client_did, credential_request, format_id, proof_of_request,
};
use crate::credential::{CredentialMeta, controller_of};
use crate::error::WalletError;
use crate::exchange::{Exchanges, Offer, Verified, Wire, agent_of};
use crate::jose::{self, Jws, b64};
use crate::message::{Body, ExchangeMessage};
use crate::plugin::ExchangeProtocol;
use crate::vocab::{PROTOCOL_OID4VCI, PROTOCOL_OID4VP, Role};

/// The peer of an issuer's OpenID4VCI exchange before the holder is known.
pub const ANY_HOLDER: &str = "urn:vertep:oid4vc:holder";

/// Everything but RFC 3986 unreserved characters.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, QUERY).to_string()
}

fn random() -> Result<String, WalletError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| WalletError::Protocol(e.to_string()))?;
    Ok(b64(&b))
}

// ---- engine thread and capture ------------------------------------------------------------

type Job = Box<dyn FnOnce(&mut Exchanges) + Send>;

/// A thread that owns an [`Exchanges`] and runs jobs on it.
#[derive(Clone, Debug)]
pub struct EngineHandle {
    tx: mpsc::Sender<Job>,
}

impl EngineHandle {
    /// Builds the engine with `make` on a new thread.
    pub fn spawn(make: impl FnOnce() -> Exchanges + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            let mut e = make();
            while let Ok(job) = rx.recv() {
                job(&mut e);
            }
        });
        EngineHandle { tx }
    }

    /// Runs `f` on the engine and returns its result.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] when the engine thread is gone.
    pub fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Exchanges) -> R + Send + 'static,
    ) -> Result<R, WalletError> {
        let (rtx, rrx) = mpsc::channel();
        self.tx
            .send(Box::new(move |e| {
                let _ = rtx.send(f(e));
            }))
            .map_err(|_| WalletError::Protocol("the engine thread is gone".into()))?;
        rrx.recv()
            .map_err(|_| WalletError::Protocol("the engine thread is gone".into()))
    }
}

/// A wire that keeps every message the engine sends, by exchange, for an HTTP binding to
/// answer with.
#[derive(Clone, Debug, Default)]
pub struct CaptureWire(Arc<Mutex<BTreeMap<String, Vec<ExchangeMessage>>>>);

impl CaptureWire {
    /// Takes the messages sent in exchange `id`.
    pub fn take(&self, id: &str) -> Vec<ExchangeMessage> {
        self.0
            .lock()
            .map(|mut m| m.remove(id).unwrap_or_default())
            .unwrap_or_default()
    }
}

impl Wire for CaptureWire {
    fn send(
        &self,
        protocol: &str,
        _from: &str,
        _to: &str,
        _msg: &str,
        bytes: &[u8],
    ) -> Result<(), WalletError> {
        let m = match protocol {
            PROTOCOL_OID4VCI => Oid4vci.decode(bytes)?,
            PROTOCOL_OID4VP => Oid4vp.decode(bytes)?,
            other => crate::protocol::native::VertepExchange
                .decode(bytes)
                .map_err(|_| WalletError::Protocol(format!("cannot capture {other}")))?,
        };
        self.0
            .lock()
            .map_err(|_| WalletError::Protocol("capture poisoned".into()))?
            .entry(m.exchange.clone())
            .or_default()
            .push(m);
        Ok(())
    }
}

fn message(protocol: &str, m: &ExchangeMessage) -> Result<Vec<u8>, WalletError> {
    match protocol {
        PROTOCOL_OID4VCI => Oid4vci.encode(m),
        _ => Oid4vp.encode(m),
    }
}

// ---- issuer and verifier endpoints --------------------------------------------------------

/// A credential configuration an issuer offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    /// The configuration id (also the SD-JWT VC `vct` unless given).
    pub id: String,
    /// The wallet format IRI.
    pub format: String,
    /// The credential type (`vct`).
    pub vct: String,
}

/// How a verifier identifies itself in authorisation requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientId {
    /// `decentralized_identifier:<DID>`, with a request object signed by the DID's key.
    Did,
    /// `redirect_uri:<response URI>`, with an unsigned request by value.
    RedirectUri,
}

/// What an [`Oid4vcServer`] serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    /// The server's base URL (the `credential_issuer`).
    pub base_url: String,
    /// The credential configurations offered.
    pub configurations: Vec<Configuration>,
    /// The verifier's client identifier prefix.
    pub client_id: ClientId,
}

#[derive(Default)]
struct Tables {
    codes: BTreeMap<String, (String, Option<String>)>,
    tokens: BTreeMap<String, String>,
    nonces: BTreeMap<String, String>,
    counter: u64,
}

struct ServerState {
    engine: EngineHandle,
    capture: CaptureWire,
    config: ServerConfig,
    did: String,
    tables: Mutex<Tables>,
}

impl ServerState {
    fn msg_id(&self) -> String {
        let mut t = self
            .tables
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        t.counter += 1;
        format!("http-{}", t.counter)
    }
}

/// An issuer's and verifier's OpenID4VC endpoints.
#[derive(Clone)]
pub struct Oid4vcServer {
    state: Arc<ServerState>,
}

impl std::fmt::Debug for Oid4vcServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Oid4vcServer")
            .field("base_url", &self.state.config.base_url)
            .finish()
    }
}

fn error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        [("Cache-Control", "no-store")],
        Json(json!({"error": error, "error_description": description})),
    )
        .into_response()
}

fn ok(v: Value) -> Response {
    (StatusCode::OK, [("Cache-Control", "no-store")], Json(v)).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| {
            h.strip_prefix("Bearer ")
                .or_else(|| h.strip_prefix("DPoP "))
        })
        .map(str::to_owned)
}

fn problem_status(reason: &str) -> &'static str {
    match reason {
        "nonce" => "invalid_nonce",
        "signature" | "malformed" | "holder" | "audience" => "invalid_proof",
        _ => "invalid_request",
    }
}

impl Oid4vcServer {
    /// A server for the wallet run by `engine`, which must send through `capture`. The wallet
    /// should list `config.base_url` among its aliases, the audience of possession proofs.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] when the engine thread is gone.
    pub fn new(
        engine: EngineHandle,
        capture: CaptureWire,
        config: ServerConfig,
    ) -> Result<Self, WalletError> {
        let did = engine.call(|e| e.wallet().did().to_owned())?;
        Ok(Oid4vcServer {
            state: Arc::new(ServerState {
                engine,
                capture,
                config,
                did,
                tables: Mutex::default(),
            }),
        })
    }

    /// The issuer metadata (`/.well-known/openid-credential-issuer`).
    // @tg: implements:: [[openspec:oid4vc#Credential Issuer Metadata]]
    pub fn metadata(&self) -> Value {
        let base = &self.state.config.base_url;
        let mut configs = serde_json::Map::new();
        for c in &self.state.config.configurations {
            let mut v = json!({
                "format": format_id(&c.format),
                "cryptographic_binding_methods_supported": ["jwk", "did:key"],
                "credential_signing_alg_values_supported": ["EdDSA"],
                "proof_types_supported": {"jwt": {"proof_signing_alg_values_supported": ["EdDSA"]}},
            });
            if c.format == crate::vocab::FORMAT_SD_JWT {
                v["vct"] = json!(c.vct);
            } else {
                v["credential_definition"] = json!({"type": ["VerifiableCredential", c.vct]});
            }
            configs.insert(c.id.clone(), v);
        }
        json!({
            "credential_issuer": base,
            "authorization_servers": [base],
            "credential_endpoint": format!("{base}/credential"),
            "nonce_endpoint": format!("{base}/nonce"),
            "credential_configurations_supported": configs,
        })
    }

    /// The axum router serving every endpoint.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/.well-known/openid-credential-issuer", get(metadata_h))
            .route(
                "/.well-known/oauth-authorization-server",
                get(as_metadata_h),
            )
            .route("/token", post(token_h))
            .route("/nonce", post(nonce_h))
            .route("/credential", post(credential_h))
            .route("/response", post(response_h))
            .with_state(self.state.clone())
    }

    /// As issuer: starts an OpenID4VCI exchange for `offer` and returns the credential offer
    /// URI (`openid-credential-offer://?credential_offer=…`), protected by `tx_code` if given.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the offer cannot be started.
    // @tg: implements:: [[openspec:oid4vc#OpenID4VCI Pre-Authorised Issuance]] {confidence: partial, note: "the issuer side; the holder is Oid4vcHolder"}
    pub fn offer(&self, offer: Offer, tx_code: Option<&str>) -> Result<String, WalletError> {
        let config_id = self
            .state
            .config
            .configurations
            .iter()
            .find(|c| c.format == offer.format && offer.types.first() == Some(&c.vct))
            .map(|c| c.id.clone())
            .ok_or_else(|| WalletError::Protocol("no configuration matches the offer".into()))?;
        let x = self
            .state
            .engine
            .call(move |e| e.offer(PROTOCOL_OID4VCI, ANY_HOLDER, offer))??;
        let nonce = self
            .state
            .capture
            .take(&x)
            .into_iter()
            .find_map(|m| match m.body {
                Body::Offer { nonce, .. } => Some(nonce),
                _ => None,
            })
            .ok_or_else(|| WalletError::Protocol("the issuer chart sent no offer".into()))?;
        let code = random()?;
        {
            let mut t = self
                .state
                .tables
                .lock()
                .map_err(|_| WalletError::Protocol("poisoned".into()))?;
            t.codes
                .insert(code.clone(), (x.clone(), tx_code.map(str::to_owned)));
            t.nonces.insert(x, nonce);
        }
        let mut grant = json!({"pre-authorized_code": code});
        if let Some(t) = tx_code {
            grant["tx_code"] = json!({"length": t.len(), "input_mode": "numeric"});
        }
        let offer = json!({
            "credential_issuer": self.state.config.base_url,
            "credential_configuration_ids": [config_id],
            "grants": {PRE_AUTHORIZED_GRANT: grant},
        });
        Ok(format!(
            "openid-credential-offer://?credential_offer={}",
            enc(&offer.to_string())
        ))
    }

    /// As verifier: starts an OpenID4VP exchange asking for `claims` of credentials of `types`,
    /// and returns the exchange id and the authorisation request URI (`openid4vp://?…`).
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the request cannot be started or signed.
    // @tg: implements:: [[openspec:oid4vc#OpenID4VP Verifier And Holder]] {confidence: partial, note: "the verifier side; the holder is Oid4vcHolder"}
    pub fn request(
        &self,
        claims: &[&str],
        types: &[&str],
        purpose: Option<&str>,
    ) -> Result<(String, String), WalletError> {
        let base = self.state.config.base_url.clone();
        let response_uri = format!("{base}/response");
        let client_id = match self.state.config.client_id {
            ClientId::Did => format!("{DID_CLIENT_PREFIX}{}", self.state.did),
            ClientId::RedirectUri => format!("{REDIRECT_CLIENT_PREFIX}{response_uri}"),
        };
        let claims: Vec<String> = claims.iter().map(|c| (*c).to_owned()).collect();
        let types: Vec<String> = types.iter().map(|t| (*t).to_owned()).collect();
        let purpose = purpose.map(str::to_owned);
        let cid = client_id.clone();
        let x = self.state.engine.call(move |e| {
            let c: Vec<&str> = claims.iter().map(String::as_str).collect();
            let t: Vec<&str> = types.iter().map(String::as_str).collect();
            e.request_for(
                PROTOCOL_OID4VP,
                ANY_HOLDER,
                &c,
                &t,
                purpose.as_deref(),
                Some(&cid),
            )
        })??;
        let req = self
            .state
            .capture
            .take(&x)
            .into_iter()
            .find_map(|m| match m.body {
                Body::PresentationRequest {
                    nonce,
                    claims,
                    types,
                    ..
                } => Some((nonce, claims, types)),
                _ => None,
            })
            .ok_or_else(|| WalletError::Protocol("the verifier chart sent no request".into()))?;
        let (nonce, claims, types) = req;
        let mut params =
            authorization_request(&client_id, &nonce, &types, &claims, &response_uri, &x);
        let uri = match self.state.config.client_id {
            ClientId::Did => {
                params["iss"] = json!(client_id);
                params["aud"] = json!("https://self-issued.me/v2");
                let jwt = self.state.engine.call(move |e| {
                    let w = e.wallet();
                    let s = w.signing()?;
                    jose::sign(
                        w.key_manager().as_ref(),
                        &s.key_name,
                        &json!({"typ": "oauth-authz-req+jwt", "kid": s.kid}),
                        &params,
                    )
                })??;
                format!(
                    "openid4vp://?client_id={}&request={}",
                    enc(&client_id),
                    enc(&jwt)
                )
            }
            ClientId::RedirectUri => {
                let mut q = String::from("openid4vp://?");
                for (k, v) in params.as_object().into_iter().flatten() {
                    let v = v
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string());
                    q.push_str(&format!("{}={}&", enc(k), enc(&v)));
                }
                q.trim_end_matches('&').to_owned()
            }
        };
        Ok((x, uri))
    }

    /// The verifier's outcome of exchange `id`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] when the engine thread is gone.
    pub fn verified(&self, id: &str) -> Result<Option<Verified>, WalletError> {
        let id = id.to_owned();
        self.state.engine.call(move |e| e.verified(&id).cloned())
    }

    /// The final or active states of exchange `id` in `role`.
    ///
    /// # Errors
    ///
    /// [`WalletError`] for unknown exchanges.
    pub fn state(&self, role: Role, id: &str) -> Result<Vec<String>, WalletError> {
        let id = id.to_owned();
        self.state.engine.call(move |e| e.state(role, &id))?
    }
}

type S = State<Arc<ServerState>>;

async fn metadata_h(State(s): S) -> Response {
    ok(Oid4vcServer { state: s }.metadata())
}

async fn as_metadata_h(State(s): S) -> Response {
    let base = &s.config.base_url;
    ok(json!({
        "issuer": base,
        "token_endpoint": format!("{base}/token"),
        "grant_types_supported": [PRE_AUTHORIZED_GRANT],
        "pre-authorized_grant_anonymous_access_supported": true,
    }))
}

async fn token_h(State(s): S, Form(f): Form<BTreeMap<String, String>>) -> Response {
    if f.get("grant_type").map(String::as_str) != Some(PRE_AUTHORIZED_GRANT) {
        return error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "only the pre-authorized_code grant",
        );
    }
    let Some(code) = f.get("pre-authorized_code") else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "no pre-authorized_code",
        );
    };
    let Ok(mut t) = s.tables.lock() else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "poisoned",
        );
    };
    let Some((x, tx)) = t.codes.get(code).cloned() else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "unknown or used pre-authorized_code",
        );
    };
    if tx.as_deref() != f.get("tx_code").map(String::as_str) && tx.is_some() {
        return error(StatusCode::BAD_REQUEST, "invalid_grant", "wrong tx_code");
    }
    t.codes.remove(code);
    let Ok(token) = random() else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "no randomness",
        );
    };
    t.tokens.insert(token.clone(), x.clone());
    let nonce = t.nonces.get(&x).cloned().unwrap_or_default();
    ok(json!({"access_token": token, "token_type": "Bearer", "expires_in": 300, "c_nonce": nonce}))
}

async fn nonce_h(State(s): S, headers: HeaderMap) -> Response {
    let Some(token) = bearer(&headers) else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "this issuer binds nonces to the access token",
        );
    };
    let t = s
        .tables
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match t.tokens.get(&token).and_then(|x| t.nonces.get(x)) {
        Some(n) => ok(json!({"c_nonce": n})),
        None => error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "unknown access token",
        ),
    }
}

async fn credential_h(State(s): S, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let Some(x) = bearer(&headers).and_then(|t| {
        s.tables
            .lock()
            .ok()
            .and_then(|tb| tb.tokens.get(&t).cloned())
    }) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "unknown access token",
        );
    };
    let proof = match proof_of_request(&body) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, "invalid_proof", &e.to_string()),
    };
    let m = ExchangeMessage {
        exchange: x.clone(),
        from: ANY_HOLDER.into(),
        to: s.did.clone(),
        body: Body::CredentialRequest { proof },
    };
    let Ok(bytes) = message(PROTOCOL_OID4VCI, &m) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "encoding",
        );
    };
    let id = s.msg_id();
    if let Err(e) = s
        .engine
        .call(move |e| e.receive(PROTOCOL_OID4VCI, &id, &bytes))
        .and_then(|r| r)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request", &e.to_string());
    }
    for m in s.capture.take(&x).into_iter().rev() {
        match m.body {
            Body::Credential { credential, .. } => {
                return ok(json!({"credentials": [{"credential": credential}]}));
            }
            Body::Problem { reason } => {
                return error(StatusCode::BAD_REQUEST, problem_status(&reason), &reason);
            }
            _ => {}
        }
    }
    error(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "no credential for this request",
    )
}

async fn response_h(State(s): S, Form(f): Form<BTreeMap<String, String>>) -> Response {
    let Some(x) = f.get("state").cloned() else {
        return error(StatusCode::BAD_REQUEST, "invalid_request", "no state");
    };
    let body = if let Some(e) = f.get("error") {
        Body::Problem {
            reason: f
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| e.clone()),
        }
    } else {
        let token: Value = match f.get("vp_token").map(|t| serde_json::from_str(t)) {
            Some(Ok(v)) => v,
            _ => return error(StatusCode::BAD_REQUEST, "invalid_request", "no vp_token"),
        };
        let presentation = token
            .as_object()
            .and_then(|o| o.values().next())
            .and_then(|v| v.as_array().and_then(|a| a.first()).or(Some(v)))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(presentation) = presentation else {
            return error(StatusCode::BAD_REQUEST, "invalid_request", "empty vp_token");
        };
        Body::Presentation {
            format: crate::vocab::FORMAT_SD_JWT.into(),
            presentation,
        }
    };
    let is_problem = matches!(body, Body::Problem { .. });
    let m = ExchangeMessage {
        exchange: x.clone(),
        from: ANY_HOLDER.into(),
        to: s.did.clone(),
        body,
    };
    let Ok(bytes) = message(PROTOCOL_OID4VP, &m) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "encoding",
        );
    };
    let id = s.msg_id();
    if let Err(e) = s
        .engine
        .call(move |e| e.receive(PROTOCOL_OID4VP, &id, &bytes))
        .and_then(|r| r)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request", &e.to_string());
    }
    if is_problem {
        return ok(json!({}));
    }
    for m in s.capture.take(&x).into_iter().rev() {
        match m.body {
            Body::Result { accepted: true, .. } => return ok(json!({})),
            Body::Result { reason, .. } => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    reason.as_deref().unwrap_or("refused"),
                );
            }
            Body::Problem { reason } => {
                return error(StatusCode::BAD_REQUEST, "invalid_request", &reason);
            }
            _ => {}
        }
    }
    error(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "the presentation was not judged",
    )
}

// ---- holder -------------------------------------------------------------------------------

/// What answering an authorisation request did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answered {
    /// A presentation was posted; the verifier accepted it or not.
    Presented {
        /// Whether the response endpoint accepted it.
        accepted: bool,
    },
    /// The holder refused (`access_denied`), with why.
    Refused(String),
}

/// A wallet's OpenID4VC client.
#[derive(Clone, Debug, Default)]
pub struct Oid4vcHolder {
    capture: CaptureWire,
}

fn query_param(uri: &str, name: &str) -> Option<String> {
    let q = uri.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (percent_encoding::percent_decode_str(k).decode_utf8_lossy() == name).then(|| {
            percent_encoding::percent_decode_str(v)
                .decode_utf8_lossy()
                .into_owned()
        })
    })
}

fn get_json(url: &str) -> Result<Value, WalletError> {
    ureq::get(url)
        .call()
        .map_err(|e| WalletError::Protocol(format!("GET {url}: {e}")))?
        .into_json()
        .map_err(|e| WalletError::Protocol(format!("GET {url}: {e}")))
}

fn http_error(e: ureq::Error) -> WalletError {
    match e {
        ureq::Error::Status(code, r) => {
            let body: Value = r.into_json().unwrap_or_default();
            let err = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_owned();
            WalletError::Protocol(format!(
                "{code} {err}: {}",
                body.get("error_description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ))
        }
        other => WalletError::Protocol(other.to_string()),
    }
}

impl Oid4vcHolder {
    /// A client whose engine sends through `capture`.
    pub fn new(capture: CaptureWire) -> Self {
        Oid4vcHolder { capture }
    }

    /// Redeems the credential offer `offer_uri` (with `tx_code` if the offer asks for one):
    /// token, nonce, possession proof through the holder chart, credential, storage.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] naming the endpoint and OAuth error that failed.
    pub fn accept_offer(
        &self,
        e: &mut Exchanges,
        offer_uri: &str,
        tx_code: Option<&str>,
    ) -> Result<CredentialMeta, WalletError> {
        let offer: Value = match query_param(offer_uri, "credential_offer") {
            Some(o) => serde_json::from_str(&o)
                .map_err(|e| WalletError::Malformed(format!("credential_offer: {e}")))?,
            None => get_json(
                &query_param(offer_uri, "credential_offer_uri")
                    .ok_or_else(|| WalletError::Malformed("not a credential offer".into()))?,
            )?,
        };
        let issuer = offer
            .get("credential_issuer")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no credential_issuer".into()))?
            .to_owned();
        let config_id = offer
            .pointer("/credential_configuration_ids/0")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no credential_configuration_ids".into()))?
            .to_owned();
        let code = offer
            .get("grants")
            .and_then(|g| g.get(PRE_AUTHORIZED_GRANT))
            .and_then(|g| g.get("pre-authorized_code"))
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("not a pre-authorized_code offer".into()))?
            .to_owned();
        let meta = get_json(&format!("{issuer}/.well-known/openid-credential-issuer"))?;
        let config = meta
            .pointer(&format!("/credential_configurations_supported/{config_id}"))
            .cloned()
            .unwrap_or_default();
        let as_meta = get_json(&format!("{issuer}/.well-known/oauth-authorization-server"))
            .unwrap_or_default();
        let token_endpoint = as_meta
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{issuer}/token"));
        let mut form = vec![
            ("grant_type", PRE_AUTHORIZED_GRANT),
            ("pre-authorized_code", code.as_str()),
        ];
        if let Some(t) = tx_code {
            form.push(("tx_code", t));
        }
        let token: Value = ureq::post(&token_endpoint)
            .send_form(&form)
            .map_err(http_error)?
            .into_json()
            .map_err(|e| WalletError::Protocol(e.to_string()))?;
        let access = token
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Protocol("no access_token".into()))?
            .to_owned();
        let nonce = match token.get("c_nonce").and_then(Value::as_str) {
            Some(n) => n.to_owned(),
            None => {
                let ne = meta
                    .get("nonce_endpoint")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Protocol("no nonce".into()))?;
                let r: Value = ureq::post(ne)
                    .set("Authorization", &format!("Bearer {access}"))
                    .call()
                    .map_err(http_error)?
                    .into_json()
                    .map_err(|e| WalletError::Protocol(e.to_string()))?;
                r.get("c_nonce")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Protocol("no c_nonce".into()))?
                    .to_owned()
            }
        };
        let x = format!("oid4vci:{code}");
        let format = super::oid4vc::format_iri(
            config
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("dc+sd-jwt"),
        )
        .to_owned();
        let me = e.wallet().did().to_owned();
        let offer_msg = ExchangeMessage {
            exchange: x.clone(),
            from: issuer.clone(),
            to: me.clone(),
            body: Body::Offer {
                issuer: issuer.clone(),
                nonce,
                types: vec![config_id.clone()],
                format: format.clone(),
            },
        };
        e.receive(
            PROTOCOL_OID4VCI,
            &format!("{x}-offer"),
            &Oid4vci.encode(&offer_msg)?,
        )?;
        let proof = self
            .capture
            .take(&x)
            .into_iter()
            .find_map(|m| match m.body {
                Body::CredentialRequest { proof } => Some(proof),
                _ => None,
            })
            .ok_or_else(|| {
                WalletError::Protocol("the holder chart sent no possession proof".into())
            })?;
        let endpoint = meta
            .get("credential_endpoint")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{issuer}/credential"));
        let resp: Value = ureq::post(&endpoint)
            .set("Authorization", &format!("Bearer {access}"))
            .send_json(credential_request(&config_id, &proof))
            .map_err(http_error)?
            .into_json()
            .map_err(|e| WalletError::Protocol(e.to_string()))?;
        let credential = resp
            .pointer("/credentials/0/credential")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Protocol("no credentials[0].credential".into()))?
            .to_owned();
        let cred_msg = ExchangeMessage {
            exchange: x.clone(),
            from: issuer,
            to: me,
            body: Body::Credential { format, credential },
        };
        e.receive(
            PROTOCOL_OID4VCI,
            &format!("{x}-credential"),
            &Oid4vci.encode(&cred_msg)?,
        )?;
        match e.state(Role::Holder, &x)?.first().map(String::as_str) {
            Some("stored") => {}
            other => {
                return Err(WalletError::Protocol(format!(
                    "the credential was not stored ({other:?})"
                )));
            }
        }
        let actor = e
            .exchange(Role::Holder, &x)
            .map(|i| i.actor.clone())
            .unwrap_or_default();
        let digest = e
            .host()
            .state(&actor)
            .ok()
            .map(|s| {
                vertep_agent::json::to_json(
                    vertep_agent::decider::ContextView::new(s.datamodel).data(),
                )
            })
            .and_then(|d| {
                d.get("credential")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        e.wallet()
            .list()?
            .into_iter()
            .find(|m| m.digest == digest)
            .ok_or(WalletError::UnknownCredential(digest.clone()))
    }

    /// Answers the authorisation request `request_uri` (`openid4vp://?…`, by value or with a
    /// signed `request` object) through the wallet's consent step, and posts the presentation
    /// or `access_denied` to the response URI.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the request is malformed, its request object does not verify, or
    /// the response cannot be posted.
    pub fn respond(&self, e: &mut Exchanges, request_uri: &str) -> Result<Answered, WalletError> {
        let client_id = query_param(request_uri, "client_id")
            .ok_or_else(|| WalletError::Malformed("no client_id".into()))?;
        let params: Value = if let Some(jwt) = query_param(request_uri, "request") {
            let jws = Jws::parse(&jwt)?;
            let kid = jws
                .header
                .get("kid")
                .and_then(Value::as_str)
                .ok_or_else(|| WalletError::Malformed("request object without kid".into()))?;
            let did = client_did(&client_id).ok_or_else(|| {
                WalletError::Malformed(
                    "a signed request needs a decentralized_identifier client id".into(),
                )
            })?;
            if controller_of(kid) != did {
                return Err(WalletError::BadSignature(format!(
                    "{kid} is not a key of {did}"
                )));
            }
            jws.verify(&e.wallet().keys().key(kid)?)?;
            if jws.payload.get("client_id").and_then(Value::as_str) != Some(client_id.as_str()) {
                return Err(WalletError::Malformed(
                    "request object for another client".into(),
                ));
            }
            jws.payload
        } else {
            let mut p = serde_json::Map::new();
            for k in [
                "response_type",
                "response_mode",
                "client_id",
                "response_uri",
                "nonce",
                "state",
                "purpose",
            ] {
                if let Some(v) = query_param(request_uri, k) {
                    p.insert(k.into(), json!(v));
                }
            }
            let q = query_param(request_uri, "dcql_query")
                .ok_or_else(|| WalletError::Malformed("no dcql_query".into()))?;
            p.insert(
                "dcql_query".into(),
                serde_json::from_str(&q)
                    .map_err(|e| WalletError::Malformed(format!("dcql_query: {e}")))?,
            );
            Value::Object(p)
        };
        let state = params
            .get("state")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no state".into()))?
            .to_owned();
        let response_uri = params
            .get("response_uri")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no response_uri".into()))?
            .to_owned();
        let me = e.wallet().did().to_owned();
        let wrapped = serde_json::to_vec(&json!({
            "kind": "authorization-request", "exchange": state, "from": agent_of(&client_id), "to": me, "message": params,
        }))
        .map_err(|e| WalletError::Protocol(e.to_string()))?;
        e.receive(PROTOCOL_OID4VP, &format!("{state}-request"), &wrapped)?;
        for m in self.capture.take(&state).into_iter().rev() {
            match m.body {
                Body::Presentation { presentation, .. } => {
                    let token = json!({"cred1": [presentation]}).to_string();
                    let r = ureq::post(&response_uri)
                        .send_form(&[("vp_token", token.as_str()), ("state", state.as_str())]);
                    return Ok(Answered::Presented {
                        accepted: r.is_ok(),
                    });
                }
                Body::Problem { reason } => {
                    let _ = ureq::post(&response_uri).send_form(&[
                        ("error", "access_denied"),
                        ("error_description", reason.as_str()),
                        ("state", state.as_str()),
                    ]);
                    return Ok(Answered::Refused(reason));
                }
                _ => {}
            }
        }
        Err(WalletError::Protocol(
            "the holder chart answered nothing".into(),
        ))
    }
}
