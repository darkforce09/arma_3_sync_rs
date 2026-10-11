//! Arma 3 addons on the player's machine and the profile's addon groups.
//!
//! * [`scan`] walks the profile's search directories and the Arma 3 folder into an
//!   [`AddonRegistry`] (mods and CDLCs under their symbolic keys) and the available-addons tree.
//! * [`priority`] keeps the launch priority list in line with the registry and reorders it.
//! * [`group_from_repository`], [`add_groups_from_repository`], [`add_groups_from_events`] and
//!   [`update_group_modsets`] build modset groups from a repository's sync tree and events.
//! * [`refresh_group_status`] and the individual checks resolve group leaves against the
//!   registry and flag missing, duplicate and doubly selected addons.
//!
//! Everything is synchronous and works on owned values; the caller persists the profile.

mod check;
mod modset;
pub mod priority;
mod registry;
mod scan;
mod tree;
mod workshop;

pub use a3s_types::local::Addon;

pub use check::{
    GroupStatus, LeafStatus, NodePath, check_duplicate_addons, check_duplicate_addons_selection,
    check_missing_addons, check_missing_selected_addons, refresh_group_status,
    resolve_addon_groups,
};
pub use modset::{
    add_groups_from_events, add_groups_from_repository, group_from_repository, update_group_modsets,
};
pub use registry::AddonRegistry;
pub use scan::{
    ScanOptions, ScanResult, arma3_directory, available_addons_list, effective_search_directories,
    excluded_system_paths, is_mod_directory, scan,
};
pub use tree::{
    apply_selected_paths, normalize_group_tree, selected_paths, set_selected_recursive,
};
pub use workshop::{find_addon_by_workshop_id, parse_published_id};

/// Root name of the available-addons tree and list.
pub const AVAILABLE_ROOT_NAME: &str = "racine1";

/// Root name of a group built from a repository ([`group_from_repository`]).
pub const REPOSITORY_GROUP_ROOT_NAME: &str = "racine";
