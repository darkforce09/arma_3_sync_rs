//! Scanning the repository folder into a sync tree.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use a3s_repo_files::layout::{A3S_FOLDER, ZIP_EXTENSION, is_builder_ignored_file, is_compressible};
use a3s_types::java_string::compare_ignore_case;
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

use super::{BuildError, BuildEvent};

/// A leaf of the scanned tree, in deep-search order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LeafRef {
    /// `/`-joined path below `racine`.
    pub relpath: String,

    /// `File.getAbsolutePath()` of the file.
    pub path: PathBuf,
}

/// Whether `name` equals `.a3s` ignoring case (`equalsIgnoreCase`).
fn is_a3s_folder(name: &str) -> bool {
    compare_ignore_case(name, A3S_FOLDER).is_eq()
}

/// Whether the builder leaves a file out of the sync tree.
///
/// Java skips names containing `.zsync`, `.pbo.zip` or `.a3s`. Intentional difference from Java:
/// the compression stage's own outputs, `<name>.zip` for any compressible `<name>` (so also
/// `.ebo.zip`), are skipped too; Java turns its `.ebo.zip` files into leaves on the next build.
// Mirrors dao/repository/RepositoryBuildProcessor.java:486-489
pub fn is_excluded_name(name: &str) -> bool {
    is_builder_ignored_file(name)
        || name
            .strip_suffix(ZIP_EXTENSION)
            .is_some_and(is_compressible)
}

/// Lists a directory sorted by name bytes (deterministic order for names that differ only
/// in case). `None` when the directory cannot be listed (Java's `listFiles()` returns null).
fn list_sorted(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("Cannot list {}: {err}", dir.display());
            return None;
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => out.push((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )),
            Err(err) => {
                tracing::warn!("Cannot list {}: {err}", dir.display());
                return None;
            }
        }
    }
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Some(out)
}

struct Scanner<'a> {
    excluded_files: &'a BTreeSet<String>,

    /// Canonical paths of the directories being scanned, to detect symlink loops.
    stack: Vec<PathBuf>,
}

impl Scanner<'_> {
    // Mirrors dao/repository/RepositoryBuildProcessor.java:468-497
    fn generate(
        &mut self,
        parent: &mut SyncTreeDirectory,
        name: String,
        path: PathBuf,
    ) -> Result<(), BuildError> {
        // `File.isDirectory()` follows symbolic links; anything else is treated as a file.
        if fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
            if is_a3s_folder(&name) {
                return Ok(());
            }
            let canonical = fs::canonicalize(&path).map_err(|source| BuildError::Scan {
                path: path.clone(),
                source,
            })?;
            if self.stack.contains(&canonical) {
                return Err(BuildError::Scan {
                    path,
                    source: io::Error::other("symbolic link loop"),
                });
            }
            let mut dir = SyncTreeDirectory::new(name);
            if let Some(children) = list_sorted(&path) {
                self.stack.push(canonical);
                for (child_name, child_path) in children {
                    if child_name.to_lowercase() == "addons" {
                        dir.mark_as_addon = true;
                    }
                    self.generate(&mut dir, child_name, child_path)?;
                }
                self.stack.pop();
            }
            dir.sort_children();
            parent.children.push(SyncTreeNode::Directory(dir));
        } else if !is_excluded_name(&name)
            && !self
                .excluded_files
                .contains(path.to_string_lossy().as_ref())
        {
            // commons-io `FileUtils.sizeOf` throws for a missing file or a broken link.
            let size = fs::metadata(&path)
                .map_err(|source| BuildError::Scan {
                    path: path.clone(),
                    source,
                })?
                .len();
            parent.children.push(SyncTreeNode::Leaf(SyncTreeLeaf {
                name,
                size: i64::try_from(size).unwrap_or(i64::MAX),
                ..Default::default()
            }));
        }
        Ok(())
    }
}

