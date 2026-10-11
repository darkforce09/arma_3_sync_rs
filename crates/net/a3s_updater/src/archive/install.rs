//! Copying staged files into the installation and tracking them in `.a3s-updater-files`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;

use a3s_foundation::paths::{PORTABLE_DATA_DIR, PORTABLE_MARKER};

use super::{PathSet, StagedEntry, normalize_entry_name, set_permissions};
use crate::UpdateError;

/// State file listing the files the updater installed, one `/`-separated path per line.
// Mirrors a3sUpdater/dao/FtpDAO.java:29
pub const MANAGED_FILES_NAME: &str = ".a3s-updater-files";

/// Line separator of the state file (Java `Files.write` uses the platform separator).
const LINE_SEPARATOR: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// User data an update never writes or deletes (ASCII case-insensitive).
// Mirrors a3sUpdater/dao/FtpDAO.java:396-412
const USER_MANAGED_FILES: [&str; 11] = [
    "a3s.cfg",
    "a3s.prefs",
    "a3s.xml",
    "updater.toml",
    "resources/configuration/updater.toml",
    "resources/configuration/a3s.cfg",
    "resources/configuration/a3s.prefs",
    "resources/configuration/a3s.xml",
    "profiles",
    "resources/ftp",
    "resources/temp",
];

/// User-data folders whose whole content is never written or deleted (ASCII case-insensitive).
const USER_MANAGED_DIRECTORIES: [&str; 3] = ["profiles/", "resources/ftp/", "resources/temp/"];

/// Whether `portable` is this application's portable-mode marker or data folder (or inside it).
fn is_portable_data(portable: &str) -> bool {
    let top = portable.split('/').next().unwrap_or_default();
    portable.eq_ignore_ascii_case(PORTABLE_MARKER) || top.eq_ignore_ascii_case(PORTABLE_DATA_DIR)
}

/// Replaces one installed file with a staged one.
pub trait FileReplacer {
    /// Makes `target` a copy of `source` with permission bits `unix_mode` (Unix only; applied as
    /// `(mode & 0o755) | 0o600`, `0o644` when `None`). The parent directory of `target` exists.
    fn replace(&self, source: &Path, target: &Path, unix_mode: Option<u32>) -> io::Result<()>;
}

/// Copies to a temporary sibling of the target and renames it over the target, so a running
/// executable is replaced rather than rewritten in place. Modification times are not kept.
#[derive(Clone, Copy, Debug, Default)]
pub struct CopyReplacer;

impl FileReplacer for CopyReplacer {
    fn replace(&self, source: &Path, target: &Path, unix_mode: Option<u32>) -> io::Result<()> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::Builder::new()
            .prefix(".a3s-new-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        io::copy(&mut File::open(source)?, temp.as_file_mut())?;
        temp.as_file_mut().flush()?;
        set_permissions(temp.as_file(), unix_mode)?;
        temp.persist(target).map_err(|e| e.error)?;
        Ok(())
    }
}

/// What an installation changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
    /// Installed paths, in archive order.
    pub installed: Vec<String>,

    /// Previously managed paths that were deleted.
    pub removed: Vec<String>,

    /// Staged paths left alone because they are user data.
    pub skipped_user_files: Vec<String>,
}

