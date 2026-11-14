//! OpenID4VCI 1.0 (pre-authorised code flow) and OpenID4VP 1.0 (DCQL, `direct_post`) as
//! exchange-protocol plugins (`oid4vc` requirements, design D11).
//!
//! Each protocol is a codec between exchange messages and the OpenID4VC JSON messages that map
//! onto the exchange vocabulary:
//!
//! | Vocabulary step | OpenID4VCI | OpenID4VP |
//! | --- | --- | --- |
//! | `offer` | the Nonce Endpoint response (`c_nonce`) | |
//! | `prove-possession` | the Credential Request (`proofs.jwt`) | |
//! | `issue` | the Credential Response (`credentials`) | |
//! | `request-presentation` | | the Authorization Request (`dcql_query`, `nonce`, `client_id`) |
//! | `present` | | the Authorization Response (`vp_token`) |
//! | none (`other`) | error responses | error responses (`access_denied`), the response endpoint's answer |
//!
//! The credential offer and the token exchange are bindings ([`super::oid4vc_http`]), not
//! vocabulary steps: they carry no nonce, key or credential. On the wire of the conformance
//! kit, each message is wrapped with its exchange id, sender and recipient; over HTTP the
//! binding correlates by pre-authorised code, access token and `state`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::WalletError;
use crate::history::Claims;
use crate::message::{Body, ExchangeMessage};
use crate::plugin::ExchangeProtocol;
use crate::vocab::{FORMAT_SD_JWT, PROTOCOL_OID4VCI, PROTOCOL_OID4VP};

/// The `pre-authorized_code` grant type.
pub const PRE_AUTHORIZED_GRANT: &str = "urn:ietf:params:oauth:grant-type:pre-authorized_code";
/// The client identifier prefix for verifiers identified by a DID.
pub const DID_CLIENT_PREFIX: &str = "decentralized_identifier:";
/// The client identifier prefix for verifiers identified by their response URI.
pub const REDIRECT_CLIENT_PREFIX: &str = "redirect_uri:";

/// The OpenID4VC format identifier of a wallet format IRI.
pub fn format_id(iri: &str) -> &str {
    match iri {
        FORMAT_SD_JWT => "dc+sd-jwt",
        crate::vocab::FORMAT_DI_JCS | crate::vocab::FORMAT_DI_RDFC => "ldp_vc",
        other => other,
    }
}

/// The wallet format IRI of an OpenID4VC format identifier.
pub fn format_iri(id: &str) -> &str {
    match id {
        "dc+sd-jwt" => FORMAT_SD_JWT,
        "ldp_vc" => crate::vocab::FORMAT_DI_JCS,
        other => other,
    }
}

/// The DID behind an OpenID4VP client identifier, if it names one.
pub fn client_did(client_id: &str) -> Option<&str> {
    client_id.strip_prefix(DID_CLIENT_PREFIX)
}

fn wrap(m: &ExchangeMessage, kind: &str, message: Value) -> Result<Vec<u8>, WalletError> {
    serde_json::to_vec(&json!({
        "kind": kind, "exchange": m.exchange, "from": m.from, "to": m.to, "message": message,
    }))
    .map_err(|e| WalletError::Protocol(e.to_string()))
}

fn unwrap(bytes: &[u8]) -> Result<(String, ExchangeMessage, Value), WalletError> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| WalletError::Malformed(format!("OpenID4VC message: {e}")))?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| WalletError::Unmapped(format!("OpenID4VC message without {k}")))
    };
    let kind = s("kind")?;
    let m = ExchangeMessage {
        exchange: s("exchange")?,
        from: s("from")?,
        to: s("to")?,
        body: Body::Problem { reason: String::new() },
    };
    Ok((kind, m, v.get("message").cloned().unwrap_or(Value::Null)))
}

fn error_message(reason: &str) -> Value {
    let error = match reason {
        "nonce" => "invalid_nonce",
        "signature" | "malformed" | "holder" | "audience" => "invalid_proof",
        "consent" | "no-match" | "refused" | "out-of-bounds" => "access_denied",
        "" => "invalid_request",
        _ => "invalid_request",
    };
    json!({"error": error, "error_description": reason})
}

