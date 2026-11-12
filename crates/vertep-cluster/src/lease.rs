//! Shard leases: time-bounded ownership of a shard with a fencing epoch that rises on every
//! grant.
//!
//! [`LeaseTable`] is the authority's side, a pure state machine driven by the authority's
//! clock; every membership implementation grants leases through one. [`HeldLeases`] is the
//! holder's side: a lease counts as held only until `request time + term - safety margin` on
//! the holder's own clock, so the holder stops before the authority may regrant the shard.

use core::fmt;
use std::collections::BTreeMap;
use std::time::Duration;

use vertep_store::FencingToken;

use crate::clock::Millis;
use crate::meta::{ClusterMeta, NodeId, ShardCount};
use crate::placement::ShardId;
use crate::view::{MemberStatus, ShardOwner, View};

/// The timing of shard leases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaseConfig {
    /// How long a grant or renewal lasts, on the authority's clock.
    pub duration: Duration,
    /// How long before the end of its term a holder stops starting macrosteps, to absorb clock
    /// drift and pauses between the holder and the authority.
    pub safety_margin: Duration,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        LeaseConfig {
            duration: Duration::from_secs(10),
            safety_margin: Duration::from_secs(2),
        }
    }
}

/// A shard lease as granted by the lease authority.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]
pub struct Lease {
    /// The leased shard.
    pub shard: ShardId,
    /// The holder.
    pub owner: NodeId,
    /// The fencing epoch, strictly greater than every epoch granted before for the shard.
    /// Renewals keep it.
    pub epoch: u64,
    /// The expiry on the authority's clock.
    pub expires_at: Millis,
    /// The term of the grant or renewal that produced this lease.
    pub term: Duration,
}

impl Lease {
    /// The fencing token every journal append under this lease carries: the shard's scope IRI
    /// and the lease epoch.
    pub fn fencing_token(&self, meta: &ClusterMeta) -> FencingToken {
        FencingToken::new(meta.shard_scope(self.shard), self.epoch)
    }
}

/// Why a lease operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseError {
    /// Another node holds an unexpired lease on the shard.
    Held {
        /// The shard.
        shard: ShardId,
        /// The holder.
        owner: NodeId,
        /// The end of its lease on the authority's clock.
        until: Millis,
    },
    /// The lease expired before it was renewed.
    Expired {
        /// The shard.
        shard: ShardId,
        /// The expired epoch.
        epoch: u64,
    },
    /// The lease was superseded by a grant at a higher epoch.
    Stale {
        /// The shard.
        shard: ShardId,
        /// The epoch presented.
        epoch: u64,
        /// The epoch of the current grant.
        current: u64,
    },
    /// The authority revoked the lease; it cannot be renewed, and the shard is regranted when
    /// the holder releases it or it expires.
    Revoked {
        /// The shard.
        shard: ShardId,
    },
    /// The node may not hold leases now (it is not an active member).
    NotEligible {
        /// The node.
        node: NodeId,
        /// Its status, if it is a member at all.
        status: Option<MemberStatus>,
    },
    /// The shard number is not below the shard count.
    OutOfRange {
        /// The shard.
        shard: ShardId,
        /// The shard count.
        count: ShardCount,
    },
    /// The lease authority cannot be reached, or will not grant yet; nothing changed.
    Unavailable(String),
    /// The request was sent to a remote lease authority and has no answer yet; ask again.
    Pending {
        /// The shard.
        shard: ShardId,
    },
}

