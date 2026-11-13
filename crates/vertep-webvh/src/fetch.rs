//! Fetchers that map `https://<domain>/...` onto a directory or a local server.
//!
//! `did:webvh` resolves domain names over HTTPS, and `didwebvh-rs` rejects IP addresses, so
//! tests and offline verification route domains through these instead of the network. The
//! production resolver path is unchanged: these are ordinary [`HttpFetcher`]s.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use vertep_federation::resolver::{FetchError, HttpFetcher};

fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    Some(match rest.split_once('/') {
        Some((host, path)) => (host, path),
        None => (rest, ""),
    })
}

/// Serves `https://<domain>/<path>` from `<root>/<domain>/<path>`, the layout publishers write.
#[derive(Clone, Debug)]
pub struct DirectoryFetcher {
    root: PathBuf,
}

impl DirectoryFetcher {
    /// A fetcher reading below `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DirectoryFetcher { root: root.into() }
    }
}

impl HttpFetcher for DirectoryFetcher {
    fn get(&self, url: &str, _timeout: Duration) -> Result<Vec<u8>, FetchError> {
        let (host, path) = split_url(url).ok_or(FetchError::NotFound)?;
        if path.split('/').any(|s| s == ".." || s == ".") || host.contains("..") {
            return Err(FetchError::NotFound);
        }
        let path = path.split(['?', '#']).next().unwrap_or("");
        std::fs::read(self.root.join(host).join(path)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FetchError::NotFound,
            _ => FetchError::Network(e.to_string()),
        })
    }
}

/// Rewrites `https://<domain>/...` to a base URL (a local server, for example
/// `http://127.0.0.1:8123`) for the domains it has routes for, and fetches through `inner`.
/// Other URLs go to `inner` unchanged.
#[derive(Clone)]
pub struct RoutedFetcher {
    inner: Arc<dyn HttpFetcher>,
    routes: BTreeMap<String, String>,
}

impl std::fmt::Debug for RoutedFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutedFetcher")
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

impl RoutedFetcher {
    /// A fetcher with no routes yet.
    pub fn new(inner: Arc<dyn HttpFetcher>) -> Self {
        RoutedFetcher {
            inner,
            routes: BTreeMap::new(),
        }
    }

    /// Routes `domain` to `base`.
    #[must_use]
    pub fn route(mut self, domain: impl Into<String>, base: impl Into<String>) -> Self {
        self.routes
            .insert(domain.into(), base.into().trim_end_matches('/').to_owned());
        self
    }
}

impl HttpFetcher for RoutedFetcher {
    // @tg: implements:: [[decisions#Tests route did:webvh domains through the fetcher]]
    fn get(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, FetchError> {
        if let Some((host, path)) = split_url(url)
            && let Some(base) = self.routes.get(host)
        {
            return self.inner.get(&format!("{base}/{path}"), timeout);
        }
        self.inner.get(url, timeout)
    }
}
