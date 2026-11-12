//! The cluster inside the actor runtime: charts on different nodes exchange events through
//! the placement, the cluster Event I/O Processor and a wire (QUIC or in memory); shards fail
//! over to standbys; stateless workers are pooled across nodes.

mod common;
mod rt;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rt::{Cluster, StaticNode, ev, inputs, runtime_node, scxml, ty, va};
use vertep_cluster::runtime::{
    CLUSTER_PROCESSOR, ClusterConfig, ClusterNode, ClusterStorage, NodeFrameHandler, QuicWire,
};
use vertep_cluster::transport::{ClusterCa, ClusterTransport, PeerSet, Receiving, TransportConfig};
use vertep_cluster::{
    ClusterId, EpochFilter, Membership, NodeId, Policy, StaticConfig, WorkerType,
};
use vertep_core::datamodel::Value;
use vertep_runtime::actor::{ActorFn, ActorInput, OutTarget, Outgoing, TurnOutput};
use vertep_runtime::mailbox::Envelope;
use vertep_runtime::{Address, Node};
use vertep_store::{JournalStore, MemoryStore};

use common::{meta, node, timing};

const T: Duration = Duration::from_secs(10);

fn ping_chart(target: &Address) -> vertep_core::ir::Chart {
    scxml(
        "null",
        "s",
        &format!(
            r#"<state id="s"><transition event="go"><send event="ping" target="{target}"/></transition>
               <transition event="pong" target="done"/></state><state id="done"/>"#
        ),
    )
}

const PONG: &str = r#"<state id="s"><transition event="ping" target="got"><send event="pong" targetexpr="_event.origin"/></transition></state><state id="got"/>"#;

const COUNTER: &str = r#"<datamodel><data id="n" expr="0"/></datamodel>
  <state id="s"><transition event="inc"><assign location="n" expr="n + 1"/></transition></state>"#;

fn configuration(node: &mut Node, a: &Address) -> Vec<String> {
    node.state(a).map(|s| s.configuration).unwrap_or_default()
}

#[tokio::test(flavor = "current_thread")]
async fn ping_pong_across_nodes_in_memory() {
    let mut c = Cluster::new(2, 8);
    c.register("pong", &scxml("ecmascript", "s", PONG));
    let b = c.actor_on("pong", 1, "b");
    c.register("ping", &ping_chart(&b));
    let a = c.actor_on("ping", 0, "a");
    c.members[1].node.send(&a, ev("go")).unwrap();
    let ok = c
        .until(T, |c| configuration(&mut c.members[0].node, &a) == ["done"])
        .await;
    assert!(ok, "a reached done");
    assert_eq!(inputs(&c.members[1].node, &b).len(), 1, "b ran on n1");
    assert!(
        inputs(&c.members[0].node, &b).is_empty(),
        "nothing of b on n0"
    );
}

// ----- QUIC -------------------------------------------------------------------------------

struct QuicMember {
    cluster: Arc<ClusterNode>,
    node: Node,
    _transport: Arc<ClusterTransport>,
}

