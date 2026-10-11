//! Local content missing from the repository.

use std::collections::HashSet;
use std::path::Path;

use super::{PART_EXTENSION, PBO_ZIP_EXTENSION, is_safe_name};
use crate::model::{PlanDirectory, PlanLeaf, PlanNode, PlanTree};

/// Adds every local file or folder that the repository doesn't have as a `deleted` node, and
/// returns the number of nodes added.
///
/// Runs after [`super::mark_hidden`]: hidden folders are skipped with their whole subtree.
/// Every remote folder below the root is scanned; the root folder (`destination`) only with
/// `exact_match` (Java quirk kept: extra files inside a remote addon folder are always found).
/// Names are compared case-insensitively. Local files whose name contains
/// [`PART_EXTENSION`] or [`PBO_ZIP_EXTENSION`] are never extra. The content of an extra folder
/// is added too. Unreadable folders are silently skipped.
///
/// A local symbolic link is never followed: it becomes one deleted file node for the link
/// itself, even when it points to a folder (D-15; Java lists the target's content, so a link
/// could make it delete outside the destination, and link loops never end). Folders whose name
/// is not a plain path component (see [`super::validate_names`]) are not listed.
// Mirrors service/RepositoryService.java:607-659
pub fn add_extra_local_files(tree: &mut PlanTree, destination: &Path, exact_match: bool) -> usize {
    scan(&mut tree.root, true, destination, exact_match)
}

fn scan(
    directory: &mut PlanDirectory,
    is_root: bool,
    destination: &Path,
    exact_match: bool,
) -> usize {
    if directory.hidden || (!is_root && !is_safe_name(&directory.name)) {
        return 0;
    }
    let listed = if !is_root {
        directory.local_path()
    } else if exact_match {
        Some(destination.to_path_buf())
    } else {
        None
    };
    let mut added = 0;
    if let Some(entries) = listed.as_deref().and_then(list) {
        // Mirrors service/RepositoryService.java:625-628
        let remote_names: HashSet<String> = directory
            .children
            .iter()
            .map(|c| c.name().to_lowercase())
            .collect();
        // Mirrors service/RepositoryService.java:635-639
        let child_destination = directory
            .local_path()
            .unwrap_or_else(|| destination.to_path_buf());
        let mut new_nodes = Vec::new();
        for LocalEntry {
            name,
            is_dir,
            is_symlink,
        } in entries
        {
            if remote_names.contains(&name.to_lowercase()) {
                continue;
            }
            if is_dir && !is_symlink {
                let mut d = PlanDirectory::new(name);
                d.deleted = true;
                d.destination_path = Some(child_destination.clone());
                new_nodes.push(PlanNode::Directory(d));
            } else if is_dir
                || (!name.contains(PART_EXTENSION) && !name.contains(PBO_ZIP_EXTENSION))
            {
                // A link to a folder is offered like a folder (Java applies the name filters to
                // files only), but as a file node: deleting it removes the link only.
                let mut l = PlanLeaf::new(name);
                l.deleted = true;
                l.destination_path = Some(child_destination.clone());
                new_nodes.push(PlanNode::Leaf(l));
            }
        }
        added = new_nodes.len();
        insert_all(directory, new_nodes);
    }
    // Mirrors service/RepositoryService.java:654-656 (the new folders are enumerated too)
    for child in &mut directory.children {
        if let PlanNode::Directory(d) = child {
            added += scan(d, false, destination, exact_match);
        }
    }
    added
}

/// Inserts `nodes` as successive [`PlanDirectory::add_child`] calls would, with one sort.
///
/// `add_child` pushes and stable-sorts; pushing everything and sorting once gives the same
/// order (ties keep insertion order either way).
fn insert_all(directory: &mut PlanDirectory, mut nodes: Vec<PlanNode>) {
    let Some(last) = nodes.pop() else {
        return;
    };
    directory.children.append(&mut nodes);
    directory.add_child(last);
}

/// One entry of a local folder.
struct LocalEntry {
    name: String,

    /// A folder, or a link to one (Java `File.isDirectory()` follows links).
    is_dir: bool,

    /// A symbolic link (never followed by the scan).
    is_symlink: bool,
}

