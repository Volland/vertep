//! Recorded outputs: compare with the file, or rewrite it under `VERTEP_BLESS=1`.

use std::path::{Path, PathBuf};

/// The `examples/` directory.
pub fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Asserts that `actual` equals the recorded file `rel` (relative to `examples/`). With
/// `VERTEP_BLESS=1` set, writes `actual` to the file instead.
pub fn check_recorded(rel: &str, actual: &str) {
    let path = examples_dir().join(rel);
    if std::env::var_os("VERTEP_BLESS").is_some() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create the output directory");
        }
        std::fs::write(&path, actual).expect("write the recorded output");
        return;
    }
    let expected = read(&path);
    assert!(
        expected == actual,
        "{rel} differs from the actual output (rerun with VERTEP_BLESS=1 to record it)\n\
         --- recorded\n{expected}\n--- actual\n{actual}"
    );
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}
