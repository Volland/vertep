//! A small blocking HTTP/1.1 client for the external lease authorities: one connection per
//! request (`Connection: close`), plain TCP for the etcd JSON gateway and rustls for the
//! Kubernetes API server, with connect, read and write timeouts.
//!
//! The authorities make a few small JSON requests per tick, so a connection per request keeps
//! the client simple and makes every failure (refused, reset, timed out) a fresh, local error.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[cfg(feature = "kubernetes")]
use std::sync::Arc;

/// A response: the status code and the whole body.
#[derive(Clone, Debug)]
pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    /// The body as JSON.
    pub fn json(&self) -> Result<serde_json::Value, String> {
        serde_json::from_slice(&self.body).map_err(|e| {
            format!(
                "malformed JSON in HTTP {} response: {e}: {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    /// True for a 2xx status.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// The scheme, host and port of a base URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub tls: bool,
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    /// Parses `http://host:port` or `https://host:port` (an optional trailing slash; the port
    /// defaults to 80 or 443; IPv6 hosts in brackets).
    pub fn parse(url: &str) -> Result<Self, String> {
        let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(format!(
                "unsupported URL {url}: expected http:// or https://"
            ));
        };
        let authority = rest.trim_end_matches('/');
        if authority.contains('/') {
            return Err(format!("URL {url} must not have a path"));
        }
        let (host, port) = if let Some(h) = authority.strip_prefix('[') {
            let (h, p) = h
                .split_once(']')
                .ok_or_else(|| format!("bad IPv6 host in {url}"))?;
            (h.to_owned(), p.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_owned(), Some(p)),
                None => (authority.to_owned(), None),
            }
        };
        let port = match port {
            Some(p) => p.parse().map_err(|_| format!("bad port in {url}"))?,
            None if tls => 443,
            None => 80,
        };
        if host.is_empty() {
            return Err(format!("no host in {url}"));
        }
        Ok(Endpoint { tls, host, port })
    }

    fn host_header(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// A client of one HTTP endpoint.
#[derive(Clone)]
pub(crate) struct Client {
    endpoint: Endpoint,
    timeout: Duration,
    headers: Vec<(String, String)>,
    #[cfg(feature = "kubernetes")]
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl core::fmt::Debug for Client {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Client")
            .field("endpoint", &self.endpoint)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client of `url` with `timeout` for connecting and for each read and write.
    pub fn new(url: &str, timeout: Duration) -> Result<Self, String> {
        Ok(Client {
            endpoint: Endpoint::parse(url)?,
            timeout,
            headers: Vec::new(),
            #[cfg(feature = "kubernetes")]
            tls: None,
        })
    }

    /// This client sending `name: value` with every request.
    #[cfg(feature = "kubernetes")]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// This client speaking TLS with `config`.
    #[cfg(feature = "kubernetes")]
    pub fn with_tls(mut self, config: Arc<rustls::ClientConfig>) -> Self {
        self.tls = Some(config);
        self
    }

    /// Sends `method path` with an optional JSON body and reads the whole response.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Response, String> {
        let body = body.map(|b| b.to_string().into_bytes());
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\nUser-Agent: vertep-cluster\r\n",
            self.endpoint.host_header()
        );
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(b) = &body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                b.len()
            ));
        }
        head.push_str("\r\n");
        let mut request = head.into_bytes();
        if let Some(b) = body {
            request.extend_from_slice(&b);
        }
        let tcp = self.connect()?;
        if self.endpoint.tls {
            self.exchange_tls(tcp, &request)
        } else {
            exchange(tcp, &request)
        }
    }

    fn connect(&self) -> Result<TcpStream, String> {
        let addrs = (self.endpoint.host.as_str(), self.endpoint.port)
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {}: {e}", self.endpoint.host))?;
        let mut last = format!("no address for {}", self.endpoint.host);
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(s) => {
                    s.set_read_timeout(Some(self.timeout))
                        .and_then(|()| s.set_write_timeout(Some(self.timeout)))
                        .map_err(|e| e.to_string())?;
                    let _ = s.set_nodelay(true);
                    return Ok(s);
                }
                Err(e) => last = format!("cannot connect to {addr}: {e}"),
            }
        }
        Err(last)
    }

    #[cfg(feature = "kubernetes")]
    fn exchange_tls(&self, tcp: TcpStream, request: &[u8]) -> Result<Response, String> {
        let config = self
            .tls
            .clone()
            .ok_or_else(|| "an https endpoint needs a TLS configuration".to_owned())?;
        let name = rustls::pki_types::ServerName::try_from(self.endpoint.host.clone())
            .map_err(|e| format!("bad server name {}: {e}", self.endpoint.host))?;
        let conn = rustls::ClientConnection::new(config, name).map_err(|e| e.to_string())?;
        exchange(rustls::StreamOwned::new(conn, tcp), request)
    }

    #[cfg(not(feature = "kubernetes"))]
    fn exchange_tls(&self, _: TcpStream, _: &[u8]) -> Result<Response, String> {
        Err("https needs the `kubernetes` feature".into())
    }
}

/// Writes `request` and reads the response from `stream`.
fn exchange<S: Read + Write>(mut stream: S, request: &[u8]) -> Result<Response, String> {
    stream
        .write_all(request)
        .and_then(|()| stream.flush())
        .map_err(|e| format!("cannot send the request: {e}"))?;
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    read_line(&mut r, &mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad HTTP status line {line:?}"))?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        read_line(&mut r, &mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "content-length" {
                length = v.parse::<usize>().ok();
            } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            read_line(&mut r, &mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| format!("bad chunk size {line:?}"))?;
            if size == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])
                .map_err(|e| format!("cannot read the response: {e}"))?;
            line.clear();
            read_line(&mut r, &mut line)?;
        }
    } else if let Some(n) = length {
        body.resize(n, 0);
        r.read_exact(&mut body)
            .map_err(|e| format!("cannot read the response: {e}"))?;
    } else {
        match r.read_to_end(&mut body) {
            Ok(_) => {}
            // a TLS peer closing without close_notify after the whole body
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
            Err(e) => return Err(format!("cannot read the response: {e}")),
        }
    }
    Ok(Response { status, body })
}

fn read_line<R: BufRead>(r: &mut R, line: &mut String) -> Result<(), String> {
    match r.read_line(line) {
        Ok(0) => Err("the connection closed before the response ended".into()),
        Ok(_) => Ok(()),
        Err(e) => Err(format!("cannot read the response: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_parse() {
        let e = Endpoint::parse("http://127.0.0.1:2379").unwrap();
        assert_eq!((e.tls, e.host.as_str(), e.port), (false, "127.0.0.1", 2379));
        let e = Endpoint::parse("https://kube.local/").unwrap();
        assert_eq!((e.tls, e.host.as_str(), e.port), (true, "kube.local", 443));
        let e = Endpoint::parse("https://[::1]:6443").unwrap();
        assert_eq!((e.host.as_str(), e.port), ("::1", 6443));
        assert!(Endpoint::parse("ftp://x").is_err());
        assert!(Endpoint::parse("http://x:1/path").is_err());
    }

    #[test]
    fn chunked_and_sized_bodies_are_read() {
        struct Io(std::io::Cursor<Vec<u8>>);
        impl Read for Io {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(b)
            }
        }
        impl Write for Io {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        let r = exchange(Io(std::io::Cursor::new(chunked.to_vec())), b"").unwrap();
        assert_eq!(r.json().unwrap()["a"], 1);
        let sized = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}trailing";
        let r = exchange(Io(std::io::Cursor::new(sized.to_vec())), b"").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (404, &b"{}"[..]));
    }
}
