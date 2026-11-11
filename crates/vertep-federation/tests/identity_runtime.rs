//! DID identities on runtime nodes: opt-in DID addresses, mixed addressing, resolution failures
//! journaled in the sender, verified origins in guards, replay after the sender rotates its keys,
//! identity history in the graph store, grant refusal end to end, and no private key material in
//! journals, snapshots, datamodels or the graph store.
#![cfg(feature = "didcomm")]

mod world;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use vertep_core::datamodel::Value;
use vertep_core::types::parse_declarations;
use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::keys::{SealedBox, SharedSecret};
use vertep_federation::origin::{DeclaredPayloads, Refusal};
use vertep_federation::vocab::DIDCOMM_PROCESSOR;
use vertep_federation::{
    ActorIdentity, DidMethodChoice, Federation, Grant, IdentityConfig, InMemoryKeyManager,
    KeyError, KeyManager, KeyType, LocalActor, PublicKey, Recipients, RelayProtocol,
    ResolutionError, Timestamp,
};
use vertep_runtime::{Address, NodeConfig, Storage};
use world::*;

fn didcomm() -> Arc<dyn RelayProtocol> {
    Arc::new(DidCommProtocol)
}

/// A key manager that counts every call.
#[derive(Default)]
struct CountingKm {
    inner: InMemoryKeyManager,
    calls: AtomicUsize,
}

impl KeyManager for CountingKm {
    fn generate(&self, t: KeyType) -> Result<String, KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.generate(t)
    }
    fn public_key(&self, n: &str) -> Result<PublicKey, KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.public_key(n)
    }
    fn sign(&self, n: &str, d: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(n, d)
    }
    fn key_agreement(&self, n: &str, p: &PublicKey) -> Result<SharedSecret, KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.key_agreement(n, p)
    }
    fn decrypt(&self, n: &str, s: &SealedBox, i: &[u8], a: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.decrypt(n, s, i, a)
    }
    fn delete(&self, n: &str) -> Result<(), KeyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(n)
    }
}

/// A key manager whose keys come from known seeds, so a test can look for them.
#[derive(Default)]
struct SeededKm {
    inner: InMemoryKeyManager,
    seeds: Mutex<Vec<[u8; 32]>>,
}

impl KeyManager for SeededKm {
    fn generate(&self, t: KeyType) -> Result<String, KeyError> {
        let mut seeds = self.seeds.lock().expect("lock");
        let n = u8::try_from(seeds.len() + 1).unwrap_or(u8::MAX);
        let seed = [n.wrapping_mul(37).wrapping_add(11); 32];
        seeds.push(seed);
        self.inner.import_seed(t, seed)
    }
    fn public_key(&self, n: &str) -> Result<PublicKey, KeyError> {
        self.inner.public_key(n)
    }
    fn sign(&self, n: &str, d: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.inner.sign(n, d)
    }
    fn key_agreement(&self, n: &str, p: &PublicKey) -> Result<SharedSecret, KeyError> {
        self.inner.key_agreement(n, p)
    }
    fn decrypt(&self, n: &str, s: &SealedBox, i: &[u8], a: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.inner.decrypt(n, s, i, a)
    }
    fn delete(&self, n: &str) -> Result<(), KeyError> {
        self.inner.delete(n)
    }
}

fn encodings(seed: &[u8; 32]) -> Vec<String> {
    vec![
        seed.iter().map(|b| format!("{b:02x}")).collect(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seed),
        base64::engine::general_purpose::STANDARD.encode(seed),
        bs58::encode(seed).into_string(),
        format!("{seed:?}"),
    ]
}

/// Every quad of a storage, as text.
fn dump(store: &Arc<dyn Storage>, any: &Address) -> String {
    let q = "SELECT ?g ?s ?p ?o WHERE { { GRAPH ?g { ?s ?p ?o } } UNION { ?s ?p ?o } }";
    let sols = store
        .graph(any.node())
        .query(q)
        .expect("query")
        .into_solutions()
        .expect("solutions");
    format!("{:?}", sols.rows)
}

/// Org A (sender DID actor) and org B (receiver DID actor) through a mediator.
struct Pair {
    _w: World,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
}

