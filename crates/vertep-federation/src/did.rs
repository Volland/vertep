//! DIDs and DID documents: parsing a DID, reading a DID document (DID Core 1.0) into a normalised
//! model, and writing it back as JSON.
//!
//! The model keeps what federation needs: verification methods with their public keys, the
//! verification relationships (`authentication`, `assertionMethod`, `keyAgreement`) as method ids,
//! and service entries with their endpoints and routing keys. Embedded verification methods are
//! moved into `verificationMethod` and referenced by id. Private key members (`d`,
//! `privateKeyMultibase`, `secretKeyMultibase`, `privateKeyBase58`, `privateKeyJwk`) are refused.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::keys::{KeyType, PublicKey};

/// A parsed DID: `did:<method>:<method-specific-id>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Did {
    did: String,
    method_end: usize,
}

/// A malformed DID or DID URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DidSyntaxError(pub String);

impl fmt::Display for DidSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid DID: {}", self.0)
    }
}

impl std::error::Error for DidSyntaxError {}

impl Did {
    /// Parses a DID (no path, query or fragment).
    ///
    /// # Errors
    ///
    /// [`DidSyntaxError`] when `s` does not follow the DID syntax.
    pub fn parse(s: &str) -> Result<Self, DidSyntaxError> {
        let rest = s
            .strip_prefix("did:")
            .ok_or_else(|| DidSyntaxError(s.to_owned()))?;
        let (method, msid) = rest
            .split_once(':')
            .ok_or_else(|| DidSyntaxError(s.to_owned()))?;
        let method_ok = !method.is_empty()
            && method
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        let msid_ok = !msid.is_empty()
            && !msid.ends_with(':')
            && msid
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_:%".contains(&c));
        if !method_ok || !msid_ok {
            return Err(DidSyntaxError(s.to_owned()));
        }
        Ok(Did {
            did: s.to_owned(),
            method_end: 4 + method.len(),
        })
    }

    /// Parses the DID part of a DID URL, dropping any path, query or fragment.
    ///
    /// # Errors
    ///
    /// [`DidSyntaxError`] when the DID part is malformed.
    pub fn from_did_url(url: &str) -> Result<Self, DidSyntaxError> {
        let end = url.find(['/', '?', '#']).unwrap_or(url.len());
        Did::parse(&url[..end])
    }

    /// The method name (`key`, `peer`, `web`, `webvh`, ...).
    pub fn method(&self) -> &str {
        &self.did[4..self.method_end]
    }

    /// The method-specific identifier.
    pub fn method_specific_id(&self) -> &str {
        &self.did[self.method_end + 1..]
    }

    /// The whole DID.
    pub fn as_str(&self) -> &str {
        &self.did
    }
}

impl fmt::Display for Did {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.did)
    }
}

/// True when `address` is a DID, so it is resolved by the DID directory and not by the cluster
/// directory. Actors on `vt:` IRIs never reach DID resolution, keys or cryptography.
// @tg: implements:: [[openspec:did-identity#Opt-In DID Identity Per Actor]] {note: "the address-scheme split"}
pub fn is_did_address(address: &str) -> bool {
    address.starts_with("did:")
}

/// A verification method with its public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationMethod {
    /// The method id, a DID URL (usually `did#fragment`).
    pub id: String,
    /// The method type (`Multikey`, `Ed25519VerificationKey2020`, `JsonWebKey2020`, ...).
    pub type_: String,
    /// The controller DID.
    pub controller: String,
    /// The public key.
    pub public_key: PublicKey,
}

impl VerificationMethod {
    /// A `Multikey` verification method.
    pub fn multikey(id: impl Into<String>, controller: impl Into<String>, key: PublicKey) -> Self {
        VerificationMethod {
            id: id.into(),
            type_: "Multikey".into(),
            controller: controller.into(),
            public_key: key,
        }
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("type".into(), json!(self.type_));
        m.insert("controller".into(), json!(self.controller));
        if self.type_ == "JsonWebKey2020" || self.type_ == "JsonWebKey" {
            m.insert("publicKeyJwk".into(), self.public_key.to_jwk());
        } else {
            m.insert(
                "publicKeyMultibase".into(),
                json!(self.public_key.multikey()),
            );
        }
        Value::Object(m)
    }
}

