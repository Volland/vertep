//! Jepsen-style fault injection with every node a process in its own container.
//!
//! Four `vertep-fault-node` containers (the binary of feature `fault-node`, built for Linux in
//! the pinned `rust` image into a cached volume) form a cluster on etcd or on the Kubernetes
//! Lease API (k3s), each also in a container on a private network. The world (client
//! messages, the fenced journal store, the side-effect receivers and the history) runs in this
//! test process; the nodes reach it on the host. While they work, the harness injects at the
//! container level:
//!
//! - network partitions from the lease authority (`docker network disconnect`),
//! - process pauses (`docker pause`: the process is frozen wherever it is, possibly between
//!   reading a journal head and appending),
//! - crashes and restarts (`docker kill`, `docker start`: a fresh process, a new client, a
//!   rejoin once the old incarnation is failed),
//! - one-way loss of a node's side-effect datagrams (`tc netem` when the kernel has it; the
//!   Docker Desktop kernel has neither netem nor the iptables `statistic` match, so then the
//!   world's receiver drops that node's datagrams),
//! - clock skew beyond the safety margin (one node's lease clock runs at half speed:
//!   containers share the host kernel's clock, so the skew is applied to the node's clock).
//!
//! The history is checked as in `faults.rs` (journals linear, one writer per epoch, every
//! acknowledged message processed once, no fenced side effect admitted) and replayed through
//! the Lean lease model in strict mode. The test skips unless `VERTEP_CONTAINER_TESTS` is set;
//! `VERTEP_FAULT_SECONDS` sets how long faults are injected (default 60).
#![cfg(unix)]

mod common;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::containers::{self, Container, ETCD_IMAGE, K3S_IMAGE, Network};
use common::oracle::{lease_oracle, verify};
use common::{actor_in, node};
use serde_json::{Value, json};
use vertep_cluster::{
    AppendDecision, ClusterId, ClusterMeta, EpochFilter, Fence, FrameDecision, History,
    HistoryEvent, NodeId, ShardId,
};
use vertep_store::oxrdf::NamedNode;
use vertep_store::testkit::sample_entry;
use vertep_store::{AppendError, AppendMeta, FencingToken, JournalStore, MemoryStore, Position};

/// The image the node binary is built in, pinned.
const BUILD_IMAGE: &str = "rust:1.95.0-slim-bookworm";
/// The image the nodes run in (the binary comes from the build volume), pinned.
const NODE_IMAGE: &str = "debian:bookworm-slim";
const TARGET_VOLUME: &str = "vertep-fault-target";
const REGISTRY_VOLUME: &str = "vertep-fault-cargo";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }
}

/// Builds `vertep-fault-node` for Linux into the target volume.
fn build_node() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let src = format!("{}:/src:ro", workspace.display());
    let started = Instant::now();
    containers::docker_run(&[
        "run",
        "--rm",
        "--label",
        "vertep-test",
        "-v",
        &src,
        "-v",
        &format!("{REGISTRY_VOLUME}:/usr/local/cargo/registry"),
        "-v",
        &format!("{TARGET_VOLUME}:/target"),
        "-w",
        "/src",
        "-e",
        "CARGO_TARGET_DIR=/target",
        // the image's toolchain, without the extra targets rust-toolchain.toml asks for
        "-e",
        "RUSTUP_TOOLCHAIN=1.95.0",
        BUILD_IMAGE,
        "cargo",
        "build",
        "--release",
        "-p",
        "vertep-cluster",
        "--no-default-features",
        "--features",
        "fault-node",
        "--bin",
        "vertep-fault-node",
    ])
    .unwrap_or_else(|e| panic!("building vertep-fault-node failed:\n{e}"));
    eprintln!("built vertep-fault-node in {:?}", started.elapsed());
}

// ---------------------------------------------------------------------------------------------
// The world: client messages, the fenced journal store, the receivers and the history