fn pair(spec: impl Fn() -> OrgSpec) -> Pair {
    let w = World::new();
    let m = mediator(&w);
    let mk = |s: OrgSpec| OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Shared)],
        ..s
    };
    let a = w.org(mk(spec()));
    let b = w.org(mk(spec()));
    let rk = vec![m.routing_key()];
    let a_id = a.did_actor("sender", "a1", vec![didcomm_service(MEDIATOR, &rk)]);
    let b_id = b.did_actor("receiver", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*", "approve"],
    ));
    Pair {
        _w: w,
        a,
        b,
        a_id,
        b_id,
    }
}

fn send(o: &mut Org, from: &str, to: &str, event: &str, payload: Value) {
    o.node
        .send(
            &addr(from),
            ev(
                "send",
                obj(&[("to", s(to)), ("event", s(event)), ("payload", payload)]),
            ),
        )
        .expect("send");
}

fn errors_of(o: &Org, a: &str) -> Vec<vertep_core::datamodel::Event> {
    inputs(&o.node, &addr(a))
        .into_iter()
        .filter(|e| e.name == "error.communication")
        .collect()
}

// @tg: verifies:: [[openspec:did-identity#Opt-In DID Identity Per Actor#Actor without a DID sends inside the cluster]] {level: integration}
#[test]
fn actor_without_a_did_sends_inside_the_cluster() {
    let w = World::new();
    let resolver = Counting::new(w.registry.clone());
    let mut org = w.org(OrgSpec {
        protocols: vec![didcomm()],
        resolver: Some(resolver.clone()),
        ..OrgSpec::default()
    });
    let km = Arc::new(CountingKm::default());
    // A DID actor exists on the node, but the two cluster actors never touch it.
    org.fed
        .provision(
            &org.node.handle(),
            &ty("receiver"),
            "did",
            &IdentityConfig::new(DidMethodChoice::Key),
            km.clone(),
        )
        .expect("provisioned");
    let km_calls = km.calls.load(Ordering::SeqCst);
    let a = Address::virtual_actor(&ty("sender"), "x").expect("address");
    let b = Address::virtual_actor(&ty("receiver"), "y").expect("address");
    send(
        &mut org,
        a.as_str(),
        b.as_str(),
        "order.placed",
        Value::Null,
    );
    settle(&mut [&mut org]);
    assert_eq!(names(&org.node, &b), vec!["order.placed"]);
    assert_eq!(resolver.calls(), 0, "no DID resolved");
    assert_eq!(km.calls.load(Ordering::SeqCst), km_calls, "no key used");
    assert!(w.net.posts().is_empty(), "nothing sealed or posted");
    assert!(org.fed.outbound_records().is_empty());
}

// @tg: verifies:: [[openspec:did-identity#Opt-In DID Identity Per Actor#Actor configured with a DID]] {level: integration}
#[test]
fn actor_configured_with_a_did() {
    let w = World::new();
    let mut org = w.org(OrgSpec {
        protocols: vec![didcomm()],
        config: NodeConfig {
            snapshot_interval: 1,
            ..NodeConfig::default()
        },
        ..OrgSpec::default()
    });
    let id = org.did_actor("receiver", "k1", Vec::new());
    assert!(id.did.starts_with("did:peer:2"));
    let a = addr(&id.did);
    org.node
        .send(&a, ev("hello", Value::Null))
        .expect("local send");
    settle(&mut [&mut org]);
    let journal = org.node.journal(&a).expect("journal");
    assert!(!journal.is_empty());
    assert!(
        journal.iter().all(|e| e.actor == a),
        "journal subject is the DID"
    );
    let g = org.store.graph(a.node());
    let state = g
        .query(&format!(
            "SELECT ?s WHERE {{ GRAPH <{d}> {{ <{d}> <{p}> ?s }} }}",
            d = id.did,
            p = vertep_runtime::vocab::IN_STATE
        ))
        .expect("query")
        .into_solutions()
        .expect("rows");
    assert!(
        !state.is_empty(),
        "the current-state graph is named by the DID"
    );
    let last = journal.last().expect("entry").position;
    let snap = g
        .query(&format!(
            "SELECT ?s WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }} LIMIT 1",
            a.snapshot_iri(last).as_str()
        ))
        .expect("query")
        .into_solutions()
        .expect("rows");
    assert!(!snap.is_empty(), "snapshots are under the DID");
}