/// Two runtime nodes connected by the QUIC cluster transport with mutual TLS.
async fn quic_cluster() -> (Vec<QuicMember>, vertep_cluster::ClusterMeta) {
    let cluster_id = ClusterId::new("test").unwrap();
    let ca = ClusterCa::generate(&cluster_id).unwrap();
    let ids = [node("n0"), node("n1")];
    let mut bound = Vec::new();
    for id in &ids {
        let handler = NodeFrameHandler::new();
        let epochs = Arc::new(EpochFilter::new());
        let transport = ClusterTransport::bind(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            ca.issue(id).unwrap(),
            Arc::new(PeerSet::new(ids.clone())),
            Receiving {
                handler: handler.clone(),
                epochs: epochs.clone(),
                dead_letters: handler.clone(),
            },
            TransportConfig::default(),
        )
        .unwrap();
        bound.push((handler, epochs, Arc::new(transport)));
    }
    let clock = vertep_cluster::ManualClock::new();
    let config = StaticConfig {
        meta: meta(8),
        members: ids
            .iter()
            .zip(&bound)
            .map(|(id, (_, _, t))| (id.clone(), t.local_addr().unwrap().to_string()))
            .collect(),
        timing: timing(),
    };
    let authority = Arc::new(config.authority(Arc::new(clock.clone())));
    let mut members = Vec::new();
    let mut memberships = Vec::new();
    for (id, (handler, epochs, transport)) in ids.iter().zip(bound) {
        let membership: Arc<StaticNode> = Arc::new(
            config
                .membership(id, authority.clone(), Arc::new(clock.clone()))
                .unwrap(),
        );
        membership.poll().unwrap();
        memberships.push(membership.clone());
        let dynm = membership.clone() as Arc<dyn Membership>;
        let cluster = ClusterNode::with_epochs(dynm.clone(), ClusterConfig::default(), epochs);
        handler.attach(cluster.clone());
        cluster.set_wire(QuicWire::new(transport.clone(), dynm.clone()));
        let stores = (0..8).map(|_| Arc::new(MemoryStore::new())).collect();
        let storage = Arc::new(ClusterStorage::new(dynm, stores).unwrap());
        let node = runtime_node(membership.clone(), &cluster, storage.clone());
        cluster.claim(&|s| storage.holds_journal(s));
        members.push(QuicMember {
            cluster,
            node,
            _transport: transport,
        });
    }
    for (m, ms) in members.iter().zip(&memberships) {
        ms.poll().unwrap();
        m.cluster.refresh();
    }
    (members, config.meta)
}

async fn quic_until(members: &mut [QuicMember], mut done: impl FnMut(&mut [QuicMember]) -> bool) {
    let start = std::time::Instant::now();
    loop {
        for m in members.iter_mut() {
            m.node.run_until_idle();
        }
        if done(members) {
            return;
        }
        assert!(start.elapsed() < T, "timed out");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn quic_actor_on(
    members: &[QuicMember],
    meta: &vertep_cluster::ClusterMeta,
    name: &str,
    i: usize,
) -> Address {
    (0..)
        .map(|k| va(name, &format!("k{k}")))
        .find(|a| {
            members[i]
                .cluster
                .membership()
                .held(meta.shard_of(a.as_str()))
                .is_some()
        })
        .unwrap()
}

// @tg: verifies:: [[openspec:cluster-transport#QUIC Cluster Transport#Send reaches an actor on another node]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_reaches_an_actor_on_another_node() {
    let (mut m, meta) = quic_cluster().await;
    let b = quic_actor_on(&m, &meta, "pong", 1);
    for x in &mut m {
        x.node
            .register_type(&ty("pong"), scxml("ecmascript", "s", PONG))
            .unwrap();
        x.node.register_type(&ty("ping"), ping_chart(&b)).unwrap();
    }
    let a = quic_actor_on(&m, &meta, "ping", 0);
    m[0].node.send(&a, ev("go")).unwrap();
    quic_until(&mut m, |m| configuration(&mut m[0].node, &a) == ["done"]).await;
    let got = inputs(&m[1].node, &b);
    assert_eq!(
        got.len(),
        1,
        "b on n1 received the ping as an external event"
    );
    assert_eq!(got[0].name, "ping");
    assert!(inputs(&m[0].node, &b).is_empty(), "b never ran on n0");
}

// @tg: verifies:: [[openspec:cluster-transport#QUIC Cluster Transport#Event origin names the sender]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn event_origin_names_the_sender() {
    let (mut m, meta) = quic_cluster().await;
    let b = quic_actor_on(&m, &meta, "pong", 1);
    for x in &mut m {
        x.node
            .register_type(&ty("pong"), scxml("ecmascript", "s", PONG))
            .unwrap();
        x.node.register_type(&ty("ping"), ping_chart(&b)).unwrap();
    }
    let a = quic_actor_on(&m, &meta, "ping", 0);
    m[0].node.send(&a, ev("go")).unwrap();
    quic_until(&mut m, |m| configuration(&mut m[0].node, &a) == ["done"]).await;
    let ping = &inputs(&m[1].node, &b)[0];
    assert_eq!(ping.origin.as_deref(), Some(a.as_str()));
    assert_eq!(ping.origintype.as_deref(), Some(CLUSTER_PROCESSOR));
    let pong = inputs(&m[0].node, &a).pop().unwrap();
    assert_eq!(pong.origin.as_deref(), Some(b.as_str()));
    assert_eq!(pong.origintype.as_deref(), Some(CLUSTER_PROCESSOR));
}

