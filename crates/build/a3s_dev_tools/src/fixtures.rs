//! Golden fixture regeneration: generate with the Java harness, install into `tests/fixtures`,
//! and compare trees.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::java_harness::{Harness, HarnessError, Part};

/// The entries of `tests/fixtures` that the harness generates. [`install`] replaces exactly
/// these and never touches anything else (`java_repos/`, `README.md`, …).
pub const GENERATED_ENTRIES: [&str; 6] = [
    "java_built",
    "java_descriptors.txt",
    "java_harness_manifest.txt",
    "java_local",
    "java_presets",
    "java_roundtrip",
];

/// One difference between two trees, relative to their roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeDiff {
    /// In `expected` only.
    Missing(PathBuf),

    /// In `actual` only.
    Extra(PathBuf),

    /// In both, with different contents.
    Differs(PathBuf),
}

impl TreeDiff {
    /// The relative path of the difference.
    pub fn path(&self) -> &Path {
        match self {
            Self::Missing(path) | Self::Extra(path) | Self::Differs(path) => path,
        }
    }
}

impl std::fmt::Display for TreeDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "only in expected: {}", path.display()),
            Self::Extra(path) => write!(f, "only in actual:   {}", path.display()),
            Self::Differs(path) => write!(f, "differs:          {}", path.display()),
        }
    }
}

/// Builds the harness, then generates `parts` into `out` (empty or missing).
pub fn regenerate(harness: &Harness, out: &Path, parts: &[Part]) -> Result<(), HarnessError> {
    harness.build()?;
    harness.generate(out, parts)
}

/// Replaces the [`GENERATED_ENTRIES`] of `fixtures_dir` with those of `generated`.
///
/// Every entry must exist in `generated`; this is checked before anything is touched. All entries
/// are first moved into a staging directory inside `fixtures_dir` (renamed, or copied and removed
/// across file systems), then swapped in with renames.
pub fn install(generated: &Path, fixtures_dir: &Path) -> io::Result<()> {
    for entry in GENERATED_ENTRIES {
        let source = generated.join(entry);
        if fs::symlink_metadata(&source).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("generated fixture entry {} is missing", source.display()),
            ));
        }
    }
    fs::create_dir_all(fixtures_dir)?;

    // Stage every entry next to its target first (the slow, failure-prone part), so a failure
    // leaves the old fixtures untouched; then swap with renames on the same file system.
    let staging = tempfile::Builder::new()
        .prefix(".a3s-fixtures-staging")
        .tempdir_in(fixtures_dir)?;
    for entry in GENERATED_ENTRIES {
        move_entry(&generated.join(entry), &staging.path().join(entry))?;
    }
    let old = staging.path().join(".old");
    fs::create_dir(&old)?;
    for entry in GENERATED_ENTRIES {
        let target = fixtures_dir.join(entry);
        if fs::symlink_metadata(&target).is_ok() {
            fs::rename(&target, old.join(entry))?;
        }
        fs::rename(staging.path().join(entry), &target)?;
    }
    // Dropping `staging` deletes the replaced entries.
    Ok(())
}

/// Renames `source` to `target`, or copies and removes it across file systems.
fn move_entry(source: &Path, target: &Path) -> io::Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            copy_entry(source, target)?;
            remove_entry(source)
        }
        Err(err) => Err(err),
    }
}

fn remove_entry(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn copy_entry(source: &Path, target: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(source)?;
    if meta.is_dir() {
        fs::create_dir_all(target)?;
        for child in fs::read_dir(source)? {
            let child = child?;
            copy_entry(&child.path(), &target.join(child.file_name()))?;
        }
        Ok(())
    } else if meta.is_file() {
        fs::copy(source, target).map(drop)
    } else {
        Err(symlink_error(source))
    }
}

fn symlink_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} is not a regular file or directory", path.display()),
    )
}

/// Compares the regular files of two directory trees by full contents.
///
/// A missing root counts as an empty tree. Directories are only traversed (empty directories are
/// ignored, as git does). Symbolic links are an error. The result is sorted by path.
pub fn compare_trees(expected: &Path, actual: &Path) -> io::Result<Vec<TreeDiff>> {
    let expected_files = list_files(expected)?;
    let actual_files = list_files(actual)?;
    let mut diffs = Vec::new();
    for path in expected_files.union(&actual_files) {
        match (expected_files.contains(path), actual_files.contains(path)) {
            (true, false) => diffs.push(TreeDiff::Missing(path.clone())),
            (false, true) => diffs.push(TreeDiff::Extra(path.clone())),
            _ => {
                if fs::read(expected.join(path))? != fs::read(actual.join(path))? {
                    diffs.push(TreeDiff::Differs(path.clone()));
                }
            }
        }
    }
    Ok(diffs)
}

