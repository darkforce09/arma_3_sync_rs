//! The status tree: what a download does to every file and folder of a planned tree.

use a3s_sync::plan::{NodeStatus, StatusTree};
use a3s_sync::{PlanDirectory, PlanLeaf, PlanNode, PlanTree};

/// A repository file; `local` is the SHA-1 of the local copy, if any.
fn file(name: &str, size: u64, local: Option<&str>, updated: bool) -> PlanNode {
    let mut leaf = PlanLeaf::new(name);
    leaf.size = size;
    leaf.sha1 = Some("remote".to_owned());
    leaf.local_sha1 = local.map(str::to_owned);
    leaf.updated = updated;
    PlanNode::Leaf(leaf)
}

/// A local file the repository does not have.
fn extra_file(name: &str) -> PlanNode {
    let mut leaf = PlanLeaf::new(name);
    leaf.deleted = true;
    PlanNode::Leaf(leaf)
}

fn folder(name: &str, missing: bool, extra: bool, children: Vec<PlanNode>) -> PlanNode {
    let mut directory = PlanDirectory::new(name);
    directory.updated = missing;
    directory.deleted = extra;
    for child in children {
        directory.add_child(child);
    }
    PlanNode::Directory(directory)
}

fn tree(children: Vec<PlanNode>) -> PlanTree {
    let mut tree = PlanTree::new();
    for child in children {
        tree.root.add_child(child);
    }
    tree.recompute_changed();
    tree
}

fn status(tree: &StatusTree, path: &str) -> NodeStatus {
    tree.get(path).expect(path).status
}

#[test]
fn installed_mod_with_one_changed_file_is_update_with_only_that_file_flagged() {
    let plan = tree(vec![folder(
        "@tfar",
        false,
        false,
        vec![
            folder(
                "addons",
                false,
                false,
                vec![file("radio.pbo", 900, Some("same"), false)],
            ),
            file("pipe.dll", 237, Some("old"), true),
            file("mod.cpp", 5, Some("same"), false),
        ],
    )]);
    let status_tree = StatusTree::from_plan(&plan);

    let tfar = status_tree.get("@tfar").expect("@tfar");
    assert_eq!(tfar.status, NodeStatus::Update);
    assert_eq!(tfar.download_size, 237);
    assert_eq!(tfar.files_to_download, 1);
    assert_eq!(status(&status_tree, "@tfar/pipe.dll"), NodeStatus::Update);
    assert_eq!(status(&status_tree, "@tfar/mod.cpp"), NodeStatus::UpToDate);
    assert_eq!(status(&status_tree, "@tfar/addons"), NodeStatus::UpToDate);
    assert_eq!(
        status(&status_tree, "@tfar/addons/radio.pbo"),
        NodeStatus::UpToDate
    );
}

#[test]
fn missing_mod_is_not_downloaded_with_its_full_size() {
    let plan = tree(vec![folder(
        "@new",
        true,
        false,
        vec![
            file("a.pbo", 40, None, true),
            folder("keys", true, false, vec![file("k.bikey", 2, None, true)]),
        ],
    )]);
    let status_tree = StatusTree::from_plan(&plan);

    let new = status_tree.get("@new").expect("@new");
    assert_eq!(new.status, NodeStatus::NotDownloaded);
    assert_eq!(new.download_size, 42);
    assert_eq!(new.files_to_download, 2);
    assert_eq!(
        status(&status_tree, "@new/keys/k.bikey"),
        NodeStatus::NotDownloaded
    );
}

#[test]
fn extra_folder_is_removed_even_though_its_content_is_changed() {
    let plan = tree(vec![folder("@old", false, true, vec![extra_file("x.txt")])]);
    let status_tree = StatusTree::from_plan(&plan);

    let old = status_tree.get("@old").expect("@old");
    assert_eq!(old.status, NodeStatus::Removed);
    assert_eq!(old.files_to_remove, 1);
    assert_eq!(old.download_size, 0);
    assert_eq!(status(&status_tree, "@old/x.txt"), NodeStatus::Removed);
}

#[test]
fn extra_file_in_an_installed_mod_makes_it_update() {
    let plan = tree(vec![folder(
        "@ace",
        false,
        false,
        vec![
            file("ace.pbo", 10, Some("same"), false),
            extra_file("stray.pbo"),
        ],
    )]);
    let status_tree = StatusTree::from_plan(&plan);

    let ace = status_tree.get("@ace").expect("@ace");
    assert_eq!(ace.status, NodeStatus::Update);
    assert_eq!(ace.files_to_remove, 1);
    assert_eq!(ace.files_to_download, 0);
}

#[test]
fn compressed_file_counts_its_compressed_size() {
    let mut leaf = PlanLeaf::new("big.pbo");
    leaf.size = 1000;
    leaf.compressed = true;
    leaf.compressed_size = 300;
    leaf.updated = true;
    let plan = tree(vec![folder(
        "@mod",
        true,
        false,
        vec![PlanNode::Leaf(leaf)],
    )]);

    assert_eq!(StatusTree::from_plan(&plan).roots[0].download_size, 300);
}

#[test]
fn totals_count_top_level_statuses_including_files() {
    let plan = tree(vec![
        folder(
            "@same",
            false,
            false,
            vec![file("s.pbo", 7, Some("same"), false)],
        ),
        folder(
            "@changed",
            false,
            false,
            vec![file("c.pbo", 100, Some("old"), true)],
        ),
        folder("@new", true, false, vec![file("n.pbo", 50, None, true)]),
        folder("@old", false, true, vec![extra_file("o.txt")]),
        file("readme.txt", 3, Some("old"), true),
    ]);
    let status_tree = StatusTree::from_plan(&plan);
    let totals = status_tree.totals();

    assert_eq!(totals.up_to_date, 1);
    assert_eq!(totals.to_update, 2);
    assert_eq!(totals.not_downloaded, 1);
    assert_eq!(totals.removed, 1);
    assert_eq!(totals.files_to_download, 3);
    assert_eq!(totals.files_to_remove, 1);
    assert_eq!(totals.download_size, 153);
    assert!(!status_tree.is_up_to_date());
    assert_eq!(status(&status_tree, "readme.txt"), NodeStatus::Update);
}

#[test]
fn tree_with_nothing_to_do_is_up_to_date() {
    let plan = tree(vec![folder(
        "@same",
        false,
        false,
        vec![file("s.pbo", 7, Some("same"), false)],
    )]);
    let status_tree = StatusTree::from_plan(&plan);

    assert!(status_tree.is_up_to_date());
    assert_eq!(status_tree.totals().download_size, 0);
}
