//! `vertep-fault-node`: one node of the container fault-injection harness
//! (`crates/vertep-cluster/tests/containers.rs`).
//!
//! The node joins the cluster through etcd or the Kubernetes Lease API, leases shards, and
//! processes client messages held by the harness (the "world"): for a message of a shard it
//! holds it reads the actor's journal head, works for a while (a macrostep), appends under its
//! lease epoch, and on acceptance sends the side effect as a UDP datagram to one of the
//! world's receivers. It reports every lease it holds to the world before appending under it.
//! The harness pauses, kills, restarts and partitions the node's container; nothing here
//! defends against that beyond the lease protocol itself.
//!
//! Configuration, from the environment: `VT_NODE`, `VT_CLUSTER`, `VT_SHARDS`, `VT_MEMBERSHIP`
//! (`etcd` or `kubernetes`), `VT_ETCD` (the etcd URL), `VT_KUBECONFIG_TEXT` (a kubeconfig) or
//! `VT_KUBECONFIG` (a kubeconfig file),
//! `VT_WORLD` (the world's TCP address, also its UDP receiver address), `VT_CLOCK_RATE` (the
//! node's clock speed in percent, for clock skew), `VT_LEASE_MS`, `VT_MARGIN_MS`,
//! `VT_SUSPECT_MS`, `VT_FAIL_MS`, `VT_STEP_MS`, `VT_WORK_MS`.
//!
//! @tg: implements:: [[decisions#Container fault injection runs every node as a process]]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use vertep_cluster::external::etcd::{EtcdAuthority, EtcdConfig};
use vertep_cluster::external::kubernetes::{KubeAuthority, KubeConfig};
use vertep_cluster::{
    Authority, AuthorityConfig, AuthorityMembership, Clock, ClusterId, ClusterMeta, DetectorConfig,
    LeaseConfig, Membership, Millis, MonotonicClock, NodeId, ShardId,
};

/// A monotonic clock running at `rate` percent: clock skew that grows.
#[derive(Debug)]
struct RateClock {
    base: MonotonicClock,
    rate: u64,
}

