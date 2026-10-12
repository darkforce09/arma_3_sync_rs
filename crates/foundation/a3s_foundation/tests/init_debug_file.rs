// Its own test binary: `init` installs a process-global subscriber once per process.
//! `init` installs a process-global subscriber, so this file holds a single test.

use a3s_foundation::log::{Error, LOG_FILE_NAME, LogConfig, init};

#[test]
fn a_l5_init_debug_file_and_second_init() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let log_dir = tmp.path().join("data").join("logs");
    let handle = init(&LogConfig {
        debug: true,
        log_dir: Some(log_dir.clone()),
        stderr: false,
    })
    .expect("first init");

    let path = log_dir.join(LOG_FILE_NAME);
    assert_eq!(handle.log_file(), Some(path.as_path()));
    assert!(path.is_file());

    tracing::debug!(target: "a3s_sync", "kept");
    tracing::debug!(target: "hyper", "dropped");
    tracing::info!(target: "hyper", "kept too");

    let contents = std::fs::read_to_string(&path).expect("read log");
    let lines: Vec<&str> = contents.lines().collect();
    assert!(
        lines[0].ends_with(&format!(
            "Debug logging enabled. Log file: {}",
            path.display()
        )),
        "{contents}"
    );
    assert!(lines[0].contains("[INFO]"), "{contents}");
    assert_eq!(lines.len(), 3, "{contents}");
    assert!(lines[1].ends_with("[DEBUG] kept"), "{contents}");
    assert!(lines[2].ends_with("[INFO] kept too"), "{contents}");

    let second = init(&LogConfig::default());
    assert!(matches!(second, Err(Error::AlreadyInitialized)));
}
