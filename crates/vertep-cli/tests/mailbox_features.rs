//! Mailbox configuration in a build without tiramemsu support. `cargo test --workspace` turns the
//! `tiramemsu` feature of vertep-runtime on (its own dev-dependencies ask for it), so CI also runs
//! `cargo test -p vertep-cli --test mailbox_features`, where it is off.

use vertep_runtime::mailbox::{MailboxConfig, TIRAMEMSU_SUPPORT};

// @tg: verifies:: [[openspec:messaging#Mailbox Configuration File#Tiramemsu route without tiramemsu support]] {level: unit}
#[test]
fn a_tiramemsu_backend_needs_the_feature() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("boxes");
    let cfg = MailboxConfig::from_json(&format!(
        r#"{{"routes":[{{"match":{{"scheme":"vt"}},
            "backend":{{"tiramemsu":{{"dir":{:?},"shards":2}}}}}}]}}"#,
        dir.to_str().unwrap()
    ))
    .unwrap();
    // Validation opens nothing and does not depend on the build.
    assert!(cfg.validate().is_ok());
    let built = cfg.build(&Default::default());
    if TIRAMEMSU_SUPPORT {
        assert!(built.is_ok(), "{:?}", built.err());
    } else {
        let Err(err) = built else {
            panic!("a tiramemsu backend was built without the feature")
        };
        assert!(err.0.contains("`tiramemsu` feature"), "{err}");
        assert!(!dir.exists(), "nothing is opened");
    }
}
