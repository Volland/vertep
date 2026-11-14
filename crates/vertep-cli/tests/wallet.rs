//! `vt wallet`, and validation and certification of the wallet's role charts.

use std::path::Path;
use std::process::{Command, Output};

const VT: &str = env!("CARGO_BIN_EXE_vt");

fn vt(args: &[&str]) -> Output {
    let mut c = Command::new(VT);
    c.args(args);
    let lean = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean/.lake/build/bin/vt-certify");
    if std::env::var_os("VT_CERTIFY").is_none() && lean.is_file() {
        c.env("VT_CERTIFY", lean);
    }
    c.output().expect("vt runs")
}

fn ok(args: &[&str]) -> String {
    let out = vt(args);
    assert!(out.status.success(), "vt {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn field<'a>(out: &'a str, name: &str) -> &'a str {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")))
        .unwrap_or_else(|| panic!("no {name} in {out}"))
}

struct Three {
    _tmp: tempfile::TempDir,
    issuer: String,
    holder: String,
    verifier: String,
    i: String,
    h: String,
    v: String,
    root: std::path::PathBuf,
}

fn three() -> Three {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = |n: &str| root.join(n).display().to_string();
    let (i, h, v) = (dir("issuer"), dir("holder"), dir("verifier"));
    let issuer = field(&ok(&["wallet", "create", "--dir", &i]), "did").to_owned();
    let holder = field(&ok(&["wallet", "create", "--dir", &h]), "did").to_owned();
    let verifier = field(&ok(&["wallet", "create", "--dir", &v]), "did").to_owned();
    Three { _tmp: tmp, issuer, holder, verifier, i, h, v, root }
}

fn presented(t: &Three, format: &str) -> (String, String) {
    let p = |n: &str| t.root.join(n).display().to_string();
    let (cred, req, pres) = (p("cred.json"), p("req.json"), p("pres.json"));
    ok(&["wallet", "issue", "--dir", &t.i, "--to", &t.holder, "--type", "https://example.org/IdentityCredential",
        "--claims", r#"{"name":"Olena","nationality":"UA"}"#, "--format", format, "--out", &cred]);
    ok(&["wallet", "accept", "--dir", &t.h, &cred]);
    ok(&["wallet", "trust", "--dir", &t.v, &t.issuer]);
    let claims = if format == "sd-jwt" { "nationality" } else { "name,nationality" };
    ok(&["wallet", "allow", "--dir", &t.h, &t.verifier, claims]);
    ok(&["wallet", "request", "--dir", &t.v, "--claims", claims, "--out", &req]);
    ok(&["wallet", "present", "--dir", &t.h, &req, "--out", &pres]);
    (req, pres)
}

// @tg: verifies:: [[openspec:agent-wallet#Wallet Command#Issue and present between two wallets]] {level: e2e}
#[test]
fn issue_and_present_between_two_wallets() {
    let t = three();
    let (req, pres) = presented(&t, "sd-jwt");
    let listed = ok(&["wallet", "list", "--dir", &t.h]);
    assert!(listed.contains(&t.issuer), "{listed}");
    let out = ok(&["wallet", "verify", "--dir", &t.v, &req, &pres]);
    assert!(out.starts_with("valid"), "{out}");
    assert_eq!(field(&out, "issuer"), t.issuer);
    assert_eq!(field(&out, "disclosed"), r#"{"nationality":"UA"}"#);
}

// @tg: verifies:: [[openspec:agent-wallet#Wallet Command#Failing check named]] {level: e2e}
#[test]
fn failing_check_named() {
    let t = three();
    let (req, pres) = presented(&t, "di-jcs");
    let text = std::fs::read_to_string(&pres).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    // change one character of the issuer's proof value inside the presented credential
    let raw = v["presentation"].as_str().unwrap().to_owned();
    let i = raw.find("\"proofValue\":\"z").unwrap() + 20;
    let mut b = raw.into_bytes();
    b[i] = if b[i] == b'A' { b'B' } else { b'A' };
    v["presentation"] = serde_json::Value::String(String::from_utf8(b).unwrap());
    std::fs::write(&pres, v.to_string()).unwrap();
    let out = vt(&["wallet", "verify", "--dir", &t.v, &req, &pres]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("signature check failed"), "{err}");
}

// @tg: verifies:: [[openspec:credential-exchange#Role Charts#Role charts validate and certify]] {level: integration}
#[test]
fn role_charts_validate_and_certify() {
    let tmp = tempfile::tempdir().unwrap();
    let certifiable = std::env::var_os("VT_CERTIFY").is_some()
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean/.lake/build/bin/vt-certify").is_file();
    for (name, xml) in vertep_wallet::exchange::charts::all() {
        let path = tmp.path().join(format!("{name}.scxml"));
        std::fs::write(&path, xml).unwrap();
        let p = path.display().to_string();
        let out = ok(&["validate", &p]);
        assert!(out.contains("ok"), "{name}: {out}");
        if certifiable {
            let cert = ok(&["certify", &p]);
            assert!(cert.contains("certifiedBy"), "{name}: {cert}");
        } else if std::env::var("VERTEP_WALLET_LEAN").as_deref() == Ok("1") {
            panic!("vt-certify is not built");
        }
    }
}

#[test]
fn conformance_command_reports_both_protocols() {
    let out = ok(&["wallet", "conformance", "--seeds", "1"]);
    assert!(out.contains("vce: 3 histories") && out.contains("oid4vc: 3 histories"), "{out}");
}