#[tokio::test(flavor = "current_thread")]
async fn the_cluster_processor_is_listed_and_selectable() {
    let mut c = Cluster::new(2, 8);
    c.register("pong", &scxml("ecmascript", "s", PONG));
    let b = c.actor_on("pong", 1, "b");
    let chart = scxml(
        "ecmascript",
        "s",
        &format!(
            r#"<datamodel><data id="io" expr="Object.keys(_ioprocessors).indexOf('{CLUSTER_PROCESSOR}') >= 0"/></datamodel>
               <state id="s"><transition event="go"><send type="cluster" event="ping" target="{b}"/></transition>
               <transition event="pong" cond="io" target="done"/></state><state id="done"/>"#
        ),
    );
    c.register("ping", &chart);
    let a = c.actor_on("ping", 0, "a");
    c.members[0].node.send(&a, ev("go")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &a) == ["done"])
            .await
    );
}

// ----- failover and replication -------------------------------------------------------

fn count(node: &Node, a: &Address) -> usize {
    inputs(node, a).iter().filter(|e| e.name == "inc").count()
}

// @tg: verifies:: [[openspec:shard-placement#Journal Replication To Standbys#Failover to a standby]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn failover_to_a_standby() {
    // four members: after the owner fails, three are left to hold the shard's three copies
    let mut c = Cluster::replicated(4, 8, 3);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 0, "x");
    let shard = c.shard(&x);
    for _ in 0..3 {
        c.members[1].node.send(&x, ev("inc")).unwrap();
    }
    assert!(c.until(T, |c| count(&c.members[0].node, &x) == 3).await);
    // every acknowledged entry is on both standbys
    for i in [1, 2] {
        assert_eq!(
            c.members[i].stores[shard.index()]
                .head(x.node())
                .unwrap()
                .get(),
            3
        );
    }
    let epoch = c.members[0].membership.held(shard).unwrap().epoch;

    // the owner fails; after its lease expires a standby takes the shard at a higher epoch
    c.wire.set_down(&c.members[0].id, true);
    c.advance_only(Duration::from_secs(15), &[1, 2, 3]);
    let taker = (1..3)
        .find(|&i| {
            let storage = c.members[i].storage.clone();
            !c.members[i]
                .cluster
                .claim(&|s| storage.holds_journal(s))
                .is_empty()
                && c.members[i].membership.held(shard).is_some()
        })
        .or_else(|| (1..3).find(|&i| c.members[i].membership.held(shard).is_some()))
        .expect("a standby took the shard");
    assert!(c.members[taker].membership.held(shard).unwrap().epoch > epoch);
    for m in &c.members[1..] {
        m.membership.poll().unwrap();
        m.cluster.refresh();
    }
    for m in &c.members[3..] {
        m.membership.poll().unwrap();
        m.cluster.refresh();
    }
    let other = if taker == 1 { 2 } else { 1 };
    c.members[other].node.send(&x, ev("inc")).unwrap();
    assert!(
        c.until(T, |c| count(&c.members[taker].node, &x) == 4).await,
        "the actor resumed from its three acknowledged entries"
    );
    // the new owner replicates to the shard's current standbys, the member that replaces the
    // failed one included (it receives the whole journal)
    let copies = (1..4)
        .filter(|&i| i != taker)
        .filter(|&i| {
            c.members[i].stores[shard.index()]
                .head(x.node())
                .unwrap()
                .get()
                == 4
        })
        .count();
    assert_eq!(copies, 2, "the fourth entry is on two standbys");
}