// @tg: verifies:: [[openspec:did-identity#Opt-In DID Identity Per Actor#Mixed addressing in one chart]] {level: integration}
#[test]
fn mixed_addressing_in_one_chart() {
    let mut p = pair(OrgSpec::default);
    let local = Address::virtual_actor(&ty("receiver"), "local").expect("address");
    p.a.node
        .send(
            &addr(&p.a_id.did),
            ev(
                "mixed",
                obj(&[("local", s(local.as_str())), ("remote", s(&p.b_id.did))]),
            ),
        )
        .expect("send");
    settle(&mut [&mut p.a, &mut p.b]);
    let entry =
        p.a.node
            .journal(&addr(&p.a_id.did))
            .expect("journal")
            .into_iter()
            .find(|e| e.effects().is_some_and(|f| f.sends.len() == 2))
            .expect("both sends journaled in one entry's outbox");
    let targets: Vec<_> = entry
        .effects()
        .expect("effects")
        .sends
        .iter()
        .filter_map(|s| s.recipient.clone())
        .collect();
    assert_eq!(targets, vec![local.as_str().to_owned(), p.b_id.did.clone()]);
    assert_eq!(
        names(&p.a.node, &local),
        vec!["ping"],
        "vt: send stayed in the cluster"
    );
    assert_eq!(names(&p.b.node, &addr(&p.b_id.did)), vec!["order.placed"]);
    let relayed: Vec<_> =
        p.a.fed
            .outbound_records()
            .into_iter()
            .map(|r| r.to)
            .collect();
    assert_eq!(
        relayed,
        vec![p.b_id.did.clone()],
        "only the DID went through the relay"
    );
}

// @tg: verifies:: [[openspec:did-identity#DID Resolution As Address Resolution#Resolution failure is reported, not retried forever]] {level: integration}
#[test]
fn resolution_failure_is_reported_and_journaled() {
    let w = World::new();
    let resolver = Counting::new(w.registry.clone());
    let mut org = w.org(OrgSpec {
        protocols: vec![didcomm()],
        resolver: Some(resolver.clone()),
        ..OrgSpec::default()
    });
    let a = org.did_actor("sender", "a", Vec::new());
    let target = "did:web:unreachable.example";
    org.fed
        .grant_outbound(Grant::new(a.did.as_str(), target, ["*"]));
    *resolver.fail.lock().expect("lock") = Some(ResolutionError::Timeout(
        "did:web:unreachable.example".into(),
    ));
    send(&mut org, &a.did, target, "order.placed", Value::Null);
    settle(&mut [&mut org]);
    let errors = errors_of(&org, &a.did);
    assert_eq!(errors.len(), 1, "journaled in the sender's journal");
    assert!(format!("{:?}", errors[0].data).contains("timed out"));
    assert_eq!(resolver.calls(), 1, "not retried");
    assert!(org.fed.outbox().pending().is_empty());
}

#[test]
fn unsupported_method_from_a_chart_names_the_method() {
    let w = World::new();
    let mut org = w.org(OrgSpec {
        protocols: vec![didcomm()],
        ..OrgSpec::default()
    });
    let a = org.did_actor("sender", "a", Vec::new());
    org.fed
        .grant_outbound(Grant::new(a.did.as_str(), "did:example:123", ["*"]));
    send(
        &mut org,
        &a.did,
        "did:example:123",
        "order.placed",
        Value::Null,
    );
    settle(&mut [&mut org]);
    let errors = errors_of(&org, &a.did);
    assert_eq!(errors.len(), 1);
    assert!(format!("{:?}", errors[0].data).contains("did:example"));
}

