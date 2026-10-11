//! Restricts a plan to the addons and userconfig folders of one event (modset).
//!
//! Port of `service/EventExtractor`. The filtered tree keeps the relative paths of every kept
//! node: ancestors of a kept addon folder are re-created with the same names (and default
//! flags), so remote paths stay valid.

use std::collections::BTreeMap;

use a3s_types::{Event, Events};

use crate::model::{PlanDirectory, PlanNode, PlanTree, ROOT_NAME};

/// The event named exactly `name` (case-sensitive). With duplicate names the **last** one wins:
/// the Java loop has no `break`.
// Mirrors service/EventExtractor.java:30-35
pub fn find_event<'a>(events: &'a Events, name: &str) -> Option<&'a Event> {
    events.list.iter().rev().find(|e| e.name == name)
}

/// The part of `tree` that belongs to `event`, in a new root.
///
/// * Userconfig: the first root folder named `userconfig` (any case) is re-created with only
///   its sub-folders listed in [`Event::userconfig_folder_names`] (case-sensitive), each with
///   `optional` set from the map. Files in it are dropped.
/// * Addons: every folder marked as addon whose name is in [`Event::addon_names`]
///   (case-sensitive) is moved with its whole subtree and `optional` set from the map. Plain
///   folders containing such an addon (at any depth) are re-created with default flags. All
///   other nodes are dropped.
///
/// An event whose maps are both empty gives an empty root.
// Mirrors service/EventExtractor.java:26-49
pub fn filter(tree: PlanTree, event: &Event) -> PlanTree {
    let mut old_children = tree.root.children;
    let mut new_root = PlanDirectory::new(ROOT_NAME);
    if !event.userconfig_folder_names.is_empty() {
        refine_userconfig(
            &mut old_children,
            &mut new_root,
            &event.userconfig_folder_names,
        );
    }
    if !event.addon_names.is_empty() {
        refine_addons(old_children, &mut new_root, &event.addon_names);
    }
    PlanTree { root: new_root }
}

/// Moves the listed addon folders of `old_children` into `new_parent`, re-creating the plain
/// folders that lead to them.
// Mirrors service/EventExtractor.java:51-74
fn refine_addons(
    old_children: Vec<PlanNode>,
    new_parent: &mut PlanDirectory,
    addon_names: &BTreeMap<String, bool>,
) {
    for node in old_children {
        // Files are dropped.
        let PlanNode::Directory(mut directory) = node else {
            continue;
        };
        if directory.mark_as_addon {
            // An addon folder that is not in the event is dropped without descending.
            if let Some(&optional) = addon_names.get(&directory.name) {
                directory.optional = optional;
                new_parent.add_child(PlanNode::Directory(directory));
            }
        } else if seek(&directory, addon_names) {
            let children = std::mem::take(&mut directory.children);
            let synthesized = synthesized_directory(new_parent, &directory.name);
            refine_addons(children, synthesized, addon_names);
        }
    }
}

/// The directory child of `parent` named `name`, created with default flags if missing.
///
/// Java adds a duplicate sibling when the name already exists (only possible for `userconfig`
/// when it contains a listed addon folder); the two are merged here (approved deviation DV-P10).
fn synthesized_directory<'a>(parent: &'a mut PlanDirectory, name: &str) -> &'a mut PlanDirectory {
    let is_named = |c: &PlanNode| matches!(c, PlanNode::Directory(d) if d.name == name);
    if !parent.children.iter().any(is_named) {
        parent.add_child(PlanNode::Directory(PlanDirectory::new(name)));
    }
    let index = parent
        .children
        .iter()
        .position(is_named)
        .expect("the directory was just added");
    match &mut parent.children[index] {
        PlanNode::Directory(d) => d,
        PlanNode::Leaf(_) => unreachable!("`is_named` only matches directories"),
    }
}

/// Whether a listed addon folder exists anywhere below `directory`.
///
/// Descends into addon folders that are not listed too, so an event addon nested inside an
/// unlisted addon makes this true although [`refine_addons`] never moves it: an empty
/// re-created folder then remains in the filtered tree (Java quirk kept).
// Mirrors service/EventExtractor.java:76-91
fn seek(directory: &PlanDirectory, addon_names: &BTreeMap<String, bool>) -> bool {
    directory.children.iter().any(|node| match node {
        PlanNode::Directory(d) => {
            (d.mark_as_addon && addon_names.contains_key(&d.name)) || seek(d, addon_names)
        }
        PlanNode::Leaf(_) => false,
    })
}

