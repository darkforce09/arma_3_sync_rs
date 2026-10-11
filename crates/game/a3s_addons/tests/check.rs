//! Resolution of group leaves, the missing / duplicate checks and `meta.cpp` Workshop ids.

use std::fs;
use std::path::Path;

use a3s_addons::{
    Addon, AddonRegistry, GroupStatus, LeafStatus, ScanOptions, check_duplicate_addons,
    check_duplicate_addons_selection, check_missing_addons, check_missing_selected_addons,
    find_addon_by_workshop_id, parse_published_id, refresh_group_status, resolve_addon_groups,
};
use a3s_types::constants::ModsetType;
use a3s_types::local::{Repository, TreeDirectory, TreeLeaf, TreeNode};
use a3s_types::{Protocol, ProtocolType};

const CASE_SENSITIVE: ScanOptions = ScanOptions {
    case_insensitive_paths: false,
};
const CASE_INSENSITIVE: ScanOptions = ScanOptions {
    case_insensitive_paths: true,
};

fn leaf(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf::new(name))
}

fn selected(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf {
        selected: true,
        ..TreeLeaf::new(name)
    })
}

fn dir(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        children,
        ..TreeDirectory::new(name)
    })
}

fn modset(name: &str, kind: ModsetType, repository: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        modset_type: Some(kind),
        modset_repository_name: Some(repository.to_owned()),
        children,
        ..TreeDirectory::new(name)
    })
}

fn root(children: Vec<TreeNode>) -> TreeDirectory {
    TreeDirectory {
        children,
        ..TreeDirectory::new("racine")
    }
}

/// The node at `path` (child indices).
fn node<'a>(root: &'a TreeDirectory, path: &[usize]) -> &'a TreeNode {
    let (first, rest) = path.split_first().unwrap();
    let mut node = &root.children[*first];
    for &i in rest {
        let TreeNode::Directory(d) = node else {
            panic!("not a directory")
        };
        node = &d.children[i];
    }
    node
}

fn leaf_at<'a>(root: &'a TreeDirectory, path: &[usize]) -> &'a TreeLeaf {
    match node(root, path) {
        TreeNode::Leaf(l) => l,
        TreeNode::Directory(d) => panic!("{} is a directory", d.name),
    }
}

fn repository(name: &str, default_location: &str, events: &[(&str, &str)]) -> Repository {
    let mut repo = Repository::new(
        name,
        Protocol::new(
            ProtocolType::Http,
            "example.org",
            "80",
            "anonymous",
            "",
            false,
        ),
    );
    repo.default_download_location = Some(default_location.to_owned());
    repo.events_download_location = events
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    repo
}

fn ace_registry() -> AddonRegistry {
    let mut registry = AddonRegistry::new();
    registry.insert(Addon::new("@ace", "@ace", "/dl/sub"));
    registry.insert(Addon::new("@ace*", "@ace", "/other"));
    registry
}

#[test]
fn modset_leaves_resolve_against_the_download_location() {
    let registry = ace_registry();
    let repos = [
        repository(
            "R",
            "/dl",
            &[("Ev", "/other"), ("Ev2", "/elsewhere/../other/.")],
        ),
        repository("Upper", "/dl", &[("Ev3", "/OTHER")]),
    ];
    let lookup = |name: &str| repos.iter().find(|r| r.name == name);
    let untouched = vec![
        dir("Plain", vec![leaf("@zz")]),
        modset("Unknown", ModsetType::Repository, "Nope", vec![leaf("@zz")]),
        dir(
            "Outer",
            vec![modset(
                "Inner",
                ModsetType::Repository,
                "R",
                vec![leaf("@zz")],
            )],
        ),
    ];
    let mut tree = root(
        [
            vec![
                modset("Ev", ModsetType::Event, "R", vec![leaf("@ace")]),
                modset("Ev2", ModsetType::Event, "R", vec![leaf("@ace")]),
                modset(
                    "R",
                    ModsetType::Repository,
                    "R",
                    vec![leaf("@ace"), leaf("@zz")],
                ),
                modset("Ev3", ModsetType::Event, "Upper", vec![leaf("@ace")]),
            ],
            untouched.clone(),
        ]
        .concat(),
    );
    let mut status = GroupStatus::default();

    resolve_addon_groups(&mut tree, &registry, lookup, &CASE_SENSITIVE, &mut status);

    assert_eq!(leaf_at(&tree, &[0, 0]).name, "@ace*");
    assert_eq!(leaf_at(&tree, &[1, 0]).name, "@ace*");
    assert_eq!(leaf_at(&tree, &[2, 0]).name, "@ace");
    assert!(!leaf_at(&tree, &[2, 0]).missing);
    let zz = leaf_at(&tree, &[2, 1]);
    assert_eq!((zz.name.as_str(), zz.missing), ("@zz", true));
    assert_eq!(
        status.leaf(&[2, 1]).unwrap().source_file_path.as_deref(),
        Some("/dl")
    );
    // Case-sensitive: `/OTHER` holds neither duplicate, so the name stays.
    assert_eq!(leaf_at(&tree, &[3, 0]).name, "@ace");
    assert_eq!(tree.children[4..], untouched[..]);
    assert!(status.leaf(&[4, 0]).is_none());

    let mut tree = root(vec![modset(
        "Ev3",
        ModsetType::Event,
        "Upper",
        vec![leaf("@ace")],
    )]);
    resolve_addon_groups(
        &mut tree,
        &registry,
        lookup,
        &CASE_INSENSITIVE,
        &mut GroupStatus::default(),
    );
    assert_eq!(leaf_at(&tree, &[0, 0]).name, "@ace*");
}

