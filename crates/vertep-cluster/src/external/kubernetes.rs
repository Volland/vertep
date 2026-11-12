//! The Kubernetes lease authority: `coordination.k8s.io/v1` `Lease` objects in one namespace,
//! read and written over HTTPS with optimistic concurrency (`resourceVersion`).
//!
//! Objects of cluster `c` (all labelled `vertep.io/cluster: c`):
//!
//! | name | holder | meaning |
//! |---|---|---|
//! | `vertep-c-meta` | — | annotations record the shard count and hash version |
//! | `vertep-c-member-<n>` | `n` | `renewTime` is the heartbeat; annotation `vertep.io/status` |
//! | `vertep-c-shard-<s>-of-<N>` | owner, empty once released | `leaseTransitions` is the epoch |
//!
//! Kubernetes stores leases but never expires them. A node therefore treats a shard held by
//! another node as expired only after it has itself seen the holder's record (holder, epoch,
//! `renewTime`) unchanged for a whole lease term on its own monotonic clock, as client-go's
//! leader election does; since it first saw the record after the holder sent that renewal, the
//! holder's local deadline (send time + term − safety margin) has passed by then, whatever the
//! wall clocks say. Members are marked suspect and failed the same way: any node that sees a
//! member's heartbeat unchanged for the suspicion or failure time writes the new status, guarded
//! by the object's `resourceVersion`, so a heartbeat in between wins.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{Value, json};

use super::http::{Client, Response};
use super::{Publisher, ceil_secs};
use crate::authority::{Authority, AuthorityConfig};
use crate::clock::{Clock, Millis};
use crate::lease::{Lease, LeaseError};
use crate::membership::{JoinError, JoinRequest, MembershipError, check_compatible};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::ShardId;
use crate::view::{MemberStatus, ShardOwner, View};

/// How to reach the Kubernetes API server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KubeConfig {
    /// The API server URL, `https://host:6443`.
    pub server: String,
    /// The namespace the `Lease` objects live in.
    pub namespace: String,
    /// The PEM certificates of the API server's certificate authority.
    pub ca_pem: Option<Vec<u8>>,
    /// The PEM client certificate chain, for client-certificate authentication.
    pub client_cert_pem: Option<Vec<u8>>,
    /// The PEM private key of the client certificate.
    pub client_key_pem: Option<Vec<u8>>,
    /// A bearer token (a service account token), instead of a client certificate.
    pub token: Option<String>,
    /// Connect, read and write timeout of every request.
    pub timeout: Duration,
}

/// The first value of `key:` in a kubeconfig, unquoted.
fn yaml_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let l = l.trim_start().trim_start_matches("- ");
        let v = l.strip_prefix(key)?.strip_prefix(':')?.trim();
        Some(v.trim_matches('"').trim_matches('\''))
    })
}

impl KubeConfig {
    /// The server, credentials and namespace of a kubeconfig file's text: the first cluster's
    /// `server` and `certificate-authority(-data)`, the first user's
    /// `client-certificate(-data)` and `client-key(-data)` or `token`, and the first
    /// `namespace` (else `default`). Enough for the kubeconfigs k3s, kind and kubeadm write;
    /// contexts are not interpreted.
    ///
    /// # Errors
    ///
    /// A missing server, bad base64, or an unreadable referenced file.
    pub fn from_kubeconfig(text: &str) -> Result<Self, String> {
        let data = |key: &str| -> Result<Option<Vec<u8>>, String> {
            if let Some(v) = yaml_value(text, &format!("{key}-data")) {
                return B64
                    .decode(v)
                    .map(Some)
                    .map_err(|e| format!("{key}-data: {e}"));
            }
            match yaml_value(text, key) {
                Some(path) => std::fs::read(path)
                    .map(Some)
                    .map_err(|e| format!("{key} {path}: {e}")),
                None => Ok(None),
            }
        };
        Ok(KubeConfig {
            server: yaml_value(text, "server")
                .ok_or("the kubeconfig names no server")?
                .to_owned(),
            namespace: yaml_value(text, "namespace")
                .unwrap_or("default")
                .to_owned(),
            ca_pem: data("certificate-authority")?,
            client_cert_pem: data("client-certificate")?,
            client_key_pem: data("client-key")?,
            token: yaml_value(text, "token").map(str::to_owned),
            timeout: Duration::from_secs(3),
        })
    }

