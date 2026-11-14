//! W3C Bitstring Status List (`agent-wallet` requirement `Credential Status`, design D12).
//!
//! An issuer keeps a bitstring per purpose (at least 131,072 entries), sets an entry to revoke,
//! and publishes the list as a credential whose `encodedList` is the multibase (`u`) base64url
//! of the GZIP-compressed bitstring; index 0 is the most significant bit of the first byte.
//! Verifiers fetch the list credential, verify it is issued by the credential's issuer, and
//! read the entry. Unknown status fails closed unless the verifier's policy accepts it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use vertep_federation::KeyManager;

use crate::credential::{Credential, KeyLookup, SigningKey, StatusRef};
use crate::error::WalletError;
use crate::format::{VC_CONTEXT, rfc3339};
use crate::jose::{b64, unb64};
use crate::plugin::{CredentialFormat, StatusResolver, WalletRegistry};

/// The minimum number of entries in a status list.
pub const MIN_ENTRIES: usize = 131_072;

/// A bitstring of status entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitstring(Vec<u8>);

impl Default for Bitstring {
    fn default() -> Self {
        Bitstring(vec![0; MIN_ENTRIES / 8])
    }
}

impl Bitstring {
    /// Whether entry `i` is set.
    pub fn get(&self, i: u64) -> bool {
        let i = i as usize;
        self.0
            .get(i / 8)
            .is_some_and(|b| b & (0x80 >> (i % 8)) != 0)
    }

    /// Sets entry `i`.
    pub fn set(&mut self, i: u64) {
        let i = i as usize;
        if i / 8 >= self.0.len() {
            self.0.resize(i / 8 + 1, 0);
        }
        self.0[i / 8] |= 0x80 >> (i % 8);
    }

    /// The `encodedList`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`] when compression fails.
    pub fn encode(&self) -> Result<String, WalletError> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&self.0)
            .map_err(|e| WalletError::Storage(e.to_string()))?;
        let bytes = gz.finish().map_err(|e| WalletError::Storage(e.to_string()))?;
        Ok(format!("u{}", b64(&bytes)))
    }

    /// Decodes an `encodedList`.
    ///
    /// # Errors
    ///
    /// [`WalletError::StatusUnavailable`] when it is not a valid list.
    pub fn decode(s: &str) -> Result<Self, WalletError> {
        let body = s
            .strip_prefix('u')
            .ok_or_else(|| WalletError::StatusUnavailable("encodedList is not multibase base64url".into()))?;
        let gz = unb64(body).map_err(|e| WalletError::StatusUnavailable(e.to_string()))?;
        let mut out = Vec::new();
        GzDecoder::new(&gz[..])
            .read_to_end(&mut out)
            .map_err(|e| WalletError::StatusUnavailable(format!("encodedList: {e}")))?;
        if out.len() * 8 < MIN_ENTRIES {
            return Err(WalletError::StatusUnavailable("status list is shorter than 131,072 entries".into()));
        }
        Ok(Bitstring(out))
    }
}

/// An issuer's status list for one purpose.
#[derive(Clone, Debug)]
pub struct StatusList {
    /// Where the list credential is published.
    pub url: String,
    /// `revocation` or `suspension`.
    pub purpose: String,
    bits: Bitstring,
    next: u64,
}

impl StatusList {
    /// An empty list published at `url`.
    pub fn new(url: &str, purpose: &str) -> Self {
        StatusList {
            url: url.to_owned(),
            purpose: purpose.to_owned(),
            bits: Bitstring::default(),
            next: 0,
        }
    }

    /// Allocates the next entry.
    pub fn allocate(&mut self) -> StatusRef {
        let index = self.next;
        self.next += 1;
        StatusRef {
            list: self.url.clone(),
            index,
            purpose: self.purpose.clone(),
        }
    }

    /// Sets entry `index` (revokes the credential that holds it).
    pub fn revoke(&mut self, index: u64) {
        self.bits.set(index);
    }

    /// Whether entry `index` is set.
    pub fn is_set(&self, index: u64) -> bool {
        self.bits.get(index)
    }

    /// The status list credential, issued by `issuer` in `format` (a Data Integrity format),
    /// valid from `now` for `ttl_ms`.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when signing fails.
    pub fn publish(
        &self,
        km: &dyn KeyManager,
        issuer: &SigningKey,
        now: i64,
        ttl_ms: i64,
    ) -> Result<Credential, WalletError> {
        let doc = json!({
            "@context": [VC_CONTEXT],
            "id": self.url,
            "type": ["VerifiableCredential", "BitstringStatusListCredential"],
            "issuer": issuer.did,
            "validFrom": rfc3339(now),
            "validUntil": rfc3339(now + ttl_ms),
            "credentialSubject": {
                "id": format!("{}#list", self.url),
                "type": "BitstringStatusList",
                "statusPurpose": self.purpose,
                "encodedList": self.bits.encode()?,
                "ttl": ttl_ms,
            }
        });
        let options = vertep_federation::data_integrity::ProofOptions {
            cryptosuite: vertep_federation::data_integrity::EDDSA_JCS_2022.into(),
            verification_method: issuer.kid.clone(),
            purpose: "assertionMethod".into(),
            created: rfc3339(now),
            challenge: None,
            domain: None,
        };
        let signed = vertep_federation::data_integrity::sign(
            km,
            &issuer.key_name,
            &doc,
            &options,
            &vertep_federation::data_integrity::jcs,
        )
        .map_err(|e| WalletError::Malformed(e.to_string()))?;
        Ok(Credential {
            format: crate::vocab::FORMAT_DI_JCS.to_owned(),
            raw: signed.to_string(),
        })
    }
}

