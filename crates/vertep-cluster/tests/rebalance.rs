//! The rebalancer's plan: the fewest shard moves when the membership view changes.

use std::collections::BTreeMap;

use vertep_cluster::{
    ClusterId, Member, MemberStatus, NodeId, ShardCount, ShardId, ShardOwner, View, plan_moves,
};

fn node(s: &str) -> NodeId {
    NodeId::new(s).unwrap()
}

fn view(members: &[(&str, MemberStatus)], owners: &[(u32, &str)]) -> View {
    View {
        cluster: ClusterId::new("test").unwrap(),
        version: 1,
        members: members
            .iter()
            .map(|(id, st)| {
                (
                    node(id),
                    Member {
                        id: node(id),
                        endpoint: format!("{id}:7400"),
                        status: *st,
                    },
                )
            })
            .collect(),
        owners: owners
            .iter()
            .map(|(s, n)| {
                (
                    ShardId::new(*s),
                    ShardOwner {
                        node: node(n),
                        epoch: 1,
                    },
                )
            })
            .collect(),
    }
}

fn per_node(v: &View, moves: &[vertep_cluster::Move]) -> BTreeMap<NodeId, usize> {
    let mut owner: BTreeMap<ShardId, NodeId> =
        v.owners.iter().map(|(s, o)| (*s, o.node.clone())).collect();
    for m in moves {
        owner.insert(m.shard, m.to.clone());
    }
    let mut out = BTreeMap::new();
    for n in owner.values() {
        *out.entry(n.clone()).or_insert(0) += 1;
    }
    out
}

// @tg: verifies:: [[openspec:shard-placement#Rebalancing By Journal File Shipping#Shard moved to a new node]] {level: unit}
#[test]
fn a_joining_node_takes_only_its_share() {
    use MemberStatus::Active;
    let sixteen = ShardCount::new(16).unwrap();
    // sixteen shards on two nodes, eight each; a third node joins
    let owners: Vec<(u32, &str)> = (0..16)
        .map(|s| (s, if s % 2 == 0 { "a" } else { "b" }))
        .collect();
    let v = view(&[("a", Active), ("b", Active), ("c", Active)], &owners);
    let moves = plan_moves(&v, sixteen);
    assert_eq!(moves.len(), 5, "only the new node's share moves: {moves:?}");
    assert!(
        moves.iter().all(|m| m.to == node("c")),
        "nothing moves between a and b"
    );
    assert!(moves.iter().all(|m| m.from.is_some()));
    let counts = per_node(&v, &moves);
    assert_eq!(counts.values().copied().collect::<Vec<_>>(), vec![6, 5, 5]);
    // a balanced view needs no move
    let mut after = v.clone();
    for m in &moves {
        after.owners.insert(
            m.shard,
            ShardOwner {
                node: m.to.clone(),
                epoch: 2,
            },
        );
    }
    assert!(plan_moves(&after, sixteen).is_empty());
}

#[test]
fn shards_of_a_failed_node_go_to_the_survivors() {
    use MemberStatus::{Active, Failed};
    let eight = ShardCount::new(8).unwrap();
    let owners: Vec<(u32, &str)> = (0..8)
        .map(|s| (s, ["a", "b", "c", "a"][s as usize % 4]))
        .collect();
    let v = view(&[("a", Active), ("b", Active), ("c", Failed)], &owners);
    let moves = plan_moves(&v, eight);
    let orphans: Vec<u32> = moves
        .iter()
        .filter(|m| m.from.is_none())
        .map(|m| m.shard.get())
        .collect();
    assert_eq!(orphans, vec![2, 6], "the failed node's shards");
    assert!(moves.iter().all(|m| m.to != node("c")));
    let counts = per_node(&v, &moves);
    assert_eq!(counts.get(&node("a")), Some(&4));
    assert_eq!(counts.get(&node("b")), Some(&4));
}
