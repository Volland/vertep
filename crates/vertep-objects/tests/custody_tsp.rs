//! Custody over TSP: a writer replicates to a host directly and to a keeper that is reachable
//! only through a store-and-forward TSP intermediary it picks up from.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use vertep_federation::did::{Service, ServiceEndpoint};
use vertep_federation::testkit::TestIntermediary;
use vertep_federation::tsp::{TspIdentity, TspSession, vid_from_document};
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, IdentityConfig, InMemoryKeyManager,
    KeyManager, LocalNetwork, RelayEndpoint, RelayNetwork, ResolverRegistry,
};
use vertep_objects::DurableObjectId;
use vertep_objects::KeyManagerSigner;
use vertep_objects::custody::tsp_link::{
    ReplyInbox, ResolveVid, TspCustodyEndpoint, TspCustodyTransport, pump_mailbox,
};
use vertep_objects::custody::{
    Custodian, CustodyConfig, CustodyHost, Replica, ResolverKeyDirectory,
};
use vertep_objects::testkit::turn;
use vertep_store::{JournalStore, MemoryStore, Position};

struct Party {
    did: String,
    keys: Arc<InMemoryKeyManager>,
    identity: ActorIdentity,
    session: Arc<TspSession>,
}

fn party(uri: &str) -> Party {
    let keys = Arc::new(InMemoryKeyManager::new());
    let identity = ActorIdentity::provision(
        keys.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![Service {
                id: "#tsp".into(),
                types: vec!["TSPTransport".into()],
                endpoints: vec![ServiceEndpoint::uri(uri)],
            }],
        }),
    )
    .unwrap();
    let me = TspIdentity::new(
        &identity,
        keys.clone() as Arc<dyn KeyManager>,
        url::Url::parse(uri).unwrap(),
    )
    .unwrap();
    Party {
        did: identity.did.clone(),
        keys,
        identity,
        session: Arc::new(TspSession::new(me, false)),
    }
}

// @tg: verifies:: [[openspec:object-custody#Journal Replication Over TSP#Custodian behind a mediator]] {level: integration}
#[test]
fn custodian_behind_a_mediator() {
    let net = LocalNetwork::new();
    let resolver: Arc<dyn DidResolver> =
        Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let resolve: ResolveVid = {
        let r = resolver.clone();
        Arc::new(move |id: &str| {
            let did = Did::parse(id).map_err(|e| vertep_federation::tsp::TspError::Open(e.0))?;
            let doc = r
                .resolve(&did)
                .map_err(|e| vertep_federation::tsp::TspError::Open(e.to_string()))?
                .document;
            vid_from_document(&doc)
        })
    };
    let mediator_uri = "https://mediator.example.net/tsp";
    let mediator = TestIntermediary::new(
        Arc::new(InMemoryKeyManager::new()),
        mediator_uri,
        net.clone() as Arc<dyn RelayNetwork>,
        resolver.clone(),
        true,
    )
    .unwrap();
    net.serve(mediator_uri, mediator.clone() as Arc<dyn RelayEndpoint>);

    let a = party("https://a.example.org/tsp");
    let b = party("https://b.example.org/tsp");
    let k = party(mediator_uri); // the keeper's TSP endpoint is its mediator
    let object = DurableObjectId::new("did:webvh:QmScid:example.org:objects:quote-17").unwrap();
    let directory = Arc::new(ResolverKeyDirectory::new(resolver.clone()));
    let config = CustodyConfig::new(
        vec![
            Custodian::host(&a.did),
            Custodian::host(&b.did),
            Custodian::keeper(&k.did),
        ],
        2,
    )
    .unwrap();
    let replica = |p: &Party| {
        Arc::new(
            Replica::open(
                object.iri(),
                Arc::new(KeyManagerSigner::new(
                    &p.did,
                    p.keys.clone() as Arc<dyn KeyManager>,
                    p.identity.signing.key_name.clone(),
                )),
                Arc::new(MemoryStore::new()) as Arc<dyn JournalStore>,
                directory.clone(),
                config.clone(),
            )
            .unwrap(),
        )
    };
    let (ra, rb, rk) = (replica(&a), replica(&b), replica(&k));

    // B answers directly at its endpoint; the keeper picks up from the mediator.
    net.serve(
        "https://b.example.org/tsp",
        Arc::new(TspCustodyEndpoint::new(
            b.session.clone(),
            rb.clone(),
            resolve.clone(),
        )),
    );
    let keeper = Arc::new(TspCustodyEndpoint::new(
        k.session.clone(),
        rk.clone(),
        resolve.clone(),
    ));
    let inbox = ReplyInbox::new(a.session.clone(), resolve.clone());
    net.serve("https://a.example.org/tsp", inbox.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let (stop, keeper, mediator, net, kd) = (
            stop.clone(),
            keeper.clone(),
            mediator.clone(),
            net.clone(),
            k.did.clone(),
        );
        thread::spawn(move || {
            let mut answered = 0;
            while !stop.load(Ordering::SeqCst) {
                answered += pump_mailbox(&keeper, mediator.as_ref(), &kd, net.as_ref()).unwrap();
                thread::sleep(Duration::from_millis(2));
            }
            answered
        })
    };

    let mediator_vid = {
        let doc = resolver
            .resolve(&Did::parse(mediator.did()).unwrap())
            .unwrap()
            .document;
        vid_from_document(&doc).unwrap()
    };
    let host = CustodyHost::new(
        object.clone(),
        Arc::new(KeyManagerSigner::new(
            &a.did,
            a.keys.clone() as Arc<dyn KeyManager>,
            a.identity.signing.key_name.clone(),
        )),
        a.keys.clone() as Arc<dyn KeyManager>,
        a.identity.agreement.key_name.clone(),
        ra.clone(),
        directory.clone(),
    )
    .with_transport(Arc::new(TspCustodyTransport::direct(
        &b.did,
        a.session.clone(),
        net.clone() as Arc<dyn RelayNetwork>,
        resolve.clone(),
    )))
    .with_transport(Arc::new(TspCustodyTransport::routed(
        &k.did,
        a.session.clone(),
        net.clone() as Arc<dyn RelayNetwork>,
        resolve.clone(),
        vec![mediator_vid],
        inbox,
        Duration::from_secs(5),
    )));

    // B is down: the quorum of two needs the keeper behind the mediator.
    net.set_down("https://b.example.org/tsp", true);
    host.elect(1).unwrap();
    let obj = object.iri();
    for i in 1..=3 {
        host.append_turn(Position::new(i - 1), &[turn(&obj, i, "over tsp")])
            .unwrap();
    }
    assert_eq!(rk.head(), Position::new(3), "the keeper stored every entry");
    assert!(
        net.posts_to(mediator_uri) > 0,
        "requests went through the mediator"
    );
    net.set_down("https://b.example.org/tsp", false);
    host.append_turn(Position::new(3), &[turn(&obj, 4, "direct")])
        .unwrap();
    assert_eq!(rb.head(), Position::new(4), "B caught up over direct TSP");
    stop.store(true, Ordering::SeqCst);
    assert!(pump.join().unwrap() >= 4);
}

