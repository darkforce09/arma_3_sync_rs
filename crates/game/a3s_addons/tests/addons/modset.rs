//! Addon groups built from a repository's sync tree and events, and the group-tree helpers.

use std::collections::BTreeMap;
use std::path::PathBuf;

use a3s_addons::{
    REPOSITORY_GROUP_ROOT_NAME, add_groups_from_events, add_groups_from_repository,
    group_from_repository, normalize_group_tree, set_selected_recursive, update_group_modsets,
};
use a3s_types::constants::ModsetType;
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};
use a3s_types::{Event, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_repos")
        .join(path)
}

/// `/`-joined paths of the leaves below `dir`.
fn leaf_paths(dir: &TreeDirectory) -> Vec<String> {
    fn visit(dir: &TreeDirectory, prefix: &str, out: &mut Vec<String>) {
        for child in &dir.children {
            let path = format!("{prefix}{}", child.name());
            match child {
                TreeNode::Leaf(_) => out.push(path),
                TreeNode::Directory(d) => visit(d, &format!("{path}/"), out),
            }
        }
    }
    let mut out = Vec::new();
    visit(dir, "", &mut out);
    out
}

/// `/`-joined paths of the sync directories marked as addons, not looking inside addons.
fn addon_paths(dir: &SyncTreeDirectory, prefix: &str, out: &mut Vec<String>) {
    for d in dir.directories() {
        let path = format!("{prefix}{}", d.name);
        if d.mark_as_addon {
            out.push(path);
        } else {
            addon_paths(d, &format!("{path}/"), out);
        }
    }
}

fn assert_well_formed(dir: &TreeDirectory) {
    assert!(!dir.marked, "{} is marked", dir.name);
    let mut sorted = dir.clone();
    sorted.children.clear();
    for child in dir.children.clone() {
        sorted.add_child(child);
    }
    assert_eq!(
        sorted.children, dir.children,
        "{} is not in add_child order",
        dir.name
    );
    for d in dir.directories() {
        assert!(!d.children.is_empty(), "{} is empty", d.name);
        assert_well_formed(d);
    }
}

#[test]
fn group_from_the_wog_repository_has_one_leaf_per_addon_directory() {
    let sync: SyncTreeDirectory = a3s_repo_files::read_a3s(&fixture("wog_ftp/sync")).unwrap();
    let mut expected = Vec::new();
    addon_paths(&sync, "", &mut expected);
    assert!(!expected.is_empty());
    expected.sort();

    let group = group_from_repository(&sync, false);
    assert_eq!(group.name, REPOSITORY_GROUP_ROOT_NAME);
    let mut paths = leaf_paths(&group);
    paths.sort();
    assert_eq!(paths, expected);
    assert_well_formed(&group);

    let with_userconfig = group_from_repository(&sync, true);
    assert_well_formed(&with_userconfig);
    let userconfig = sync
        .directories()
        .find(|d| d.name.eq_ignore_ascii_case("userconfig"));
    match userconfig {
        Some(source) => {
            let made = with_userconfig
                .directories()
                .find(|d| d.name == source.name)
                .unwrap();
            assert_eq!(made.children.len(), source.children.len());
        }
        None => assert_eq!(with_userconfig, group),
    }
}

fn sync_dir(name: &str, mark_as_addon: bool, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        mark_as_addon,
        children,
        ..SyncTreeDirectory::new(name)
    })
}

fn sync_file(name: &str) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        ..Default::default()
    })
}

fn sync_root(children: Vec<SyncTreeNode>) -> SyncTreeDirectory {
    let mut root = SyncTreeDirectory::new_root();
    for child in children {
        root.add_child(child);
    }
    root
}

fn leaf(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf::new(name))
}

fn dir(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        children,
        ..TreeDirectory::new(name)
    })
}

fn group_root(name: &str, children: Vec<TreeNode>) -> TreeDirectory {
    TreeDirectory {
        children,
        ..TreeDirectory::new(name)
    }
}

