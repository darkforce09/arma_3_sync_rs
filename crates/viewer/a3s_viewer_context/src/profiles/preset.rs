//! Event presets: a repository event's mods as the selection of the active profile.

use a3s_addons::{add_groups_from_events, set_selected_recursive};
use a3s_repo_files::services::ServiceError;
use a3s_types::Event;
use a3s_types::constants::ModsetType;
use a3s_types::local::{TreeDirectory, TreeNode};

use crate::state::AppState;

/// Makes `event` of `repository` the selection of the active profile.
///
/// The event's group is added or refreshed (optional addons keep their previous choice when the
/// group existed), its required addons are enabled and every other addon is disabled. Returns whether the profile changed. Undo by restoring the profile read before.
// Mirrors ui/main/AddonsPanel.java:1846-1883
pub fn use_event_preset(
    state: &mut AppState,
    repository: &str,
    event: &Event,
) -> Result<bool, ServiceError> {
    state.client.update_active_profile(|profile| {
        let before = profile.tree.clone();
        add_groups_from_events(&mut profile.tree, &[(repository, event)], false);
        for child in &mut profile.tree.children {
            match child {
                TreeNode::Directory(group) if is_event_group(group, repository, event) => {
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
        profile.tree != before
    })
}

fn is_event_group(group: &TreeDirectory, repository: &str, event: &Event) -> bool {
    group.name == event.name
        && group.modset_type == Some(ModsetType::Event)
        && group.modset_repository_name.as_deref() == Some(repository)
}

/// Whether `directory` has leaves and all of them are selected.
fn all_leaves_selected(directory: &TreeDirectory) -> bool {
    let mut leaves = Vec::new();
    collect_leaf_selection(directory, &mut leaves);
    !leaves.is_empty() && leaves.iter().all(|selected| *selected)
}

fn collect_leaf_selection(directory: &TreeDirectory, out: &mut Vec<bool>) {
    for child in &directory.children {
        match child {
            TreeNode::Leaf(leaf) => out.push(leaf.selected),
            TreeNode::Directory(child) => collect_leaf_selection(child, out),
        }
    }
}
