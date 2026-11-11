//! Offline test fixtures: canned HTTP, counting resolvers and a static directory of documents.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vertep_federation::resolver::{FetchError, HttpFetcher};
use vertep_federation::{Did, DidDocument, DidResolver, Resolution, ResolutionError};

/// An HTTP fetcher answering from a map of URL to response, counting requests.
#[derive(Default)]
pub struct CannedHttp {
    pub responses: Mutex<HashMap<String, Result<Vec<u8>, FetchError>>>,
    pub requests: Mutex<Vec<String>>,
}

impl CannedHttp {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn serve(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.responses
            .lock()
            .expect("lock")
            .insert(url.to_owned(), Ok(body.into()));
    }

    pub fn fail(&self, url: &str, e: FetchError) {
        self.responses
            .lock()
            .expect("lock")
            .insert(url.to_owned(), Err(e));
    }

    pub fn count(&self) -> usize {
        self.requests.lock().expect("lock").len()
    }
}

impl HttpFetcher for CannedHttp {
    fn get(&self, url: &str, _timeout: Duration) -> Result<Vec<u8>, FetchError> {
        self.requests.lock().expect("lock").push(url.to_owned());
        self.responses
            .lock()
            .expect("lock")
            .get(url)
            .cloned()
            .unwrap_or(Err(FetchError::NotFound))
    }
}

/// A resolver serving documents from memory, counting resolutions.
#[derive(Default)]
pub struct StaticResolver {
    pub docs: Mutex<HashMap<String, DidDocument>>,
    pub calls: AtomicUsize,
}

impl StaticResolver {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn put(&self, doc: DidDocument) {
        self.docs.lock().expect("lock").insert(doc.id.clone(), doc);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DidResolver for StaticResolver {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.docs
            .lock()
            .expect("lock")
            .get(did.as_str())
            .cloned()
            .map(Resolution::of)
            .ok_or_else(|| ResolutionError::NotFound(did.to_string()))
    }
}

/// A resolver that tries `first` and falls back to `second`.
pub struct Chain(pub Arc<dyn DidResolver>, pub Arc<dyn DidResolver>);

impl DidResolver for Chain {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        self.0.resolve(did).or_else(|_| self.1.resolve(did))
    }
}
