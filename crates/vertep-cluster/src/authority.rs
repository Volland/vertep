//! Membership through a lease authority: one service that admits members, detects failures
//! from heartbeats, grants shard leases and publishes versioned views.
//!
//! [`Authority`] is that service's interface. [`MemoryAuthority`] implements it in memory: the
//! static membership hosts one built from its member list, and it is the in-memory stand-in
//! for the etcd and Kubernetes Lease authorities. [`AuthorityMembership`] is the node's side,
//! the [`Membership`] implementation shared by the static and external memberships.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{Clock, Millis};
use crate::lease::{HeldLeases, Lease, LeaseConfig, LeaseError, LeaseTable};
use crate::membership::{
    DetectorConfig, Drain, JoinError, JoinRequest, Membership, MembershipError, check_compatible,
};
use crate::meta::{ClusterMeta, NodeId};
use crate::placement::{ShardId, assignment};
use crate::view::{Member, MemberStatus, ShardOwner, Subscription, View, ViewTracker};

/// A lease authority and member registry, as a service a node calls.
pub trait Authority: Send + Sync {
    /// Admits a node.
    ///
    /// # Errors
    ///
    /// A [`JoinError`] when the node is incompatible, a duplicate, not listed, or the service
    /// is unreachable.
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError>;

    /// Records that `node` is alive; a suspect member becomes active again.
    ///
    /// # Errors
    ///
    /// [`MembershipError::NotMember`] for a node that is not a live member,
    /// [`MembershipError::Unavailable`].
    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError>;

    /// Marks `node` draining: it keeps and may renew its leases but gets no new ones.
    ///
    /// # Errors
    ///
    /// As [`Authority::heartbeat`].
    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError>;

    /// Removes `node` from the view; refused while it holds unexpired leases.
    ///
    /// # Errors
    ///
    /// [`MembershipError::HoldsLeases`], [`MembershipError::Unavailable`].
    fn leave(&self, node: &NodeId) -> Result<View, MembershipError>;

    /// The current view.
    ///
    /// # Errors
    ///
    /// [`MembershipError::Unavailable`].
    fn view(&self) -> Result<View, MembershipError>;

    /// Grants `shard` to `node`, which must be an active member.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the grant.
    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError>;

    /// Renews `lease`.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the renewal.
    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError>;

    /// Releases `lease`.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Unavailable`].
    fn release(&self, lease: &Lease) -> Result<(), LeaseError>;

    /// Revokes the current lease of `shard`.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Unavailable`].
    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError>;

    /// Moves `lease` to `to` at a higher epoch, after its holder drained the shard.
    ///
    /// # Errors
    ///
    /// The [`LeaseError`] of the handover.
    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError>;
}

impl<A: Authority + ?Sized> Authority for Arc<A> {
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError> {
        (**self).join(req)
    }
    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError> {
        (**self).heartbeat(node)
    }
    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError> {
        (**self).begin_drain(node)
    }
    fn leave(&self, node: &NodeId) -> Result<View, MembershipError> {
        (**self).leave(node)
    }
    fn view(&self) -> Result<View, MembershipError> {
        (**self).view()
    }
    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError> {
        (**self).acquire(shard, node)
    }
    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        (**self).renew(lease)
    }
    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        (**self).release(lease)
    }
    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        (**self).revoke(shard)
    }
    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        (**self).handover(lease, to)
    }
}

/// The timing of an authority: leases and failure detection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityConfig {
    /// Lease duration and holder safety margin.
    pub lease: LeaseConfig,
    /// Suspicion and failure times.
    pub detector: DetectorConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    endpoint: String,
    status: MemberStatus,
    last_heard: Millis,
}

#[derive(Debug)]
struct State {
    allow: Option<BTreeSet<NodeId>>,
    members: BTreeMap<NodeId, Record>,
    leases: LeaseTable,
    version: u64,
    published: (
        BTreeMap<NodeId, (String, MemberStatus)>,
        BTreeMap<ShardId, ShardOwner>,
    ),
}

impl State {
    fn snapshot(
        &self,
    ) -> (
        BTreeMap<NodeId, (String, MemberStatus)>,
        BTreeMap<ShardId, ShardOwner>,
    ) {
        (
            self.members
                .iter()
                .map(|(k, r)| (k.clone(), (r.endpoint.clone(), r.status)))
                .collect(),
            self.leases.owners(),
        )
    }

