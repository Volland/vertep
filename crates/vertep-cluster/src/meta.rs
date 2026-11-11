//! Cluster metadata: the identifiers, the shard count and the hash version fixed at creation.

use core::fmt;
use std::path::{Path, PathBuf};

use crate::placement::ShardId;
use vertep_store::oxrdf::NamedNode;

/// The file name of the cluster metadata inside a cluster directory.
pub const META_FILE: &str = "cluster.meta";

/// Why cluster metadata, an identifier or a shard count is not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaError {
    /// A cluster or node identifier is not a DNS label (`[a-z0-9-]`, 1 to 63 characters, no
    /// hyphen at either end). Identifiers become certificate names, so they follow DNS rules.
    InvalidId {
        /// `"cluster"` or `"node"`.
        kind: &'static str,
        /// The rejected value.
        value: String,
    },
    /// The shard count is zero or not a power of two.
    NotPowerOfTwo(u64),
    /// The hash function version is not one this build knows.
    UnknownHashVersion(u32),
    /// The replication factor is zero.
    InvalidReplicationFactor(u32),
    /// Cluster metadata already exists with other values.
    AlreadyExists {
        /// The metadata file.
        path: PathBuf,
    },
    /// The metadata file cannot be read or written.
    Io(String),
    /// The metadata text is malformed.
    Parse(String),
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetaError::InvalidId { kind, value } => write!(
                f,
                "invalid {kind} identifier {value:?}: use 1 to 63 characters of a-z, 0-9 and '-', \
                 not starting or ending with '-'"
            ),
            MetaError::NotPowerOfTwo(n) => {
                write!(f, "the shard count must be a power of two, got {n}")
            }
            MetaError::UnknownHashVersion(v) => write!(f, "unknown shard hash version {v}"),
            MetaError::InvalidReplicationFactor(r) => {
                write!(f, "the replication factor must be at least 1, got {r}")
            }
            MetaError::AlreadyExists { path } => write!(
                f,
                "{} already records a cluster with other metadata",
                path.display()
            ),
            MetaError::Io(m) => write!(f, "cluster metadata: {m}"),
            MetaError::Parse(m) => write!(f, "malformed cluster metadata: {m}"),
        }
    }
}

impl std::error::Error for MetaError {}

fn check_label(kind: &'static str, value: &str) -> Result<(), MetaError> {
    let ok = !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(MetaError::InvalidId {
            kind,
            value: value.to_owned(),
        })
    }
}

macro_rules! label_id {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The identifier `value`, which must be a DNS label.
            ///
            /// # Errors
            ///
            /// [`MetaError::InvalidId`] for anything else.
            pub fn new(value: impl Into<String>) -> Result<Self, MetaError> {
                let value = value.into();
                check_label($kind, &value)?;
                Ok($name(value))
            }

            /// The identifier text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl core::str::FromStr for $name {
            type Err = MetaError;
            fn from_str(s: &str) -> Result<Self, MetaError> {
                $name::new(s)
            }
        }
    };
}

label_id!(
    /// The identifier of a cluster, a DNS label recorded in the metadata at creation.
    ClusterId,
    "cluster"
);
label_id!(
    /// The identifier of a node, a DNS label unique among the cluster's active members.
    NodeId,
    "node"
);

/// A shard count: a power of two, fixed when the cluster is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShardCount(u32);

impl ShardCount {
    /// The count `n`.
    ///
    /// # Errors
    ///
    /// [`MetaError::NotPowerOfTwo`] when `n` is zero or not a power of two.
    // @tg: implements:: [[openspec:shard-placement#Fixed Shard Count]]
    pub fn new(n: u64) -> Result<Self, MetaError> {
        match u32::try_from(n) {
            Ok(v) if v.is_power_of_two() => Ok(ShardCount(v)),
            _ => Err(MetaError::NotPowerOfTwo(n)),
        }
    }

    /// The number of shards.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The count after one resharding split, `2n`, if it still fits.
    pub fn doubled(self) -> Option<ShardCount> {
        self.0.checked_mul(2).map(ShardCount)
    }

    /// Every shard of this count, in order.
    pub fn shards(self) -> impl Iterator<Item = ShardId> {
        (0..self.0).map(ShardId::new)
    }

