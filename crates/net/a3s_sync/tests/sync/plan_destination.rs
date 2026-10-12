//! Local folder of every planned node, with and without addon auto-discovery.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use a3s_sync::plan::{NoAddons, assign_destinations};
use a3s_sync::{PlanNode, PlanTree};
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

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
fn destinations_default() {
    let base = tempfile::tempdir().expect("tempdir");
    let base = base.path();
    let mut t = tree(true);
    assign_destinations(&mut t, base, true, &NoAddons).expect("safe names");
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
fn destinations_relative_root_children_verbatim() {
    let mut t = tree(false);
    assign_destinations(&mut t, Path::new("rel"), true, &NoAddons).expect("safe names");
    assert_eq!(dest(&t, "mods"), Some(PathBuf::from("rel")));
    let nested = dest(&t, "mods/@ACE").expect("set");
    assert!(nested.is_absolute(), "{nested:?}");
    assert!(nested.ends_with("rel/mods"), "{nested:?}");
}

#[test]
fn destinations_auto_discover() {
    let base = tempfile::tempdir().expect("tempdir");
    let base = base.path();
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let found = elsewhere.path().join("addons");
    let addons: HashMap<String, PathBuf> = [("@ace".to_owned(), found.clone())].into();

    let mut t = tree(true);
    assign_destinations(&mut t, base, true, &addons).expect("safe names");
    assert_eq!(dest(&t, "mods/@ACE"), Some(found.clone()));
    assert_eq!(dest(&t, "mods/@ACE/addons"), Some(found.join("@ACE")));
    assert_eq!(
        dest(&t, "mods/@ACE/addons/ace.pbo"),
        Some(found.join("@ACE").join("addons"))
    );
    assert_eq!(dest(&t, "mods"), Some(base.to_path_buf()));

    // Auto-discover off.
    let mut t = tree(true);
    assign_destinations(&mut t, base, false, &addons).expect("safe names");
    assert_eq!(dest(&t, "mods/@ACE"), Some(base.join("mods")));

    // Not marked as addon.
    let mut t = tree(false);
    assign_destinations(&mut t, base, true, &addons).expect("safe names");
    assert_eq!(dest(&t, "mods/@ACE"), Some(base.join("mods")));

    // A closure receives the name as written.
    let mut t = tree(true);
    let closure = |name: &str| (name == "@ACE").then(|| found.clone());
    assign_destinations(&mut t, base, true, &closure).expect("safe names");
    assert_eq!(dest(&t, "mods/@ACE"), Some(found.clone()));
}
