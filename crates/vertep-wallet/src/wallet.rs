//! The wallet's operations for the three roles, independent of protocol and chart: the role
//! charts of [`crate::exchange`] call these for every cryptographic and storage step.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vertep_federation::{Clock, KeyManager, PublicKey};

use crate::credential::{
    Credential, CredentialMeta, IssueRequest, KeyLookup, Presentation, PresentationRequest,
    SigningKey, VerifiedPresentation,
};
use crate::error::WalletError;
use crate::history::Claims;
use crate::jose::{self, Jws};
use crate::plugin::{ConsentRequest, CredentialStore, DisclosureOption, WalletRegistry};
use crate::vocab;

/// How a wallet is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletConfig {
    /// The identity the wallet holds credentials for (and issues and verifies as).
    pub did: String,
    /// The verification method id of the signing key (a DID URL).
    pub kid: String,
    /// The signing key's name in the key manager (Ed25519): issuer key, and the holder key
    /// credentials are bound to.
    pub signing_key: String,
    /// Issuers this wallet trusts when it verifies.
    pub trusted_issuers: Vec<String>,
    /// The credential store's IRI.
    pub store: String,
    /// The consent policy's IRI.
    pub consent: String,
    /// The status resolver's IRI, if status is checked.
    pub status: Option<String>,
    /// Whether verification accepts a credential whose status cannot be determined.
    pub accept_unknown_status: bool,
}

impl WalletConfig {
    /// A configuration for `identity` (any DID method): its signing key signs as issuer and
    /// binds credentials as holder; built-in store, consent policy and status resolver.
    pub fn from_identity(identity: &vertep_federation::ActorIdentity) -> Self {
        WalletConfig {
            did: identity.did.clone(),
            kid: identity.signing.kid.clone(),
            signing_key: identity.signing.key_name.clone(),
            trusted_issuers: Vec::new(),
            store: vocab::STORE_SEALED.to_owned(),
            consent: vocab::CONSENT_RULES.to_owned(),
            status: Some(vocab::STATUS_BITSTRING.to_owned()),
            accept_unknown_status: false,
        }
    }

    /// A configuration for a `did:key` identity of the key `signing_key`, with the built-in
    /// sealed store, rule consent policy and Bitstring Status List resolver.
    ///
    /// # Errors
    ///
    /// [`WalletError::Key`] when the key manager does not hold the key.
    pub fn did_key(km: &dyn KeyManager, signing_key: &str) -> Result<Self, WalletError> {
        let k = SigningKey::did_key(km, signing_key)?;
        Ok(WalletConfig {
            did: k.did,
            kid: k.kid,
            signing_key: signing_key.to_owned(),
            trusted_issuers: Vec::new(),
            store: vocab::STORE_SEALED.to_owned(),
            consent: vocab::CONSENT_RULES.to_owned(),
            status: Some(vocab::STATUS_BITSTRING.to_owned()),
            accept_unknown_status: false,
        })
    }
}

/// A wallet: an identity's credentials and the operations of the issuer, holder and verifier
/// roles.
pub struct Wallet {
    config: WalletConfig,
    km: Arc<dyn KeyManager>,
    registry: Arc<WalletRegistry>,
    keys: Arc<dyn KeyLookup>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallet").field("did", &self.config.did).finish_non_exhaustive()
    }
}

impl Wallet {
    /// A wallet over `registry`, which must hold the configured store and consent policy.
    pub fn new(
        config: WalletConfig,
        km: Arc<dyn KeyManager>,
        registry: Arc<WalletRegistry>,
        keys: Arc<dyn KeyLookup>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Wallet {
            config,
            km,
            registry,
            keys,
            clock,
        }
    }

    /// The wallet's identity.
    pub fn did(&self) -> &str {
        &self.config.did
    }

    /// The configuration.
    pub fn config(&self) -> &WalletConfig {
        &self.config
    }

    /// The registry.
    pub fn registry(&self) -> &Arc<WalletRegistry> {
        &self.registry
    }

    /// The key manager.
    pub fn key_manager(&self) -> &Arc<dyn KeyManager> {
        &self.km
    }

    /// The key lookup used to verify.
    pub fn keys(&self) -> &Arc<dyn KeyLookup> {
        &self.keys
    }

    /// Now, in milliseconds.
    pub fn now(&self) -> i64 {
        self.clock.now().0
    }

    /// Trusts `issuer` when verifying.
    pub fn trust(&mut self, issuer: &str) {
        if !self.config.trusted_issuers.iter().any(|i| i == issuer) {
            self.config.trusted_issuers.push(issuer.to_owned());
        }
    }