// @tg: verifies:: [[openspec:did-identity#Authenticated Event Origin#Guard accepts only a specific sender]] {level: e2e}
#[test]
fn guard_accepts_only_a_specific_sender() {
    let mut p = pair(OrgSpec::default);
    let b = addr(&p.b_id.did);
    // The approver is some other DID; A's approve must not pass the guard.
    let approver = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    p.b.node
        .send(&b, ev("set.approver", s(approver)))
        .expect("local send");
    p.b.node.run_until_idle();
    send(&mut p.a, &p.a_id.did, &p.b_id.did, "approve", Value::Null);
    settle(&mut [&mut p.a, &mut p.b]);
    let got = inputs(&p.b.node, &b);
    assert_eq!(got.last().map(|e| e.name.as_str()), Some("approve"));
    assert_eq!(
        got.last().and_then(|e| e.origin.as_deref()),
        Some(p.a_id.did.as_str())
    );
    assert_eq!(
        p.b.node.state(&b).expect("state").configuration,
        vec!["idle"]
    );
    // From the approver's DID it passes.
    p.b.node
        .send(&b, ev("set.approver", s(&p.a_id.did)))
        .expect("local send");
    p.b.node.run_until_idle();
    send(&mut p.a, &p.a_id.did, &p.b_id.did, "approve", Value::Null);
    settle(&mut [&mut p.a, &mut p.b]);
    assert_eq!(
        p.b.node.state(&b).expect("state").configuration,
        vec!["approved"]
    );
}

// @tg: verifies:: [[openspec:did-identity#Journaled Verification Outcome#Replay after key rotation]] {level: integration}
#[test]
fn replay_after_key_rotation() {
    let w = World::new();
    let m = mediator(&w);
    let resolver = Counting::new(w.registry.clone());
    let spec = || OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Shared)],
        resolver: Some(resolver.clone()),
        ..OrgSpec::default()
    };
    let mut a = w.org(spec());
    let mut b = w.org(spec());
    let rk = vec![m.routing_key()];
    // A's DID is hosted (did:web), so its keys can rotate.
    let a_did = "did:web:a.example";
    let mut a_id = a
        .fed
        .provision(
            &a.node.handle(),
            &ty("sender"),
            "a1",
            &IdentityConfig::new(DidMethodChoice::Hosted {
                did: a_did.into(),
                services: vec![didcomm_service(MEDIATOR, &rk)],
            }),
            a.km.clone(),
        )
        .expect("provisioned");
    w.docs.put(a_id.document(a.km.as_ref()).expect("doc"));
    let b_id = b.did_actor("receiver", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    a.fed
        .grant_outbound(Grant::new(a_did, b_id.did.as_str(), ["*"]));
    b.fed
        .grant_inbound(Grant::new(a_did, b_id.did.as_str(), ["approve", "order.*"]));
    let bb = addr(&b_id.did);
    b.node
        .send(&bb, ev("set.approver", s(a_did)))
        .expect("send");
    b.node.run_until_idle();
    send(&mut a, a_did, &b_id.did, "approve", Value::Null);
    settle(&mut [&mut a, &mut b]);
    let before = b.node.state(&bb).expect("state");
    assert_eq!(before.configuration, vec!["approved"]);
    let journaled = inputs(&b.node, &bb);

    // A rotates its keys and republishes; the old key is gone.
    let doc = a_id
        .rotate(
            a.km.as_ref(),
            Timestamp(vertep_runtime::Clock::now_ms(&w.clock)),
        )
        .expect("rotated");
    w.docs.put(doc);
    a.fed.update_identity(a_id.clone());

    // B restarts and replays its journal.
    let store = b.store.clone();
    let km = b.km.clone();
    b = w.org(OrgSpec {
        store: Some(store),
        km: Some(km),
        ..spec()
    });
    b.rehost("receiver", "b1", &b_id);
    let calls = resolver.calls();
    let after = b.node.state(&bb).expect("replayed");
    assert_eq!(
        resolver.calls(),
        calls,
        "replay neither resolves nor verifies"
    );
    assert_eq!(after.configuration, before.configuration);
    assert_eq!(after.datamodel, before.datamodel);
    let replayed = inputs(&b.node, &bb);
    assert_eq!(replayed, journaled);
    let approve = replayed
        .iter()
        .find(|e| e.name == "approve")
        .expect("approve");
    assert_eq!(
        approve.origin.as_deref(),
        Some(a_did),
        "origin as recorded at receipt"
    );
    assert_eq!(approve.origintype.as_deref(), Some(DIDCOMM_PROCESSOR));
}

