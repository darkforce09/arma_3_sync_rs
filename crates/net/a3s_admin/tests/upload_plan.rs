//! Upload planning and the expansion of nodes into remote files.

use a3s_admin::check::expected_remote_files;
use a3s_admin::upload::{PlannedNode, plan_upload};
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

fn leaf(name: &str, sha1: Option<&str>, compressed: bool) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: sha1.map(str::to_owned),
        size: 1,
        compressed_size: 0,
        compressed,
        updated: false,
        deleted: false,
    })
}

fn dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    let mut d = SyncTreeDirectory::new(name);
    for child in children {
        d.add_child(child);
    }
    SyncTreeNode::Directory(d)
}

fn root(children: Vec<SyncTreeNode>) -> SyncTreeDirectory {
    let mut r = SyncTreeDirectory::new_root();
    for child in children {
        r.add_child(child);
    }
    r
}

fn node(path: &str, is_directory: bool, compressed: bool) -> PlannedNode {
    PlannedNode {
        relative_path: path.to_owned(),
        tree_path: path.to_owned(),
        is_directory,
        compressed,
    }
}

fn tree_paths(nodes: &[PlannedNode]) -> Vec<&str> {
    nodes.iter().map(|n| n.tree_path.as_str()).collect()
}

fn paths(nodes: &[PlannedNode]) -> Vec<&str> {
    nodes.iter().map(|n| n.relative_path.as_str()).collect()
}

#[test]
fn expected_files_per_node_kind() {
    let nodes = [
        node("@a", true, false),
        node("@a/x.pbo", false, true),
        node("y.txt", false, false),
    ];
    let render = |only_zip, zsync| -> Vec<String> {
        expected_remote_files(&nodes, only_zip, zsync)
            .iter()
            .map(|f| f.relative_file_path())
            .collect()
    };
    assert_eq!(
        render(false, true),
        [
            "/@a/",
            "/@a/x.pbo",
            "/@a/x.pbo.zip",
            "/@a/x.pbo.zsync",
            "/y.txt",
            "/y.txt.zsync"
        ]
    );
    assert_eq!(
        render(false, false),
        ["/@a/", "/@a/x.pbo", "/@a/x.pbo.zip", "/y.txt"]
    );
    // Only-zip mode: a compressed leaf is only its `.zip`, never a `.zsync`.
    assert_eq!(
        render(true, true),
        ["/@a/", "/@a/x.pbo.zip", "/y.txt", "/y.txt.zsync"]
    );
    assert_eq!(render(true, false), ["/@a/", "/@a/x.pbo.zip", "/y.txt"]);
    let files = expected_remote_files(&nodes, false, false);
    assert_eq!(files[1].transport_path(), "@a/x.pbo");
    assert_eq!(files[1].parent_relative_path, "@a");
    assert_eq!(files[3].parent_relative_path, "");
}

#[test]
fn plan_against_no_remote_uploads_everything_in_tree_order() {
    let local = root(vec![
        dir("@a", vec![leaf("a.pbo", Some("1"), true)]),
        leaf("top.txt", Some("2"), false),
    ]);
    let plan = plan_upload(&local, None);
    assert_eq!(paths(&plan.to_upload), ["@a", "@a/a.pbo", "top.txt"]);
    assert!(plan.to_upload[0].is_directory);
    assert!(plan.to_upload[1].compressed);
    assert!(plan.to_check.is_empty() && plan.to_delete.is_empty());
}

#[test]
fn plan_compares_paths_and_hashes() {
    let local = root(vec![
        dir(
            "@a",
            vec![
                leaf("same.pbo", Some("1"), false),
                leaf("changed.pbo", Some("2"), false),
                leaf("no_sha.pbo", None, false),
                leaf("kind", Some("3"), false),
            ],
        ),
        // Below a folder named like the root, remote paths restart at the root.
        dir(
            "@n",
            vec![dir("racine", vec![leaf("r.pbo", Some("4"), false)])],
        ),
    ]);
    let remote = root(vec![
        dir(
            "@a",
            vec![
                leaf("same.pbo", Some("1"), false),
                leaf("changed.pbo", Some("9"), false),
                leaf("no_sha.pbo", None, false),
                dir("kind", vec![]),
                leaf("gone.pbo", Some("5"), true),
            ],
        ),
        dir(
            "@n",
            vec![dir("racine", vec![leaf("r.pbo", Some("4"), false)])],
        ),
        dir(
            "@old",
            vec![dir("addons", vec![leaf("o.pbo", Some("6"), false)])],
        ),
        dir(".a3s", vec![leaf("sync", Some("7"), false)]),
    ]);
    let plan = plan_upload(&local, Some(&remote));
    assert_eq!(paths(&plan.to_check), ["@a", "@a/same.pbo", "@n", "r.pbo"]);
    assert_eq!(
        paths(&plan.to_upload),
        ["@a/changed.pbo", "@a/kind", "@a/no_sha.pbo"]
    );
    assert_eq!(tree_paths(&plan.to_check)[3], "@n/racine/r.pbo");
    // Remote-only paths, with the remote leaf's flag; nothing below a deleted directory, never
    // the remote `.a3s` folder.
    assert_eq!(
        plan.to_delete,
        [node("@a/gone.pbo", false, true), node("@old", true, false)]
    );
}