    /// True when `shard` is below the count.
    pub fn contains(self, shard: ShardId) -> bool {
        shard.get() < self.0
    }
}

impl fmt::Display for ShardCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The version of the function that hashes actor IRIs to shards.
///
/// The hash is part of the cluster's identity: a node built with another version would place
/// actors in other shards, so it is refused at join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum HashVersion {
    /// Version 1: FNV-1a 64-bit over the UTF-8 bytes of the IRI, the hash `vertep-tiramemsu`
    /// uses for single-node shards, so a single-node shard directory and a cluster agree.
    Fnv1a64,
}

impl HashVersion {
    /// The version this build creates clusters with.
    pub const CURRENT: HashVersion = HashVersion::Fnv1a64;

    /// The version number recorded in metadata.
    pub const fn number(self) -> u32 {
        match self {
            HashVersion::Fnv1a64 => 1,
        }
    }

    /// The version with number `n`.
    ///
    /// # Errors
    ///
    /// [`MetaError::UnknownHashVersion`] for a number this build does not know.
    pub fn from_number(n: u32) -> Result<Self, MetaError> {
        match n {
            1 => Ok(HashVersion::Fnv1a64),
            other => Err(MetaError::UnknownHashVersion(other)),
        }
    }

    /// The 64-bit hash of `bytes`. It depends only on the bytes, never on the platform, the
    /// pointer width or the endianness.
    pub fn hash64(self, bytes: &[u8]) -> u64 {
        match self {
            HashVersion::Fnv1a64 => {
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for b in bytes {
                    h ^= u64::from(*b);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                h
            }
        }
    }
}

/// The metadata of a cluster, fixed at creation: its identifier, shard count, hash version and
/// replication factor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:shard-placement#Fixed Shard Count]]
// @tg: implements:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash]]
pub struct ClusterMeta {
    /// The cluster identifier.
    pub cluster: ClusterId,
    /// The shard count.
    pub shard_count: ShardCount,
    /// The version of the IRI hash.
    pub hash_version: HashVersion,
    /// How many nodes store each shard's journal (1 = no standby).
    pub replication_factor: u32,
}

impl ClusterMeta {
    /// The metadata of a new cluster with `shard_count` shards, the current hash version and
    /// no replication.
    ///
    /// # Errors
    ///
    /// [`MetaError::NotPowerOfTwo`] when the count is not a power of two.
    pub fn new(cluster: ClusterId, shard_count: u64) -> Result<Self, MetaError> {
        Ok(ClusterMeta {
            cluster,
            shard_count: ShardCount::new(shard_count)?,
            hash_version: HashVersion::CURRENT,
            replication_factor: 1,
        })
    }

    /// This metadata with replication factor `rf`.
    ///
    /// # Errors
    ///
    /// [`MetaError::InvalidReplicationFactor`] for zero.
    pub fn with_replication_factor(mut self, rf: u32) -> Result<Self, MetaError> {
        if rf == 0 {
            return Err(MetaError::InvalidReplicationFactor(rf));
        }
        self.replication_factor = rf;
        Ok(self)
    }

    /// The shard of the actor with address `iri`: the hash of its UTF-8 bytes modulo the shard
    /// count (a mask, since the count is a power of two).
    pub fn shard_of(&self, iri: &str) -> ShardId {
        crate::placement::shard_of(iri, self.shard_count, self.hash_version)
    }

    /// The fencing scope of `shard`: the IRI that journal stores count epochs in,
    /// `urn:vertep:cluster:<cluster>:shard:<s>-of-<n>`.
    pub fn shard_scope(&self, shard: ShardId) -> NamedNode {
        NamedNode::new_unchecked(format!(
            "urn:vertep:cluster:{}:shard:{}-of-{}",
            self.cluster,
            shard.get(),
            self.shard_count
        ))
    }

    /// The metadata as text, one `key = value` line each.
    pub fn to_text(&self) -> String {
        format!(
            "cluster = {}\nshard_count = {}\nhash_version = {}\nreplication_factor = {}\n",
            self.cluster,
            self.shard_count,
            self.hash_version.number(),
            self.replication_factor
        )
    }

