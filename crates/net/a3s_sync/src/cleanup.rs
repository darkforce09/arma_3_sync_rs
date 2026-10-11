//! Deleting extra local files without following links, and copying folders.
//!
//! [`delete_extra_files`] removes the plan's [`DeleteTask`]s. Every path is checked again
//! against the destination roots before anything is removed, and the entry itself is never
//! followed: a symbolic link is removed as a link, so its target survives. Java recurses through
//! links (`FileAccessMethods.deleteDirectory`) and can delete outside the destination.

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::{Component, Path, PathBuf};

use a3s_transport::CancellationToken;

use crate::download::{PART_EXTENSION, ZIP_EXTENSION};
use crate::events::{EventSender, Phase, SyncEvent};
use crate::model::{DeleteTask, DownloadTask, DownloadTaskKind, PlanDirectory, PlanNode, PlanTree};
use crate::plan::{is_hidden_path, is_safe_name};

/// The folders below which deletions are allowed: the destination, plus every addon folder that
/// auto-discovery redirected elsewhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeleteRoots {
    roots: Vec<PathBuf>,
}

impl DeleteRoots {
    /// The roots of a planned tree synchronized into `destination`.
    pub fn from_plan(tree: &PlanTree, destination: &Path) -> Self {
        let mut roots = vec![absolute(destination)];
        collect_redirected(&tree.root, &absolute(destination), &mut roots);
        Self { roots }
    }

    /// The roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether `path` lies strictly below one of the roots (compared component-wise, without
    /// touching the disk; a root itself is not contained).
    pub fn contains(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| {
            path.starts_with(root) && path.components().count() > root.components().count()
        })
    }
}

/// Adds the local path of every folder below `directory` whose destination is not the folder
/// containing it in the tree (`expected`).
fn collect_redirected(directory: &PlanDirectory, expected: &Path, roots: &mut Vec<PathBuf>) {
    for child in &directory.children {
        let PlanNode::Directory(d) = child else {
            continue;
        };
        let Some(destination) = &d.destination_path else {
            continue;
        };
        if absolute(destination) != expected {
            roots.push(absolute(&destination.join(&d.name)));
        }
        collect_redirected(d, &absolute(&destination.join(&d.name)), roots);
    }
}

/// Paths a run may still need: for every file task, its `.zip`, `.part` and `.zip.part`.
///
/// A stale `.ebo.zip` is extra for the plan, yet it may be the archive a paused run downloaded
/// and the next run uncompresses.
pub fn protected_paths(tasks: &[DownloadTask]) -> HashSet<PathBuf> {
    let mut protected = HashSet::new();
    for task in tasks {
        if matches!(task.kind, DownloadTaskKind::File { .. }) {
            let local = absolute(&task.local_path());
            for suffix in [
                ZIP_EXTENSION.to_owned(),
                PART_EXTENSION.to_owned(),
                format!("{ZIP_EXTENSION}{PART_EXTENSION}"),
            ] {
                let mut path = local.as_os_str().to_owned();
                path.push(&suffix);
                protected.insert(PathBuf::from(path));
            }
        }
    }
    protected
}

/// What [`delete_extra_files`] did.
#[derive(Debug, Default)]
pub struct CleanupReport {
    /// Entries removed.
    pub deleted: usize,

    /// Entries already missing (e.g. inside a folder deleted earlier) or protected.
    pub skipped: usize,

    /// Entries refused by the safety checks; untouched.
    pub refused: Vec<PathBuf>,

    /// Entries that could not be removed, with the error.
    pub failed: Vec<(PathBuf, io::Error)>,
}

/// What [`delete_no_follow`] removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteKind {
    /// Nothing was there.
    Missing,

    /// A symbolic link (or junction); its target is untouched.
    Link,

    /// A regular file.
    File,

    /// A real folder with its content (links inside are removed, not followed).
    Directory,
}

/// Removes `path` without following it: a link is removed as a link, a folder recursively
/// (`remove_dir_all` does not follow the links it meets either).
pub fn delete_no_follow(path: &Path) -> io::Result<DeleteKind> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(DeleteKind::Missing),
        Err(err) => return Err(err),
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        remove_link(path, &metadata)?;
        Ok(DeleteKind::Link)
    } else if file_type.is_dir() {
        std::fs::remove_dir_all(path)?;
        Ok(DeleteKind::Directory)
    } else {
        std::fs::remove_file(path)?;
        Ok(DeleteKind::File)
    }
}