// @tg: verifies:: [[openspec:did-identity#DID Documents In The Runtime Graph#Document change is history, not overwrite]] {level: integration}
#[test]
fn document_change_is_history_not_overwrite() {
    let dir = tempfile::tempdir().expect("tmp");
    let shard = Arc::new(
        vertep_tiramemsu::TiramemsuShard::open(dir.path().join("identity.tira")).expect("shard"),
    );
    let w = World::new();
    let fed = Federation::builder(w.registry.clone(), w.net.clone())
        .storage(shard.clone() as Arc<dyn Storage>)
        .build();
    let km = Arc::new(InMemoryKeyManager::new());
    let did = "did:web:billing.example";
    let mut id = ActorIdentity::provision(
        km.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Hosted {
            did: did.into(),
            services: Vec::new(),
        }),
    )
    .expect("identity");
    fed.host(
        LocalActor {
            identity: id.clone(),
            km: km.clone(),
        },
        Default::default(),
        Arc::new(vertep_federation::origin::Untyped),
    );
    fed.publish_identity(did).expect("published");
    let t1 = shard.last_tx().expect("tx");
    let keys = |view: &dyn vertep_store::GraphStore| -> Vec<String> {
        let q = format!(
            "SELECT ?k WHERE {{ GRAPH <{g}> {{ ?vm <{p}> ?k }} }} ORDER BY ?k",
            g = Federation::identity_graph(did),
            p = "https://w3id.org/vertep/vt-id#publicKeyMultibase"
        );
        view.query(&q)
            .expect("query")
            .into_solutions()
            .expect("rows")
            .values("k")
            .iter()
            .map(ToString::to_string)
            .collect()
    };
    let old = keys(shard.as_ref());
    assert_eq!(old.len(), 2);
    id.rotate(km.as_ref(), Timestamp(T0)).expect("rotated");
    assert!(fed.update_identity(id));
    let new = keys(shard.as_ref());
    assert_eq!(new.len(), 2);
    assert!(old.iter().all(|k| !new.contains(k)), "the document changed");
    assert_eq!(
        keys(&shard.view_as_of(t1)),
        old,
        "the previous state is still queryable"
    );
}

// @tg: verifies:: [[openspec:did-identity#DID-Scoped Capability Grants#Event outside the grant]] {level: e2e}
/// Task 10.3: two clusters through a mediator; verified origin, a guard on the sender DID and a
/// grant refusal reported back to the sender.
#[test]
fn two_clusters_through_a_mediator() {
    let mut p = pair(OrgSpec::default);
    let b = addr(&p.b_id.did);
    p.b.node
        .send(&b, ev("set.approver", s(&p.a_id.did)))
        .expect("send");
    p.b.node.run_until_idle();
    send(
        &mut p.a,
        &p.a_id.did,
        &p.b_id.did,
        "order.placed",
        Value::Null,
    );
    send(
        &mut p.a,
        &p.a_id.did,
        &p.b_id.did,
        "admin.reset",
        Value::Null,
    );
    send(&mut p.a, &p.a_id.did, &p.b_id.did, "approve", Value::Null);
    settle(&mut [&mut p.a, &mut p.b]);
    let got = inputs(&p.b.node, &b);
    let order = got
        .iter()
        .find(|e| e.name == "order.placed")
        .expect("delivered");
    assert_eq!(order.origin.as_deref(), Some(p.a_id.did.as_str()));
    assert_eq!(order.origintype.as_deref(), Some(DIDCOMM_PROCESSOR));
    assert!(got.iter().all(|e| e.name != "admin.reset"), "not delivered");
    assert_eq!(
        p.b.node.state(&b).expect("state").configuration,
        vec!["approved"]
    );
    let refused: Vec<_> =
        p.b.fed
            .inbound_records()
            .into_iter()
            .filter(|r| matches!(r.refusal, Some(Refusal::NotGranted(_))))
            .collect();
    assert_eq!(refused.len(), 1, "the refusal is recorded");
    assert_eq!(refused[0].event.as_deref(), Some("admin.reset"));
    // Auditors find what B accepted from A, with the verification outcome, in B's graph store.
    let q = format!(
        "PREFIX vt: <https://w3id.org/vertep/vt#>
         PREFIX vt-id: <https://w3id.org/vertep/vt-id#>
         SELECT ?event ?accepted WHERE {{ GRAPH ?g {{
           ?m vt-id:receivedBy <{b}> ; vt-id:verifiedOrigin <{a}> ;
              vt-id:originVerified true ; vt-id:accepted ?accepted ; vt:eventName ?event }} }}
         ORDER BY ?event",
        a = p.a_id.did,
        b = p.b_id.did
    );
    let rows =
        p.b.store
            .graph(b.node())
            .query(&q)
            .expect("query")
            .into_solutions()
            .expect("rows");
    let events: Vec<String> = rows
        .values("event")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        events,
        vec!["\"admin.reset\"", "\"approve\"", "\"order.placed\""]
    );
    // The sender gets error.communication over the reply path, for that send.
    let errors = errors_of(&p.a, &p.a_id.did);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].sendid.is_some());
    assert!(format!("{:?}", errors[0].data).contains("not granted"));
    assert_eq!(errors[0].origin.as_deref(), Some(p.b_id.did.as_str()));
}