    /// Bumps the version once if members, statuses or owners changed since the last view.
    fn publish(&mut self) {
        let snap = self.snapshot();
        if snap != self.published {
            self.version += 1;
            self.published = snap;
        }
    }

    /// Applies failure detection at `now`.
    fn detect(&mut self, detector: DetectorConfig, now: Millis) {
        for r in self.members.values_mut() {
            let silence = now.since(r.last_heard);
            r.status = match r.status {
                MemberStatus::Failed => MemberStatus::Failed,
                _ if silence >= detector.failure_time => MemberStatus::Failed,
                MemberStatus::Active if silence >= detector.suspicion_time => MemberStatus::Suspect,
                s => s,
            };
        }
    }

    fn status(&self, node: &NodeId) -> Option<MemberStatus> {
        self.members.get(node).map(|r| r.status)
    }
}

/// A lease authority and member registry held in memory, driven by an injected clock.
///
/// Failure detection is by heartbeat: a member silent for the suspicion time is suspect, for
/// the failure time failed; every change is published as a new view version. A failed
/// member's leases are left to expire, never taken early.
///
/// It is the authority a static cluster hosts, and the in-memory stand-in for the etcd and
/// Kubernetes Lease authorities. [`MemoryAuthority::set_reachable`] simulates losing it.
// @tg: implements:: [[openspec:cluster-membership#Failure Detection]]
// @tg: implements:: [[openspec:cluster-membership#Static Membership]]
// @tg: implements:: [[openspec:cluster-membership#External Membership]] {confidence: partial}
#[derive(Debug)]
pub struct MemoryAuthority {
    meta: ClusterMeta,
    config: AuthorityConfig,
    clock: Arc<dyn Clock>,
    reachable: AtomicBool,
    state: Mutex<State>,
}

impl MemoryAuthority {
    /// An authority admitting any compatible node, with no members yet: the in-memory stand-in
    /// for an external coordination service.
    pub fn new(meta: ClusterMeta, config: AuthorityConfig, clock: Arc<dyn Clock>) -> Self {
        let leases = LeaseTable::new(meta.shard_count, config.lease);
        let state = State {
            allow: None,
            members: BTreeMap::new(),
            leases,
            version: 0,
            published: (BTreeMap::new(), BTreeMap::new()),
        };
        MemoryAuthority {
            meta,
            config,
            clock,
            reachable: AtomicBool::new(true),
            state: Mutex::new(state),
        }
    }

    /// The authority of a static cluster: the listed members are active from the start, and
    /// no other node can join.
    pub fn with_static_members(
        meta: ClusterMeta,
        config: AuthorityConfig,
        clock: Arc<dyn Clock>,
        members: &[(NodeId, String)],
    ) -> Self {
        let a = MemoryAuthority::new(meta, config, clock);
        {
            let now = a.clock.now();
            let mut st = a.lock();
            st.allow = Some(members.iter().map(|(n, _)| n.clone()).collect());
            for (n, e) in members {
                st.members.insert(
                    n.clone(),
                    Record {
                        endpoint: e.clone(),
                        status: MemberStatus::Active,
                        last_heard: now,
                    },
                );
            }
            st.publish();
        }
        a
    }

    /// The cluster metadata.
    pub fn meta(&self) -> &ClusterMeta {
        &self.meta
    }

    /// The timing.
    pub fn config(&self) -> AuthorityConfig {
        self.config
    }

    /// Makes the authority reachable or not, for every caller.
    pub fn set_reachable(&self, up: bool) {
        self.reachable.store(up, Ordering::SeqCst);
    }

    /// Every unexpired lease now, by the authority's clock.
    pub fn live_leases(&self) -> Vec<Lease> {
        let now = self.clock.now();
        self.lock().leases.live(now)
    }

    /// Runs failure detection now and returns the view.
    pub fn tick(&self) -> View {
        let mut st = self.begin();
        self.view_of(&mut st)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn up(&self) -> Result<(), String> {
        if self.reachable.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(format!(
                "the lease authority of cluster {} is unreachable",
                self.meta.cluster
            ))
        }
    }

