//! `webvh-identity`: custodians witness their object's DID log, and custody changes publish
//! themselves as witnessed log entries (in process and over TSP).

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use vertep_federation::did::{Service, ServiceEndpoint};
use vertep_federation::resolver::HttpFetcher;
use vertep_federation::tsp::{TspIdentity, TspSession, vid_from_document};
use vertep_federation::{
    ActorIdentity, Did, DidDocument, DidMethodChoice, DidResolver, IdentityConfig,
    InMemoryKeyManager, KeyManager, KeyType, LocalNetwork, RelayEndpoint, RelayNetwork,
    ResolverRegistry,
};
use vertep_objects::custody::tsp_link::{
    ReplyInbox, ResolveVid, TspCustodyEndpoint, TspCustodyTransport,
};
use vertep_objects::custody::{
    Custodian, CustodyConfig, CustodyHost, CustodyRequest, CustodyResponse, Replica,
    ResolverKeyDirectory, WitnessRequest,
};
use vertep_objects::testkit::{CustodyFixture, TestHost};
use vertep_objects::{DurableObjectId, KeyManagerSigner};
use vertep_store::{JournalStore, MemoryStore};
use vertep_webvh::document::{object_document, provision, with_custody};
use vertep_webvh::witness::{
    CustodianWitness, WebvhPublisher, custody_change, custody_witnessing, publish_witnessed,
};
use vertep_webvh::{
    Author, CreateOptions, DidLog, DirectoryFetcher, DirectoryPublisher, KmSigner, LogPublisher,
    WebvhError,
};

/// An object whose custodians are `f`'s parties, created and published with its genesis
/// witnessed by them.
struct Object {
    f: CustodyFixture,
    author: Author,
    log: DidLog,
    site: tempfile::TempDir,
    publisher: Arc<DirectoryPublisher>,
}

fn object(hosts: &[&str], keepers: &[&str], quorum: Option<usize>) -> Object {
    let mut f = CustodyFixture::new(hosts, keepers, quorum);
    let keys = common::keys();
    let mut author = common::author(&keys, true);
    let options = CreateOptions::new("example.org")
        .path("objects:ticket-8812")
        .pre_rotation(true)
        .witnesses(custody_witnessing(&f.config, f.directory.as_ref()).unwrap());
    let identity = provision(
        keys.as_ref(),
        &options,
        vec![common::tsp_service("https://hosts.example/tsp")],
    )
    .unwrap();
    let doc = object_document(&identity, keys.as_ref(), &f.config, &f.controller.public).unwrap();
    let log = author.create(&options, &doc).unwrap();
    f.rebind(DurableObjectId::new(log.did()).unwrap());
    for p in &f.parties {
        p.replica.set_witness(Arc::new(
            CustodianWitness::new(
                p.id.keys.clone() as Arc<dyn KeyManager>,
                p.id.public.multikey(),
            )
            .unwrap(),
        ));
    }
    let site = tempfile::tempdir().unwrap();
    let publisher = Arc::new(DirectoryPublisher::new(site.path()));
    let host = f.custody_host(hosts[0]);
    publish_witnessed(publisher.as_ref(), log.clone(), host.as_ref()).unwrap();
    let log = publisher.current(log.did()).unwrap().unwrap();
    Object {
        f,
        author,
        log,
        site,
        publisher,
    }
}

fn with_tsp(doc: &Value, url: &str) -> Value {
    let mut d = DidDocument::from_json(doc).unwrap();
    d.service.retain(|s| !s.has_type("TSPTransport"));
    let mut s = common::tsp_service(url);
    s.id = format!("{}#tsp", d.id);
    d.service.push(s);
    d.to_json()
}

fn proofs_for(log: &DidLog, version_id: &str) -> usize {
    log.witness_proofs()
        .iter()
        .filter(|r| r["versionId"] == version_id)
        .map(|r| r["proof"].as_array().map_or(0, Vec::len))
        .sum()
}

const FIVE: [&str; 3] = [
    "did:web:a.example",
    "did:web:b.example",
    "did:web:c.example",
];
const KEEPERS: [&str; 2] = ["did:web:k1.example", "did:web:k2.example"];

