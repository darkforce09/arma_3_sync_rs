//! D-15: hostile remote names never reach the local file system.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_error::{ErrorKind, UserFacing as _};
use a3s_sync::plan::{
    NoAddons, assign_destinations, is_safe_name, plan_repository, select_all_except_root,
    validate_names,
};
use a3s_sync::{EventSender, PlanError, PlanSummary, PlanTree, RemoteMetadata, SyncOptions};
use a3s_transport::CancellationToken;
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

const HOSTILE: [&str; 9] = [
    "..",
    "/abs",
    "a/../..",
    "..\\x",
    "",
    ".",
    "C:x",
    "a/b",
    "nul\0byte",
];

fn leaf(name: &str) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: Some("x".to_owned()),
        size: 1,
        ..Default::default()
    })
}

fn dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        name: name.to_owned(),
        mark_as_addon: true,
        children,
        ..Default::default()
    })
}

fn sync_tree(children: Vec<SyncTreeNode>) -> SyncTreeDirectory {
    let mut root = SyncTreeDirectory::new_root();
    root.children = children;
    root
}

fn options(destination: &Path, exact_match: bool) -> SyncOptions {
    SyncOptions {
        repository_name: "R".to_owned(),
        event_name: None,
        destination: destination.to_path_buf(),
        auto_discover: true,
        exact_match,
        hidden_folder_paths: Default::default(),
        client_connections: 1,
        max_download_speed: 0,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
    }
}

/// Every shape at the root and nested, as a folder and as a file.
fn hostile_trees(name: &str) -> Vec<(String, SyncTreeDirectory)> {
    vec![
        (name.to_owned(), sync_tree(vec![leaf(name)])),
        (name.to_owned(), sync_tree(vec![dir(name, vec![leaf("f")])])),
        (
            format!("@ok/{name}"),
            sync_tree(vec![dir("@ok", vec![leaf(name)])]),
        ),
        (
            format!("@ok/{name}"),
            sync_tree(vec![dir("@ok", vec![dir(name, vec![])])]),
        ),
    ]
}

#[test]
fn d15_is_safe_name_table() {
    for name in HOSTILE {
        assert!(!is_safe_name(name), "{name:?}");
    }
    for name in [
        "@ace", "addons", "x.pbo", "..x", "x..", "a b", ".hidden", "ünï",
    ] {
        assert!(is_safe_name(name), "{name:?}");
    }
}

#[test]
fn d15_plan_rejects_hostile_names() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    // Bait outside the destination.
    std::fs::write(base.path().join("precious.bin"), b"keep").expect("write");
    for name in HOSTILE {
        for (relative_path, sync) in hostile_trees(name) {
            for exact_match in [false, true] {
                let metadata = RemoteMetadata {
                    sync: Some(Arc::new(sync.clone())),
                    ..Default::default()
                };
                let result = plan_repository(
                    &metadata,
                    &options(&dest, exact_match),
                    &NoAddons,
                    &mut a3s_hash::Sha1Cache::default(),
                    &CancellationToken::new(),
                    &EventSender::noop(),
                );
                let Err(err) = result else {
                    panic!("{name:?} accepted (exact_match {exact_match})")
                };
                assert!(
                    matches!(&err, PlanError::UnsafeName { relative_path: p, .. } if *p == relative_path),
                    "{name:?}: {err:?}"
                );
                assert_eq!(err.kind(), ErrorKind::InvalidData);
                assert_eq!(
                    err.to_string(),
                    format!("Unsafe file name in the repository: {relative_path}")
                );
            }
        }
    }
    assert!(base.path().join("precious.bin").exists());
}

#[test]
fn d15_assign_destinations_rejects_and_sets_nothing() {
    for name in HOSTILE {
        for (relative_path, sync) in hostile_trees(name) {
            let mut tree = PlanTree::from_sync_tree(&sync);
            assert!(validate_names(&tree).is_err());
            let err = assign_destinations(&mut tree, Path::new("/dest"), false, &NoAddons)
                .expect_err("unsafe");
            assert!(
                matches!(&err, PlanError::UnsafeName { relative_path: p, .. } if *p == relative_path)
            );
            let mut any = false;
            tree.walk(&mut |_, node| any |= node.destination_path().is_some());
            assert!(!any, "{name:?}: destinations set");
        }
    }
}

#[test]
fn d15_summary_never_builds_tasks_from_unsafe_names() {
    // A tree with destinations set by hand, bypassing validation.
    let mut tree = PlanTree::from_sync_tree(&sync_tree(vec![
        dir("..", vec![leaf("evil")]),
        leaf("/abs"),
        dir("@ok", vec![leaf("ok.pbo")]),
    ]));
    tree.walk_mut(&mut |_, node| match node {
        a3s_sync::PlanNode::Directory(d) => {
            d.destination_path = Some(PathBuf::from("/dest"));
            d.updated = true;
        }
        a3s_sync::PlanNode::Leaf(l) => {
            l.destination_path = Some(PathBuf::from("/dest"));
            l.updated = true;
        }
    });
    select_all_except_root(&mut tree);
    let summary = PlanSummary::compute(&tree);
    let paths: Vec<&str> = summary
        .download_tasks
        .iter()
        .map(|t| t.relative_path.as_str())
        .collect();
    assert_eq!(paths, ["@ok", "@ok/ok.pbo"]);
}
