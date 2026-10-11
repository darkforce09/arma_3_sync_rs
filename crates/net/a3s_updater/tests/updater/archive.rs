//! Archive validation and staging: entry-name safety, CRC, permissions, links.

use std::path::Path;

use a3s_updater::{
    CopyReplacer, StagedEntry, UpdateError, install_archive, install_staged, stage_archive,
    validate_archive,
};

use crate::support::{Entry, write_zip};

/// Validates and stages a one-file archive named `name`.
fn stage_one(
    dir: &Path,
    name: &str,
) -> (
    Result<(), UpdateError>,
    Result<Vec<StagedEntry>, UpdateError>,
) {
    let archive = dir.join(format!("{}.zip", dir.read_dir().expect("list").count()));
    write_zip(&archive, &[Entry::File(name, b"data")]);
    (
        validate_archive(&archive),
        stage_archive(&archive, &dir.join("staged")),
    )
}

#[test]
fn entry_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let rejected = [
        "../../outside.txt",
        "/etc/x",
        "C:/x",
        "C:x",
        "\\\\server\\share\\x",
        "a/../../x",
        "a:b",
        "dir/stream:x",
        " ",
        "a/..",
        ".",
    ];
    for name in rejected {
        let (validated, staged) = stage_one(temp.path(), name);
        assert!(validated.is_err(), "validate accepted {name:?}");
        assert!(staged.is_err(), "stage accepted {name:?}");
    }
    assert!(!temp.path().join("outside.txt").exists());
    assert!(
        !temp
            .path()
            .parent()
            .expect("parent")
            .join("outside.txt")
            .exists()
    );

    for (name, relative) in [
        ("a/../b", "b"),
        ("./a", "a"),
        ("runtime\\legal\\x", "runtime/legal/x"),
        ("bin//./tool", "bin/tool"),
    ] {
        let (validated, staged) = stage_one(temp.path(), name);
        validated.expect(name);
        let staged = staged.expect(name);
        let entry = staged.last().expect("entry");
        assert_eq!(entry.relative, relative);
        assert!(entry.source.starts_with(temp.path().join("staged")));
        assert!(entry.source.is_file());
    }
}

#[test]
fn normal_entry_is_staged_and_installed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("update.zip");
    write_zip(&archive, &[Entry::File("bin/launcher.txt", b"ok")]);
    validate_archive(&archive).expect("valid");
    let entries = stage_archive(&archive, &temp.path().join("extract")).expect("stage");
    let dest = temp.path().join("installation");
    install_staged(&dest, &entries, &CopyReplacer).expect("install");
    assert_eq!(
        std::fs::read_to_string(dest.join("bin/launcher.txt")).expect("read"),
        "ok"
    );
}

#[test]
fn truncated_empty_and_corrupt_archives() {
    let temp = tempfile::tempdir().expect("tempdir");
    let truncated = temp.path().join("truncated.zip");
    std::fs::write(&truncated, [0x50, 0x4b, 0x03, 0x04, 0x01]).expect("write");
    assert!(matches!(
        validate_archive(&truncated),
        Err(UpdateError::ArchiveInvalid(_))
    ));

    let empty = temp.path().join("empty.zip");
    write_zip(&empty, &[Entry::Directory("only/")]);
    assert!(matches!(
        validate_archive(&empty),
        Err(UpdateError::ArchiveEmpty)
    ));

    // Flip one byte of the stored data: the CRC check fails during validation.
    let corrupt = temp.path().join("corrupt.zip");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("a.txt", stored).expect("start");
    std::io::Write::write_all(&mut writer, b"0123456789abcdef").expect("write");
    let mut bytes = writer.finish().expect("finish").into_inner();
    let at = bytes
        .windows(16)
        .position(|w| w == b"0123456789abcdef")
        .expect("data");
    bytes[at] ^= 0xff;
    std::fs::write(&corrupt, bytes).expect("write");
    assert!(matches!(
        validate_archive(&corrupt),
        Err(UpdateError::ArchiveInvalid(_))
    ));
}

#[test]
fn every_entry_is_validated_before_anything_is_installed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("update.zip");
    write_zip(
        &archive,
        &[
            Entry::File("bin/first.txt", b"1"),
            Entry::File("../escape.txt", b"2"),
        ],
    );
    let dest = temp.path().join("installation");
    let err = install_archive(&archive, &dest, &CopyReplacer).expect_err("unsafe");
    assert!(
        matches!(&err, UpdateError::Stage { stage: "archive validation", source } if matches!(**source, UpdateError::UnsafeEntry(_))),
        "{err:?}"
    );
    assert!(!dest.join("bin/first.txt").exists());
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn runtime_tree_is_copied_from_the_staged_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("runtime.zip");
    write_zip(
        &archive,
        &[
            Entry::File("runtime/legal/java.base/LICENSE", b"license"),
            Entry::File("runtime/bin/java.exe", &[1, 2, 3]),
        ],
    );
    validate_archive(&archive).expect("valid");
    let entries = stage_archive(&archive, &temp.path().join("extracted")).expect("stage");
    // Staged files are flat.
    assert!(
        entries
            .iter()
            .all(|e| e.source.parent() == Some(&*temp.path().join("extracted")))
    );
    let dest = temp.path().join("installation");
    install_staged(&dest, &entries, &CopyReplacer).expect("install");
    assert_eq!(
        std::fs::read_to_string(dest.join("runtime/legal/java.base/LICENSE")).expect("read"),
        "license"
    );
    assert_eq!(
        std::fs::read(dest.join("runtime/bin/java.exe")).expect("read"),
        [1, 2, 3]
    );
}