impl LeaseError {
    /// True for [`LeaseError::Pending`].
    pub fn is_pending(&self) -> bool {
        matches!(self, LeaseError::Pending { .. })
    }
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeaseError::Held {
                shard,
                owner,
                until,
            } => write!(f, "{shard} is leased to {owner} until {until}"),
            LeaseError::Expired { shard, epoch } => {
                write!(f, "the lease on {shard} at epoch {epoch} expired")
            }
            LeaseError::Stale {
                shard,
                epoch,
                current,
            } => write!(
                f,
                "the lease on {shard} at epoch {epoch} was superseded by epoch {current}"
            ),
            LeaseError::Revoked { shard } => write!(f, "the lease on {shard} was revoked"),
            LeaseError::NotEligible { node, status } => match status {
                Some(s) => write!(f, "{node} is {s} and cannot hold leases"),
                None => write!(f, "{node} is not a member and cannot hold leases"),
            },
            LeaseError::OutOfRange { shard, count } => {
                write!(f, "{shard} is out of range for {count} shards")
            }
            LeaseError::Unavailable(m) => write!(f, "lease authority unavailable: {m}"),
            LeaseError::Pending { shard } => {
                write!(f, "the lease request for {shard} is in flight")
            }
        }
    }
}

impl std::error::Error for LeaseError {}

/// How a lease authority chooses the epoch of a new grant from the highest epoch it knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochPolicy {
    /// `highest + 1`, for a single authority (static, external).
    Sequential,
    /// `((highest >> 16) + 1) << 16 | tag`, for authorities that may run concurrently (a gossip
    /// leader on each side of a partition): two authorities with different tags never grant the
    /// same epoch, so each epoch has at most one writer.
    Tagged(u16),
}