    /// The configuration of a pod: the API server from `KUBERNETES_SERVICE_HOST` and
    /// `KUBERNETES_SERVICE_PORT`, and the service account's token, CA and namespace.
    ///
    /// # Errors
    ///
    /// Not running in a pod, or the service account files are unreadable.
    pub fn in_cluster() -> Result<Self, String> {
        let dir = "/var/run/secrets/kubernetes.io/serviceaccount";
        let host = std::env::var("KUBERNETES_SERVICE_HOST")
            .map_err(|_| "KUBERNETES_SERVICE_HOST is not set: not in a pod")?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
        let read = |f: &str| std::fs::read(format!("{dir}/{f}")).map_err(|e| format!("{f}: {e}"));
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Ok(KubeConfig {
            server: format!("https://{host}:{port}"),
            namespace: String::from_utf8_lossy(&read("namespace")?)
                .trim()
                .to_owned(),
            ca_pem: Some(read("ca.crt")?),
            client_cert_pem: None,
            client_key_pem: None,
            token: Some(String::from_utf8_lossy(&read("token")?).trim().to_owned()),
            timeout: Duration::from_secs(3),
        })
    }

    fn client(&self) -> Result<Client, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        if let Some(ca) = &self.ca_pem {
            for c in CertificateDer::pem_slice_iter(ca) {
                roots
                    .add(c.map_err(|e| format!("CA certificate: {e:?}"))?)
                    .map_err(|e| format!("CA certificate: {e}"))?;
            }
        }
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots);
        let tls = match (&self.client_cert_pem, &self.client_key_pem) {
            (Some(cert), Some(key)) => {
                let chain = CertificateDer::pem_slice_iter(cert)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("client certificate: {e:?}"))?;
                let key =
                    PrivateKeyDer::from_pem_slice(key).map_err(|e| format!("client key: {e:?}"))?;
                builder
                    .with_client_auth_cert(chain, key)
                    .map_err(|e| format!("client certificate: {e}"))?
            }
            _ => builder.with_no_client_auth(),
        };
        let mut client = Client::new(&self.server, self.timeout)?.with_tls(Arc::new(tls));
        if let Some(t) = &self.token {
            client = client.with_header("Authorization", &format!("Bearer {t}"));
        }
        Ok(client)
    }
}

/// The name of a shard's `Lease` object: `vertep-<cluster>-shard-<s>-of-<n>`.
pub fn lease_name(meta: &ClusterMeta, shard: ShardId) -> String {
    format!(
        "vertep-{}-shard-{}-of-{}",
        meta.cluster,
        shard.get(),
        meta.shard_count
    )
}

/// The name of a member's heartbeat `Lease` object: `vertep-<cluster>-member-<node>`.
pub fn member_lease_name(meta: &ClusterMeta, node: &NodeId) -> String {
    format!("vertep-{}-member-{node}", meta.cluster)
}

fn meta_name(meta: &ClusterMeta) -> String {
    format!("vertep-{}-meta", meta.cluster)
}

const STATUS: &str = "vertep.io/status";
const ENDPOINT: &str = "vertep.io/endpoint";
const REVOKED: &str = "vertep.io/revoked";

/// `t` as a Kubernetes `MicroTime`: RFC 3339 UTC with microseconds.
fn micro_time(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // civil from days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        rem / 3_600,
        rem / 60 % 60,
        rem % 60,
        d.subsec_micros()
    )
}

fn status_name(s: MemberStatus) -> &'static str {
    match s {
        MemberStatus::Active => "active",
        MemberStatus::Suspect => "suspect",
        MemberStatus::Draining => "draining",
        MemberStatus::Failed => "failed",
    }
}

fn parse_status(s: &str) -> Option<MemberStatus> {
    Some(match s {
        "active" => MemberStatus::Active,
        "suspect" => MemberStatus::Suspect,
        "draining" => MemberStatus::Draining,
        "failed" => MemberStatus::Failed,
        _ => return None,
    })
}

