//! `init` scenarios in child processes (the subscriber is process-global), with the real
//! environment set through `Command::env` instead of `set_var`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use a3s_foundation::log::{Error, LOG_FILE_NAME, LogConfig, init};

const CHILD_VAR: &str = "A3S_VERIFY_LOG_CHILD";
const DIR_VAR: &str = "A3S_VERIFY_LOG_DIR";

fn run_child(log_dir: &Path, extra_env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .args([
            "verify_child_entry",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_remove("A3S_LOG")
        .env(CHILD_VAR, "1")
        .env(DIR_VAR, log_dir);
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("spawn child");
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn verify_child_entry() {
    let Some(dir) = std::env::var_os(DIR_VAR) else {
        return;
    };
    if std::env::var_os(CHILD_VAR).is_none() {
        return;
    }
    let handle = init(&LogConfig {
        debug: true,
        log_dir: Some(PathBuf::from(dir)),
        stderr: true,
    })
    .expect("first init");
    std::thread::Builder::new()
        .name("worker".to_owned())
        .spawn(|| {
            tracing::info!(target: "a3s_sync", n = 1, "hello");
            tracing::error!(target: "a3s_sync", "boom");
            // suppaftp logs through the `log` crate.
            log::trace!(target: "suppaftp::async_ftp::tokio_ftp::control", "CC OUT: PASS hunter2");
            log::debug!(target: "suppaftp::async_ftp::tokio_ftp::control", "CC IN: 230 ok");
        })
        .expect("spawn")
        .join()
        .expect("join");
    // A second init from another thread returns an error, never panics.
    let second = std::thread::Builder::new()
        .name("second-init".to_owned())
        .spawn(|| init(&LogConfig::default()))
        .expect("spawn")
        .join()
        .expect("no panic");
    assert!(matches!(second, Err(Error::AlreadyInitialized)));
    println!("LOGFILE {:?}", handle.log_file());
}

fn assert_line_shape(line: &str, thread: &str, level: &str, rest: &str) {
    // `YYYY-MM-DD HH:MM:SS.mmm [thread] [LEVEL] rest`
    let bytes = line.as_bytes();
    assert!(line.len() > 24, "{line:?}");
    for (i, b) in bytes[..23].iter().enumerate() {
        let expected_digit = !matches!(i, 4 | 7 | 10 | 13 | 16 | 19);
        assert_eq!(b.is_ascii_digit(), expected_digit, "{line:?} at {i}");
    }
    assert_eq!(
        &line[23..],
        format!(" [{thread}] [{level}] {rest}"),
        "{line:?}"
    );
}

/// G5: the first line of the debug file is the "enabled" line, even when `A3S_LOG` is invalid
/// (the filter warning comes after it).
#[test]
fn verify_first_line_with_invalid_filter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("logs");
    run_child(&dir, &[("A3S_LOG", "a3s_sync=[[[nonsense")]);
    let path = dir.join(LOG_FILE_NAME);
    let contents = std::fs::read_to_string(&path).expect("log file");
    let first = contents.lines().next().expect("one line");
    assert!(
        first.ends_with(&format!(
            "[INFO] Debug logging enabled. Log file: {}",
            path.display()
        )),
        "{contents}"
    );
}

/// G6 in the file: thread name, level names, extra fields, no ANSI escapes.
#[test]
fn verify_file_line_format() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("logs");
    let output = run_child(&dir, &[]);
    let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).expect("log file");
    assert!(!contents.contains('\u{1b}'), "{contents:?}");
    assert!(contents.ends_with('\n'));
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 3, "{contents}");
    assert_line_shape(lines[1], "worker", "INFO", "hello n=1");
    assert_line_shape(lines[2], "worker", "ERROR", "boom");
    // Stderr defaults to `warn`: only the error line, same format.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_lines: Vec<&str> = stderr.lines().filter(|line| line.contains("] [")).collect();
    assert_eq!(stderr_lines.len(), 1, "{stderr}");
    assert_line_shape(stderr_lines[0], "worker", "ERROR", "boom");
}

/// G4: an unusable log dir prints exactly one diagnostic line and continues.
#[test]
fn verify_unopenable_file_prints_one_line() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, "x").expect("write");
    let output = run_child(&blocker.join("logs"), &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LOGFILE None"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostic: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("Arma3Sync diagnostic logging could not be initialized: "))
        .collect();
    assert_eq!(diagnostic.len(), 1, "{stderr}");
}

/// `suppaftp` trace lines (`PASS <password>`) reach neither stderr nor the file, even with
/// `A3S_LOG=trace`; its debug lines still do.
#[test]
fn verify_ftp_password_never_logged() {
    for filter in ["trace", "suppaftp=trace", "suppaftp::async_ftp=trace"] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("logs");
        let output = run_child(&dir, &[("A3S_LOG", filter)]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("hunter2"), "{filter}: {stderr}");
        assert!(stderr.contains("CC IN: 230 ok"), "{filter}: {stderr}");
        let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).expect("log file");
        assert!(!contents.contains("hunter2"), "{filter}: {contents}");
    }
}
