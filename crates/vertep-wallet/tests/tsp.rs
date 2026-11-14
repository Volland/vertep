//! The Vertep Credential Exchange over TSP: direct relationships, and routed through a
//! mediator that sees only sealed bytes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vertep_federation::tsp::{
    TspError, TspIdentity, TspSession, forward_routed, vid_from_document,
};
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, IdentityConfig, InMemoryKeyManager,
    KeyManager, ResolverRegistry, RuntimeClock, Service, ServiceEndpoint,
};
use vertep_runtime::VirtualClock;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::credential::ResolverKeys;
use vertep_wallet::exchange::{Exchanges, Offer};
use vertep_wallet::protocol::tsp::{Deliver, ResolveVid, TspWire};
use vertep_wallet::store::{MemoryRecords, SealedStore};
use vertep_wallet::vocab::{self, PROTOCOL_VCE, Role};
use vertep_wallet::{Wallet, WalletConfig, WalletRegistry};

const T0: i64 = 1_794_355_200_000;

type Mail = Arc<Mutex<VecDeque<(String, Vec<u8>)>>>;

struct Party {
    did: String,
    engine: Exchanges,
    wire: Arc<TspWire>,
}

struct Mediator {
    me: TspIdentity,
    did: String,
    seen: Vec<Vec<u8>>,
}

fn registry() -> Arc<ResolverRegistry> {
    Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)))
}

fn resolve_vid(reg: Arc<ResolverRegistry>) -> ResolveVid {
    Arc::new(move |id: &str| {
        let did = Did::parse(id).map_err(|e| TspError::NoVid(e.to_string()))?;
        let doc = reg
            .resolve(&did)
            .map_err(|e| TspError::NoVid(e.to_string()))?
            .document;
        vid_from_document(&doc)
    })
}

fn identity(km: &Arc<InMemoryKeyManager>, host: &str) -> (ActorIdentity, TspIdentity) {
    let config = IdentityConfig::new(DidMethodChoice::Peer {
        services: vec![Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint::uri(format!("https://{host}/tsp"))],
        }],
    });
    let id = ActorIdentity::provision(km.as_ref(), &config).unwrap();
    let me = TspIdentity::new(
        &id,
        km.clone() as Arc<dyn KeyManager>,
        format!("https://{host}/tsp").parse().unwrap(),
    )
    .unwrap();
    (id, me)
}

fn party(host: &str, mail: &Mail, clock: &VirtualClock, policy: RulePolicy) -> Party {
    let km = Arc::new(InMemoryKeyManager::new());
    let (id, me) = identity(&km, host);
    let reg = registry();
    let registry = Arc::new(WalletRegistry::with_builtins());
    registry.register_store(
        vocab::STORE_SEALED,
        Arc::new(
            SealedStore::open(
                km.as_ref(),
                &id.agreement.key_name,
                Arc::new(MemoryRecords::new()),
            )
            .unwrap(),
        ),
    );
    registry.register_consent(vocab::CONSENT_RULES, Arc::new(policy));
    let mut config = WalletConfig::from_identity(&id);
    config.status = None;
    let wallet = Wallet::new(
        config,
        km.clone(),
        registry,
        Arc::new(ResolverKeys(reg.clone())),
        Arc::new(RuntimeClock(Arc::new(clock.clone()))),
    );
    let m = mail.clone();
    let deliver: Deliver = Arc::new(move |to: &str, sealed| {
        m.lock().unwrap().push_back((to.to_owned(), sealed.bytes));
        Ok(())
    });
    let wire = Arc::new(TspWire::new(
        Arc::new(TspSession::new(me, true)),
        resolve_vid(reg),
        deliver,
    ));
    let engine = Exchanges::builder(Arc::new(clock.clone()))
        .create(wallet, wire.clone())
        .unwrap();
    Party {
        did: id.did,
        engine,
        wire,
    }
}