/// One endpoint of a service entry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceEndpoint {
    /// The endpoint URI (an `https:`, `wss:` or `did:` URI).
    pub uri: String,
    /// DIDComm routing keys (DID URLs of mediator keys), outermost last as listed.
    pub routing_keys: Vec<String>,
    /// Accepted media types or profiles.
    pub accept: Vec<String>,
}

impl ServiceEndpoint {
    /// An endpoint with only a URI.
    pub fn uri(uri: impl Into<String>) -> Self {
        ServiceEndpoint {
            uri: uri.into(),
            routing_keys: Vec::new(),
            accept: Vec::new(),
        }
    }
}

/// A service entry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Service {
    /// The service id (a DID URL).
    pub id: String,
    /// The service types.
    pub types: Vec<String>,
    /// The endpoints, in document order.
    pub endpoints: Vec<ServiceEndpoint>,
}

impl Service {
    /// True when the service has type `t`.
    pub fn has_type(&self, t: &str) -> bool {
        self.types.iter().any(|x| x == t)
    }

    fn to_json(&self) -> Value {
        let ty = if self.types.len() == 1 {
            json!(self.types[0])
        } else {
            json!(self.types)
        };
        let eps: Vec<Value> = self
            .endpoints
            .iter()
            .map(|e| {
                if e.routing_keys.is_empty() && e.accept.is_empty() {
                    json!(e.uri)
                } else {
                    let mut m = Map::new();
                    m.insert("uri".into(), json!(e.uri));
                    if !e.accept.is_empty() {
                        m.insert("accept".into(), json!(e.accept));
                    }
                    if !e.routing_keys.is_empty() {
                        m.insert("routingKeys".into(), json!(e.routing_keys));
                    }
                    Value::Object(m)
                }
            })
            .collect();
        let ep = if eps.len() == 1 {
            eps.into_iter().next().unwrap_or(Value::Null)
        } else {
            Value::Array(eps)
        };
        json!({"id": self.id, "type": ty, "serviceEndpoint": ep})
    }
}

/// A DID document, normalised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DidDocument {
    /// The DID subject.
    pub id: String,
    /// `alsoKnownAs`.
    pub also_known_as: Vec<String>,
    /// Every verification method, embedded ones included.
    pub verification_method: Vec<VerificationMethod>,
    /// Method ids usable for authentication.
    pub authentication: Vec<String>,
    /// Method ids usable for assertions (credential issuance).
    pub assertion_method: Vec<String>,
    /// Method ids usable for key agreement (encryption to the subject).
    pub key_agreement: Vec<String>,
    /// Service entries.
    pub service: Vec<Service>,
}

/// A DID document that cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentError(pub String);

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid DID document: {}", self.0)
    }
}

impl std::error::Error for DocumentError {}

const PRIVATE_MEMBERS: [&str; 5] = [
    "privateKeyMultibase",
    "secretKeyMultibase",
    "privateKeyBase58",
    "privateKeyJwk",
    "privateKeyHex",
];

fn absolute(id: &str, did: &str) -> String {
    if id.starts_with('#') || id.starts_with('?') || id.starts_with('/') {
        format!("{did}{id}")
    } else {
        id.to_owned()
    }
}

fn string_or_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn read_vm(v: &Value, did: &str) -> Result<VerificationMethod, DocumentError> {
    let obj = v
        .as_object()
        .ok_or_else(|| DocumentError("verification method is not an object".into()))?;
    for p in PRIVATE_MEMBERS {
        if obj.contains_key(p) {
            return Err(DocumentError(format!("verification method carries {p}")));
        }
    }
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| DocumentError("verification method without id".into()))?;
    let id = absolute(id, did);
    let type_ = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("Multikey")
        .to_owned();
    let controller = obj
        .get("controller")
        .and_then(Value::as_str)
        .unwrap_or(did)
        .to_owned();
    let public_key = if let Some(mb) = obj.get("publicKeyMultibase").and_then(Value::as_str) {
        match PublicKey::from_multikey(mb) {
            Ok(k) => k,
            // The 2020 suites carry the raw key without a multicodec prefix.
            Err(_) => raw_multibase(mb, &type_)?,
        }
    } else if let Some(jwk) = obj.get("publicKeyJwk") {
        PublicKey::from_jwk(jwk).map_err(|e| DocumentError(format!("{id}: {e}")))?
    } else if let Some(b58) = obj.get("publicKeyBase58").and_then(Value::as_str) {
        raw_multibase(&format!("z{b58}"), &type_)?
    } else {
        return Err(DocumentError(format!("{id}: no supported public key")));
    };
    Ok(VerificationMethod {
        id,
        type_,
        controller,
        public_key,
    })
}