// @tg: verifies:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping#Payload fails the receiver's type]] {level: e2e}
#[test]
fn payload_type_rejection_between_nodes() {
    let mut p = pair(OrgSpec::default);
    let decls =
        parse_declarations("event order.placed: { id: string, amount: decimal };").expect("decls");
    p.b.fed
        .set_payload_types(&p.b_id.did, Arc::new(DeclaredPayloads::new(decls)));
    send(
        &mut p.a,
        &p.a_id.did,
        &p.b_id.did,
        "order.placed",
        obj(&[("id", Value::Number(5.0))]),
    );
    settle(&mut [&mut p.a, &mut p.b]);
    assert!(
        names(&p.b.node, &addr(&p.b_id.did)).is_empty(),
        "not delivered"
    );
    assert!(
        p.b.fed
            .inbound_records()
            .iter()
            .any(|r| matches!(r.refusal, Some(Refusal::PayloadType { .. })))
    );
    assert_eq!(errors_of(&p.a, &p.a_id.did).len(), 1, "the sender is told");
}

/// Task 3.4: no private key material in journals, snapshots, datamodels or the graph store.
#[test]
fn no_private_key_material_in_journals_snapshots_datamodels_or_graphs() {
    let w = World::new();
    let m = mediator(&w);
    let spec = || OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(&m, Recipients::Shared)],
        config: NodeConfig {
            snapshot_interval: 1,
            ..NodeConfig::default()
        },
        ..OrgSpec::default()
    };
    let mut a = w.org(spec());
    let mut b = w.org(spec());
    let km = Arc::new(SeededKm::default());
    let rk = vec![m.routing_key()];
    let provision = |o: &Org, chart: &str| {
        o.fed
            .provision(
                &o.node.handle(),
                &ty(chart),
                "k",
                &IdentityConfig::new(DidMethodChoice::Peer {
                    services: vec![didcomm_service(MEDIATOR, &rk)],
                }),
                km.clone(),
            )
            .expect("provisioned")
    };
    let a_id = provision(&a, "sender");
    let b_id = provision(&b, "receiver");
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed
        .grant_inbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    a.node
        .send(&addr(&a_id.did), ev("go", obj(&[("to", s(&b_id.did))])))
        .expect("send");
    settle(&mut [&mut a, &mut b]);
    assert_eq!(names(&b.node, &addr(&b_id.did)), vec!["order.placed"]);
    let mut text = String::new();
    for (o, id) in [(&mut a, &a_id), (&mut b, &b_id)] {
        let x = addr(&id.did);
        text.push_str(&format!("{:?}", o.node.journal(&x).expect("journal")));
        text.push_str(&format!("{:?}", o.node.state(&x).expect("state").datamodel));
        text.push_str(&dump(&o.store, &x));
    }
    assert!(
        text.contains("publicKeyMultibase"),
        "the identity graph is there"
    );
    let seeds = km.seeds.lock().expect("lock").clone();
    assert_eq!(seeds.len(), 4);
    for seed in &seeds {
        for e in encodings(seed) {
            assert!(!text.contains(&e), "private key material leaked: {e}");
        }
    }
}

