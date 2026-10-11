//! The Java client's `known-workshop-mods.properties` (`service/WorkshopMetadataStore.java`).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::LegacyFile;
use super::properties;
use crate::local::WorkshopEntry;
use crate::local::naming::{java_is_blank, java_trim};
use crate::local::toml_file::file_name;

/// Prefix of every property (`PREFIX`).
// Mirrors service/WorkshopMetadataStore.java:29
const PREFIX: &str = "mod.";

/// Reads the Workshop metadata entries of a Java client, sorted by addon key.
///
/// Each `mod.<segment>.publishedid` with a matching `mod.<segment>.name` is an entry whose key
/// is the base64url-decoded segment (falling back to `mod.<segment>.key`). Entries with a blank
/// key, id or name are skipped, as Java's `find` ignores them. A file that cannot be read or
/// parsed gives no entries and one warning: Java then starts with an empty cache.
// Mirrors service/WorkshopMetadataStore.java:42-53, 106-123
pub fn read_workshop_file(path: &Path) -> LegacyFile<Vec<WorkshopEntry>> {
    parse_workshop_file(path).unwrap_or_else(|warning| LegacyFile {
        value: Vec::new(),
        warnings: vec![warning],
    })
}

/// Like [`read_workshop_file`], but an unreadable file is an `Err` holding the warning.
pub(crate) fn parse_workshop_file(path: &Path) -> Result<LegacyFile<Vec<WorkshopEntry>>, String> {
    let file = file_name(path);
    let mut warnings = Vec::new();
    let pairs = std::fs::read(path)
        .map_err(|err| err.to_string())
        .and_then(|bytes| properties::parse(&bytes).map_err(|err| err.to_string()))
        .map_err(|err| format!("{file}: ignored, cannot be read: {err}"))?;
    // `Properties` keeps the last value of a repeated key.
    let properties: HashMap<String, String> = pairs.into_iter().collect();

    let mut entries = BTreeMap::new();
    let mut segments: Vec<&str> = properties
        .keys()
        .filter_map(|k| k.strip_prefix(PREFIX)?.strip_suffix(".publishedid"))
        .collect();
    segments.sort_unstable();
    for segment in segments {
        let get = |suffix: &str| properties.get(&format!("{PREFIX}{segment}.{suffix}"));
        let (Some(published_id), Some(name)) = (get("publishedid"), get("name")) else {
            continue;
        };
        let decoded = URL_SAFE_NO_PAD
            .decode(segment)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let Some(addon_key) = decoded.or_else(|| get("key").cloned()) else {
            warnings.push(format!(
                "{file}: skipped {PREFIX}{segment}: the addon key cannot be decoded"
            ));
            continue;
        };
        if java_is_blank(&addon_key) || java_is_blank(published_id) || java_is_blank(name) {
            continue;
        }
        entries.insert(
            addon_key.clone(),
            WorkshopEntry {
                addon_key,
                published_id: java_trim(published_id).to_owned(),
                name: java_trim(name).to_owned(),
            },
        );
    }
    Ok(LegacyFile {
        value: entries.into_values().collect(),
        warnings,
    })
}
