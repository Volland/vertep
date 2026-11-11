//! Task 4.6: appends per second per shard and journal read (replay) speed. Ignored by default;
//! run with `cargo test --release -p vertep-tiramemsu --test throughput -- --ignored --nocapture`.

use std::time::Instant;

use vertep_store::testkit::{iri, sample_entry, vt};
use vertep_store::{AppendMeta, JournalStore, Position, StateDelta};
use vertep_tiramemsu::{TiramemsuBackend, shard_of};

#[test]
#[ignore = "benchmark"]
fn appends_and_replay_per_shard() {
    const ACTORS: usize = 20;
    const APPENDS: u64 = 50;
    let dir = tempfile::tempdir().unwrap();
    let backend = TiramemsuBackend::open(dir.path(), 1).unwrap();
    let actors: Vec<_> = (0..ACTORS).map(|i| iri(&format!("bench/{i}"))).collect();
    let start = Instant::now();
    for n in 0..APPENDS {
        for a in &actors {
            let delta = StateDelta {
                retract: (n > 0)
                    .then(|| oxrdf::Triple::new(a.clone(), vt("s"), iri(&format!("s{}", n - 1))))
                    .into_iter()
                    .collect(),
                assert: vec![oxrdf::Triple::new(
                    a.clone(),
                    vt("s"),
                    iri(&format!("s{n}")),
                )],
            };
            backend
                .append(
                    a,
                    Position::new(n),
                    &[sample_entry(a, &n.to_string())],
                    &AppendMeta::none().with_state(delta),
                )
                .unwrap();
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let total = ACTORS as f64 * APPENDS as f64;
    println!(
        "one shard: {total} appends (1 entry of 14 statements + state delta) in {secs:.2}s = {:.0} appends/s",
        total / secs
    );

    let start = Instant::now();
    let mut read = 0;
    for a in &actors {
        read += backend.read(a, Position::ZERO).unwrap().len();
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "replay read: {read} entries in {secs:.3}s = {:.0} entries/s",
        read as f64 / secs
    );
    assert_eq!(read, total as usize);

    // parallel appends on 4 shards
    let dir4 = tempfile::tempdir().unwrap();
    let four = TiramemsuBackend::open(dir4.path(), 4).unwrap();
    let per_shard: Vec<Vec<_>> = (0..4)
        .map(|s| {
            (0..)
                .map(|i| iri(&format!("bench4/{i}")))
                .filter(|a| shard_of(a.as_str(), 4) == s)
                .take(5)
                .collect()
        })
        .collect();
    let start = Instant::now();
    std::thread::scope(|sc| {
        for actors in &per_shard {
            let four = &four;
            sc.spawn(move || {
                for n in 0..APPENDS {
                    for a in actors {
                        four.append(
                            a,
                            Position::new(n),
                            &[sample_entry(a, &n.to_string())],
                            &AppendMeta::none(),
                        )
                        .unwrap();
                    }
                }
            });
        }
    });
    let secs = start.elapsed().as_secs_f64();
    let total = 4.0 * 5.0 * APPENDS as f64;
    println!(
        "four shards in parallel: {total} appends in {secs:.2}s = {:.0} appends/s",
        total / secs
    );
}
