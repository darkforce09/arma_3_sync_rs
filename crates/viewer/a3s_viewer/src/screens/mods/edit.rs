//! Edits of the active profile behind the Mods screen: selection, groups, loose addons from
//! this computer, event presets and the launch order. Pure functions over the profile; the
//! screen saves the result.

use a3s_addons::{add_groups_from_events, set_selected_recursive};
use a3s_types::Event;
use a3s_types::java_string::compare_ignore_case;
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};

/// The node at `path` (child indices from the root).
pub(crate) fn node<'t>(root: &'t TreeDirectory, path: &[usize]) -> Option<&'t TreeNode> {
    let (first, rest) = path.split_first()?;
    let mut node = root.children.get(*first)?;
    for &index in rest {
        let TreeNode::Directory(directory) = node else {
            return None;
        };
        node = directory.children.get(index)?;
    }
    Some(node)
}

fn node_mut<'t>(root: &'t mut TreeDirectory, path: &[usize]) -> Option<&'t mut TreeNode> {
    let (first, rest) = path.split_first()?;
    let mut node = root.children.get_mut(*first)?;
    for &index in rest {
        let TreeNode::Directory(directory) = node else {
            return None;
        };
        node = directory.children.get_mut(index)?;
    }
    Some(node)
}

/// Whether every leaf below `directory` is selected (`false` when it has none).
pub(crate) fn all_leaves_selected(directory: &TreeDirectory) -> bool {
    let mut leaves = Vec::new();
    collect_leaf_selection(directory, &mut leaves);
    !leaves.is_empty() && leaves.iter().all(|selected| *selected)
}

fn collect_leaf_selection(directory: &TreeDirectory, out: &mut Vec<bool>) {
    for child in &directory.children {
        match child {
            TreeNode::Leaf(leaf) => out.push(leaf.selected),
            TreeNode::Directory(d) => collect_leaf_selection(d, out),
        }
    }
}

/// Flips the node at `path`: a leaf on its own, a directory with everything below it (a partly
/// selected directory becomes fully selected). The directories above follow: selected when all
/// their leaves are. Returns whether the node exists.
// Mirrors ui/main/AddonsPanel.java:877-918
pub(crate) fn toggle(root: &mut TreeDirectory, path: &[usize]) -> bool {
    let Some(node) = node_mut(root, path) else {
        return false;
    };
    match node {
        TreeNode::Leaf(leaf) => leaf.selected = !leaf.selected,
        TreeNode::Directory(directory) => {
            let value = !all_leaves_selected(directory);
            set_selected_recursive(directory, value);
        }
    }
    for depth in (1..path.len()).rev() {
        if let Some(TreeNode::Directory(directory)) = node_mut(root, &path[..depth]) {
            directory.selected = all_leaves_selected(directory);
        }
    }
    true
}

/// The index of the top-level leaf named `key` (ignoring case): an addon from this computer
/// enabled on its own, outside any group.
pub(crate) fn loose_leaf(root: &TreeDirectory, key: &str) -> Option<usize> {
    root.children.iter().position(|child| match child {
        TreeNode::Leaf(leaf) => compare_ignore_case(&leaf.name, key).is_eq(),
        TreeNode::Directory(_) => false,
    })
}

/// Whether the addon `key` from this computer is enabled on its own.
pub(crate) fn loose_enabled(root: &TreeDirectory, key: &str) -> bool {
    loose_leaf(root, key).is_some_and(|index| root.children[index].selected())
}

/// Enables (adds a selected top-level leaf) or disables (removes it) the addon `key` from this
/// computer. Returns whether anything changed.
pub(crate) fn set_loose(root: &mut TreeDirectory, key: &str, enabled: bool) -> bool {
    match (loose_leaf(root, key), enabled) {
        (Some(index), true) => {
            let changed = !root.children[index].selected();
            root.children[index].set_selected(true);
            changed
        }
        (Some(index), false) => {
            root.remove_child(index);
            true
        }
        (None, true) => {
            root.add_child(TreeNode::Leaf(TreeLeaf {
                selected: true,
                ..TreeLeaf::new(key)
            }));
            true
        }
        (None, false) => false,
    }
}

