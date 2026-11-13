//! The log server behind `vt webvh serve` (feature `server`): serves a directory of published
//! DIDs, laid out by [`log_dir`](crate::publish::log_dir), over HTTP.
//!
//! - `did.jsonl` as `text/jsonl`; `did-witness.json` and `did.json` as `application/json`;
//! - `Access-Control-Allow-Origin: *`;
//! - `Cache-Control: max-age` of the DID's `ttl`, at most 300 seconds;
//! - GET and HEAD only, 404 for anything missing;
//! - the request's host picks `<root>/<host>/`, falling back to a default domain; no path can
//!   leave the root.
//!
//! TLS is the job of a proxy or CDN in front.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;

/// The longest cache lifetime the server announces.
pub const MAX_AGE_CAP: u32 = 300;

struct Site {
    root: PathBuf,
    default_domain: Option<String>,
}

/// A router serving the published DIDs below `root`. Requests whose host has no directory
/// below `root` are served from `default_domain`'s directory, if given.
///
/// # Errors
///
/// When `root` does not exist.
pub fn router(root: &Path, default_domain: Option<String>) -> std::io::Result<Router> {
    let site = Arc::new(Site {
        root: root.canonicalize()?,
        default_domain,
    });
    Ok(
        Router::new().fallback(move |method: Method, headers: HeaderMap, uri: Uri| {
            let site = site.clone();
            async move { respond(&site, &method, &headers, &uri) }
        }),
    )
}

/// Serves `router` on `listener` until the task is dropped.
///
/// # Errors
///
/// When the server fails.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    axum::serve(listener, router).await
}

fn status(code: StatusCode) -> Response {
    let mut r = Response::new(Body::empty());
    *r.status_mut() = code;
    r.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    r
}

/// The `ttl` in force in a `did.jsonl` (the last one any entry set), or 3600.
fn ttl_of(log: &str) -> u32 {
    log.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|e| {
            e.pointer("/parameters/ttl")
                .and_then(serde_json::Value::as_u64)
        })
        .next_back()
        .and_then(|t| u32::try_from(t).ok())
        .unwrap_or(3600)
}

fn safe_segments(path: &str) -> Option<Vec<&str>> {
    let lower = path.to_ascii_lowercase();
    if lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || path.contains('\\')
    {
        return None;
    }
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.iter().any(|s| *s == "." || *s == "..") || segs.is_empty() {
        return None;
    }
    Some(segs)
}

// @tg: implements:: [[openspec:webvh-hosting#Serving Logs]]
// @tg: implements:: [[decisions#vt webvh serve is a static log server]]
fn respond(site: &Site, method: &Method, headers: &HeaderMap, uri: &Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        let mut r = status(StatusCode::METHOD_NOT_ALLOWED);
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    let Some(segs) = safe_segments(uri.path()) else {
        return status(StatusCode::NOT_FOUND);
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let host_dir = [host.as_str(), host.split(':').next().unwrap_or("")]
        .into_iter()
        .filter(|h| !h.is_empty() && !h.contains(['/', '\\']) && *h != "." && *h != "..")
        .map(|h| site.root.join(h))
        .find(|d| d.is_dir())
        .or_else(|| site.default_domain.as_ref().map(|d| site.root.join(d)));
    let Some(dir) = host_dir else {
        return status(StatusCode::NOT_FOUND);
    };
    let mut file = dir;
    for s in &segs {
        file.push(s);
    }
    // Symbolic links and anything else must still resolve inside the root.
    let Ok(real) = file.canonicalize() else {
        return status(StatusCode::NOT_FOUND);
    };
    if !real.starts_with(&site.root) || !real.is_file() {
        return status(StatusCode::NOT_FOUND);
    }
    let Ok(body) = std::fs::read(&real) else {
        return status(StatusCode::NOT_FOUND);
    };
    let name = segs.last().copied().unwrap_or("");
    let content_type = if name.ends_with(".jsonl") {
        "text/jsonl"
    } else if name.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    };
    let ttl = real
        .parent()
        .map(|p| p.join("did.jsonl"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map_or(3600, |l| ttl_of(&l));
    let len = body.len();
    let mut r = Response::new(if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    });
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("max-age={}", ttl.min(MAX_AGE_CAP))) {
        h.insert(header::CACHE_CONTROL, v);
    }
    r
}

/// Serves the published DIDs below `root` on `addr` on a new tokio runtime, blocking the calling
/// thread (the `vt webvh serve` command).
///
/// # Errors
///
/// When `root` does not exist, `addr` cannot be bound, or the server fails.
pub fn serve_blocking(
    root: &Path,
    addr: &str,
    default_domain: Option<String>,
) -> std::io::Result<()> {
    let router = router(root, default_domain)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        serve(listener, router).await
    })
}
