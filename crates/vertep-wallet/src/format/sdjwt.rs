//! SD-JWT VC with EdDSA: selective disclosure of claims, bound to the holder through `cnf.jwk`.
//!
//! Issuance is `<issuer JWT>~<disclosure>~…~`; each disclosure is the base64url JSON array
//! `[salt, name, value]`, and the issuer JWT carries the base64url SHA-256 of each in `_sd`. A
//! presentation keeps the issuer JWT, the selected disclosures and a key-binding JWT
//! (`typ` `kb+jwt`) over `aud`, `nonce`, `iat` and `sd_hash`, the hash of everything before it.
//! The credential's id is the digest of the issuer JWT alone, so holder and verifier agree on
//! it whatever was disclosed. Status uses the Bitstring Status List entry in a `status` claim.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vertep_federation::{KeyManager, PublicKey};

use super::{status_from, status_json};
use crate::credential::{
    Credential, CredentialMeta, IssueRequest, KeyLookup, Presentation, SigningKey,
    VerifiedPresentation, controller_of, digest,
};
use crate::error::WalletError;
use crate::history::Claims;
use crate::jose::{self, Jws, b64, unb64};
use crate::plugin::CredentialFormat;
use crate::vocab::FORMAT_SD_JWT;

/// The SD-JWT VC format.
// @tg: implements:: [[openspec:agent-wallet#Credential Formats]] {confidence: partial, note: "SD-JWT VC; Data Integrity in format::di"}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SdJwtVc;

/// The base64url SHA-256 of an ASCII string (disclosure digests and `sd_hash`).
pub fn sd_digest(s: &str) -> String {
    b64(&Sha256::digest(s.as_bytes()))
}

fn salt() -> Result<String, WalletError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| WalletError::Malformed(e.to_string()))?;
    Ok(b64(&b))
}

/// A decoded disclosure.
#[derive(Clone, Debug, PartialEq)]
struct Disclosure {
    encoded: String,
    name: String,
    value: Value,
}

fn decode_disclosure(encoded: &str) -> Result<Disclosure, WalletError> {
    let arr: Value = serde_json::from_slice(&unb64(encoded)?)
        .map_err(|e| WalletError::Malformed(format!("disclosure: {e}")))?;
    let a = arr
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| WalletError::Malformed("disclosure is not [salt, name, value]".into()))?;
    Ok(Disclosure {
        encoded: encoded.to_owned(),
        name: a[1]
            .as_str()
            .ok_or_else(|| WalletError::Malformed("disclosure name".into()))?
            .to_owned(),
        value: a[2].clone(),
    })
}

/// The parts of an SD-JWT: issuer JWT, disclosures, and the key-binding JWT if present.
struct Parts<'a> {
    jwt: &'a str,
    disclosures: Vec<&'a str>,
    kb: Option<&'a str>,
}

fn split(raw: &str) -> Result<Parts<'_>, WalletError> {
    let segs: Vec<&str> = raw.split('~').collect();
    if segs.len() < 2 {
        return Err(WalletError::Malformed("not an SD-JWT".into()));
    }
    let last = segs[segs.len() - 1];
    Ok(Parts {
        jwt: segs[0],
        disclosures: segs[1..segs.len() - 1].to_vec(),
        kb: (!last.is_empty()).then_some(last),
    })
}

struct Checked {
    meta: CredentialMeta,
    disclosed: Map<String, Value>,
    holder: PublicKey,
}

impl SdJwtVc {
    fn check(&self, parts: &Parts<'_>, keys: &dyn KeyLookup) -> Result<Checked, WalletError> {
        let jws = Jws::parse(parts.jwt)?;
        if jws.header.get("typ").and_then(Value::as_str) != Some("dc+sd-jwt") {
            return Err(WalletError::Malformed("typ is not dc+sd-jwt".into()));
        }
        let kid = jws
            .header
            .get("kid")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no kid".into()))?
            .to_owned();
        jws.verify(&keys.key(&kid)?)?;
        let p = &jws.payload;
        let iss = p
            .get("iss")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Malformed("no iss".into()))?
            .to_owned();
        if iss.starts_with("did:") && controller_of(&kid) != iss {
            return Err(WalletError::BadSignature(format!("{kid} is not a key of {iss}")));
        }
        if p.get("_sd_alg").and_then(Value::as_str).unwrap_or("sha-256") != "sha-256" {
            return Err(WalletError::Malformed("_sd_alg is not sha-256".into()));
        }
        let sd: Vec<&str> = p
            .get("_sd")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut disclosed = Map::new();
        for d in &parts.disclosures {
            let dd = decode_disclosure(d)?;
            if !sd.contains(&sd_digest(&dd.encoded).as_str()) {
                return Err(WalletError::BadSignature(format!(
                    "disclosure of {} is not signed by the issuer",
                    dd.name
                )));
            }
            if disclosed.insert(dd.name.clone(), dd.value).is_some() {
                return Err(WalletError::Malformed(format!("claim {} disclosed twice", dd.name)));
            }
        }
        let holder = p
            .get("cnf")
            .and_then(|c| c.get("jwk"))
            .ok_or_else(|| WalletError::Malformed("no cnf.jwk".into()))
            .and_then(|j| PublicKey::from_jwk(j).map_err(WalletError::Key))?;
        let secs = |name: &str| p.get(name).and_then(Value::as_i64).map(|s| s * 1000);
        let meta = CredentialMeta {
            digest: digest(parts.jwt.as_bytes()),
            format: FORMAT_SD_JWT.to_owned(),
            types: p
                .get("vct")
                .and_then(Value::as_str)
                .map(|v| vec![v.to_owned()])
                .unwrap_or_default(),
            issuer: iss,
            issuer_key: kid,
            holder_key: vertep_federation::resolver::key::did_key(&holder),
            valid_from: secs("nbf").or_else(|| secs("iat")),
            valid_until: secs("exp"),
            status: status_from(p.get("status").and_then(|s| s.get("bitstring"))),
            claims: disclosed.keys().cloned().collect(),
            selective: true,
        };
        Ok(Checked {
            meta,
            disclosed,
            holder,
        })
    }
}

