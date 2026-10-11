//! Addon groups built from a repository's content tree (`.a3s/sync`) and from its events, and
//! their refresh after a synchronization, keeping the user's selection.

use a3s_types::constants::ModsetType;
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};
use a3s_types::{Event, SyncTreeDirectory, SyncTreeNode};

use crate::REPOSITORY_GROUP_ROOT_NAME;
use crate::tree::{apply_selected_paths, normalize_group_tree, selected_paths, sort_children};

/// The cleaned node for a sync directory: a leaf for a directory marked as an addon (never
/// descended into), a directory for a folder with addons below it, `None` otherwise. Files and
/// the `deleted`/`hidden` flags are ignored.
// Mirrors service/RepositoryService.java:1112-1144
fn addon_node(dir: &SyncTreeDirectory) -> Option<TreeNode> {
    if dir.mark_as_addon {
        return Some(TreeNode::Leaf(TreeLeaf::new(dir.name.clone())));
    }
    let mut out = TreeDirectory::new(dir.name.clone());
    out.children = dir.directories().filter_map(addon_node).collect();
    sort_children(&mut out);
    (!out.children.is_empty()).then_some(TreeNode::Directory(out))
}

/// An addon group (root [`REPOSITORY_GROUP_ROOT_NAME`]) from a repository's sync tree.
///
/// Every directory marked as an addon becomes a leaf named like the folder; the folders leading
/// to it become directories.
///
/// With `with_userconfig`, each top-level `userconfig` directory (any case) is added with one
/// leaf per entry; it is dropped when the addon tree already has a directory of the same name.
// Mirrors service/RepositoryService.java:1069-1110
pub fn group_from_repository(sync: &SyncTreeDirectory, with_userconfig: bool) -> TreeDirectory {
    let mut root = TreeDirectory::new(REPOSITORY_GROUP_ROOT_NAME);
    root.children = sync.directories().filter_map(addon_node).collect();
    sort_children(&mut root);
    if with_userconfig {
        for node in &sync.children {
            if let SyncTreeNode::Directory(d) = node
                && d.name.to_lowercase() == "userconfig"
            {
                let mut userconfig = TreeDirectory::new(d.name.clone());
                userconfig.children = d
                    .children
                    .iter()
                    .map(|n| TreeNode::Leaf(TreeLeaf::new(n.name())))
                    .collect();
                sort_children(&mut userconfig);
                root.add_child(TreeNode::Directory(userconfig));
            }
        }
    }
    normalize_group_tree(&mut root);
    root
}

/// Removes the last top-level node of `root` named `name` and returns its selected paths.
// Mirrors ui/main/AddonsPanel.java:1815-1825
fn take_group(root: &mut TreeDirectory, name: &str) -> Vec<String> {
    let Some(index) = root.children.iter().rposition(|n| n.name() == name) else {
        return Vec::new();
    };
    match root.remove_child(index) {
        Some(TreeNode::Directory(d)) => selected_paths(&d),
        Some(TreeNode::Leaf(l)) if l.selected => vec![l.name],
        _ => Vec::new(),
    }
}

/// Adds or replaces one top-level group per repository, named after the repository.
///
/// Each group is built from the sync tree ([`group_from_repository`] without userconfig). A
/// repository whose sync tree is not loaded is skipped. A replaced group's selection carries over
/// by path.
// Mirrors ui/main/AddonsPanel.java:1810-1835
pub fn add_groups_from_repository(
    root: &mut TreeDirectory,
    repositories: &[(&str, Option<&SyncTreeDirectory>)],
    updated: bool,
) {
    for &(repository_name, sync) in repositories {
        let Some(sync) = sync else {
            continue;
        };
        let mut group = group_from_repository(sync, false);
        let selection = take_group(root, repository_name);
        group.name = repository_name.to_owned();
        group.modset_type = Some(ModsetType::Repository);
        group.modset_repository_name = Some(repository_name.to_owned());
        group.updated = updated;
        apply_selected_paths(&mut group, &selection);
        root.add_child(TreeNode::Directory(group));
    }
}

/// Adds or replaces one top-level group per `(repository name, event)`, named after the event.
///
/// The group has one leaf per addon and DLC of the event (`optional` from the event; a name in
/// both maps takes the DLC value). Userconfig folders are not added. A replaced group's selection
/// carries over by path.
// Mirrors ui/main/AddonsPanel.java:1846-1883
pub fn add_groups_from_events(root: &mut TreeDirectory, events: &[(&str, &Event)], updated: bool) {
    for &(repository_name, event) in events {
        let selection = take_group(root, &event.name);
        let mut group = TreeDirectory::new(event.name.clone());
        group.modset_type = Some(ModsetType::Event);
        group.modset_repository_name = Some(repository_name.to_owned());
        group.updated = updated;
        let mut names = event.addon_names.clone();
        names.extend(event.dlc_names.iter().map(|(k, v)| (k.clone(), *v)));
        group.children = names
            .into_iter()
            .map(|(name, optional)| {
                TreeNode::Leaf(TreeLeaf {
                    optional,
                    ..TreeLeaf::new(name)
                })
            })
            .collect();
        sort_children(&mut group);
        apply_selected_paths(&mut group, &selection);
        root.add_child(TreeNode::Directory(group));
    }
}

/// Refreshes the modset groups of one repository after it was synchronized.
///
/// Its repository
/// group (if `root` has one) from `sync`, then each of its event groups from the first event of
/// `events` with the group's name. Refreshed groups are marked `updated`.
// Mirrors ui/main/AddonsPanel.java:1885-1936 and service/RepositoryService.java:866-892
pub fn update_group_modsets(
    root: &mut TreeDirectory,
    repository_name: &str,
    sync: Option<&SyncTreeDirectory>,
    events: &[Event],
) {
    let is_modset_of = |d: &TreeDirectory, kind: ModsetType| {
        d.modset_type == Some(kind) && d.modset_repository_name.as_deref() == Some(repository_name)
    };
    if root
        .directories()
        .any(|d| is_modset_of(d, ModsetType::Repository))
    {
        add_groups_from_repository(root, &[(repository_name, sync)], true);
    }
    let picked: Vec<(&str, &Event)> = root
        .directories()
        .filter(|d| is_modset_of(d, ModsetType::Event))
        .filter_map(|d| events.iter().find(|e| e.name == d.name))
        .map(|e| (repository_name, e))
        .collect();
    add_groups_from_events(root, &picked, true);
}
