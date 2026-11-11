//! Shards: the fixed mapping from actor IRIs to shards, the split of a shard on resharding,
//! the deterministic assignment of shards to members, and one journal store per shard.

use core::fmt;
use std::collections::BTreeMap;

use crate::meta::{HashVersion, NodeId, ShardCount};
use crate::view::View;

/// A shard number, below the cluster's shard count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShardId(u32);

impl ShardId {
    /// Shard `n`.
    pub const fn new(n: u32) -> Self {
        ShardId(n)
    }

    /// The number.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The number as an index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The two shards this shard splits into when `old` doubles: `s` and `s + old`. Under
    /// `hash mod 2n`, an actor of shard `s` lands in exactly one of them.
    pub fn split(self, old: ShardCount) -> (ShardId, ShardId) {
        (self, ShardId(self.0 + old.get()))
    }

    /// The shard of the halved count this shard was split from: `s mod (n / 2)`.
    pub fn parent(self, new: ShardCount) -> ShardId {
        ShardId(self.0 % (new.get() / 2).max(1))
    }
}

impl fmt::Display for ShardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "shard {}", self.0)
    }
}

/// The shard of the actor with address `iri`: `hash(iri) mod count`, computed as a mask since
/// the count is a power of two. For [`HashVersion::Fnv1a64`] this is exactly
/// `vertep_tiramemsu::shard_of(iri, count)`.
// @tg: implements:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash]]
pub fn shard_of(iri: &str, count: ShardCount, hash: HashVersion) -> ShardId {
    let h = hash.hash64(iri.as_bytes());
    ShardId((h & (u64::from(count.get()) - 1)) as u32)
}

/// The deterministic assignment of every shard to a member: the members of `view` that may
/// hold leases (active ones), sorted by identifier, take shards round robin. Every node that
/// holds the same view computes the same table.
pub fn assignment(view: &View, count: ShardCount) -> BTreeMap<ShardId, NodeId> {
    let members: Vec<&NodeId> = view.active().map(|m| &m.id).collect();
    let mut out = BTreeMap::new();
    if members.is_empty() {
        return out;
    }
    for s in count.shards() {
        out.insert(s, members[s.index() % members.len()].clone());
    }
    out
}

/// One journal store instance per shard, for the shards this node opened.
///
/// A shard is one journal with one writer, so two shards never share an instance; an actor's
/// journal is always in the instance of the shard its IRI hashes to.
// @tg: implements:: [[openspec:shard-placement#One Journal File Per Shard]]
#[derive(Debug)]
pub struct ShardStores<J> {
    count: ShardCount,
    hash: HashVersion,
    stores: Vec<Option<J>>,
}

impl<J> ShardStores<J> {
    /// No shard opened yet.
    pub fn new(count: ShardCount, hash: HashVersion) -> Self {
        ShardStores {
            count,
            hash,
            stores: (0..count.get()).map(|_| None).collect(),
        }
    }

    /// Installs `store` as the journal of `shard`, returning the instance it replaces.
    ///
    /// # Panics
    ///
    /// When `shard` is not below the shard count.
    pub fn insert(&mut self, shard: ShardId, store: J) -> Option<J> {
        assert!(self.count.contains(shard), "{shard} out of range");
        self.stores[shard.index()].replace(store)
    }

    /// Closes the journal of `shard`, returning it.
    pub fn remove(&mut self, shard: ShardId) -> Option<J> {
        self.stores.get_mut(shard.index()).and_then(Option::take)
    }

    /// The journal of `shard`, if opened.
    pub fn get(&self, shard: ShardId) -> Option<&J> {
        self.stores.get(shard.index()).and_then(Option::as_ref)
    }

    /// The shard of `actor`.
    pub fn shard_of(&self, actor: &str) -> ShardId {
        shard_of(actor, self.count, self.hash)
    }

    /// The journal holding `actor`'s entries, if its shard is opened here.
    pub fn for_actor(&self, actor: &str) -> Option<(ShardId, &J)> {
        let s = self.shard_of(actor);
        self.get(s).map(|j| (s, j))
    }

    /// The opened shards, in order.
    pub fn opened(&self) -> impl Iterator<Item = (ShardId, &J)> {
        self.stores
            .iter()
            .enumerate()
            .filter_map(|(i, j)| j.as_ref().map(|j| (ShardId(i as u32), j)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_parent_are_inverse() {
        let n = ShardCount::new(64).unwrap();
        let n2 = n.doubled().unwrap();
        for s in n.shards() {
            let (a, b) = s.split(n);
            assert_eq!(a.parent(n2), s);
            assert_eq!(b.parent(n2), s);
        }
    }

    #[test]
    fn an_actor_lands_in_one_of_the_two_split_shards() {
        let n = ShardCount::new(64).unwrap();
        let n2 = n.doubled().unwrap();
        for i in 0..2000 {
            let iri = format!("urn:vt:actor:{i}");
            let old = shard_of(&iri, n, HashVersion::CURRENT);
            let new = shard_of(&iri, n2, HashVersion::CURRENT);
            let (a, b) = old.split(n);
            assert!(new == a || new == b);
        }
    }
}
