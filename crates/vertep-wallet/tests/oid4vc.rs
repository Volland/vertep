//! OpenID4VCI and OpenID4VP over real HTTP (an axum server on a loopback port and a `ureq`
//! holder), and the specifications' examples.

use std::net::TcpListener;
use std::sync::Arc;

use serde_json::{Value, json};
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType, RuntimeClock};
use vertep_runtime::VirtualClock;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::credential::DidKeyOnly;
use vertep_wallet::exchange::{Exchanges, Offer};
use vertep_wallet::protocol::oid4vc::model::*;
use vertep_wallet::protocol::oid4vc_http::{
    Answered, CaptureWire, ClientId, Configuration, EngineHandle, Oid4vcHolder, Oid4vcServer,
    ServerConfig,
};
use vertep_wallet::store::{MemoryRecords, SealedStore};
use vertep_wallet::vocab::{self, Role};
use vertep_wallet::{Wallet, WalletConfig, WalletRegistry};

const T0: i64 = 1_794_355_200_000;
const VCT: &str = "https://example.org/IdentityCredential";

fn engine(clock: &VirtualClock, capture: &CaptureWire, aliases: Vec<String>, trust: Vec<String>, policy: RulePolicy) -> Exchanges {
    let km = Arc::new(InMemoryKeyManager::new());
    let key = km.generate(KeyType::Ed25519).unwrap();
    let registry = Arc::new(WalletRegistry::with_builtins());
    registry.register_store(
        vocab::STORE_SEALED,
        Arc::new(SealedStore::open(km.as_ref(), &key, Arc::new(MemoryRecords::new())).unwrap()),
    );
    registry.register_consent(vocab::CONSENT_RULES, Arc::new(policy));
    let mut config = WalletConfig::did_key(km.as_ref(), &key).unwrap();
    config.aliases = aliases;
    config.trusted_issuers = trust;
    config.status = None;
    let km: Arc<dyn KeyManager> = km;
    let wallet = Wallet::new(config, km, registry, Arc::new(DidKeyOnly), Arc::new(RuntimeClock(Arc::new(clock.clone()))));
    Exchanges::builder(Arc::new(clock.clone()))
        .create(wallet, Arc::new(capture.clone()))
        .unwrap()
}

/// Binds a loopback port and returns its base URL with the listener.
fn bind() -> (String, TcpListener) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    (format!("http://{}", l.local_addr().unwrap()), l)
}

fn serve(router: axum::Router, listener: TcpListener) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let l = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(l, router).await.unwrap();
        });
    });
}

/// An issuer server (also a verifier, with `client`), trusting `trust`.
fn server(clock: &VirtualClock, trust: Vec<String>, client: ClientId) -> (Oid4vcServer, String) {
    let (base, listener) = bind();
    let capture = CaptureWire::default();
    let (c, cap, b) = (clock.clone(), capture.clone(), base.clone());
    let handle = EngineHandle::spawn(move || engine(&c, &cap, vec![b], trust, RulePolicy::refuse_all()));
    let config = ServerConfig {
        base_url: base.clone(),
        configurations: vec![
            Configuration { id: "IdentityCredential".into(), format: vocab::FORMAT_SD_JWT.into(), vct: VCT.into() },
            Configuration { id: "MembershipCredential".into(), format: vocab::FORMAT_DI_JCS.into(), vct: "https://example.org/MembershipCredential".into() },
        ],
        client_id: client,
    };
    let s = Oid4vcServer::new(handle, capture, config).unwrap();
    serve(s.router(), listener);
    (s, base)
}

fn holder(clock: &VirtualClock, policy: RulePolicy) -> (Exchanges, Oid4vcHolder) {
    let capture = CaptureWire::default();
    let e = engine(clock, &capture, vec![], vec![], policy);
    (e, Oid4vcHolder::new(capture))
}

