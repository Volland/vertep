//! A replicated mediator deployment: three nodes of one mediator share its mailboxes through M4
//! standby replication. The owner's mailbox journal is a `ReplicatedJournal`; an append (and so
//! the acceptance of a message from the previous hop) completes only after a standby stored it.
//! When the owner is lost, a standby takes the shard and serves every accepted message.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use vertep_cluster::{
    AuthorityConfig, AuthorityMembership, ClusterId, ClusterMeta, DetectorConfig, LeaseConfig,
    LocalStandby, ManualClock, Membership, MemoryAuthority, NodeId, ReplicatedJournal, ShardId,
    ShardJournal, StandbyLink, StandbyReplica,
};
use vertep_federation::live::live_uri;
use vertep_store::{JournalStore, MemoryStore};
use vertep_tsp_mediator::MediatorOptions;

type Node = AuthorityMembership<Arc<MemoryAuthority>>;

fn timing() -> AuthorityConfig {
    AuthorityConfig {
        lease: LeaseConfig {
            duration: Duration::from_secs(10),
            safety_margin: Duration::from_secs(2),
        },
        detector: DetectorConfig {
            suspicion_time: Duration::from_secs(3),
            failure_time: Duration::from_secs(10),
        },
    }
}

fn fenced() -> Arc<MemoryStore> {
    Arc::new(MemoryStore::new().with_fencing(true))
}

// @tg: verifies:: [[openspec:tsp-mediator#Replicated Mediator Deployment#Node lost]] {level: integration}
#[test]
fn a_lost_node_loses_no_accepted_message() {
    // One shard holds every mailbox journal; three nodes, the owner and two standbys.
    let meta = ClusterMeta::new(ClusterId::new("mediators").unwrap(), 1).unwrap();
    let clock = ManualClock::new();
    let authority = Arc::new(MemoryAuthority::new(
        meta.clone(),
        timing(),
        Arc::new(clock.clone()),
    ));
    let nodes: Vec<Arc<Node>> = (0..3)
        .map(|i| {
            let id = format!("m{i}");
            let n = Arc::new(AuthorityMembership::new(
                authority.clone(),
                meta.clone(),
                NodeId::new(&id).unwrap(),
                format!("{id}.local:7400"),
                Arc::new(clock.clone()),
                timing().lease,
            ));
            n.join().unwrap();
            n
        })
        .collect();
    for n in &nodes {
        let _ = n.tick();
    }
    let shard = ShardId::new(0);
    nodes[0].acquire(shard).unwrap();
    let replicas: Vec<Arc<StandbyReplica<Arc<MemoryStore>>>> = (1..3)
        .map(|_| Arc::new(StandbyReplica::new(meta.clone(), shard, fenced())))
        .collect();
    let links: Vec<Arc<dyn StandbyLink>> = replicas
        .iter()
        .enumerate()
        .map(|(i, r)| {
            Arc::new(LocalStandby::new(nodes[i + 1].local().clone(), r.clone()))
                as Arc<dyn StandbyLink>
        })
        .collect();
    let owner_journal: Arc<dyn JournalStore> = Arc::new(ReplicatedJournal::new(
        ShardJournal::new(fenced(), shard, nodes[0].clone()).unwrap(),
        links,
        1,
    ));

    // Node m0 serves the mediator; B registers, A sends five messages through it.
    let w = World::new();
    let m0 = w.mediator_on(M1, owner_journal, MediatorOptions::default());
    let a = w.party("https://a.example/tsp", &[]);
    let b = w.party("https://b.example/tsp", &[m0.did()]);
    let client = w.client(&m0.did());
    client.register(b.session.clone()).unwrap();
    for i in 0..5 {
        w.send_routed(&a, &b, &[m0.did().as_str()], M1, &format!("n-{i}"))
            .expect("accepted after a standby stored it");
    }

    // m0 is lost. m1 takes the shard once m0's lease has run out, from its replica.
    let (identity, km) = (m0.identity.clone(), m0.km.clone());
    drop(m0);
    w.net.remove(M1);
    w.live.remove(&live_uri(M1));
    clock.advance_ms(30_000);
    let _ = nodes[1].tick();
    let _ = nodes[2].tick();
    nodes[1].acquire(shard).expect("m1 takes the shard");
    let standby_store = replicas[0].store().clone();
    let m1_journal: Arc<dyn JournalStore> = Arc::new(ReplicatedJournal::new(
        ShardJournal::new(standby_store, shard, nodes[1].clone()).unwrap(),
        vec![Arc::new(LocalStandby::new(
            nodes[2].local().clone(),
            replicas[1].clone(),
        )) as Arc<dyn StandbyLink>],
        1,
    ));
    let _m1 = w.mediator_with(M1, identity, km, m1_journal, MediatorOptions::default());

    // Every accepted message is still there, in order, and B still has its mailbox.
    let batch = client.deliver(b.did(), 10).expect("m1 serves the mailbox");
    let ids: Vec<String> = batch
        .messages
        .iter()
        .map(|m| b.open(&w, &m.data).1.message_id)
        .collect();
    assert_eq!(ids, vec!["n-0", "n-1", "n-2", "n-3", "n-4"]);
    // And m1 keeps accepting.
    w.send_routed(&a, &b, &[_m1.did().as_str()], M1, "n-5")
        .expect("m1 accepts");
    assert_eq!(client.status(b.did()).unwrap().message_count, 6);
}