    /// The signing key (issuer key and holder key).
    ///
    /// # Errors
    ///
    /// [`WalletError::Key`] when the key manager no longer holds it.
    pub fn signing(&self) -> Result<SigningKey, WalletError> {
        Ok(SigningKey {
            key_name: self.config.signing_key.clone(),
            kid: self.config.kid.clone(),
            did: self.config.did.clone(),
            public: self.km.public_key(&self.config.signing_key)?,
        })
    }

    /// The holder key, identified as a `did:key` (what credentials are bound to).
    ///
    /// # Errors
    ///
    /// [`WalletError::Key`] when the key manager no longer holds it.
    // @tg: implements:: [[openspec:agent-wallet#Holder Binding Keys]]
    pub fn holder(&self) -> Result<SigningKey, WalletError> {
        SigningKey::did_key(self.km.as_ref(), &self.config.signing_key)
    }

    fn store(&self) -> Result<Arc<dyn CredentialStore>, WalletError> {
        self.registry.store(&self.config.store)
    }

    // ---- holder -------------------------------------------------------------------------

    /// A possession proof of the holder key over `nonce` for `issuer`: a compact JWS of type
    /// `openid4vci-proof+jwt` with the public key in its header.
    ///
    /// # Errors
    ///
    /// [`WalletError::Key`] when signing fails.
    pub fn prove_possession(&self, issuer: &str, nonce: &str) -> Result<String, WalletError> {
        let h = self.holder()?;
        jose::sign(
            self.km.as_ref(),
            &h.key_name,
            &json!({"typ": "openid4vci-proof+jwt", "kid": h.kid, "jwk": h.public.to_jwk()}),
            &json!({"aud": issuer, "nonce": nonce, "iat": self.now() / 1000}),
        )
    }

    /// Verifies `credential` from `issuer`, checks it is bound to this wallet's holder key,
    /// and stores it sealed.
    ///
    /// # Errors
    ///
    /// [`WalletError`] naming the failed check.
    pub fn accept(&self, credential: &Credential, issuer: &str) -> Result<CredentialMeta, WalletError> {
        let format = self.registry.format(&credential.format)?;
        let meta = format.verify(credential, self.keys.as_ref())?;
        if meta.issuer != issuer {
            return Err(WalletError::UntrustedIssuer(format!(
                "credential from {} offered by {issuer}",
                meta.issuer
            )));
        }
        let mine = self.holder()?.did_key_id();
        if meta.holder_key != mine {
            return Err(WalletError::HolderBinding(format!(
                "credential bound to {}, not {mine}",
                meta.holder_key
            )));
        }
        self.store()?.put(&meta, credential)?;
        Ok(meta)
    }