struct WorldState {
    meta: ClusterMeta,
    store: MemoryStore,
    actors: Vec<(ShardId, NamedNode)>,
    pending: VecDeque<(String, usize)>,
    receivers: Vec<(NodeId, EpochFilter)>,
    history: History,
    granted: BTreeSet<(NodeId, ShardId, u64)>,
    rng: Rng,
    next: u64,
    /// Nodes whose side-effect datagrams are dropped (one-way loss).
    lossy: BTreeSet<String>,
}

impl WorldState {
    fn processed(&self, actor: usize, message: &str) -> bool {
        let suffix = format!("/entry/{message}");
        self.store
            .read(&self.actors[actor].1, Position::ZERO)
            .unwrap()
            .iter()
            .any(|r| r.entry.name().as_str().ends_with(&suffix))
    }

    fn ack(&mut self, message: &str) {
        self.pending.retain(|(m, _)| m != message);
        self.history.record(HistoryEvent::Ack {
            message: message.to_owned(),
        });
    }

    fn handle(&mut self, req: &Value) -> Value {
        let node_of = |v: &Value| NodeId::new(v["node"].as_str().unwrap_or("")).ok();
        match req["op"].as_str() {
            Some("grant") => {
                let (Some(n), Some(s), Some(e)) =
                    (node_of(req), req["shard"].as_u64(), req["epoch"].as_u64())
                else {
                    return json!({"error": "bad grant"});
                };
                let shard = ShardId::new(s as u32);
                if self.granted.insert((n.clone(), shard, e)) {
                    self.history.record(HistoryEvent::Grant {
                        node: n,
                        shard,
                        epoch: e,
                    });
                }
                json!({"ok": true})
            }
            Some("work") => {
                let held: BTreeMap<u32, u64> = req["held"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| Some((p[0].as_u64()? as u32, p[1].as_u64()?)))
                            .collect()
                    })
                    .unwrap_or_default();
                loop {
                    let found = self
                        .pending
                        .iter()
                        .find(|(_, a)| held.contains_key(&self.actors[*a].0.get()))
                        .cloned();
                    let Some((message, actor)) = found else {
                        break;
                    };
                    if self.processed(actor, &message) {
                        // a retried message already in the journal: acknowledge, don't redo
                        self.ack(&message);
                        continue;
                    }
                    let (shard, iri) = &self.actors[actor];
                    return json!({
                        "message": message,
                        "actor": iri.as_str(),
                        "shard": shard.get(),
                        "epoch": held[&shard.get()],
                        "head": self.store.head(iri).unwrap().get(),
                    });
                }
                if self.pending.is_empty() {
                    return json!({});
                }
                let k = self.rng.below(self.pending.len() as u64) as usize;
                json!({"acquire": self.actors[self.pending[k].1].0.get()})
            }
            Some("append") => self.append(req),
            _ => json!({"error": "unknown op"}),
        }
    }

    fn append(&mut self, req: &Value) -> Value {
        let (Some(n), Some(s), Some(e), Some(actor), Some(head), Some(message)) = (
            NodeId::new(req["node"].as_str().unwrap_or("")).ok(),
            req["shard"].as_u64(),
            req["epoch"].as_u64(),
            req["actor"].as_str(),
            req["head"].as_u64(),
            req["message"].as_str(),
        ) else {
            return json!({"error": "bad append"});
        };
        let shard = ShardId::new(s as u32);
        let iri = NamedNode::new_unchecked(actor);
        let meta =
            AppendMeta::none().with_fencing(FencingToken::new(self.meta.shard_scope(shard), e));
        let decision = match self.store.append(
            &iri,
            Position::new(head),
            &[sample_entry(&iri, message)],
            &meta,
        ) {
            Ok(_) => AppendDecision::Accept,
            Err(AppendError::Fenced { .. }) => AppendDecision::Fenced,
            Err(AppendError::Conflict { .. } | AppendError::Invalid(_)) => AppendDecision::Conflict,
            Err(e) => return json!({"error": e.to_string()}),
        };
        self.history.record(HistoryEvent::Append {
            node: n,
            shard,
            epoch: e,
            actor: actor.to_owned(),
            version: head + 1,
            message: message.to_owned(),
            decision,
        });
        // the acknowledgement to the client is lost now and then: the client retries
        if decision == AppendDecision::Accept && !self.rng.chance(200) {
            self.ack(message);
        }
        json!({"decision": match decision {
            AppendDecision::Accept => "accept",
            AppendDecision::Fenced => "fenced",
            AppendDecision::Conflict => "conflict",
        }})
    }

    fn frame(&mut self, f: &Value) {
        let (Some(r), Some(s), Some(e), Some(message), Some(from)) = (
            f["receiver"].as_u64(),
            f["shard"].as_u64(),
            f["epoch"].as_u64(),
            f["message"].as_str(),
            f["node"].as_str(),
        ) else {
            return;
        };
        if self.lossy.contains(from) && self.rng.chance(700) {
            return; // lost one way
        }
        let (id, filter) = &self.receivers[r as usize % self.receivers.len()];
        let fence = Fence {
            shard: ShardId::new(s as u32),
            epoch: e,
        };
        let decision = match filter.admit(fence) {
            Ok(()) => FrameDecision::Admit,
            Err(_) => FrameDecision::Fenced,
        };
        self.history.record(HistoryEvent::Frame {
            receiver: id.clone(),
            shard: fence.shard,
            epoch: e,
            message: message.to_owned(),
            decision,
        });
    }
}