// @tg: verifies:: [[openspec:did-identity#Unauthenticated Event Marking#Anonymous message allowed]] {level: e2e}
#[test]
fn anonymous_sender_from_a_cluster_actor() {
    let mut p = pair(OrgSpec::default);
    // A cluster actor without a DID sends to B's DID: anoncrypt, no sender to verify.
    let anon = Address::virtual_actor(&ty("sender"), "no-did").expect("address");
    p.a.fed
        .grant_outbound(Grant::new(anon.as_str(), p.b_id.did.as_str(), ["*"]));
    send(
        &mut p.a,
        anon.as_str(),
        &p.b_id.did,
        "order.anon",
        Value::Null,
    );
    settle(&mut [&mut p.a, &mut p.b]);
    let b = addr(&p.b_id.did);
    assert!(
        names(&p.b.node, &b).is_empty(),
        "refused by the default policy"
    );
    assert!(
        p.b.fed
            .inbound_records()
            .iter()
            .any(|r| matches!(r.refusal, Some(Refusal::AnonymousRefused)))
    );
    // Once B allows anonymous senders, the event arrives without an origin.
    p.b.fed
        .set_anonymous(&p.b_id.did, vertep_federation::AnonymousPolicy::Allow);
    send(
        &mut p.a,
        anon.as_str(),
        &p.b_id.did,
        "order.anon",
        Value::Null,
    );
    settle(&mut [&mut p.a, &mut p.b]);
    let got = inputs(&p.b.node, &b);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origin, None, "unverified: no sender DID");
    assert_eq!(got[0].origintype.as_deref(), Some(DIDCOMM_PROCESSOR));
}

// @tg: verifies:: [[openspec:did-identity#Outbound Grant Enforcement#Ungranted outbound send]] {level: e2e}
#[test]
fn ungranted_outbound_send_leaves_nothing_on_the_wire() {
    let w = World::new();
    let mut org = w.org(OrgSpec {
        protocols: vec![didcomm()],
        ..OrgSpec::default()
    });
    let a = org.did_actor("sender", "llm", Vec::new());
    let external = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    send(&mut org, &a.did, external, "quote.request", Value::Null);
    settle(&mut [&mut org]);
    let errors = errors_of(&org, &a.did);
    assert_eq!(errors.len(), 1);
    assert!(format!("{:?}", errors[0].data).contains("not granted"));
    assert!(w.net.posts().is_empty(), "no envelope produced");
    assert!(org.fed.outbox().pending().is_empty());
}

// @tg: verifies:: [[openspec:did-identity#Credential-Based Grants#Valid credential grant]] {level: e2e}
#[test]
fn credential_grant_between_nodes() {
    let mut p = pair(OrgSpec::default);
    let issuer_km = InMemoryKeyManager::new();
    let issuer = ActorIdentity::provision(&issuer_km, &IdentityConfig::new(DidMethodChoice::Key))
        .expect("issuer");
    let grant = Grant::new(p.a_id.did.as_str(), p.b_id.did.as_str(), ["quote.*"]);
    let vc = vertep_federation::issue_grant_credential(&issuer_km, &issuer, &grant, Timestamp(T0))
        .expect("issued");
    let untrusted =
        p.b.fed
            .accept_credential(&p.b_id.did, &vc, &["did:web:other.example".to_owned()]);
    assert!(untrusted.is_err(), "an untrusted issuer grants nothing");
    p.b.fed
        .accept_credential(&p.b_id.did, &vc, std::slice::from_ref(&issuer.did))
        .expect("accepted");
    send(
        &mut p.a,
        &p.a_id.did,
        &p.b_id.did,
        "quote.request",
        Value::Null,
    );
    settle(&mut [&mut p.a, &mut p.b]);
    assert_eq!(names(&p.b.node, &addr(&p.b_id.did)), vec!["quote.request"]);
}