#[cfg(windows)]
fn remove_link(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    use std::os::windows::fs::{FileTypeExt as _, MetadataExt as _};
    // Directory symlinks and junctions carry the directory attribute and need `RemoveDirectory`.
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    if metadata.file_type().is_symlink_dir()
        || metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
    {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(not(windows))]
fn remove_link(path: &Path, _metadata: &std::fs::Metadata) -> io::Result<()> {
    std::fs::remove_file(path)
}

/// Deletes the extra local files of a synchronization.
///
/// A task is removed only if its tree path is made of plain names, is not below a hidden folder,
/// its absolute local path has no `..` component and lies strictly below one of `roots`, and it
/// is not in `protected`; otherwise it is reported as refused. The check is lexical, so
/// links the user created above the entry (an addon folder on another disk) are honoured; the
/// entry itself is never followed ([`delete_no_follow`]). Failures do not fail the
/// synchronization (Java ignores them); they are logged and reported. Emits
/// `Phase(DeletingExtraFiles)` when there is something to delete. Blocking; stops between
/// tasks once `cancel` is cancelled.
// Mirrors service/ConnectionService.java:513-531
pub fn delete_extra_files(
    tasks: &[DeleteTask],
    roots: &DeleteRoots,
    protected: &HashSet<PathBuf>,
    hidden_folder_paths: &BTreeSet<String>,
    cancel: &CancellationToken,
    events: &EventSender,
) -> CleanupReport {
    let mut report = CleanupReport::default();
    if tasks.is_empty() || cancel.is_cancelled() {
        return report;
    }
    events.emit(SyncEvent::Phase(Phase::DeletingExtraFiles));
    for task in tasks {
        if cancel.is_cancelled() {
            break;
        }
        let path = absolute(&task.local_path);
        if !deletion_allowed(&task.relative_path, &path, roots, hidden_folder_paths) {
            tracing::warn!(
                "Refusing to delete {} ({}): outside the destination",
                path.display(),
                task.relative_path
            );
            report.refused.push(path);
            continue;
        }
        if protected.contains(&path) {
            report.skipped += 1;
            continue;
        }
        match delete_no_follow(&path) {
            Ok(DeleteKind::Missing) => report.skipped += 1,
            Ok(_) => report.deleted += 1,
            Err(err) => {
                tracing::warn!("Failed to delete {}: {err}", path.display());
                report.failed.push((path, err));
            }
        }
    }
    report
}

/// The safety checks of [`delete_extra_files`] for the entry at tree path `relative_path` and
/// absolute local `path`.
fn deletion_allowed(
    relative_path: &str,
    path: &Path,
    roots: &DeleteRoots,
    hidden: &BTreeSet<String>,
) -> bool {
    let plain_components = path.components().enumerate().all(|(i, c)| match c {
        Component::Normal(_) | Component::RootDir => true,
        Component::Prefix(_) => i == 0,
        Component::CurDir | Component::ParentDir => false,
    });
    relative_path.split('/').all(is_safe_name)
        && !is_hidden_path(relative_path, hidden)
        && path.is_absolute()
        && plain_components
        && roots.contains(path)
}

/// What [`copy_directory`] copied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyStats {
    /// Files copied.
    pub files: usize,

    /// Symbolic links skipped: in the source, or already in the target where a file or folder
    /// would be written.
    pub skipped_links: usize,
}

/// Copies `source` (a folder, recursively, or a file) to `target`.
///
/// Missing folders are created one level at a time (`create_dir`), existing files are
/// overwritten, extra files in `target` are kept and modification times are not preserved.
/// Symbolic links are never followed: a link in `source` is not copied, and a link in `target`
/// is never written through (Java follows both). Stops at the first error.
// Mirrors dao/FileAccessMethods.java:26-45
pub fn copy_directory(source: &Path, target: &Path) -> io::Result<CopyStats> {
    let mut stats = CopyStats::default();
    copy_recursive(source, target, &mut stats)?;
    Ok(stats)
}

fn copy_recursive(source: &Path, target: &Path, stats: &mut CopyStats) -> io::Result<()> {
    let file_type = std::fs::symlink_metadata(source)?.file_type();
    if file_type.is_symlink() {
        tracing::warn!("Not copying the symbolic link {}", source.display());
        stats.skipped_links += 1;
        return Ok(());
    }
    let existing = match std::fs::symlink_metadata(target) {
        Ok(metadata) => Some(metadata.file_type()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    if existing.is_some_and(|t| t.is_symlink()) {
        tracing::warn!("Not writing through the symbolic link {}", target.display());
        stats.skipped_links += 1;
        return Ok(());
    }
    if file_type.is_dir() {
        if existing.is_none() {
            std::fs::create_dir(target)?;
        }
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &target.join(entry.file_name()), stats)?;
        }
    } else {
        std::fs::copy(source, target)?;
        stats.files += 1;
    }
    Ok(())
}

/// Like Java `File.getAbsolutePath()`: no symlink resolution, no `..` folding.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
