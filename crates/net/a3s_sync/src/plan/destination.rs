//! Local folder of every node.

use std::path::Path;

use super::{AddonLocator, absolute};
use crate::model::{PlanDirectory, PlanNode, PlanTree};

/// Sets the `destination_path` of every node below the root (pre-order).
///
/// * Children of the root get `destination` as given.
/// * Other nodes get the absolute path of their parent folder
///   (`parent.destination_path/parent.name`).
/// * With `auto_discover`, a folder marked as addon at any depth whose name `addons` knows is
///   redirected to the folder containing the existing local copy; its content follows.
///
/// The root's destination stays `None`.
// Mirrors service/RepositoryService.java:477-511
pub fn assign_destinations(
    tree: &mut PlanTree,
    destination: &Path,
    auto_discover: bool,
    addons: &dyn AddonLocator,
) {
    tree.root.destination_path = None;
    assign_children(&mut tree.root, destination, auto_discover, addons);
}

/// Assigns `child_destination` to the children of `directory`, then recurses.
fn assign_children(
    directory: &mut PlanDirectory,
    child_destination: &Path,
    auto_discover: bool,
    addons: &dyn AddonLocator,
) {
    for child in &mut directory.children {
        match child {
            PlanNode::Leaf(leaf) => leaf.destination_path = Some(child_destination.to_path_buf()),
            PlanNode::Directory(d) => {
                let mut path = child_destination.to_path_buf();
                // Mirrors service/RepositoryService.java:494-499
                if auto_discover
                    && d.mark_as_addon
                    && let Some(found) = addons.addon_parent_dir(&d.name)
                {
                    path = found;
                }
                let grandchild_destination = absolute(&path.join(&d.name));
                d.destination_path = Some(path);
                assign_children(d, &grandchild_destination, auto_discover, addons);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use super::*;
    use crate::plan::NoAddons;

    fn leaf(name: &str) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            ..Default::default()
        })
    }

    fn dir(name: &str, mark_as_addon: bool, children: Vec<SyncTreeNode>) -> SyncTreeNode {
        SyncTreeNode::Directory(SyncTreeDirectory {
            name: name.to_owned(),
            mark_as_addon,
            children,
            ..Default::default()
        })
    }

    /// `top.txt`, `mods/@ACE/addons/ace.pbo` (`@ACE` marked as addon if `marked`).
    fn tree(marked: bool) -> PlanTree {
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![
            dir(
                "mods",
                false,
                vec![dir(
                    "@ACE",
                    marked,
                    vec![dir("addons", false, vec![leaf("ace.pbo")])],
                )],
            ),
            leaf("top.txt"),
        ];
        PlanTree::from_sync_tree(&root)
    }

    fn dest(tree: &PlanTree, path: &str) -> Option<PathBuf> {
        tree.get(path)
            .and_then(PlanNode::destination_path)
            .map(Path::to_path_buf)
    }

    #[test]
    fn a2_destinations_default() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        let mut t = tree(true);
        assign_destinations(&mut t, base, true, &NoAddons);
        assert_eq!(t.root.destination_path, None);
        assert_eq!(dest(&t, "mods"), Some(base.to_path_buf()));
        assert_eq!(dest(&t, "top.txt"), Some(base.to_path_buf()));
        assert_eq!(dest(&t, "mods/@ACE"), Some(base.join("mods")));
        assert_eq!(dest(&t, "mods/@ACE/addons"), Some(base.join("mods/@ACE")));
        assert_eq!(
            dest(&t, "mods/@ACE/addons/ace.pbo"),
            Some(base.join("mods/@ACE/addons"))
        );
        assert_eq!(
            t.get("mods/@ACE/addons/ace.pbo")
                .and_then(PlanNode::local_path),
            Some(base.join("mods/@ACE/addons/ace.pbo"))
        );
    }

    #[test]
    fn a2_destinations_relative_root_children_verbatim() {
        let mut t = tree(false);
        assign_destinations(&mut t, Path::new("rel"), true, &NoAddons);
        assert_eq!(dest(&t, "mods"), Some(PathBuf::from("rel")));
        let nested = dest(&t, "mods/@ACE").expect("set");
        assert!(nested.is_absolute(), "{nested:?}");
        assert!(nested.ends_with("rel/mods"), "{nested:?}");
    }

    #[test]
    fn a2_destinations_auto_discover() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        let addons: HashMap<String, PathBuf> =
            [("@ace".to_owned(), PathBuf::from("/opt/addons"))].into();

        let mut t = tree(true);
        assign_destinations(&mut t, base, true, &addons);
        assert_eq!(dest(&t, "mods/@ACE"), Some(PathBuf::from("/opt/addons")));
        assert_eq!(
            dest(&t, "mods/@ACE/addons"),
            Some(PathBuf::from("/opt/addons/@ACE"))
        );
        assert_eq!(
            dest(&t, "mods/@ACE/addons/ace.pbo"),
            Some(PathBuf::from("/opt/addons/@ACE/addons"))
        );
        assert_eq!(dest(&t, "mods"), Some(base.to_path_buf()));

        // Auto-discover off.
        let mut t = tree(true);
        assign_destinations(&mut t, base, false, &addons);
        assert_eq!(dest(&t, "mods/@ACE"), Some(base.join("mods")));

        // Not marked as addon.
        let mut t = tree(false);
        assign_destinations(&mut t, base, true, &addons);
        assert_eq!(dest(&t, "mods/@ACE"), Some(base.join("mods")));

        // A closure receives the name as written.
        let mut t = tree(true);
        let closure = |name: &str| (name == "@ACE").then(|| PathBuf::from("/c"));
        assign_destinations(&mut t, base, true, &closure);
        assert_eq!(dest(&t, "mods/@ACE"), Some(PathBuf::from("/c")));
    }
}
