//! Installation into an existing installation: managed files, user data, state file safety.

use std::path::Path;

use a3s_updater::{
    CopyReplacer, InstallReport, MANAGED_FILES_NAME, StagedEntry, UpdateError, install_staged,
    is_user_managed,
};

use crate::support::stage_tree;

const LINE_SEPARATOR: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Installs every file below `source` into `dest`.
fn install(source: &Path, dest: &Path) -> Result<InstallReport, UpdateError> {
    install_staged(dest, &stage_tree(source), &CopyReplacer)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}

#[test]
fn removes_previously_managed_files_but_keeps_user_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("extracted");
    write(&dest.join("bin/old-launcher.exe"), "old");
    write(&dest.join("user-file.txt"), "keep");
    write(&dest.join(MANAGED_FILES_NAME), "bin/old-launcher.exe\n");
    write(
        &dest.join("resources/configuration/updater.toml"),
        "existing user config",
    );
    write(&dest.join("profiles/me/a.profile"), "mine");
    write(&source.join("bin/new-launcher.exe"), "new");
    write(&source.join("user-file.txt"), "from update");
    write(
        &source.join("resources/configuration/updater.toml"),
        "shipped config",
    );
    write(&source.join("Profiles/me/a.profile"), "shipped");

    let report = install(&source, &dest).expect("install");
    assert!(!dest.join("bin/old-launcher.exe").exists());
    assert_eq!(read(&dest.join("bin/new-launcher.exe")), "new");
    assert!(dest.join("user-file.txt").exists());
    assert_eq!(
        read(&dest.join("resources/configuration/updater.toml")),
        "existing user config"
    );
    assert_eq!(read(&dest.join("profiles/me/a.profile")), "mine");
    let state = read(&dest.join(MANAGED_FILES_NAME));
    assert!(state.lines().any(|l| l == "bin/new-launcher.exe"));
    assert!(!state.contains("updater.toml"));
    assert_eq!(report.removed, ["bin/old-launcher.exe"]);
    assert_eq!(
        report.skipped_user_files,
        [
            "Profiles/me/a.profile",
            "resources/configuration/updater.toml"
        ]
    );
}

#[test]
fn replaces_changed_files_and_drops_renamed_entries_from_the_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("extracted");
    write(&dest.join("Arma3Sync.jar"), "old-content");
    write(&dest.join(MANAGED_FILES_NAME), "ArmA3Sync.jar\n");
    write(&source.join("Arma3Sync.jar"), "new-content");

    install(&source, &dest).expect("install");
    assert_eq!(read(&dest.join("Arma3Sync.jar")), "new-content");
    let state = read(&dest.join(MANAGED_FILES_NAME));
    assert!(state.contains("Arma3Sync.jar"));
    assert!(!state.contains("ArmA3Sync.jar"));
}

#[test]
fn keeps_legacy_root_configuration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("legacy-installation");
    let source = temp.path().join("legacy-extracted");
    write(&dest.join("a3s.cfg"), "repository paths");
    write(&dest.join("a3s.prefs"), "preferences");
    write(&dest.join("bin/old-launcher.exe"), "old");
    write(
        &dest.join(MANAGED_FILES_NAME),
        "a3s.cfg\r\na3s.prefs\r\n\r\nbin\\old-launcher.exe\r\n",
    );
    write(&source.join("bin/new-launcher.exe"), "new");

    install(&source, &dest).expect("install");
    assert!(dest.join("a3s.cfg").exists());
    assert!(dest.join("a3s.prefs").exists());
    assert!(!dest.join("bin/old-launcher.exe").exists());
    assert!(dest.join("bin/new-launcher.exe").exists());
    let state = read(&dest.join(MANAGED_FILES_NAME));
    assert!(!state.contains("a3s.cfg"));
    assert!(!state.contains("a3s.prefs"));
}

