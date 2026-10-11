//! Resolution of addon-group leaves against the registry and the checks shown on the groups:
//! missing addons, duplicates and duplicated selection.
//!
//! The functions mutate the tree in place like the Java display copy: a leaf renamed to another
//! registry key keeps its position. Persist a [`crate::normalize_group_tree`]ed clone, as Java's
//! `setAddonGroups` does.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use a3s_types::constants::{GameDlc, ModsetType};
use a3s_types::java_string::compare_ignore_case;
use a3s_types::local::{Addon, Repository, TreeDirectory, TreeLeaf, TreeNode};

use crate::registry::AddonRegistry;
use crate::scan::{ScanOptions, absolute};
use crate::workshop::find_addon_by_workshop_id;

/// Child indices from the root of a group tree down to a node. Valid until the tree is
/// mutated.
pub type NodePath = Vec<usize>;

/// Per-leaf check results the profile tree does not store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeafStatus {
    /// Another available addon has the same name.
    pub duplicate: bool,

    /// The leaf's name is selected more than once in the groups.
    pub duplicated_selection: bool,

    /// For a duplicate: the folders (`/`-joined) of the same-named addon in the available-addons
    /// tree; `None` at its root.
    pub source_relative_path: Option<String>,

    /// For a missing modset addon: the download location it was expected in.
    pub source_file_path: Option<String>,
}

/// The [`LeafStatus`] of the leaves of one group tree, by [`NodePath`]. Recompute it after
/// every change to the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupStatus {
    pub leaves: BTreeMap<NodePath, LeafStatus>,
}

impl GroupStatus {
    pub fn leaf(&self, path: &[usize]) -> Option<&LeafStatus> {
        self.leaves.get(path)
    }

    fn entry(&mut self, path: &[usize]) -> &mut LeafStatus {
        self.leaves.entry(path.to_vec()).or_default()
    }
}

fn for_each_leaf_mut(
    dir: &mut TreeDirectory,
    path: &mut NodePath,
    f: &mut impl FnMut(&[usize], &mut TreeLeaf),
) {
    for (i, child) in dir.children.iter_mut().enumerate() {
        path.push(i);
        match child {
            TreeNode::Leaf(leaf) => f(path, leaf),
            TreeNode::Directory(d) => for_each_leaf_mut(d, path, f),
        }
        path.pop();
    }
}

fn for_each_leaf(
    dir: &TreeDirectory,
    path: &mut NodePath,
    f: &mut impl FnMut(&[usize], &TreeLeaf),
) {
    for (i, child) in dir.children.iter().enumerate() {
        path.push(i);
        match child {
            TreeNode::Leaf(leaf) => f(path, leaf),
            TreeNode::Directory(d) => for_each_leaf(d, path, f),
        }
        path.pop();
    }
}

/// `path` made absolute and lexically normalised (`.` removed, `name/..` collapsed, no link
/// resolution), lowercased when comparing ignoring case (Java `Path.toAbsolutePath().normalize()`).
fn normalized_path(path: &str, opts: &ScanOptions) -> PathBuf {
    let path = if path.is_empty() { "." } else { path };
    let mut out = PathBuf::new();
    for component in absolute(Path::new(path)).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    PathBuf::from(opts.compare_form(&out.to_string_lossy()))
}

/// Whether `candidate` is `location` or inside it, comparing whole components.
// Mirrors service/AddonService.java:517-529
fn is_same_or_within(candidate: &str, location: &str, opts: &ScanOptions) -> bool {
    normalized_path(candidate, opts).starts_with(normalized_path(location, opts))
}