/// Compares the [`GENERATED_ENTRIES`] of two fixture directories; paths are relative to them.
pub fn compare_generated(expected_dir: &Path, actual_dir: &Path) -> io::Result<Vec<TreeDiff>> {
    let mut diffs = Vec::new();
    for entry in GENERATED_ENTRIES {
        let expected = expected_dir.join(entry);
        let actual = actual_dir.join(entry);
        let is_dir = |path: &Path| fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
        if is_dir(&expected) || is_dir(&actual) {
            for diff in compare_trees(&expected, &actual)? {
                diffs.push(match diff {
                    TreeDiff::Missing(path) => TreeDiff::Missing(Path::new(entry).join(path)),
                    TreeDiff::Extra(path) => TreeDiff::Extra(Path::new(entry).join(path)),
                    TreeDiff::Differs(path) => TreeDiff::Differs(Path::new(entry).join(path)),
                });
            }
            continue;
        }
        match (read_optional(&expected)?, read_optional(&actual)?) {
            (Some(_), None) => diffs.push(TreeDiff::Missing(PathBuf::from(entry))),
            (None, Some(_)) => diffs.push(TreeDiff::Extra(PathBuf::from(entry))),
            (Some(a), Some(b)) if a != b => diffs.push(TreeDiff::Differs(PathBuf::from(entry))),
            _ => {}
        }
    }
    diffs.sort_by(|a, b| a.path().cmp(b.path()));
    Ok(diffs)
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => fs::read(path).map(Some),
        Ok(_) => Err(symlink_error(path)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Relative paths of the regular files below `root` (empty when `root` is missing).
fn list_files(root: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    match fs::symlink_metadata(root) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(symlink_error(root)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(files),
        Err(err) => return Err(err),
    }
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        for child in fs::read_dir(root.join(&dir))? {
            let child = child?;
            let rel = dir.join(child.file_name());
            let file_type = child.file_type()?;
            if file_type.is_dir() {
                stack.push(rel);
            } else if file_type.is_file() {
                files.insert(rel);
            } else {
                return Err(symlink_error(&root.join(rel)));
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{GENERATED_ENTRIES, TreeDiff, compare_generated, compare_trees, install};

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn a1_compare_trees_reports_sorted_differences() {
        let tmp = tempfile::tempdir().unwrap();
        let expected = tmp.path().join("expected");
        let actual = tmp.path().join("actual");
        for root in [&expected, &actual] {
            write(&root.join("same.txt"), "same");
            write(&root.join("dir/nested/changed.bin"), "aaaa");
        }
        fs::create_dir_all(expected.join("empty_dir_is_ignored")).unwrap();
        assert_eq!(compare_trees(&expected, &actual).unwrap(), Vec::new());

        write(&actual.join("dir/nested/changed.bin"), "aaab");
        write(&expected.join("b_only_expected.txt"), "x");
        write(&actual.join("a_only_actual.txt"), "y");
        assert_eq!(
            compare_trees(&expected, &actual).unwrap(),
            vec![
                TreeDiff::Extra(PathBuf::from("a_only_actual.txt")),
                TreeDiff::Missing(PathBuf::from("b_only_expected.txt")),
                TreeDiff::Differs(PathBuf::from("dir/nested/changed.bin")),
            ]
        );
    }

    #[test]
    fn a1_compare_trees_treats_missing_root_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("actual/x"), "x");
        assert_eq!(
            compare_trees(&tmp.path().join("nothing"), &tmp.path().join("actual")).unwrap(),
            vec![TreeDiff::Extra(PathBuf::from("x"))]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a1_compare_trees_rejects_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let expected = tmp.path().join("expected");
        write(&expected.join("target.txt"), "t");
        std::os::unix::fs::symlink(expected.join("target.txt"), expected.join("link.txt")).unwrap();
        let err = compare_trees(&expected, &tmp.path().join("actual")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    fn fake_generated(root: &Path, marker: &str) {
        for entry in GENERATED_ENTRIES {
            if entry.ends_with(".txt") {
                write(&root.join(entry), marker);
            } else {
                write(&root.join(entry).join("file.txt"), marker);
            }
        }
    }

    #[test]
    fn a2_install_replaces_only_generated_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let fixtures = tmp.path().join("fixtures");
        fake_generated(&fixtures, "old");
        write(&fixtures.join("java_local/stale_only_in_old.txt"), "stale");
        write(&fixtures.join("java_repos/keep.bin"), "keep");
        write(&fixtures.join("README.md"), "readme");

        let generated = tmp.path().join("generated");
        fake_generated(&generated, "new");
        install(&generated, &fixtures).unwrap();

        for entry in GENERATED_ENTRIES {
            let file = if entry.ends_with(".txt") {
                fixtures.join(entry)
            } else {
                fixtures.join(entry).join("file.txt")
            };
            assert_eq!(fs::read_to_string(file).unwrap(), "new", "{entry}");
        }
        assert!(!fixtures.join("java_local/stale_only_in_old.txt").exists());
        assert_eq!(
            fs::read_to_string(fixtures.join("java_repos/keep.bin")).unwrap(),
            "keep"
        );
        assert_eq!(
            fs::read_to_string(fixtures.join("README.md")).unwrap(),
            "readme"
        );
        assert!(!generated.join("java_local").exists());
    }

    #[test]
    fn a2_install_refuses_incomplete_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let fixtures = tmp.path().join("fixtures");
        fake_generated(&fixtures, "old");
        let generated = tmp.path().join("generated");
        write(&generated.join("java_local/file.txt"), "new");
        assert!(install(&generated, &fixtures).is_err());
        assert_eq!(
            fs::read_to_string(fixtures.join("java_local/file.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn a2_compare_generated_prefixes_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        fake_generated(&a, "same");
        fake_generated(&b, "same");
        write(&a.join("README.md"), "not compared");
        assert_eq!(compare_generated(&a, &b).unwrap(), Vec::new());
        write(&b.join("java_descriptors.txt"), "changed");
        write(&b.join("java_built/extra.txt"), "extra");
        assert_eq!(
            compare_generated(&a, &b).unwrap(),
            vec![
                TreeDiff::Extra(PathBuf::from("java_built/extra.txt")),
                TreeDiff::Differs(PathBuf::from("java_descriptors.txt")),
            ]
        );
    }
}
