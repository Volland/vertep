//! DID resolution: the method resolver trait, the registry through which applications add or
//! override methods, and a bounded resolution cache.
//!
//! Resolution is synchronous like the runtime's `Directory`; the shell runs it off the actor's
//! task and bounds it with the configured timeout. Network methods fetch through an injectable
//! [`HttpFetcher`], so tests and air-gapped deployments resolve offline.

pub mod key;
pub mod peer;
pub mod web;
#[cfg(feature = "webvh")]
pub mod webvh;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::did::{Did, DidDocument};
use crate::error::CommunicationFailure;
use crate::time::{Clock, Timestamp};

pub use key::KeyResolver;
pub use peer::PeerResolver;
pub use web::{FetchError, HttpFetcher, WebResolver};
#[cfg(feature = "webvh")]
pub use webvh::WebVhResolver;

/// A resolved DID: its current document and the resolution metadata federation uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The DID document.
    pub document: DidDocument,
    /// The document version (`did:webvh` version id), when the method has versions.
    pub version_id: Option<String>,
}

impl Resolution {
    /// A resolution with no version metadata.
    pub fn of(document: DidDocument) -> Self {
        Resolution {
            document,
            version_id: None,
        }
    }
}

/// A failed resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionError {
    /// The DID is malformed for its method.
    InvalidDid(String),
    /// No resolver handles the method.
    MethodNotSupported(String),
    /// The DID has no document.
    NotFound(String),
    /// The resolution did not finish within the configured timeout.
    Timeout(String),
    /// A network failure other than a timeout.
    Network(String),
    /// The document is malformed or does not describe the DID.
    InvalidDocument(String),
    /// The method's verification (for example a `did:webvh` log) failed.
    VerificationFailed(String),
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolutionError::InvalidDid(m) => write!(f, "invalid DID {m}"),
            ResolutionError::MethodNotSupported(m) => write!(f, "unsupported DID method did:{m}"),
            ResolutionError::NotFound(m) => write!(f, "DID not found: {m}"),
            ResolutionError::Timeout(m) => write!(f, "DID resolution timed out: {m}"),
            ResolutionError::Network(m) => write!(f, "network error resolving DID: {m}"),
            ResolutionError::InvalidDocument(m) => write!(f, "invalid DID document: {m}"),
            ResolutionError::VerificationFailed(m) => write!(f, "DID verification failed: {m}"),
        }
    }
}

impl std::error::Error for ResolutionError {}

impl ResolutionError {
    /// The communication failure a send to an unresolvable DID reports.
    pub fn to_failure(&self) -> CommunicationFailure {
        match self {
            ResolutionError::MethodNotSupported(m) => {
                CommunicationFailure::UnsupportedMethod(m.clone())
            }
            ResolutionError::InvalidDid(d) => CommunicationFailure::InvalidAddress(d.clone()),
            other => CommunicationFailure::ResolutionFailed(other.to_string()),
        }
    }
}

/// Resolves DIDs. Each DID method has one; [`ResolverRegistry`] dispatches by method.
// @tg: implements:: [[openspec:did-identity#Pluggable DID Resolver]]
pub trait DidResolver: Send + Sync {
    /// Resolves `did` to its current document.
    ///
    /// # Errors
    ///
    /// [`ResolutionError`] when the DID cannot be resolved or verified.
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError>;
}

/// The method resolvers, keyed by DID method. Applications add methods Vertep does not ship,
/// or replace a built-in method, with [`ResolverRegistry::register`].
// @tg: implements:: [[openspec:did-identity#Pluggable DID Resolver]]
// @tg: implements:: [[openspec:did-identity#Supported DID Methods]]
#[derive(Default)]
pub struct ResolverRegistry {
    resolvers: RwLock<BTreeMap<String, Arc<dyn DidResolver>>>,
}

impl fmt::Debug for ResolverRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolverRegistry")
            .field("methods", &self.methods())
            .finish()
    }
}

impl ResolverRegistry {
    /// A registry with no methods.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the offline methods `did:key` and `did:peer` and, when `fetcher` is given,
    /// `did:web` (and `did:webvh` with the `webvh` feature) fetching through it with `timeout`.
    pub fn with_builtin(fetcher: Option<Arc<dyn HttpFetcher>>, timeout: Duration) -> Self {
        let r = Self::new();
        r.register("key", Arc::new(KeyResolver));
        r.register("peer", Arc::new(PeerResolver));
        if let Some(fetcher) = fetcher {
            r.register("web", Arc::new(WebResolver::new(fetcher.clone(), timeout)));
            #[cfg(feature = "webvh")]
            r.register("webvh", Arc::new(WebVhResolver::new(fetcher, timeout)));
        }
        r
    }