// @tg: verifies:: [[openspec:shard-placement#Journal Replication To Standbys#No standby with replication factor one]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn no_standby_with_replication_factor_one() {
    let mut c = Cluster::new(2, 8);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 1, "x");
    let shard = c.shard(&x);
    c.members[0].node.send(&x, ev("inc")).unwrap();
    assert!(c.until(T, |c| count(&c.members[1].node, &x) == 1).await);

    // n1 fails; its lease expires, but n0 holds no journal of the shard and does not take it
    c.wire.set_down(&c.members[1].id, true);
    c.advance_only(Duration::from_secs(15), &[0]);
    let storage = c.members[0].storage.clone();
    let taken = c.members[0].cluster.claim(&|s| storage.holds_journal(s));
    assert!(!taken.contains(&shard), "n0 must not start the shard empty");
    assert!(c.members[0].membership.held(shard).is_none());

    // a message for the actor waits in n0's mailbox
    c.members[0].node.send(&x, ev("inc")).unwrap();
    c.run_for(Duration::from_millis(100)).await;
    assert_eq!(c.members[0].node.mailbox(&x).len(), 1, "the message waits");
    assert!(!c.members[0].node.is_active(&x));
    assert!(inputs(&c.members[0].node, &x).is_empty());

    // n1's disk comes back: it rejoins, takes its shard, and the waiting message reaches it
    c.wire.set_down(&c.members[1].id, false);
    c.advance(Duration::from_secs(1));
    let storage = c.members[1].storage.clone();
    c.members[1].cluster.claim(&|s| storage.holds_journal(s));
    assert!(c.members[1].membership.held(shard).is_some());
    for m in &c.members {
        let _ = m.membership.poll();
        m.cluster.refresh();
    }
    assert!(
        c.until(T, |c| count(&c.members[1].node, &x) == 2).await,
        "the waiting message was processed once its shard was back"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn an_append_without_quorum_is_not_acknowledged() {
    let mut c = Cluster::replicated(3, 8, 3);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 0, "x");
    let shard = c.shard(&x);
    c.members[0].node.send(&x, ev("inc")).unwrap();
    assert!(c.until(T, |c| count(&c.members[0].node, &x) == 1).await);
    // one standby is cut off: the next append reaches only one of the two
    c.members[0].links[shard.index()][0].set_up(false);
    c.members[0].node.send(&x, ev("inc")).unwrap();
    c.run_for(Duration::from_millis(100)).await;
    assert!(
        c.members[0].node.journal(&x).is_err() || count(&c.members[0].node, &x) == 1,
        "the unreplicated entry is not readable"
    );
    // the standby comes back: the entry is shipped, and the message is processed once
    for l in &c.members[0].links[shard.index()] {
        l.set_up(true);
    }
    assert!(
        c.until(Duration::from_secs(5), |c| count(&c.members[0].node, &x)
            == 2)
            .await
    );
    for i in [1, 2] {
        assert_eq!(
            c.members[i].stores[shard.index()]
                .head(x.node())
                .unwrap()
                .get(),
            2
        );
    }
}

// ----- workers --------------------------------------------------------------------------

fn worker_addr() -> Address {
    Address::parse("urn:vertep:worker:echo").unwrap()
}

/// A worker that replies `{by: <node>}`, counting its calls.
fn echo(by: &str, calls: Arc<AtomicUsize>) -> ActorFn {
    let by = by.to_owned();
    Arc::new(move |_input: ActorInput| {
        calls.fetch_add(1, Ordering::SeqCst);
        let by = by.clone();
        Box::pin(async move {
            Ok(TurnOutput {
                sends: vec![Outgoing::new(
                    OutTarget::Origin,
                    "vt:reply",
                    Value::Object(vec![("by".into(), Value::String(by))]),
                )],
                ..TurnOutput::default()
            })
        })
    })
}

/// A worker that never answers (its call hangs), counting its calls.
fn hang(calls: Arc<AtomicUsize>) -> ActorFn {
    Arc::new(move |_input: ActorInput| {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::pending())
    })
}