fn reason_of(message: &Value) -> String {
    message
        .get("error_description")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| message.get("error").and_then(Value::as_str))
        .unwrap_or("error")
        .to_owned()
}

/// OpenID4VCI 1.0, pre-authorised code flow.
// @tg: implements:: [[openspec:oid4vc#OpenID4VCI Pre-Authorised Issuance]] {confidence: partial, note: "the message mapping; the endpoints are in oid4vc_http"}
#[derive(Clone, Copy, Debug, Default)]
pub struct Oid4vci;

/// The Credential Request of a possession proof, for configuration `configuration`.
pub fn credential_request(configuration: &str, proof: &str) -> Value {
    json!({"credential_configuration_id": configuration, "proofs": {"jwt": [proof]}})
}

/// The possession proof of a Credential Request.
///
/// # Errors
///
/// [`WalletError::Malformed`] when it has no JWT proof.
pub fn proof_of_request(request: &Value) -> Result<String, WalletError> {
    request
        .pointer("/proofs/jwt/0")
        .or_else(|| request.pointer("/proof/jwt"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| WalletError::Malformed("credential request without a jwt proof".into()))
}

impl ExchangeProtocol for Oid4vci {
    fn iri(&self) -> &str {
        PROTOCOL_OID4VCI
    }

    fn encode(&self, m: &ExchangeMessage) -> Result<Vec<u8>, WalletError> {
        match &m.body {
            Body::Offer {
                issuer,
                nonce,
                types,
                format,
            } => wrap(
                m,
                "nonce-response",
                json!({"c_nonce": nonce, "credential_issuer": issuer,
                       "credential_configuration_ids": types, "format": format_id(format)}),
            ),
            Body::CredentialRequest { proof } => {
                wrap(m, "credential-request", credential_request("", proof))
            }
            Body::Credential { format, credential } => wrap(
                m,
                "credential-response",
                json!({"credentials": [{"credential": credential}], "format": format_id(format)}),
            ),
            Body::Problem { reason } => wrap(m, "error", error_message(reason)),
            Body::Result { accepted, reason } => wrap(
                m,
                "error",
                if *accepted { json!({}) } else { error_message(reason.as_deref().unwrap_or("")) },
            ),
            other => Err(WalletError::Protocol(format!(
                "OpenID4VCI does not carry {:?}",
                other.step()
            ))),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<ExchangeMessage, WalletError> {
        let (kind, mut m, msg) = unwrap(bytes)?;
        m.body = match kind.as_str() {
            "nonce-response" => Body::Offer {
                issuer: msg.get("credential_issuer").and_then(Value::as_str).unwrap_or(&m.from).to_owned(),
                nonce: msg
                    .get("c_nonce")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Malformed("no c_nonce".into()))?
                    .to_owned(),
                types: msg
                    .get("credential_configuration_ids")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default(),
                format: format_iri(msg.get("format").and_then(Value::as_str).unwrap_or("dc+sd-jwt")).to_owned(),
            },
            "credential-request" => Body::CredentialRequest {
                proof: proof_of_request(&msg)?,
            },
            "credential-response" => Body::Credential {
                format: format_iri(msg.get("format").and_then(Value::as_str).unwrap_or("dc+sd-jwt")).to_owned(),
                credential: msg
                    .pointer("/credentials/0/credential")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Malformed("no credentials[0].credential".into()))?
                    .to_owned(),
            },
            "error" => Body::Problem { reason: reason_of(&msg) },
            other => return Err(WalletError::Unmapped(format!("OpenID4VCI {other}"))),
        };
        Ok(m)
    }
}

/// OpenID4VP 1.0 with DCQL and `direct_post`.
// @tg: implements:: [[openspec:oid4vc#OpenID4VP Verifier And Holder]] {confidence: partial, note: "the message mapping and DCQL; the endpoints are in oid4vc_http"}
#[derive(Clone, Copy, Debug, Default)]
pub struct Oid4vp;

/// The DCQL query for one credential of `types` (SD-JWT VC `vct` values) with `claims`.
pub fn dcql_query(types: &[String], claims: &Claims) -> Value {
    let mut cred = Map::new();
    cred.insert("id".into(), json!("cred1"));
    cred.insert("format".into(), json!("dc+sd-jwt"));
    if !types.is_empty() {
        cred.insert("meta".into(), json!({"vct_values": types}));
    }
    cred.insert(
        "claims".into(),
        Value::Array(claims.iter().map(|c| json!({"path": [c]})).collect()),
    );
    json!({"credentials": [Value::Object(cred)]})
}

/// What a DCQL query asks for: per credential query, its id, acceptable types and claim names.
///
/// # Errors
///
/// [`WalletError::Malformed`] for queries without credentials or claim paths.
pub fn read_dcql(query: &Value) -> Result<Vec<(String, Vec<String>, Claims)>, WalletError> {
    let creds = query
        .get("credentials")
        .and_then(Value::as_array)
        .ok_or_else(|| WalletError::Malformed("dcql_query without credentials".into()))?;
    let mut out = Vec::new();
    for c in creds {
        let id = c.get("id").and_then(Value::as_str).unwrap_or("cred").to_owned();
        let types = c
            .pointer("/meta/vct_values")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let mut claims = Claims::new();
        for cl in c.get("claims").and_then(Value::as_array).cloned().unwrap_or_default() {
            // a claim path is a list of names, array indices or null (any element); nested
            // paths name a dotted claim, which flat credentials do not hold
            let path = cl
                .get("path")
                .and_then(Value::as_array)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| WalletError::Malformed("a claims query without a path".into()))?;
            let name: Vec<String> = path
                .iter()
                .map(|seg| match seg {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => "*".to_owned(),
                })
                .collect();
            claims.insert(name.join("."));
        }
        out.push((id, types, claims));
    }
    Ok(out)
}