fn member_status(obj: &Value) -> Option<MemberStatus> {
    obj["metadata"]["annotations"][STATUS]
        .as_str()
        .and_then(parse_status)
}

fn holder(obj: &Value) -> Option<NodeId> {
    obj["spec"]["holderIdentity"]
        .as_str()
        .filter(|h| !h.is_empty())
        .and_then(|h| NodeId::new(h).ok())
}

fn transitions(obj: &Value) -> u64 {
    obj["spec"]["leaseTransitions"].as_u64().unwrap_or(0)
}

fn revoked(obj: &Value) -> bool {
    obj["metadata"]["annotations"][REVOKED].as_str() == Some("true")
}

/// What changes when the holder renews or the lease moves: an observer that sees it unchanged
/// for a term treats the lease as expired.
fn shard_mark(obj: &Value) -> String {
    format!(
        "{}|{}|{}",
        obj["spec"]["holderIdentity"].as_str().unwrap_or(""),
        transitions(obj),
        obj["spec"]["renewTime"].as_str().unwrap_or("")
    )
}

/// When this client first saw a record in its current form.
#[derive(Clone, Debug)]
struct Seen {
    mark: String,
    since: Millis,
}

#[derive(Debug, Default)]
struct State {
    publisher: Publisher,
    seen: BTreeMap<String, Seen>,
    /// Per shard, the epoch and local send time of this client's last successful grant or
    /// renewal: past `sent + term` the lease may have been taken, so it is not renewed.
    renewed: BTreeMap<ShardId, (u64, Millis)>,
}

/// The outcome of a write guarded by `resourceVersion`.
enum Write {
    Done(Value),
    Conflict,
}