fn caller_chart() -> vertep_core::ir::Chart {
    let w = worker_addr();
    scxml(
        "null",
        "idle",
        &format!(
            r#"<state id="idle"><transition event="ask" target="asking"/></state>
               <state id="asking"><vt:request target="{w}" event="work" timeout="600s" reply="done" ontimeout="timedout"/>
                 <transition event="error.communication" target="failed"/></state>
               <state id="done"/><state id="timedout"/><state id="failed"/>"#
        ),
    )
}

/// Registers the worker type on every member's dispatcher and the handlers on the members
/// that run it.
fn workers(c: &mut Cluster, ty: WorkerType, handlers: &[(usize, ActorFn)]) {
    let nodes: Vec<NodeId> = handlers
        .iter()
        .map(|(i, _)| c.members[*i].id.clone())
        .collect();
    for m in &c.members {
        m.cluster.add_worker_type(ty.clone(), nodes.clone());
    }
    for (i, h) in handlers {
        c.members[*i]
            .node
            .register_worker(worker_addr(), 4, h.clone());
    }
    c.register("caller", &caller_chart());
}

fn ask_type() -> WorkerType {
    WorkerType::new(worker_addr().as_str()).with_limits(1, 1, 8)
}

// @tg: verifies:: [[openspec:worker-pools#Stateless Worker Activations#Any activation can serve a request]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn any_activation_can_serve_a_request() {
    let mut c = Cluster::new(3, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(
        &mut c,
        ask_type(),
        &[
            (1, echo("n1", calls.clone())),
            (2, echo("n2", calls.clone())),
        ],
    );
    let p = c.actor_on("caller", 0, "p");
    let q = c.actor_on("caller", 0, "q");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    c.members[0].node.send(&q, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| {
            configuration(&mut c.members[0].node, &p) == ["done"]
                && configuration(&mut c.members[0].node, &q) == ["done"]
        })
        .await
    );
    let acts = c.members[0]
        .cluster
        .worker_activations(worker_addr().as_str());
    assert_eq!(acts.len(), 2, "two activations served the two requests");
    assert_ne!(acts[0].1, acts[1].1);
    let rp = inputs(&c.members[0].node, &p).pop().unwrap();
    let rq = inputs(&c.members[0].node, &q).pop().unwrap();
    assert_eq!(rp.name, "vt:reply");
    assert_eq!(rq.name, "vt:reply");
    let keys = |v: &Value| match v {
        Value::Object(kv) => kv.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(keys(&rp.data), keys(&rq.data), "the same kind of reply");
}

// @tg: verifies:: [[openspec:worker-pools#No Journal For Workers#Worker writes nothing to the journal store]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn worker_writes_nothing_to_the_journal_store() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(&mut c, ask_type(), &[(1, echo("n1", calls.clone()))]);
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p) == ["done"])
            .await
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    for m in &c.members {
        let actors = m.storage.actors().unwrap();
        assert!(
            actors.iter().all(|a| a.as_str() != worker_addr().as_str()),
            "no journal for the worker on {}",
            m.id
        );
        assert_eq!(m.storage.head(worker_addr().node()).unwrap().get(), 0);
    }
}

// @tg: verifies:: [[openspec:worker-pools#No Journal For Workers#Activation discarded under memory pressure]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn activation_discarded_under_memory_pressure() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(&mut c, ask_type(), &[(1, echo("n1", calls.clone()))]);
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p) == ["done"])
            .await
    );
    let journal = c.members[0].node.journal(&p).unwrap().len();
    assert_eq!(c.members[0].cluster.discard_idle_workers(), 1);
    assert!(
        c.members[0]
            .cluster
            .worker_activations(worker_addr().as_str())
            .is_empty()
    );
    assert_eq!(
        c.members[0].node.journal(&p).unwrap().len(),
        journal,
        "nothing lost"
    );
    let q = c.actor_on("caller", 0, "q");
    c.members[0].node.send(&q, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &q) == ["done"])
            .await,
        "the next request created another activation"
    );
    assert_eq!(
        c.members[0]
            .cluster
            .worker_activations(worker_addr().as_str())
            .len(),
        1
    );
}

