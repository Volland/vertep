//! The etcd lease authority, over the etcd v3 JSON gateway (`/v3/kv/*`, `/v3/lease/*`).
//!
//! Key space of cluster `c` (values are JSON):
//!
//! | key | value | attached to |
//! |---|---|---|
//! | `/vertep/c/meta` | shard count and hash version, written by the first join | — |
//! | `/vertep/c/nodes/<n>` | endpoint, draining flag | — |
//! | `/vertep/c/alive/<n>` | empty | etcd lease with TTL = suspicion time |
//! | `/vertep/c/present/<n>` | empty | etcd lease with TTL = failure time |
//! | `/vertep/c/shards/<s>` | owner, epoch, etcd lease id, term, revoked flag | etcd lease with TTL = lease term |
//! | `/vertep/c/epochs/<s>` | highest epoch, its owner, released flag | — |
//!
//! A member is active while its `alive` key exists, suspect once only `present` is left, and
//! failed once both expired; heartbeats keep both etcd leases alive, and a heartbeat of a
//! suspect member puts a fresh `alive` key. A shard is leased while its `shards` key exists;
//! etcd deletes it when the holder stops renewing. Grants and handovers are transactions that
//! compare the `mod_revision` of the shard key and of the epoch key, so two nodes never both
//! win, and the epoch key (never deleted) makes every grant's epoch exceed every earlier one.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};

use super::http::Client;
use super::{Publisher, ceil_secs};
use crate::authority::{Authority, AuthorityConfig};
use crate::clock::{Clock, duration_ms};
use crate::lease::{Lease, LeaseError};
use crate::membership::{JoinError, JoinRequest, MembershipError, check_compatible};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;
use crate::view::{MemberStatus, ShardOwner, View};

/// Where the etcd JSON gateway is and how long to wait for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EtcdConfig {
    /// The client URL, `http://host:2379`.
    pub endpoint: String,
    /// Connect, read and write timeout of every request.
    pub timeout: Duration,
}

impl EtcdConfig {
    /// The gateway at `endpoint` with a 2 s timeout.
    pub fn new(endpoint: impl Into<String>) -> Self {
        EtcdConfig {
            endpoint: endpoint.into(),
            timeout: Duration::from_secs(2),
        }
    }
}

/// The prefix of every key of the cluster: `/vertep/<cluster>/`.
pub fn prefix(meta: &ClusterMeta) -> String {
    format!("/vertep/{}/", meta.cluster)
}

/// The key of a member's record (endpoint and draining flag).
pub fn member_key(meta: &ClusterMeta, node: &NodeId) -> String {
    format!("{}nodes/{node}", prefix(meta))
}

/// The key of a shard's lease record, attached to the holder's etcd lease.
pub fn shard_key(meta: &ClusterMeta, shard: ShardId) -> String {
    format!("{}shards/{}", prefix(meta), shard.get())
}

fn alive_key(meta: &ClusterMeta, node: &NodeId) -> String {
    format!("{}alive/{node}", prefix(meta))
}

fn present_key(meta: &ClusterMeta, node: &NodeId) -> String {
    format!("{}present/{node}", prefix(meta))
}

fn epoch_key(meta: &ClusterMeta, shard: ShardId) -> String {
    format!("{}epochs/{}", prefix(meta), shard.get())
}

fn meta_key(meta: &ClusterMeta) -> String {
    format!("{}meta", prefix(meta))
}

/// The key one past every key starting with `p` (etcd's prefix `range_end`).
fn prefix_end(p: &str) -> Vec<u8> {
    let mut b = p.as_bytes().to_vec();
    while let Some(last) = b.pop() {
        if last < 0xff {
            b.push(last + 1);
            return b;
        }
    }
    vec![0]
}

fn int(v: &Value) -> i64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    }
}

/// One key-value pair of a range response.
#[derive(Clone, Debug)]
struct Kv {
    key: String,
    value: Value,
    mod_revision: i64,
}