#[test]
fn group_from_a_synthetic_repository() {
    let mut deleted = sync_dir("@a", true, vec![sync_dir("addons", false, vec![])]);
    let mut hidden = sync_dir("@h", true, vec![]);
    if let SyncTreeNode::Directory(d) = &mut deleted {
        d.deleted = true;
    }
    if let SyncTreeNode::Directory(d) = &mut hidden {
        d.hidden = true;
    }
    let sync = sync_root(vec![
        sync_file("readme.txt"),
        deleted,
        hidden,
        sync_dir("pack", false, vec![sync_dir("@n", true, vec![])]),
        sync_dir("empty", false, vec![sync_dir("x", false, vec![])]),
        sync_dir(
            "UserConfig",
            false,
            vec![sync_file("cfg.hpp"), sync_dir("ace", false, vec![])],
        ),
    ]);

    let expected = group_root(
        REPOSITORY_GROUP_ROOT_NAME,
        vec![dir("pack", vec![leaf("@n")]), leaf("@a"), leaf("@h")],
    );
    assert_eq!(group_from_repository(&sync, false), expected);

    let expected = group_root(
        REPOSITORY_GROUP_ROOT_NAME,
        vec![
            dir("pack", vec![leaf("@n")]),
            dir("UserConfig", vec![leaf("ace"), leaf("cfg.hpp")]),
            leaf("@a"),
            leaf("@h"),
        ],
    );
    assert_eq!(group_from_repository(&sync, true), expected);
}

#[test]
fn an_addon_folder_named_userconfig_shadows_the_userconfig_leaves() {
    let sync = sync_root(vec![sync_dir(
        "userconfig",
        false,
        vec![sync_dir("@u", true, vec![]), sync_file("other.hpp")],
    )]);
    let expected = group_root(
        REPOSITORY_GROUP_ROOT_NAME,
        vec![dir("userconfig", vec![leaf("@u")])],
    );
    assert_eq!(group_from_repository(&sync, true), expected);
}

fn modset(name: &str, kind: ModsetType, repository: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        modset_type: Some(kind),
        modset_repository_name: Some(repository.to_owned()),
        children,
        ..TreeDirectory::new(name)
    })
}

fn selected(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf {
        selected: true,
        ..TreeLeaf::new(name)
    })
}

fn find<'a>(root: &'a TreeDirectory, name: &str) -> &'a TreeDirectory {
    root.directories().find(|d| d.name == name).unwrap()
}

fn event(name: &str, addons: &[(&str, bool)], dlcs: &[(&str, bool)]) -> Event {
    let map = |items: &[(&str, bool)]| -> BTreeMap<String, bool> {
        items.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    };
    Event {
        name: name.to_owned(),
        description: None,
        addon_names: map(addons),
        dlc_names: map(dlcs),
        userconfig_folder_names: map(&[("cfg", false)]),
    }
}

#[test]
fn repository_groups_are_replaced_keeping_the_selection() {
    let mine = dir("Mine", vec![selected("@m")]);
    let mut root = group_root(
        "racine",
        vec![
            mine.clone(),
            modset(
                "Repo",
                ModsetType::Repository,
                "Repo",
                vec![selected("@a"), leaf("@b")],
            ),
        ],
    );
    let sync = sync_root(vec![
        sync_dir("@a", true, vec![]),
        sync_dir("@b", true, vec![]),
        sync_dir("@c", true, vec![]),
    ]);

    add_groups_from_repository(&mut root, &[("Repo", Some(&sync)), ("Gone", None)], true);

    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0], mine);
    let repo = find(&root, "Repo");
    assert_eq!(repo.modset_type, Some(ModsetType::Repository));
    assert_eq!(repo.modset_repository_name.as_deref(), Some("Repo"));
    assert!(repo.updated);
    assert!(!repo.selected);
    assert_eq!(repo.children, vec![selected("@a"), leaf("@b"), leaf("@c")]);
}