fn offer() -> Offer {
    Offer {
        format: vocab::FORMAT_SD_JWT.into(),
        types: vec![VCT.into()],
        claims: json!({"name": "Olena", "birthDate": "1990-05-01", "nationality": "UA"}).as_object().cloned().unwrap(),
        valid_until: None,
        status: None,
    }
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VCI Pre-Authorised Issuance#Holder obtains a credential from an offer]] {level: e2e}
#[test]
fn holder_obtains_a_credential_from_an_offer() {
    let clock = VirtualClock::new(T0);
    let (issuer, base) = server(&clock, vec![], ClientId::Did);
    let uri = issuer.offer(offer(), Some("4711")).unwrap();
    assert!(uri.starts_with("openid-credential-offer://?credential_offer="));
    let (mut wallet, client) = holder(&clock, RulePolicy::refuse_all());
    let meta = client.accept_offer(&mut wallet, &uri, Some("4711")).unwrap();
    assert_eq!(meta.types, vec![VCT.to_owned()]);
    assert_eq!(wallet.wallet().list().unwrap().len(), 1);
    assert!(meta.claims.contains("nationality"));
    let _ = base;
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VCI Pre-Authorised Issuance#Wrong transaction code refused]] {level: e2e}
#[test]
fn wrong_transaction_code_refused() {
    let clock = VirtualClock::new(T0);
    let (issuer, _) = server(&clock, vec![], ClientId::Did);
    let uri = issuer.offer(offer(), Some("4711")).unwrap();
    let (mut wallet, client) = holder(&clock, RulePolicy::refuse_all());
    let err = client.accept_offer(&mut wallet, &uri, Some("0000")).unwrap_err();
    assert!(err.to_string().contains("invalid_grant"), "{err}");
    assert!(wallet.wallet().list().unwrap().is_empty());
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VCI Pre-Authorised Issuance#Proof without a fresh nonce refused]] {level: e2e}
#[test]
fn proof_without_a_fresh_nonce_refused() {
    let clock = VirtualClock::new(T0);
    let (issuer, base) = server(&clock, vec![], ClientId::Did);
    let uri = issuer.offer(offer(), None).unwrap();
    let code = {
        let q = uri.split_once("credential_offer=").unwrap().1;
        let o: Value = serde_json::from_str(&percent_encoding::percent_decode_str(q).decode_utf8_lossy()).unwrap();
        o.pointer("/grants/urn:ietf:params:oauth:grant-type:pre-authorized_code/pre-authorized_code").and_then(Value::as_str).unwrap().to_owned()
    };
    let token: Value = ureq::post(&format!("{base}/token"))
        .send_form(&[("grant_type", "urn:ietf:params:oauth:grant-type:pre-authorized_code"), ("pre-authorized_code", &code)])
        .unwrap()
        .into_json()
        .unwrap();
    let access = token["access_token"].as_str().unwrap().to_owned();
    let (wallet, _) = holder(&clock, RulePolicy::refuse_all());
    let post = |nonce: &str| {
        let proof = wallet.wallet().prove_possession(&base, nonce).unwrap();
        ureq::post(&format!("{base}/credential"))
            .set("Authorization", &format!("Bearer {access}"))
            .send_json(json!({"credential_configuration_id": "IdentityCredential", "proofs": {"jwt": [proof]}}))
    };
    match post("a-nonce-the-issuer-never-issued") {
        Err(ureq::Error::Status(400, r)) => assert_eq!(r.into_json::<Value>().unwrap()["error"], "invalid_nonce"),
        other => panic!("expected invalid_nonce, got {other:?}"),
    }
    // the real nonce issues once; reusing it is refused
    let n = token["c_nonce"].as_str().unwrap().to_owned();
    assert!(post(&n).is_ok());
    match post(&n) {
        Err(ureq::Error::Status(400, r)) => assert_eq!(r.into_json::<Value>().unwrap()["error"], "invalid_nonce"),
        other => panic!("expected invalid_nonce for a used nonce, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:oid4vc#Credential Issuer Metadata#Metadata lists configurations]] {level: e2e}
#[test]
fn metadata_lists_configurations() {
    let clock = VirtualClock::new(T0);
    let (_, base) = server(&clock, vec![], ClientId::Did);
    let meta: Value = ureq::get(&format!("{base}/.well-known/openid-credential-issuer")).call().unwrap().into_json().unwrap();
    let typed: IssuerMetadata = serde_json::from_value(meta.clone()).unwrap();
    assert_eq!(typed.credential_issuer.as_deref(), Some(base.as_str()));
    assert_eq!(meta["credential_endpoint"], json!(format!("{base}/credential")));
    assert_eq!(meta["nonce_endpoint"], json!(format!("{base}/nonce")));
    let configs = typed.credential_configurations_supported.unwrap();
    assert_eq!(configs["IdentityCredential"]["format"], "dc+sd-jwt");
    assert_eq!(configs["IdentityCredential"]["vct"], VCT);
    assert_eq!(configs["MembershipCredential"]["format"], "ldp_vc");
    assert_eq!(configs["IdentityCredential"]["proof_types_supported"]["jwt"]["proof_signing_alg_values_supported"], json!(["EdDSA"]));
    let as_meta: Value = ureq::get(&format!("{base}/.well-known/oauth-authorization-server")).call().unwrap().into_json().unwrap();
    assert_eq!(as_meta["token_endpoint"], json!(format!("{base}/token")));
}

/// An issued credential in a holder wallet, and a verifier server trusting the issuer.
fn holder_with_credential(clock: &VirtualClock, policy: RulePolicy, client: ClientId) -> (Exchanges, Oid4vcHolder, Oid4vcServer) {
    let (issuer, _) = server(clock, vec![], ClientId::Did);
    let uri = issuer.offer(offer(), None).unwrap();
    let (mut wallet, h) = holder(clock, policy);
    let meta = h.accept_offer(&mut wallet, &uri, None).unwrap();
    let (verifier, _) = server(clock, vec![meta.issuer], client);
    (wallet, h, verifier)
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VP Verifier And Holder#Holder answers a DCQL query]] {level: e2e}
#[test]
fn holder_answers_a_dcql_query() {
    let clock = VirtualClock::new(T0);
    let (mut wallet, h, verifier) = holder_with_credential(&clock, RulePolicy::refuse_all().allow("*", &["nationality"]), ClientId::Did);
    let (x, uri) = verifier.request(&["nationality"], &[VCT], None).unwrap();
    assert!(percent_encoding::percent_decode_str(&uri).decode_utf8_lossy().starts_with("openid4vp://?client_id=decentralized_identifier:did:key:"));
    assert_eq!(h.respond(&mut wallet, &uri).unwrap(), Answered::Presented { accepted: true });
    let v = verifier.verified(&x).unwrap().expect("accepted");
    assert_eq!(v.disclosed.keys().collect::<Vec<_>>(), vec!["nationality"]);
    assert_eq!(verifier.state(Role::Verifier, &x).unwrap(), vec!["accepted"]);
}

#[test]
fn redirect_uri_client_answered_by_value() {
    let clock = VirtualClock::new(T0);
    let (mut wallet, h, verifier) = holder_with_credential(&clock, RulePolicy::refuse_all().allow("*", &["birthDate"]), ClientId::RedirectUri);
    let (x, uri) = verifier.request(&["birthDate"], &[], Some("age check")).unwrap();
    assert!(percent_encoding::percent_decode_str(&uri).decode_utf8_lossy().contains("client_id=redirect_uri:http"));
    assert_eq!(h.respond(&mut wallet, &uri).unwrap(), Answered::Presented { accepted: true });
    assert!(verifier.verified(&x).unwrap().is_some());
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VP Verifier And Holder#Unsatisfiable query reported]] {level: e2e}
#[test]
fn unsatisfiable_query_reported() {
    let clock = VirtualClock::new(T0);
    let (mut wallet, h, verifier) = holder_with_credential(&clock, RulePolicy::refuse_all().allow("*", &["nationality"]), ClientId::Did);
    let (x, uri) = verifier.request(&["nationality"], &["https://example.org/DriversLicence"], None).unwrap();
    assert_eq!(h.respond(&mut wallet, &uri).unwrap(), Answered::Refused("no-match".into()));
    assert!(verifier.verified(&x).unwrap().is_none());
    assert_eq!(verifier.state(Role::Verifier, &x).unwrap(), vec!["refused"]);
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VP Verifier And Holder#Response for another client refused]] {level: e2e}
#[test]
fn response_for_another_client_refused() {
    let clock = VirtualClock::new(T0);
    let (wallet, _h, verifier) = holder_with_credential(&clock, RulePolicy::refuse_all(), ClientId::Did);
    let (x, uri) = verifier.request(&["nationality"], &[], None).unwrap();
    let jwt = uri.split_once("request=").unwrap().1;
    let jwt = percent_encoding::percent_decode_str(jwt).decode_utf8_lossy().into_owned();
    let params = vertep_wallet::jose::Jws::parse(&jwt).unwrap().payload;
    let digest = wallet.wallet().list().unwrap()[0].digest.clone();
    let names: vertep_wallet::history::Claims = ["nationality".to_owned()].into();
    // the presentation names another client as its audience
    let (_, p) = wallet
        .wallet()
        .present(&digest, &names, "decentralized_identifier:did:key:z6MkOtherVerifier", params["nonce"].as_str().unwrap())
        .unwrap();
    let token = json!({"cred1": [p.raw]}).to_string();
    let r = ureq::post(params["response_uri"].as_str().unwrap()).send_form(&[("vp_token", token.as_str()), ("state", x.as_str())]);
    assert!(matches!(r, Err(ureq::Error::Status(400, _))), "{r:?}");
    assert!(verifier.verified(&x).unwrap().is_none());
    assert_eq!(verifier.state(Role::Verifier, &x).unwrap(), vec!["requested"]);
}

fn fixtures(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/vectors/oid4vc/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
}

fn roundtrip<T: serde::de::DeserializeOwned + serde::Serialize>(v: &Value, what: &str) {
    let t: T = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{what}: {e}\n{v}"));
    assert_eq!(&serde_json::to_value(&t).unwrap(), v, "{what} re-serialises differently");
}

// @tg: verifies:: [[openspec:oid4vc#OpenID4VC Interoperability#Specification examples parse]] {level: unit}
#[test]
fn specification_examples_parse() {
    let vci = fixtures("vci-examples.json");
    let each = |k: &str| vci[k].as_array().unwrap().clone();
    for v in each("credential_offers") { roundtrip::<CredentialOffer>(&v, "credential offer"); }
    for v in each("issuer_metadata") { roundtrip::<IssuerMetadata>(&v, "issuer metadata"); }
    for v in each("token_responses") { roundtrip::<TokenResponse>(&v, "token response"); }
    for v in each("nonce_responses") { roundtrip::<NonceResponse>(&v, "nonce response"); }
    for v in each("credential_responses") { roundtrip::<CredentialResponse>(&v, "credential response"); }
    for v in each("error_responses") { roundtrip::<ErrorResponse>(&v, "error response"); }
    let vp = fixtures("vp-examples.json");
    let each = |k: &str| vp[k].as_array().unwrap().clone();
    for v in each("authorization_requests") { roundtrip::<AuthorizationRequest>(&v, "authorization request"); }
    let queries = each("dcql_queries");
    assert!(queries.len() >= 5);
    for v in &queries {
        roundtrip::<DcqlQuery>(v, "DCQL query");
        assert!(!vertep_wallet::protocol::oid4vc::read_dcql(v).unwrap().is_empty());
    }
    for v in each("authorization_responses") { roundtrip::<AuthorizationResponse>(&v, "authorization response"); }
    for v in each("error_responses") { roundtrip::<ErrorResponse>(&v, "error response"); }
    // what we emit has the members the specifications require
    let req = vertep_wallet::protocol::oid4vc::authorization_request(
        "decentralized_identifier:did:key:z6Mk", "n", &[VCT.into()], &["nationality".to_owned()].into(), "https://v.example/response", "s",
    );
    let typed: AuthorizationRequest = serde_json::from_value(req).unwrap();
    assert_eq!(typed.response_type, "vp_token");
    assert_eq!(typed.dcql_query.unwrap().credentials[0].format, "dc+sd-jwt");
}
