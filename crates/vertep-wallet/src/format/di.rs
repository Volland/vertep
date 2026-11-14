//! VCDM 2.0 credentials and presentations with Data Integrity EdDSA proofs.
//!
//! `eddsa-jcs-2022` canonicalises with JCS; `eddsa-rdfc-2022` reads the document as JSON-LD,
//! loading only bundled contexts, and canonicalises the RDF with `vertep-rdf` (RDFC-1.0).
//! Credentials bind to the holder through `credentialSubject.id`, a `did:key`; presentations
//! carry an `authentication` proof by that key with the verifier's nonce as `challenge` and its
//! identifier as `domain`. Data Integrity credentials disclose all their claims.

use oxrdf::Dataset;
use serde_json::{Map, Value, json};
use vertep_federation::KeyManager;
use vertep_federation::data_integrity::{
    self, EDDSA_JCS_2022, EDDSA_RDFC_2022, ProofError, ProofOptions,
};

use super::{VC_CONTEXT, bundled_context, parse_time, rfc3339, status_from, status_json};
use crate::credential::{
    Credential, CredentialMeta, IssueRequest, KeyLookup, Presentation, SigningKey,
    VerifiedPresentation, controller_of, digest,
};
use crate::error::WalletError;
use crate::history::Claims;
use crate::plugin::CredentialFormat;
use crate::vocab::{FORMAT_DI_JCS, FORMAT_DI_RDFC};

/// The RDF canonical form of a JSON-LD document: `eddsa-rdfc-2022`'s transformation.
///
/// # Errors
///
/// A message when the document is not JSON-LD over bundled contexts or cannot be
/// canonicalised.
pub fn rdfc(doc: &Value) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
    let parser = oxjsonld::JsonLdParser::new()
        .for_slice(&bytes)
        .with_load_document_callback(|url, _opts| match bundled_context(url) {
            Some(text) => Ok(oxjsonld::JsonLdRemoteDocument {
                document: text.as_bytes().to_vec(),
                document_url: url.to_owned(),
            }),
            None => Err(format!("remote JSON-LD context {url} is not loaded").into()),
        });
    let mut ds = Dataset::new();
    for quad in parser {
        let q = quad.map_err(|e| e.to_string())?;
        ds.insert(q.as_ref());
    }
    vertep_rdf::canon::canonicalize(&ds)
        .map(|c| c.nquads.into_bytes())
        .map_err(|e| e.to_string())
}

/// The Data Integrity format with one EdDSA cryptosuite.
// @tg: implements:: [[openspec:agent-wallet#Credential Formats]] {confidence: partial, note: "Data Integrity; SD-JWT VC in format::sdjwt"}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataIntegrity {
    rdfc: bool,
}

fn proof_err(e: ProofError) -> WalletError {
    match e {
        ProofError::BadSignature => WalletError::BadSignature("Data Integrity proof".into()),
        ProofError::Signing(k) => WalletError::Key(k),
        other => WalletError::Malformed(other.to_string()),
    }
}

fn str_of<'a>(v: &'a Value, name: &str) -> Result<&'a str, WalletError> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| WalletError::Malformed(format!("no {name}")))
}

impl DataIntegrity {
    /// `eddsa-jcs-2022`.
    pub fn jcs() -> Self {
        DataIntegrity { rdfc: false }
    }

    /// `eddsa-rdfc-2022`.
    pub fn rdfc() -> Self {
        DataIntegrity { rdfc: true }
    }

    /// The cryptosuite name.
    pub fn cryptosuite(&self) -> &'static str {
        if self.rdfc { EDDSA_RDFC_2022 } else { EDDSA_JCS_2022 }
    }

    fn canon(&self) -> &'static dyn Fn(&Value) -> Result<Vec<u8>, String> {
        if self.rdfc { &rdfc } else { &data_integrity::jcs }
    }

    /// Verifies only the proof of a secured document against the key its
    /// `verificationMethod` names, and returns that method.
    ///
    /// # Errors
    ///
    /// [`WalletError`] naming the failed check.
    pub fn verify_proof(
        &self,
        doc: &Value,
        purpose: &str,
        keys: &dyn KeyLookup,
    ) -> Result<String, WalletError> {
        let proof =
            data_integrity::proof_of(doc, self.cryptosuite(), purpose).map_err(proof_err)?;
        let vm = proof
            .get("verificationMethod")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no verificationMethod".into()))?
            .to_owned();
        let key = keys.key(&vm)?;
        data_integrity::verify(doc, &key, self.canon()).map_err(proof_err)?;
        Ok(vm)
    }

    fn meta_of(&self, doc: &Value, issuer_key: String, raw: &str) -> Result<CredentialMeta, WalletError> {
        let issuer = match doc.get("issuer") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Object(o)) => o
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| WalletError::Malformed("issuer without id".into()))?
                .to_owned(),
            _ => return Err(WalletError::Malformed("no issuer".into())),
        };
        if issuer.starts_with("did:") && controller_of(&issuer_key) != issuer {
            return Err(WalletError::BadSignature(format!(
                "{issuer_key} is not a key of {issuer}"
            )));
        }
        let subject = doc
            .get("credentialSubject")
            .and_then(Value::as_object)
            .ok_or_else(|| WalletError::Malformed("no credentialSubject".into()))?;
        let types = doc
            .get("type")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|t| *t != "VerifiableCredential")
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(CredentialMeta {
            digest: digest(raw.as_bytes()),
            format: self.iri().to_owned(),
            types,
            issuer,
            issuer_key,
            holder_key: subject
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            valid_from: parse_time(doc.get("validFrom"))?,
            valid_until: parse_time(doc.get("validUntil"))?,
            status: status_from(doc.get("credentialStatus")),
            claims: subject.keys().filter(|k| *k != "id").cloned().collect(),
            selective: false,
        })
    }
}

