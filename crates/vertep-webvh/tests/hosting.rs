//! `webvh-hosting`: the published layout, the git and command publishers, and the log server.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vertep_federation::resolver::{DidResolver, FetchError, HttpFetcher, WebVhResolver};
use vertep_federation::{Did, KeyType};
use vertep_webvh::witness::CustodianWitness;
use vertep_webvh::{
    CommandPublisher, CreateOptions, DidLog, DirectoryPublisher, GitPublisher, LogPublisher,
    RoutedFetcher, WebvhError, Witnessing,
};

fn created(domain_path: Option<&str>) -> DidLog {
    let keys = common::keys();
    let mut author = common::author(&keys, false);
    let mut options = CreateOptions::new("example.org");
    if let Some(p) = domain_path {
        options = options.path(p);
    }
    let (_, doc) = common::document(&keys, &options, vec![]);
    author.create(&options, &doc).unwrap()
}

// @tg: verifies:: [[openspec:webvh-hosting#Published File Layout#Root and path DIDs]] {level: integration}
#[test]
fn root_and_path_dids() {
    let site = tempfile::tempdir().unwrap();
    let p = DirectoryPublisher::new(site.path());
    let root = created(None);
    let path = created(Some("objects:ticket-8812"));
    p.publish(&root).unwrap();
    p.publish(&path).unwrap();
    for dir in ["example.org/.well-known", "example.org/objects/ticket-8812"] {
        let d = site.path().join(dir);
        assert!(d.join("did.jsonl").is_file(), "{dir}/did.jsonl");
        let web: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(d.join("did.json")).unwrap()).unwrap();
        assert!(
            web["id"]
                .as_str()
                .unwrap()
                .starts_with("did:web:example.org")
        );
        assert!(
            !d.join("did-witness.json").exists(),
            "no witnesses, no witness file"
        );
    }
    assert_eq!(
        p.current(path.did()).unwrap().unwrap().jsonl(),
        path.jsonl(),
        "the publisher reads back what it wrote"
    );
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// @tg: verifies:: [[openspec:webvh-hosting#Log Publishers#Git publish commits]] {level: integration}
#[test]
fn git_publish_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    git(
        tmp.path(),
        &["clone", "-q", remote.to_str().unwrap(), "work"],
    );
    git(&work, &["config", "user.email", "ci@example.org"]);
    git(&work, &["config", "user.name", "CI"]);

    let log = created(Some("objects:ticket-8812"));
    let published = GitPublisher::new(&work)
        .subdir("docs")
        .branch("main")
        .publish(&log)
        .unwrap();
    assert_eq!(published.files.len(), 2, "did.jsonl and did.json");

    let commits = git(&remote, &["log", "--oneline", "main"]);
    assert_eq!(commits.lines().count(), 1, "{commits}");
    assert!(commits.contains("Publish did:webvh:"));
    let files = git(&remote, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(
        files.contains("docs/example.org/objects/ticket-8812/did.jsonl"),
        "{files}"
    );
    assert!(
        files.contains("docs/example.org/objects/ticket-8812/did.json"),
        "{files}"
    );
}

// @tg: verifies:: [[openspec:webvh-hosting#Log Publishers#Command failure reported]] {level: integration}
#[test]
fn command_failure_reported() {
    let site = tempfile::tempdir().unwrap();
    let log = created(None);
    let ok = CommandPublisher::new(
        site.path(),
        "sh",
        vec![
            "-c".into(),
            "test -f \"$VT_WEBVH_DIR/did.jsonl\" && echo synced".into(),
        ],
    )
    .publish(&log)
    .unwrap();
    assert_eq!(ok.detail.trim(), "synced");

    let err = CommandPublisher::new(
        site.path(),
        "sh",
        vec!["-c".into(), "echo bucket unreachable >&2; exit 3".into()],
    )
    .publish(&log)
    .unwrap_err();
    match &err {
        WebvhError::Publish(m) => {
            assert!(m.contains("status 3"), "{m}");
            assert!(m.contains("bucket unreachable"), "{m}");
        }
        other => panic!("expected a publish failure, got {other}"),
    }
}

/// Plain HTTP for the routed fetcher, recording every URL it fetched.
#[derive(Default)]
struct Http {
    fetched: Mutex<Vec<String>>,
}

