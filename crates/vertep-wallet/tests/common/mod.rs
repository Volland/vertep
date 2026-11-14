//! Shared helpers for the wallet tests: agents with did:key identities, sealed stores, local
//! status lists and the conformance kit's network.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vertep_agent::decider::Decider;
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType, RuntimeClock};
use vertep_runtime::VirtualClock;
use vertep_wallet::credential::{DidKeyOnly, KeyLookup};
use vertep_wallet::exchange::{Exchanges, Offer};
use vertep_wallet::history::History;
use vertep_wallet::plugin::WalletRegistry;
use vertep_wallet::status::LocalStatus;
use vertep_wallet::store::{MemoryRecords, SealedStore};
use vertep_wallet::testkit::Kit;
use vertep_wallet::vocab::{self, PROTOCOL_VCE};
use vertep_wallet::{Wallet, WalletConfig};

/// 2026-11-11T00:00:00Z.
pub const T0: i64 = 1_794_355_200_000;
/// One day in milliseconds.
pub const DAY: i64 = 86_400_000;

/// One agent's parts, kept so a test can restart it.
pub struct Agent {
    pub did: String,
    pub km: Arc<InMemoryKeyManager>,
    pub key: String,
    pub records: MemoryRecords,
    pub journal: Arc<dyn vertep_agent::journal::AgentJournal>,
    pub registry: Arc<WalletRegistry>,
}

/// The world: a clock, shared status lists and the kit.
pub struct World {
    pub clock: VirtualClock,
    pub status: LocalStatus,
    pub kit: Kit,
    pub agents: Vec<Agent>,
}

impl World {
    pub fn new() -> Self {
        World {
            clock: VirtualClock::new(T0),
            status: LocalStatus::new(Arc::new(DidKeyOnly)),
            kit: Kit::new(),
            agents: Vec::new(),
        }
    }

    pub fn keys(&self) -> Arc<dyn KeyLookup> {
        Arc::new(DidKeyOnly)
    }

    /// A new agent with a sealed in-memory store; its engine is added to the kit.
    pub fn agent(&mut self) -> String {
        self.agent_with(None, |_| {})
    }

    /// A new agent whose consent decider is `consent` (default: the policy decider) and whose
    /// registry `tweak` adjusts.
    pub fn agent_with(
        &mut self,
        consent: Option<Arc<dyn Decider>>,
        tweak: impl FnOnce(&WalletRegistry),
    ) -> String {
        let km = Arc::new(InMemoryKeyManager::new());
        let key = km.generate(KeyType::Ed25519).expect("key");
        let config = WalletConfig::did_key(km.as_ref(), &key).expect("config");
        let did = config.did.clone();
        let records = MemoryRecords::new();
        let registry = Arc::new(WalletRegistry::with_builtins());
        let store = SealedStore::open(km.as_ref(), &key, Arc::new(records.clone())).expect("store");
        registry.register_store(vocab::STORE_SEALED, Arc::new(store));
        registry.register_status(vocab::STATUS_BITSTRING, Arc::new(self.status.clone()));
        tweak(&registry);
        let journal: Arc<dyn vertep_agent::journal::AgentJournal> =
            Arc::new(vertep_agent::MemoryJournal::default());
        let agent = Agent {
            did: did.clone(),
            km,
            key,
            records,
            journal,
            registry,
        };
        let e = self.engine(&agent, consent, true);
        self.kit.add(e);
        self.agents.push(agent);
        did
    }

    /// An engine for `agent` (`create` or `open`).
    pub fn engine(&self, a: &Agent, consent: Option<Arc<dyn Decider>>, create: bool) -> Exchanges {
        let km: Arc<dyn KeyManager> = a.km.clone();
        let config = WalletConfig::did_key(km.as_ref(), &a.key).expect("config");
        let wallet = Wallet::new(
            config,
            km,
            a.registry.clone(),
            self.keys(),
            Arc::new(RuntimeClock(Arc::new(self.clock.clone()))),
        );
        let mut b = Exchanges::builder(Arc::new(self.clock.clone()))
            .journal(a.journal.clone())
            .history(self.kit.history());
        if let Some(c) = consent {
            b = b.consent(c);
        }
        if create {
            b.create(wallet, self.kit.wire()).expect("wallet created")
        } else {
            b.open(wallet, self.kit.wire()).expect("wallet opened")
        }
    }

    pub fn agent_parts(&self, did: &str) -> &Agent {
        self.agents.iter().find(|a| a.did == did).expect("agent")
    }

    pub fn party(&mut self, did: &str) -> &mut Exchanges {
        self.kit.party(did)
    }

    pub fn history(&self) -> History {
        self.kit.history()
    }
}

/// Claims as a JSON map.
pub fn claims(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("an object")
}

/// An SD-JWT VC offer of an identity credential with `name`, `birthDate` and `nationality`.
pub fn id_offer() -> Offer {
    Offer {
        format: vocab::FORMAT_SD_JWT.to_owned(),
        types: vec!["https://example.org/IdentityCredential".to_owned()],
        claims: claims(json!({"name": "Olena", "birthDate": "1990-05-01", "nationality": "UA"})),
        valid_until: Some(T0 + 365 * DAY),
        status: None,
    }
}

/// A Data Integrity offer with one claim.
pub fn di_offer(format: &str) -> Offer {
    Offer {
        format: format.to_owned(),
        types: vec!["https://example.org/MembershipCredential".to_owned()],
        claims: claims(json!({"memberOf": "Vertep Guild"})),
        valid_until: Some(T0 + 365 * DAY),
        status: None,
    }
}

/// Issues `offer` from `issuer` to `holder` over the native protocol, delivered honestly;
/// returns the issuer's exchange id.
pub fn issue(w: &mut World, issuer: &str, holder: &str, offer: Offer) -> String {
    let x = w.party(issuer).offer(PROTOCOL_VCE, holder, offer).expect("offer");
    w.kit.run(vertep_wallet::testkit::Schedule::honest());
    x
}

/// Sets the consent policy of `holder` (a rule policy).
pub fn allow(w: &World, holder: &str, policy: vertep_wallet::consent::RulePolicy) {
    w.agent_parts(holder)
        .registry
        .register_consent(vocab::CONSENT_RULES, Arc::new(policy));
}

/// The kit's history checked by the Lean checker when it is built (required with
/// `VERTEP_WALLET_LEAN=1`).
pub fn lean_accepts(w: &World) {
    match w.kit.check() {
        Some(Ok(_)) => {}
        Some(Err(report)) => panic!("the Lean checker rejects the history:\n{report}\n{}", w.kit.jsonl()),
        None if std::env::var("VERTEP_WALLET_LEAN").as_deref() == Ok("1") => {
            panic!("the Lean checker vertep-wallet is not built")
        }
        None => eprintln!("skipped: the Lean checker vertep-wallet is not built"),
    }
}
