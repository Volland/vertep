//! Randomised custody faults: hosts elect at random epochs, write, crash and recover; links go
//! down and come back; old batches are delivered again late. Every run checks the safety
//! properties on its history in Rust (one writer per epoch; every committed log survives into
//! every later election and every later commit) and, when the Lean custody model is built,
//! replays the history through it in strict mode (`vertep-custody --history`).
//!
//! `VERTEP_CUSTODY_SEEDS` sets the number of seeds (default 16; CI runs 256). With the `differential` cargo
//! feature a missing Lean model fails the test instead of skipping the replay.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use vertep_objects::custody::{
    Custodian, CustodyHost, CustodyRequest, History, ReconfigurationRequest,
};
use vertep_objects::testkit::{CustodyFixture, turn};
use vertep_store::Position;

const HOSTS: [&str; 3] = [
    "did:web:a.example.org",
    "did:web:b.example.org",
    "did:web:c.example.org",
];
const KEEPERS: [&str; 2] = ["did:web:k1.example.net", "did:web:k2.example.net"];
/// A spare keeper that reconfigurations swap in for, and out of, K2's place.
const SPARE: &str = "did:web:k3.example.net";

/// xorshift64*: deterministic per seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn hashes(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|h| h.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn is_prefix(a: &[String], b: &[String]) -> bool {
    a.len() <= b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// The safety properties on a history.
fn check(history: &[Value]) -> Result<(), String> {
    let mut elected: BTreeMap<u64, String> = BTreeMap::new();
    let mut commits: Vec<(u64, Vec<String>)> = Vec::new();
    for (i, e) in history.iter().enumerate() {
        let epoch = e["epoch"].as_u64().unwrap_or(0);
        match e["kind"].as_str() {
            Some("elected") => {
                let host = e["host"].as_str().unwrap_or_default().to_owned();
                if let Some(other) = elected.insert(epoch, host.clone())
                    && other != host
                {
                    return Err(format!("line {i}: epoch {epoch} won by {other} and {host}"));
                }
                let adopted = hashes(&e["adopted"]);
                for (ce, log) in &commits {
                    if *ce < epoch && !is_prefix(log, &adopted) {
                        return Err(format!(
                            "line {i}: epoch {epoch} adopted a chain without the log committed in {ce}"
                        ));
                    }
                }
            }
            Some("committed") => {
                let log = hashes(&e["log"]);
                for (ce, prev) in &commits {
                    if !(is_prefix(prev, &log) || is_prefix(&log, prev)) {
                        return Err(format!(
                            "line {i}: commit in {epoch} contradicts the commit in {ce}"
                        ));
                    }
                    if *ce < epoch && !is_prefix(prev, &log) {
                        return Err(format!("line {i}: commit in {epoch} lost entries of {ce}"));
                    }
                }
                commits.push((epoch, log));
            }
            _ => {}
        }
    }
    Ok(())
}