    /// Every stored credential's metadata.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    pub fn list(&self) -> Result<Vec<CredentialMeta>, WalletError> {
        let mut l = self.store()?.list()?;
        l.sort_by(|a, b| a.digest.cmp(&b.digest));
        Ok(l)
    }

    /// A credential's metadata and claims.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownCredential`] or [`WalletError::Integrity`].
    pub fn show(&self, digest: &str) -> Result<(CredentialMeta, Map<String, Value>), WalletError> {
        let (meta, cred) = self.store()?.get(digest)?;
        let claims = self.registry.format(&cred.format)?.claims(&cred)?;
        Ok((meta, claims))
    }

    /// Deletes a credential: its sealed record is erased.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    pub fn delete(&self, digest: &str) -> Result<(), WalletError> {
        self.store()?.erase(digest)
    }

    fn revoked(&self, meta: &CredentialMeta) -> Result<bool, WalletError> {
        match (&meta.status, &self.config.status) {
            (Some(s), Some(iri)) => self.registry.status(iri)?.is_set(s, &meta.issuer, self.now()),
            _ => Ok(false),
        }
    }

    /// The credentials that may be presented: valid now, not known to be revoked, not deleted.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    // @tg: implements:: [[openspec:agent-wallet#Credential Lifecycle]]
    pub fn live(&self) -> Result<Vec<CredentialMeta>, WalletError> {
        let now = self.now();
        Ok(self
            .list()?
            .into_iter()
            .filter(|m| m.valid_at(now) && !self.revoked(m).unwrap_or(false))
            .collect())
    }

    /// What the consent policy decides on for `request`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    pub fn consent_request(&self, request: &PresentationRequest) -> Result<ConsentRequest, WalletError> {
        Ok(ConsentRequest {
            verifier: request.verifier.clone(),
            nonce: request.nonce.clone(),
            requested: request.claims.clone(),
            types: request.types.clone(),
            purpose: request.purpose.clone(),
            held: self.live()?,
        })
    }

    /// The disclosures the consent policy allows for `request`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`] or [`WalletError::Storage`].
    pub fn options(&self, request: &PresentationRequest) -> Result<Vec<DisclosureOption>, WalletError> {
        let policy = self.registry.consent(&self.config.consent)?;
        Ok(policy.options(&self.consent_request(request)?))
    }

    /// A presentation of credential `digest` disclosing `claims` to `audience` for `nonce`.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the credential is unknown, altered, expired or revoked, or the
    /// format cannot disclose exactly `claims`.
    pub fn present(
        &self,
        digest: &str,
        claims: &Claims,
        audience: &str,
        nonce: &str,
    ) -> Result<(CredentialMeta, Presentation), WalletError> {
        let (meta, cred) = self.store()?.get(digest)?;
        if !meta.valid_at(self.now()) {
            return Err(WalletError::NotValidNow(digest.to_owned()));
        }
        if self.revoked(&meta).unwrap_or(false) {
            return Err(WalletError::Revoked(digest.to_owned()));
        }
        let format = self.registry.format(&cred.format)?;
        let p = format.present(self.km.as_ref(), &cred, &self.holder()?, audience, nonce, claims)?;
        Ok((meta, p))
    }

    // ---- issuer -------------------------------------------------------------------------

    /// Issues `request` in `format`, signed by this wallet's signing key.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the format is unknown or signing fails.
    pub fn issue(&self, format: &str, request: &IssueRequest) -> Result<(CredentialMeta, Credential), WalletError> {
        let f = self.registry.format(format)?;
        let cred = f.issue(self.km.as_ref(), &self.signing()?, request)?;
        let meta = f.verify(&cred, self.keys.as_ref())?;
        Ok((meta, cred))
    }

    /// Checks a possession proof addressed to this issuer over `nonce` and returns the
    /// holder's key.
    ///
    /// # Errors
    ///
    /// [`WalletError::Nonce`], [`WalletError::WrongAudience`] or
    /// [`WalletError::BadSignature`].
    // @tg: implements:: [[openspec:credential-exchange#Possession Before Issuance]] {confidence: partial, note: "the proof check; freshness and single use are in the issuer role chart"}
    pub fn check_possession(&self, proof: &str, nonce: &str) -> Result<PublicKey, WalletError> {
        let jws = Jws::parse(proof)?;
        if jws.header.get("typ").and_then(Value::as_str) != Some("openid4vci-proof+jwt") {
            return Err(WalletError::Malformed("possession proof typ".into()));
        }
        let key = jws
            .header
            .get("jwk")
            .ok_or_else(|| WalletError::Malformed("possession proof has no jwk".into()))
            .and_then(|j| PublicKey::from_jwk(j).map_err(WalletError::Key))?;
        jws.verify(&key)?;
        let aud = jws.payload.get("aud").and_then(Value::as_str).unwrap_or_default();
        if aud != self.config.did {
            return Err(WalletError::WrongAudience(format!("possession proof for {aud}")));
        }
        if jws.payload.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return Err(WalletError::Nonce("possession proof is over another nonce".into()));
        }
        Ok(key)
    }

    // ---- verifier -----------------------------------------------------------------------

    /// Verifies a presentation answering `request`: signatures and holder binding (format),
    /// audience, nonce, trust, validity, status and requested claims, in that order.
    ///
    /// # Errors
    ///
    /// [`WalletError`] naming the first failed check.
    // @tg: implements:: [[openspec:credential-exchange#Bound And Fresh Presentations]] {confidence: partial, note: "the checks; single use per nonce is in the verifier role chart"}
    pub fn verify_presentation(
        &self,
        presentation: &Presentation,
        request: &PresentationRequest,
    ) -> Result<VerifiedPresentation, WalletError> {
        let format = self.registry.format(&presentation.format)?;
        let v = format.verify_presentation(presentation, self.keys.as_ref())?;
        if v.audience != request.verifier {
            return Err(WalletError::WrongAudience(format!(
                "presentation for {}, not {}",
                v.audience, request.verifier
            )));
        }
        if v.nonce != request.nonce {
            return Err(WalletError::Nonce("presentation answers another nonce".into()));
        }
        if !self.config.trusted_issuers.contains(&v.credential.issuer) {
            return Err(WalletError::UntrustedIssuer(v.credential.issuer.clone()));
        }
        if !v.credential.valid_at(self.now()) {
            return Err(WalletError::NotValidNow(v.credential.digest.clone()));
        }
        match self.revoked(&v.credential) {
            Ok(true) => return Err(WalletError::Revoked(v.credential.digest.clone())),
            Ok(false) => {}
            Err(e) if !self.config.accept_unknown_status => return Err(e),
            Err(_) => {}
        }
        if !request.types.is_empty()
            && !v.credential.types.iter().any(|t| request.types.contains(t))
        {
            return Err(WalletError::NoMatchingCredential("credential type not requested".into()));
        }
        if let Some(extra) = v.disclosed.keys().find(|k| !request.claims.contains(*k)) {
            return Err(WalletError::UnrequestedClaim(extra.clone()));
        }
        Ok(v)
    }
}