#[test]
fn unsafe_state_is_rejected_before_anything_changes() {
    for line in ["../outside.txt", "/etc/passwd", "a/../..", ".", "C:\\x"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("installation");
        let source = temp.path().join("extracted");
        write(&dest.join(MANAGED_FILES_NAME), &format!("{line}\n"));
        write(&dest.join("keep.txt"), "keep");
        write(&source.join("keep.txt"), "replaced");
        write(&temp.path().join("outside.txt"), "outside");

        let err = install(&source, &dest).expect_err(line);
        assert!(
            matches!(err, UpdateError::UnsafeStatePath(_)),
            "{line}: {err:?}"
        );
        assert_eq!(read(&dest.join("keep.txt")), "keep");
        assert!(temp.path().join("outside.txt").exists());
    }

    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(MANAGED_FILES_NAME)).expect("mkdir");
    let err = install_staged(temp.path(), &[], &CopyReplacer).expect_err("state dir");
    assert!(matches!(err, UpdateError::StateNotFile(_)), "{err:?}");
}

#[test]
fn missing_sources_are_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let entries = [StagedEntry {
        source: temp.path().join("gone.tmp"),
        relative: "bin/x".to_owned(),
        unix_mode: None,
    }];
    let err = install_staged(&dest, &entries, &CopyReplacer).expect_err("missing entry");
    assert!(matches!(err, UpdateError::MissingSource(_)), "{err:?}");
}

#[test]
fn non_empty_directory_in_the_state_fails_and_keeps_the_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("extracted");
    write(&dest.join("plugins/user.dll"), "user");
    write(&dest.join("empty/.placeholder"), "");
    std::fs::remove_file(dest.join("empty/.placeholder")).expect("rm");
    write(&dest.join(MANAGED_FILES_NAME), "empty\nplugins\n");
    write(&source.join("bin/new.exe"), "new");
    write(&source.join(".A3S-UPDATER-FILES"), "bogus");

    let err = install(&source, &dest).expect_err("non-empty dir");
    assert!(matches!(err, UpdateError::Io { .. }), "{err:?}");
    assert_eq!(read(&dest.join(MANAGED_FILES_NAME)), "empty\nplugins\n");
    assert!(dest.join("plugins/user.dll").exists());

    // Without the non-empty directory the install completes; the archive's own copy of the
    // state file is never installed.
    write(&dest.join(MANAGED_FILES_NAME), "empty\n");
    let report = install(&source, &dest).expect("install");
    assert!(!dest.join("empty").exists());
    assert_eq!(report.installed, ["bin/new.exe"]);
    let state = read(&dest.join(MANAGED_FILES_NAME));
    assert_eq!(state.lines().collect::<Vec<_>>(), ["bin/new.exe"]);
    assert!(!state.contains("bogus"));
}

/// On a case-insensitive file system the old spelling is the file just installed and must
/// survive; on a case-sensitive one it does not exist.
#[test]
fn previous_spelling_of_an_installed_file_is_kept() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("extracted");
    write(&dest.join("ArmA3Sync.exe"), "old");
    write(&dest.join(MANAGED_FILES_NAME), "ARMA3SYNC.EXE\r\n");
    write(&source.join("ArmA3Sync.exe"), "new");
    let report = install(&source, &dest).expect("install");
    assert!(report.removed.is_empty());
    assert_eq!(read(&dest.join("ArmA3Sync.exe")), "new");
    assert_eq!(
        read(&dest.join(MANAGED_FILES_NAME)),
        format!("ArmA3Sync.exe{LINE_SEPARATOR}")
    );
}