/// The authorization request (by value) of a presentation request.
pub fn authorization_request(
    client_id: &str,
    nonce: &str,
    types: &[String],
    claims: &Claims,
    response_uri: &str,
    state: &str,
) -> Value {
    json!({
        "response_type": "vp_token",
        "response_mode": "direct_post",
        "client_id": client_id,
        "response_uri": response_uri,
        "nonce": nonce,
        "state": state,
        "dcql_query": dcql_query(types, claims),
    })
}

impl ExchangeProtocol for Oid4vp {
    fn iri(&self) -> &str {
        PROTOCOL_OID4VP
    }

    fn encode(&self, m: &ExchangeMessage) -> Result<Vec<u8>, WalletError> {
        match &m.body {
            Body::PresentationRequest {
                verifier,
                nonce,
                claims,
                types,
                purpose,
            } => {
                let mut req = authorization_request(verifier, nonce, types, claims, "", &m.exchange);
                if let Some(p) = purpose {
                    req["purpose"] = json!(p);
                }
                wrap(m, "authorization-request", req)
            }
            Body::Presentation { format, presentation } => wrap(
                m,
                "authorization-response",
                json!({"vp_token": {"cred1": [presentation]}, "state": m.exchange, "format": format_id(format)}),
            ),
            Body::Result { accepted, reason } => wrap(
                m,
                "response-result",
                if *accepted { json!({}) } else { error_message(reason.as_deref().unwrap_or("")) },
            ),
            Body::Problem { reason } => wrap(m, "error", error_message(reason)),
            other => Err(WalletError::Protocol(format!(
                "OpenID4VP does not carry {:?}",
                other.step()
            ))),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<ExchangeMessage, WalletError> {
        let (kind, mut m, msg) = unwrap(bytes)?;
        m.body = match kind.as_str() {
            "authorization-request" => {
                let queries = read_dcql(msg.get("dcql_query").unwrap_or(&Value::Null))?;
                let (_, types, claims) = queries
                    .into_iter()
                    .next()
                    .ok_or_else(|| WalletError::Malformed("empty dcql_query".into()))?;
                Body::PresentationRequest {
                    verifier: msg
                        .get("client_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| WalletError::Malformed("no client_id".into()))?
                        .to_owned(),
                    nonce: msg
                        .get("nonce")
                        .and_then(Value::as_str)
                        .ok_or_else(|| WalletError::Malformed("no nonce".into()))?
                        .to_owned(),
                    claims,
                    types,
                    purpose: msg.get("purpose").and_then(Value::as_str).map(str::to_owned),
                }
            }
            "authorization-response" => Body::Presentation {
                format: format_iri(msg.get("format").and_then(Value::as_str).unwrap_or("dc+sd-jwt")).to_owned(),
                presentation: msg
                    .get("vp_token")
                    .and_then(Value::as_object)
                    .and_then(|o| o.values().next())
                    .and_then(|v| v.as_array().and_then(|a| a.first()).or(Some(v)))
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Malformed("no vp_token".into()))?
                    .to_owned(),
            },
            "response-result" if msg.get("error").is_none() => Body::Result {
                accepted: true,
                reason: None,
            },
            "response-result" | "error" => Body::Problem { reason: reason_of(&msg) },
            other => return Err(WalletError::Unmapped(format!("OpenID4VP {other}"))),
        };
        Ok(m)
    }

    fn audience(&self, verifier: &str) -> String {
        format!("{DID_CLIENT_PREFIX}{verifier}")
    }
}

/// Typed OpenID4VC structures: the members this crate reads, with every other member kept, so
/// a document parses only with its required members and re-serialises with the same members
/// (`oid4vc` requirement `OpenID4VC Interoperability`).
// @tg: implements:: [[openspec:oid4vc#OpenID4VC Interoperability]]
pub mod model {
    use super::*;

    /// A credential offer.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct CredentialOffer {
        /// The issuer URL.
        pub credential_issuer: String,
        /// Offered configurations.
        pub credential_configuration_ids: Vec<String>,
        /// Grants (`pre-authorized_code`, `authorization_code`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub grants: Option<Map<String, Value>>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// Credential issuer metadata (or a fragment of it).
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct IssuerMetadata {
        /// The issuer URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub credential_issuer: Option<String>,
        /// The credential endpoint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub credential_endpoint: Option<String>,
        /// The nonce endpoint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub nonce_endpoint: Option<String>,
        /// Configurations by id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub credential_configurations_supported: Option<Map<String, Value>>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A token response.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct TokenResponse {
        /// The access token.
        pub access_token: String,
        /// Its type.
        pub token_type: String,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A nonce endpoint response.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct NonceResponse {
        /// The nonce.
        pub c_nonce: String,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A credential response.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct CredentialResponse {
        /// The issued credentials.
        pub credentials: Vec<Value>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// An OAuth error response.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct ErrorResponse {
        /// The error code.
        pub error: String,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A claims query.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct ClaimsQuery {
        /// The claim path.
        pub path: Vec<Value>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A credential query.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct CredentialQuery {
        /// The query id.
        pub id: String,
        /// The format identifier.
        pub format: String,
        /// Format-specific constraints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub meta: Option<Value>,
        /// Requested claims.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub claims: Option<Vec<ClaimsQuery>>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// A DCQL query.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct DcqlQuery {
        /// Credential queries.
        pub credentials: Vec<CredentialQuery>,
        /// Other members (`credential_sets`).
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// An authorisation request (by value, or the payload of a request object).
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct AuthorizationRequest {
        /// `vp_token`.
        pub response_type: String,
        /// The client identifier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub client_id: Option<String>,
        /// The nonce.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub nonce: Option<String>,
        /// The query.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub dcql_query: Option<DcqlQuery>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// An authorisation response.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct AuthorizationResponse {
        /// Presentations by credential query id.
        pub vp_token: Map<String, Value>,
        /// Other members.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }
}