/// Re-creates the first root folder named `userconfig` (any case) in `new_root`, moving into it
/// the listed sub-folders (which are removed from `old_children`).
// Mirrors service/EventExtractor.java:93-119
fn refine_userconfig(
    old_children: &mut [PlanNode],
    new_root: &mut PlanDirectory,
    userconfig_folder_names: &BTreeMap<String, bool>,
) {
    let userconfig = old_children.iter_mut().find_map(|node| match node {
        PlanNode::Directory(d) if d.name.to_lowercase() == "userconfig" => Some(d),
        _ => None,
    });
    let Some(userconfig) = userconfig else {
        return;
    };
    let mut new_userconfig = PlanDirectory::new(userconfig.name.clone());
    // Java shares the moved nodes between the old and the new tree; moving them out keeps the
    // addon pass from seeing them a second time.
    let (moved, kept): (Vec<PlanNode>, Vec<PlanNode>) = std::mem::take(&mut userconfig.children)
        .into_iter()
        .partition(|node| {
            matches!(node, PlanNode::Directory(d) if userconfig_folder_names.contains_key(&d.name))
        });
    userconfig.children = kept;
    for node in moved {
        if let PlanNode::Directory(mut d) = node {
            d.optional = userconfig_folder_names
                .get(&d.name)
                .copied()
                .unwrap_or(false);
            new_userconfig.add_child(PlanNode::Directory(d));
        }
    }
    new_root.add_child(PlanNode::Directory(new_userconfig));
}

#[cfg(test)]
mod tests {
    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use super::*;

    fn leaf(name: &str) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: Some("abc".to_owned()),
            size: 1,
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

    fn tree(children: Vec<SyncTreeNode>) -> PlanTree {
        let mut root = SyncTreeDirectory::new_root();
        root.children = children;
        root.sort_children();
        PlanTree::from_sync_tree(&root)
    }

    fn event(name: &str, addons: &[(&str, bool)], userconfig: &[(&str, bool)]) -> Event {
        Event {
            name: name.to_owned(),
            addon_names: addons.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            userconfig_folder_names: userconfig
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect(),
            ..Default::default()
        }
    }

    fn paths(tree: &PlanTree) -> Vec<String> {
        let mut out = Vec::new();
        tree.walk(&mut |path, _| out.push(path.to_owned()));
        out
    }

