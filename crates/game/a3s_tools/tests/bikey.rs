//! Bikey extraction: validation order, matching, walk, flat copy, cancellation.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use a3s_tools::CancelToken;
use a3s_tools::bikey::{
    BikeyOutcome, BikeyValidationError, copy_bikeys, extract, find_bikeys, is_bikey, validate,
};

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// Makes `dir` read-only; `false` when the process can still write into it (running as root).
#[cfg(unix)]
fn make_read_only(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = dir.join("probe");
    if fs::write(&probe, "").is_ok() {
        fs::remove_file(probe).unwrap();
        return false;
    }
    true
}

#[test]
fn validation_reports_the_first_failure_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().to_str().unwrap();
    let missing = temp.path().join("missing");
    let missing = missing.to_str().unwrap();

    assert_eq!(validate("", ""), Err(BikeyValidationError::SourceEmpty));
    assert_eq!(
        validate(missing, ""),
        Err(BikeyValidationError::SourceMissing)
    );
    assert_eq!(
        validate(existing, ""),
        Err(BikeyValidationError::TargetEmpty)
    );
    assert_eq!(
        validate(existing, missing),
        Err(BikeyValidationError::TargetMissing)
    );
    assert_eq!(validate(existing, existing), Ok(()));

    let file = temp.path().join("file.txt");
    write(&file, "x");
    assert_eq!(
        validate(existing, file.to_str().unwrap()),
        Err(BikeyValidationError::TargetNotWritable)
    );
}

#[cfg(unix)]
#[test]
fn validation_rejects_a_read_only_target() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("keys");
    fs::create_dir(&target).unwrap();
    if !make_read_only(&target) {
        return;
    }
    assert_eq!(
        validate(temp.path().to_str().unwrap(), target.to_str().unwrap()),
        Err(BikeyValidationError::TargetNotWritable)
    );
}

#[test]
fn bikey_names_match_on_the_last_extension_ignoring_case() {
    for name in ["a.bikey", "A.BiKeY", ".bikey"] {
        assert!(is_bikey(OsStr::new(name)), "{name}");
    }
    for name in ["a.bikey.bak", "bikey", "a_bikey"] {
        assert!(!is_bikey(OsStr::new(name)), "{name}");
    }
}

#[test]
fn extraction_copies_every_key_flat_and_counts_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("mods");
    let target = temp.path().join("keys");
    fs::create_dir(&target).unwrap();
    write(&source.join("@a/keys/x.bikey"), "from a");
    write(&source.join("@b/Keys/Y.BIKEY"), "from b");
    write(&source.join("@b/addons/z.pbo"), "pbo");
    write(&source.join("@c/keys/x.bikey"), "from c");

    let outcome = extract(&source, &target, &CancelToken::new()).unwrap();

    assert_eq!(outcome, BikeyOutcome::Copied { count: 3 });
    let mut names: Vec<_> = fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["Y.BIKEY", "x.bikey"]);
    assert_eq!(text(&target.join("x.bikey")), "from c");
}

#[test]
fn nothing_found_leaves_the_target_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("mods");
    let target = temp.path().join("keys");
    fs::create_dir(&target).unwrap();
    write(&source.join("@a/addons/a.pbo"), "pbo");

    let outcome = extract(&source, &target, &CancelToken::new()).unwrap();

    assert_eq!(outcome, BikeyOutcome::NoneFound);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn a_single_key_file_is_a_valid_source() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("server.bikey");
    let target = temp.path().join("keys");
    fs::create_dir(&target).unwrap();
    write(&key, "key");

    let outcome = extract(&key, &target, &CancelToken::new()).unwrap();

    assert_eq!(outcome, BikeyOutcome::Copied { count: 1 });
    assert_eq!(text(&target.join("server.bikey")), "key");
}

#[test]
fn cancellation_stops_the_walk_and_the_copies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("mods");
    let target = temp.path().join("keys");
    fs::create_dir(&target).unwrap();
    write(&source.join("@a/keys/a.bikey"), "a");
    write(&source.join("@b/keys/b.bikey"), "b");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert_eq!(
        extract(&source, &target, &cancelled).unwrap(),
        BikeyOutcome::Cancelled {
            found: 0,
            copied: 0
        }
    );
    assert_eq!(find_bikeys(&source, &cancelled), None);

    let found = find_bikeys(&source, &CancelToken::new()).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(
        copy_bikeys(&found, &target, &cancelled).unwrap(),
        BikeyOutcome::Cancelled {
            found: 2,
            copied: 0
        }
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn a_key_already_in_the_target_keeps_its_content() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("server");
    let target = source.join("keys");
    write(&target.join("k.bikey"), "original");

    let outcome = extract(&source, &target, &CancelToken::new()).unwrap();

    assert_eq!(outcome, BikeyOutcome::Copied { count: 1 });
    assert_eq!(text(&target.join("k.bikey")), "original");
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_cycle_ends_the_walk() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("mods");
    write(&source.join("a/k.bikey"), "k");
    std::os::unix::fs::symlink("..", source.join("a/loop")).unwrap();

    let found = find_bikeys(&source, &CancelToken::new()).unwrap();

    assert_eq!(found, [source.join("a/k.bikey")]);
}

#[test]
fn the_first_copy_error_aborts_and_keeps_earlier_copies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("mods");
    let target = temp.path().join("keys");
    write(&source.join("@a/a.bikey"), "a");
    write(&source.join("@b/x.bikey"), "x");
    write(&source.join("@c/z.bikey"), "z");
    fs::create_dir_all(target.join("x.bikey")).unwrap();

    let error = extract(&source, &target, &CancelToken::new()).unwrap_err();

    assert_eq!(error.path, target.join("x.bikey"));
    assert_eq!(text(&target.join("a.bikey")), "a");
    assert!(!target.join("z.bikey").exists());
}
