//! Diagnostic log and elevated-startup marker.

use a3s_updater::{DiagnosticLog, UpdateError};

/// An error whose message does not repeat its source.
#[derive(Debug)]
struct Outer(std::io::Error);

impl std::fmt::Display for Outer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("copy failed")
    }
}

impl std::error::Error for Outer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn disabled_log_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("logs/updater.log");
    let log = DiagnosticLog::new(false, Some(path.clone()));
    log.info("hello");
    log.error("failed", None);
    assert!(!log.is_enabled());
    assert!(!path.exists());
}

#[test]
fn enabled_log_appends_lines() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("logs/updater.log");
    let log = DiagnosticLog::new(true, Some(path.clone()));
    assert_eq!(log.path(), Some(path.as_path()));
    log.info("first");
    let inner = UpdateError::Http { status: 503 };
    let err = Outer(std::io::Error::other("disk full"));
    log.error("second", Some(&inner));
    std::thread::Builder::new()
        .name("updater-worker".to_owned())
        .spawn({
            let log = log.clone();
            move || log.error("third", Some(&err))
        })
        .expect("spawn")
        .join()
        .expect("join");

    let text = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = text.lines().collect();
    let header = |line: &str, level: &str, message: &str| {
        let (timestamp, rest) = line.split_once(' ').expect("timestamp");
        assert!(
            chrono::DateTime::parse_from_rfc3339(timestamp).is_ok(),
            "timestamp {timestamp}"
        );
        assert!(rest.ends_with(&format!("[{level}] {message}")), "{line}");
        rest.to_owned()
    };
    header(lines[0], "INFO", "first");
    header(lines[1], "ERROR", "second");
    assert!(lines[2].contains("503"));
    let third = header(lines[3], "ERROR", "third");
    assert!(third.starts_with("[updater-worker]"), "{third}");
    assert_eq!(lines[4], "copy failed");
    assert_eq!(lines[5], "Caused by: disk full");
    assert_eq!(lines.len(), 6, "{text}");
}

#[test]
fn unwritable_path_is_ignored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let blocker = temp.path().join("file");
    std::fs::write(&blocker, "x").expect("write");
    let log = DiagnosticLog::new(true, Some(blocker.join("sub/updater.log")));
    log.info("lost");
    log.error("lost", None);
    log.mark_elevated_startup(&blocker.join("sub/marker"));
}

#[test]
fn elevated_startup_marker_is_written_even_when_disabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("nested/dir/elevated.marker");
    std::fs::create_dir_all(marker.parent().expect("parent")).expect("mkdir");
    std::fs::write(&marker, "a much longer previous content than a timestamp").expect("write");
    DiagnosticLog::disabled().mark_elevated_startup(&marker);
    let text = std::fs::read_to_string(&marker).expect("read");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&text).is_ok(),
        "{text}"
    );
}