// @tg: verifies:: [[openspec:worker-pools#Results Journaled By The Caller#Replay does not call the worker]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn replay_does_not_call_the_worker() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(&mut c, ask_type(), &[(1, echo("n1", calls.clone()))]);
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p) == ["done"])
            .await
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    c.members[0].node.deactivate(&p);
    assert!(!c.members[0].node.is_active(&p));
    c.members[0].node.activate_actor(&p).unwrap();
    c.run_for(Duration::from_millis(50)).await;
    assert_eq!(configuration(&mut c.members[0].node, &p), ["done"]);
    let reply = inputs(&c.members[0].node, &p).pop().unwrap();
    assert_eq!(reply.name, "vt:reply", "the recorded result");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the worker was not called again"
    );
}

// @tg: verifies:: [[openspec:worker-pools#Results Journaled By The Caller#Duplicate result discarded]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn duplicate_result_discarded() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(&mut c, ask_type(), &[(1, echo("n1", calls.clone()))]);
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p) == ["done"])
            .await
    );
    let before = c.members[0].node.journal(&p).unwrap().len();
    // the worker's result arrives again (a retry), under another message id
    let mut again = Envelope::new("urn:vertep:test:retried-result", &p, ev("vt:reply"));
    again.correlation = Some(p.correlation_id(1, 0));
    c.members[1].node.handle().send_envelope(again).unwrap();
    c.run_for(Duration::from_millis(100)).await;
    assert_eq!(
        c.members[0].node.journal(&p).unwrap().len(),
        before,
        "the caller's journal records the result once"
    );
}

// @tg: verifies:: [[openspec:worker-pools#Worker Failure Reporting#Worker node crashes]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn worker_node_crashes() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(&mut c, ask_type(), &[(1, hang(calls.clone()))]);
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(c.until(T, |_| calls.load(Ordering::SeqCst) == 1).await);
    // n1 crashes while handling the request; n0 learns it from failure detection
    c.wire.set_down(&c.members[1].id, true);
    c.advance_only(Duration::from_secs(12), &[0]);
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p)
            == ["failed"])
            .await,
        "the caller received an error event"
    );
    let err = inputs(&c.members[0].node, &p).pop().unwrap();
    assert_eq!(err.name, "error.communication");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "not retried: the type is not idempotent"
    );
}

// @tg: verifies:: [[openspec:worker-pools#Worker Failure Reporting#Idempotent worker is retried]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn idempotent_worker_is_retried() {
    let mut c = Cluster::new(3, 8);
    let hung = Arc::new(AtomicUsize::new(0));
    let served = Arc::new(AtomicUsize::new(0));
    workers(
        &mut c,
        ask_type().idempotent(),
        &[(1, hang(hung.clone())), (2, echo("n2", served.clone()))],
    );
    let p = c.actor_on("caller", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(c.until(T, |_| hung.load(Ordering::SeqCst) == 1).await);
    c.wire.set_down(&c.members[1].id, true);
    c.advance_only(Duration::from_secs(12), &[0, 2]);
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p) == ["done"])
            .await,
        "retried once on another activation"
    );
    assert_eq!(served.load(Ordering::SeqCst), 1);
    assert_eq!(hung.load(Ordering::SeqCst), 1);
}

/// The caller chart with a request timeout of `timeout`.
fn quick_caller_chart(timeout: &str) -> vertep_core::ir::Chart {
    let w = worker_addr();
    scxml(
        "null",
        "idle",
        &format!(
            r#"<state id="idle"><transition event="ask" target="asking"/></state>
               <state id="asking"><vt:request target="{w}" event="work" timeout="{timeout}" reply="done" ontimeout="timedout"/>
                 <transition event="error.communication" target="failed"/></state>
               <state id="done"/><state id="timedout"/><state id="failed"/>"#
        ),
    )
}