#[test]
fn event_groups_list_addons_and_dlcs() {
    let mut root = group_root("racine", vec![]);
    let ev = event(
        "Ev",
        &[("@a", false), ("@x", true), ("GM", true)],
        &[("GM", false)],
    );

    add_groups_from_events(&mut root, &[("Repo", &ev)], false);

    let group = find(&root, "Ev");
    assert_eq!(group.modset_type, Some(ModsetType::Event));
    assert_eq!(group.modset_repository_name.as_deref(), Some("Repo"));
    assert!(!group.updated);
    let optional_x = TreeNode::Leaf(TreeLeaf {
        optional: true,
        ..TreeLeaf::new("@x")
    });
    assert_eq!(group.children, vec![leaf("@a"), optional_x, leaf("GM")]);
}

#[test]
fn modsets_of_one_repository_are_refreshed() {
    let other = modset("Other", ModsetType::Event, "R2", vec![leaf("@o")]);
    let mut root = group_root(
        "racine",
        vec![
            modset("Repo", ModsetType::Repository, "Repo", vec![leaf("@old")]),
            modset("Ev", ModsetType::Event, "Repo", vec![selected("@first")]),
            other.clone(),
            dir("Mine", vec![leaf("@m")]),
        ],
    );
    let sync = sync_root(vec![sync_dir("@new", true, vec![])]);
    let events = [
        event("Ev", &[("@first", false), ("@more", false)], &[]),
        event("Ev", &[("@second", false)], &[]),
        event("Other", &[("@changed", false)], &[]),
    ];

    update_group_modsets(&mut root, "Repo", Some(&sync), &events);

    let repo = find(&root, "Repo");
    assert!(repo.updated);
    assert_eq!(repo.children, vec![leaf("@new")]);
    let ev = find(&root, "Ev");
    assert!(ev.updated);
    assert_eq!(ev.children, vec![selected("@first"), leaf("@more")]);
    assert_eq!(TreeNode::Directory(find(&root, "Other").clone()), other);
    assert_eq!(root.children.len(), 4);

    // Without a repository group, no repository group is created.
    let mut root = group_root("racine", vec![]);
    update_group_modsets(&mut root, "Repo", Some(&sync), &events);
    assert!(root.children.is_empty());
}

#[test]
fn normalize_keeps_the_first_of_each_name() {
    let marked = TreeNode::Directory(TreeDirectory {
        marked: true,
        optional: true,
        selected: true,
        children: vec![leaf("z")],
        ..TreeDirectory::new("d")
    });
    let mut root = group_root(
        "racine",
        vec![
            leaf("x"),
            dir("x", vec![leaf("lost")]),
            leaf("b"),
            marked,
            leaf("b"),
        ],
    );
    root.marked = true;

    normalize_group_tree(&mut root);

    let expected_d = TreeNode::Directory(TreeDirectory {
        selected: true,
        children: vec![leaf("z")],
        ..TreeDirectory::new("d")
    });
    assert_eq!(root.children, vec![expected_d, leaf("b"), leaf("x")]);
    assert!(root.marked, "the root's own fields are kept");
}

#[test]
fn set_selected_recursive_reaches_every_node() {
    let mut root = group_root("racine", vec![dir("d", vec![leaf("a")]), leaf("b")]);
    set_selected_recursive(&mut root, true);
    assert_eq!(
        root,
        TreeDirectory {
            selected: true,
            children: vec![
                TreeNode::Directory(TreeDirectory {
                    selected: true,
                    children: vec![selected("a")],
                    ..TreeDirectory::new("d")
                }),
                selected("b"),
            ],
            ..TreeDirectory::new("racine")
        }
    );
    set_selected_recursive(&mut root, false);
    assert_eq!(
        root,
        group_root("racine", vec![dir("d", vec![leaf("a")]), leaf("b")])
    );
}
