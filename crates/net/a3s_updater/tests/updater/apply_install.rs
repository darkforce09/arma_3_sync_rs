//! Release layout, installation kinds and installing a downloaded archive.

use std::path::{Path, PathBuf};

use a3s_foundation::paths::Os;
use a3s_updater::apply::install::{
    InstallKind, check_release_layout, detect_install_kind, install_verified_archive,
};
use a3s_updater::apply::replace::SelfReplacer;
use a3s_updater::{ApplyError, DiagnosticLog, MANAGED_FILES_NAME, StagedEntry, UpdateError};

use crate::apply_fixture::NEW_BINARY;
use crate::support::{Entry, sha256_hex, write_zip};

fn entries(names: &[(&str, Option<u32>)]) -> Vec<StagedEntry> {
    names
        .iter()
        .map(|&(relative, unix_mode)| StagedEntry {
            source: PathBuf::from("staged"),
            relative: relative.to_owned(),
            unix_mode,
        })
        .collect()
}

#[test]
fn release_must_hold_the_platform_binary_at_its_root() {
    let missing = check_release_layout(&mut entries(&[("LICENSE.txt", None)]), Os::Linux);
    let nested = check_release_layout(
        &mut entries(&[("Arma3Sync-1.0/arma3sync", None)]),
        Os::Linux,
    );
    assert!(missing.is_err());
    assert_eq!(missing, nested);
    assert!(check_release_layout(&mut entries(&[("arma3sync.exe", None)]), Os::Linux).is_err());
    assert!(check_release_layout(&mut entries(&[("ARMA3SYNC", None)]), Os::Linux).is_err());
    assert!(check_release_layout(&mut entries(&[("ARMA3SYNC.EXE", None)]), Os::Windows).is_ok());
}

#[test]
fn release_must_not_carry_portable_user_data() {
    for name in [
        "arma3sync-rs-data/x",
        "Arma3Sync-RS.portable",
        "ARMA3SYNC-RS-DATA",
    ] {
        let mut list = entries(&[("arma3sync", None), (name, None)]);
        assert!(
            check_release_layout(&mut list, Os::Linux).is_err(),
            "{name}"
        );
    }
}

#[test]
fn binary_becomes_executable_on_unix() {
    for (mode, expected) in [
        (None, 0o755),
        (Some(0o644), 0o755),
        (Some(0o750), 0o750),
        (Some(0o104_755), 0o755),
    ] {
        let mut list = entries(&[("arma3sync", mode), ("LICENSE.txt", Some(0o644))]);
        check_release_layout(&mut list, Os::Linux).expect("layout");
        assert_eq!(list[0].unix_mode, Some(expected), "{mode:?}");
        assert_eq!(list[1].unix_mode, Some(0o644));
    }
    let mut list = entries(&[("arma3sync.exe", None)]);
    check_release_layout(&mut list, Os::Windows).expect("layout");
    assert_eq!(list[0].unix_mode, None);
}

#[test]
fn installation_kinds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plain = temp.path().join("plain");
    let java = temp.path().join("java");
    let cargo = temp.path().join("target/debug");
    for dir in [&plain, &java, &cargo] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    std::fs::write(java.join("ArmA3Sync-Updater.jar"), b"jar").expect("jar");
    std::fs::write(cargo.join(".cargo-lock"), b"").expect("lock");
    let exe = |dir: &Path| dir.join("arma3sync");

    assert_eq!(
        detect_install_kind(&exe(&plain), Os::Linux, Some("/tmp/a.AppImage")),
        InstallKind::AppImage
    );
    assert_eq!(
        detect_install_kind(&exe(&plain), Os::Linux, Some("  ")),
        InstallKind::Directory
    );
    assert_eq!(
        detect_install_kind(&exe(&plain), Os::Windows, Some("/tmp/a.AppImage")),
        InstallKind::Directory
    );
    assert_eq!(
        detect_install_kind(
            Path::new("/Applications/Arma3Sync.app/Contents/MacOS/arma3sync"),
            Os::MacOs,
            None
        ),
        InstallKind::MacBundle
    );
    assert_eq!(
        detect_install_kind(&exe(&java), Os::Windows, None),
        InstallKind::JavaInstallation
    );
    assert_eq!(
        detect_install_kind(&exe(&cargo), Os::Linux, None),
        InstallKind::Development
    );
    assert_eq!(
        detect_install_kind(&exe(&plain), Os::Linux, None),
        InstallKind::Directory
    );
}

/// An installation directory and an archive file next to it.
fn setup(archive: &[Entry<'_>]) -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let install = temp.path().join("installation");
    std::fs::create_dir_all(&install).expect("mkdir");
    std::fs::write(install.join("arma3sync"), b"old").expect("old");
    let zip = temp.path().join("update.zip");
    write_zip(&zip, archive);
    let sha256 = sha256_hex(&std::fs::read(&zip).expect("zip"));
    (temp, install, zip, sha256)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn verified_archive_is_installed_and_staging_removed() {
    let (_temp, install, zip, sha256) = setup(&[
        Entry::File("arma3sync", NEW_BINARY),
        Entry::File("LICENSE.txt", b"license"),
    ]);
    std::fs::write(install.join(".a3s-old-1-0-arma3sync"), b"leftover").expect("leftover");

    let report = install_verified_archive(
        &zip,
        &sha256.to_ascii_uppercase(),
        &install,
        Os::Linux,
        &SelfReplacer::new(),
        &DiagnosticLog::disabled(),
    )
    .expect("install");

    assert_eq!(report.installed, ["arma3sync", "LICENSE.txt"]);
    assert_eq!(
        std::fs::read(install.join("arma3sync")).expect("read"),
        NEW_BINARY
    );
    assert_eq!(
        names(&install),
        [MANAGED_FILES_NAME, "LICENSE.txt", "arma3sync"]
    );
}

#[test]
fn checksum_mismatch_and_bad_layout_leave_the_installation_unchanged() {
    let (_temp, install, zip, _) = setup(&[Entry::File("arma3sync", b"new")]);
    let err = install_verified_archive(
        &zip,
        &"0".repeat(64),
        &install,
        Os::Linux,
        &SelfReplacer::new(),
        &DiagnosticLog::disabled(),
    )
    .expect_err("integrity");
    assert!(matches!(err, ApplyError::Download(UpdateError::Integrity)));
    assert_eq!(names(&install), ["arma3sync"]);

    let (_temp, install, zip, sha256) = setup(&[Entry::File("other", b"new")]);
    let err = install_verified_archive(
        &zip,
        &sha256,
        &install,
        Os::Linux,
        &SelfReplacer::new(),
        &DiagnosticLog::disabled(),
    )
    .expect_err("layout");
    assert!(matches!(err, ApplyError::Layout(_)));
    assert_eq!(names(&install), ["arma3sync"]);
    assert_eq!(
        std::fs::read(install.join("arma3sync")).expect("read"),
        b"old"
    );
}

#[test]
fn invalid_archive_is_an_install_error() {
    let (_temp, install, zip, _) = setup(&[]);
    std::fs::write(&zip, b"not a zip").expect("zip");
    let sha256 = sha256_hex(b"not a zip");
    let err = install_verified_archive(
        &zip,
        &sha256,
        &install,
        Os::Linux,
        &SelfReplacer::new(),
        &DiagnosticLog::disabled(),
    )
    .expect_err("invalid");
    assert!(matches!(
        err,
        ApplyError::Install {
            source: UpdateError::Stage {
                stage: "archive validation",
                ..
            },
            ..
        }
    ));
    assert!(!err.is_permission_failure());
}
