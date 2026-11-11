//! Moving shards: journal shipping with suffix resume and end-version verification, lease
//! transfer over a shared journal store, and the offline doubling of the shard count.

use core::fmt;
use std::collections::BTreeMap;

use vertep_store::oxrdf::NamedNode;
use vertep_store::{AppendError, AppendMeta, JournalStore, Position, StoreError};

use crate::lease::{Lease, LeaseError};
use crate::membership::{Drain, Membership};
use crate::meta::{ClusterMeta, NodeId, ShardCount};
use crate::placement::ShardId;

/// What a shipment copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipReport {
    /// The entries copied by this shipment (0 when the target was already complete).
    pub entries: u64,
    /// Every actor's journal head, equal in source and target after the shipment.
    pub heads: BTreeMap<NamedNode, Position>,
}

/// Why a shipment, a move or a resharding stopped. Nothing already copied is lost: a retry
/// resumes from the target's heads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShipError {
    /// Reading the source or the target failed.
    Store(StoreError),
    /// Appending to the target failed.
    Append {
        /// The actor being copied.
        actor: NamedNode,
        /// The store's error.
        error: AppendError,
    },
    /// After copying, an actor's journal ends at different versions, or the target holds an
    /// actor or entries the source does not.
    Mismatch {
        /// The actor.
        actor: NamedNode,
        /// The source head.
        source: Position,
        /// The target head.
        target: Position,
    },
    /// The shard could not be drained.
    Drain(String),
    /// The lease could not be handed over.
    Lease(LeaseError),
}

impl fmt::Display for ShipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShipError::Store(e) => write!(f, "shipping failed: {e}"),
            ShipError::Append { actor, error } => {
                write!(f, "shipping {actor} failed: {error}")
            }
            ShipError::Mismatch {
                actor,
                source,
                target,
            } => write!(
                f,
                "{actor} ends at {} in the source and {} in the target",
                source.get(),
                target.get()
            ),
            ShipError::Drain(m) => write!(f, "the shard could not be drained: {m}"),
            ShipError::Lease(e) => write!(f, "the lease could not be handed over: {e}"),
        }
    }
}

impl std::error::Error for ShipError {}

impl From<StoreError> for ShipError {
    fn from(e: StoreError) -> Self {
        ShipError::Store(e)
    }
}

/// Copies to `target` every journal entry of `source` that `target` lacks, one entry per
/// append, so an interrupted shipment resumes from the last entry the target stored.
///
/// # Errors
///
/// [`ShipError::Append`] when the target refuses an entry, [`ShipError::Mismatch`] when the
/// target is ahead of the source, [`ShipError::Store`] when a read fails.
// @tg: implements:: [[openspec:shard-placement#Rebalancing By Journal File Shipping]]
pub fn ship_journal(
    source: &dyn JournalStore,
    target: &dyn JournalStore,
) -> Result<ShipReport, ShipError> {
    let mut report = ShipReport::default();
    for actor in source.actors()? {
        let src = source.head(&actor)?;
        let mut dst = target.head(&actor)?;
        if dst > src {
            return Err(ShipError::Mismatch {
                actor,
                source: src,
                target: dst,
            });
        }
        if dst < src {
            for record in source.read(&actor, dst.next())? {
                dst = target
                    .append(&actor, dst, &[record.entry], &AppendMeta::none())
                    .map_err(|error| ShipError::Append {
                        actor: actor.clone(),
                        error,
                    })?;
                report.entries += 1;
            }
        }
    }
    report.heads = verify_same_end(source, target)?;
    Ok(report)
}

/// Checks that `target` holds exactly the actors of `source`, each ending at the same version.
///
/// # Errors
///
/// [`ShipError::Mismatch`] for the first actor that differs.
pub fn verify_same_end(
    source: &dyn JournalStore,
    target: &dyn JournalStore,
) -> Result<BTreeMap<NamedNode, Position>, ShipError> {
    let mut heads = BTreeMap::new();
    for actor in source.actors()? {
        let (s, t) = (source.head(&actor)?, target.head(&actor)?);
        if s != t {
            return Err(ShipError::Mismatch {
                actor,
                source: s,
                target: t,
            });
        }
        heads.insert(actor, s);
    }
    for actor in target.actors()? {
        if !heads.contains_key(&actor) {
            let t = target.head(&actor)?;
            return Err(ShipError::Mismatch {
                actor,
                source: Position::ZERO,
                target: t,
            });
        }
    }
    Ok(heads)
}

