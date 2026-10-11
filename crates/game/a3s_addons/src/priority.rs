//! The launch priority list and the search-directory order.
//!
//! The lists are `Profile::addon_names_by_priority` and `Profile::addon_search_directories`. Every
//! function returns whether it changed the list,
//! so the caller knows when to persist the profile.

use crate::registry::AddonRegistry;

/// Brings the priority list in line with the registry.
///
/// Every registry key (lowercase, DLCs
/// included) missing from `list` is appended in sorted order, then every entry that is not a
/// registry key is removed. An empty registry clears the list.
// Mirrors service/AddonService.java:374-417
pub fn sync_addon_priorities(list: &mut Vec<String>, registry: &AddonRegistry) -> bool {
    let mut available: Vec<&str> = registry.keys().collect();
    available.sort_by(|a, b| {
        a.to_lowercase()
            .encode_utf16()
            .cmp(b.to_lowercase().encode_utf16())
    });
    let before = list.clone();
    if available.is_empty() {
        list.clear();
    } else {
        for key in &available {
            if !list.iter().any(|name| name == key) {
                list.push((*key).to_owned());
            }
        }
        list.retain(|name| available.contains(&name.as_str()));
    }
    *list != before
}

/// Empties the priority list.
// Mirrors service/ProfileService.java:290-302
pub fn reset_addon_priority(list: &mut Vec<String>) -> bool {
    let changed = !list.is_empty();
    list.clear();
    changed
}

/// Moves the entry at `index` to the front; nothing happens for the first entry or an index out
/// of range.
// Mirrors service/ProfileService.java:304-321
pub fn move_top(list: &mut [String], index: usize) -> bool {
    if index == 0 || index >= list.len() {
        return false;
    }
    list[..=index].rotate_right(1);
    true
}

/// Swaps the entry at `index` with the one before it; nothing happens for the first entry or an
/// index out of range. Used for addon priorities and search directories alike.
// Mirrors service/ProfileService.java:323-341 and :363-381
pub fn move_up(list: &mut [String], index: usize) -> bool {
    if index == 0 || index >= list.len() {
        return false;
    }
    list.swap(index, index - 1);
    true
}

/// Swaps the entry at `index` with the one after it; nothing happens for the last entry, an
/// index out of range or an empty list. Used for addon priorities and search directories alike.
// Mirrors service/ProfileService.java:343-361 and :383-401
pub fn move_down(list: &mut [String], index: usize) -> bool {
    if index.saturating_add(1) >= list.len() {
        return false;
    }
    list.swap(index, index + 1);
    true
}