// @tg: verifies:: [[openspec:webvh-identity#Custodians Witness Object Logs#Witnessed by a quorum]] {level: integration}
#[test]
fn witnessed_by_a_quorum() {
    let mut o = object(&FIVE, &KEEPERS, Some(3));
    assert_eq!(o.f.config.custodians.len(), 5);
    let v = o.log.verify().unwrap();
    assert_eq!(
        v.witnesses.as_ref().map(|(w, t)| (w.len(), *t)),
        Some((5, 3))
    );

    // Two custodians are unreachable: the writer and two others witness.
    o.f.set_up(KEEPERS[0], false);
    o.f.set_up(KEEPERS[1], false);
    let mut log = o.log.clone();
    let doc = with_tsp(log.document().unwrap(), "https://moved.example/tsp");
    let vid = o.author.update(&mut log, doc).unwrap();
    let host = o.f.custody_host(FIVE[0]);
    publish_witnessed(o.publisher.as_ref(), log, host.as_ref()).unwrap();

    let published = o.publisher.current(o.log.did()).unwrap().unwrap();
    assert_eq!(published.version_id(), Some(vid.as_str()));
    assert_eq!(proofs_for(&published, &vid), 3);
    let r = common::resolve(o.site.path(), o.log.did());
    assert!(
        r.document
            .service
            .iter()
            .any(|s| s.endpoints[0].uri == "https://moved.example/tsp")
    );
}

// @tg: verifies:: [[openspec:webvh-identity#Custodians Witness Object Logs#Too few witnesses blocks publishing]] {level: integration}
#[test]
fn too_few_witnesses_blocks_publishing() {
    let mut o = object(&FIVE, &KEEPERS, Some(3));
    for d in [FIVE[2], KEEPERS[0], KEEPERS[1]] {
        o.f.set_up(d, false);
    }
    let before = std::fs::read_to_string(
        o.site
            .path()
            .join(vertep_webvh::log_dir(o.log.did()).unwrap())
            .join("did.jsonl"),
    )
    .unwrap();
    let mut log = o.log.clone();
    let doc = with_tsp(log.document().unwrap(), "https://moved.example/tsp");
    o.author.update(&mut log, doc).unwrap();
    let host = o.f.custody_host(FIVE[0]);
    let err = publish_witnessed(o.publisher.as_ref(), log, host.as_ref()).unwrap_err();
    match &err {
        WebvhError::Unwitnessed { have, need, .. } => assert_eq!((*have, *need), (2, 3)),
        other => panic!("expected too few witnesses, got {other}"),
    }
    assert!(err.to_string().contains("1 missing"), "{err}");
    let after = std::fs::read_to_string(
        o.site
            .path()
            .join(vertep_webvh::log_dir(o.log.did()).unwrap())
            .join("did.jsonl"),
    )
    .unwrap();
    assert_eq!(before, after, "the published log is unchanged");
}

fn request(log: &DidLog, entry_log: &DidLog) -> WitnessRequest {
    WitnessRequest {
        object: log.did().to_owned(),
        log: log.jsonl(),
        witness: log.witness_json(),
        entry: entry_log.entries().last().cloned().unwrap(),
    }
}

// @tg: verifies:: [[openspec:webvh-identity#Custodian Witnessing Rules#Valid entry witnessed]] {level: integration}
#[test]
fn valid_entry_witnessed() {
    let mut o = object(&FIVE[..2], &KEEPERS[..1], None);
    let mut next = o.log.clone();
    let doc = with_tsp(next.document().unwrap(), "https://moved.example/tsp");
    let vid = o.author.update(&mut next, doc).unwrap();
    let keeper = o.f.party(KEEPERS[0]);
    let answer = keeper
        .replica
        .handle(&CustodyRequest::Witness(request(&o.log, &next)));
    let CustodyResponse::Witnessed { custodian, proof } = answer else {
        panic!("expected a proof, got {answer:?}");
    };
    assert_eq!(custodian, KEEPERS[0]);
    let mk = keeper.id.public.multikey();
    assert_eq!(proof["verificationMethod"], format!("did:key:{mk}#{mk}"));
    // The proof counts toward the entry.
    next.add_witness_proof(&vid, &proof).unwrap();
    assert_eq!(next.last_entry_witnessing().unwrap(), Some((1, 2)));
}