// @tg: verifies:: [[openspec:worker-pools#Worker Concurrency Limits#Limit reached]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn a_timed_out_request_frees_its_worker_capacity() {
    let config = ClusterConfig {
        delivery_deadline: Duration::from_millis(300),
        retry_backoff: Duration::from_millis(50),
        ..ClusterConfig::default()
    };
    let mut c = Cluster::with_config(2, 8, config);
    let calls = Arc::new(AtomicUsize::new(0));
    // one activation with one request at a time
    workers(&mut c, ask_type(), &[(1, echo("n1", calls.clone()))]);
    c.register("quick", &quick_caller_chart("500ms"));
    // the worker's reply to p is lost for good: the link back is cut past the delivery deadline
    c.wire.set_cut(&c.members[1].id, &c.members[0].id, true);
    let p = c.actor_on("quick", 0, "p");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &p)
            == ["timedout"])
            .await,
        "the caller's request timed out"
    );
    c.run_for(Duration::from_millis(500)).await;
    c.wire.set_cut(&c.members[1].id, &c.members[0].id, false);
    // the timeout freed the activation's capacity (the reply that would have freed it never
    // comes): the next request is served
    let q = c.actor_on("quick", 0, "q");
    c.members[0].node.send(&q, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &q) == ["done"])
            .await,
        "the next request got the activation"
    );
    // two work requests (the third input was the error about its own undeliverable reply)
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_pool_queue_refuses_at_once() {
    let mut c = Cluster::new(2, 8);
    let calls = Arc::new(AtomicUsize::new(0));
    workers(
        &mut c,
        WorkerType::new(worker_addr().as_str())
            .with_limits(1, 1, 0)
            .with_policy(Policy::LeastOutstanding),
        &[(1, hang(calls.clone()))],
    );
    let p = c.actor_on("caller", 0, "p");
    let q = c.actor_on("caller", 0, "q");
    c.members[0].node.send(&p, ev("ask")).unwrap();
    assert!(c.until(T, |_| calls.load(Ordering::SeqCst) == 1).await);
    c.members[0].node.send(&q, ev("ask")).unwrap();
    assert!(
        c.until(T, |c| configuration(&mut c.members[0].node, &q)
            == ["failed"])
            .await,
        "the caller received an error event at once"
    );
}

// @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Fenced outbound messages are dropped]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn fenced_frames_land_in_the_runtime_dead_letter_graph() {
    use vertep_cluster::frame::RejectKind;
    use vertep_cluster::{Fence, Frame, Reply};
    let mut c = Cluster::new(2, 8);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 1, "x");
    let sender = c.actor_on("counter", 0, "s");
    let shard = c.shard(&sender);
    let frame = |id: &str, epoch: u64| {
        let mut e = ev("inc");
        e.origin = Some(sender.as_str().to_owned());
        let env = Envelope::new(id, &x, e);
        Frame {
            id: id.into(),
            sender: sender.as_str().into(),
            target: x.as_str().into(),
            fence: Some(Fence { shard, epoch }),
            hops: 0,
            event: "inc".into(),
            data: serde_json::to_vec(&env).unwrap(),
        }
    };
    let n1 = c.members[1].cluster.clone();
    let from = c.members[0].id.clone();
    // the receiver has seen epoch 9 of the sender's shard; a frame under epoch 3 is stale
    assert_eq!(
        n1.receive_checked(from.clone(), frame("m-new", 9)).await,
        Reply::Ack { id: "m-new".into() }
    );
    let stale = n1.receive_checked(from, frame("m-old", 3)).await;
    assert!(matches!(
        stale,
        Reply::Rejected {
            kind: RejectKind::Fenced,
            ..
        }
    ));
    assert!(c.until(T, |c| count(&c.members[1].node, &x) == 1).await);
    let dead = c.members[1].node.dead_letters(&x).unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].reason, "fenced");
    assert_eq!(dead[0].event, "inc");
    assert_eq!(dead[0].sender.as_deref(), Some(sender.as_str()));
}