/// Hosts share the object's TSP keys: both open a message addressed to the object, but only
/// the writer's journal append succeeds, so only the writer would acknowledge it.
#[test]
fn hosts_share_the_object_vid_and_only_the_writer_appends() {
    use vertep_federation::envelope::RelayEnvelope;
    use vertep_federation::tsp::TspReceived;
    use vertep_objects::custody::{CustodyStore, ObjectKeys};
    use vertep_objects::testkit::CustodyFixture;

    let f = CustodyFixture::new(
        &["did:web:a.example.org", "did:web:b.example.org"],
        &["did:web:k.example.net"],
        None,
    );
    let object_keys = ObjectKeys::generate().unwrap();
    let (sig, agree) = object_keys.public_keys().unwrap();
    // Each host installs the object's keys from its share.
    let mut sessions = Vec::new();
    for d in ["did:web:a.example.org", "did:web:b.example.org"] {
        let p = f.party(d);
        let share = object_keys
            .share_with(f.object.as_str(), d, &p.id.agreement_public)
            .unwrap();
        let installed = share
            .install(p.id.keys.as_ref(), &p.id.agreement, &p.id.keys)
            .unwrap();
        assert_eq!(p.id.keys.public_key(&installed.signing).unwrap(), sig);
        assert_eq!(p.id.keys.public_key(&installed.agreement).unwrap(), agree);
        let identity = ActorIdentity {
            did: f.object.as_str().to_owned(),
            signing: p_ref(&format!("{}#sig", f.object.as_str()), &installed.signing),
            agreement: p_ref(
                &format!("{}#agree", f.object.as_str()),
                &installed.agreement,
            ),
            retired: Vec::new(),
            services: Vec::new(),
            rotatable: true,
            grace_ms: 0,
        };
        let me = TspIdentity::new(
            &identity,
            p.id.keys.clone() as Arc<dyn KeyManager>,
            url::Url::parse("https://mediator.example.net/tsp").unwrap(),
        )
        .unwrap();
        sessions.push(TspSession::new(me, false));
    }
    // A sender seals one event to the object's VID.
    let sender = party("https://sender.example.org/tsp");
    let object_vid = sessions[0].identity().vid().clone();
    let event = vertep_core::datamodel::Event::named(
        "quote.request",
        vertep_core::datamodel::EventType::External,
    );
    let sealed = sender
        .session
        .seal_event(&object_vid, &RelayEnvelope::from_event("m-1", &event))
        .unwrap();
    let resolver = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let resolve = |id: &str| {
        let doc = resolver
            .resolve(&Did::parse(id).unwrap())
            .map_err(|e| vertep_federation::tsp::TspError::Open(e.to_string()))?
            .document;
        vid_from_document(&doc)
    };
    for s in &sessions {
        let mut bytes = sealed.bytes.clone();
        assert!(matches!(
            s.open(&mut bytes, &resolve).unwrap(),
            TspReceived::Event(_)
        ));
    }
    // A is the writer; B is a standby: B's append (the step before an acknowledgement) fails.
    let a = f.custody_host("did:web:a.example.org");
    a.activate().unwrap();
    let b = f.custody_host("did:web:b.example.org");
    b.set_standby(true);
    let store_b = CustodyStore::new(Arc::new(MemoryStore::new()));
    store_b.add(b.clone());
    let obj = f.object.iri();
    assert!(
        store_b
            .append(
                &obj,
                Position::ZERO,
                &[turn(&obj, 1, "x")],
                &Default::default()
            )
            .is_err()
    );
    assert_eq!(b.writer_epoch(), None, "a standby never elects on its own");
    let store_a = CustodyStore::new(Arc::new(MemoryStore::new()));
    store_a.add(a);
    store_a
        .append(
            &obj,
            Position::ZERO,
            &[turn(&obj, 1, "x")],
            &Default::default(),
        )
        .unwrap();
}

fn p_ref(kid: &str, name: &str) -> vertep_federation::KeyRef {
    vertep_federation::KeyRef {
        kid: kid.to_owned(),
        key_name: name.to_owned(),
    }
}
