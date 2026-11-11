//! `vt cluster reshard`: the offline doubling of a cluster's shard count from the command line.

use std::path::Path;
use std::process::Command;

use vertep_cluster::{ClusterId, ClusterMeta};
use vertep_store::oxrdf::NamedNode;
use vertep_store::testkit::sample_entry;
use vertep_store::{AppendMeta, JournalStore, Position};
use vertep_tiramemsu::{TiramemsuShard, shard_file_name};

const VT: &str = env!("CARGO_BIN_EXE_vt");

/// A stopped 4-shard cluster directory with `n` actors of two entries each.
fn cluster_dir(dir: &Path, n: usize) -> (ClusterMeta, Vec<NamedNode>) {
    let meta = ClusterMeta::new(ClusterId::new("prod").unwrap(), 4).unwrap();
    meta.create(dir).unwrap();
    let shards: Vec<TiramemsuShard> = (0..4)
        .map(|i| TiramemsuShard::open(dir.join(shard_file_name(i, 4))).unwrap())
        .collect();
    let mut actors = Vec::new();
    for i in 0..n {
        let a = NamedNode::new(format!("urn:vt:actor:r{i}")).unwrap();
        let s = &shards[meta.shard_of(a.as_str()).index()];
        s.append(
            &a,
            Position::ZERO,
            &[sample_entry(&a, "one")],
            &AppendMeta::none(),
        )
        .unwrap();
        s.append(
            &a,
            Position::new(1),
            &[sample_entry(&a, "two")],
            &AppendMeta::none(),
        )
        .unwrap();
        actors.push(a);
    }
    (meta, actors)
}

// @tg: verifies:: [[openspec:shard-placement#Resharding Procedure#Doubling the shard count]] {level: e2e}
#[test]
fn vt_cluster_reshard_doubles_the_shard_count() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (_, actors) = cluster_dir(dir.path(), 40);
    let o = Command::new(VT)
        .args(["cluster", "reshard"])
        .arg(dir.path())
        .arg(out.path())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("to 8 shards"));
    let meta = ClusterMeta::load(out.path()).unwrap();
    assert_eq!(
        meta.shard_count.get(),
        8,
        "the metadata records the new count"
    );
    for a in &actors {
        let s = meta.shard_of(a.as_str()).index();
        let shard = TiramemsuShard::open(out.path().join(shard_file_name(s, 8))).unwrap();
        assert_eq!(
            shard.head(a).unwrap().get(),
            2,
            "{a} in the shard it hashes to"
        );
    }
}

// @tg: verifies:: [[openspec:shard-placement#Resharding Procedure#Resharding refuses a running cluster]] {level: e2e}
#[test]
fn vt_cluster_reshard_refuses_a_running_cluster() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    cluster_dir(dir.path(), 4);
    std::fs::write(dir.path().join("leases"), "node-b 1 7\nnode-a 0 3\n").unwrap();
    let o = Command::new(VT)
        .args(["cluster", "reshard"])
        .arg(dir.path())
        .arg(out.path())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("node-a, node-b"), "names the nodes: {err}");
    assert!(!out.path().join("cluster.meta").exists(), "nothing written");
}