impl EpochPolicy {
    /// The epoch of the next grant after `highest`.
    pub fn next(self, highest: u64) -> u64 {
        match self {
            EpochPolicy::Sequential => highest + 1,
            EpochPolicy::Tagged(tag) => (((highest >> 16) + 1) << 16) | u64::from(tag),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Grant {
    owner: NodeId,
    epoch: u64,
    expires_at: Millis,
    term: Duration,
    revoked: bool,
}

impl Grant {
    fn lease(&self, shard: ShardId) -> Lease {
        Lease {
            shard,
            owner: self.owner.clone(),
            epoch: self.epoch,
            expires_at: self.expires_at,
            term: self.term,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    highest: u64,
    grant: Option<Grant>,
}

/// The lease authority's table: per shard, the current grant and the highest epoch ever
/// granted.
///
/// Rules: a shard has at most one unexpired grant; a new grant's epoch is strictly greater
/// than every earlier one; renewal keeps the epoch and moves the expiry; a revoked or expired
/// grant cannot be renewed, and a revoked grant blocks new grants until the holder releases it
/// or it expires, so the old holder has stopped before anyone else starts.
// @tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]
// @tg: implements:: [[openspec:cluster-membership#Lease Expiry And Handover]]
#[derive(Clone, Debug)]
pub struct LeaseTable {
    count: ShardCount,
    config: LeaseConfig,
    policy: EpochPolicy,
    slots: Vec<Slot>,
    blocked_until: Millis,
}

impl LeaseTable {
    /// A table for `count` shards with no grants.
    pub fn new(count: ShardCount, config: LeaseConfig) -> Self {
        LeaseTable {
            count,
            config,
            policy: EpochPolicy::Sequential,
            slots: vec![Slot::default(); count.get() as usize],
            blocked_until: Millis(0),
        }
    }

    /// This table with epoch policy `policy`.
    pub fn with_policy(mut self, policy: EpochPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The lease timing.
    pub fn config(&self) -> LeaseConfig {
        self.config
    }

    fn slot(&mut self, shard: ShardId) -> Result<&mut Slot, LeaseError> {
        let count = self.count;
        self.slots
            .get_mut(shard.index())
            .ok_or(LeaseError::OutOfRange { shard, count })
    }

    /// Grants `shard` to `node`. If `node` already holds an unexpired, unrevoked lease on it,
    /// that lease is renewed with its epoch unchanged.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Held`] while another node's grant is unexpired (revoked or not),
    /// [`LeaseError::Revoked`] when `node`'s own grant was revoked,
    /// [`LeaseError::Unavailable`] during a takeover grace period, [`LeaseError::OutOfRange`].
    pub fn acquire(
        &mut self,
        shard: ShardId,
        node: &NodeId,
        now: Millis,
    ) -> Result<Lease, LeaseError> {
        let (config, policy, blocked) = (self.config, self.policy, self.blocked_until);
        let slot = self.slot(shard)?;
        if let Some(g) = &mut slot.grant
            && now < g.expires_at
        {
            if &g.owner != node {
                return Err(LeaseError::Held {
                    shard,
                    owner: g.owner.clone(),
                    until: g.expires_at,
                });
            }
            if g.revoked {
                return Err(LeaseError::Revoked { shard });
            }
            g.expires_at = now.after(config.duration);
            g.term = config.duration;
            return Ok(g.lease(shard));
        }
        if now < blocked {
            return Err(LeaseError::Unavailable(format!(
                "the lease authority grants no new leases before {blocked}"
            )));
        }
        let epoch = policy.next(slot.highest);
        debug_assert!(epoch > slot.highest);
        slot.highest = epoch;
        let g = Grant {
            owner: node.clone(),
            epoch,
            expires_at: now.after(config.duration),
            term: config.duration,
            revoked: false,
        };
        let lease = g.lease(shard);
        slot.grant = Some(g);
        Ok(lease)
    }

    /// Renews `lease`: moves its expiry to `now + duration` and keeps its epoch.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Expired`] after expiry, [`LeaseError::Stale`] when a newer grant exists,
    /// [`LeaseError::Revoked`] after revocation.
    pub fn renew(&mut self, lease: &Lease, now: Millis) -> Result<Lease, LeaseError> {
        let config = self.config;
        let shard = lease.shard;
        let slot = self.slot(shard)?;
        match &mut slot.grant {
            Some(g) if g.epoch == lease.epoch && g.owner == lease.owner => {
                if g.revoked {
                    Err(LeaseError::Revoked { shard })
                } else if now >= g.expires_at {
                    Err(LeaseError::Expired {
                        shard,
                        epoch: lease.epoch,
                    })
                } else {
                    g.expires_at = now.after(config.duration);
                    g.term = config.duration;
                    Ok(g.lease(shard))
                }
            }
            Some(g) => Err(LeaseError::Stale {
                shard,
                epoch: lease.epoch,
                current: g.epoch,
            }),
            None => Err(LeaseError::Expired {
                shard,
                epoch: lease.epoch,
            }),
        }
    }

    /// Releases `lease`, freeing the shard at once. Releasing a lease that is no longer the
    /// current grant changes nothing.
    ///
    /// # Errors
    ///
    /// [`LeaseError::OutOfRange`].
    pub fn release(&mut self, lease: &Lease) -> Result<(), LeaseError> {
        let slot = self.slot(lease.shard)?;
        if slot
            .grant
            .as_ref()
            .is_some_and(|g| g.epoch == lease.epoch && g.owner == lease.owner)
        {
            slot.grant = None;
        }
        Ok(())
    }

    /// Revokes the current grant of `shard`: it can no longer be renewed, and the shard is
    /// regranted only once the holder releases it or it expires.
    ///
    /// # Errors
    ///
    /// [`LeaseError::OutOfRange`].
    pub fn revoke(&mut self, shard: ShardId, now: Millis) -> Result<(), LeaseError> {
        let slot = self.slot(shard)?;
        if let Some(g) = &mut slot.grant
            && now < g.expires_at
        {
            g.revoked = true;
        }
        Ok(())
    }

    /// Moves `lease` to `to` at a higher epoch, for a planned handover after the holder
    /// drained the shard. The new lease starts now; `to` holds it once it renews it.
    ///
    /// # Errors
    ///
    /// As [`LeaseTable::renew`] when `lease` is not the current, live grant.
    pub fn handover(
        &mut self,
        lease: &Lease,
        to: &NodeId,
        now: Millis,
    ) -> Result<Lease, LeaseError> {
        self.renew(lease, now)?;
        let (config, policy) = (self.config, self.policy);
        let slot = self.slot(lease.shard)?;
        let epoch = policy.next(slot.highest);
        slot.highest = epoch;
        let g = Grant {
            owner: to.clone(),
            epoch,
            expires_at: now.after(config.duration),
            term: config.duration,
            revoked: false,
        };
        let out = g.lease(lease.shard);
        slot.grant = Some(g);
        Ok(out)
    }

    /// The unexpired grant of `shard`, if any.
    pub fn current(&self, shard: ShardId, now: Millis) -> Option<Lease> {
        self.slots
            .get(shard.index())?
            .grant
            .as_ref()
            .filter(|g| now < g.expires_at)
            .map(|g| g.lease(shard))
    }

    /// Every unexpired grant.
    pub fn live(&self, now: Millis) -> Vec<Lease> {
        self.count
            .shards()
            .filter_map(|s| self.current(s, now))
            .collect()
    }

    /// The unexpired grants held by `node`.
    pub fn held_by(&self, node: &NodeId, now: Millis) -> Vec<Lease> {
        self.live(now)
            .into_iter()
            .filter(|l| &l.owner == node)
            .collect()
    }

    /// Raises the highest epoch known for `shard` to `epoch`, without a grant: the next grant
    /// will be above it.
    pub fn note_epoch(&mut self, shard: ShardId, epoch: u64) {
        if let Some(slot) = self.slots.get_mut(shard.index()) {
            slot.highest = slot.highest.max(epoch);
        }
    }

    /// The highest epoch ever granted for `shard`.
    pub fn highest_epoch(&self, shard: ShardId) -> u64 {
        self.slots.get(shard.index()).map_or(0, |s| s.highest)
    }

    /// The last grant of every shard that has one, expired or not: what routing should use.
    pub fn owners(&self) -> BTreeMap<ShardId, ShardOwner> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.grant.as_ref().map(|g| {
                    (
                        ShardId::new(i as u32),
                        ShardOwner {
                            node: g.owner.clone(),
                            epoch: g.epoch,
                        },
                    )
                })
            })
            .collect()
    }

    /// Records a grant made by another authority (a previous gossip leader), so this table
    /// never grants that epoch again and treats the grant as live until `expires_at`.
    pub fn seed(&mut self, shard: ShardId, owner: &NodeId, epoch: u64, expires_at: Millis) {
        let term = self.config.duration;
        let Some(slot) = self.slots.get_mut(shard.index()) else {
            return;
        };
        slot.highest = slot.highest.max(epoch);
        let newer = slot.grant.as_ref().is_none_or(|g| g.epoch <= epoch);
        if newer {
            slot.grant = Some(Grant {
                owner: owner.clone(),
                epoch,
                expires_at,
                term,
                revoked: false,
            });
        }
    }

    /// Refuses new grants (not renewals) before `until`: a new authority waits out every lease
    /// a previous authority may have granted without it knowing.
    pub fn block_new_grants_until(&mut self, until: Millis) {
        self.blocked_until = self.blocked_until.max(until);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    lease: Lease,
    requested_at: Millis,
}

impl Held {
    fn deadline(&self, margin: Duration) -> Millis {
        self.requested_at.after(self.lease.term).before(margin)
    }
}

/// The leases a node holds, judged by its own clock.
///
/// A lease granted or renewed by a request sent at local time `t` counts as held until
/// `t + term - safety margin`: the authority started the term after `t`, so without clock
/// drift the holder stops at least a safety margin before the authority may regrant.
// @tg: implements:: [[openspec:cluster-membership#Lease Expiry And Handover]]
#[derive(Clone, Debug, Default)]
pub struct HeldLeases {
    margin: Duration,
    held: BTreeMap<ShardId, Held>,
}

impl HeldLeases {
    /// No lease, with safety margin `margin`.
    pub fn new(margin: Duration) -> Self {
        HeldLeases {
            margin,
            held: BTreeMap::new(),
        }
    }

    /// Records `lease`, granted or renewed by a request sent at local time `requested_at`.
    pub fn record(&mut self, lease: Lease, requested_at: Millis) {
        self.held.insert(
            lease.shard,
            Held {
                lease,
                requested_at,
            },
        );
    }

    /// The lease on `shard` if it is still held at local time `now`: the holder may start a
    /// macrostep for the shard's actors only while this is `Some`.
    pub fn valid(&self, shard: ShardId, now: Millis) -> Option<&Lease> {
        self.held
            .get(&shard)
            .filter(|h| now < h.deadline(self.margin))
            .map(|h| &h.lease)
    }

    /// The local instant at which the lease on `shard` stops counting as held.
    pub fn deadline(&self, shard: ShardId) -> Option<Millis> {
        self.held.get(&shard).map(|h| h.deadline(self.margin))
    }

    /// Forgets the lease on `shard` (released, superseded or fenced), returning it.
    pub fn forget(&mut self, shard: ShardId) -> Option<Lease> {
        self.held.remove(&shard).map(|h| h.lease)
    }

    /// Every recorded lease, valid or not.
    pub fn all(&self) -> impl Iterator<Item = &Lease> {
        self.held.values().map(|h| &h.lease)
    }

    /// The recorded leases still valid at `now` whose first half of term has passed: renew
    /// them now.
    pub fn due(&self, now: Millis) -> Vec<Lease> {
        self.held
            .values()
            .filter(|h| now >= h.requested_at.after(h.lease.term / 2))
            .filter(|h| now < h.deadline(self.margin))
            .map(|h| h.lease.clone())
            .collect()
    }

    /// Forgets every lease that `view` shows superseded: another owner, or a higher epoch.
    /// Returns the shards forgotten.
    pub fn retain_current(&mut self, view: &View) -> Vec<ShardId> {
        let stale: Vec<ShardId> = self
            .held
            .values()
            .filter(|h| {
                view.owner(h.lease.shard).is_some_and(|o| {
                    o.epoch > h.lease.epoch || (o.epoch == h.lease.epoch && o.node != h.lease.owner)
                })
            })
            .map(|h| h.lease.shard)
            .collect();
        for s in &stale {
            self.held.remove(s);
        }
        stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(s: &str) -> NodeId {
        NodeId::new(s).unwrap()
    }

    fn table() -> LeaseTable {
        LeaseTable::new(ShardCount::new(64).unwrap(), LeaseConfig::default())
    }

    #[test]
    fn revoked_lease_blocks_regrant_until_release_or_expiry() {
        let mut t = table();
        let s = ShardId::new(1);
        let l = t.acquire(s, &node("a"), Millis(0)).unwrap();
        t.revoke(s, Millis(1)).unwrap();
        assert!(matches!(
            t.renew(&l, Millis(2)),
            Err(LeaseError::Revoked { .. })
        ));
        assert!(matches!(
            t.acquire(s, &node("b"), Millis(3)),
            Err(LeaseError::Held { .. })
        ));
        t.release(&l).unwrap();
        assert!(t.acquire(s, &node("b"), Millis(4)).unwrap().epoch > l.epoch);
    }

    #[test]
    fn tagged_epochs_of_two_authorities_never_collide() {
        let a = EpochPolicy::Tagged(1);
        let b = EpochPolicy::Tagged(2);
        assert_ne!(a.next(5 << 16), b.next(5 << 16));
        assert!(a.next(7) > 7);
        assert!(b.next(a.next(7)) > a.next(7));
    }

    #[test]
    fn grace_period_blocks_new_grants_but_not_renewals() {
        let mut t = table();
        let s = ShardId::new(2);
        let l = t.acquire(s, &node("a"), Millis(0)).unwrap();
        t.block_new_grants_until(Millis(5_000));
        assert!(t.renew(&l, Millis(10)).is_ok());
        assert!(matches!(
            t.acquire(ShardId::new(3), &node("a"), Millis(10)),
            Err(LeaseError::Unavailable(_))
        ));
        assert!(
            t.acquire(ShardId::new(3), &node("a"), Millis(5_000))
                .is_ok()
        );
    }
}
