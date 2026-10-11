use std::collections::HashSet;

use a3s_types::local::{TreeDirectory, TreeNode};

/// Reorders `dir`'s children like repeated `TreeDirectory::add_child` calls in their current
/// order: directories first, then leaves, each stably sorted ignoring case.
pub(crate) fn sort_children(dir: &mut TreeDirectory) {
    let (mut directories, mut leaves): (Vec<_>, Vec<_>) = std::mem::take(&mut dir.children)
        .into_iter()
        .partition(|n| !n.is_leaf());
    directories.sort_by(TreeNode::java_cmp);
    leaves.sort_by(TreeNode::java_cmp);
    dir.children = directories;
    dir.children.append(&mut leaves);
}

/// Rebuilds the children of `dir` the way the Java client's profile round trip does.
///
/// The round trip is profile tree → display copy → profile tree: in every directory only the
/// first child of each exact name is kept (a later same-named sibling is dropped with its
/// subtree, leaf or directory alike), children are re-sorted in `add_child` order, and every
/// descendant directory loses `marked` and `optional`, which the display copy does not carry.
/// `dir`'s own fields are left as they are.
// Mirrors service/ObjectDTOtransformer.java:137-215 and service/ProfileService.java:251-288
pub fn normalize_group_tree(dir: &mut TreeDirectory) {
    let mut seen = HashSet::new();
    let children = std::mem::take(&mut dir.children);
    for child in children {
        if !seen.insert(child.name().to_owned()) {
            continue;
        }
        match child {
            TreeNode::Directory(mut d) => {
                d.marked = false;
                d.optional = false;
                normalize_group_tree(&mut d);
                dir.children.push(TreeNode::Directory(d));
            }
            leaf @ TreeNode::Leaf(_) => dir.children.push(leaf),
        }
    }
    sort_children(dir);
}

/// The paths of every selected node of `group` (the group itself and directories included),
/// each the `/`-joined names starting with `group.name`.
///
/// Paths are relative to `group`'s parent; capturing them before replacing a top-level group and
/// applying them to its replacement ([`apply_selected_paths`]) keeps the user's selection.
// Mirrors ui/main/AddonsPanel.java:1473-1492
pub fn selected_paths(group: &TreeDirectory) -> Vec<String> {
    fn visit(node: &TreeNode, prefix: &str, out: &mut Vec<String>) {
        let path = format!("{prefix}/{}", node.name());
        if node.selected() {
            out.push(path.clone());
        }
        if let TreeNode::Directory(d) = node {
            for child in &d.children {
                visit(child, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    if group.selected {
        out.push(group.name.clone());
    }
    for child in &group.children {
        visit(child, &group.name, &mut out);
    }
    out
}

/// Sets `selected` on every node of `group` (the group itself included) to whether its path,
/// built like [`selected_paths`], is in `paths`: anything not listed is deselected.
// Mirrors ui/main/AddonsPanel.java:1494-1515
pub fn apply_selected_paths(group: &mut TreeDirectory, paths: &[String]) {
    fn visit(node: &mut TreeNode, prefix: &str, paths: &[String]) {
        let path = format!("{prefix}/{}", node.name());
        node.set_selected(paths.contains(&path));
        if let TreeNode::Directory(d) = node {
            for child in &mut d.children {
                visit(child, &path, paths);
            }
        }
    }
    group.selected = paths.contains(&group.name);
    let name = group.name.clone();
    for child in &mut group.children {
        visit(child, &name, paths);
    }
}

/// Sets `selected` on `dir` and every descendant.
// Mirrors ui/main/AddonsPanel.java:887-907
pub fn set_selected_recursive(dir: &mut TreeDirectory, value: bool) {
    dir.selected = value;
    for child in &mut dir.children {
        match child {
            TreeNode::Directory(d) => set_selected_recursive(d, value),
            TreeNode::Leaf(l) => l.selected = value,
        }
    }
}
