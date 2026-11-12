//! The owner's replicated journal: what happens before it writes its own copy.

mod common;

use std::sync::Arc;

use common::{AuthorityHarness, actor_in};
use vertep_cluster::{
    Fence, LocalStandby, Membership, ReplicatedJournal, ReplicationBatch, ShardId, ShardJournal,
    StandbyLink, StandbyReplica,
};
use vertep_store::testkit::sample_entry;
use vertep_store::{AppendError, AppendMeta, JournalStore, MemoryStore, Position};

type Replica = StandbyReplica<Arc<MemoryStore>>;
type Link = LocalStandby<Arc<MemoryStore>>;

/// Node 0 owns `shard` with a replicated journal over one standby on node 1.
fn owner_and_standby(
    shard: ShardId,
) -> (AuthorityHarness, Arc<MemoryStore>, Arc<Replica>, Arc<Link>) {
    let h = AuthorityHarness::external_cluster(2, 8);
    h.nodes[0].acquire(shard).unwrap();
    let local = Arc::new(MemoryStore::new());
    let replica = Arc::new(StandbyReplica::new(
        h.authority.meta().clone(),
        shard,
        Arc::new(MemoryStore::new()),
    ));
    let link = Arc::new(LocalStandby::new(
        h.nodes[1].local().clone(),
        replica.clone(),
    ));
    (h, local, replica, link)
}

#[test]
fn a_standby_that_saw_a_newer_epoch_fences_the_owner_before_it_writes() {
    let shard = ShardId::new(3);
    let (h, local, replica, link) = owner_and_standby(shard);
    let meta = h.authority.meta().clone();
    let actor = actor_in(&meta, shard, "rep");
    let epoch = h.nodes[0].held(shard).unwrap().epoch;
    // the standby already stored entries from a newer owner of the shard
    replica
        .receive(&ReplicationBatch {
            fence: Fence {
                shard,
                epoch: epoch + 5,
            },
            actor: actor.clone(),
            after: Position::ZERO,
            entries: vec![sample_entry(&actor, "newer")],
        })
        .unwrap();
    let journal = ReplicatedJournal::new(
        ShardJournal::new(local.clone(), shard, h.nodes[0].clone()).unwrap(),
        vec![link as Arc<dyn StandbyLink>],
        1,
    );
    let r = journal.append(
        &actor,
        Position::ZERO,
        &[sample_entry(&actor, "m1")],
        &AppendMeta::none(),
    );
    assert!(
        matches!(r, Err(AppendError::Fenced { epoch: e, .. }) if e == epoch),
        "the stale owner is fenced: {r:?}"
    );
    assert_eq!(
        local.head(&actor).unwrap(),
        Position::ZERO,
        "and its own journal is untouched"
    );
}

#[test]
fn a_batch_that_stores_nothing_does_not_raise_the_fence() {
    let shard = ShardId::new(2);
    let (h, _, replica, _) = owner_and_standby(shard);
    let actor = actor_in(h.authority.meta(), shard, "rep");
    let batch = |epoch, entries| ReplicationBatch {
        fence: Fence { shard, epoch },
        actor: actor.clone(),
        after: Position::ZERO,
        entries,
    };
    // a newer owner's pre-flight check, with nothing to store
    replica.receive(&batch(9, Vec::new())).unwrap();
    assert_eq!(replica.highest_epoch(), 0);
    // so the current owner's entries are still taken, and then fence the old epochs
    replica
        .receive(&batch(3, vec![sample_entry(&actor, "m1")]))
        .unwrap();
    assert_eq!(replica.highest_epoch(), 3);
    assert!(matches!(
        replica.receive(&batch(2, Vec::new())),
        Err(vertep_cluster::ReplicaError::Fenced {
            epoch: 2,
            highest: 3
        })
    ));
}

#[test]
fn an_unreachable_quorum_leaves_the_owners_journal_untouched() {
    let shard = ShardId::new(5);
    let (h, local, replica, link) = owner_and_standby(shard);
    let actor = actor_in(h.authority.meta(), shard, "rep");
    let journal = ReplicatedJournal::new(
        ShardJournal::new(local.clone(), shard, h.nodes[0].clone()).unwrap(),
        vec![link.clone() as Arc<dyn StandbyLink>],
        1,
    );
    let entry = |m: &str| [sample_entry(&actor, m)];
    journal
        .append(&actor, Position::ZERO, &entry("m1"), &AppendMeta::none())
        .unwrap();
    link.set_up(false);
    let r = journal.append(&actor, Position::new(1), &entry("m2"), &AppendMeta::none());
    assert!(matches!(r, Err(AppendError::Storage(_))), "{r:?}");
    assert_eq!(
        local.head(&actor).unwrap(),
        Position::new(1),
        "nothing written without the quorum"
    );
    // once the standby is back the append goes through, and both copies agree
    link.set_up(true);
    journal
        .append(&actor, Position::new(1), &entry("m2"), &AppendMeta::none())
        .unwrap();
    assert_eq!(local.head(&actor).unwrap(), Position::new(2));
    assert_eq!(replica.store().head(&actor).unwrap(), Position::new(2));
}

#[test]
fn a_shards_standbys_are_the_next_active_members_after_its_owner() {
    use vertep_cluster::{Member, MemberStatus, View, standbys_of};
    let mut v = View::empty(vertep_cluster::ClusterId::new("test").unwrap());
    for (n, status) in [
        ("n0", MemberStatus::Active),
        ("n1", MemberStatus::Failed),
        ("n2", MemberStatus::Active),
        ("n3", MemberStatus::Active),
    ] {
        let id = common::node(n);
        v.members.insert(
            id.clone(),
            Member {
                id,
                endpoint: String::new(),
                status,
            },
        );
    }
    let names = |o: &str, rf| -> Vec<String> {
        standbys_of(&v, &common::node(o), rf)
            .iter()
            .map(|n| n.to_string())
            .collect()
    };
    assert_eq!(names("n0", 3), ["n2", "n3"], "the failed member is skipped");
    assert_eq!(names("n3", 3), ["n0", "n2"], "wrapping");
    assert_eq!(names("n2", 1), Vec::<String>::new());
    assert_eq!(names("n0", 5), ["n2", "n3"], "no more than are active");
}