    /// Locks the state and applies failure detection at the current time.
    fn begin(&self) -> MutexGuard<'_, State> {
        let now = self.clock.now();
        let mut st = self.lock();
        st.detect(self.config.detector, now);
        st.publish();
        st
    }

    fn view_of(&self, st: &mut State) -> View {
        st.publish();
        View {
            cluster: self.meta.cluster.clone(),
            version: st.version,
            members: st
                .members
                .iter()
                .map(|(k, r)| {
                    (
                        k.clone(),
                        Member {
                            id: k.clone(),
                            endpoint: r.endpoint.clone(),
                            status: r.status,
                        },
                    )
                })
                .collect(),
            owners: st.leases.owners(),
        }
    }
}

impl Authority for MemoryAuthority {
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError> {
        self.up().map_err(JoinError::Unavailable)?;
        check_compatible(&self.meta, req)?;
        let now = self.clock.now();
        let mut st = self.begin();
        if st.allow.as_ref().is_some_and(|a| !a.contains(&req.node)) {
            return Err(JoinError::NotListed(req.node.clone()));
        }
        if let Some(r) = st.members.get(&req.node) {
            // a listed static member reconnecting with its own endpoint is a heartbeat
            let same_static = st.allow.is_some() && r.endpoint == req.endpoint;
            if r.status != MemberStatus::Failed && !same_static {
                return Err(JoinError::DuplicateNode(req.node.clone()));
            }
        }
        st.members.insert(
            req.node.clone(),
            Record {
                endpoint: req.endpoint.clone(),
                status: MemberStatus::Active,
                last_heard: now,
            },
        );
        Ok(self.view_of(&mut st))
    }

    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError> {
        self.up().map_err(MembershipError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        match st.members.get_mut(node) {
            Some(r) if r.status != MemberStatus::Failed => {
                r.last_heard = now;
                if r.status == MemberStatus::Suspect {
                    r.status = MemberStatus::Active;
                }
                st.publish();
                Ok(())
            }
            _ => Err(MembershipError::NotMember(node.clone())),
        }
    }

    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError> {
        self.up().map_err(MembershipError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        match st.members.get_mut(node) {
            Some(r) if r.status != MemberStatus::Failed => {
                r.status = MemberStatus::Draining;
                r.last_heard = now;
            }
            _ => return Err(MembershipError::NotMember(node.clone())),
        }
        Ok(self.view_of(&mut st))
    }

    fn leave(&self, node: &NodeId) -> Result<View, MembershipError> {
        self.up().map_err(MembershipError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        let held: Vec<ShardId> = st
            .leases
            .held_by(node, now)
            .into_iter()
            .map(|l| l.shard)
            .collect();
        if !held.is_empty() {
            return Err(MembershipError::HoldsLeases {
                node: node.clone(),
                shards: held,
            });
        }
        st.members.remove(node);
        Ok(self.view_of(&mut st))
    }

    fn view(&self) -> Result<View, MembershipError> {
        self.up().map_err(MembershipError::Unavailable)?;
        let mut st = self.begin();
        Ok(self.view_of(&mut st))
    }

    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError> {
        self.up().map_err(LeaseError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        match st.status(node) {
            Some(MemberStatus::Active) => {}
            status => {
                return Err(LeaseError::NotEligible {
                    node: node.clone(),
                    status,
                });
            }
        }
        let lease = st.leases.acquire(shard, node, now)?;
        st.publish();
        Ok(lease)
    }

    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        self.up().map_err(LeaseError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        match st.status(&lease.owner) {
            Some(MemberStatus::Active | MemberStatus::Suspect | MemberStatus::Draining) => {}
            status => {
                return Err(LeaseError::NotEligible {
                    node: lease.owner.clone(),
                    status,
                });
            }
        }
        st.leases.renew(lease, now)
    }

    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.up().map_err(LeaseError::Unavailable)?;
        let mut st = self.begin();
        st.leases.release(lease)?;
        st.publish();
        Ok(())
    }

    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.up().map_err(LeaseError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        st.leases.revoke(shard, now)
    }

    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        self.up().map_err(LeaseError::Unavailable)?;
        let now = self.clock.now();
        let mut st = self.begin();
        match st.status(to) {
            Some(MemberStatus::Active) => {}
            status => {
                return Err(LeaseError::NotEligible {
                    node: to.clone(),
                    status,
                });
            }
        }
        let out = st.leases.handover(lease, to, now)?;
        st.publish();
        Ok(out)
    }
}

/// One node's connection to an authority, which can be cut: a network partition between that
/// node and the authority, while other nodes still reach it.
#[derive(Clone, Debug)]
pub struct AuthorityLink<A> {
    inner: A,
    up: Arc<AtomicBool>,
}

impl<A> AuthorityLink<A> {
    /// A connected link to `inner`.
    pub fn new(inner: A) -> Self {
        AuthorityLink {
            inner,
            up: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Connects or cuts the link.
    pub fn set_connected(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), String> {
        if self.up.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("partitioned from the lease authority".into())
        }
    }
}

impl<A: Authority> Authority for AuthorityLink<A> {
    fn join(&self, req: &JoinRequest) -> Result<View, JoinError> {
        self.check().map_err(JoinError::Unavailable)?;
        self.inner.join(req)
    }
    fn heartbeat(&self, node: &NodeId) -> Result<(), MembershipError> {
        self.check().map_err(MembershipError::Unavailable)?;
        self.inner.heartbeat(node)
    }
    fn begin_drain(&self, node: &NodeId) -> Result<View, MembershipError> {
        self.check().map_err(MembershipError::Unavailable)?;
        self.inner.begin_drain(node)
    }
    fn leave(&self, node: &NodeId) -> Result<View, MembershipError> {
        self.check().map_err(MembershipError::Unavailable)?;
        self.inner.leave(node)
    }
    fn view(&self) -> Result<View, MembershipError> {
        self.check().map_err(MembershipError::Unavailable)?;
        self.inner.view()
    }
    fn acquire(&self, shard: ShardId, node: &NodeId) -> Result<Lease, LeaseError> {
        self.check().map_err(LeaseError::Unavailable)?;
        self.inner.acquire(shard, node)
    }
    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        self.check().map_err(LeaseError::Unavailable)?;
        self.inner.renew(lease)
    }
    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.check().map_err(LeaseError::Unavailable)?;
        self.inner.release(lease)
    }
    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.check().map_err(LeaseError::Unavailable)?;
        self.inner.revoke(shard)
    }
    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        self.check().map_err(LeaseError::Unavailable)?;
        self.inner.handover(lease, to)
    }
}