// @tg: verifies:: [[openspec:webvh-identity#Custodian Witnessing Rules#Unaccepted custodian set refused]] {level: integration}
#[test]
fn unaccepted_custodian_set_refused() {
    let mut o = object(&FIVE[..2], &KEEPERS[..1], None);
    let rogue = CustodyConfig {
        version: 2,
        custodians: vec![
            Custodian::host(FIVE[0]),
            Custodian::host("did:web:mallory.example"),
            Custodian::keeper(KEEPERS[0]),
        ],
        quorum: 2,
    };
    let mut next = o.log.clone();
    let doc = with_custody(next.document().unwrap(), &rogue).unwrap();
    o.author.update(&mut next, doc).unwrap();
    let answer =
        o.f.party(KEEPERS[0])
            .replica
            .handle(&CustodyRequest::Witness(request(&o.log, &next)));
    match answer {
        CustodyResponse::Refused { reason, .. } => {
            assert!(reason.contains("has not accepted"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:webvh-identity#Custodian Witnessing Rules#Forged entry refused]] {level: integration}
#[test]
fn forged_entry_refused() {
    let mut o = object(&FIVE[..2], &KEEPERS[..1], None);
    let mut next = o.log.clone();
    let doc = with_tsp(next.document().unwrap(), "https://moved.example/tsp");
    o.author.update(&mut next, doc).unwrap();
    // Re-sign the entry with a key that is not an update key of the DID.
    let mut entry = next.entries().last().cloned().unwrap();
    entry.as_object_mut().unwrap().remove("proof");
    let keys = common::keys();
    let rogue = KmSigner::new(keys.clone(), keys.generate(KeyType::Ed25519).unwrap()).unwrap();
    let proof = futures::executor::block_on(affinidi_data_integrity::DataIntegrityProof::sign(
        &entry,
        &rogue,
        affinidi_data_integrity::SignOptions::new(),
    ))
    .unwrap();
    entry["proof"] = Value::Array(vec![serde_json::to_value(proof).unwrap()]);
    let req = WitnessRequest {
        object: o.log.did().to_owned(),
        log: o.log.jsonl(),
        witness: o.log.witness_json(),
        entry,
    };
    match o
        .f
        .party(KEEPERS[0])
        .replica
        .handle(&CustodyRequest::Witness(req))
    {
        CustodyResponse::Refused { reason, .. } => {
            assert!(reason.contains("update key"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn resolver_for(site: &std::path::Path) -> Arc<dyn DidResolver> {
    let fetcher: Arc<dyn HttpFetcher> = Arc::new(DirectoryFetcher::new(site));
    Arc::new(ResolverRegistry::with_builtin(
        Some(fetcher),
        Duration::from_secs(5),
    ))
}

// @tg: verifies:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries#Replace a keeper end to end]] {level: integration}
#[test]
fn replace_a_keeper_end_to_end() {
    let mut o = object(&FIVE[..2], &KEEPERS[..1], None);
    let host = o.f.custody_host(FIVE[0]);
    host.set_publisher(Arc::new(WebvhPublisher::new(o.publisher.clone())));
    host.elect(1).unwrap();
    let obj = o.f.object.iri();
    host.append_turn(
        vertep_store::Position::ZERO,
        &[vertep_objects::testkit::turn(&obj, 1, "before the change")],
    )
    .unwrap();

    // The newcomer: a keeper with a key in the directory and a witnessing replica.
    let newcomer = o.f.make_party("did:web:k3.example");
    newcomer.replica.set_witness(Arc::new(
        CustodianWitness::new(
            newcomer.id.keys.clone() as Arc<dyn KeyManager>,
            newcomer.id.public.multikey(),
        )
        .unwrap(),
    ));
    host.add_transport(newcomer.transport.clone());
    let new = CustodyConfig {
        version: 2,
        custodians: vec![
            Custodian::host(FIVE[0]),
            Custodian::host(FIVE[1]),
            Custodian::keeper("did:web:k3.example"),
        ],
        quorum: 2,
    };
    let mut log = o.log.clone();
    let request = custody_change(
        &mut o.author,
        &mut log,
        &new,
        o.f.directory.as_ref(),
        &o.f.controller.signer,
    )
    .unwrap();
    host.reconfigure(&request).unwrap();

    let published = o.publisher.current(o.log.did()).unwrap().unwrap();
    let vid = published.version_id().unwrap().to_owned();
    assert_eq!(Some(vid.as_str()), log.version_id());
    assert!(
        proofs_for(&published, &vid) >= 2,
        "a quorum of the old set witnessed"
    );
    let v = published.verify().unwrap();
    assert_eq!(v.witnesses.map(|(w, t)| (w.len(), t)), Some((3, 2)));
    let resolved = resolver_for(o.site.path())
        .resolve(&Did::parse(o.log.did()).unwrap())
        .unwrap()
        .document;
    assert_eq!(CustodyConfig::from_document(&resolved).unwrap(), new);
}

struct TspParty {
    did: String,
    keys: Arc<InMemoryKeyManager>,
    identity: ActorIdentity,
    session: Arc<TspSession>,
}

fn tsp_party(uri: &str) -> TspParty {
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
    TspParty {
        did: identity.did.clone(),
        keys,
        identity,
        session: Arc::new(TspSession::new(me, false)),
    }
}

impl TspParty {
    fn signer(&self) -> KeyManagerSigner {
        KeyManagerSigner::new(
            &self.did,
            self.keys.clone() as Arc<dyn KeyManager>,
            self.identity.signing.key_name.clone(),
        )
    }
    fn witness(&self) -> Arc<CustodianWitness> {
        Arc::new(
            CustodianWitness::new(
                self.keys.clone() as Arc<dyn KeyManager>,
                self.identity.signing.key_name.clone(),
            )
            .unwrap(),
        )
    }
}

// @tg: verifies:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries#Replace a keeper end to end]] {level: e2e}
#[test]
fn replace_a_keeper_over_tsp() {
    let site = tempfile::tempdir().unwrap();
    let resolver = resolver_for(site.path());
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
    let net = LocalNetwork::new();
    let a = tsp_party("https://a.example.org/tsp");
    let b = tsp_party("https://b.example.org/tsp");
    let k = tsp_party("https://k.example.org/tsp");
    let k2 = tsp_party("https://k2.example.org/tsp");
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

    // The object's DID, with the custodians as witnesses and a controller key.
    let keys = common::keys();
    let mut author = common::author(&keys, true);
    let controller = TestHost::new("did:web:controller.example.org");
    let options = CreateOptions::new("example.org")
        .path("objects:quote-17")
        .pre_rotation(true)
        .witnesses(custody_witnessing(&config, directory.as_ref()).unwrap());
    let identity = provision(keys.as_ref(), &options, vec![]).unwrap();
    let doc = object_document(&identity, keys.as_ref(), &config, &controller.public).unwrap();
    let genesis = author.create(&options, &doc).unwrap();
    let object = DurableObjectId::new(genesis.did()).unwrap();

    let replica = |p: &TspParty| {
        let r = Arc::new(
            Replica::open(
                object.iri(),
                Arc::new(p.signer()),
                Arc::new(MemoryStore::new()) as Arc<dyn JournalStore>,
                directory.clone(),
                config.clone(),
            )
            .unwrap(),
        );
        r.set_witness(p.witness());
        r
    };
    let (ra, rb, rk, rk2) = (replica(&a), replica(&b), replica(&k), replica(&k2));
    for (p, r) in [(&b, &rb), (&k, &rk), (&k2, &rk2)] {
        let uri = format!(
            "https://{}/tsp",
            match p.did.as_str() {
                d if d == b.did => "b.example.org",
                d if d == k.did => "k.example.org",
                _ => "k2.example.org",
            }
        );
        net.serve(
            &uri,
            Arc::new(TspCustodyEndpoint::new(
                p.session.clone(),
                r.clone(),
                resolve.clone(),
            )) as Arc<dyn RelayEndpoint>,
        );
    }
    let inbox = ReplyInbox::new(a.session.clone(), resolve.clone());
    net.serve("https://a.example.org/tsp", inbox);
    let direct = |p: &TspParty| {
        Arc::new(TspCustodyTransport::direct(
            &p.did,
            a.session.clone(),
            net.clone() as Arc<dyn RelayNetwork>,
            resolve.clone(),
        ))
    };
    let publisher = Arc::new(DirectoryPublisher::new(site.path()));
    let host = CustodyHost::new(
        object.clone(),
        Arc::new(a.signer()),
        a.keys.clone() as Arc<dyn KeyManager>,
        a.identity.agreement.key_name.clone(),
        ra.clone(),
        directory.clone(),
    )
    .with_transport(direct(&b))
    .with_transport(direct(&k))
    .with_publisher(Arc::new(WebvhPublisher::new(publisher.clone())));

    // The genesis entry is witnessed over TSP, then published; the controller key then
    // resolves from the object's document.
    publish_witnessed(publisher.as_ref(), genesis.clone(), &host).unwrap();
    host.elect(1).unwrap();

    host.add_transport(direct(&k2));
    let new = CustodyConfig {
        version: 2,
        custodians: vec![
            Custodian::host(&a.did),
            Custodian::host(&b.did),
            Custodian::keeper(&k2.did),
        ],
        quorum: 2,
    };
    let mut log = publisher.current(genesis.did()).unwrap().unwrap();
    let request = custody_change(
        &mut author,
        &mut log,
        &new,
        directory.as_ref(),
        &controller.signer,
    )
    .unwrap();
    host.reconfigure(&request).unwrap();

    let resolved = resolver
        .resolve(&Did::parse(genesis.did()).unwrap())
        .unwrap()
        .document;
    assert_eq!(CustodyConfig::from_document(&resolved).unwrap(), new);
    let published = publisher.current(genesis.did()).unwrap().unwrap();
    assert!(proofs_for(&published, published.version_id().unwrap()) >= 2);
    assert_eq!(
        rk2.config(),
        new,
        "the newcomer accepted the switch over TSP"
    );
}