/// Whether `relative` (an installation-relative path, `\` or `/` separated) is user data: the
/// Java client's files and folders, and this application's portable-mode marker and data folder.
// Mirrors a3sUpdater/dao/FtpDAO.java:396-412
pub fn is_user_managed(relative: &str) -> bool {
    let portable = relative.replace('\\', "/");
    is_portable_data(&portable)
        || USER_MANAGED_FILES
            .iter()
            .any(|name| portable.eq_ignore_ascii_case(name))
        || USER_MANAGED_DIRECTORIES.iter().any(|prefix| {
            portable
                .get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
}

/// Installs staged files into `dest`.
///
/// 1. Reads the previous `.a3s-updater-files`; any invalid line fails before a file is touched.
/// 2. Checks every entry before writing: the staged source exists, the path is safe, no two
///    entries collide, no target is an existing directory and no parent is an existing file.
///    A parent that is a file the previous update installed and this one drops is deleted
///    first; any other conflict fails before the installation is touched.
/// 3. Copies every staged entry with `replacer`, except user data and the state file itself.
/// 4. Deletes previously managed files that this update no longer contains, except user data
///    and files that are the same file as an installed target (case-insensitive file systems).
///    A non-empty directory fails the installation.
/// 5. Writes the new state file atomically.
///
/// A failure in steps 3 or 4 leaves a partially updated installation. The staged files are
/// reopened by path: they must sit in a directory no less-privileged user can write (see
/// [`super::stage_archive`]).
// Mirrors a3sUpdater/dao/FtpDAO.java:287-320
pub fn install_staged(
    dest: &Path,
    entries: &[StagedEntry],
    replacer: &dyn FileReplacer,
) -> Result<InstallReport, UpdateError> {
    std::fs::create_dir_all(dest)
        .map_err(|e| UpdateError::io(format!("Could not create {}", dest.display()), e))?;
    let previous = read_managed_files(dest)?;
    let mut report = InstallReport::default();
    let plan = plan_install(entries, &mut report)?;
    let mut current = ManagedSet::default();
    for (relative, _) in &plan {
        current.insert(relative);
    }
    let stale: Vec<&String> = previous
        .iter()
        .filter(|file| !current.contains(file) && !is_user_managed(file))
        .collect();

    for parent in blocking_parents(dest, &plan, &stale)? {
        if remove_managed(&dest.join(&parent))? {
            report.removed.push(parent);
        }
    }

    let mut installed_files = HashSet::new();
    for (relative, entry) in &plan {
        let target = dest.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                UpdateError::io(format!("Could not create {}", parent.display()), e)
            })?;
        }
        replacer
            .replace(&entry.source, &target, entry.unix_mode)
            .map_err(|e| UpdateError::io(format!("Could not install {relative}"), e))?;
        installed_files.extend(file_identity(&target));
        report.installed.push(relative.clone());
    }

    for file in stale {
        if report.removed.contains(file) {
            continue;
        }
        let target = dest.join(file);
        // On a case-insensitive file system an old spelling is the file just installed.
        if file_identity(&target).is_some_and(|id| installed_files.contains(&id)) {
            continue;
        }
        if remove_managed(&target)? {
            report.removed.push(file.clone());
        }
    }

    write_managed_files(dest, &current.paths)?;
    Ok(report)
}

/// The entries to install, with their normalized paths; user data is reported as skipped.
fn plan_install<'a>(
    entries: &'a [StagedEntry],
    report: &mut InstallReport,
) -> Result<Vec<(String, &'a StagedEntry)>, UpdateError> {
    let mut paths = PathSet::default();
    let mut plan = Vec::new();
    for entry in entries {
        if !entry.source.is_file() {
            return Err(UpdateError::MissingSource(entry.source.clone()));
        }
        let relative = normalize_entry_name(&entry.relative)?;
        if relative.is_empty() {
            return Err(UpdateError::UnsafeEntry(entry.relative.clone()));
        }
        paths.add_file(&relative)?;
        if is_user_managed(&relative) {
            report.skipped_user_files.push(relative);
        } else if !relative.eq_ignore_ascii_case(MANAGED_FILES_NAME) {
            plan.push((relative, entry));
        }
    }
    Ok(plan)
}

/// Existing files in `dest` that are parents of a planned path. Each must be a stale managed
/// file, which is returned for deletion; any other file, or a planned target that is an
/// existing directory, is a conflict.
fn blocking_parents(
    dest: &Path,
    plan: &[(String, &StagedEntry)],
    stale: &[&String],
) -> Result<Vec<String>, UpdateError> {
    let mut blocking = Vec::new();
    for (relative, _) in plan {
        if std::fs::metadata(dest.join(relative)).is_ok_and(|meta| meta.is_dir()) {
            return Err(UpdateError::PathConflict(relative.clone()));
        }
        for (index, _) in relative.match_indices('/') {
            let parent = &relative[..index];
            let path = dest.join(parent);
            let is_file = match std::fs::metadata(&path) {
                Ok(meta) => !meta.is_dir(),
                // A dangling link or an unreadable entry cannot be a directory either.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    std::fs::symlink_metadata(&path).is_ok()
                }
                Err(_) => true,
            };
            if !is_file || blocking.iter().any(|b: &String| b == parent) {
                continue;
            }
            let managed = stale
                .iter()
                .find(|file| same_managed_file(dest, file, parent))
                .ok_or_else(|| UpdateError::PathConflict(relative.clone()))?;
            blocking.push((*managed).clone());
        }
    }
    Ok(blocking)
}