/// Resolves the leaves of every top-level modset group against its download location.
///
/// The repository is looked up by name with `repository` (unknown repositories are skipped); an
/// event group uses the event's own location when one is set, otherwise the repository's
/// default location. Other groups and nested modset directories are left alone.
///
/// A leaf whose name has duplicates is renamed to the key of the first duplicate (in registry
/// order) inside the location. A leaf with no addon of its name is marked missing and its
/// status records the location.
// Mirrors service/AddonService.java:419-515
pub fn resolve_addon_groups<'r>(
    root: &mut TreeDirectory,
    registry: &AddonRegistry,
    repository: impl Fn(&str) -> Option<&'r Repository>,
    opts: &ScanOptions,
    status: &mut GroupStatus,
) {
    for (i, child) in root.children.iter_mut().enumerate() {
        let TreeNode::Directory(dir) = child else {
            continue;
        };
        let Some(kind) = dir.modset_type else {
            continue;
        };
        let Some(repo) = dir.modset_repository_name.as_deref().and_then(&repository) else {
            continue;
        };
        let location = match kind {
            ModsetType::Repository => repo.default_download_location.as_ref(),
            ModsetType::Event => repo
                .events_download_location
                .get(&dir.name)
                .or(repo.default_download_location.as_ref()),
        };
        let Some(location) = location else {
            continue;
        };
        for_each_leaf_mut(dir, &mut vec![i], &mut |path, leaf| {
            let found = if registry.has_duplicate(&leaf.name) {
                let key = registry
                    .duplicates(&leaf.name)
                    .into_iter()
                    .filter_map(|key| registry.get(&key))
                    .find(|addon| is_same_or_within(&addon.path, location, opts))
                    .map(|addon| addon.key.clone());
                if let Some(key) = key {
                    leaf.name = key;
                    true
                } else {
                    false
                }
            } else {
                registry
                    .get(&leaf.name)
                    .is_some_and(|addon| is_same_or_within(&addon.path, location, opts))
            };
            let entry = status.entry(path);
            if found || registry.contains(&leaf.name) {
                leaf.missing = false;
                entry.source_file_path = None;
            } else {
                leaf.missing = true;
                entry.source_file_path = Some(location.clone());
            }
        });
    }
}

/// Whether `s` is non-empty and not only whitespace (Java `!String.isBlank()`).
fn non_blank(s: Option<&str>) -> Option<&str> {
    let is_java_whitespace = |c: char| {
        (c.is_whitespace() && !matches!(c, '\u{85}' | '\u{A0}' | '\u{2007}' | '\u{202F}'))
            || ('\u{1C}'..='\u{1F}').contains(&c)
    };
    s.filter(|s| !s.chars().all(is_java_whitespace))
}

/// Marks every leaf missing or present.
///
/// A leaf with a Workshop id is resolved through the mods'
/// `meta.cpp` and renamed to the found addon's key (no fallback to its name); a leaf with a DLC
/// app id is resolved to the DLC's addon and renamed to the DLC's Java name; any other leaf is
/// looked up by name. Names of leaves that become missing are appended to `newly_missing`.
// Mirrors service/AddonService.java:531-569 and :592-603
pub fn check_missing_addons(
    root: &mut TreeDirectory,
    registry: &AddonRegistry,
    newly_missing: &mut Vec<String>,
    status: &mut GroupStatus,
) {
    for_each_leaf_mut(root, &mut Vec::new(), &mut |path, leaf| {
        let was_missing = leaf.missing;
        let resolved: Option<&Addon> = if let Some(id) = non_blank(leaf.workshop_id.as_deref()) {
            let addon = find_addon_by_workshop_id(registry, id);
            if let Some(addon) = addon {
                leaf.name.clone_from(&addon.key);
            }
            addon
        } else if let Some(id) = non_blank(leaf.dlc_app_id.as_deref()) {
            GameDlc::from_steam_app_id(id).and_then(|dlc| {
                let addon = registry
                    .addons()
                    .find(|a| compare_ignore_case(dlc.java_name(), &a.name).is_eq());
                if addon.is_some() {
                    dlc.java_name().clone_into(&mut leaf.name);
                }
                addon
            })
        } else {
            registry.get(&leaf.name)
        };
        if resolved.is_some() {
            leaf.missing = false;
            status.entry(path).source_file_path = None;
        } else {
            leaf.missing = true;
            if !was_missing {
                newly_missing.push(leaf.name.clone());
            }
        }
    });
}

