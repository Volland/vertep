//! The mediator's HTTP and WebSocket server on axum:
//!
//! | route | |
//! |---|---|
//! | `POST <tsp path>` | one TSP message; the response body is the reply (relationship or mailbox), empty for an accepted routed message. 403 refused, 503 not stored (retry), 404 gone |
//! | `GET <tsp path>/live` | WebSocket live session: binary frames are sealed mailbox messages |
//! | `GET /health` | `ok` |
//! | `GET /metrics` | Prometheus text |
//! | `GET /did.json`, `GET /.well-known/did.json` | the mediator's DID document (`did:web` resolves here) |
//! | `GET <did path>/did.jsonl`, `GET <did path>/did.json` | with `did = "webvh:..."`: its DID log and the `did:web` form of its document |
//!
//! Message handling runs on the blocking pool: it appends to the mailbox journal and may post
//! to the next hop.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use vertep_federation::live::{FrameSink, LiveEndpoint};
use vertep_federation::net::NetError;
use vertep_federation::{DidResolver, ResolverRegistry, SystemClock};

use crate::config::{ConfigError, MediatorConfig};
use crate::http::{HttpNetwork, TSP_MEDIA_TYPE};
use crate::mediator::Mediator;
use crate::store::MailboxStore;

/// The path of the TSP endpoint in `public_url` (`/tsp` for `https://m.example/tsp`).
pub fn tsp_path(public_url: &str) -> String {
    let p = url::Url::parse(public_url)
        .map(|u| u.path().trim_end_matches('/').to_owned())
        .unwrap_or_default();
    if p.is_empty() { "/".into() } else { p }
}

/// Builds the mediator `config` describes: its identity and keys in the data directory, its
/// mailboxes in tiramemsu shard files, an HTTP network and a resolver for `did:key`,
/// `did:peer` and `did:web`.
///
/// # Errors
///
/// [`ConfigError`] when the identity, the store or the mediator cannot be set up.
pub fn build(config: &MediatorConfig) -> Result<Arc<Mediator>, ConfigError> {
    let km = config.key_manager()?;
    let identity = config.identity(km.as_ref())?;
    let backend = vertep_tiramemsu::TiramemsuBackend::open(config.mailbox_dir(), config.shards)
        .map_err(|e| ConfigError(e.to_string()))?;
    let store = MailboxStore::open(Arc::new(backend)).map_err(|e| ConfigError(e.to_string()))?;
    let http = Arc::new(HttpNetwork::default());
    let resolver: Arc<dyn DidResolver> = Arc::new(ResolverRegistry::with_builtin(
        Some(http.clone()),
        Duration::from_secs(10),
    ));
    Mediator::new(
        identity,
        km,
        resolver,
        http,
        store,
        Arc::new(SystemClock),
        config.options(),
    )
    .map_err(|e| ConfigError(e.to_string()))
}

/// The routes of `mediator`, with the TSP endpoint at `tsp_path`.
pub fn router(mediator: Arc<Mediator>, tsp_path: &str) -> Router {
    router_with_log(mediator, tsp_path, None)
}

/// As [`router`], also serving the mediator's own `did:webvh` log: `did.jsonl` and the
/// `did:web` form of its document as `did.json`, below `log.0` (`/.well-known` or the DID's
/// path).
pub fn router_with_log(
    mediator: Arc<Mediator>,
    tsp_path: &str,
    log: Option<(String, vertep_webvh::DidLog)>,
) -> Router {
    let live = if tsp_path == "/" {
        "/live".to_owned()
    } else {
        format!("{tsp_path}/live")
    };
    let mut r = Router::new()
        .route(tsp_path, post(post_tsp))
        .route(&live, get(live_upgrade))
        .route("/health", get(|| async { "ok" }))
        .route("/metrics", get(metrics));
    let mut doc_paths = vec!["/did.json".to_owned(), "/.well-known/did.json".to_owned()];
    if let Some((base, log)) = log {
        let base = base.trim_end_matches('/').to_owned();
        let jsonl = log.jsonl();
        let web = log
            .did_web_document()
            .map(|d| d.to_string())
            .unwrap_or_default();
        r = r
            .route(
                &format!("{base}/did.jsonl"),
                get(move || async move { ([(header::CONTENT_TYPE, "text/jsonl")], jsonl) }),
            )
            .route(
                &format!("{base}/did.json"),
                get(move || async move { ([(header::CONTENT_TYPE, "application/json")], web) }),
            );
        doc_paths.retain(|p| *p != format!("{base}/did.json"));
    }
    for p in doc_paths {
        r = r.route(&p, get(did_document));
    }
    r.with_state(mediator)
}

