//! Event presets: a repository event's mods as an addon group of the active profile.

use a3s_addons::{add_groups_from_events, set_selected_recursive};
use a3s_repo_files::services::ServiceError;
use a3s_types::Event;
use a3s_types::constants::ModsetType;
use a3s_types::local::TreeNode;

use crate::state::AppState;

/// Adds (or refreshes) the group of `event` in the active profile and selects all its mods.
///
/// Other groups are left as they are. Returns whether the profile changed. Undo by restoring
/// the profile tree read before.
// Mirrors ui/main/AddonsPanel.java:1846-1883
pub fn use_event_preset(
    state: &mut AppState,
    repository: &str,
    event: &Event,
) -> Result<bool, ServiceError> {
    state.client.update_active_profile(|profile| {
        let before = profile.tree.clone();
        add_groups_from_events(&mut profile.tree, &[(repository, event)], false);
        let group = profile
            .tree
            .children
            .iter_mut()
            .rev()
            .find_map(|node| match node {
                TreeNode::Directory(group)
                    if group.name == event.name
                        && group.modset_type == Some(ModsetType::Event)
                        && group.modset_repository_name.as_deref() == Some(repository) =>
                {
                    Some(group)
                }
                _ => None,
            });
        if let Some(group) = group {
            set_selected_recursive(group, true);
        }
        profile.tree != before
    })
}