impl HttpFetcher for Http {
    fn get(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, FetchError> {
        self.fetched.lock().unwrap().push(url.to_owned());
        match ureq::get(url).timeout(timeout).call() {
            Ok(r) => {
                let mut out = Vec::new();
                r.into_reader().read_to_end(&mut out).unwrap();
                Ok(out)
            }
            Err(ureq::Error::Status(404, _)) => Err(FetchError::NotFound),
            Err(ureq::Error::Status(c, _)) => Err(FetchError::Status(c)),
            Err(e) => Err(FetchError::Network(e.to_string())),
        }
    }
}

/// Starts the log server for `root` on a free port; returns its address.
fn start_server(root: &Path, default_domain: &str) -> String {
    let router = vertep_webvh::server::router(root, Some(default_domain.into())).unwrap();
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
            vertep_webvh::server::serve(listener, router).await.unwrap();
        });
    });
    addr.to_string()
}

// @tg: verifies:: [[openspec:webvh-hosting#Serving Logs#Resolver resolves from the server]] {level: integration}
#[test]
fn resolver_resolves_from_the_server() {
    // A DID with one witness, so resolution needs the witness file too.
    let keys = common::keys();
    let wkey = keys.generate(KeyType::Ed25519).unwrap();
    let witness = CustodianWitness::new(keys.clone(), &wkey).unwrap();
    let mut author = common::author(&keys, false);
    let options = CreateOptions::new("example.org")
        .path("objects:ticket-8812")
        .ttl(60)
        .witnesses(Witnessing {
            witnesses: vec![witness.did_key()],
            threshold: 1,
        });
    let (_, doc) = common::document(&keys, &options, vec![]);
    let mut log = author.create(&options, &doc).unwrap();
    let vid = log.version_id().unwrap().to_owned();
    log.add_witness_proof(&vid, &witness.prove(&vid).unwrap())
        .unwrap();

    let site = tempfile::tempdir().unwrap();
    DirectoryPublisher::new(site.path()).publish(&log).unwrap();
    let addr = start_server(site.path(), "example.org");

    let http = Arc::new(Http::default());
    let fetcher = RoutedFetcher::new(http.clone()).route("example.org", format!("http://{addr}"));
    let resolver = WebVhResolver::new(Arc::new(fetcher), Duration::from_secs(5));
    let r = resolver.resolve(&Did::parse(log.did()).unwrap()).unwrap();
    assert_eq!(r.document.id, log.did());
    let fetched = http.fetched.lock().unwrap().clone();
    assert!(
        fetched
            .iter()
            .any(|u| u.ends_with("/objects/ticket-8812/did.jsonl")),
        "{fetched:?}"
    );
    assert!(
        fetched
            .iter()
            .any(|u| u.ends_with("/objects/ticket-8812/did-witness.json")),
        "{fetched:?}"
    );

    // Content types, CORS and the cache lifetime from the ttl.
    let resp = ureq::get(&format!("http://{addr}/objects/ticket-8812/did.jsonl"))
        .call()
        .unwrap();
    assert_eq!(resp.header("content-type"), Some("text/jsonl"));
    assert_eq!(resp.header("access-control-allow-origin"), Some("*"));
    assert_eq!(resp.header("cache-control"), Some("max-age=60"));
    let w = ureq::get(&format!(
        "http://{addr}/objects/ticket-8812/did-witness.json"
    ))
    .call()
    .unwrap();
    assert_eq!(w.header("content-type"), Some("application/json"));
    let head = ureq::head(&format!("http://{addr}/objects/ticket-8812/did.json"))
        .call()
        .unwrap();
    assert_eq!(head.header("content-type"), Some("application/json"));
    match ureq::get(&format!("http://{addr}/objects/nothing/did.jsonl")).call() {
        Err(ureq::Error::Status(404, _)) => {}
        other => panic!("expected 404, got {other:?}"),
    }
    match ureq::post(&format!("http://{addr}/objects/ticket-8812/did.jsonl")).call() {
        Err(ureq::Error::Status(405, _)) => {}
        other => panic!("expected 405, got {other:?}"),
    }
}

fn raw_get(addr: &str, path: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: example.org\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

// @tg: verifies:: [[openspec:webvh-hosting#Serving Logs#Path traversal refused]] {level: integration}
#[test]
fn path_traversal_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let site = tmp.path().join("site");
    std::fs::create_dir_all(site.join("example.org/.well-known")).unwrap();
    std::fs::write(tmp.path().join("secret"), "do not serve").unwrap();
    std::fs::write(site.join("example.org/.well-known/did.json"), "{}").unwrap();
    let addr = start_server(&site, "example.org");

    for path in [
        "/../secret",
        "/../../secret",
        "/%2e%2e/secret",
        "/.well-known/../../../secret",
    ] {
        let resp = raw_get(&addr, path);
        assert!(resp.starts_with("HTTP/1.1 404"), "{path}: {resp}");
        assert!(!resp.contains("do not serve"), "{path}");
    }
    assert!(raw_get(&addr, "/.well-known/did.json").starts_with("HTTP/1.1 200"));
}
