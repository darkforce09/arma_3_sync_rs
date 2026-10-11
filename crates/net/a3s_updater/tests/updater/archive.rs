//! Archive verification and staging: hash, entry-name safety, conflicts, CRC, permissions, links.

use std::path::Path;

use a3s_updater::{
    CopyReplacer, StagedEntry, UpdateError, VerifiedArchive, install_staged, stage_archive,
    validate_archive,
};

use crate::support::{Entry, sha256_hex, write_zip, zip_bytes};

fn validate(archive: &Path) -> Result<VerifiedArchive, UpdateError> {
    validate_archive(archive, None)
}

/// Validates and stages an archive written to a new file in `dir`.
fn check(dir: &Path, entries: &[Entry<'_>]) -> Result<Vec<StagedEntry>, UpdateError> {
    let count = dir.read_dir().expect("list").count();
    let archive = dir.join(format!("{count}.zip"));
    write_zip(&archive, entries);
    let mut verified = validate(&archive)?;
    stage_archive(&mut verified, &dir.join(format!("staged-{count}")))
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
        "dir/file.txt:stream",
        " ",
        "a/..",
        ".",
        "dir./x",
        "x.txt ",
        "CON",
        "bin/aux.txt",
        "lpt1.log",
        "Com9",
        "a\u{1}b",
        "what?.txt",
    ];
    for name in rejected {
        let result = check(temp.path(), &[Entry::File(name, b"data")]);
        assert!(result.is_err(), "accepted {name:?}");
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
        ("console.txt", "console.txt"),
        ("com10", "com10"),
    ] {
        let staged = check(temp.path(), &[Entry::File(name, b"data")]).expect(name);
        assert_eq!(staged[0].relative, relative);
        assert!(staged[0].source.is_file());
    }
}

#[test]
fn colliding_paths_are_rejected_before_staging() {
    let temp = tempfile::tempdir().expect("tempdir");
    let collisions: [&[Entry<'_>]; 5] = [
        &[
            Entry::File("bin/App.exe", b"1"),
            Entry::File("bin/app.exe", b"2"),
        ],
        &[Entry::File("a/b", b"1"), Entry::File("a/./b", b"2")],
        &[Entry::File("x", b"1"), Entry::File("x/y", b"2")],
        &[Entry::File("X/y", b"1"), Entry::File("x", b"2")],
        &[Entry::Directory("data/"), Entry::File("DATA", b"1")],
    ];
    for entries in collisions {
        let err = check(temp.path(), entries).expect_err("collision");
        assert!(matches!(err, UpdateError::PathConflict(_)), "{err:?}");
    }
}

#[test]
fn hash_is_checked_on_the_bytes_that_are_staged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let archive = temp.path().join("update.zip");
    let original = zip_bytes(&[Entry::File("bin/launcher.txt", b"original")]);
    std::fs::write(&archive, &original).expect("write");

    let err = validate_archive(&archive, Some(&"0".repeat(64))).expect_err("wrong hash");
    assert!(matches!(err, UpdateError::Integrity), "{err:?}");

    let hash = sha256_hex(&original).to_ascii_uppercase();
    let mut verified = validate_archive(&archive, Some(&hash)).expect("hash matches");
    assert_eq!(verified.sha256(), sha256_hex(&original));

    // Replacing the file after verification changes nothing: staging reads the verified copy.
    write_zip(&archive, &[Entry::File("bin/launcher.txt", b"swapped")]);
    let staged = stage_archive(&mut verified, &temp.path().join("staged")).expect("stage");
    assert_eq!(std::fs::read(&staged[0].source).expect("read"), b"original");

    let missing = validate(&temp.path().join("missing.zip")).expect_err("missing");
    assert!(
        matches!(missing, UpdateError::ArchiveNotFound(_)),
        "{missing:?}"
    );
}

#[test]
fn normal_entry_is_staged_and_installed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let entries = check(temp.path(), &[Entry::File("bin/launcher.txt", b"ok")]).expect("stage");
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
        validate(&truncated),
        Err(UpdateError::ArchiveInvalid(_))
    ));

    let empty = temp.path().join("empty.zip");
    write_zip(&empty, &[Entry::Directory("only/")]);
    assert!(matches!(validate(&empty), Err(UpdateError::ArchiveEmpty)));

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
        validate(&corrupt),
        Err(UpdateError::ArchiveInvalid(_))
    ));
}

#[test]
fn every_entry_is_validated_before_anything_is_staged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let err = check(
        temp.path(),
        &[
            Entry::File("bin/first.txt", b"1"),
            Entry::File("../escape.txt", b"2"),
        ],
    )
    .expect_err("unsafe");
    assert!(matches!(err, UpdateError::UnsafeEntry(_)), "{err:?}");
    assert!(!temp.path().join("staged-0").exists());
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn runtime_tree_is_copied_from_the_staged_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let entries = check(
        temp.path(),
        &[
            Entry::File("runtime/legal/java.base/LICENSE", b"license"),
            Entry::File("runtime/bin/java.exe", &[1, 2, 3]),
        ],
    )
    .expect("stage");
    // Staged files are flat.
    let staged_dir = temp.path().join("staged-0");
    assert!(
        entries
            .iter()
            .all(|e| e.source.parent() == Some(&*staged_dir))
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
    let entries = check(
        temp.path(),
        &[
            Entry::Directory("runtime\\legal\\"),
            Entry::File("runtime\\legal\\java.base\\LICENSE", b"license"),
        ],
    )
    .expect("stage");
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
    // The writer refuses duplicates; renaming the second entry's bytes produces one.
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

    let mut verified = validate(&archive).expect("valid");
    let entries = stage_archive(&mut verified, &temp.path().join("staged")).expect("stage");
    assert_eq!(entries.len(), 1);
    assert_eq!(std::fs::read(&entries[0].source).expect("read"), b"last");
}

#[cfg(unix)]
#[test]
fn unix_permissions_and_links() {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().expect("tempdir");
    let entries = check(
        temp.path(),
        &[
            Entry::FileWithMode("bin/tool", b"#!/bin/sh\n", 0o755),
            Entry::FileWithMode("bin/setuid", b"x", 0o4755),
            Entry::FileWithMode("data.txt", b"x", 0o600),
            Entry::FileWithMode("open.txt", b"x", 0o777),
            Entry::FileWithMode("none.txt", b"x", 0o000),
            Entry::FileWithMode("write-only.txt", b"x", 0o222),
        ],
    )
    .expect("stage");
    let expected = [0o755, 0o755, 0o600, 0o755, 0o600, 0o600];
    let modes: Vec<Option<u32>> = entries.iter().map(|e| e.unix_mode).collect();
    assert_eq!(modes, expected.map(Some));
    let mode_of =
        |path: &Path| std::fs::metadata(path).expect("meta").permissions().mode() & 0o7777;
    for (entry, mode) in entries.iter().zip(expected) {
        assert_eq!(mode_of(&entry.source), mode, "{}", entry.relative);
    }

    let dest = temp.path().join("installation");
    install_staged(&dest, &entries, &CopyReplacer).expect("install");
    for (entry, mode) in entries.iter().zip(expected) {
        assert_eq!(
            mode_of(&dest.join(&entry.relative)),
            mode,
            "{}",
            entry.relative
        );
    }

    let err = check(
        temp.path(),
        &[
            Entry::File("a.txt", b"x"),
            Entry::Symlink("evil", "/etc/passwd"),
        ],
    )
    .expect_err("symlink");
    assert!(matches!(err, UpdateError::UnsafeEntry(_)), "{err:?}");
}