/// One randomised run; returns its history.
fn run(seed: u64, steps: usize) -> Arc<History> {
    let mut f = CustodyFixture::new(&HOSTS, &KEEPERS, None);
    let spare = f.make_party(SPARE);
    f.parties.push(spare);
    let history = History::new(&f.config);
    for p in &f.parties {
        p.replica.set_history(history.clone());
    }
    let hosts: Vec<Arc<CustodyHost>> = HOSTS
        .iter()
        .map(|d| {
            let h = f.custody_host(d);
            h.set_history(history.clone());
            h
        })
        .collect();
    let all: Vec<&str> = HOSTS
        .iter()
        .chain(KEEPERS.iter())
        .chain(std::iter::once(&SPARE))
        .copied()
        .collect();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut turn_no = 0u64;
    for _ in 0..steps {
        match rng.below(100) {
            // Partitions: a custodian's links go down or come back.
            0..=14 => {
                let d = all[rng.below(all.len() as u64) as usize];
                f.set_up(d, rng.below(3) != 0);
            }
            // Elections, at the next free epoch or at a random (possibly colliding) one.
            15..=29 => {
                let h = &hosts[rng.below(3) as usize];
                h.resume();
                if rng.below(2) == 0 {
                    let _ = h.activate();
                } else {
                    let _ = h.elect(rng.below(24) + 1);
                }
            }
            // Turns by whoever believes it writes.
            30..=69 => {
                let h = &hosts[rng.below(3) as usize];
                if h.writer_epoch().is_some() {
                    turn_no += 1;
                    let obj = f.object.iri();
                    let _ = h.append_turn(
                        h.committed(),
                        &[turn(&obj, h.committed().get() + 1, &format!("t{turn_no}"))],
                    );
                }
            }
            // Delays and duplicates: an old batch is delivered again, late.
            70..=79 => {
                let p = &f.parties[rng.below(all.len() as u64) as usize];
                let carried = p.transport.carried();
                let appends: Vec<&String> =
                    carried.iter().filter(|m| m.contains("#append")).collect();
                if !appends.is_empty() {
                    let m = appends[rng.below(appends.len() as u64) as usize];
                    if let Ok(req) = serde_json::from_str::<CustodyRequest>(m) {
                        let _ = p.replica.handle(&req);
                    }
                }
            }
            // Crashes: a custodian recovers from its store; a host also loses its memory.
            80..=89 => {
                let i = rng.below(all.len() as u64) as usize;
                f.parties[i].replica.reload().unwrap();
                if i < hosts.len() {
                    hosts[i].restart();
                }
            }
            // Custodian-set changes: a writer swaps K2 and the spare keeper (joint consensus).
            90..=93 => {
                let h = &hosts[rng.below(3) as usize];
                if h.writer_epoch().is_some() {
                    let cur = h.config();
                    let (out, inn) = if cur.role_of(KEEPERS[1]).is_some() {
                        (KEEPERS[1], SPARE)
                    } else {
                        (SPARE, KEEPERS[1])
                    };
                    let mut cfg = cur.clone();
                    cfg.version += 1;
                    cfg.custodians.retain(|c| c.id != out);
                    cfg.custodians.push(Custodian::keeper(inn));
                    let req = ReconfigurationRequest {
                        object: f.object.as_str().into(),
                        config: cfg,
                        did_log_entry: None,
                        signature: String::new(),
                    }
                    .sign(&f.controller.signer)
                    .unwrap();
                    let _ = h.reconfigure(&req);
                }
            }
            // Catch-up attempts and reads.
            _ => {
                let h = &hosts[rng.below(3) as usize];
                let _ = h.read_committed(Position::ZERO);
            }
        }
    }
    history
}

fn lean_custody() -> Option<PathBuf> {
    let dir = std::env::var_os("VT_LEAN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean/.lake/build/bin")
        });
    let bin = dir.join("vertep-custody");
    bin.is_file().then_some(bin)
}

// @tg: verifies:: [[openspec:object-custody#Quorum-Promised Custody Epochs]] {level: integration}
// @tg: verifies:: [[openspec:object-custody#Quorum Before Effects]] {level: integration}
#[test]
fn randomised_custody_faults_keep_safety() {
    let seeds: u64 = std::env::var("VERTEP_CUSTODY_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("custody-histories");
    std::fs::create_dir_all(&out).unwrap();
    let lean = lean_custody();
    if lean.is_none() && cfg!(feature = "differential") {
        panic!(
            "the Lean custody model is not built: cd lean/vertep-lean && lake build vertep-custody"
        );
    }
    let mut elections = 0;
    let mut commits = 0;
    for seed in 0..seeds {
        let h = run(seed, 300);
        let events = h.events();
        check(&events).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        elections += events.iter().filter(|e| e["kind"] == "elected").count();
        commits += events.iter().filter(|e| e["kind"] == "committed").count();
        let path = out.join(format!("seed-{seed}.jsonl"));
        std::fs::write(&path, h.to_jsonl()).unwrap();
        if let Some(bin) = &lean {
            let o = std::process::Command::new(bin)
                .arg("--history")
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "seed {seed}: the Lean custody model disagrees ({}): {}",
                path.display(),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }
    assert!(elections > seeds as usize, "the runs exercise elections");
    assert!(commits > seeds as usize, "the runs exercise commits");
    if lean.is_none() {
        eprintln!("vertep-custody is not built: histories checked in Rust only");
    }
}
