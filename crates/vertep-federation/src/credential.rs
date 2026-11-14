//! Credential-based grants: a grant presented as a W3C Verifiable Credential (VCDM 2.0) secured
//! with an `eddsa-jcs-2022` Data Integrity proof, issued by a DID the receiving actor trusts.
//!
//! The credential's subject is the grantee (`credentialSubject.id`), with the `target` actor and
//! the granted `events`; `validFrom` and `validUntil` bound it. Verification checks the issuer is
//! trusted, the proof's method is one of the issuer's `assertionMethod` keys, the Ed25519
//! signature over the JCS-canonical credential and proof options, the target, and the validity
//! period, and only then yields a [`Grant`].

use std::fmt;

use serde_json::{Map, Value, json};

use crate::data_integrity::{self, ProofError, ProofOptions};
use crate::did::Did;
use crate::grants::Grant;
use crate::identity::ActorIdentity;
use crate::keys::{KeyError, KeyManager};
use crate::resolver::DidResolver;
use crate::time::Timestamp;
use crate::vocab::{GRANT_CONTEXT, GRANT_CREDENTIAL_TYPE};

/// The VCDM 2.0 context.
pub const VC_CONTEXT: &str = "https://www.w3.org/ns/credentials/v2";
const CRYPTOSUITE: &str = data_integrity::EDDSA_JCS_2022;

/// A credential that does not yield a grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialError {
    /// The credential is not a well-formed grant credential.
    Malformed(String),
    /// The issuer is not one the receiver trusts.
    UntrustedIssuer(String),
    /// The issuer's DID could not be resolved.
    IssuerUnresolvable(String),
    /// The proof does not verify against the issuer's assertion keys.
    BadProof(String),
    /// The credential grants rights on another actor.
    WrongTarget(String),
    /// The current time is outside `validFrom`..`validUntil`.
    NotValidNow,
    /// The key manager could not sign.
    Signing(KeyError),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::Malformed(m) => write!(f, "malformed grant credential: {m}"),
            CredentialError::UntrustedIssuer(i) => write!(f, "untrusted issuer {i}"),
            CredentialError::IssuerUnresolvable(m) => write!(f, "issuer unresolvable: {m}"),
            CredentialError::BadProof(m) => write!(f, "proof does not verify: {m}"),
            CredentialError::WrongTarget(t) => write!(f, "credential targets {t}"),
            CredentialError::NotValidNow => f.write_str("credential is not valid now"),
            CredentialError::Signing(e) => write!(f, "cannot sign: {e}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Issues `grant` as a credential signed by `issuer`'s assertion key, valid from `valid_from`
/// until the grant's expiry.
///
/// # Errors
///
/// [`CredentialError::Signing`] when the key manager cannot sign.
pub fn issue_grant_credential(
    km: &dyn KeyManager,
    issuer: &ActorIdentity,
    grant: &Grant,
    valid_from: Timestamp,
) -> Result<Value, CredentialError> {
    let mut cred = Map::new();
    cred.insert("@context".into(), json!([VC_CONTEXT, GRANT_CONTEXT]));
    cred.insert(
        "type".into(),
        json!(["VerifiableCredential", GRANT_CREDENTIAL_TYPE]),
    );
    cred.insert("issuer".into(), json!(issuer.did));
    cred.insert("validFrom".into(), json!(valid_from.to_rfc3339()));
    if let Some(exp) = grant.expires {
        cred.insert("validUntil".into(), json!(Timestamp(exp).to_rfc3339()));
    }
    cred.insert(
        "credentialSubject".into(),
        json!({"id": grant.grantee, "target": grant.target, "events": grant.events}),
    );
    let options = ProofOptions {
        cryptosuite: CRYPTOSUITE.into(),
        verification_method: issuer.signing.kid.clone(),
        purpose: "assertionMethod".into(),
        created: valid_from.to_rfc3339(),
        challenge: None,
        domain: None,
    };
    data_integrity::sign(
        km,
        &issuer.signing.key_name,
        &Value::Object(cred),
        &options,
        &data_integrity::jcs,
    )
    .map_err(|e| match e {
        ProofError::Signing(k) => CredentialError::Signing(k),
        other => CredentialError::Malformed(other.to_string()),
    })
}

fn str_field<'a>(v: &'a Value, name: &str) -> Result<&'a str, CredentialError> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| CredentialError::Malformed(format!("no {name}")))
}