/// Scans `root` (absolute, not canonicalized) into a `racine` tree, then prunes missing leaves and
/// empty directories.
// Mirrors dao/repository/RepositoryBuildProcessor.java:113-124
pub(crate) fn scan(
    root: &Path,
    excluded_files: &BTreeSet<String>,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<SyncTreeDirectory, BuildError> {
    let mut sync = SyncTreeDirectory::new_root();
    let mut scanner = Scanner {
        excluded_files,
        stack: Vec::new(),
    };
    if let Some(children) = list_sorted(root) {
        if let Ok(canonical) = fs::canonicalize(root) {
            scanner.stack.push(canonical);
        }
        for (name, path) in children {
            scanner.generate(&mut sync, name, path)?;
        }
    }
    sync.sort_children();
    prune(&mut sync, root, "", on_event);
    Ok(sync)
}

/// Removes leaves whose file vanished and directories left empty, depth first.
// Mirrors dao/repository/RepositoryBuildProcessor.java:423-445
fn prune(
    dir: &mut SyncTreeDirectory,
    root: &Path,
    prefix: &str,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) {
    let warn = |text: String| {
        tracing::warn!("{text}");
        on_event(BuildEvent::Warning(text));
    };
    let children = std::mem::take(&mut dir.children);
    for child in children {
        let relpath = if prefix.is_empty() {
            child.name().to_owned()
        } else {
            format!("{prefix}/{}", child.name())
        };
        match child {
            SyncTreeNode::Leaf(leaf) => {
                if relative(root, &relpath).exists() {
                    dir.children.push(SyncTreeNode::Leaf(leaf));
                } else {
                    warn(format!(
                        "Warning: Removing missing file from metadata: {relpath}"
                    ));
                }
            }
            SyncTreeNode::Directory(mut sub) => {
                prune(&mut sub, root, &relpath, on_event);
                if sub.children.is_empty() {
                    warn(format!(
                        "Warning: Removing empty directory from metadata: {relpath}"
                    ));
                } else {
                    dir.children.push(SyncTreeNode::Directory(sub));
                }
            }
        }
    }
}

/// `<root>/<relpath>`.
pub(crate) fn relative(root: &Path, relpath: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(relpath.split('/'));
    path
}

/// The leaves of `sync` in deep-search order, with their paths.
pub(crate) fn leaves(sync: &SyncTreeDirectory, root: &Path) -> Vec<LeafRef> {
    let mut out = Vec::new();
    sync.walk(&mut |relpath, node| {
        if matches!(node, SyncTreeNode::Leaf(_)) {
            out.push(LeafRef {
                relpath: relpath.to_owned(),
                path: relative(root, relpath),
            });
        }
    });
    out
}

/// Calls `f` on every leaf in deep-search order, with its index.
pub(crate) fn for_each_leaf_mut(
    sync: &mut SyncTreeDirectory,
    f: &mut impl FnMut(usize, &mut SyncTreeLeaf),
) {
    fn rec(
        dir: &mut SyncTreeDirectory,
        next: &mut usize,
        f: &mut impl FnMut(usize, &mut SyncTreeLeaf),
    ) {
        for child in &mut dir.children {
            match child {
                SyncTreeNode::Directory(d) => rec(d, next, f),
                SyncTreeNode::Leaf(l) => {
                    f(*next, l);
                    *next += 1;
                }
            }
        }
    }
    rec(sync, &mut 0, f);
}

/// Sum of the sizes of all regular files below `root`, except below a top-level `.a3s`.
///
/// Like `Files.walk`, symbolic links to directories are not followed, but links to files count
/// with their target's size. Computed before the zsync and zip stages, so the outputs of the
/// previous build (including orphans) are counted and this build's new ones are not.
// Mirrors dao/repository/RepositoryBuildProcessor.java:499-519
pub(crate) fn total_files_size(root: &Path) -> Result<i64, BuildError> {
    fn rec(dir: &Path, top: bool, total: &mut u64) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if top && is_a3s_folder(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                rec(&path, false, total)?;
            } else if let Ok(meta) = fs::metadata(&path) {
                // `Files.isRegularFile` follows links; a broken link is not a regular file.
                if meta.is_file() {
                    *total = total.saturating_add(meta.len());
                }
            }
        }
        Ok(())
    }
    let mut total = 0u64;
    rec(root, true, &mut total).map_err(|source| BuildError::Scan {
        path: root.to_path_buf(),
        source,
    })?;
    Ok(i64::try_from(total).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::is_excluded_name;

    #[test]
    fn excluded_names() {
        for name in [
            "x.zsync",
            "a.pbo.zip",
            "b.a3s.bak",
            "c.ebo.zip",
            "d.PBO.zip",
            "e.zsync.txt",
        ] {
            assert!(is_excluded_name(name), "{name}");
        }
        for name in ["c.ebo", "f.zip", "g.pbo.ZIP", "h.txt.zip", "mod.pbo"] {
            assert!(!is_excluded_name(name), "{name}");
        }
    }
}