/// The entries of `path`, or `None` when it can't be listed (Java `File.listFiles()` returning
/// `null`).
fn list(path: &Path) -> Option<Vec<LocalEntry>> {
    let read_dir = match std::fs::read_dir(path) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            tracing::debug!("Not listing {}: {err}", path.display());
            return None;
        }
    };
    let mut out = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!("Not listing {}: {err}", path.display());
                return None;
            }
        };
        let entry_path = entry.path();
        match entry.file_name().into_string() {
            Ok(name) => {
                let is_symlink = std::fs::symlink_metadata(&entry_path)
                    .is_ok_and(|m| m.file_type().is_symlink());
                out.push(LocalEntry {
                    name,
                    is_dir: entry_path.is_dir(),
                    is_symlink,
                });
            }
            Err(name) => tracing::debug!("Skipping non-UTF-8 local name {name:?}"),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use std::path::PathBuf;

    use super::*;
    use crate::plan::{NoAddons, assign_destinations, mark_hidden};

    fn leaf(name: &str) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: Some("abc".to_owned()),
            ..Default::default()
        })
    }

    fn dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
        SyncTreeNode::Directory(SyncTreeDirectory {
            name: name.to_owned(),
            mark_as_addon: name.starts_with('@'),
            children,
            ..Default::default()
        })
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"x").expect("write");
    }

    /// Remote `@a/addons/a.pbo`; local extras around it.
    fn setup() -> (tempfile::TempDir, PlanTree) {
        let base = tempfile::tempdir().expect("tempdir");
        let b = base.path();
        for f in [
            "@a/addons/a.pbo",
            "@a/addons/old.pbo",
            "@a/addons/a.pbo.part",
            "@a/addons/b.pbo.zip",
            "@a/addons/my.particles.pbo",
            "@a/addons/A.PBO",
            "@a/old/sub/f.txt",
            "@stale/x.txt",
        ] {
            touch(&b.join(f));
        }
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![dir("@a", vec![dir("addons", vec![leaf("a.pbo")])])];
        let mut tree = PlanTree::from_sync_tree(&root);
        assign_destinations(&mut tree, b, false, &NoAddons).expect("safe names");
        (base, tree)
    }

    fn deleted(tree: &PlanTree) -> Vec<(String, bool, PathBuf)> {
        let mut out = Vec::new();
        tree.walk(&mut |path, node| {
            if node.deleted() {
                out.push((
                    path.to_owned(),
                    !node.is_leaf(),
                    node.destination_path().expect("destination").to_path_buf(),
                ));
            }
        });
        out
    }

    #[test]
    fn a5_extra_files_in_remote_folders() {
        let (base, mut tree) = setup();
        let b = base.path();
        let added = add_extra_local_files(&mut tree, b, false);
        assert_eq!(added, 4);
        assert_eq!(
            deleted(&tree),
            [
                ("@a/addons/old.pbo".to_owned(), false, b.join("@a/addons")),
                ("@a/old".to_owned(), true, b.join("@a")),
                ("@a/old/sub".to_owned(), true, b.join("@a/old")),
                ("@a/old/sub/f.txt".to_owned(), false, b.join("@a/old/sub")),
            ]
        );
        let Some(PlanNode::Leaf(l)) = tree.get("@a/addons/old.pbo") else {
            panic!("leaf")
        };
        assert_eq!((l.sha1.as_deref(), l.size, l.compressed_size), (None, 0, 0));
        let Some(PlanNode::Directory(d)) = tree.get("@a/old") else {
            panic!("dir")
        };
        assert!(!d.updated && !d.hidden && !d.mark_as_addon && !d.selected);
        assert!(tree.get("@stale").is_none());

        // Children order: directories first.
        let Some(PlanNode::Directory(a)) = tree.get("@a") else {
            panic!("dir")
        };
        let names: Vec<_> = a.children.iter().map(PlanNode::name).collect();
        assert_eq!(names, ["addons", "old"]);

        // `changed`: the deleted leaves propagate; an empty deleted folder alone doesn't.
        tree.recompute_changed();
        assert!(tree.root.changed);
        assert!(matches!(tree.get("@a"), Some(PlanNode::Directory(d)) if d.changed));
    }

    #[test]
    fn a5_extra_root_only_with_exact_match() {
        let (base, mut tree) = setup();
        let b = base.path();
        add_extra_local_files(&mut tree, b, true);
        let found: Vec<_> = deleted(&tree)
            .into_iter()
            .filter(|(p, ..)| p.starts_with("@stale"))
            .collect();
        assert_eq!(
            found,
            [
                ("@stale".to_owned(), true, b.to_path_buf()),
                ("@stale/x.txt".to_owned(), false, b.join("@stale")),
            ]
        );
    }

    #[test]
    fn a5_extra_hidden_folder_skipped() {
        let (base, mut tree) = setup();
        mark_hidden(&mut tree, &["@a".to_owned()].into());
        assert_eq!(add_extra_local_files(&mut tree, base.path(), false), 0);
        assert!(deleted(&tree).is_empty());
    }

    #[test]
    fn a5_empty_deleted_folder_does_not_set_changed() {
        let base = tempfile::tempdir().expect("tempdir");
        let b = base.path();
        std::fs::create_dir_all(b.join("@a/empty")).expect("mkdir");
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![dir("@a", vec![])];
        let mut tree = PlanTree::from_sync_tree(&root);
        assign_destinations(&mut tree, b, false, &NoAddons).expect("safe names");
        assert_eq!(add_extra_local_files(&mut tree, b, false), 1);
        tree.recompute_changed();
        assert!(!tree.root.changed);
        assert!(matches!(tree.get("@a/empty"), Some(n) if n.deleted()));
    }

    #[cfg(unix)]
    #[test]
    fn d15_symlinks_are_never_followed() {
        let base = tempfile::tempdir().expect("tempdir");
        let dest = base.path().join("dest");
        touch(&base.path().join("outside/data.bin"));
        std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
        let link = |target: &Path, name: &str| {
            std::os::unix::fs::symlink(target, dest.join("@a").join(name)).expect("ln");
        };
        link(&base.path().join("outside"), "to_dir");
        link(&base.path().join("outside/data.bin"), "to_file");
        link(Path::new("."), "loop");
        link(&base.path().join("missing"), "dangling.part");
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![dir("@a", vec![])];
        for exact_match in [false, true] {
            let mut tree = PlanTree::from_sync_tree(&root);
            assign_destinations(&mut tree, &dest, false, &NoAddons).expect("safe names");
            assert_eq!(add_extra_local_files(&mut tree, &dest, exact_match), 3);
            // Each link is one deleted file node; nothing below it.
            assert_eq!(
                deleted(&tree),
                [
                    ("@a/loop".to_owned(), false, dest.join("@a")),
                    ("@a/to_dir".to_owned(), false, dest.join("@a")),
                    ("@a/to_file".to_owned(), false, dest.join("@a")),
                ]
            );
        }
    }
}
