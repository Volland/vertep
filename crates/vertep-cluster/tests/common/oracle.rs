//! The Lean lease and fencing model as a differential oracle for fault-injection histories.
#![allow(dead_code)]

use vertep_cluster::History;

/// The Lean lease checker `vertep-lease`, or `None` (with a message) when it is not built,
/// unless the `differential` feature requires it.
pub fn lease_oracle() -> Option<std::path::PathBuf> {
    let required = cfg!(feature = "differential");
    let tools = vertep_conformance::LeanTools::for_test(required)?;
    if tools.lease.is_file() {
        Some(tools.lease)
    } else if required {
        panic!(
            "{} is not built: run `cd lean/vertep-lean && lake build vertep-lease`",
            tools.lease.display()
        )
    } else {
        eprintln!("skipped: {} is not built", tools.lease.display());
        None
    }
}

/// Replays `h` through the Lean lease model in strict mode (every append under a recorded
/// grant): `Ok` with its summary when every decision agrees, `Err` with its reason otherwise.
pub fn lean_lease_verdict(lease: &std::path::Path, h: &History) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("history.jsonl");
    std::fs::write(&path, h.to_jsonl()).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(lease)
        .arg("--history")
        .arg(&path)
        .output()
        .map_err(|e| format!("{}: {e}", lease.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Checks `h`, keeps it under `VERTEP_HISTORY_DIR` as `cluster-history-<name>.jsonl`, and
/// replays it through the Lean model when `lease` is given.
pub fn verify(
    name: &str,
    h: &History,
    lease: Option<&std::path::Path>,
) -> vertep_cluster::CheckReport {
    if let Ok(dir) = std::env::var("VERTEP_HISTORY_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::path::Path::new(&dir).join(format!("cluster-history-{name}.jsonl"));
        std::fs::write(path, h.to_jsonl()).unwrap();
    }
    let r = h.check().unwrap_or_else(|v| panic!("{name}: {v}"));
    // the Lean lease and fencing model takes the same decisions (m4-cluster 6.5)
    if let Some(lease) = lease {
        lean_lease_verdict(lease, h)
            .unwrap_or_else(|e| panic!("{name}: the Lean lease model disagrees: {e}"));
    }
    assert_eq!(h.to_jsonl().lines().count(), h.events.len());
    r
}
