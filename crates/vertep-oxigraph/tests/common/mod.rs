//! A local SPARQL 1.1 Protocol endpoint for tests: an in-memory Oxigraph store behind a
//! single-threaded HTTP server on 127.0.0.1, recording every request, and switchable to
//! answering 503 to simulate an outage.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use oxigraph::io::{RdfFormat, RdfSerializer};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use sparesults::{QueryResultsFormat, QueryResultsSerializer};

/// One request the endpoint received.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub content_type: String,
    pub body: String,
}

pub struct TestEndpoint {
    pub base: String,
    pub store: Store,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    down: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
    handle: Option<JoinHandle<()>>,
}

impl TestEndpoint {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let store = Store::new().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let down = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (store, requests, down, stop) =
                (store.clone(), requests.clone(), down.clone(), stop.clone());
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(conn) = conn {
                        serve(conn, &store, &requests, &down);
                    }
                }
            })
        };
        TestEndpoint {
            base: format!("http://{addr}"),
            store,
            requests,
            down,
            stop,
            addr,
            handle: Some(handle),
        }
    }

    pub fn query_url(&self) -> String {
        format!("{}/query", self.base)
    }

    pub fn update_url(&self) -> String {
        format!("{}/update", self.base)
    }

    pub fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    pub fn take_requests(&self) -> Vec<Recorded> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

impl Drop for TestEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_request(conn: &mut TcpStream) -> Option<Recorded> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = conn.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let mut length = 0;
    let mut content_type = String::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "content-type" => content_type = v.trim().to_owned(),
                _ => {}
            }
        }
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = conn.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Recorded {
        method,
        path,
        content_type,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn respond(conn: &mut TcpStream, status: &str, content_type: &str, body: &[u8], chunked: bool) {
    let mut out =
        format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nConnection: close\r\n");
    if chunked {
        out.push_str("Transfer-Encoding: chunked\r\n\r\n");
        let mut bytes = out.into_bytes();
        for part in body.chunks(7) {
            bytes.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            bytes.extend_from_slice(part);
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(b"0\r\n\r\n");
        let _ = conn.write_all(&bytes);
    } else {
        out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(body);
        let _ = conn.write_all(&bytes);
    }
}

fn serve(mut conn: TcpStream, store: &Store, requests: &Mutex<Vec<Recorded>>, down: &AtomicBool) {
    let Some(req) = read_request(&mut conn) else {
        return;
    };
    requests.lock().unwrap().push(req.clone());
    if down.load(Ordering::SeqCst) {
        respond(
            &mut conn,
            "503 Service Unavailable",
            "text/plain",
            b"down for maintenance",
            false,
        );
        return;
    }
    match req.path.as_str() {
        "/update" => {
            let result = SparqlEvaluator::new()
                .parse_update(&req.body)
                .map_err(|e| e.to_string())
                .and_then(|u| u.on_store(store).execute().map_err(|e| e.to_string()));
            match result {
                Ok(()) => respond(&mut conn, "204 No Content", "text/plain", b"", false),
                Err(e) => respond(
                    &mut conn,
                    "400 Bad Request",
                    "text/plain",
                    e.as_bytes(),
                    false,
                ),
            }
        }
        "/query" => {
            let prepared = match SparqlEvaluator::new().parse_query(&req.body) {
                Ok(p) => p,
                Err(e) => {
                    respond(
                        &mut conn,
                        "400 Bad Request",
                        "text/plain",
                        e.to_string().as_bytes(),
                        false,
                    );
                    return;
                }
            };
            match prepared.on_store(store).execute() {
                Ok(QueryResults::Boolean(b)) => {
                    let body = QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                        .serialize_boolean_to_writer(Vec::new(), b)
                        .unwrap();
                    respond(
                        &mut conn,
                        "200 OK",
                        "application/sparql-results+json",
                        &body,
                        false,
                    );
                }
                Ok(QueryResults::Solutions(solutions)) => {
                    let mut w = QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                        .serialize_solutions_to_writer(Vec::new(), solutions.variables().to_vec())
                        .unwrap();
                    for s in solutions {
                        let s = s.unwrap();
                        w.serialize(s.iter()).unwrap();
                    }
                    let body = w.finish().unwrap();
                    respond(
                        &mut conn,
                        "200 OK",
                        "application/sparql-results+json; charset=utf-8",
                        &body,
                        false,
                    );
                }
                Ok(QueryResults::Graph(triples)) => {
                    let mut w =
                        RdfSerializer::from_format(RdfFormat::NTriples).for_writer(Vec::new());
                    for t in triples {
                        w.serialize_triple(&t.unwrap()).unwrap();
                    }
                    let body = w.finish().unwrap();
                    respond(&mut conn, "200 OK", "application/n-triples", &body, true);
                }
                Err(e) => respond(
                    &mut conn,
                    "500 Internal Server Error",
                    "text/plain",
                    e.to_string().as_bytes(),
                    false,
                ),
            }
        }
        _ => respond(&mut conn, "404 Not Found", "text/plain", b"", false),
    }
}
