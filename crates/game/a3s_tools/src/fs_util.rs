//! File helpers shared by the tools: Java's `FileAccessMethods.copyDirectory` and a writability
//! probe standing in for `Files.isWritable`.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Copies `source` to `target`, recursively when `source` is a directory.
///
/// A directory target is created with a single-level `mkdir` whose failure is ignored; the
/// children (in file-name order) are then copied into it. Anything that is not a directory,
/// including a missing source, is copied as a file, so a missing source is an error. Existing
/// files are overwritten. The first error aborts and whatever was copied stays.
// Mirrors dao/FileAccessMethods.java:26-45
pub(crate) fn copy_directory(source: &Path, target: &Path) -> io::Result<()> {
    if source.is_dir() {
        if !target.exists() {
            // Java ignores the result of `mkdir`; a failure surfaces on the first child.
            let _ignored: io::Result<()> = fs::create_dir(target);
        }
        // An unreadable directory has no children in Java (`list()` returns null).
        let Ok(entries) = fs::read_dir(source) else {
            return Ok(());
        };
        let mut names: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        names.sort();
        for name in names {
            copy_directory(&source.join(&name), &target.join(&name))?;
        }
        Ok(())
    } else {
        copy_file(source, target)
    }
}

/// Copies one file's bytes to `target`, truncating an existing target. The error message names
/// the path that failed.
pub(crate) fn copy_file(source: &Path, target: &Path) -> io::Result<()> {
    copy_file_bytes(source, target).map_err(|(path, err)| with_path(path, &err))
}

/// Copies one file's bytes to `target`, truncating an existing target. The error comes with the
/// path that failed: `source` when it cannot be read, else `target`.
// Mirrors dao/FileAccessMethods.java:64-72
pub(crate) fn copy_file_bytes<'a>(
    source: &'a Path,
    target: &'a Path,
) -> Result<(), (&'a Path, io::Error)> {
    let mut input = File::open(source).map_err(|err| (source, err))?;
    let mut output = File::create(target).map_err(|err| (target, err))?;
    io::copy(&mut input, &mut output).map_err(|err| (target, err))?;
    Ok(())
}

/// Whether a file can be created in `dir`.
///
/// Probes by creating and deleting a uniquely named file, which honours ACLs, read-only mounts
/// and the effective user alike. A path that is not a directory is not writable.
pub(crate) fn is_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let probe: PathBuf = dir.join(format!(
        ".a3s_write_probe_{}_{nanos}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            // The probe is ours and empty; a failed removal leaves a harmless dot file.
            let _ignored: io::Result<()> = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `err` with `path` in front of its message, keeping its kind.
pub(crate) fn with_path(path: &Path, err: &io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", path.display()))
}