/// Returns the names of the selected missing leaves; a non-empty list blocks the launch.
///
/// Leaves with no addon of their name are marked missing first. Pass a clone to keep the extra
/// `missing` flags out of the profile tree, as Java does.
// Mirrors service/AddonService.java:605-624
pub fn check_missing_selected_addons(
    root: &mut TreeDirectory,
    registry: &AddonRegistry,
) -> Vec<String> {
    let mut out = Vec::new();
    for_each_leaf_mut(root, &mut Vec::new(), &mut |_, leaf| {
        if !leaf.missing && !registry.contains(&leaf.name) {
            leaf.missing = true;
        }
        if leaf.missing && leaf.selected {
            out.push(leaf.name.clone());
        }
    });
    out
}

/// The folders above the last leaf of `tree` named `name` ignoring case, skipping any name
/// containing `racine` (empty at the root); `None` when there is no such leaf.
// Mirrors service/AddonService.java:645-670
fn source_relative_path(tree: &TreeDirectory, name: &str) -> Option<Vec<String>> {
    fn visit<'a>(
        dir: &'a TreeDirectory,
        ancestors: &mut Vec<&'a str>,
        name: &str,
        found: &mut Option<Vec<String>>,
    ) {
        ancestors.push(&dir.name);
        for child in &dir.children {
            match child {
                TreeNode::Leaf(leaf) if compare_ignore_case(&leaf.name, name).is_eq() => {
                    *found = Some(
                        ancestors
                            .iter()
                            .filter(|n| !n.contains("racine"))
                            .map(|n| (*n).to_owned())
                            .collect(),
                    );
                }
                TreeNode::Leaf(_) => {}
                TreeNode::Directory(d) => visit(d, ancestors, name, found),
            }
        }
        ancestors.pop();
    }
    let mut found = None;
    visit(tree, &mut Vec::new(), name, &mut found);
    found
}

/// Records for every leaf whether another available addon has its name and, if so, where the
/// same-named addon sits in `available_tree` (the scan's tree).
// Mirrors service/AddonService.java:626-643
pub fn check_duplicate_addons(
    root: &TreeDirectory,
    registry: &AddonRegistry,
    available_tree: &TreeDirectory,
    status: &mut GroupStatus,
) {
    for_each_leaf(root, &mut Vec::new(), &mut |path, leaf| {
        let duplicate = registry.has_duplicate(&leaf.name);
        let entry = status.entry(path);
        entry.duplicate = duplicate;
        if !duplicate {
            entry.source_relative_path = None;
        } else if let Some(folders) = source_relative_path(available_tree, &leaf.name) {
            entry.source_relative_path = (!folders.is_empty()).then(|| folders.join("/"));
        }
    });
}

/// Returns every selected leaf name that is selected more than once, once per occurrence (a
/// non-empty list blocks the launch), and flags the leaves carrying those names.
// Mirrors service/AddonService.java:672-730
pub fn check_duplicate_addons_selection(
    root: &TreeDirectory,
    status: &mut GroupStatus,
) -> Vec<String> {
    let mut selected = Vec::new();
    for_each_leaf(root, &mut Vec::new(), &mut |_, leaf| {
        if leaf.selected {
            selected.push(leaf.name.clone());
        }
    });
    let duplicated: Vec<String> = selected
        .iter()
        .filter(|name| selected.iter().filter(|n| n == name).count() > 1)
        .cloned()
        .collect();
    for_each_leaf(root, &mut Vec::new(), &mut |path, leaf| {
        status.entry(path).duplicated_selection = duplicated.contains(&leaf.name);
    });
    duplicated
}

/// The full refresh of the addon groups after a scan or a group change: resolve the modset
/// groups, check missing addons, duplicates and duplicated selection, in Java's order.
// Mirrors ui/main/AddonsPanel.java:731-738
pub fn refresh_group_status<'r>(
    root: &mut TreeDirectory,
    registry: &AddonRegistry,
    repository: impl Fn(&str) -> Option<&'r Repository>,
    available_tree: &TreeDirectory,
    opts: &ScanOptions,
) -> GroupStatus {
    let mut status = GroupStatus::default();
    resolve_addon_groups(root, registry, repository, opts, &mut status);
    check_missing_addons(root, registry, &mut Vec::new(), &mut status);
    check_duplicate_addons(root, registry, available_tree, &mut status);
    check_duplicate_addons_selection(root, &mut status);
    status
}