#[derive(Debug)]
struct Client {
    tracker: ViewTracker,
    held: HeldLeases,
    draining: bool,
}

/// A node's membership through an [`Authority`]: the static membership (authority hosted from
/// the static member list) and the external memberships (etcd, Kubernetes) are this type over
/// different authorities.
///
/// The node keeps its own view (adopting only newer versions) and its own record of held
/// leases judged by its own clock. [`AuthorityMembership::tick`] is its periodic duty:
/// heartbeat, fetch the view, take over shards handed to it, renew leases due.
// @tg: implements:: [[openspec:cluster-membership#Membership Trait]]
// @tg: implements:: [[openspec:cluster-membership#Graceful Join]]
// @tg: implements:: [[openspec:cluster-membership#Graceful Leave With Drain]] {confidence: partial}
// @tg: implements:: [[openspec:cluster-membership#Lease Expiry And Handover]]
#[derive(Debug)]
pub struct AuthorityMembership<A> {
    authority: A,
    meta: ClusterMeta,
    local: NodeId,
    endpoint: String,
    clock: Arc<dyn Clock>,
    state: Mutex<Client>,
}

impl<A: Authority> AuthorityMembership<A> {
    /// The membership of node `local` at `endpoint`, reaching the cluster through `authority`,
    /// timing its leases on `clock` with safety margin `lease.safety_margin`.
    pub fn new(
        authority: A,
        meta: ClusterMeta,
        local: NodeId,
        endpoint: impl Into<String>,
        clock: Arc<dyn Clock>,
        lease: LeaseConfig,
    ) -> Self {
        let tracker = ViewTracker::new(View::empty(meta.cluster.clone()));
        AuthorityMembership {
            authority,
            meta,
            local,
            endpoint: endpoint.into(),
            clock,
            state: Mutex::new(Client {
                tracker,
                held: HeldLeases::new(lease.safety_margin),
                draining: false,
            }),
        }
    }

