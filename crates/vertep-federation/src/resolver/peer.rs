//! `did:peer` numalgo 0 (one inception key) and numalgo 2 (several keys and services), resolved
//! offline from the identifier, and the construction of `did:peer:2` identifiers.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};

use crate::did::{Did, DidDocument, Service, ServiceEndpoint, VerificationMethod};
use crate::keys::PublicKey;

use super::{DidResolver, Resolution, ResolutionError, key::key_document};

/// The `did:peer` resolver (numalgo 0 and 2).
#[derive(Clone, Copy, Debug, Default)]
pub struct PeerResolver;

/// What a key of a `did:peer:2` identifier is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerPurpose {
    /// `V`: authentication (and assertion).
    Verification,
    /// `E`: key agreement.
    Encryption,
}

impl PeerPurpose {
    fn prefix(self) -> char {
        match self {
            PeerPurpose::Verification => 'V',
            PeerPurpose::Encryption => 'E',
        }
    }
}

fn abbreviate(s: &Service) -> Value {
    let mut m = Map::new();
    m.insert(
        "t".into(),
        json!(
            s.types
                .iter()
                .map(|t| if t == "DIDCommMessaging" {
                    "dm".to_owned()
                } else {
                    t.clone()
                })
                .next()
                .unwrap_or_default()
        ),
    );
    let ep = s
        .endpoints
        .first()
        .cloned()
        .unwrap_or_else(|| ServiceEndpoint::uri(""));
    let mut e = Map::new();
    e.insert("uri".into(), json!(ep.uri));
    if !ep.accept.is_empty() {
        e.insert("a".into(), json!(ep.accept));
    }
    if !ep.routing_keys.is_empty() {
        e.insert("r".into(), json!(ep.routing_keys));
    }
    m.insert("s".into(), Value::Object(e));
    if let Some((_, frag)) = s.id.split_once('#')
        && !frag.is_empty()
    {
        m.insert("id".into(), json!(format!("#{frag}")));
    }
    Value::Object(m)
}

/// The `did:peer:2` identifier for `keys` (in order) and `services`. Service ids are kept as
/// their `#fragment`.
pub fn did_peer_2(keys: &[(PeerPurpose, PublicKey)], services: &[Service]) -> String {
    let mut s = String::from("did:peer:2");
    for (purpose, key) in keys {
        s.push('.');
        s.push(purpose.prefix());
        s.push_str(&key.multikey());
    }
    for svc in services {
        s.push_str(".S");
        let json = serde_json::to_vec(&abbreviate(svc)).unwrap_or_default();
        s.push_str(&URL_SAFE_NO_PAD.encode(json));
    }
    s
}

