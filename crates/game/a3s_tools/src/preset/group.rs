//! The pure parts of the addon-group panel's preset buttons: building the imported group,
//! choosing the modset to export, file names and the metadata dialog's checks.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use a3s_types::java_string::{compare_ignore_case, java_is_blank, java_trim};
use a3s_types::local::{TreeDirectory, TreeLeaf, TreeNode};

use super::{DEFAULT_EXPORT_PRESET_NAME, DEFAULT_IMPORTED_GROUP_NAME, ImportResult};

/// Adds a checked group named after the preset (made unique) to `root` with one checked leaf
/// per preset entry, and returns the group's name.
///
/// Matched mods and CDLCs are named by their local addon key (the CDLC's Java name); missing
/// ones by their display name (or CDLC Java name) and flagged `missing`. Leaves carry their
/// Workshop id or CDLC app id.
// Mirrors ui/main/AddonsPanel.java:1049-1100
pub fn add_imported_group(root: &mut TreeDirectory, result: &ImportResult) -> String {
    let name = unique_imported_group_name(root, &result.preset_name);
    let mut group = TreeDirectory::new(name.clone());
    group.selected = true;
    let leaf = |name: &str, missing: bool| TreeLeaf {
        selected: true,
        missing,
        ..TreeLeaf::new(name)
    };
    for workshop_mod in &result.matched {
        let key = workshop_mod.local_addon_key.as_deref().unwrap_or_default();
        group.add_child(TreeNode::Leaf(TreeLeaf {
            workshop_id: Some(workshop_mod.published_id.clone()),
            ..leaf(key, false)
        }));
    }
    for workshop_mod in &result.missing {
        group.add_child(TreeNode::Leaf(TreeLeaf {
            workshop_id: Some(workshop_mod.published_id.clone()),
            ..leaf(&workshop_mod.name, true)
        }));
    }
    for cdlc in &result.matched_dlc {
        let key = cdlc.local_addon_key.as_deref().unwrap_or_default();
        group.add_child(TreeNode::Leaf(TreeLeaf {
            dlc_app_id: Some(cdlc.steam_app_id.clone()),
            ..leaf(key, false)
        }));
    }
    for cdlc in &result.missing_dlc {
        let key = cdlc.addon_key.as_deref().unwrap_or(&cdlc.name);
        group.add_child(TreeNode::Leaf(TreeLeaf {
            dlc_app_id: Some(cdlc.steam_app_id.clone()),
            ..leaf(key, true)
        }));
    }
    root.add_child(TreeNode::Directory(group));
    name
}

/// The trimmed preset name (or [`DEFAULT_IMPORTED_GROUP_NAME`] when blank), suffixed ` (2)`,
/// ` (3)`, … until no group directly under `root` has that name, ignoring case.
// Mirrors ui/main/AddonsPanel.java:1102-1120
pub fn unique_imported_group_name(root: &TreeDirectory, preset_name: &str) -> String {
    let base = if java_is_blank(preset_name) {
        DEFAULT_IMPORTED_GROUP_NAME
    } else {
        java_trim(preset_name)
    };
    let taken = |candidate: &str| {
        root.directories()
            .any(|group| compare_ignore_case(candidate, &group.name).is_eq())
    };
    let mut candidate = base.to_owned();
    let mut suffix = 2;
    while taken(&candidate) {
        candidate = format!("{base} ({suffix})");
        suffix += 1;
    }
    candidate
}

/// One `- <message>` line per message.
// Mirrors ui/main/AddonsPanel.java:1462-1471
pub fn format_messages(messages: &[String]) -> String {
    messages
        .iter()
        .map(|message| format!("- {message}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The index of the top-level modset of `root` to export.
///
/// `selected_path` is the child-index path of the selected tree row: its top-level ancestor is
/// used when it is a modset (a group created from a repository or event). Otherwise the one
/// checked top-level modset is used; none or several give `None`. Plain user groups are never
/// exported.
// Mirrors ui/main/AddonsPanel.java:1417-1454
pub fn select_export_modset(
    root: &TreeDirectory,
    selected_path: Option<&[usize]>,
) -> Option<usize> {
    if let Some(&top) = selected_path.and_then(<[usize]>::first)
        && let Some(TreeNode::Directory(directory)) = root.children.get(top)
        && directory.modset_type.is_some()
    {
        return Some(top);
    }
    let mut checked = root
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| {
            matches!(child, TreeNode::Directory(directory)
                if directory.modset_type.is_some() && directory.selected)
        })
        .map(|(index, _)| index);
    match (checked.next(), checked.next()) {
        (Some(index), None) => Some(index),
        _ => None,
    }
}

/// The save dialog's proposed file name: the trimmed modset name with `<>:"/\|?*` replaced by
/// `_` (or [`DEFAULT_EXPORT_PRESET_NAME`] when that is empty), plus `.html`.
// Mirrors ui/main/AddonsPanel.java:1274, 1456-1460
pub fn suggested_file_name(modset_name: &str) -> String {
    let name: String = java_trim(modset_name)
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            other => other,
        })
        .collect();
    if name.is_empty() {
        format!("{DEFAULT_EXPORT_PRESET_NAME}.html")
    } else {
        format!("{name}.html")
    }
}

/// `path` with `.html` appended to its file name unless that already ends with `.html` in any
/// case (so `x.htm` becomes `x.htm.html`).
// Mirrors ui/main/AddonsPanel.java:1281-1284
pub fn ensure_html_extension(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default();
    if name.to_string_lossy().to_lowercase().ends_with(".html") {
        return path.to_path_buf();
    }
    let mut with_extension = OsString::from(name);
    with_extension.push(".html");
    path.with_file_name(with_extension)
}

/// Checks the metadata dialog's Workshop id and display name and returns them trimmed. The id
/// must be a decimal number without a leading zero; the name must not be empty.
// Mirrors ui/main/AddonsPanel.java:1366-1376
pub fn validate_metadata_input(id: &str, name: &str) -> Result<(String, String), &'static str> {
    let id = java_trim(id);
    let name = java_trim(name);
    let id_valid = id
        .as_bytes()
        .first()
        .is_some_and(|b| matches!(b, b'1'..=b'9'))
        && id.bytes().all(|b| b.is_ascii_digit());
    if !id_valid {
        return Err("Workshop ID must be a positive number.");
    }
    if name.is_empty() {
        return Err("Display name must not be empty.");
    }
    Ok((id.to_owned(), name.to_owned()))
}