    /// The authority this node talks to.
    pub fn authority(&self) -> &A {
        &self.authority
    }

    fn lock(&self) -> MutexGuard<'_, Client> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn adopt(client: &mut Client, view: View) -> bool {
        if client.tracker.offer(view) {
            let current = client.tracker.current().clone();
            client.held.retain_current(&current);
            true
        } else {
            false
        }
    }

    /// Joins the cluster.
    ///
    /// # Errors
    ///
    /// The authority's [`JoinError`].
    pub fn join(&self) -> Result<View, JoinError> {
        let req = JoinRequest::new(&self.meta, self.local.clone(), self.endpoint.clone());
        let view = self.authority.join(&req)?;
        let mut c = self.lock();
        Self::adopt(&mut c, view);
        Ok(c.tracker.current().clone())
    }

    /// Fetches the authority's view and adopts it if newer. Returns whether it was adopted.
    ///
    /// # Errors
    ///
    /// [`MembershipError::Unavailable`].
    pub fn poll(&self) -> Result<bool, MembershipError> {
        let view = self.authority.view()?;
        Ok(Self::adopt(&mut self.lock(), view))
    }

    /// The periodic duty: heartbeat, poll the view, take over the shards the view shows
    /// handed to this node, renew the leases whose term is half over.
    ///
    /// # Errors
    ///
    /// The first error of the heartbeat or the poll; lease errors only drop the lease.
    pub fn tick(&self) -> Result<(), MembershipError> {
        match self.authority.heartbeat(&self.local) {
            // marked failed (or never joined): rejoin, learning the current owners and epochs
            Err(MembershipError::NotMember(_)) if !self.lock().draining => {
                self.join()?;
            }
            other => other?,
        }
        self.poll()?;
        let (view, draining) = {
            let c = self.lock();
            (c.tracker.current().clone(), c.draining)
        };
        if !draining {
            for shard in view.shards_of(&self.local).collect::<Vec<_>>() {
                if self.lock().held.all().all(|l| l.shard != shard) {
                    // a shard handed to this node: hold it by renewing it ourselves
                    let _ = self.acquire(shard);
                }
            }
        }
        let now = self.clock.now();
        let due = self.lock().held.due(now);
        for lease in due {
            if self.renew(&lease).is_err() {
                self.forget(lease.shard);
            }
        }
        Ok(())
    }

    /// Acquires every shard the deterministic assignment of the current view gives this node.
    /// Returns the shards acquired; shards still leased elsewhere are skipped.
    pub fn claim_assigned(&self) -> Vec<ShardId> {
        let view = self.view();
        assignment(&view, self.meta.shard_count)
            .into_iter()
            .filter(|(_, n)| n == &self.local)
            .filter_map(|(s, _)| self.acquire(s).ok().map(|l| l.shard))
            .collect()
    }

    /// Leaves gracefully: marks this node draining so it gets no new leases, drains each held
    /// shard through `drain` and hands it to the active member the assignment of the remaining
    /// members gives it, at a higher epoch, and only then leaves the view.
    ///
    /// # Errors
    ///
    /// [`MembershipError::Drain`] when a shard cannot be drained (its lease stays),
    /// [`MembershipError::NoSuccessor`] when no active member is left, and authority errors.
    pub fn leave(&self, drain: &mut dyn Drain) -> Result<(), MembershipError> {
        let view = self.authority.begin_drain(&self.local)?;
        let leases: Vec<Lease> = {
            let mut c = self.lock();
            c.draining = true;
            Self::adopt(&mut c, view);
            c.held.all().cloned().collect()
        };
        for lease in leases {
            drain
                .drain_shard(lease.shard)
                .map_err(|reason| MembershipError::Drain {
                    shard: lease.shard,
                    reason,
                })?;
            let view = self.authority.view()?;
            let others: Vec<NodeId> = view
                .active()
                .filter(|m| m.id != self.local)
                .map(|m| m.id.clone())
                .collect();
            if others.is_empty() {
                return Err(MembershipError::NoSuccessor(lease.shard));
            }
            let to = &others[lease.shard.index() % others.len()];
            match Membership::handover(self, &lease, to) {
                Ok(_) => {}
                Err(LeaseError::Expired { .. } | LeaseError::Stale { .. }) => {
                    self.lock().held.forget(lease.shard);
                }
                Err(e) => return Err(e.into()),
            }
        }
        let view = self.authority.leave(&self.local)?;
        Self::adopt(&mut self.lock(), view);
        Ok(())
    }

    /// The local instant at which the lease on `shard` stops counting as held.
    pub fn held_until(&self, shard: ShardId) -> Option<Millis> {
        self.lock().held.deadline(shard)
    }
}