/// Reads entry `status` of a fetched list credential issued by `issuer`, at `now`.
///
/// # Errors
///
/// [`WalletError::StatusUnavailable`] when the list does not verify, is from another issuer,
/// is expired or has another purpose.
pub fn read_entry(
    list: &Credential,
    status: &StatusRef,
    issuer: &str,
    keys: &dyn KeyLookup,
    now: i64,
) -> Result<bool, WalletError> {
    let format = crate::format::di::DataIntegrity::jcs();
    let meta = format
        .verify(list, keys)
        .map_err(|e| WalletError::StatusUnavailable(format!("{}: {e}", status.list)))?;
    if meta.issuer != issuer {
        return Err(WalletError::StatusUnavailable(format!(
            "{} is issued by {}, not {issuer}",
            status.list, meta.issuer
        )));
    }
    if !meta.valid_at(now) {
        return Err(WalletError::StatusUnavailable(format!("{} is expired", status.list)));
    }
    let doc: Value = serde_json::from_str(&list.raw)
        .map_err(|e| WalletError::StatusUnavailable(e.to_string()))?;
    let subject = doc.get("credentialSubject").cloned().unwrap_or_default();
    if subject.get("statusPurpose").and_then(Value::as_str) != Some(status.purpose.as_str()) {
        return Err(WalletError::StatusUnavailable(format!(
            "{} is not a {} list",
            status.list, status.purpose
        )));
    }
    let encoded = subject
        .get("encodedList")
        .and_then(Value::as_str)
        .ok_or_else(|| WalletError::StatusUnavailable("no encodedList".into()))?;
    Ok(Bitstring::decode(encoded)?.get(status.index))
}

/// Status lists published in process: a map from list URL to the latest list credential. For
/// local agents, tests and the conformance kit; [`HttpStatus`] fetches over HTTP.
// @tg: implements:: [[openspec:agent-wallet#Credential Status]]
#[derive(Clone)]
pub struct LocalStatus {
    lists: Arc<Mutex<BTreeMap<String, Credential>>>,
    keys: Arc<dyn KeyLookup>,
}

impl std::fmt::Debug for LocalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalStatus")
    }
}

impl LocalStatus {
    /// An empty set of lists, verified with `keys`.
    pub fn new(keys: Arc<dyn KeyLookup>) -> Self {
        LocalStatus {
            lists: Arc::default(),
            keys,
        }
    }

    /// Publishes (or replaces) the list credential at `url`.
    pub fn publish(&self, url: &str, list: Credential) {
        if let Ok(mut m) = self.lists.lock() {
            m.insert(url.to_owned(), list);
        }
    }

    /// Removes the list at `url` (it becomes unreachable).
    pub fn withdraw(&self, url: &str) {
        if let Ok(mut m) = self.lists.lock() {
            m.remove(url);
        }
    }
}

impl StatusResolver for LocalStatus {
    fn is_set(&self, status: &StatusRef, issuer: &str, now: i64) -> Result<bool, WalletError> {
        let list = self
            .lists
            .lock()
            .ok()
            .and_then(|m| m.get(&status.list).cloned())
            .ok_or_else(|| WalletError::StatusUnavailable(format!("{} cannot be fetched", status.list)))?;
        read_entry(&list, status, issuer, self.keys.as_ref(), now)
    }
}

/// Status lists fetched over HTTP and cached for at most their `ttl` and `max_age_ms`.
#[cfg(feature = "http-status")]
pub struct HttpStatus {
    keys: Arc<dyn KeyLookup>,
    max_age_ms: i64,
    cache: Mutex<BTreeMap<String, (i64, Credential)>>,
}

#[cfg(feature = "http-status")]
impl HttpStatus {
    /// A resolver verifying lists with `keys` and caching them at most `max_age_ms`.
    pub fn new(keys: Arc<dyn KeyLookup>, max_age_ms: i64) -> Self {
        HttpStatus {
            keys,
            max_age_ms,
            cache: Mutex::default(),
        }
    }

    fn fetch(&self, url: &str, now: i64) -> Result<Credential, WalletError> {
        if let Ok(c) = self.cache.lock()
            && let Some((at, list)) = c.get(url)
        {
            let ttl = serde_json::from_str::<Value>(&list.raw)
                .ok()
                .and_then(|d| d.pointer("/credentialSubject/ttl").and_then(Value::as_i64))
                .unwrap_or(self.max_age_ms);
            if now - at < ttl.min(self.max_age_ms) {
                return Ok(list.clone());
            }
        }
        let body = ureq::get(url)
            .call()
            .map_err(|e| WalletError::StatusUnavailable(format!("{url}: {e}")))?
            .into_string()
            .map_err(|e| WalletError::StatusUnavailable(format!("{url}: {e}")))?;
        let list = Credential {
            format: crate::vocab::FORMAT_DI_JCS.to_owned(),
            raw: body,
        };
        if let Ok(mut c) = self.cache.lock() {
            c.insert(url.to_owned(), (now, list.clone()));
        }
        Ok(list)
    }
}

#[cfg(feature = "http-status")]
impl StatusResolver for HttpStatus {
    fn is_set(&self, status: &StatusRef, issuer: &str, now: i64) -> Result<bool, WalletError> {
        let list = self.fetch(&status.list, now)?;
        read_entry(&list, status, issuer, self.keys.as_ref(), now)
    }
}

/// Registers `resolver` as the built-in Bitstring Status List resolver of `registry`.
pub fn register(registry: &WalletRegistry, resolver: Arc<dyn StatusResolver>) {
    registry.register_status(crate::vocab::STATUS_BITSTRING, resolver);
}