impl Kv {
    fn parse(v: &Value) -> Option<Kv> {
        let key = B64.decode(v["key"].as_str()?).ok()?;
        let value = v["value"]
            .as_str()
            .and_then(|s| B64.decode(s).ok())
            .unwrap_or_default();
        Some(Kv {
            key: String::from_utf8(key).ok()?,
            value: serde_json::from_slice(&value).unwrap_or(Value::Null),
            mod_revision: int(&v["mod_revision"]),
        })
    }
}

fn kvs(v: &Value) -> Vec<Kv> {
    v["kvs"]
        .as_array()
        .map(|a| a.iter().filter_map(Kv::parse).collect())
        .unwrap_or_default()
}

/// A shard's lease record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ShardRecord {
    owner: NodeId,
    epoch: u64,
    lease: i64,
    term_ms: u64,
    revoked: bool,
}

impl ShardRecord {
    fn to_json(&self) -> Value {
        json!({
            "owner": self.owner.as_str(),
            "epoch": self.epoch,
            "lease": self.lease.to_string(),
            "term_ms": self.term_ms,
            "revoked": self.revoked,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        Some(ShardRecord {
            owner: NodeId::new(v["owner"].as_str()?).ok()?,
            epoch: v["epoch"].as_u64()?,
            lease: int(&v["lease"]),
            term_ms: v["term_ms"].as_u64()?,
            revoked: v["revoked"].as_bool().unwrap_or(false),
        })
    }
}

fn put(key: &str, value: &Value, lease: i64) -> Value {
    let mut p = json!({ "key": B64.encode(key), "value": B64.encode(value.to_string()) });
    if lease != 0 {
        p["lease"] = Value::String(lease.to_string());
    }
    json!({ "request_put": p })
}

fn delete(key: &str) -> Value {
    json!({ "request_delete_range": { "key": B64.encode(key) } })
}

fn get(key: &str) -> Value {
    json!({ "request_range": { "key": B64.encode(key) } })
}

/// The key's `mod_revision` equals `rev` (0: the key does not exist).
fn unchanged(key: &str, rev: i64) -> Value {
    json!({
        "key": B64.encode(key),
        "target": "MOD",
        "result": "EQUAL",
        "mod_revision": rev.to_string(),
    })
}

/// A shard's lease key, its epoch key, and a node's status, read together.
type ShardRead = (Option<Kv>, Option<Kv>, Option<MemberStatus>);

/// The etcd lease ids of a member this client joined.
#[derive(Clone, Copy, Debug)]
struct MemberLeases {
    alive: i64,
    present: i64,
}

#[derive(Debug, Default)]
struct State {
    members: BTreeMap<NodeId, MemberLeases>,
    publisher: Publisher,
}

/// What a read of one member's keys shows.
struct MemberKeys {
    record: Option<Kv>,
    alive: bool,
    present: bool,
}

impl MemberKeys {
    fn status(&self) -> Option<MemberStatus> {
        let record = self.record.as_ref()?;
        Some(if !self.present {
            MemberStatus::Failed
        } else if record.value["draining"].as_bool().unwrap_or(false) {
            MemberStatus::Draining
        } else if self.alive {
            MemberStatus::Active
        } else {
            MemberStatus::Suspect
        })
    }
}

/// etcd as the lease authority and member registry of one cluster, through its v3 JSON
/// gateway. One instance serves the nodes of one process; every node of the cluster may run
/// its own instance against the same etcd.
// @tg: implements:: [[openspec:cluster-membership#External Membership]]
// @tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]
// @tg: implements:: [[openspec:cluster-membership#Failure Detection]]
#[derive(Debug)]
pub struct EtcdAuthority {
    meta: ClusterMeta,
    config: AuthorityConfig,
    client: Client,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

impl EtcdAuthority {
    /// The authority of the cluster `meta` in the etcd at `etcd`, with lease and failure
    /// detection timing `config` (rounded up to whole seconds, etcd's TTL unit), timing local
    /// deadlines on `clock`.
    ///
    /// # Errors
    ///
    /// A malformed endpoint URL.
    pub fn new(
        meta: ClusterMeta,
        config: AuthorityConfig,
        etcd: &EtcdConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, String> {
        Ok(EtcdAuthority {
            meta,
            config,
            client: Client::new(&etcd.endpoint, etcd.timeout)?,
            clock,
            state: Mutex::new(State::default()),
        })
    }

    /// The cluster metadata.
    pub fn meta(&self) -> &ClusterMeta {
        &self.meta
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn call(&self, path: &str, body: &Value) -> Result<Value, String> {
        let r = self.client.request("POST", path, Some(body))?;
        let v = r.json()?;
        if r.ok() {
            Ok(v)
        } else {
            Err(format!(
                "etcd {path}: HTTP {}: {}",
                r.status,
                v["message"].as_str().unwrap_or("")
            ))
        }
    }

    /// Reads `keys` in one transaction: the store revision and each key's pair.
    fn read(&self, keys: &[String]) -> Result<(u64, Vec<Option<Kv>>), String> {
        let ops: Vec<Value> = keys.iter().map(|k| get(k)).collect();
        let v = self.call("/v3/kv/txn", &json!({ "success": ops }))?;
        let revision = int(&v["header"]["revision"]).max(0) as u64;
        let mut out = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let r = &v["responses"][i]["response_range"];
            out.push(kvs(r).into_iter().find(|kv| &kv.key == k));
        }
        Ok((revision, out))
    }

    /// Runs a transaction; true when every comparison held and `success` ran.
    fn txn(&self, compare: Vec<Value>, success: Vec<Value>) -> Result<bool, String> {
        let v = self.call(
            "/v3/kv/txn",
            &json!({ "compare": compare, "success": success }),
        )?;
        Ok(v["succeeded"].as_bool().unwrap_or(false))
    }

    /// Grants an etcd lease of `ttl` seconds: its id and granted TTL.
    fn grant(&self, ttl: u64) -> Result<(i64, u64), String> {
        let v = self.call("/v3/lease/grant", &json!({ "TTL": ttl.to_string() }))?;
        let id = int(&v["ID"]);
        if id == 0 {
            return Err(format!("etcd granted no lease: {v}"));
        }
        Ok((id, int(&v["TTL"]).max(1) as u64))
    }

    /// Keeps etcd lease `id` alive: its renewed TTL, or `None` once it expired.
    fn keepalive(&self, id: i64) -> Result<Option<u64>, String> {
        let v = self.call("/v3/lease/keepalive", &json!({ "ID": id.to_string() }))?;
        let ttl = int(&v["result"]["TTL"]);
        Ok((ttl > 0).then_some(ttl as u64))
    }

    /// Revokes etcd lease `id`, deleting the keys attached to it; an unknown lease is fine.
    fn revoke_lease(&self, id: i64) {
        let _ = self.call("/v3/lease/revoke", &json!({ "ID": id.to_string() }));
    }

    fn member_keys(&self, node: &NodeId) -> [String; 3] {
        [
            member_key(&self.meta, node),
            alive_key(&self.meta, node),
            present_key(&self.meta, node),
        ]
    }

    fn lease_of(&self, shard: ShardId, rec: &ShardRecord, ttl: u64) -> Lease {
        let term = Duration::from_secs(ttl);
        Lease {
            shard,
            owner: rec.owner.clone(),
            epoch: rec.epoch,
            expires_at: self.clock.now().after(term),
            term,
        }
    }

    fn check_range(&self, shard: ShardId) -> Result<(), LeaseError> {
        if self.meta.shard_count.contains(shard) {
            Ok(())
        } else {
            Err(LeaseError::OutOfRange {
                shard,
                count: self.meta.shard_count,
            })
        }
    }

    /// The shard key, the epoch key and `node`'s member keys, read together.
    fn read_shard(&self, shard: ShardId, node: &NodeId) -> Result<ShardRead, LeaseError> {
        let [record, alive, present] = self.member_keys(node);
        let (_, mut kv) = self
            .read(&[
                shard_key(&self.meta, shard),
                epoch_key(&self.meta, shard),
                record,
                alive,
                present,
            ])
            .map_err(LeaseError::Unavailable)?;
        let keys = MemberKeys {
            present: kv[4].is_some(),
            alive: kv[3].is_some(),
            record: kv[2].take(),
        };
        Ok((kv[0].take(), kv[1].take(), keys.status()))
    }

    /// Writes a new grant of `shard` to `node` after `highest`, if the shard and epoch keys are
    /// still at the revisions read; `old` is the etcd lease to revoke on success.
    fn write_grant(
        &self,
        shard: ShardId,
        node: &NodeId,
        shard_rev: i64,
        epochs: Option<&Kv>,
        old: Option<i64>,
    ) -> Result<Option<Lease>, LeaseError> {
        let highest = epochs.and_then(|e| e.value["epoch"].as_u64()).unwrap_or(0);
        let epochs_rev = epochs.map_or(0, |e| e.mod_revision);
        let (id, ttl) = self
            .grant(ceil_secs(self.config.lease.duration))
            .map_err(LeaseError::Unavailable)?;
        let rec = ShardRecord {
            owner: node.clone(),
            epoch: highest + 1,
            lease: id,
            term_ms: ttl * 1000,
            revoked: false,
        };
        let (sk, ek) = (shard_key(&self.meta, shard), epoch_key(&self.meta, shard));
        let won = self
            .txn(
                vec![unchanged(&sk, shard_rev), unchanged(&ek, epochs_rev)],
                vec![
                    put(&sk, &rec.to_json(), id),
                    put(
                        &ek,
                        &json!({ "epoch": rec.epoch, "owner": node.as_str(), "released": false }),
                        0,
                    ),
                ],
            )
            .map_err(LeaseError::Unavailable)?;
        if won {
            if let Some(old) = old {
                self.revoke_lease(old);
            }
            Ok(Some(self.lease_of(shard, &rec, ttl)))
        } else {
            self.revoke_lease(id);
            Ok(None)
        }
    }

    /// Creates the cluster's metadata key, or checks the node against the recorded one.
    fn ensure_meta(&self, req: &JoinRequest) -> Result<(), JoinError> {
        let key = meta_key(&self.meta);
        let value = json!({
            "shard_count": self.meta.shard_count.get(),
            "hash_version": self.meta.hash_version.number(),
        });
        self.txn(vec![unchanged(&key, 0)], vec![put(&key, &value, 0)])
            .map_err(JoinError::Unavailable)?;
        let (_, kv) = self
            .read(std::slice::from_ref(&key))
            .map_err(JoinError::Unavailable)?;
        let recorded = kv[0].as_ref().map(|k| k.value.clone()).unwrap_or(value);
        let shards = recorded["shard_count"].as_u64().unwrap_or(0);
        if shards != u64::from(req.shard_count.get()) {
            return Err(JoinError::ShardCountMismatch {
                cluster: crate::meta::ShardCount::new(shards).unwrap_or(req.shard_count),
                node: req.shard_count,
            });
        }
        let hash = recorded["hash_version"].as_u64().unwrap_or(0) as u32;
        if hash != req.hash_version {
            return Err(JoinError::HashVersionMismatch {
                cluster: hash,
                node: req.hash_version,
            });
        }
        Ok(())
    }

    fn current_view(&self) -> Result<View, String> {
        let p = prefix(&self.meta);
        let v = self.call(
            "/v3/kv/range",
            &json!({ "key": B64.encode(&p), "range_end": B64.encode(prefix_end(&p)) }),
        )?;
        let revision = int(&v["header"]["revision"]).max(0) as u64;
        let mut records = BTreeMap::new();
        let (mut alive, mut present) = (Vec::new(), Vec::new());
        let mut owners = BTreeMap::new();
        for kv in kvs(&v) {
            let Some(rest) = kv.key.strip_prefix(&p) else {
                continue;
            };
            let (kind, name) = rest.split_once('/').unwrap_or((rest, ""));
            match kind {
                "nodes" => {
                    if let Ok(n) = NodeId::new(name) {
                        records.insert(n, kv);
                    }
                }
                "alive" => alive.push(name.to_owned()),
                "present" => present.push(name.to_owned()),
                "epochs" => {
                    let (Ok(s), Some(epoch), Some(owner)) = (
                        name.parse::<u32>(),
                        kv.value["epoch"].as_u64(),
                        kv.value["owner"].as_str().and_then(|o| NodeId::new(o).ok()),
                    ) else {
                        continue;
                    };
                    if !kv.value["released"].as_bool().unwrap_or(false) {
                        owners.insert(ShardId::new(s), ShardOwner { node: owner, epoch });
                    }
                }
                _ => {}
            }
        }
        let members = records
            .into_iter()
            .filter_map(|(n, record)| {
                let keys = MemberKeys {
                    alive: alive.iter().any(|a| a == n.as_str()),
                    present: present.iter().any(|a| a == n.as_str()),
                    record: Some(record),
                };
                let status = keys.status()?;
                let endpoint = keys.record?.value["endpoint"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                Some((n, (endpoint, status)))
            })
            .collect();
        Ok(self
            .lock()
            .publisher
            .view(&self.meta.cluster, (members, owners), revision))
    }

    /// The live shard leases `node` holds, by etcd.
    fn held_by(&self, node: &NodeId) -> Result<Vec<ShardId>, String> {
        let p = format!("{}shards/", prefix(&self.meta));
        let v = self.call(
            "/v3/kv/range",
            &json!({ "key": B64.encode(&p), "range_end": B64.encode(prefix_end(&p)) }),
        )?;
        Ok(kvs(&v)
            .into_iter()
            .filter_map(|kv| {
                let rec = ShardRecord::from_json(&kv.value)?;
                let s = kv.key.strip_prefix(&p)?.parse().ok()?;
                (&rec.owner == node).then_some(ShardId::new(s))
            })
            .collect())
    }
}

impl Authority for EtcdAuthority {
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError> {
        check_compatible(&self.meta, req)?;
        self.ensure_meta(req)?;
        let [record, alive, present] = self.member_keys(&req.node);
        let detector = self.config.detector;
        let (a, _) = self
            .grant(ceil_secs(detector.suspicion_time))
            .map_err(JoinError::Unavailable)?;
        let (p, _) = match self.grant(ceil_secs(detector.failure_time)) {
            Ok(x) => x,
            Err(e) => {
                self.revoke_lease(a);
                return Err(JoinError::Unavailable(e));
            }
        };
        let value = json!({ "endpoint": req.endpoint, "draining": false });
        let joined = self.txn(
            vec![unchanged(&present, 0)],
            vec![
                put(&record, &value, 0),
                put(&alive, &json!({}), a),
                put(&present, &json!({}), p),
            ],
        );
        match joined {
            Ok(true) => {}
            other => {
                self.revoke_lease(a);
                self.revoke_lease(p);
                return Err(match other {
                    Err(e) => JoinError::Unavailable(e),
                    _ => JoinError::DuplicateNode(req.node.clone()),
                });
            }
        }
        if let Some(old) = self.lock().members.insert(
            req.node.clone(),
            MemberLeases {
                alive: a,
                present: p,
            },
        ) {
            self.revoke_lease(old.alive);
            self.revoke_lease(old.present);
        }
        self.current_view().map_err(JoinError::Unavailable)
    }

    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError> {
        let Some(leases) = self.lock().members.get(node).copied() else {
            return Err(MembershipError::NotMember(node.clone()));
        };
        if self
            .keepalive(leases.present)
            .map_err(MembershipError::Unavailable)?
            .is_none()
        {
            // silent for the failure time: failed; the node must join again
            self.lock().members.remove(node);
            self.revoke_lease(leases.alive);
            return Err(MembershipError::NotMember(node.clone()));
        }
        if self
            .keepalive(leases.alive)
            .map_err(MembershipError::Unavailable)?
            .is_none()
        {
            // suspect: answer with a fresh liveness key, which makes the member active again
            let (a, _) = self
                .grant(ceil_secs(self.config.detector.suspicion_time))
                .map_err(MembershipError::Unavailable)?;
            let present = present_key(&self.meta, node);
            let ok = self
                .txn(
                    vec![json!({
                        "key": B64.encode(&present),
                        "target": "MOD",
                        "result": "GREATER",
                        "mod_revision": "0",
                    })],
                    vec![put(&alive_key(&self.meta, node), &json!({}), a)],
                )
                .map_err(MembershipError::Unavailable)?;
            if !ok {
                self.revoke_lease(a);
                return Err(MembershipError::NotMember(node.clone()));
            }
            if let Some(l) = self.lock().members.get_mut(node) {
                l.alive = a;
            }
        }
        Ok(())
    }

    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError> {
        let [record, alive, present] = self.member_keys(node);
        let (_, kv) = self
            .read(&[record.clone(), alive, present.clone()])
            .map_err(MembershipError::Unavailable)?;
        let (Some(rec), Some(p)) = (&kv[0], &kv[2]) else {
            return Err(MembershipError::NotMember(node.clone()));
        };
        let mut value = rec.value.clone();
        value["draining"] = Value::Bool(true);
        let ok = self
            .txn(
                vec![unchanged(&present, p.mod_revision)],
                vec![put(&record, &value, 0)],
            )
            .map_err(MembershipError::Unavailable)?;
        if !ok {
            return Err(MembershipError::NotMember(node.clone()));
        }
        self.current_view().map_err(MembershipError::Unavailable)
    }

    fn leave(&self, node: &NodeId) -> Result<View, MembershipError> {
        let held = self.held_by(node).map_err(MembershipError::Unavailable)?;
        if !held.is_empty() {
            return Err(MembershipError::HoldsLeases {
                node: node.clone(),
                shards: held,
            });
        }
        let [record, alive, present] = self.member_keys(node);
        self.txn(
            Vec::new(),
            vec![delete(&record), delete(&alive), delete(&present)],
        )
        .map_err(MembershipError::Unavailable)?;
        if let Some(l) = self.lock().members.remove(node) {
            self.revoke_lease(l.alive);
            self.revoke_lease(l.present);
        }
        self.current_view().map_err(MembershipError::Unavailable)
    }

    fn view(&self) -> Result<View, MembershipError> {
        self.current_view().map_err(MembershipError::Unavailable)
    }

    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError> {
        self.check_range(shard)?;
        for _ in 0..4 {
            let (current, epochs, status) = self.read_shard(shard, node)?;
            if status != Some(MemberStatus::Active) {
                return Err(LeaseError::NotEligible {
                    node: node.clone(),
                    status,
                });
            }
            let mut old = None;
            if let Some(kv) = &current
                && let Some(rec) = ShardRecord::from_json(&kv.value)
            {
                if &rec.owner != node {
                    return Err(LeaseError::Held {
                        shard,
                        owner: rec.owner,
                        until: self.clock.now().after(Duration::from_millis(rec.term_ms)),
                    });
                }
                if rec.revoked {
                    return Err(LeaseError::Revoked { shard });
                }
                // this node's own grant: renew it, keeping its epoch
                if let Some(ttl) = self.keepalive(rec.lease).map_err(LeaseError::Unavailable)? {
                    return Ok(self.lease_of(shard, &rec, ttl));
                }
                old = Some(rec.lease); // expired, about to be deleted: replace it
            }
            let rev = current.as_ref().map_or(0, |k| k.mod_revision);
            if let Some(lease) = self.write_grant(shard, node, rev, epochs.as_ref(), old)? {
                return Ok(lease);
            }
        }
        Err(LeaseError::Unavailable(format!(
            "the grant of {shard} lost every race in etcd"
        )))
    }

    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        self.check_range(lease.shard)?;
        let (current, _, status) = self.read_shard(lease.shard, &lease.owner)?;
        if !matches!(
            status,
            Some(MemberStatus::Active | MemberStatus::Suspect | MemberStatus::Draining)
        ) {
            return Err(LeaseError::NotEligible {
                node: lease.owner.clone(),
                status,
            });
        }
        let expired = LeaseError::Expired {
            shard: lease.shard,
            epoch: lease.epoch,
        };
        let Some(rec) = current.and_then(|kv| ShardRecord::from_json(&kv.value)) else {
            return Err(expired);
        };
        if rec.epoch != lease.epoch || rec.owner != lease.owner {
            return Err(LeaseError::Stale {
                shard: lease.shard,
                epoch: lease.epoch,
                current: rec.epoch,
            });
        }
        if rec.revoked {
            return Err(LeaseError::Revoked { shard: lease.shard });
        }
        match self.keepalive(rec.lease).map_err(LeaseError::Unavailable)? {
            Some(ttl) => Ok(self.lease_of(lease.shard, &rec, ttl)),
            None => Err(expired),
        }
    }

    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.check_range(lease.shard)?;
        let (sk, ek) = (
            shard_key(&self.meta, lease.shard),
            epoch_key(&self.meta, lease.shard),
        );
        let (_, kv) = self
            .read(std::slice::from_ref(&sk))
            .map_err(LeaseError::Unavailable)?;
        let Some(kv) = &kv[0] else {
            return Ok(());
        };
        let Some(rec) = ShardRecord::from_json(&kv.value) else {
            return Ok(());
        };
        if rec.epoch != lease.epoch || rec.owner != lease.owner {
            return Ok(());
        }
        let released = json!({ "epoch": rec.epoch, "owner": rec.owner.as_str(), "released": true });
        let ok = self
            .txn(
                vec![unchanged(&sk, kv.mod_revision)],
                vec![delete(&sk), put(&ek, &released, 0)],
            )
            .map_err(LeaseError::Unavailable)?;
        if ok {
            self.revoke_lease(rec.lease);
        }
        Ok(())
    }

    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.check_range(shard)?;
        let sk = shard_key(&self.meta, shard);
        let (_, kv) = self
            .read(std::slice::from_ref(&sk))
            .map_err(LeaseError::Unavailable)?;
        if let Some(kv) = &kv[0]
            && let Some(mut rec) = ShardRecord::from_json(&kv.value)
            && !rec.revoked
        {
            rec.revoked = true;
            // fails harmlessly when the lease expired in between: nothing left to revoke
            let _ = self.txn(
                vec![unchanged(&sk, kv.mod_revision)],
                vec![put(&sk, &rec.to_json(), rec.lease)],
            );
        }
        Ok(())
    }

    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        self.check_range(lease.shard)?;
        let (current, epochs, status) = self.read_shard(lease.shard, to)?;
        if status != Some(MemberStatus::Active) {
            return Err(LeaseError::NotEligible {
                node: to.clone(),
                status,
            });
        }
        let expired = LeaseError::Expired {
            shard: lease.shard,
            epoch: lease.epoch,
        };
        let Some(kv) = current else {
            return Err(expired);
        };
        let Some(rec) = ShardRecord::from_json(&kv.value) else {
            return Err(expired);
        };
        if rec.epoch != lease.epoch || rec.owner != lease.owner {
            return Err(LeaseError::Stale {
                shard: lease.shard,
                epoch: lease.epoch,
                current: rec.epoch,
            });
        }
        if rec.revoked {
            return Err(LeaseError::Revoked { shard: lease.shard });
        }
        if self
            .keepalive(rec.lease)
            .map_err(LeaseError::Unavailable)?
            .is_none()
        {
            return Err(expired);
        }
        match self.write_grant(
            lease.shard,
            to,
            kv.mod_revision,
            epochs.as_ref(),
            Some(rec.lease),
        )? {
            Some(next) => Ok(next),
            None => Err(LeaseError::Unavailable(format!(
                "the handover of {} raced another change in etcd",
                lease.shard
            ))),
        }
    }
}

impl EtcdAuthority {
    /// The lease term granted for shard leases, in milliseconds (the configured duration
    /// rounded up to whole seconds).
    pub fn term_ms(&self) -> u64 {
        duration_ms(Duration::from_secs(ceil_secs(self.config.lease.duration)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_end_is_one_past_the_prefix() {
        assert_eq!(prefix_end("/vertep/c/"), b"/vertep/c0".to_vec());
        assert_eq!(prefix_end("a\u{7f}"), b"a\x80".to_vec());
    }

    #[test]
    fn shard_records_round_trip() {
        let r = ShardRecord {
            owner: NodeId::new("n1").unwrap(),
            epoch: 42,
            lease: 7_587_898_844_219_875_081,
            term_ms: 4_000,
            revoked: true,
        };
        assert_eq!(ShardRecord::from_json(&r.to_json()), Some(r));
        assert_eq!(ShardRecord::from_json(&json!({"owner": "n1"})), None);
    }
}