    /// The metadata in `text`, as written by [`ClusterMeta::to_text`].
    ///
    /// # Errors
    ///
    /// [`MetaError::Parse`] for a missing or unknown key, and the validation errors of the
    /// values.
    pub fn from_text(text: &str) -> Result<Self, MetaError> {
        let mut cluster = None;
        let mut count = None;
        let mut hash = None;
        let mut rf = None;
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| MetaError::Parse(format!("line {}: expected key = value", i + 1)))?;
            let (k, v) = (k.trim(), v.trim());
            let num = |v: &str| {
                v.parse::<u64>()
                    .map_err(|_| MetaError::Parse(format!("line {}: {k} is not a number", i + 1)))
            };
            match k {
                "cluster" => cluster = Some(ClusterId::new(v)?),
                "shard_count" => count = Some(ShardCount::new(num(v)?)?),
                "hash_version" => {
                    let n = u32::try_from(num(v)?)
                        .map_err(|_| MetaError::Parse("hash_version is too large".into()))?;
                    hash = Some(HashVersion::from_number(n)?);
                }
                "replication_factor" => {
                    let n = u32::try_from(num(v)?)
                        .map_err(|_| MetaError::Parse("replication_factor is too large".into()))?;
                    rf = Some(n);
                }
                other => return Err(MetaError::Parse(format!("unknown key {other}"))),
            }
        }
        let missing = |k: &str| MetaError::Parse(format!("missing {k}"));
        ClusterMeta {
            cluster: cluster.ok_or_else(|| missing("cluster"))?,
            shard_count: count.ok_or_else(|| missing("shard_count"))?,
            hash_version: hash.ok_or_else(|| missing("hash_version"))?,
            replication_factor: 1,
        }
        .with_replication_factor(rf.ok_or_else(|| missing("replication_factor"))?)
    }

    /// Records this metadata in `dir` at cluster creation. Creating again with the same
    /// metadata is a no-op; creating with other metadata is refused.
    ///
    /// # Errors
    ///
    /// [`MetaError::AlreadyExists`] when `dir` records other metadata, [`MetaError::Io`] when
    /// the file cannot be written.
    pub fn create(&self, dir: impl AsRef<Path>) -> Result<(), MetaError> {
        let dir = dir.as_ref();
        let path = dir.join(META_FILE);
        if path.exists() {
            return if ClusterMeta::load(dir)? == *self {
                Ok(())
            } else {
                Err(MetaError::AlreadyExists { path })
            };
        }
        std::fs::create_dir_all(dir)
            .map_err(|e| MetaError::Io(format!("{}: {e}", dir.display())))?;
        std::fs::write(&path, self.to_text())
            .map_err(|e| MetaError::Io(format!("{}: {e}", path.display())))
    }

    /// The metadata recorded in `dir`.
    ///
    /// # Errors
    ///
    /// [`MetaError::Io`] when the file cannot be read, and the errors of
    /// [`ClusterMeta::from_text`].
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, MetaError> {
        let path = dir.as_ref().join(META_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| MetaError::Io(format!("{}: {e}", path.display())))?;
        ClusterMeta::from_text(&text)
    }

    /// Overwrites the metadata in `dir`, used only by the offline resharding procedure.
    ///
    /// # Errors
    ///
    /// [`MetaError::Io`] when the file cannot be written.
    pub fn store(&self, dir: impl AsRef<Path>) -> Result<(), MetaError> {
        let path = dir.as_ref().join(META_FILE);
        std::fs::write(&path, self.to_text())
            .map_err(|e| MetaError::Io(format!("{}: {e}", path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_dns_labels() {
        assert!(NodeId::new("node-1").is_ok());
        assert!(NodeId::new("Node").is_err());
        assert!(NodeId::new("-a").is_err());
        assert!(NodeId::new("").is_err());
        assert!(ClusterId::new("a".repeat(64)).is_err());
    }

    #[test]
    fn meta_text_round_trip() {
        let m = ClusterMeta::new(ClusterId::new("prod").unwrap(), 256)
            .unwrap()
            .with_replication_factor(3)
            .unwrap();
        assert_eq!(ClusterMeta::from_text(&m.to_text()).unwrap(), m);
        assert!(ClusterMeta::from_text("cluster = prod").is_err());
    }
}