async fn post_tsp(State(m): State<Arc<Mediator>>, body: Bytes) -> Response {
    let r = tokio::task::spawn_blocking(move || m.handle(&body)).await;
    match r {
        Ok(Ok(reply)) => ([(header::CONTENT_TYPE, TSP_MEDIA_TYPE)], reply).into_response(),
        Ok(Err(NetError::Refused(m))) => (StatusCode::FORBIDDEN, m).into_response(),
        Ok(Err(NetError::Gone(m))) => (StatusCode::NOT_FOUND, m).into_response(),
        Ok(Err(NetError::Unreachable(m))) => (StatusCode::SERVICE_UNAVAILABLE, m).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn metrics(State(m): State<Arc<Mediator>>) -> Response {
    let text = tokio::task::spawn_blocking(move || m.metrics().to_prometheus())
        .await
        .unwrap_or_default();
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], text).into_response()
}

async fn did_document(State(m): State<Arc<Mediator>>) -> Response {
    match m.document() {
        Ok(doc) => (
            [(header::CONTENT_TYPE, "application/did+json")],
            doc.to_json().to_string(),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// @tg: implements:: [[openspec:tsp-mediator#Live Delivery Session]] {note: "the WebSocket binding"}
async fn live_upgrade(State(m): State<Arc<Mediator>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| live_session(m, socket))
}

struct ToClient {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl FrameSink for ToClient {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        self.tx
            .send(frame.to_vec())
            .map_err(|_| NetError::Unreachable("live session closed".into()))
    }
}

async fn live_session(m: Arc<Mediator>, socket: WebSocket) {
    let (mut write, mut read) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let session = m.accept(Arc::new(ToClient { tx }));
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write.send(Message::Binary(frame)).await.is_err() {
                break;
            }
        }
        let _ = write.close().await;
    });
    let mut opened = false;
    while let Some(Ok(msg)) = read.next().await {
        let frame = match msg {
            Message::Binary(b) => b,
            Message::Text(t) => t.into_bytes(),
            Message::Close(_) => break,
            _ => continue,
        };
        let s = session.clone();
        let r = tokio::task::spawn_blocking(move || s.send(&frame)).await;
        match r {
            Ok(Ok(())) => opened = true,
            // A session that does not open (unregistered, revoked, not a live request) ends.
            Ok(Err(_)) if !opened => break,
            Ok(Err(_)) => {}
            Err(_) => break,
        }
    }
    session.closed();
    // Give queued frames (a refusal) a moment to go out before the writer stops.
    tokio::time::sleep(Duration::from_millis(50)).await;
    writer.abort();
}

// @tg: implements:: [[openspec:tsp-mediator#Mediator Operability]]
/// Runs the mediator `config` describes until Ctrl-C: the HTTP server and an expiry sweep
/// every minute.
///
/// # Errors
///
/// [`ConfigError`] when the mediator cannot be built or the address cannot be bound.
pub async fn serve(config: MediatorConfig) -> Result<(), ConfigError> {
    let mediator = build(&config)?;
    let sweeper = mediator.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let m = sweeper.clone();
            let _ = tokio::task::spawn_blocking(move || m.expire()).await;
        }
    });
    let app = router_with_log(
        mediator.clone(),
        &tsp_path(&config.public_url),
        config.webvh_log()?,
    );
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(|e| ConfigError(format!("{}: {e}", config.listen)))?;
    eprintln!(
        "vt-mediator {} serving {} on {}",
        mediator.did(),
        mediator.endpoint(),
        config.listen
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| ConfigError(e.to_string()))
}