    /// Registers `resolver` for the DID method `method`, replacing any earlier one.
    pub fn register(&self, method: &str, resolver: Arc<dyn DidResolver>) {
        if let Ok(mut r) = self.resolvers.write() {
            r.insert(method.to_owned(), resolver);
        }
    }

    /// The registered methods.
    pub fn methods(&self) -> Vec<String> {
        self.resolvers
            .read()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl DidResolver for ResolverRegistry {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        let resolver = self
            .resolvers
            .read()
            .ok()
            .and_then(|r| r.get(did.method()).cloned())
            .ok_or_else(|| ResolutionError::MethodNotSupported(did.method().to_owned()))?;
        let res = resolver.resolve(did)?;
        if res.document.id != did.as_str() {
            return Err(ResolutionError::InvalidDocument(format!(
                "document id {} does not match {did}",
                res.document.id
            )));
        }
        Ok(res)
    }
}

/// One failed delivery attempt against a resolved document, as [`CachingResolver::deliver`]
/// needs to know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptError<E> {
    /// The cached endpoint or key no longer works (endpoint gone, key unknown to the recipient):
    /// worth re-resolving and retrying once.
    Stale(E),
    /// Any other failure: no retry.
    Fatal(E),
}

/// A resolver that caches documents for a bounded lifetime and a bounded number of DIDs, and
/// re-resolves on expiry or on a stale endpoint or key.
// @tg: implements:: [[openspec:did-identity#Resolution Caching]]
pub struct CachingResolver {
    inner: Arc<dyn DidResolver>,
    ttl: Duration,
    max_entries: usize,
    clock: Arc<dyn Clock>,
    cache: Mutex<HashMap<String, (Timestamp, Resolution)>>,
}

impl fmt::Debug for CachingResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachingResolver")
            .field("ttl", &self.ttl)
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

impl CachingResolver {
    /// Caches `inner` for `ttl`, holding at most `max_entries` documents.
    pub fn new(
        inner: Arc<dyn DidResolver>,
        ttl: Duration,
        max_entries: usize,
        clock: Arc<dyn Clock>,
    ) -> Self {
        CachingResolver {
            inner,
            ttl,
            max_entries: max_entries.max(1),
            clock,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Drops the cached document of `did`, so the next resolution goes to the method.
    pub fn invalidate(&self, did: &Did) {
        if let Ok(mut c) = self.cache.lock() {
            c.remove(did.as_str());
        }
    }

    /// Runs `attempt` against the (possibly cached) document of `did`. If it fails with
    /// [`AttemptError::Stale`], the DID is resolved again and `attempt` runs once more with the
    /// fresh document before the failure is reported.
    ///
    /// # Errors
    ///
    /// The resolution error, or the last attempt's error.
    pub fn deliver<T, E>(
        &self,
        did: &Did,
        mut attempt: impl FnMut(&Resolution) -> Result<T, AttemptError<E>>,
    ) -> Result<T, DeliverError<E>> {
        let first = self.resolve(did).map_err(DeliverError::Resolution)?;
        match attempt(&first) {
            Ok(t) => Ok(t),
            Err(AttemptError::Fatal(e)) => Err(DeliverError::Attempt(e)),
            Err(AttemptError::Stale(_)) => {
                self.invalidate(did);
                let fresh = self.resolve(did).map_err(DeliverError::Resolution)?;
                match attempt(&fresh) {
                    Ok(t) => Ok(t),
                    Err(AttemptError::Stale(e) | AttemptError::Fatal(e)) => {
                        Err(DeliverError::Attempt(e))
                    }
                }
            }
        }
    }
}

/// The failure of [`CachingResolver::deliver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliverError<E> {
    /// The DID could not be resolved.
    Resolution(ResolutionError),
    /// The delivery attempt failed (after the one retry for a stale document).
    Attempt(E),
}

impl DidResolver for CachingResolver {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        let now = self.clock.now();
        if let Ok(c) = self.cache.lock()
            && let Some((expires, res)) = c.get(did.as_str())
            && now < *expires
        {
            return Ok(res.clone());
        }
        let res = self.inner.resolve(did)?;
        if let Ok(mut c) = self.cache.lock() {
            c.retain(|_, (exp, _)| now < *exp);
            while c.len() >= self.max_entries {
                let oldest = c
                    .iter()
                    .min_by_key(|(_, (exp, _))| *exp)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        c.remove(&k);
                    }
                    None => break,
                }
            }
            c.insert(did.as_str().to_owned(), (now.plus(self.ttl), res.clone()));
        }
        Ok(res)
    }
}