fn expand_str(s: &str) -> String {
    if s == "dm" {
        "DIDCommMessaging".into()
    } else {
        s.to_owned()
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn read_service(did: &str, v: &Value, index: usize) -> Result<Service, ResolutionError> {
    let bad = || ResolutionError::InvalidDid(format!("{did}: bad service entry"));
    let obj = v.as_object().ok_or_else(bad)?;
    let types: Vec<String> = str_list(obj.get("t").or_else(|| obj.get("type")))
        .iter()
        .map(|t| expand_str(t))
        .collect();
    let ep = obj
        .get("s")
        .or_else(|| obj.get("serviceEndpoint"))
        .ok_or_else(bad)?;
    let endpoint = match ep {
        Value::String(uri) => ServiceEndpoint {
            uri: uri.clone(),
            routing_keys: str_list(obj.get("r").or_else(|| obj.get("routingKeys"))),
            accept: str_list(obj.get("a").or_else(|| obj.get("accept"))),
        },
        Value::Object(e) => ServiceEndpoint {
            uri: e
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(bad)?
                .to_owned(),
            routing_keys: str_list(e.get("r").or_else(|| e.get("routingKeys"))),
            accept: str_list(e.get("a").or_else(|| e.get("accept"))),
        },
        _ => return Err(bad()),
    };
    let id = match obj.get("id").and_then(Value::as_str) {
        Some(frag) if frag.starts_with('#') => format!("{did}{frag}"),
        Some(full) => full.to_owned(),
        None if index == 0 => format!("{did}#service"),
        None => format!("{did}#service-{index}"),
    };
    Ok(Service {
        id,
        types,
        endpoints: vec![endpoint],
    })
}

fn numalgo_2(did: &str, body: &str) -> Result<DidDocument, ResolutionError> {
    let mut doc = DidDocument::new(did);
    let mut key_index = 0usize;
    let mut service_index = 0usize;
    for element in body.split('.').filter(|e| !e.is_empty()) {
        let (purpose, value) = element.split_at(1);
        match purpose {
            "V" | "E" | "A" => {
                let key = PublicKey::from_multikey(value)
                    .map_err(|e| ResolutionError::InvalidDid(format!("{did}: {e}")))?;
                key_index += 1;
                let id = format!("{did}#key-{key_index}");
                doc.verification_method
                    .push(VerificationMethod::multikey(&id, did, key));
                match purpose {
                    "V" => {
                        doc.authentication.push(id.clone());
                        doc.assertion_method.push(id);
                    }
                    "A" => doc.assertion_method.push(id),
                    _ => doc.key_agreement.push(id),
                }
            }
            "S" => {
                let raw = URL_SAFE_NO_PAD
                    .decode(value.trim_end_matches('='))
                    .map_err(|e| ResolutionError::InvalidDid(format!("{did}: {e}")))?;
                let v: Value = serde_json::from_slice(&raw)
                    .map_err(|e| ResolutionError::InvalidDid(format!("{did}: {e}")))?;
                let entries = match v {
                    Value::Array(a) => a,
                    other => vec![other],
                };
                for entry in entries {
                    doc.service.push(read_service(did, &entry, service_index)?);
                    service_index += 1;
                }
            }
            // Capability invocation and delegation keys are not used by federation.
            "I" | "D" => {}
            _ => {
                return Err(ResolutionError::InvalidDid(format!(
                    "{did}: unknown purpose {purpose}"
                )));
            }
        }
    }
    Ok(doc)
}

impl DidResolver for PeerResolver {
    // @tg: implements:: [[openspec:did-identity#Supported DID Methods]] {confidence: partial, note: "did:peer numalgo 0 and 2"}
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        if did.method() != "peer" {
            return Err(ResolutionError::MethodNotSupported(did.method().to_owned()));
        }
        let msid = did.method_specific_id();
        let doc = if let Some(mk) = msid.strip_prefix('0') {
            key_document(did.as_str(), mk)?
        } else if let Some(body) = msid.strip_prefix('2') {
            numalgo_2(did.as_str(), body)?
        } else {
            return Err(ResolutionError::InvalidDid(format!(
                "{did}: unsupported did:peer numalgo"
            )));
        };
        Ok(Resolution::of(doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::PublicKey;

    #[test]
    fn peer_2_round_trip() {
        let v = PublicKey::from_multikey("z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK")
            .expect("v");
        let e = v.to_x25519().expect("e");
        let svc = Service {
            id: "#didcomm".into(),
            types: vec!["DIDCommMessaging".into()],
            endpoints: vec![ServiceEndpoint {
                uri: "https://mediator.example/".into(),
                routing_keys: vec!["did:key:z6Mk#z6Mk".into()],
                accept: vec!["didcomm/v2".into()],
            }],
        };
        let did = did_peer_2(
            &[(PeerPurpose::Verification, v), (PeerPurpose::Encryption, e)],
            &[svc],
        );
        let doc = PeerResolver
            .resolve(&Did::parse(&did).expect("did"))
            .expect("resolves")
            .document;
        assert_eq!(doc.authentication, vec![format!("{did}#key-1")]);
        assert_eq!(doc.key_agreement, vec![format!("{did}#key-2")]);
        assert_eq!(doc.service[0].id, format!("{did}#didcomm"));
        assert!(doc.service[0].has_type("DIDCommMessaging"));
        assert_eq!(doc.service[0].endpoints[0].routing_keys.len(), 1);
    }
}
