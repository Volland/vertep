//! Credential formats: Data Integrity EdDSA (JCS and RDF canonicalisation) and SD-JWT VC,
//! against the specifications' test vectors and their own round trips.

use std::sync::Arc;

use serde_json::{Value, json};
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType};
use vertep_wallet::credential::{Credential, DidKeyOnly, IssueRequest, SigningKey};
use vertep_wallet::format::di::{DataIntegrity, rdfc};
use vertep_wallet::format::sdjwt::SdJwtVc;
use vertep_wallet::history::Claims;
use vertep_wallet::plugin::CredentialFormat;

const T0: i64 = 1_794_355_200_000;

fn keys() -> (Arc<InMemoryKeyManager>, SigningKey, SigningKey) {
    let km = Arc::new(InMemoryKeyManager::new());
    let i = km.generate(KeyType::Ed25519).unwrap();
    let h = km.generate(KeyType::Ed25519).unwrap();
    let issuer = SigningKey::did_key(km.as_ref(), &i).unwrap();
    let holder = SigningKey::did_key(km.as_ref(), &h).unwrap();
    (km, issuer, holder)
}

fn request(holder: &SigningKey) -> IssueRequest {
    IssueRequest {
        types: vec!["IdentityCredential".into()],
        claims: json!({"name": "Olena", "birthDate": "1990-05-01", "nationality": "UA"})
            .as_object()
            .cloned()
            .unwrap(),
        holder_key: holder.public,
        valid_from: T0,
        valid_until: None,
        status: None,
    }
}

fn names(n: &[&str]) -> Claims {
    n.iter().map(|s| (*s).to_owned()).collect()
}

fn tamper(raw: &str) -> String {
    raw.replace("Olena", "Oleh")
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Formats#Data Integrity round trip]] {level: unit}
#[test]
fn data_integrity_round_trip() {
    let (km, issuer, holder) = keys();
    for f in [DataIntegrity::jcs(), DataIntegrity::rdfc()] {
        let cred = f.issue(km.as_ref(), &issuer, &request(&holder)).unwrap();
        let meta = f.verify(&cred, &DidKeyOnly).unwrap();
        assert_eq!(meta.issuer, issuer.did);
        assert_eq!(meta.holder_key, holder.did_key_id());
        assert_eq!(meta.claims, names(&["birthDate", "name", "nationality"]));
        let bad = Credential {
            format: cred.format.clone(),
            raw: tamper(&cred.raw),
        };
        assert!(f.verify(&bad, &DidKeyOnly).is_err(), "{} accepts a changed claim", f.cryptosuite());
        let all = names(&["birthDate", "name", "nationality"]);
        let p = f.present(km.as_ref(), &cred, &holder, "did:example:v", "n-1", &all).unwrap();
        let v = f.verify_presentation(&p, &DidKeyOnly).unwrap();
        assert_eq!((v.audience.as_str(), v.nonce.as_str()), ("did:example:v", "n-1"));
        assert_eq!(v.signer, holder.did_key_id());
    }
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Formats#RDF canonical proof survives reserialisation]] {level: unit}
#[test]
fn rdf_canonical_proof_survives_reserialisation() {
    let (km, issuer, holder) = keys();
    let f = DataIntegrity::rdfc();
    let cred = f.issue(km.as_ref(), &issuer, &request(&holder)).unwrap();
    // reorder every object's members (serde_json keeps insertion order with preserve_order off:
    // rebuild the document with members reversed)
    fn reversed(v: &Value) -> Value {
        match v {
            Value::Object(o) => {
                let mut m = serde_json::Map::new();
                for (k, x) in o.iter().rev() {
                    m.insert(k.clone(), reversed(x));
                }
                Value::Object(m)
            }
            Value::Array(a) => Value::Array(a.iter().map(reversed).collect()),
            other => other.clone(),
        }
    }
    let doc: Value = serde_json::from_str(&cred.raw).unwrap();
    let text = serde_json::to_string_pretty(&reversed(&doc)).unwrap();
    assert_ne!(text, cred.raw);
    let again = Credential {
        format: cred.format.clone(),
        raw: text,
    };
    f.verify(&again, &DidKeyOnly).expect("the reserialised credential verifies");
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Formats#Selective disclosure]] {level: unit}
#[test]
fn selective_disclosure() {
    let (km, issuer, holder) = keys();
    let f = SdJwtVc;
    let cred = f.issue(km.as_ref(), &issuer, &request(&holder)).unwrap();
    let p = f
        .present(km.as_ref(), &cred, &holder, "did:example:v", "n-2", &names(&["nationality"]))
        .unwrap();
    for hidden in ["Olena", "1990-05-01"] {
        // neither the values nor their disclosures are in the presentation
        assert!(!p.raw.contains(hidden));
        let decoded: String = p
            .raw
            .split('~')
            .filter_map(|s| vertep_wallet::jose::unb64(s).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        assert!(!decoded.contains(hidden), "{hidden} is recoverable");
    }
    let v = f.verify_presentation(&p, &DidKeyOnly).unwrap();
    assert_eq!(v.disclosed_names(), names(&["nationality"]));
    assert_eq!(v.disclosed["nationality"], json!("UA"));
    // a changed disclosure no longer matches the issuer's digests
    let forged = p.raw.replacen('~', "~WyJzYWx0IiwibmFtZSIsIk9sZWgiXQ~", 1);
    assert!(f.verify_presentation(&vertep_wallet::Presentation { format: p.format.clone(), raw: forged }, &DidKeyOnly).is_err());
}

fn vector(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/vectors/di/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Formats#Interoperable examples]] {level: unit}
#[test]
fn specification_examples() {
    // W3C vc-di-eddsa test vectors: the signed examples verify, a tampered one does not, and
    // our RDF canonical form of the unsigned example equals the published one
    for (file, f) in [("signedJCS.json", DataIntegrity::jcs()), ("signedDataInt.json", DataIntegrity::rdfc())] {
        let doc: Value = serde_json::from_str(&vector(file)).unwrap();
        f.verify_proof(&doc, "assertionMethod", &DidKeyOnly)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut bad = doc.clone();
        bad["credentialSubject"]["alumniOf"] = json!("Another School");
        assert!(f.verify_proof(&bad, "assertionMethod", &DidKeyOnly).is_err(), "{file}");
    }
    let unsigned: Value = serde_json::from_str(&vector("unsigned.json")).unwrap();
    let canon = String::from_utf8(rdfc(&unsigned).unwrap()).unwrap();
    assert_eq!(canon, vector("canonDocDataInt.txt"));
    // a document needing a context that is not bundled is refused
    let remote = json!({"@context": ["https://example.org/unknown/v1"], "type": "X"});
    assert!(rdfc(&remote).is_err());
}

#[test]
fn sd_jwt_is_bound_to_the_holder() {
    let (km, issuer, holder) = keys();
    let other = SigningKey::did_key(km.as_ref(), &km.generate(KeyType::Ed25519).unwrap()).unwrap();
    let f = SdJwtVc;
    let cred = f.issue(km.as_ref(), &issuer, &request(&holder)).unwrap();
    let p = f
        .present(km.as_ref(), &cred, &other, "did:example:v", "n", &names(&["name"]))
        .unwrap();
    assert_eq!(
        f.verify_presentation(&p, &DidKeyOnly).unwrap_err().check(),
        "holder-binding"
    );
}