/// Moves a shard whose journal is a local file: the old owner drains it, ships the journal
/// suffix the new owner lacks, verifies both end at the same versions, and only then hands the
/// lease to `to` at a higher epoch. On any failure before the handover the old owner keeps the
/// lease and can resume; a retry ships only what is still missing.
///
/// # Errors
///
/// [`ShipError`] for the step that failed.
// @tg: implements:: [[openspec:shard-placement#Rebalancing By Journal File Shipping]]
pub fn move_by_shipping<M: Membership + ?Sized>(
    old: &M,
    lease: &Lease,
    to: &NodeId,
    drain: &mut dyn Drain,
    source: &dyn JournalStore,
    target: &dyn JournalStore,
) -> Result<(Lease, ShipReport), ShipError> {
    drain.drain_shard(lease.shard).map_err(ShipError::Drain)?;
    let report = ship_journal(source, target)?;
    let next = old.handover(lease, to).map_err(ShipError::Lease)?;
    Ok((next, report))
}

/// Moves a shard whose journal is a store shared by every node: drain, then hand the lease to
/// `to` at a higher epoch. No journal data is copied; the new owner continues in the shared
/// store under its epoch, which fences the old owner.
///
/// # Errors
///
/// [`ShipError::Drain`] or [`ShipError::Lease`].
// @tg: implements:: [[openspec:shard-placement#Rebalancing By Lease Transfer]]
pub fn move_by_lease_transfer<M: Membership + ?Sized>(
    old: &M,
    lease: &Lease,
    to: &NodeId,
    drain: &mut dyn Drain,
) -> Result<Lease, ShipError> {
    drain.drain_shard(lease.shard).map_err(ShipError::Drain)?;
    old.handover(lease, to).map_err(ShipError::Lease)
}

/// One shard move the rebalancer plans: `shard` goes from `from` (none when its owner is
/// gone) to `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    /// The shard.
    pub shard: ShardId,
    /// The current owner, if it is still live.
    pub from: Option<NodeId>,
    /// The member taking it.
    pub to: NodeId,
}

/// The rebalancer's plan for `view`: the fewest shard moves that leave every active member
/// with `count / members` shards (the first `count mod members` members by identifier one
/// more). A shard stays with its current owner while that owner is live and under its share;
/// shards of members that are gone or over their share move to the members under theirs.
/// Each move then runs [`move_by_shipping`] or [`move_by_lease_transfer`].
// @tg: implements:: [[openspec:shard-placement#Rebalancing By Journal File Shipping]]
pub fn plan_moves(view: &crate::view::View, count: ShardCount) -> Vec<Move> {
    let members: Vec<NodeId> = view.active().map(|m| m.id.clone()).collect();
    if members.is_empty() {
        return Vec::new();
    }
    let n = count.get() as usize;
    let k = members.len();
    let share = |i: usize| n / k + usize::from(i < n % k);
    let mut kept: BTreeMap<NodeId, Vec<ShardId>> =
        members.iter().map(|m| (m.clone(), Vec::new())).collect();
    let mut loose: Vec<(ShardId, Option<NodeId>)> = Vec::new();
    for s in count.shards() {
        match view.owner(s) {
            Some(o) if kept.contains_key(&o.node) => {
                if let Some(v) = kept.get_mut(&o.node) {
                    v.push(s);
                }
            }
            Some(o) if view.is_live(&o.node) => loose.push((s, Some(o.node.clone()))),
            _ => loose.push((s, None)),
        }
    }
    for (i, m) in members.iter().enumerate() {
        let held = kept.get_mut(m).map(std::mem::take).unwrap_or_default();
        let (stay, over) = held.split_at(held.len().min(share(i)));
        kept.insert(m.clone(), stay.to_vec());
        loose.extend(over.iter().map(|s| (*s, Some(m.clone()))));
    }
    loose.sort_by_key(|(s, _)| *s);
    let mut moves = Vec::new();
    let mut loose = loose.into_iter();
    for (i, m) in members.iter().enumerate() {
        let have = kept.get(m).map_or(0, Vec::len);
        for _ in have..share(i) {
            let Some((shard, from)) = loose.next() else {
                break;
            };
            moves.push(Move {
                shard,
                from,
                to: m.clone(),
            });
        }
    }
    moves.sort_by_key(|m| m.shard);
    moves
}

