//! The HTTP layer of the SPARQL 1.1 Protocol, pluggable, with a minimal plain-HTTP client.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use vertep_store::StoreError;

/// One `POST` of the SPARQL 1.1 Protocol.
#[derive(Clone, Copy, Debug)]
pub struct HttpRequest<'a> {
    /// The endpoint URL.
    pub url: &'a str,
    /// The request media type (`application/sparql-query` or `application/sparql-update`).
    pub content_type: &'a str,
    /// The accepted response media types.
    pub accept: &'a str,
    /// The request body: the query or update text.
    pub body: &'a [u8],
}

/// A response: its status, media type and body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The media type of the body, without parameters, lower-cased.
    pub content_type: Option<String>,
    /// The body.
    pub body: Vec<u8>,
}

/// Sends SPARQL Protocol requests. Implement it to use another HTTP stack (TLS, proxies,
/// authentication) or to test without a network.
pub trait HttpClient: Send + Sync {
    /// Sends one `POST`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unavailable`] when the endpoint cannot be reached; a response with any
    /// status is `Ok`.
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, StoreError>;
}

/// A minimal HTTP/1.1 client on `std::net` for `http://` URLs, one connection per request.
/// It has no TLS: for `https://` endpoints plug another [`HttpClient`].
#[derive(Clone, Debug)]
pub struct StdHttpClient {
    /// The connect, read and write timeout.
    pub timeout: Duration,
}

impl Default for StdHttpClient {
    fn default() -> Self {
        StdHttpClient {
            timeout: Duration::from_secs(30),
        }
    }
}

/// The host (with port) and path of an `http://` URL.
fn split_url(url: &str) -> Result<(String, u16, String), StoreError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        StoreError::Unsupported(format!(
            "{url}: the built-in client speaks plain http only; plug an HttpClient for TLS"
        ))
    })?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
            h.to_owned(),
            p.parse::<u16>()
                .map_err(|_| StoreError::invalid(format!("{url}: invalid port")))?,
        ),
        _ => (authority.to_owned(), 80),
    };
    if host.is_empty() {
        return Err(StoreError::invalid(format!("{url}: no host")));
    }
    Ok((host, port, path.to_owned()))
}

fn unavailable(url: &str, e: impl std::fmt::Display) -> StoreError {
    StoreError::Unavailable(format!("{url}: {e}"))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, StoreError> {
    let mut out = Vec::new();
    loop {
        let line_end = find(body, b"\r\n").ok_or_else(|| StoreError::backend("truncated chunk"))?;
        let size_text = std::str::from_utf8(&body[..line_end]).map_err(StoreError::backend)?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(StoreError::backend)?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err(StoreError::backend("truncated chunk"));
        }
        out.extend_from_slice(&body[..size]);
        body = body.get(size + 2..).unwrap_or(&[]);
    }
}

/// Parses a complete HTTP/1.x response.
pub(crate) fn parse_response(raw: &[u8]) -> Result<HttpResponse, StoreError> {
    let head_end = find(raw, b"\r\n\r\n").ok_or_else(|| StoreError::backend("no HTTP header"))?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(StoreError::backend)?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| StoreError::Backend(format!("bad status line {status_line:?}")))?;
    let mut content_type = None;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-type" => {
                content_type = Some(
                    v.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase(),
                );
            }
            "content-length" => length = v.parse::<usize>().ok(),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &raw[head_end + 4..];
    let body = if chunked {
        dechunk(body)?
    } else if let Some(n) = length {
        body.get(..n)
            .ok_or_else(|| StoreError::backend("truncated body"))?
            .to_vec()
    } else {
        body.to_vec()
    };
    Ok(HttpResponse {
        status,
        content_type,
        body,
    })
}

impl HttpClient for StdHttpClient {
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, StoreError> {
        let url = request.url;
        let (host, port, path) = split_url(url)?;
        let addr = (host.trim_start_matches('[').trim_end_matches(']'), port)
            .to_socket_addrs()
            .map_err(|e| unavailable(url, e))?
            .next()
            .ok_or_else(|| unavailable(url, "no address"))?;
        let mut stream =
            TcpStream::connect_timeout(&addr, self.timeout).map_err(|e| unavailable(url, e))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|e| unavailable(url, e))?;
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: vertep-oxigraph\r\n\
             Content-Type: {ct}\r\nAccept: {accept}\r\nContent-Length: {len}\r\n\
             Connection: close\r\n\r\n",
            ct = request.content_type,
            accept = request.accept,
            len = request.body.len(),
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(request.body))
            .and_then(|()| stream.flush())
            .map_err(|e| unavailable(url, e))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| unavailable(url, e))?;
        parse_response(&raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            split_url("http://localhost:7878/query").unwrap(),
            ("localhost".into(), 7878, "/query".into())
        );
        assert_eq!(
            split_url("http://example.org").unwrap(),
            ("example.org".into(), 80, "/".into())
        );
        assert!(matches!(
            split_url("https://example.org/sparql"),
            Err(StoreError::Unsupported(_))
        ));
    }

    #[test]
    fn responses() {
        let r = parse_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/sparql-results+json; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}extra",
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(
            r.content_type.as_deref(),
            Some("application/sparql-results+json")
        );
        assert_eq!(r.body, b"{}");
        let c = parse_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(c.body, b"abcde");
        assert!(parse_response(b"garbage").is_err());
    }
}