/// Whether the managed path `file` names the existing file at `relative`.
fn same_managed_file(dest: &Path, file: &str, relative: &str) -> bool {
    if file == relative {
        return true;
    }
    if cfg!(windows) {
        return file.eq_ignore_ascii_case(relative);
    }
    let id = file_identity(&dest.join(relative));
    id.is_some() && id == file_identity(&dest.join(file))
}

/// Device and inode of a path (not following a final link); `None` where unavailable.
#[cfg(unix)]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn file_identity(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// Insertion-ordered set of managed paths; ASCII case-insensitive on Windows.
#[derive(Default)]
struct ManagedSet {
    paths: Vec<String>,
    keys: HashSet<String>,
}

impl ManagedSet {
    fn key(path: &str) -> String {
        if cfg!(windows) {
            path.to_ascii_lowercase()
        } else {
            path.to_owned()
        }
    }

    fn insert(&mut self, path: &str) -> bool {
        let inserted = self.keys.insert(Self::key(path));
        if inserted {
            self.paths.push(path.to_owned());
        }
        inserted
    }

    // Mirrors a3sUpdater/dao/FtpDAO.java:373-380
    fn contains(&self, path: &str) -> bool {
        self.keys.contains(&Self::key(path))
    }
}

/// Deletes a previously managed path; returns whether something was deleted. An empty
/// directory is removed, a non-empty one is an error.
fn remove_managed(target: &Path) -> Result<bool, UpdateError> {
    let error = |e| UpdateError::io(format!("Could not delete {}", target.display()), e);
    let meta = match std::fs::symlink_metadata(target) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(error(e)),
    };
    let removed = if meta.is_dir() {
        std::fs::remove_dir(target)
    } else {
        std::fs::remove_file(target)
    };
    match removed {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(error(e)),
    }
}

/// Reads the previous state file: missing = empty; blank lines skipped; `\r` and `\` accepted
/// (files written by Java on Windows); every path must stay inside `dest`.
// Mirrors a3sUpdater/dao/FtpDAO.java:340-371
fn read_managed_files(dest: &Path) -> Result<Vec<String>, UpdateError> {
    let path = dest.join(MANAGED_FILES_NAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    if !path.is_file() {
        return Err(UpdateError::StateNotFile(path));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| UpdateError::io(format!("Could not read {}", path.display()), e))?;
    let mut files = ManagedSet::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let relative = match normalize_entry_name(line) {
            Ok(relative) if !relative.is_empty() => relative,
            _ => return Err(UpdateError::UnsafeStatePath(line.to_owned())),
        };
        files.insert(&relative);
    }
    Ok(files.paths)
}

/// Writes the state file through a temporary file renamed over it.
// Mirrors a3sUpdater/dao/FtpDAO.java:382-394
fn write_managed_files(dest: &Path, files: &[String]) -> Result<(), UpdateError> {
    let path = dest.join(MANAGED_FILES_NAME);
    let error = |e| UpdateError::io(format!("Could not write {}", path.display()), e);
    let mut temp = tempfile::Builder::new()
        .prefix(&format!("{MANAGED_FILES_NAME}."))
        .suffix(".tmp")
        .tempfile_in(dest)
        .map_err(error)?;
    let mut text = String::new();
    for file in files {
        text.push_str(file);
        text.push_str(LINE_SEPARATOR);
    }
    temp.write_all(text.as_bytes()).map_err(error)?;
    temp.persist(&path).map_err(|e| error(e.error))?;
    Ok(())
}