/// What an offline resharding did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReshardReport {
    /// The shard count before.
    pub old: ShardCount,
    /// The shard count after.
    pub new: ShardCount,
    /// The actors moved, by the shard they ended up in.
    pub actors: BTreeMap<NamedNode, ShardId>,
    /// The entries copied.
    pub entries: u64,
}

/// Why resharding refused to run or stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReshardError {
    /// Nodes still hold shard leases: the cluster is running.
    LeasesHeld(Vec<NodeId>),
    /// The shard count cannot be doubled.
    TooManyShards(ShardCount),
    /// The number of stores given does not match the old or new shard count.
    StoreCount {
        /// The number expected.
        expected: u32,
        /// The number given.
        got: usize,
    },
    /// Copying or verifying a journal failed.
    Ship(ShipError),
    /// An actor's journal read back from its new shard differs from the old one.
    StateChanged(NamedNode),
}

impl fmt::Display for ReshardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReshardError::LeasesHeld(nodes) => {
                let names: Vec<&str> = nodes.iter().map(NodeId::as_str).collect();
                write!(
                    f,
                    "resharding refused: the cluster is running, nodes {} still hold shard leases",
                    names.join(", ")
                )
            }
            ReshardError::TooManyShards(n) => write!(f, "{n} shards cannot be doubled"),
            ReshardError::StoreCount { expected, got } => {
                write!(f, "expected {expected} shard stores, got {got}")
            }
            ReshardError::Ship(e) => e.fmt(f),
            ReshardError::StateChanged(a) => {
                write!(f, "the journal of {a} differs after resharding")
            }
        }
    }
}

impl std::error::Error for ReshardError {}

impl From<ShipError> for ReshardError {
    fn from(e: ShipError) -> Self {
        ReshardError::Ship(e)
    }
}

impl From<StoreError> for ReshardError {
    fn from(e: StoreError) -> Self {
        ReshardError::Ship(ShipError::Store(e))
    }
}

/// The offline resharding procedure: doubles the shard count of a stopped cluster.
///
/// 1. Refuse if any node holds a shard lease (`live_leases` is what the authority reports).
/// 2. For each old shard `s`, copy each actor's journal into the store of the shard its IRI
///    hashes to under `2n`, which is `s` or `s + n`.
/// 3. Verify that every actor's journal reads back entry for entry from its new shard.
/// 4. Return the metadata with the doubled count, for the caller to record before restart.
///
/// # Errors
///
/// [`ReshardError::LeasesHeld`] naming the nodes still holding leases, and the copy errors.
// @tg: implements:: [[openspec:shard-placement#Resharding Procedure]]
pub fn reshard_offline(
    meta: &ClusterMeta,
    live_leases: &[Lease],
    old: &[&dyn JournalStore],
    new: &[&dyn JournalStore],
) -> Result<(ClusterMeta, ReshardReport), ReshardError> {
    if !live_leases.is_empty() {
        let mut nodes: Vec<NodeId> = live_leases.iter().map(|l| l.owner.clone()).collect();
        nodes.sort();
        nodes.dedup();
        return Err(ReshardError::LeasesHeld(nodes));
    }
    let n = meta.shard_count;
    let n2 = n
        .doubled()
        .ok_or(ReshardError::TooManyShards(meta.shard_count))?;
    if old.len() != n.get() as usize {
        return Err(ReshardError::StoreCount {
            expected: n.get(),
            got: old.len(),
        });
    }
    if new.len() != n2.get() as usize {
        return Err(ReshardError::StoreCount {
            expected: n2.get(),
            got: new.len(),
        });
    }
    let mut next = meta.clone();
    next.shard_count = n2;
    let mut report = ReshardReport {
        old: n,
        new: n2,
        actors: BTreeMap::new(),
        entries: 0,
    };
    for (s, source) in old.iter().enumerate() {
        let (low, high) = ShardId::new(s as u32).split(n);
        for actor in source.actors()? {
            let to = next.shard_of(actor.as_str());
            debug_assert!(to == low || to == high);
            let target = new[to.index()];
            let mut head = target.head(&actor)?;
            let records = source.read(&actor, head.next())?;
            for r in records {
                head = target
                    .append(&actor, head, &[r.entry], &AppendMeta::none())
                    .map_err(|error| ShipError::Append {
                        actor: actor.clone(),
                        error,
                    })?;
                report.entries += 1;
            }
            let before = source.read(&actor, Position::ZERO)?;
            let after = target.read(&actor, Position::ZERO)?;
            let same = before.len() == after.len()
                && before
                    .iter()
                    .zip(&after)
                    .all(|(a, b)| a.position == b.position && a.entry.is_isomorphic(&b.entry));
            if !same {
                return Err(ReshardError::StateChanged(actor));
            }
            report.actors.insert(actor, to);
        }
    }
    Ok((next, report))
}