impl CredentialFormat for DataIntegrity {
    fn iri(&self) -> &str {
        if self.rdfc { FORMAT_DI_RDFC } else { FORMAT_DI_JCS }
    }

    fn issue(
        &self,
        km: &dyn KeyManager,
        issuer: &SigningKey,
        request: &IssueRequest,
    ) -> Result<Credential, WalletError> {
        let mut types = vec![Value::from("VerifiableCredential")];
        types.extend(request.types.iter().map(|t| Value::from(t.as_str())));
        let mut subject = request.claims.clone();
        subject.insert(
            "id".into(),
            Value::from(vertep_federation::resolver::key::did_key(&request.holder_key)),
        );
        let mut doc = Map::new();
        doc.insert("@context".into(), json!([VC_CONTEXT]));
        doc.insert("type".into(), Value::Array(types));
        doc.insert("issuer".into(), Value::from(issuer.did.as_str()));
        doc.insert("validFrom".into(), Value::from(rfc3339(request.valid_from)));
        if let Some(u) = request.valid_until {
            doc.insert("validUntil".into(), Value::from(rfc3339(u)));
        }
        doc.insert("credentialSubject".into(), Value::Object(subject));
        if let Some(s) = &request.status {
            doc.insert("credentialStatus".into(), status_json(s));
        }
        let options = ProofOptions {
            cryptosuite: self.cryptosuite().into(),
            verification_method: issuer.kid.clone(),
            purpose: "assertionMethod".into(),
            created: rfc3339(request.valid_from),
            challenge: None,
            domain: None,
        };
        let signed = data_integrity::sign(
            km,
            &issuer.key_name,
            &Value::Object(doc),
            &options,
            self.canon(),
        )
        .map_err(proof_err)?;
        Ok(Credential {
            format: self.iri().to_owned(),
            raw: serde_json::to_string(&signed).map_err(|e| WalletError::Malformed(e.to_string()))?,
        })
    }

    fn verify(
        &self,
        credential: &Credential,
        keys: &dyn KeyLookup,
    ) -> Result<CredentialMeta, WalletError> {
        let doc: Value = serde_json::from_str(&credential.raw)
            .map_err(|e| WalletError::Malformed(format!("credential JSON: {e}")))?;
        let vm = self.verify_proof(&doc, "assertionMethod", keys)?;
        self.meta_of(&doc, vm, &credential.raw)
    }

    fn claims(&self, credential: &Credential) -> Result<Map<String, Value>, WalletError> {
        let doc: Value = serde_json::from_str(&credential.raw)
            .map_err(|e| WalletError::Malformed(format!("credential JSON: {e}")))?;
        let mut subject = doc
            .get("credentialSubject")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| WalletError::Malformed("no credentialSubject".into()))?;
        subject.remove("id");
        Ok(subject)
    }

    fn present(
        &self,
        km: &dyn KeyManager,
        credential: &Credential,
        holder: &SigningKey,
        audience: &str,
        nonce: &str,
        disclose: &Claims,
    ) -> Result<Presentation, WalletError> {
        let all: Claims = self.claims(credential)?.keys().cloned().collect();
        if &all != disclose {
            return Err(WalletError::Malformed(
                "a Data Integrity credential discloses all of its claims".into(),
            ));
        }
        let vc: Value = serde_json::from_str(&credential.raw)
            .map_err(|e| WalletError::Malformed(e.to_string()))?;
        let vp = json!({
            "@context": [VC_CONTEXT],
            "type": ["VerifiablePresentation"],
            "holder": holder.did_key_id(),
            "verifiableCredential": [vc],
        });
        let options = ProofOptions {
            cryptosuite: self.cryptosuite().into(),
            verification_method: holder.kid.clone(),
            purpose: "authentication".into(),
            created: vertep_federation::Timestamp::now().to_rfc3339(),
            challenge: Some(nonce.to_owned()),
            domain: Some(audience.to_owned()),
        };
        let signed = data_integrity::sign(km, &holder.key_name, &vp, &options, self.canon())
            .map_err(proof_err)?;
        Ok(Presentation {
            format: self.iri().to_owned(),
            raw: serde_json::to_string(&signed).map_err(|e| WalletError::Malformed(e.to_string()))?,
        })
    }

    fn verify_presentation(
        &self,
        presentation: &Presentation,
        keys: &dyn KeyLookup,
    ) -> Result<VerifiedPresentation, WalletError> {
        let vp: Value = serde_json::from_str(&presentation.raw)
            .map_err(|e| WalletError::Malformed(format!("presentation JSON: {e}")))?;
        let vm = self.verify_proof(&vp, "authentication", keys)?;
        let proof = vp.get("proof").cloned().unwrap_or_default();
        let nonce = str_of(&proof, "challenge")?.to_owned();
        let audience = str_of(&proof, "domain")?.to_owned();
        let vc = vp
            .get("verifiableCredential")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .ok_or_else(|| WalletError::Malformed("no verifiableCredential".into()))?;
        let raw = serde_json::to_string(vc).map_err(|e| WalletError::Malformed(e.to_string()))?;
        let meta = self.verify(
            &Credential {
                format: self.iri().to_owned(),
                raw,
            },
            keys,
        )?;
        let signer = controller_of(&vm).to_owned();
        if signer != meta.holder_key {
            return Err(WalletError::HolderBinding(format!(
                "presentation signed by {signer}, credential bound to {}",
                meta.holder_key
            )));
        }
        let mut disclosed = vc
            .get("credentialSubject")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        disclosed.remove("id");
        Ok(VerifiedPresentation {
            credential: meta,
            disclosed,
            signer,
            audience,
            nonce,
        })
    }
}
