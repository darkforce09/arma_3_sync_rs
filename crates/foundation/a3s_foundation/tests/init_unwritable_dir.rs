//! `init` installs a process-global subscriber, so this file holds a single test.

use a3s_foundation::log::{LogConfig, init};

#[test]
fn a_l5_init_succeeds_without_debug_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let not_a_dir = tmp.path().join("file");
    std::fs::write(&not_a_dir, "x").expect("write");

    let handle = init(&LogConfig {
        debug: true,
        log_dir: Some(not_a_dir),
        stderr: true,
    })
    .expect("init succeeds without the file");
    assert_eq!(handle.log_file(), None);
    assert!(handle.writer().is_none());
    // Logging still works.
    tracing::warn!("still logging");
}