/// The file in a cluster data directory listing the shard leases still live, one
/// `<node> <shard> <epoch>` line each, dumped from the lease authority before an offline
/// resharding. An absent or empty file means no node holds a lease.
pub const LEASES_FILE: &str = "leases";

/// Reads the [`LEASES_FILE`] of `dir`: the holders and shards of the live leases.
///
/// # Errors
///
/// [`ReshardError::Ship`] with a store error when a line is malformed.
pub fn read_live_leases(
    dir: &std::path::Path,
) -> Result<Vec<(NodeId, ShardId, u64)>, ReshardError> {
    let path = dir.join(LEASES_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let bad = |l: &str| {
        ReshardError::Ship(ShipError::Store(StoreError::invalid(format!(
            "{}: not `<node> <shard> <epoch>`: {l}",
            path.display()
        ))))
    };
    let mut out = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [n, s, e] = parts.as_slice() else {
            return Err(bad(line));
        };
        let node = NodeId::new(*n).map_err(|_| bad(line))?;
        let shard = s.parse::<u32>().map_err(|_| bad(line))?;
        let epoch = e.parse::<u64>().map_err(|_| bad(line))?;
        out.push((node, ShardId::new(shard), epoch));
    }
    Ok(out)
}

/// The offline resharding of a cluster data directory of tiramemsu shard files (the tool
/// behind `vt cluster reshard`): reads the metadata of `dir`, refuses while
/// [`LEASES_FILE`] lists a live lease, writes `2n` shard files and the doubled metadata to
/// `out`, and verifies every actor's journal there. `dir` is left untouched, so rolling back
/// is keeping it.
///
/// # Errors
///
/// [`ReshardError::LeasesHeld`] naming the nodes still holding leases, metadata and store
/// errors, and the errors of [`reshard_offline`].
// @tg: implements:: [[openspec:shard-placement#Resharding Procedure]]
#[cfg(feature = "tiramemsu")]
pub fn reshard_directory(
    dir: &std::path::Path,
    out: &std::path::Path,
) -> Result<(ClusterMeta, ReshardReport), ReshardError> {
    use vertep_tiramemsu::{TiramemsuShard, shard_file_name};
    let meta_err = |e: crate::meta::MetaError| {
        ReshardError::Ship(ShipError::Store(StoreError::invalid(e.to_string())))
    };
    let meta = ClusterMeta::load(dir).map_err(meta_err)?;
    let mut holders: Vec<NodeId> = read_live_leases(dir)?
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    if !holders.is_empty() {
        holders.sort();
        holders.dedup();
        return Err(ReshardError::LeasesHeld(holders));
    }
    let n = meta.shard_count.get() as usize;
    let n2 = meta
        .shard_count
        .doubled()
        .ok_or(ReshardError::TooManyShards(meta.shard_count))?
        .get() as usize;
    std::fs::create_dir_all(out)
        .map_err(|e| ReshardError::Ship(ShipError::Store(StoreError::backend(e))))?;
    let old: Vec<TiramemsuShard> = (0..n)
        .map(|i| TiramemsuShard::open(dir.join(shard_file_name(i, n))))
        .collect::<Result<_, _>>()?;
    let new: Vec<TiramemsuShard> = (0..n2)
        .map(|i| TiramemsuShard::open(out.join(shard_file_name(i, n2))))
        .collect::<Result<_, _>>()?;
    let old_refs: Vec<&dyn JournalStore> = old.iter().map(|s| s as &dyn JournalStore).collect();
    let new_refs: Vec<&dyn JournalStore> = new.iter().map(|s| s as &dyn JournalStore).collect();
    let (next, report) = reshard_offline(&meta, &[], &old_refs, &new_refs)?;
    next.store(out).map_err(meta_err)?;
    Ok((next, report))
}