fn write_meta(dir: &Path, content: &[u8]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("meta.cpp"), content).unwrap();
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn missing_addons_resolve_by_workshop_id_dlc_or_name() {
    let temp = tempfile::tempdir().unwrap();
    let mods = temp.path().join("mods");
    write_meta(
        &mods.join("@ace"),
        b"name = \"ACE\";\npublishedid = 463939057;\n",
    );
    let mut registry = AddonRegistry::new();
    registry.insert(Addon::new("@ace", "@ace", s(&mods)));
    registry.insert(Addon::new("GM*", "GM", "/arma"));

    let workshop = TreeNode::Leaf(TreeLeaf {
        workshop_id: Some("463939057".to_owned()),
        ..TreeLeaf::new("ACE3")
    });
    let dlc = TreeNode::Leaf(TreeLeaf {
        dlc_app_id: Some("1042220".to_owned()),
        ..TreeLeaf::new("Global Mobilization")
    });
    let unknown_dlc = TreeNode::Leaf(TreeLeaf {
        dlc_app_id: Some("01042220".to_owned()),
        ..TreeLeaf::new("Old DLC")
    });
    let already_missing = TreeNode::Leaf(TreeLeaf {
        missing: true,
        ..TreeLeaf::new("@gone")
    });
    let blank_id = TreeNode::Leaf(TreeLeaf {
        workshop_id: Some(" \t".to_owned()),
        ..TreeLeaf::new("@ace")
    });
    let mut tree = root(vec![dir(
        "G",
        vec![
            workshop,
            dlc,
            unknown_dlc,
            already_missing,
            blank_id,
            leaf("@new"),
        ],
    )]);
    let mut newly_missing = Vec::new();
    let mut status = GroupStatus::default();

    check_missing_addons(&mut tree, &registry, &mut newly_missing, &mut status);

    let names_missing: Vec<(&str, bool)> = (0..6)
        .map(|i| {
            let l = leaf_at(&tree, &[0, i]);
            (l.name.as_str(), l.missing)
        })
        .collect();
    assert_eq!(
        names_missing,
        [
            ("@ace", false),
            ("GM", false),
            ("Old DLC", true),
            ("@gone", true),
            ("@ace", false),
            ("@new", true),
        ]
    );
    assert_eq!(newly_missing, ["Old DLC", "@new"]);
}

#[test]
fn a_workshop_id_without_meta_cpp_does_not_fall_back_to_the_name() {
    let temp = tempfile::tempdir().unwrap();
    let mods = temp.path().join("mods");
    fs::create_dir_all(mods.join("@ace")).unwrap();
    let mut registry = AddonRegistry::new();
    registry.insert(Addon::new("@ace", "@ace", s(&mods)));
    let mut tree = root(vec![TreeNode::Leaf(TreeLeaf {
        workshop_id: Some("463939057".to_owned()),
        ..TreeLeaf::new("@ace")
    })]);
    let mut newly_missing = Vec::new();

    check_missing_addons(
        &mut tree,
        &registry,
        &mut newly_missing,
        &mut GroupStatus::default(),
    );

    assert!(leaf_at(&tree, &[0]).missing);
    assert_eq!(newly_missing, ["@ace"]);
}

#[test]
fn workshop_lookup_reads_meta_cpp_in_registry_order() {
    let temp = tempfile::tempdir().unwrap();
    let mods = temp.path().join("mods");
    write_meta(&mods.join("@bad"), b"publishedid = 5;\xff\n");
    write_meta(&mods.join("@one"), b"publishedid = 5;\n");
    write_meta(&mods.join("@two"), b"publishedid = 5;\n");
    let mut registry = AddonRegistry::new();
    for name in ["@missing", "@bad", "@one", "@two"] {
        registry.insert(Addon::new(name, name, s(&mods)));
    }
    assert_eq!(
        find_addon_by_workshop_id(&registry, "5").map(|a| a.key.as_str()),
        Some("@one")
    );
    assert!(find_addon_by_workshop_id(&registry, "6").is_none());
}