/// The world served on the host: JSON lines over TCP, side effects as UDP datagrams.
struct World {
    state: Arc<Mutex<WorldState>>,
    port: u16,
    stop: Arc<AtomicBool>,
}

impl World {
    fn start(meta: ClusterMeta, seed: u64) -> World {
        // Docker Desktop forwards host.docker.internal to the host's loopback; on Linux the
        // host gateway reaches only the host's other addresses
        let host = if cfg!(target_os = "linux") {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let tcp = TcpListener::bind((host, 0)).unwrap();
        let port = tcp.local_addr().unwrap().port();
        let udp = UdpSocket::bind((host, port)).unwrap();
        udp.set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        tcp.set_nonblocking(true).unwrap();
        let actors = meta
            .shard_count
            .shards()
            .flat_map(|s| {
                [
                    (s, actor_in(&meta, s, "c-a")),
                    (s, actor_in(&meta, s, "c-b")),
                ]
            })
            .collect();
        let state = Arc::new(Mutex::new(WorldState {
            meta,
            store: MemoryStore::new(),
            actors,
            pending: VecDeque::new(),
            receivers: vec![
                (node("r0"), EpochFilter::new()),
                (node("r1"), EpochFilter::new()),
            ],
            history: History::new(),
            granted: BTreeSet::new(),
            rng: Rng(seed | 1),
            next: 0,
            lossy: BTreeSet::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                match tcp.accept() {
                    Ok((conn, _)) => {
                        let st = st.clone();
                        std::thread::spawn(move || {
                            let _ = conn.set_nonblocking(false);
                            let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
                            let mut line = String::new();
                            let mut r = BufReader::new(&conn);
                            if r.read_line(&mut line).is_err() {
                                return;
                            }
                            let Ok(req) = serde_json::from_str::<Value>(&line) else {
                                return;
                            };
                            let resp = lock(&st).handle(&req);
                            let _ = writeln!(&conn, "{resp}");
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        let (st, sp) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !sp.load(Ordering::SeqCst) {
                if let Ok((n, _)) = udp.recv_from(&mut buf)
                    && let Ok(f) = serde_json::from_slice::<Value>(&buf[..n])
                {
                    lock(&st).frame(&f);
                }
            }
        });
        // clients: a message every 150 ms or so
        let (st, sp) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(150));
                let mut w = lock(&st);
                if w.rng.chance(600) && w.pending.len() < 200 {
                    let n = w.actors.len() as u64;
                    let actor = w.rng.below(n) as usize;
                    let m = format!("m{}", w.next);
                    w.next += 1;
                    w.pending.push_back((m, actor));
                }
            }
        });
        World { state, port, stop }
    }

    fn set_lossy(&self, node: &str, lossy: bool) {
        let mut w = lock(&self.state);
        if lossy {
            w.lossy.insert(node.to_owned());
        } else {
            w.lossy.remove(node);
        }
    }

    fn finish(self) -> History {
        self.stop.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(400));
        std::mem::take(&mut lock(&self.state).history)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------
// The cluster in containers, and its faults

/// The lease authority container and how nodes reach it.
struct Service {
    _container: Container,
    env: Vec<String>,
}

fn etcd_on(net: &Network) -> Service {
    let c = Container::start(
        "vt-fetcd",
        ETCD_IMAGE,
        &["--network", &net.name],
        &[
            "etcd",
            "--listen-client-urls",
            "http://0.0.0.0:2379",
            "--advertise-client-urls",
            "http://127.0.0.1:2379",
            "--heartbeat-interval",
            "50",
            "--election-timeout",
            "500",
        ],
    );
    c.wait("healthy", Duration::from_secs(60), |c| {
        c.exec(&["etcdctl", "endpoint", "health"]).is_some()
    });
    let env = vec![
        "VT_MEMBERSHIP=etcd".to_owned(),
        format!("VT_ETCD=http://{}:2379", c.name),
    ];
    Service { _container: c, env }
}

fn k3s_on(net: &Network) -> Service {
    let name = containers::unique("vt-fk3s");
    let san = format!("--tls-san={name}");
    let c = Container::start(
        "vt-fk3s",
        K3S_IMAGE,
        &[
            "--network",
            &net.name,
            "--hostname",
            &name,
            "--network-alias",
            &name,
        ],
        &[
            "server",
            "--disable-agent",
            "--disable=traefik,servicelb,metrics-server,local-storage,coredns",
            &san,
        ],
    );
    c.wait("ready", Duration::from_secs(120), |c| {
        c.exec(&["kubectl", "get", "--raw", "/readyz"])
            .is_some_and(|o| o.trim() == "ok")
            && c.exec(&[
                "kubectl",
                "get",
                "serviceaccount",
                "default",
                "-n",
                "default",
            ])
            .is_some()
    });
    let kubeconfig = c
        .exec(&["cat", "/etc/rancher/k3s/k3s.yaml"])
        .unwrap()
        .replace("https://127.0.0.1:6443", &format!("https://{name}:6443"));
    let env = vec![
        "VT_MEMBERSHIP=kubernetes".to_owned(),
        format!("VT_KUBECONFIG_TEXT={kubeconfig}"),
    ];
    Service { _container: c, env }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    Partitioned,
    Paused,
    Crashed,
    Lossy,
}

/// The count of each fault injected.
#[derive(Debug, Default)]
struct Injected {
    partitions: u32,
    pauses: u32,
    crashes: u32,
    losses: u32,
}

fn run_cluster(label: &str, start: fn(&Network) -> Service, seed: u64) {
    let lease = lease_oracle();
    let seconds: u64 = std::env::var("VERTEP_FAULT_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let auth = Network::create("vt-auth");
    let data = Network::create("vt-data");
    let service = start(&auth);
    let cluster = ClusterId::new(containers::unique("cf")).unwrap();
    let meta = ClusterMeta::new(cluster.clone(), 8).unwrap();
    let world = World::start(meta, seed);
    let target = format!("{TARGET_VOLUME}:/target:ro");
    let nodes: Vec<(String, Container)> = (0..4)
        .map(|i| {
            let id = format!("n{i}");
            let mut env = vec![
                format!("VT_NODE={id}"),
                format!("VT_CLUSTER={cluster}"),
                "VT_SHARDS=8".to_owned(),
                format!("VT_WORLD=host.docker.internal:{}", world.port),
                // the last node's lease clock runs at half speed: skew beyond the margin
                format!("VT_CLOCK_RATE={}", if i == 3 { 50 } else { 100 }),
            ];
            env.extend(service.env.iter().cloned());
            let mut args = vec![
                "--network".to_owned(),
                data.name.clone(),
                "--add-host=host.docker.internal:host-gateway".to_owned(),
                "--cap-add=NET_ADMIN".to_owned(),
                "-v".to_owned(),
                target.clone(),
            ];
            for e in &env {
                args.push("-e".into());
                args.push(e.clone());
            }
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let c = Container::start_kept(
                &format!("vt-{id}"),
                NODE_IMAGE,
                &args,
                &["/target/release/vertep-fault-node"],
            );
            assert!(auth.connect(&c), "{id} reaches the lease authority");
            (id, c)
        })
        .collect();
    // tc netem for one-way loss, where the kernel has it
    let netem = nodes[0].1.exec(&["sh", "-c", "command -v tc"]).is_some();
    let mut rng = Rng(seed | 1);
    let mut faults = vec![(Fault::None, Instant::now()); nodes.len()];
    let mut injected = Injected::default();
    let end = Instant::now() + Duration::from_secs(seconds);
    // let the cluster form before the faults start
    std::thread::sleep(Duration::from_secs(5));
    let heal = |i: usize, f: Fault| match f {
        Fault::Partitioned => {
            auth.connect(&nodes[i].1);
        }
        Fault::Paused => {
            nodes[i].1.docker(&["unpause"]);
        }
        Fault::Crashed => {
            nodes[i].1.docker(&["start"]);
        }
        Fault::Lossy => {
            if netem {
                nodes[i]
                    .1
                    .exec(&["tc", "qdisc", "del", "dev", "eth0", "root"]);
            }
            world.set_lossy(&nodes[i].0, false);
        }
        Fault::None => {}
    };
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(400));
        for i in 0..nodes.len() {
            let (f, until) = faults[i];
            if f != Fault::None {
                if Instant::now() >= until {
                    heal(i, f);
                    faults[i] = (Fault::None, Instant::now());
                }
                continue;
            }
            let span = Duration::from_millis(1_500 + rng.below(6_000));
            let c = &nodes[i].1;
            let next = if rng.chance(25) {
                auth.disconnect(c);
                injected.partitions += 1;
                Fault::Partitioned
            } else if rng.chance(20) {
                c.docker(&["pause"]);
                injected.pauses += 1;
                Fault::Paused
            } else if rng.chance(12) {
                c.docker(&["kill"]);
                injected.crashes += 1;
                Fault::Crashed
            } else if rng.chance(20) {
                let tc = netem
                    && c.exec(&[
                        "tc", "qdisc", "add", "dev", "eth0", "root", "netem", "loss", "70%",
                    ])
                    .is_some();
                if !tc {
                    world.set_lossy(&nodes[i].0, true);
                }
                injected.losses += 1;
                Fault::Lossy
            } else {
                Fault::None
            };
            faults[i] = (next, Instant::now() + span);
        }
    }
    for (i, (f, _)) in faults.iter().enumerate() {
        heal(i, *f);
    }
    // a calm period, then stop every node before reading the history
    std::thread::sleep(Duration::from_secs(6));
    for (_, c) in &nodes {
        c.docker(&["kill"]);
    }
    let h = world.finish();
    let r = verify(&format!("containers-{label}-{seed}"), &h, lease.as_deref());
    eprintln!("{label}: {injected:?}, {r:?}");
    assert!(
        injected.partitions > 0 && injected.pauses > 0 && injected.crashes > 0,
        "{label}: every kind of fault was injected: {injected:?}"
    );
    assert!(r.accepted > 50, "{label}: progress {r:?}");
    assert!(r.acked > 50, "{label}: acknowledged {r:?}");
    drop(nodes);
    drop(service);
}

// @tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}
#[test]
fn container_faults_on_etcd_and_kubernetes_membership() {
    if !containers::enabled("the container fault-injection harness") {
        return;
    }
    build_node();
    run_cluster("etcd", etcd_on, 7);
    run_cluster("kubernetes", k3s_on, 11);
}