// @tg: verifies:: [[openspec:cluster-membership#Graceful Leave With Drain#Leaving node hands over its shards]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn a_leaving_node_ships_its_shard_journals() {
    use vertep_cluster::runtime::NodeDrain;
    let mut c = Cluster::new(3, 8);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 1, "x");
    let shard = c.shard(&x);
    for _ in 0..3 {
        c.members[0].node.send(&x, ev("inc")).unwrap();
    }
    assert!(c.until(T, |c| count(&c.members[1].node, &x) == 3).await);
    // n1 leaves: each shard's local journal goes to its successor before the lease does
    let stores: Vec<Vec<Arc<MemoryStore>>> = c.members.iter().map(|m| m.stores.clone()).collect();
    let ids: Vec<NodeId> = c.members.iter().map(|m| m.id.clone()).collect();
    let membership = c.members[1].membership.clone();
    let meta = c.meta.clone();
    let mut drain = NodeDrain::new(&mut c.members[1].node, meta).with_shipping(|s, to| {
        let j = ids.iter().position(|n| n == to).unwrap();
        vertep_cluster::ship_journal(stores[1][s.index()].as_ref(), stores[j][s.index()].as_ref())
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    membership.leave(&mut drain).unwrap();
    drop(drain);
    c.wire.set_down(&c.members[1].id, true);
    c.advance_only(Duration::from_millis(100), &[0, 2]);
    for m in [0, 2] {
        c.members[m].cluster.refresh();
    }
    let to = c
        .members
        .iter()
        .position(|m| m.membership.held(shard).is_some())
        .expect("a successor holds the shard");
    assert_ne!(to, 1);
    c.members[0].node.send(&x, ev("inc")).unwrap();
    assert!(
        c.until(T, |c| count(&c.members[to].node, &x) == 4).await,
        "the actor resumed on its successor from its three entries"
    );
}

// @tg: verifies:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication#Lost acknowledgement causes a duplicate that is discarded]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn a_restarted_sender_sends_again_what_was_not_acknowledged() {
    let mut c = Cluster::new(2, 8);
    let chart = scxml("ecmascript", "s", COUNTER);
    c.register("counter", &chart);
    let x = c.actor_on("counter", 1, "x");
    // frames from n0 to n1 are lost: the message is handed off, never acknowledged
    c.wire.set_cut(&c.members[0].id, &c.members[1].id, true);
    c.members[0].node.send(&x, ev("inc")).unwrap();
    c.run_for(Duration::from_millis(100)).await;
    // n0 crashes before the acknowledgement and starts again over its storage
    c.restart(0);
    c.members[0]
        .node
        .register_type(&ty("counter"), chart)
        .unwrap();
    c.run_for(Duration::from_millis(300)).await;
    assert_eq!(count(&c.members[1].node, &x), 0, "still cut off");
    c.wire.set_cut(&c.members[0].id, &c.members[1].id, false);
    assert!(
        c.until(T, |c| count(&c.members[1].node, &x) == 1).await,
        "the message reached its actor after the sender's restart"
    );
    c.run_for(Duration::from_millis(200)).await;
    assert_eq!(count(&c.members[1].node, &x), 1, "once");
}

#[tokio::test(flavor = "current_thread")]
async fn draining_a_shard_deactivates_only_its_actors() {
    use vertep_cluster::Drain;
    use vertep_cluster::runtime::NodeDrain;
    let mut c = Cluster::new(2, 8);
    c.register("counter", &scxml("ecmascript", "s", COUNTER));
    let x = c.actor_on("counter", 0, "x");
    let y = (0..)
        .map(|k| va("counter", &format!("y{k}")))
        .find(|a| c.owner_of(a) == 0 && c.shard(a) != c.shard(&x))
        .unwrap();
    c.members[0].node.send(&x, ev("inc")).unwrap();
    c.members[0].node.send(&y, ev("inc")).unwrap();
    assert!(
        c.until(T, |c| count(&c.members[0].node, &x) == 1
            && count(&c.members[0].node, &y) == 1)
            .await
    );
    assert!(c.members[0].node.is_active(&x) && c.members[0].node.is_active(&y));
    let meta = c.meta.clone();
    let shard = c.shard(&x);
    NodeDrain::new(&mut c.members[0].node, meta)
        .drain_shard(shard)
        .unwrap();
    assert!(
        !c.members[0].node.is_active(&x),
        "the drained shard's actor"
    );
    assert!(
        c.members[0].node.is_active(&y),
        "another shard's actor keeps running"
    );
}
