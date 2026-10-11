//! Arma 3 Launcher HTML presets.
//!
//! Import ([`compare`], [`import_preset`]), export ([`generate`], [`write()`]) and the pure
//! helpers behind the addon-group buttons ([`add_imported_group`], [`select_export_modset`], …).
//!
//! The exported HTML is read by the Arma 3 Launcher and by other users' Arma3Sync, so
//! [`render_html`] reproduces Java's output byte for byte.

mod error;
mod export;
mod group;
mod import;
pub mod meta_cpp;
mod paths;
mod text;

pub use a3s_repo_files::local::WorkshopEntry;
pub use error::{ImportError, WriteError};
pub use export::{
    ExportResult, MetadataIssue, MetadataIssueReason, PresetEntry, WorkshopMetadataLookup,
    escape_html, generate, render_html, write,
};
pub use group::{
    add_imported_group, ensure_html_extension, format_messages, select_export_modset,
    suggested_file_name, unique_imported_group_name, validate_metadata_input,
};
pub use import::{Cdlc, ImportResult, WorkshopMod, compare, import_preset};

/// Prefix of a canonical Steam Workshop item URL.
pub const WORKSHOP_URL_PREFIX: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=";

/// Preset name of an imported preset that names none.
pub const DEFAULT_IMPORT_PRESET_NAME: &str = "Imported Arma 3 preset";

/// Preset name of an exported modset whose name is blank.
pub const DEFAULT_EXPORT_PRESET_NAME: &str = "Arma3Sync Preset";

/// Name of an imported addon group whose preset name is blank.
pub const DEFAULT_IMPORTED_GROUP_NAME: &str = "Imported Workshop preset";

/// The canonical Steam Workshop URL of an item.
// Mirrors service/Arma3LauncherPresetExporter.java:276-278
pub fn workshop_url(published_id: &str) -> String {
    format!("{WORKSHOP_URL_PREFIX}{published_id}")
}