/// Verifies a grant credential presented to the actor `receiver` and returns the grant it
/// carries, with its issuer recorded.
///
/// # Errors
///
/// [`CredentialError`] naming the first check that failed.
// @tg: implements:: [[openspec:did-identity#Credential-Based Grants]]
pub fn verify_grant_credential(
    credential: &Value,
    receiver: &str,
    trusted_issuers: &[String],
    resolver: &dyn DidResolver,
    now: Timestamp,
) -> Result<Grant, CredentialError> {
    let types: Vec<&str> = credential
        .get("type")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !types.contains(&"VerifiableCredential") || !types.contains(&GRANT_CREDENTIAL_TYPE) {
        return Err(CredentialError::Malformed("not a grant credential".into()));
    }
    let issuer = match credential.get("issuer") {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Object(o)) => o
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| CredentialError::Malformed("issuer without id".into()))?,
        _ => return Err(CredentialError::Malformed("no issuer".into())),
    };
    if !trusted_issuers.iter().any(|t| t == issuer) {
        return Err(CredentialError::UntrustedIssuer(issuer.to_owned()));
    }
    let proof = data_integrity::proof_of(credential, CRYPTOSUITE, "assertionMethod").map_err(|e| {
        match e {
            ProofError::Malformed(m) if m == "no proof" => CredentialError::Malformed(m),
            _ => CredentialError::BadProof("unsupported proof".into()),
        }
    })?;
    let vm_id = proof
        .get("verificationMethod")
        .and_then(Value::as_str)
        .ok_or_else(|| CredentialError::BadProof("no verificationMethod".into()))?;
    let issuer_did = Did::parse(issuer).map_err(|e| CredentialError::Malformed(e.to_string()))?;
    if Did::from_did_url(vm_id).ok().as_ref() != Some(&issuer_did) {
        return Err(CredentialError::BadProof(format!(
            "{vm_id} is not a key of {issuer}"
        )));
    }
    let doc = resolver
        .resolve(&issuer_did)
        .map_err(|e| CredentialError::IssuerUnresolvable(e.to_string()))?
        .document;
    let key = doc
        .assertion_methods()
        .into_iter()
        .find(|m| m.id == vm_id)
        .ok_or_else(|| CredentialError::BadProof(format!("{vm_id} is not an assertion key")))?
        .public_key;
    data_integrity::verify(credential, &key, &data_integrity::jcs).map_err(|e| match e {
        ProofError::BadSignature => CredentialError::BadProof("signature mismatch".into()),
        other => CredentialError::BadProof(other.to_string()),
    })?;
    let valid_from = credential
        .get("validFrom")
        .and_then(Value::as_str)
        .map(|s| {
            Timestamp::parse_rfc3339(s)
                .ok_or_else(|| CredentialError::Malformed("bad validFrom".into()))
        })
        .transpose()?;
    let valid_until = credential
        .get("validUntil")
        .and_then(Value::as_str)
        .map(|s| {
            Timestamp::parse_rfc3339(s)
                .ok_or_else(|| CredentialError::Malformed("bad validUntil".into()))
        })
        .transpose()?;
    if valid_from.is_some_and(|f| now < f) || valid_until.is_some_and(|u| now >= u) {
        return Err(CredentialError::NotValidNow);
    }
    let subject = credential
        .get("credentialSubject")
        .ok_or_else(|| CredentialError::Malformed("no credentialSubject".into()))?;
    let target = str_field(subject, "target")?;
    if target != receiver {
        return Err(CredentialError::WrongTarget(target.to_owned()));
    }
    let events: Vec<String> = subject
        .get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if events.is_empty() {
        return Err(CredentialError::Malformed("no events".into()));
    }
    Ok(Grant {
        id: credential
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        grantee: str_field(subject, "id")?.to_owned(),
        target: target.to_owned(),
        events,
        expires: valid_until.map(Timestamp::millis),
        issuer: Some(issuer.to_owned()),
    })
}
