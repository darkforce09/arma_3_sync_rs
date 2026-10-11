//! Acceptance tests that need neither a JDK nor the Java sources (A4, A5).

use std::process::Command;

use a3s_dev_tools::{Harness, HarnessError};

const BIN: &str = env!("CARGO_BIN_EXE_a3s_dev_tools");

#[test]
fn a4_missing_run_sh_is_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    match Harness::with_repo_root(tmp.path()) {
        Err(HarnessError::NotFound(root)) => assert_eq!(root, tmp.path()),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a4_missing_jdk_is_no_jdk_naming_a3s_java_home() {
    let harness = Harness::locate()
        .unwrap()
        .env("A3S_JAVA_HOME", "/nonexistent/a3s-jdk");
    let err = harness.build().unwrap_err();
    assert!(matches!(err, HarnessError::NoJdk(_)), "{err:?}");
    let message = err.to_string();
    assert!(message.contains("A3S_JAVA_HOME"), "{message}");
    let HarnessError::NoJdk(tail) = err else {
        unreachable!()
    };
    assert!(tail.contains("A3S_JAVA_HOME"), "{tail}");
}

#[test]
fn a4_locate_finds_this_repository() {
    let harness = Harness::locate().unwrap();
    assert!(harness.harness_dir().join("run.sh").is_file());
    assert!(harness.harness_dir().join("build.sh").is_file());
}

#[test]
fn a5_cli_rejects_write_fixtures_with_only() {
    let output = Command::new(BIN)
        .args(["regen-fixtures", "--write-fixtures", "--only", "local"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(BIN)
        .args(["regen-fixtures", "--check", "--write-fixtures"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(BIN)
        .args(["regen-fixtures", "--only", "nonsense"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a5_cli_missing_prerequisites_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let output = Command::new(BIN)
        .env("A3S_REPO_ROOT", tmp.path())
        .args(["java-verify", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("run.sh not found"), "{stderr}");
}