fn raw_multibase(mb: &str, type_: &str) -> Result<PublicKey, DocumentError> {
    let body = mb
        .strip_prefix('z')
        .ok_or_else(|| DocumentError(format!("{mb}: not base58btc")))?;
    let raw = bs58::decode(body)
        .into_vec()
        .map_err(|e| DocumentError(e.to_string()))?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| DocumentError(format!("{mb}: not a 32-byte key")))?;
    let key_type = if type_.starts_with("X25519") {
        KeyType::X25519
    } else if type_.starts_with("Ed25519") {
        KeyType::Ed25519
    } else {
        return Err(DocumentError(format!("{type_}: unsupported key type")));
    };
    Ok(PublicKey { key_type, bytes })
}

fn read_endpoints(v: Option<&Value>) -> Vec<ServiceEndpoint> {
    match v {
        Some(Value::String(s)) => vec![ServiceEndpoint::uri(s.clone())],
        Some(Value::Object(m)) => {
            let uri = m
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            vec![ServiceEndpoint {
                uri,
                routing_keys: string_or_array(m.get("routingKeys")),
                accept: string_or_array(m.get("accept")),
            }]
        }
        Some(Value::Array(a)) => a.iter().flat_map(|x| read_endpoints(Some(x))).collect(),
        _ => Vec::new(),
    }
}

impl DidDocument {
    /// An empty document for `did`.
    pub fn new(did: impl Into<String>) -> Self {
        DidDocument {
            id: did.into(),
            also_known_as: Vec::new(),
            verification_method: Vec::new(),
            authentication: Vec::new(),
            assertion_method: Vec::new(),
            key_agreement: Vec::new(),
            service: Vec::new(),
        }
    }