#[test]
fn backslash_directory_entries_are_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("windows-style.zip");
    write_zip(
        &archive,
        &[
            Entry::Directory("runtime\\legal\\"),
            Entry::File("runtime\\legal\\java.base\\LICENSE", b"license"),
        ],
    );
    validate_archive(&archive).expect("valid");
    let entries = stage_archive(&archive, &temp.path().join("extracted")).expect("stage");
    assert_eq!(entries.len(), 1);
    let dest = temp.path().join("installation");
    install_staged(&dest, &entries, &CopyReplacer).expect("install");
    assert_eq!(
        std::fs::read_to_string(dest.join("runtime/legal/java.base/LICENSE")).expect("read"),
        "license"
    );
}

#[test]
fn duplicate_entry_names_keep_the_last_content() {
    // The zip reader indexes entries by name: a repeated name is one entry with the last data.
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("dup.zip");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("a.txt", options).expect("first");
    std::io::Write::write_all(&mut writer, b"first").expect("write");
    // The writer refuses duplicates; a renamed raw copy produces one.
    writer.start_file("b.txt", options).expect("second");
    std::io::Write::write_all(&mut writer, b"last").expect("write");
    let mut bytes = writer.finish().expect("finish").into_inner();
    let positions: Vec<usize> = (0..bytes.len() - 4)
        .filter(|&i| &bytes[i..i + 5] == b"b.txt")
        .collect();
    for at in positions {
        bytes[at] = b'a';
    }
    std::fs::write(&archive, bytes).expect("write");

    validate_archive(&archive).expect("valid");
    let entries = stage_archive(&archive, &temp.path().join("staged")).expect("stage");
    assert_eq!(entries.len(), 1);
    assert_eq!(std::fs::read(&entries[0].source).expect("read"), b"last");
}

#[cfg(unix)]
#[test]
fn unix_permissions_and_links() {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("modes.zip");
    write_zip(
        &archive,
        &[
            Entry::FileWithMode("bin/tool", b"#!/bin/sh\n", 0o755),
            Entry::FileWithMode("bin/setuid", b"x", 0o4755),
            Entry::FileWithMode("data.txt", b"x", 0o600),
        ],
    );
    let entries = stage_archive(&archive, &temp.path().join("staged")).expect("stage");
    let modes: Vec<Option<u32>> = entries.iter().map(|e| e.unix_mode).collect();
    assert_eq!(modes, [Some(0o755), Some(0o755), Some(0o600)]);
    let staged_mode = |e: &StagedEntry| {
        std::fs::metadata(&e.source)
            .expect("meta")
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(staged_mode(&entries[0]), 0o755);
    assert_eq!(staged_mode(&entries[1]), 0o755);

    let dest = temp.path().join("installation");
    install_staged(&dest, &entries, &CopyReplacer).expect("install");
    let mode = |p: &str| {
        std::fs::metadata(dest.join(p))
            .expect("meta")
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(mode("bin/tool"), 0o755);
    assert_eq!(mode("bin/setuid"), 0o755);
    assert_eq!(mode("data.txt"), 0o600);

    let linked = temp.path().join("link.zip");
    write_zip(
        &linked,
        &[
            Entry::File("a.txt", b"x"),
            Entry::Symlink("evil", "/etc/passwd"),
        ],
    );
    assert!(matches!(
        validate_archive(&linked),
        Err(UpdateError::UnsafeEntry(_))
    ));
    assert!(matches!(
        stage_archive(&linked, &temp.path().join("linked")),
        Err(UpdateError::UnsafeEntry(_))
    ));
}

/// Installs a real release archive (`A3S_UPDATER_RELEASE_ARCHIVE`).
#[test]
#[ignore = "needs release artifact"]
fn installs_a_built_release_archive() {
    let archive = std::env::var_os("A3S_UPDATER_RELEASE_ARCHIVE")
        .expect("set A3S_UPDATER_RELEASE_ARCHIVE to a release archive");
    let archive = Path::new(&archive);
    let temp = tempfile::tempdir().expect("tempdir");
    let dest = temp.path().join("installation");
    let report = install_archive(archive, &dest, &CopyReplacer).expect("install");
    assert!(!report.installed.is_empty());
    for relative in &report.installed {
        assert!(dest.join(relative).is_file(), "{relative}");
    }
    let binary = if cfg!(windows) {
        "arma3sync.exe"
    } else {
        "arma3sync"
    };
    assert!(
        report
            .installed
            .iter()
            .any(|p| p.rsplit('/').next() == Some(binary)),
        "no {binary} in the archive"
    );
}
