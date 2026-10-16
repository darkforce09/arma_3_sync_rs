//! Presets and modset groups: a repository event's or an addon group's mods as the selection
//! of the active profile, and the groups made from a repository kept up to date.

use a3s_addons::{add_groups_from_events, set_selected_recursive, update_group_modsets};
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

/// Makes the top-level addon group `group` the selection of the active profile.
///
/// All its addons are enabled and every other addon disabled. Returns whether the profile
/// changed; `false` also when the profile has no such group.
// Mirrors ui/main/AddonsPanel.java:1787-1799
pub fn use_group_preset(state: &mut AppState, group: &str) -> Result<bool, ServiceError> {
    state.client.update_active_profile(|profile| {
        let exists = profile
            .tree
            .directories()
            .any(|directory| directory.name == group);
        if !exists {
            return false;
        }
        let before = profile.tree.clone();
        for child in &mut profile.tree.children {
            match child {
                TreeNode::Directory(directory) => {
                    set_selected_recursive(directory, directory.name == group);
                }
                TreeNode::Leaf(leaf) => leaf.selected = false,
            }
        }
        profile.tree != before
    })
}

/// Refreshes the repository and event groups made from `repository` in every profile from its
/// last fetched metadata: new mods appear, removed ones go, enabled mods stay enabled.
// Mirrors ui/main/AddonsPanel.java:1885-1936
pub(crate) fn refresh_modset_groups(
    state: &mut AppState,
    repository: &str,
) -> Result<(), ServiceError> {
    let metadata = state.repositories.runtime(repository).metadata;
    let events: Vec<Event> = metadata
        .events
        .as_ref()
        .map(|events| events.list.clone())
        .unwrap_or_default();
    for name in state.client.profile_names() {
        state.client.update_profile(&name, |profile| {
            let before = profile.tree.clone();
            update_group_modsets(
                &mut profile.tree,
                repository,
                metadata.sync.as_deref(),
                &events,
            );
            profile.tree != before
        })?;
    }
    Ok(())
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
