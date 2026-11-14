//! The plugin interfaces and the exchange vocabulary.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use serde_json::{Map, Value};
use vertep_federation::KeyManager;
use vertep_wallet::credential::{
    Credential, CredentialMeta, IssueRequest, KeyLookup, Presentation, SigningKey,
    VerifiedPresentation,
};
use vertep_wallet::format::sdjwt::SdJwtVc;
use vertep_wallet::history::{Claims, Line};
use vertep_wallet::plugin::{CredentialFormat, CredentialStore};
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{EXCHANGE_NS, PROTOCOL_VCE, STORE_SEALED};
use vertep_wallet::{Role, WalletError};

const MY_FORMAT: &str = "https://example.org/formats/my-sd-jwt";

/// An application format: SD-JWT VC under the application's own IRI.
struct MyFormat;

impl CredentialFormat for MyFormat {
    fn iri(&self) -> &str {
        MY_FORMAT
    }
    fn issue(
        &self,
        km: &dyn KeyManager,
        i: &SigningKey,
        r: &IssueRequest,
    ) -> Result<Credential, WalletError> {
        let mut c = SdJwtVc.issue(km, i, r)?;
        c.format = MY_FORMAT.into();
        Ok(c)
    }
    fn verify(&self, c: &Credential, k: &dyn KeyLookup) -> Result<CredentialMeta, WalletError> {
        let mut m = SdJwtVc.verify(c, k)?;
        m.format = MY_FORMAT.into();
        Ok(m)
    }
    fn claims(&self, c: &Credential) -> Result<Map<String, Value>, WalletError> {
        SdJwtVc.claims(c)
    }
    fn present(
        &self,
        km: &dyn KeyManager,
        c: &Credential,
        h: &SigningKey,
        a: &str,
        n: &str,
        d: &Claims,
    ) -> Result<Presentation, WalletError> {
        let mut p = SdJwtVc.present(km, c, h, a, n, d)?;
        p.format = MY_FORMAT.into();
        Ok(p)
    }
    fn verify_presentation(
        &self,
        p: &Presentation,
        k: &dyn KeyLookup,
    ) -> Result<VerifiedPresentation, WalletError> {
        let mut v = SdJwtVc.verify_presentation(p, k)?;
        v.credential.format = MY_FORMAT.into();
        Ok(v)
    }
}

// @tg: verifies:: [[openspec:agent-wallet#Pluggable Wallet Interfaces#Application registers a credential format]] {level: integration}
#[test]
fn application_registers_a_credential_format() {
    let mut w = World::new();
    let issuer = w.agent_with(None, |r| r.register_format(Arc::new(MyFormat)));
    let holder = w.agent_with(None, |r| r.register_format(Arc::new(MyFormat)));
    let mut offer = id_offer();
    offer.format = MY_FORMAT.into();
    issue(&mut w, &issuer, &holder, offer);
    let held = w.party(&holder).wallet().list().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].format, MY_FORMAT);
}

/// A store that counts writes around the sealed store.
struct Counting {
    inner: Arc<dyn CredentialStore>,
    puts: Mutex<usize>,
}

impl CredentialStore for Counting {
    fn put(&self, m: &CredentialMeta, c: &Credential) -> Result<(), WalletError> {
        *self.puts.lock().unwrap() += 1;
        self.inner.put(m, c)
    }
    fn get(&self, d: &str) -> Result<(CredentialMeta, Credential), WalletError> {
        self.inner.get(d)
    }
    fn list(&self) -> Result<Vec<CredentialMeta>, WalletError> {
        self.inner.list()
    }
    fn erase(&self, d: &str) -> Result<(), WalletError> {
        self.inner.erase(d)
    }
}

// @tg: verifies:: [[openspec:agent-wallet#Pluggable Wallet Interfaces#Built-in plugin replaced]] {level: integration}
#[test]
fn built_in_plugin_replaced() {
    let mut w = World::new();
    let issuer = w.agent();
    let counting = Arc::new(Mutex::new(None::<Arc<Counting>>));
    let slot = counting.clone();
    let holder = w.agent_with(None, move |r| {
        let c = Arc::new(Counting {
            inner: r.store(STORE_SEALED).unwrap(),
            puts: Mutex::new(0),
        });
        *slot.lock().unwrap() = Some(c.clone());
        r.register_store(STORE_SEALED, c);
    });
    issue(&mut w, &issuer, &holder, id_offer());
    let c = counting.lock().unwrap().clone().unwrap();
    assert_eq!(*c.puts.lock().unwrap(), 1);
}

// @tg: verifies:: [[openspec:agent-wallet#Pluggable Wallet Interfaces#Unknown protocol refused]] {level: integration}
#[test]
fn unknown_protocol_refused() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let err = w
        .party(&issuer)
        .offer("https://example.org/protocols/none", &holder, id_offer())
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("https://example.org/protocols/none"),
        "{err}"
    );
    assert!(w.party(&issuer).exchanges().is_empty());
    assert!(w.history().lines().is_empty());
}

// @tg: verifies:: [[openspec:credential-exchange#Exchange Vocabulary#History in the vocabulary]] {level: integration}
#[test]
fn history_in_the_vocabulary() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    issue(&mut w, &issuer, &holder, id_offer());
    allow(
        &w,
        &holder,
        vertep_wallet::consent::RulePolicy::refuse_all().allow(&verifier, &["name"]),
    );
    w.party(&verifier)
        .request(PROTOCOL_VCE, &holder, &["name"], &[], None)
        .unwrap();
    w.kit.run(Schedule::honest());
    let steps: Vec<&'static str> = w
        .history()
        .lines()
        .iter()
        .map(Line::kind)
        .filter(|k| !matches!(*k, "deliver" | "drop" | "duplicate" | "inject" | "send"))
        .collect();
    assert_eq!(
        steps,
        [
            "offer",
            "prove-possession",
            "issue",
            "store",
            "request-presentation",
            "consent",
            "present",
            "verify"
        ]
    );
    let vocab: Vec<&str> = vertep_wallet::Step::ALL
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert!(steps.iter().all(|s| vocab.contains(s)));
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:credential-exchange#Exchange Vocabulary#Unmapped message refused]] {level: integration}
#[test]
fn unmapped_message_refused() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "type": format!("{EXCHANGE_NS}Gossip"),
        "exchange": "x-1", "from": issuer, "to": holder, "body": {}
    }))
    .unwrap();
    let err = w
        .party(&holder)
        .receive(PROTOCOL_VCE, "m-1", &bytes)
        .unwrap_err();
    assert_eq!(err.check(), "unmapped");
    assert!(err.to_string().contains("Gossip"));
    assert!(w.party(&holder).exchanges().is_empty());
    assert!(w.party(&holder).wallet().list().unwrap().is_empty());
    assert!(w.party(&holder).exchange(Role::Holder, "x-1").is_none());
}