impl<A: Authority> Membership for AuthorityMembership<A> {
    fn local(&self) -> &NodeId {
        &self.local
    }

    fn meta(&self) -> &ClusterMeta {
        &self.meta
    }

    fn view(&self) -> View {
        self.lock().tracker.current().clone()
    }

    fn subscribe(&self) -> Subscription {
        self.lock().tracker.subscribe()
    }

    fn acquire(&self, shard: ShardId) -> Result<Lease, LeaseError> {
        if self.lock().draining {
            return Err(LeaseError::NotEligible {
                node: self.local.clone(),
                status: Some(MemberStatus::Draining),
            });
        }
        let sent = self.clock.now();
        let lease = self.authority.acquire(shard, &self.local)?;
        self.lock().held.record(lease.clone(), sent);
        let _ = self.poll();
        Ok(lease)
    }

    fn renew(&self, lease: &Lease) -> Result<Lease, LeaseError> {
        let sent = self.clock.now();
        let renewed = self.authority.renew(lease)?;
        self.lock().held.record(renewed.clone(), sent);
        Ok(renewed)
    }

    fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        self.lock().held.forget(lease.shard);
        self.authority.release(lease)?;
        let _ = self.poll();
        Ok(())
    }

    fn revoke(&self, shard: ShardId) -> Result<(), LeaseError> {
        self.authority.revoke(shard)
    }

    fn handover(&self, lease: &Lease, to: &NodeId) -> Result<Lease, LeaseError> {
        let next = self.authority.handover(lease, to)?;
        self.lock().held.forget(lease.shard);
        let _ = self.poll();
        Ok(next)
    }

    fn held(&self, shard: ShardId) -> Option<Lease> {
        let now = self.clock.now();
        self.lock().held.valid(shard, now).cloned()
    }

    fn forget(&self, shard: ShardId) {
        self.lock().held.forget(shard);
    }
}

/// The configuration of a static cluster: the metadata, the member list and the timing.
#[derive(Clone, Debug)]
// @tg: implements:: [[openspec:cluster-membership#Static Membership]]
pub struct StaticConfig {
    /// The cluster metadata.
    pub meta: ClusterMeta,
    /// Every member's identifier and endpoint.
    pub members: Vec<(NodeId, String)>,
    /// Lease and failure detection timing.
    pub timing: AuthorityConfig,
}

impl StaticConfig {
    /// The lease authority of this static cluster, hosted by one process.
    pub fn authority(&self, clock: Arc<dyn Clock>) -> MemoryAuthority {
        MemoryAuthority::with_static_members(self.meta.clone(), self.timing, clock, &self.members)
    }

    /// The membership of listed node `node`, reaching the cluster's authority through
    /// `authority`.
    ///
    /// # Errors
    ///
    /// [`JoinError::NotListed`] when `node` is not in the member list.
    pub fn membership<A: Authority>(
        &self,
        node: &NodeId,
        authority: A,
        clock: Arc<dyn Clock>,
    ) -> Result<StaticMembership<A>, JoinError> {
        let endpoint = self
            .members
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, e)| e.clone())
            .ok_or_else(|| JoinError::NotListed(node.clone()))?;
        let m = AuthorityMembership::new(
            authority,
            self.meta.clone(),
            node.clone(),
            endpoint,
            clock,
            self.timing.lease,
        );
        Ok(m)
    }
}

/// The static membership: members from configuration, leases from the hosted authority,
/// assigned deterministically with [`AuthorityMembership::claim_assigned`].
pub type StaticMembership<A> = AuthorityMembership<A>;

/// An external membership: an etcd or Kubernetes Lease authority (or its in-memory stand-in,
/// [`MemoryAuthority`]) is the authority for members and leases.
pub type ExternalMembership<A> = AuthorityMembership<A>;