impl CredentialFormat for SdJwtVc {
    fn iri(&self) -> &str {
        FORMAT_SD_JWT
    }

    fn issue(
        &self,
        km: &dyn KeyManager,
        issuer: &SigningKey,
        request: &IssueRequest,
    ) -> Result<Credential, WalletError> {
        let mut disclosures = Vec::new();
        let mut sd = Vec::new();
        for (name, value) in &request.claims {
            let enc = b64(
                serde_json::to_string(&json!([salt()?, name, value]))
                    .map_err(|e| WalletError::Malformed(e.to_string()))?
                    .as_bytes(),
            );
            sd.push(Value::from(sd_digest(&enc)));
            disclosures.push(enc);
        }
        sd.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        let mut payload = Map::new();
        payload.insert("iss".into(), Value::from(issuer.did.as_str()));
        payload.insert("iat".into(), Value::from(request.valid_from / 1000));
        payload.insert("nbf".into(), Value::from(request.valid_from / 1000));
        if let Some(u) = request.valid_until {
            payload.insert("exp".into(), Value::from(u / 1000));
        }
        payload.insert(
            "vct".into(),
            Value::from(request.types.first().cloned().unwrap_or_default()),
        );
        payload.insert("cnf".into(), json!({"jwk": request.holder_key.to_jwk()}));
        payload.insert("_sd".into(), Value::Array(sd));
        payload.insert("_sd_alg".into(), Value::from("sha-256"));
        if let Some(s) = &request.status {
            payload.insert("status".into(), json!({"bitstring": status_json(s)}));
        }
        let jwt = jose::sign(
            km,
            &issuer.key_name,
            &json!({"typ": "dc+sd-jwt", "kid": issuer.kid}),
            &Value::Object(payload),
        )?;
        let mut raw = jwt;
        for d in disclosures {
            raw.push('~');
            raw.push_str(&d);
        }
        raw.push('~');
        Ok(Credential {
            format: FORMAT_SD_JWT.to_owned(),
            raw,
        })
    }

    fn verify(
        &self,
        credential: &Credential,
        keys: &dyn KeyLookup,
    ) -> Result<CredentialMeta, WalletError> {
        let parts = split(&credential.raw)?;
        Ok(self.check(&parts, keys)?.meta)
    }

    fn claims(&self, credential: &Credential) -> Result<Map<String, Value>, WalletError> {
        let parts = split(&credential.raw)?;
        let mut out = Map::new();
        for d in parts.disclosures {
            let dd = decode_disclosure(d)?;
            out.insert(dd.name, dd.value);
        }
        Ok(out)
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
        let parts = split(&credential.raw)?;
        let mut kept = Vec::new();
        let mut found = Claims::new();
        for d in &parts.disclosures {
            let dd = decode_disclosure(d)?;
            if disclose.contains(&dd.name) {
                found.insert(dd.name.clone());
                kept.push(*d);
            }
        }
        if &found != disclose {
            let missing: Vec<&String> = disclose.difference(&found).collect();
            return Err(WalletError::Malformed(format!(
                "the credential has no claims {missing:?}"
            )));
        }
        let mut sd_jwt = parts.jwt.to_owned();
        for d in kept {
            sd_jwt.push('~');
            sd_jwt.push_str(d);
        }
        sd_jwt.push('~');
        let kb = jose::sign(
            km,
            &holder.key_name,
            &json!({"typ": "kb+jwt"}),
            &json!({
                "iat": vertep_federation::Timestamp::now().0 / 1000,
                "aud": audience,
                "nonce": nonce,
                "sd_hash": sd_digest(&sd_jwt),
            }),
        )?;
        Ok(Presentation {
            format: FORMAT_SD_JWT.to_owned(),
            raw: format!("{sd_jwt}{kb}"),
        })
    }

    fn verify_presentation(
        &self,
        presentation: &Presentation,
        keys: &dyn KeyLookup,
    ) -> Result<VerifiedPresentation, WalletError> {
        let parts = split(&presentation.raw)?;
        let kb = parts
            .kb
            .ok_or_else(|| WalletError::HolderBinding("no key-binding JWT".into()))?;
        let checked = self.check(&parts, keys)?;
        let kb_jws = Jws::parse(kb)?;
        if kb_jws.header.get("typ").and_then(Value::as_str) != Some("kb+jwt") {
            return Err(WalletError::Malformed("typ is not kb+jwt".into()));
        }
        kb_jws
            .verify(&checked.holder)
            .map_err(|_| WalletError::HolderBinding("key-binding JWT not signed by cnf key".into()))?;
        let presented = &presentation.raw[..presentation.raw.len() - kb.len()];
        if kb_jws.payload.get("sd_hash").and_then(Value::as_str) != Some(&sd_digest(presented)) {
            return Err(WalletError::HolderBinding("sd_hash does not match".into()));
        }
        let field = |n: &str| {
            kb_jws
                .payload
                .get(n)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| WalletError::Malformed(format!("key-binding JWT has no {n}")))
        };
        Ok(VerifiedPresentation {
            signer: checked.meta.holder_key.clone(),
            credential: checked.meta,
            disclosed: checked.disclosed,
            audience: field("aud")?,
            nonce: field("nonce")?,
        })
    }
}
