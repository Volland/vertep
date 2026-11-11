//! `did:web`: the document is fetched over HTTPS through an injectable [`HttpFetcher`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::did::{Did, DidDocument};

use super::{DidResolver, Resolution, ResolutionError};

/// A fetch failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// HTTP 404 or 410.
    NotFound,
    /// The fetch did not finish within its timeout.
    Timeout,
    /// Another HTTP status.
    Status(u16),
    /// A connection or TLS failure.
    Network(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::NotFound => f.write_str("not found"),
            FetchError::Timeout => f.write_str("timed out"),
            FetchError::Status(s) => write!(f, "HTTP status {s}"),
            FetchError::Network(m) => write!(f, "network error: {m}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetches a document by HTTPS `GET`. The runtime supplies an implementation over its HTTP
/// client; tests supply canned responses. Implementations must give up after `timeout`.
pub trait HttpFetcher: Send + Sync {
    /// The body at `url`.
    ///
    /// # Errors
    ///
    /// [`FetchError`] when the body cannot be fetched within `timeout`.
    fn get(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, FetchError>;
}

/// The URL of the HTTPS resource a `did:web` or `did:webvh` method-specific id (without any
/// SCID) points at, with `file` as the last segment.
pub(crate) fn web_url(msid: &str, file: &str) -> Option<String> {
    let mut parts = msid.split(':');
    let host = parts.next()?.replace("%3A", ":").replace("%3a", ":");
    if host.is_empty() || host.contains('/') {
        return None;
    }
    let path: Vec<&str> = parts.collect();
    if path.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    Some(if path.is_empty() {
        format!("https://{host}/.well-known/{file}")
    } else {
        format!("https://{host}/{}/{file}", path.join("/"))
    })
}

pub(crate) fn fetch_error(url: &str, e: FetchError) -> ResolutionError {
    match e {
        FetchError::NotFound => ResolutionError::NotFound(url.to_owned()),
        FetchError::Timeout => ResolutionError::Timeout(url.to_owned()),
        other => ResolutionError::Network(format!("{url}: {other}")),
    }
}

/// The `did:web` resolver.
#[derive(Clone)]
pub struct WebResolver {
    fetcher: Arc<dyn HttpFetcher>,
    timeout: Duration,
}

impl fmt::Debug for WebResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebResolver")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl WebResolver {
    /// A resolver fetching through `fetcher`, giving up after `timeout`.
    pub fn new(fetcher: Arc<dyn HttpFetcher>, timeout: Duration) -> Self {
        WebResolver { fetcher, timeout }
    }
}

impl DidResolver for WebResolver {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        if did.method() != "web" {
            return Err(ResolutionError::MethodNotSupported(did.method().to_owned()));
        }
        let url = web_url(did.method_specific_id(), "did.json")
            .ok_or_else(|| ResolutionError::InvalidDid(did.to_string()))?;
        let body = self
            .fetcher
            .get(&url, self.timeout)
            .map_err(|e| fetch_error(&url, e))?;
        let json: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| ResolutionError::InvalidDocument(format!("{url}: {e}")))?;
        let doc = DidDocument::from_json(&json)
            .map_err(|e| ResolutionError::InvalidDocument(e.to_string()))?;
        if doc.id != did.as_str() {
            return Err(ResolutionError::InvalidDocument(format!(
                "{url} describes {}, not {did}",
                doc.id
            )));
        }
        Ok(Resolution::of(doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_urls() {
        assert_eq!(
            web_url("example.com", "did.json").as_deref(),
            Some("https://example.com/.well-known/did.json")
        );
        assert_eq!(
            web_url("example.com%3A8443:user:alice", "did.json").as_deref(),
            Some("https://example.com:8443/user/alice/did.json")
        );
        assert!(web_url("example.com:..:x", "did.json").is_none());
    }
}