    /// Reads a DID document from its JSON (or JSON-LD) representation.
    ///
    /// # Errors
    ///
    /// [`DocumentError`] for a missing `id`, unreadable keys or private key members.
    pub fn from_json(v: &Value) -> Result<Self, DocumentError> {
        let obj = v
            .as_object()
            .ok_or_else(|| DocumentError("not a JSON object".into()))?;
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DocumentError("no id".into()))?
            .to_owned();
        Did::parse(&id).map_err(|e| DocumentError(e.to_string()))?;
        let mut doc = DidDocument::new(id.clone());
        doc.also_known_as = string_or_array(obj.get("alsoKnownAs"));
        if let Some(Value::Array(vms)) = obj.get("verificationMethod") {
            for vm in vms {
                doc.verification_method.push(read_vm(vm, &id)?);
            }
        }
        let mut relation = |name: &str| -> Result<Vec<String>, DocumentError> {
            let mut out = Vec::new();
            if let Some(Value::Array(items)) = obj.get(name) {
                for item in items {
                    match item {
                        Value::String(s) => out.push(absolute(s, &id)),
                        Value::Object(_) => {
                            let vm = read_vm(item, &id)?;
                            out.push(vm.id.clone());
                            if !doc.verification_method.iter().any(|x| x.id == vm.id) {
                                doc.verification_method.push(vm);
                            }
                        }
                        _ => return Err(DocumentError(format!("{name}: bad entry"))),
                    }
                }
            }
            Ok(out)
        };
        let authentication = relation("authentication")?;
        let assertion_method = relation("assertionMethod")?;
        let key_agreement = relation("keyAgreement")?;
        doc.authentication = authentication;
        doc.assertion_method = assertion_method;
        doc.key_agreement = key_agreement;
        if let Some(Value::Array(services)) = obj.get("service") {
            for s in services {
                let sid = s
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DocumentError("service without id".into()))?;
                doc.service.push(Service {
                    id: absolute(sid, &id),
                    types: string_or_array(s.get("type")),
                    endpoints: read_endpoints(s.get("serviceEndpoint")),
                });
            }
        }
        Ok(doc)
    }

    /// The document as DID Core JSON-LD.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert(
            "@context".into(),
            json!([
                "https://www.w3.org/ns/did/v1",
                "https://w3id.org/security/multikey/v1"
            ]),
        );
        m.insert("id".into(), json!(self.id));
        if !self.also_known_as.is_empty() {
            m.insert("alsoKnownAs".into(), json!(self.also_known_as));
        }
        m.insert(
            "verificationMethod".into(),
            Value::Array(
                self.verification_method
                    .iter()
                    .map(VerificationMethod::to_json)
                    .collect(),
            ),
        );
        for (name, ids) in [
            ("authentication", &self.authentication),
            ("assertionMethod", &self.assertion_method),
            ("keyAgreement", &self.key_agreement),
        ] {
            if !ids.is_empty() {
                m.insert(name.into(), json!(ids));
            }
        }
        if !self.service.is_empty() {
            m.insert(
                "service".into(),
                Value::Array(self.service.iter().map(Service::to_json).collect()),
            );
        }
        Value::Object(m)
    }

    /// The verification method with id `id` (absolute or `#fragment`).
    pub fn method(&self, id: &str) -> Option<&VerificationMethod> {
        let id = absolute(id, &self.id);
        self.verification_method.iter().find(|m| m.id == id)
    }

    fn related(&self, ids: &[String]) -> Vec<&VerificationMethod> {
        ids.iter().filter_map(|id| self.method(id)).collect()
    }

    /// The methods listed under `authentication`.
    pub fn authentication_methods(&self) -> Vec<&VerificationMethod> {
        self.related(&self.authentication)
    }

    /// The methods listed under `assertionMethod`.
    pub fn assertion_methods(&self) -> Vec<&VerificationMethod> {
        self.related(&self.assertion_method)
    }

    /// The methods listed under `keyAgreement`.
    pub fn key_agreement_methods(&self) -> Vec<&VerificationMethod> {
        self.related(&self.key_agreement)
    }

    /// True when the method `id` is listed under `authentication`.
    pub fn authenticates_with(&self, id: &str) -> bool {
        let id = absolute(id, &self.id);
        self.authentication.contains(&id)
    }

    /// True when the method `id` is listed under `keyAgreement`.
    pub fn agrees_with(&self, id: &str) -> bool {
        let id = absolute(id, &self.id);
        self.key_agreement.contains(&id)
    }

    /// The service with id `id` (absolute or `#fragment`).
    pub fn service_by_id(&self, id: &str) -> Option<&Service> {
        let id = absolute(id, &self.id);
        self.service.iter().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_syntax() {
        let d = Did::parse("did:web:example.com%3A8443:user:alice").expect("valid");
        assert_eq!(d.method(), "web");
        assert_eq!(d.method_specific_id(), "example.com%3A8443:user:alice");
        assert!(Did::parse("did:Web:x").is_err());
        assert!(Did::parse("did:web:").is_err());
        assert!(Did::parse("vt:actor").is_err());
        assert_eq!(
            Did::from_did_url("did:key:z6Mk#z6Mk")
                .expect("url")
                .as_str(),
            "did:key:z6Mk"
        );
    }

    #[test]
    fn document_refuses_private_keys() {
        let doc = json!({
            "id": "did:example:a",
            "verificationMethod": [{
                "id": "#k", "type": "Multikey", "controller": "did:example:a",
                "publicKeyMultibase": "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                "secretKeyMultibase": "zsecret"
            }]
        });
        assert!(DidDocument::from_json(&doc).is_err());
    }

    #[test]
    fn document_round_trip() {
        let doc = json!({
            "id": "did:example:a",
            "verificationMethod": [{
                "id": "#k", "type": "Multikey", "controller": "did:example:a",
                "publicKeyMultibase": "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
            }],
            "authentication": ["#k"],
            "keyAgreement": [{
                "id": "did:example:a#x", "type": "X25519KeyAgreementKey2020",
                "controller": "did:example:a",
                "publicKeyMultibase": "z6LSj72tK8brWgZja8NLRwPigth2T9QRiG1uH9oKZuKjdh9p"
            }],
            "service": [{
                "id": "#dm", "type": "DIDCommMessaging",
                "serviceEndpoint": {"uri": "https://mediator.example", "routingKeys": ["did:example:m#k"]}
            }]
        });
        let d = DidDocument::from_json(&doc).expect("reads");
        assert_eq!(d.authentication, vec!["did:example:a#k".to_owned()]);
        assert_eq!(d.key_agreement_methods().len(), 1);
        assert_eq!(d.service[0].endpoints[0].routing_keys.len(), 1);
        let again = DidDocument::from_json(&d.to_json()).expect("re-reads");
        assert_eq!(again, d);
    }
}