/// `base`, or `base 2`, `base 3`, … whichever no top-level node of `root` is named (ignoring
/// case).
fn unique_name(root: &TreeDirectory, base: &str) -> String {
    let taken = |name: &str| {
        root.children
            .iter()
            .any(|child| compare_ignore_case(child.name(), name).is_eq())
    };
    let mut name = base.to_owned();
    let mut number = 2;
    while taken(&name) {
        name = format!("{base} {number}");
        number += 1;
    }
    name
}

/// Adds an empty user group with a free name; returns the name.
pub(crate) fn add_group(root: &mut TreeDirectory) -> String {
    let name = unique_name(root, "New group");
    root.add_child(TreeNode::Directory(TreeDirectory::new(name.clone())));
    name
}

/// The index of the top-level group named `name`.
pub(crate) fn group_index(root: &TreeDirectory, name: &str) -> Option<usize> {
    root.children
        .iter()
        .position(|child| !child.is_leaf() && child.name() == name)
}

/// Renames the top-level group `old` to `new` (trimmed). Refused for an empty name or one
/// another top-level node has (ignoring case).
pub(crate) fn rename_group(root: &mut TreeDirectory, old: &str, new: &str) -> bool {
    let new = new.trim();
    if new.is_empty() || new == old {
        return false;
    }
    let clash = root
        .children
        .iter()
        .any(|child| child.name() != old && compare_ignore_case(child.name(), new).is_eq());
    let Some(index) = group_index(root, old) else {
        return false;
    };
    if clash {
        return false;
    }
    let Some(TreeNode::Directory(mut group)) = root.remove_child(index) else {
        return false;
    };
    new.clone_into(&mut group.name);
    root.add_child(TreeNode::Directory(group));
    true
}

/// Adds the addon `key` to the top-level group `group`, selected, unless it is there already.
pub(crate) fn add_to_group(root: &mut TreeDirectory, group: &str, key: &str) -> bool {
    let Some(index) = group_index(root, group) else {
        return false;
    };
    let TreeNode::Directory(directory) = &mut root.children[index] else {
        return false;
    };
    if directory
        .leaves()
        .any(|leaf| compare_ignore_case(&leaf.name, key).is_eq())
    {
        return false;
    }
    directory.add_child(TreeNode::Leaf(TreeLeaf {
        selected: true,
        ..TreeLeaf::new(key)
    }));
    directory.selected = all_leaves_selected(directory);
    true
}

/// Makes `event` of `repository` the selection: adds or refreshes its group, enables its
/// required addons (optional ones keep their previous choice when the group existed) and
/// disables everything else.
// Mirrors ui/main/AddonsPanel.java:1846-1883
pub(crate) fn apply_event_preset(root: &mut TreeDirectory, repository: &str, event: &Event) {
    add_groups_from_events(root, &[(repository, event)], false);
    for child in &mut root.children {
        match child {
            TreeNode::Directory(group) if group.name == event.name => {
                for leaf in &mut group.children {
                    if !leaf.optional() {
                        leaf.set_selected(true);
                    }
                }
                group.selected = all_leaves_selected(group);
            }
            TreeNode::Directory(group) => set_selected_recursive(group, false),
            TreeNode::Leaf(leaf) => leaf.selected = false,
        }
    }
}

/// Moves the enabled addons of the priority list into `enabled_order`, leaving every other
/// entry where it is: the slots the enabled addons held are refilled in the new order. Names
/// compare ignoring case; enabled addons missing from the list are appended. Returns whether
/// the list changed.
pub(crate) fn reorder_enabled(priority: &mut Vec<String>, enabled_order: &[String]) -> bool {
    let is_enabled = |name: &str| {
        enabled_order
            .iter()
            .any(|enabled| compare_ignore_case(enabled, name).is_eq())
    };
    let before = priority.clone();
    let mut next = enabled_order.iter();
    for entry in priority.iter_mut() {
        if is_enabled(entry)
            && let Some(name) = next.next()
        {
            name.clone_into(entry);
        }
    }
    priority.extend(next.cloned());
    *priority != before
}