    fn directory<'a>(tree: &'a PlanTree, path: &str) -> &'a PlanDirectory {
        match tree.get(path) {
            Some(PlanNode::Directory(d)) => d,
            other => panic!("{path}: {other:?}"),
        }
    }

    fn addon_tree() -> PlanTree {
        tree(vec![
            dir(
                "@a",
                true,
                vec![dir("addons", false, vec![leaf("a.pbo"), leaf("b.pbo")])],
            ),
            dir("@b", true, vec![dir("addons", false, vec![leaf("x.pbo")])]),
            leaf("readme.txt"),
            dir(
                "mods",
                false,
                vec![
                    dir("@c", true, vec![dir("addons", false, vec![leaf("c.pbo")])]),
                    leaf("x.txt"),
                ],
            ),
            dir(
                "mods2",
                false,
                vec![dir(
                    "@d",
                    true,
                    vec![dir("@e", true, vec![dir("addons", false, vec![])])],
                )],
            ),
        ])
    }

    #[test]
    fn filter_keeps_listed_addons() {
        let original = addon_tree();
        let ev = event("e", &[("@a", true), ("@c", false), ("@e", true)], &[]);
        let filtered = filter(original.clone(), &ev);
        assert_eq!(filtered.root.name, ROOT_NAME);
        assert_eq!(
            paths(&filtered),
            [
                "@a",
                "@a/addons",
                "@a/addons/a.pbo",
                "@a/addons/b.pbo",
                "mods",
                "mods/@c",
                "mods/@c/addons",
                "mods/@c/addons/c.pbo",
                "mods2",
            ]
        );
        let a = directory(&filtered, "@a");
        assert!(a.optional && a.mark_as_addon);
        // The subtree of a moved addon is intact (flags included).
        let PlanNode::Directory(original_a) = original.get("@a").expect("@a") else {
            panic!("@a is a directory")
        };
        assert_eq!(a.children, original_a.children);

        let mods = directory(&filtered, "mods");
        assert_eq!(mods.children.len(), 1);
        assert!(!mods.mark_as_addon && !mods.optional && !mods.updated);
        let c = directory(&filtered, "mods/@c");
        assert!(!c.optional && c.mark_as_addon);

        // `seek` finds `@e` inside the unlisted `@d`, which is never moved: empty `mods2`.
        assert!(directory(&filtered, "mods2").children.is_empty());
        assert!(filtered.get("@b").is_none());
        assert!(filtered.get("readme.txt").is_none());

        // Relative paths are unchanged.
        for path in paths(&filtered) {
            assert_eq!(
                original.get(&path).map(PlanNode::name),
                filtered.get(&path).map(PlanNode::name),
                "{path}"
            );
        }
    }

    #[test]
    fn filter_is_case_sensitive() {
        let filtered = filter(addon_tree(), &event("e", &[("@A", true)], &[]));
        assert!(filtered.root.children.is_empty());
    }

    #[test]
    fn filter_userconfig() {
        let t = tree(vec![
            dir(
                "UserConfig",
                false,
                vec![
                    dir("ace", false, vec![leaf("ace.hpp")]),
                    dir("tfar", false, vec![]),
                    leaf("cfg.hpp"),
                ],
            ),
            dir("@a", true, vec![]),
        ]);
        let filtered = filter(t.clone(), &event("e", &[], &[("ace", false)]));
        assert_eq!(
            paths(&filtered),
            ["UserConfig", "UserConfig/ace", "UserConfig/ace/ace.hpp"]
        );
        assert!(!directory(&filtered, "UserConfig/ace").optional);
        assert!(!directory(&filtered, "UserConfig").mark_as_addon);

        let filtered = filter(t.clone(), &event("e", &[], &[("ace", true)]));
        assert!(directory(&filtered, "UserConfig/ace").optional);

        // Nothing listed is present: the folder is still re-created.
        let filtered = filter(t, &event("e", &[], &[("nope", true)]));
        assert_eq!(paths(&filtered), ["UserConfig"]);

        // No userconfig folder on the root.
        let t = tree(vec![dir(
            "mods",
            false,
            vec![dir("userconfig", false, vec![dir("ace", false, vec![])])],
        )]);
        let filtered = filter(t, &event("e", &[], &[("ace", true)]));
        assert!(filtered.root.children.is_empty());
    }

    #[test]
    fn filter_userconfig_and_addons_merge() {
        let t = tree(vec![
            dir(
                "userconfig",
                false,
                vec![
                    dir("ace", false, vec![]),
                    dir("@in_uc", true, vec![leaf("x")]),
                ],
            ),
            dir("@a", true, vec![]),
        ]);
        let ev = event("e", &[("@a", false), ("@in_uc", true)], &[("ace", true)]);
        let filtered = filter(t, &ev);
        // Java would create two `userconfig` siblings; they are merged (DV-P10).
        assert_eq!(
            paths(&filtered),
            [
                "@a",
                "userconfig",
                "userconfig/@in_uc",
                "userconfig/@in_uc/x",
                "userconfig/ace",
            ]
        );
        assert_eq!(directory(&filtered, "userconfig").children.len(), 2);
        assert!(directory(&filtered, "userconfig/@in_uc").optional);
        assert!(directory(&filtered, "userconfig/ace").optional);
    }

    #[test]
    fn find_event_last_duplicate_wins() {
        let events = Events {
            list: vec![
                event("dup", &[("@a", true)], &[]),
                event("other", &[("@b", true)], &[]),
                event("dup", &[("@b", true)], &[]),
            ],
        };
        let found = find_event(&events, "dup").expect("event");
        assert!(found.addon_names.contains_key("@b"));
        assert!(find_event(&events, "DUP").is_none());
        let filtered = filter(addon_tree(), found);
        assert_eq!(paths(&filtered), ["@b", "@b/addons", "@b/addons/x.pbo"]);
    }

    #[test]
    fn filter_children_order() {
        let t = tree(vec![
            dir("@z", true, vec![]),
            dir("Zeta", false, vec![dir("@y", true, vec![])]),
            dir("@B", true, vec![]),
            dir("alpha", false, vec![dir("@x", true, vec![])]),
        ]);
        let ev = event(
            "e",
            &[("@z", false), ("@y", false), ("@B", false), ("@x", false)],
            &[],
        );
        let filtered = filter(t, &ev);
        let names: Vec<_> = filtered.root.children.iter().map(PlanNode::name).collect();
        assert_eq!(names, ["@B", "@z", "alpha", "Zeta"]);
    }
}