/// The Kubernetes Lease API as the lease authority and member registry of one cluster. One
/// instance serves the nodes of one process; every node may run its own instance.
// @tg: implements:: [[openspec:cluster-membership#External Membership]]
// @tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]
// @tg: implements:: [[openspec:cluster-membership#Failure Detection]]
#[derive(Debug)]
pub struct KubeAuthority {
    meta: ClusterMeta,
    config: AuthorityConfig,
    namespace: String,
    client: Client,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

impl KubeAuthority {
    /// The authority of the cluster `meta` in the namespace and API server of `kube`, with
    /// lease and failure detection timing `config` (lease terms rounded up to whole seconds),
    /// timing observations on `clock`.
    ///
    /// # Errors
    ///
    /// A malformed server URL or unusable credentials.
    pub fn new(
        meta: ClusterMeta,
        config: AuthorityConfig,
        kube: &KubeConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, String> {
        Ok(KubeAuthority {
            meta,
            config,
            namespace: kube.namespace.clone(),
            client: kube.client()?,
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

    fn term(&self) -> Duration {
        Duration::from_secs(ceil_secs(self.config.lease.duration))
    }

    fn path(&self, name: Option<&str>) -> String {
        let base = format!(
            "/apis/coordination.k8s.io/v1/namespaces/{}/leases",
            self.namespace
        );
        match name {
            Some(n) => format!("{base}/{n}"),
            None => base,
        }
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Response, String> {
        let r = self.client.request(method, path, body)?;
        if r.ok() || r.status == 404 || r.status == 409 {
            Ok(r)
        } else {
            Err(format!(
                "Kubernetes {method} {path}: HTTP {}: {}",
                r.status,
                String::from_utf8_lossy(&r.body)
            ))
        }
    }

    /// The object `name`, or `None` when it does not exist.
    fn get(&self, name: &str) -> Result<Option<Value>, String> {
        let r = self.send("GET", &self.path(Some(name)), None)?;
        if r.status == 404 {
            Ok(None)
        } else {
            r.json().map(Some)
        }
    }

    /// Creates `obj`; a conflict when it exists.
    fn create(&self, obj: &Value) -> Result<Write, String> {
        let r = self.send("POST", &self.path(None), Some(obj))?;
        match r.status {
            409 => Ok(Write::Conflict),
            404 => Err(format!("namespace {} not found", self.namespace)),
            _ => r.json().map(Write::Done),
        }
    }

    /// Replaces `obj` (which carries the `resourceVersion` read); a conflict when it changed.
    fn replace(&self, obj: &Value) -> Result<Write, String> {
        let name = obj["metadata"]["name"].as_str().unwrap_or("");
        let r = self.send("PUT", &self.path(Some(name)), Some(obj))?;
        match r.status {
            409 | 404 => Ok(Write::Conflict),
            _ => r.json().map(Write::Done),
        }
    }

    fn new_object(&self, name: &str, role: &str, extra_labels: Value) -> Value {
        let mut labels =
            json!({ "vertep.io/cluster": self.meta.cluster.as_str(), "vertep.io/role": role });
        if let (Some(l), Some(e)) = (labels.as_object_mut(), extra_labels.as_object()) {
            l.extend(e.clone());
        }
        json!({
            "apiVersion": "coordination.k8s.io/v1",
            "kind": "Lease",
            "metadata": { "name": name, "namespace": self.namespace, "labels": labels, "annotations": {} },
            "spec": {},
        })
    }

    /// How long this client has seen `name` in the form `mark`.
    fn silence(&self, name: &str, mark: String) -> (Duration, Millis) {
        let now = self.clock.now();
        let mut st = self.lock();
        let seen = st
            .seen
            .entry(name.to_owned())
            .and_modify(|s| {
                if s.mark != mark {
                    *s = Seen {
                        mark: mark.clone(),
                        since: now,
                    };
                }
            })
            .or_insert(Seen { mark, since: now });
        (now.since(seen.since), seen.since)
    }

    /// The status `obj` (a member's heartbeat lease) should have now, by this client's
    /// observation of its heartbeat.
    fn detected(&self, obj: &Value) -> Option<MemberStatus> {
        let status = member_status(obj)?;
        let name = obj["metadata"]["name"].as_str().unwrap_or("");
        let mark = obj["spec"]["renewTime"].as_str().unwrap_or("").to_owned();
        let (silence, _) = self.silence(name, mark);
        let d = self.config.detector;
        Some(match status {
            MemberStatus::Failed => MemberStatus::Failed,
            _ if silence >= d.failure_time => MemberStatus::Failed,
            MemberStatus::Active if silence >= d.suspicion_time => MemberStatus::Suspect,
            s => s,
        })
    }

    /// Applies failure detection to the member lease `obj`: writes the detected status when it
    /// changed. Returns the status the service now records.
    fn detect(&self, obj: &Value) -> Result<Option<MemberStatus>, String> {
        let (now, detected) = (member_status(obj), self.detected(obj));
        if now == detected {
            return Ok(now);
        }
        let mut next = obj.clone();
        next["metadata"]["annotations"][STATUS] =
            Value::String(status_name(detected.unwrap_or(MemberStatus::Failed)).to_owned());
        Ok(match self.replace(&next)? {
            Write::Done(_) => detected,
            Write::Conflict => self
                .get(obj["metadata"]["name"].as_str().unwrap_or(""))?
                .as_ref()
                .and_then(member_status),
        })
    }

    /// The status the service records for `node`.
    fn status_of(&self, node: &NodeId) -> Result<Option<MemberStatus>, String> {
        Ok(self
            .get(&member_lease_name(&self.meta, node))?
            .as_ref()
            .and_then(member_status))
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

    fn lease(&self, shard: ShardId, owner: &NodeId, epoch: u64, sent: Millis) -> Lease {
        self.lock().renewed.insert(shard, (epoch, sent));
        Lease {
            shard,
            owner: owner.clone(),
            epoch,
            expires_at: sent.after(self.term()),
            term: self.term(),
        }
    }

    /// True when this client's own last renewal of `shard` at `epoch` is more than a term ago.
    fn own_lapsed(&self, shard: ShardId, epoch: u64, now: Millis) -> bool {
        self.lock()
            .renewed
            .get(&shard)
            .is_some_and(|(e, sent)| *e == epoch && now >= sent.after(self.term()))
    }

    /// `obj` held by `node` from now at `epoch`.
    fn granted(&self, obj: &Value, node: &NodeId, epoch: u64, fresh: bool) -> Value {
        let now = micro_time(SystemTime::now());
        let mut next = obj.clone();
        next["spec"]["holderIdentity"] = Value::String(node.as_str().to_owned());
        next["spec"]["leaseDurationSeconds"] = json!(ceil_secs(self.config.lease.duration));
        next["spec"]["leaseTransitions"] = json!(epoch);
        next["spec"]["renewTime"] = Value::String(now.clone());
        if fresh {
            next["spec"]["acquireTime"] = Value::String(now);
            if let Some(a) = next["metadata"]["annotations"].as_object_mut() {
                a.remove(REVOKED);
            }
        }
        next
    }

    /// Creates the cluster's metadata object, or checks the node against the recorded one.
    fn ensure_meta(&self, req: &JoinRequest) -> Result<(), JoinError> {
        let name = meta_name(&self.meta);
        let mut obj = self.new_object(&name, "meta", json!({}));
        obj["metadata"]["annotations"] = json!({
            "vertep.io/shard-count": self.meta.shard_count.get().to_string(),
            "vertep.io/hash-version": self.meta.hash_version.number().to_string(),
        });
        let recorded = match self.create(&obj).map_err(JoinError::Unavailable)? {
            Write::Done(o) => o,
            Write::Conflict => self
                .get(&name)
                .map_err(JoinError::Unavailable)?
                .unwrap_or(obj),
        };
        let a = &recorded["metadata"]["annotations"];
        let num = |k: &str| {
            a[k].as_str()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let shards = num("vertep.io/shard-count");
        if shards != u64::from(req.shard_count.get()) {
            return Err(JoinError::ShardCountMismatch {
                cluster: crate::meta::ShardCount::new(shards).unwrap_or(req.shard_count),
                node: req.shard_count,
            });
        }
        let hash = num("vertep.io/hash-version") as u32;
        if hash != req.hash_version {
            return Err(JoinError::HashVersionMismatch {
                cluster: hash,
                node: req.hash_version,
            });
        }
        Ok(())
    }

    fn list(&self) -> Result<(u64, Vec<Value>), String> {
        let path = format!(
            "{}?labelSelector=vertep.io%2Fcluster%3D{}",
            self.path(None),
            self.meta.cluster
        );
        let v = self.send("GET", &path, None)?.json()?;
        let revision = v["metadata"]["resourceVersion"]
            .as_str()
            .and_then(|r| r.parse().ok())
            .unwrap_or(0);
        Ok((revision, v["items"].as_array().cloned().unwrap_or_default()))
    }

    fn current_view(&self) -> Result<View, String> {
        let (_, items) = self.list()?;
        let mut wrote = false;
        for obj in items.iter().filter(|o| role(o) == "member") {
            if self.detect(obj)? != member_status(obj) {
                wrote = true;
            }
        }
        let (revision, items) = if wrote { self.list()? } else { (0, items) };
        let revision = if revision == 0 {
            self.list()?.0
        } else {
            revision
        };
        let mut members = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for obj in &items {
            match role(obj) {
                "member" => {
                    let (Some(n), Some(s)) = (holder(obj), member_status(obj)) else {
                        continue;
                    };
                    let endpoint = obj["metadata"]["annotations"][ENDPOINT]
                        .as_str()
                        .unwrap_or("")
                        .to_owned();
                    members.insert(n, (endpoint, s));
                }
                "shard" => {
                    let shard = obj["metadata"]["labels"]["vertep.io/shard"]
                        .as_str()
                        .and_then(|s| s.parse::<u32>().ok());
                    if let (Some(s), Some(h)) = (shard, holder(obj)) {
                        owners.insert(
                            ShardId::new(s),
                            ShardOwner {
                                node: h,
                                epoch: transitions(obj),
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(self
            .lock()
            .publisher
            .view(&self.meta.cluster, (members, owners), revision))
    }

    fn eligible(&self, node: &NodeId, statuses: &[MemberStatus]) -> Result<(), LeaseError> {
        let status = self.status_of(node).map_err(LeaseError::Unavailable)?;
        if status.is_some_and(|s| statuses.contains(&s)) {
            Ok(())
        } else {
            Err(LeaseError::NotEligible {
                node: node.clone(),
                status,
            })
        }
    }

    /// Checks that `obj` is the live grant `lease` describes, as a renewal would.
    fn check_current(&self, obj: Option<&Value>, lease: &Lease) -> Result<Value, LeaseError> {
        let expired = LeaseError::Expired {
            shard: lease.shard,
            epoch: lease.epoch,
        };
        let Some(obj) = obj else {
            return Err(expired);
        };
        let current = transitions(obj);
        if holder(obj).as_ref() != Some(&lease.owner) || current != lease.epoch {
            return Err(LeaseError::Stale {
                shard: lease.shard,
                epoch: lease.epoch,
                current,
            });
        }
        if revoked(obj) {
            return Err(LeaseError::Revoked { shard: lease.shard });
        }
        if self.own_lapsed(lease.shard, lease.epoch, self.clock.now()) {
            return Err(expired);
        }
        Ok(obj.clone())
    }
}

fn role(obj: &Value) -> &str {
    obj["metadata"]["labels"]["vertep.io/role"]
        .as_str()
        .unwrap_or("")
}

impl Authority for KubeAuthority {
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError> {
        check_compatible(&self.meta, req)?;
        self.ensure_meta(req)?;
        let name = member_lease_name(&self.meta, &req.node);
        let unavailable = JoinError::Unavailable;
        let existing = self.get(&name).map_err(unavailable)?;
        let mut obj = match &existing {
            Some(o) => {
                if self.detect(o).map_err(unavailable)? != Some(MemberStatus::Failed) {
                    return Err(JoinError::DuplicateNode(req.node.clone()));
                }
                self.get(&name)
                    .map_err(unavailable)?
                    .ok_or_else(|| JoinError::DuplicateNode(req.node.clone()))?
            }
            None => self.new_object(&name, "member", json!({})),
        };
        obj["spec"]["holderIdentity"] = Value::String(req.node.as_str().to_owned());
        obj["spec"]["leaseDurationSeconds"] = json!(ceil_secs(self.config.detector.failure_time));
        obj["spec"]["renewTime"] = Value::String(micro_time(SystemTime::now()));
        obj["metadata"]["annotations"][ENDPOINT] = Value::String(req.endpoint.clone());
        obj["metadata"]["annotations"][STATUS] = Value::String("active".into());
        let written = if existing.is_some() {
            self.replace(&obj)
        } else {
            self.create(&obj)
        };
        match written.map_err(unavailable)? {
            Write::Done(_) => self.current_view().map_err(unavailable),
            Write::Conflict => Err(JoinError::DuplicateNode(req.node.clone())),
        }
    }

    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError> {
        let name = member_lease_name(&self.meta, node);
        for _ in 0..3 {
            let Some(mut obj) = self.get(&name).map_err(MembershipError::Unavailable)? else {
                return Err(MembershipError::NotMember(node.clone()));
            };
            match member_status(&obj) {
                None | Some(MemberStatus::Failed) => {
                    return Err(MembershipError::NotMember(node.clone()));
                }
                Some(MemberStatus::Suspect) => {
                    obj["metadata"]["annotations"][STATUS] = Value::String("active".into());
                }
                Some(_) => {}
            }
            obj["spec"]["renewTime"] = Value::String(micro_time(SystemTime::now()));
            if let Write::Done(_) = self.replace(&obj).map_err(MembershipError::Unavailable)? {
                return Ok(());
            }
        }
        Err(MembershipError::Unavailable(format!(
            "the heartbeat of {node} kept conflicting"
        )))
    }

    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError> {
        let name = member_lease_name(&self.meta, node);
        for _ in 0..3 {
            let obj = self.get(&name).map_err(MembershipError::Unavailable)?;
            let Some(mut obj) =
                obj.filter(|o| member_status(o).is_some_and(|s| s != MemberStatus::Failed))
            else {
                return Err(MembershipError::NotMember(node.clone()));
            };
            obj["metadata"]["annotations"][STATUS] = Value::String("draining".into());
            obj["spec"]["renewTime"] = Value::String(micro_time(SystemTime::now()));
            if let Write::Done(_) = self.replace(&obj).map_err(MembershipError::Unavailable)? {
                return self.current_view().map_err(MembershipError::Unavailable);
            }
        }
        Err(MembershipError::Unavailable(format!(
            "the drain of {node} kept conflicting"
        )))
    }

    fn leave(&self, node: &NodeId) -> Result<View, MembershipError> {
        let (_, items) = self.list().map_err(MembershipError::Unavailable)?;
        let now = self.clock.now();
        let mut held = Vec::new();
        for obj in items.iter().filter(|o| role(o) == "shard") {
            if holder(obj).as_ref() != Some(node) {
                continue;
            }
            let Some(s) = obj["metadata"]["labels"]["vertep.io/shard"]
                .as_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let shard = ShardId::new(s);
            let name = obj["metadata"]["name"].as_str().unwrap_or("");
            let (silence, _) = self.silence(name, shard_mark(obj));
            if !self.own_lapsed(shard, transitions(obj), now) && silence < self.term() {
                held.push(shard);
            }
        }
        if !held.is_empty() {
            return Err(MembershipError::HoldsLeases {
                node: node.clone(),
                shards: held,
            });
        }
        let name = member_lease_name(&self.meta, node);
        self.send("DELETE", &self.path(Some(&name)), None)
            .map_err(MembershipError::Unavailable)?;
        self.current_view().map_err(MembershipError::Unavailable)
    }

    fn view(&self) -> Result<View, MembershipError> {
        self.current_view().map_err(MembershipError::Unavailable)
    }

    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError> {
        self.check_range(shard)?;
        self.eligible(node, &[MemberStatus::Active])?;
        let name = lease_name(&self.meta, shard);
        let unavailable = LeaseError::Unavailable;
        for _ in 0..4 {
            let sent = self.clock.now();
            let Some(obj) = self.get(&name).map_err(unavailable)? else {
                let obj = self.new_object(
                    &name,
                    "shard",
                    json!({ "vertep.io/shard": shard.get().to_string() }),
                );
                if let Write::Done(_) = self
                    .create(&self.granted(&obj, node, 1, true))
                    .map_err(unavailable)?
                {
                    return Ok(self.lease(shard, node, 1, sent));
                }
                continue;
            };
            let epoch = transitions(&obj);
            let (next, epoch) = match holder(&obj) {
                Some(h) if &h == node => {
                    if revoked(&obj) {
                        return Err(LeaseError::Revoked { shard });
                    }
                    if self.own_lapsed(shard, epoch, sent) {
                        // its own grant ran out: a new grant, as for anyone
                        (self.granted(&obj, node, epoch + 1, true), epoch + 1)
                    } else {
                        (self.granted(&obj, node, epoch, false), epoch)
                    }
                }
                None => (self.granted(&obj, node, epoch + 1, true), epoch + 1),
                Some(h) => {
                    let (silence, since) = self.silence(&name, shard_mark(&obj));
                    if silence < self.term() {
                        return Err(LeaseError::Held {
                            shard,
                            owner: h,
                            until: since.after(self.term()),
                        });
                    }
                    (self.granted(&obj, node, epoch + 1, true), epoch + 1)
                }
            };
            if let Write::Done(_) = self.replace(&next).map_err(unavailable)? {
                return Ok(self.lease(shard, node, epoch, sent));
            }
        }
        Err(LeaseError::Unavailable(format!(
            "the grant of {shard} lost every race in Kubernetes"
        )))
    }

    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        self.check_range(lease.shard)?;
        self.eligible(
            &lease.owner,
            &[
                MemberStatus::Active,
                MemberStatus::Suspect,
                MemberStatus::Draining,
            ],
        )?;
        let name = lease_name(&self.meta, lease.shard);
        let sent = self.clock.now();
        let obj = self.get(&name).map_err(LeaseError::Unavailable)?;
        let obj = self.check_current(obj.as_ref(), lease)?;
        match self
            .replace(&self.granted(&obj, &lease.owner, lease.epoch, false))
            .map_err(LeaseError::Unavailable)?
        {
            Write::Done(_) => Ok(self.lease(lease.shard, &lease.owner, lease.epoch, sent)),
            Write::Conflict => {
                let obj = self.get(&name).map_err(LeaseError::Unavailable)?;
                self.check_current(obj.as_ref(), lease)?;
                Err(LeaseError::Unavailable(format!(
                    "the renewal of {} raced another change",
                    lease.shard
                )))
            }
        }
    }

    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.check_range(lease.shard)?;
        let name = lease_name(&self.meta, lease.shard);
        for _ in 0..3 {
            let Some(mut obj) = self.get(&name).map_err(LeaseError::Unavailable)? else {
                return Ok(());
            };
            if holder(&obj).as_ref() != Some(&lease.owner) || transitions(&obj) != lease.epoch {
                return Ok(());
            }
            if let Some(spec) = obj["spec"].as_object_mut() {
                spec.remove("holderIdentity");
            }
            if let Some(a) = obj["metadata"]["annotations"].as_object_mut() {
                a.remove(REVOKED);
            }
            if let Write::Done(_) = self.replace(&obj).map_err(LeaseError::Unavailable)? {
                self.lock().renewed.remove(&lease.shard);
                return Ok(());
            }
        }
        Err(LeaseError::Unavailable(format!(
            "the release of {} kept conflicting",
            lease.shard
        )))
    }

    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.check_range(shard)?;
        let name = lease_name(&self.meta, shard);
        for _ in 0..3 {
            let Some(mut obj) = self.get(&name).map_err(LeaseError::Unavailable)? else {
                return Ok(());
            };
            if holder(&obj).is_none() || revoked(&obj) {
                return Ok(());
            }
            obj["metadata"]["annotations"][REVOKED] = Value::String("true".into());
            if let Write::Done(_) = self.replace(&obj).map_err(LeaseError::Unavailable)? {
                return Ok(());
            }
        }
        Err(LeaseError::Unavailable(format!(
            "the revocation of {shard} kept conflicting"
        )))
    }

    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        self.check_range(lease.shard)?;
        self.eligible(to, &[MemberStatus::Active])?;
        let name = lease_name(&self.meta, lease.shard);
        let sent = self.clock.now();
        let obj = self.get(&name).map_err(LeaseError::Unavailable)?;
        let obj = self.check_current(obj.as_ref(), lease)?;
        let epoch = lease.epoch + 1;
        match self
            .replace(&self.granted(&obj, to, epoch, true))
            .map_err(LeaseError::Unavailable)?
        {
            Write::Done(_) => {
                let next = self.lease(lease.shard, to, epoch, sent);
                // the new holder renews it itself; this client no longer holds it
                self.lock().renewed.remove(&lease.shard);
                Ok(next)
            }
            Write::Conflict => Err(LeaseError::Unavailable(format!(
                "the handover of {} raced another change",
                lease.shard
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micro_times_are_rfc3339() {
        assert_eq!(micro_time(UNIX_EPOCH), "1970-01-01T00:00:00.000000Z");
        let t = UNIX_EPOCH + Duration::from_micros(1_794_396_731_123_456);
        assert_eq!(micro_time(t), "2026-11-11T11:32:11.123456Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(micro_time(leap), "2000-02-29T00:00:00.000000Z");
    }

    #[test]
    fn kubeconfigs_are_read() {
        let text = "apiVersion: v1\nclusters:\n- cluster:\n    certificate-authority-data: YWJj\n    server: https://127.0.0.1:6443\n  name: default\nusers:\n- name: default\n  user:\n    client-certificate-data: ZGVm\n    client-key-data: Z2hp\n";
        let k = KubeConfig::from_kubeconfig(text).unwrap();
        assert_eq!(k.server, "https://127.0.0.1:6443");
        assert_eq!(k.namespace, "default");
        assert_eq!(k.ca_pem.as_deref(), Some(&b"abc"[..]));
        assert_eq!(k.client_cert_pem.as_deref(), Some(&b"def"[..]));
        assert_eq!(k.client_key_pem.as_deref(), Some(&b"ghi"[..]));
        assert!(k.token.is_none());
        assert!(KubeConfig::from_kubeconfig("kind: Config").is_err());
    }
}