/// Delivers every queued message: to a party (which opens it) or to the mediator (which
/// forwards it).
fn run(mail: &Mail, parties: &mut [&mut Party], mediator: Option<&mut Mediator>) {
    let mut mediator = mediator;
    for _ in 0..1000 {
        let Some((to, mut bytes)) = mail.lock().unwrap().pop_front() else {
            return;
        };
        if let Some(m) = mediator.as_deref_mut()
            && m.did == to
        {
            m.seen.push(bytes.clone());
            let hop = forward_routed(&m.me, &mut bytes, registry().as_ref()).unwrap();
            mail.lock().unwrap().push_back((hop.next, hop.bytes));
            continue;
        }
        let p = parties.iter_mut().find(|p| p.did == to).expect("addressee");
        if let Some(o) = p.wire.open(&mut bytes).unwrap() {
            p.engine.receive(&o.protocol, &o.msg, &o.bytes).unwrap();
        }
    }
    panic!("the mail never settles");
}

fn offer() -> Offer {
    Offer {
        format: vocab::FORMAT_SD_JWT.into(),
        types: vec!["https://example.org/IdentityCredential".into()],
        claims: serde_json::json!({"name": "Olena", "nationality": "UA"})
            .as_object()
            .cloned()
            .unwrap(),
        valid_until: None,
        status: None,
    }
}

// @tg: verifies:: [[openspec:credential-exchange#Native Exchange Over TSP#Issuance over TSP]] {level: integration}
#[test]
fn issuance_over_tsp() {
    let clock = VirtualClock::new(T0);
    let mail: Mail = Arc::default();
    let mut issuer = party("issuer.example", &mail, &clock, RulePolicy::refuse_all());
    let mut holder = party("holder.example", &mail, &clock, RulePolicy::refuse_all());
    let x = issuer
        .engine
        .offer(PROTOCOL_VCE, &holder.did, offer())
        .unwrap();
    run(&mail, &mut [&mut issuer, &mut holder], None);
    assert_eq!(
        issuer.engine.state(Role::Issuer, &x).unwrap(),
        vec!["issued"]
    );
    assert_eq!(
        holder.engine.state(Role::Holder, &x).unwrap(),
        vec!["stored"]
    );
    let held = holder.engine.wallet().list().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].issuer, issuer.did);
    // both journals record the exchange
    for (p, role) in [(&mut issuer, Role::Issuer), (&mut holder, Role::Holder)] {
        let actor = p.engine.exchange(role, &x).unwrap().actor.clone();
        assert!(p.engine.host().journal(&actor).len() >= 2);
    }
}

// @tg: verifies:: [[openspec:credential-exchange#Native Exchange Over TSP#Presentation through a mediator]] {level: integration}
#[test]
fn presentation_through_a_mediator() {
    let clock = VirtualClock::new(T0);
    let mail: Mail = Arc::default();
    let mut issuer = party("issuer.example", &mail, &clock, RulePolicy::refuse_all());
    let mut holder = party(
        "holder.example",
        &mail,
        &clock,
        RulePolicy::refuse_all().allow("*", &["nationality"]),
    );
    let mut verifier = party("verifier.example", &mail, &clock, RulePolicy::refuse_all());
    verifier.engine.wallet_mut().trust(&issuer.did);
    issuer
        .engine
        .offer(PROTOCOL_VCE, &holder.did, offer())
        .unwrap();
    run(&mail, &mut [&mut issuer, &mut holder, &mut verifier], None);
    // the holder is reachable only through its mediator, both ways
    let mkm = Arc::new(InMemoryKeyManager::new());
    let (mid, mme) = identity(&mkm, "mediator.example");
    let mut mediator = Mediator {
        me: mme,
        did: mid.did.clone(),
        seen: Vec::new(),
    };
    verifier.wire.route(&holder.did, &[&mediator.did]);
    holder.wire.route(&verifier.did, &[&mediator.did]);
    let r = verifier
        .engine
        .request(PROTOCOL_VCE, &holder.did, &["nationality"], &[], None)
        .unwrap();
    run(
        &mail,
        &mut [&mut issuer, &mut holder, &mut verifier],
        Some(&mut mediator),
    );
    let v = verifier
        .engine
        .verified(&r)
        .cloned()
        .expect("verified through the mediator");
    assert_eq!(v.disclosed["nationality"], serde_json::json!("UA"));
    assert!(
        mediator.seen.len() >= 2,
        "the request and the presentation passed the mediator"
    );
    for bytes in &mediator.seen {
        let text = String::from_utf8_lossy(bytes);
        for secret in [
            "nationality",
            "\"UA\"",
            "Olena",
            "vertep.exchange",
            "PresentationRequest",
        ] {
            assert!(!text.contains(secret), "the mediator saw {secret}");
        }
    }
}
