//! Shared helpers: a backdated clock, a key manager and a resolver over published files.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use vertep_federation::resolver::{DidResolver, HttpFetcher, WebVhResolver};
use vertep_federation::{Did, InMemoryKeyManager, KeyManager, Service, ServiceEndpoint};
use vertep_webvh::{Author, Clock, CreateOptions, DirectoryFetcher};

/// A clock two hours in the past that advances one second per entry, so tests never wait for
/// `versionTime` to increase.
pub fn clock() -> Clock {
    let start = Utc::now().timestamp() - 7200;
    let n = Arc::new(AtomicI64::new(0));
    Arc::new(move || {
        let k = n.fetch_add(1, Ordering::SeqCst);
        DateTime::<Utc>::from_timestamp(start + k, 0).unwrap_or_else(Utc::now)
    })
}

pub fn keys() -> Arc<dyn KeyManager> {
    Arc::new(InMemoryKeyManager::new())
}

pub fn author(keys: &Arc<dyn KeyManager>, pre_rotation: bool) -> Author {
    Author::generate(keys.clone(), pre_rotation)
        .expect("update keys")
        .with_clock(clock())
}

pub fn tsp_service(url: &str) -> Service {
    Service {
        id: "#tsp".into(),
        types: vec!["TSPTransport".into()],
        endpoints: vec![ServiceEndpoint::uri(url)],
    }
}

/// A document for `options` from a freshly provisioned identity with `services`.
pub fn document(
    keys: &Arc<dyn KeyManager>,
    options: &CreateOptions,
    services: Vec<Service>,
) -> (vertep_federation::ActorIdentity, serde_json::Value) {
    let identity =
        vertep_webvh::document::provision(keys.as_ref(), options, services).expect("identity");
    let doc = vertep_webvh::document::identity_document(&identity, keys.as_ref()).expect("doc");
    (identity, doc)
}

pub fn resolver_over(root: &Path) -> WebVhResolver {
    let fetcher: Arc<dyn HttpFetcher> = Arc::new(DirectoryFetcher::new(root));
    WebVhResolver::new(fetcher, Duration::from_secs(5))
}

pub fn resolve(root: &Path, did: &str) -> vertep_federation::resolver::Resolution {
    resolver_over(root)
        .resolve(&Did::parse(did).expect("did"))
        .expect("resolves")
}