impl Clock for RateClock {
    fn now(&self) -> Millis {
        Millis(self.base.now().0 * self.rate / 100)
    }
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn env_ms(name: &str, default: u64) -> Duration {
    Duration::from_millis(
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

/// One request to the world: a JSON line out, a JSON line back.
fn call(world: &str, request: &Value) -> Result<Value, String> {
    let mut s = TcpStream::connect(world).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    writeln!(s, "{request}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

struct Rng(u64);

impl Rng {
    fn chance(&mut self, percent: u64) -> bool {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x % 100 < percent
    }
}

fn run<A: Authority>(
    authority: A,
    meta: ClusterMeta,
    timing: AuthorityConfig,
    clock: Arc<RateClock>,
) {
    let id = NodeId::new(env("VT_NODE")).expect("a node identifier");
    let world = env("VT_WORLD");
    let step = env_ms("VT_STEP_MS", 100);
    let work = env_ms("VT_WORK_MS", 40);
    let udp = UdpSocket::bind("0.0.0.0:0").expect("a UDP socket");
    let m = AuthorityMembership::new(
        authority,
        meta.clone(),
        id.clone(),
        id.as_str(),
        clock,
        timing.lease,
    );
    // a restarted node is a duplicate until its previous incarnation is marked failed
    while let Err(e) = m.join() {
        eprintln!("{id}: join: {e}");
        std::thread::sleep(step);
    }
    eprintln!("{id}: joined");
    let mut rng = Rng(std::process::id() as u64 | 1);
    let mut reported: BTreeSet<(ShardId, u64)> = BTreeSet::new();
    loop {
        std::thread::sleep(step);
        let _ = m.tick();
        // report every lease held before working under it
        let mut held = Vec::new();
        for s in meta.shard_count.shards() {
            if let Some(l) = m.held(s) {
                if !reported.contains(&(s, l.epoch)) {
                    let r = json!({"op": "grant", "node": id.as_str(), "shard": s.get(), "epoch": l.epoch});
                    if call(&world, &r).is_ok() {
                        reported.insert((s, l.epoch));
                    }
                }
                if reported.contains(&(s, l.epoch)) {
                    held.push(json!([s.get(), l.epoch]));
                }
            }
        }
        let Ok(w) = call(
            &world,
            &json!({"op": "work", "node": id.as_str(), "held": held}),
        ) else {
            continue;
        };
        if let Some(shard) = w["acquire"].as_u64() {
            if rng.chance(40) {
                let _ = m.acquire(ShardId::new(shard as u32));
            }
        } else if let (Some(message), Some(shard), Some(epoch)) = (
            w["message"].as_str(),
            w["shard"].as_u64(),
            w["epoch"].as_u64(),
        ) {
            let shard = ShardId::new(shard as u32);
            // the macrostep: the head is read, the work takes a while, then the append
            std::thread::sleep(work);
            if m.held(shard).is_none_or(|l| l.epoch != epoch) {
                continue; // the lease ran out by this node's clock: stop
            }
            let append = json!({
                "op": "append", "node": id.as_str(), "shard": shard.get(), "epoch": epoch,
                "actor": w["actor"], "head": w["head"], "message": message,
            });
            match call(&world, &append) {
                Ok(r) if r["decision"] == "accept" => {
                    let frame = json!({
                        "receiver": u64::from(rng.chance(50)), "node": id.as_str(),
                        "shard": shard.get(), "epoch": epoch, "message": message,
                    });
                    let _ = udp.send_to(frame.to_string().as_bytes(), &world);
                }
                Ok(r) if r["decision"] == "fenced" => m.forget(shard),
                _ => {}
            }
        }
        if rng.chance(2) {
            let s = meta.shard_count.shards().find(|s| m.held(*s).is_some());
            if let Some(l) = s.and_then(|s| m.held(s)) {
                let _ = m.release(&l);
            }
        }
    }
}

fn main() {
    let cluster = ClusterId::new(env("VT_CLUSTER")).expect("a cluster identifier");
    let shards = env("VT_SHARDS").parse().expect("a shard count");
    let meta = ClusterMeta::new(cluster, shards).expect("cluster metadata");
    let timing = AuthorityConfig {
        lease: LeaseConfig {
            duration: env_ms("VT_LEASE_MS", 4_000),
            safety_margin: env_ms("VT_MARGIN_MS", 1_000),
        },
        detector: DetectorConfig {
            suspicion_time: env_ms("VT_SUSPECT_MS", 2_000),
            failure_time: env_ms("VT_FAIL_MS", 4_000),
        },
    };
    let rate = std::env::var("VT_CLOCK_RATE")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(100);
    let clock = Arc::new(RateClock {
        base: MonotonicClock::new(),
        rate,
    });
    // the lease authority's own observations run on true time, only the node's lease
    // judgement is skewed
    let real: Arc<dyn Clock> = Arc::new(MonotonicClock::new());
    match env("VT_MEMBERSHIP").as_str() {
        "etcd" => {
            let mut c = EtcdConfig::new(env("VT_ETCD"));
            c.timeout = Duration::from_millis(500);
            let a = EtcdAuthority::new(meta.clone(), timing, &c, real).expect("etcd client");
            run(a, meta, timing, clock);
        }
        "kubernetes" => {
            let text = std::env::var("VT_KUBECONFIG_TEXT").unwrap_or_else(|_| {
                std::fs::read_to_string(env("VT_KUBECONFIG")).expect("the kubeconfig")
            });
            let mut c = KubeConfig::from_kubeconfig(&text).expect("a kubeconfig");
            c.timeout = Duration::from_millis(500);
            let a = KubeAuthority::new(meta.clone(), timing, &c, real).expect("Kubernetes client");
            run(a, meta, timing, clock);
        }
        other => panic!("unknown membership {other}"),
    }
}