#[test]
fn published_id_follows_the_java_regex() {
    let cases: [(&str, Option<&str>); 12] = [
        ("publishedid = 123;", Some("123")),
        ("  publishedid=123 ;\r\n", Some("123")),
        ("name=\"x\";\npublishedid =\n 77\n;", Some("77")),
        ("publishedid = 0;\npublishedid = 5;", Some("5")),
        ("publishedid = 1;\npublishedid = 5;", Some("1")),
        ("x publishedid = 9;", None),
        ("PublishedId = 9;", None),
        ("publishedid = 012;", None),
        ("publishedid = 12a;", None),
        ("\u{2028}publishedid = 4;", Some("4")),
        ("a\r\n\tpublishedid = 8;", Some("8")),
        ("\u{a0}publishedid = 4;", None),
    ];
    for (text, expected) in cases {
        assert_eq!(parse_published_id(text), expected, "{text:?}");
    }
}

#[test]
fn missing_selected_addons_block_the_launch() {
    let registry = ace_registry();
    let mut tree = root(vec![dir(
        "G",
        vec![selected("@ace"), selected("@gone"), leaf("@gone2")],
    )]);
    assert_eq!(
        check_missing_selected_addons(&mut tree, &registry),
        ["@gone"]
    );
    assert!(!leaf_at(&tree, &[0, 0]).missing);
    assert!(leaf_at(&tree, &[0, 1]).missing);
    assert!(leaf_at(&tree, &[0, 2]).missing);
}

#[test]
fn duplicated_selection_lists_each_occurrence() {
    let mut tree = root(vec![
        dir("G1", vec![selected("@a"), selected("@b")]),
        dir("G2", vec![selected("@a"), leaf("@b"), leaf("@c")]),
        dir("G3", vec![leaf("@c")]),
    ]);
    let mut status = GroupStatus::default();
    assert_eq!(
        check_duplicate_addons_selection(&tree, &mut status),
        ["@a", "@a"]
    );
    let flagged: Vec<bool> = [[0, 0], [0, 1], [1, 0], [1, 1], [1, 2], [2, 0]]
        .iter()
        .map(|p| status.leaf(p).unwrap().duplicated_selection)
        .collect();
    assert_eq!(flagged, [true, false, true, false, false, false]);

    // Selection changes are picked up on the next check.
    if let TreeNode::Directory(d) = &mut tree.children[1] {
        d.children[0].set_selected(false);
    }
    assert!(check_duplicate_addons_selection(&tree, &mut status).is_empty());
    assert!(!status.leaf(&[0, 0]).unwrap().duplicated_selection);
}

#[test]
fn duplicates_point_at_the_available_addon() {
    let mut registry = ace_registry();
    registry.insert(Addon::new("@solo", "@solo", "/dl"));
    let available = TreeDirectory {
        children: vec![dir("sub", vec![leaf("@ACE")]), leaf("@ace*"), leaf("@solo")],
        ..TreeDirectory::new("racine1")
    };
    let tree = root(vec![dir(
        "G",
        vec![leaf("@ace"), leaf("@ace*"), leaf("@solo")],
    )]);
    let mut status = GroupStatus::default();

    check_duplicate_addons(&tree, &registry, &available, &mut status);

    let expect = |duplicate, path: Option<&str>| LeafStatus {
        duplicate,
        source_relative_path: path.map(str::to_owned),
        ..LeafStatus::default()
    };
    assert_eq!(status.leaf(&[0, 0]), Some(&expect(true, Some("sub"))));
    assert_eq!(status.leaf(&[0, 1]), Some(&expect(true, None)));
    assert_eq!(status.leaf(&[0, 2]), Some(&expect(false, None)));
}

#[test]
fn refresh_runs_the_checks_in_java_order() {
    let registry = ace_registry();
    let repos = [repository("R", "/dl", &[("Ev", "/other")])];
    let lookup = |name: &str| repos.iter().find(|r| r.name == name);
    let available = TreeDirectory {
        children: vec![dir("sub", vec![leaf("@ace")]), leaf("@ace*")],
        ..TreeDirectory::new("racine1")
    };
    let tree = root(vec![
        modset("Ev", ModsetType::Event, "R", vec![selected("@ace")]),
        dir("Mine", vec![selected("@ace*"), selected("@gone")]),
    ]);

    let mut refreshed = tree.clone();
    let status = refresh_group_status(
        &mut refreshed,
        &registry,
        lookup,
        &available,
        &CASE_SENSITIVE,
    );

    let mut manual = tree;
    let mut expected = GroupStatus::default();
    resolve_addon_groups(
        &mut manual,
        &registry,
        lookup,
        &CASE_SENSITIVE,
        &mut expected,
    );
    check_missing_addons(&mut manual, &registry, &mut Vec::new(), &mut expected);
    check_duplicate_addons(&manual, &registry, &available, &mut expected);
    let duplicated = check_duplicate_addons_selection(&manual, &mut expected);

    assert_eq!(refreshed, manual);
    assert_eq!(status, expected);
    assert_eq!(leaf_at(&refreshed, &[0, 0]).name, "@ace*");
    assert!(leaf_at(&refreshed, &[1, 1]).missing);
    assert_eq!(duplicated, ["@ace*", "@ace*"]);
    assert!(status.leaf(&[0, 0]).unwrap().duplicated_selection);
}