#[test]
fn user_managed_paths() {
    for path in [
        "a3s.cfg",
        "A3S.PREFS",
        "a3s.xml",
        "updater.toml",
        "resources\\configuration\\updater.toml",
        "resources/configuration/a3s.cfg",
        "Resources/Configuration/A3S.prefs",
        "resources/configuration/a3s.xml",
        "profiles",
        "Profiles/x/y",
        "resources/ftp",
        "resources/FTP/cache",
        "resources/temp/x",
        "arma3sync-rs.portable",
        "Arma3sync-RS-data",
        "arma3sync-rs-data/config/settings.toml",
    ] {
        assert!(is_user_managed(path), "{path}");
    }
    for path in [
        "bin/a3s.cfg",
        "profilesx",
        "resources/ftpx",
        "a3s.cfg.bak",
        "arma3sync-rs-datax/x",
        "bin/arma3sync-rs.portable",
    ] {
        assert!(!is_user_managed(path), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn copy_replacer_renames_over_the_target() {
    use std::os::unix::fs::MetadataExt as _;

    use a3s_updater::FileReplacer as _;
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("dir/app");
    let source = temp.path().join("staged.tmp");
    write(&target, "old");
    write(&source, "new");
    let original_inode = std::fs::metadata(&target).expect("meta").ino();
    let mut open = std::fs::File::open(&target).expect("open");

    CopyReplacer
        .replace(&source, &target, Some(0o755))
        .expect("replace");
    assert_ne!(
        std::fs::metadata(&target).expect("meta").ino(),
        original_inode
    );
    let old = std::io::read_to_string(&mut open).expect("read");
    assert_eq!(old, "old");
    assert_eq!(read(&target), "new");
    let siblings: Vec<_> = std::fs::read_dir(temp.path().join("dir"))
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(siblings, ["app"]);
}

#[test]
fn hostile_staged_paths_are_rejected_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("staged.tmp");
    write(&source, "payload");
    for relative in [
        "../x",
        "/etc/x",
        "a/../../x",
        "C:/x",
        "nul",
        "x/..",
        "bin/a:b",
    ] {
        let entries = [StagedEntry {
            source: source.clone(),
            relative: relative.to_owned(),
            unix_mode: None,
        }];
        let err = install_staged(&dest, &entries, &CopyReplacer).expect_err(relative);
        assert!(
            matches!(err, UpdateError::UnsafeEntry(_)),
            "{relative}: {err:?}"
        );
    }
    assert!(!temp.path().join("x").exists());
    assert_eq!(std::fs::read_dir(&dest).expect("list").count(), 0);

    let colliding = [
        StagedEntry {
            source: source.clone(),
            relative: "bin/App".to_owned(),
            unix_mode: None,
        },
        StagedEntry {
            source,
            relative: "bin/app".to_owned(),
            unix_mode: None,
        },
    ];
    let err = install_staged(&dest, &colliding, &CopyReplacer).expect_err("collision");
    assert!(matches!(err, UpdateError::PathConflict(_)), "{err:?}");
    assert!(!dest.join("bin").exists());
}

#[test]
fn managed_file_replaced_by_a_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    write(&first.join("x"), "file");
    write(&first.join("keep.txt"), "1");
    install(&first, &dest).expect("first version");

    write(&second.join("x/y"), "nested");
    write(&second.join("keep.txt"), "2");
    let report = install(&second, &dest).expect("second version");
    assert_eq!(read(&dest.join("x/y")), "nested");
    assert_eq!(report.removed, ["x"]);
    let state = read(&dest.join(MANAGED_FILES_NAME));
    assert_eq!(state.lines().collect::<Vec<_>>(), ["keep.txt", "x/y"]);
}

#[test]
fn unmanaged_conflicts_fail_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let source = temp.path().join("extracted");
    write(&dest.join("x"), "user file");
    write(&dest.join("dir/inside.txt"), "user");
    write(&source.join("a.txt"), "new");
    write(&source.join("x/y"), "nested");
    let err = install(&source, &dest).expect_err("file in the way");
    assert!(matches!(err, UpdateError::PathConflict(_)), "{err:?}");
    assert!(!dest.join("a.txt").exists());
    assert_eq!(read(&dest.join("x")), "user file");

    let source = temp.path().join("extracted-2");
    write(&source.join("a.txt"), "new");
    write(&source.join("dir"), "file over a directory");
    let err = install(&source, &dest).expect_err("directory in the way");
    assert!(matches!(err, UpdateError::PathConflict(_)), "{err:?}");
    assert!(!dest.join("a.txt").exists());
    assert!(dest.join("dir/inside.txt").exists());
}
