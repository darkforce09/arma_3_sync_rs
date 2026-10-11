//! Copying staged files into the installation and tracking them in `.a3s-updater-files`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;

use super::{StagedEntry, normalize_entry_name, set_permissions, stage_archive, validate_archive};
use crate::UpdateError;

/// State file listing the files the updater installed, one `/`-separated path per line.
// Mirrors a3sUpdater/dao/FtpDAO.java:30
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

/// Replaces one installed file with a staged one.
pub trait FileReplacer {
    /// Makes `target` a copy of `source` with permission bits `unix_mode` (Unix only; `None`
    /// means `0o644`). The parent directory of `target` exists.
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
            .prefix(".a3s-update-")
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

/// Whether `relative` (an installation-relative path, `\` or `/` separated) is user data.
// Mirrors a3sUpdater/dao/FtpDAO.java:396-412
pub fn is_user_managed(relative: &str) -> bool {
    let portable = relative.replace('\\', "/");
    USER_MANAGED_FILES
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
/// 2. Copies every staged entry with `replacer`, except user data and the state file itself.
/// 3. Deletes previously managed files that this update no longer contains (user data is
///    kept; a non-empty directory fails the installation).
/// 4. Writes the new state file atomically.
///
/// A failure in steps 2 or 3 leaves a partially updated installation.
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
    let mut current = ManagedSet::default();

    for entry in entries {
        if !entry.source.is_file() {
            return Err(UpdateError::MissingSource(entry.source.clone()));
        }
        let relative = normalize_entry_name(&entry.relative)?;
        if relative.is_empty() {
            return Err(UpdateError::UnsafeEntry(entry.relative.clone()));
        }
        if is_user_managed(&relative) {
            report.skipped_user_files.push(relative);
            continue;
        }
        if relative.eq_ignore_ascii_case(MANAGED_FILES_NAME) {
            continue;
        }
        let target = dest.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                UpdateError::io(format!("Could not create {}", parent.display()), e)
            })?;
        }
        replacer
            .replace(&entry.source, &target, entry.unix_mode)
            .map_err(|e| UpdateError::io(format!("Could not install {relative}"), e))?;
        if current.insert(&relative) {
            report.installed.push(relative);
        }
    }

    for file in previous {
        if current.contains(&file) || is_user_managed(&file) {
            continue;
        }
        if remove_managed(&dest.join(&file))? {
            report.removed.push(file);
        }
    }

    write_managed_files(dest, &current.paths)?;
    Ok(report)
}

/// Installs every file below `source_dir` into `dest` (see [`install_staged`]).
// Mirrors a3sUpdater/dao/FtpDAO.java:262-285
pub fn install_tree(
    source_dir: &Path,
    dest: &Path,
    replacer: &dyn FileReplacer,
) -> Result<InstallReport, UpdateError> {
    if !source_dir.exists() {
        return Err(UpdateError::MissingSource(source_dir.to_path_buf()));
    }
    let mut entries = Vec::new();
    collect_tree(source_dir, "", &mut entries)?;
    install_staged(dest, &entries, replacer)
}

fn collect_tree(dir: &Path, prefix: &str, out: &mut Vec<StagedEntry>) -> Result<(), UpdateError> {
    let read_error = |e| UpdateError::io(format!("Could not list {}", dir.display()), e);
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .map_err(read_error)?
        .collect::<Result<_, _>>()
        .map_err(read_error)?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let name = child.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            collect_tree(&path, &relative, out)?;
        } else {
            out.push(StagedEntry {
                unix_mode: file_permissions(&path),
                source: path,
                relative,
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn file_permissions(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_permissions(_path: &Path) -> Option<u32> {
    None
}

/// Validates, stages and installs an update archive into `dest`.
///
/// Extraction goes to a temporary `arma3sync-update-extracted-*` directory, which is always
/// removed. Failures are wrapped in [`UpdateError::Stage`] (`archive validation`,
/// `archive extraction`, `installation copy`).
// Mirrors a3sUpdater/dao/HttpDAO.java:98-124
pub fn install_archive(
    archive: &Path,
    dest: &Path,
    replacer: &dyn FileReplacer,
) -> Result<InstallReport, UpdateError> {
    if !archive.is_file() {
        return Err(UpdateError::ArchiveNotFound(archive.to_path_buf()));
    }
    validate_archive(archive).map_err(|e| e.in_stage("archive validation"))?;
    let extracted = tempfile::Builder::new()
        .prefix("arma3sync-update-extracted-")
        .tempdir()
        .map_err(|e| {
            UpdateError::io("Could not create the extraction directory", e)
                .in_stage("archive extraction")
        })?;
    let entries =
        stage_archive(archive, extracted.path()).map_err(|e| e.in_stage("archive extraction"))?;
    install_staged(dest, &entries, replacer).map_err(|e| e.in_stage("installation copy"))
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
