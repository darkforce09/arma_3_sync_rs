//! Turning a modset into an Arma 3 Launcher preset and writing it to disk.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use a3s_repo_files::local::{WorkshopEntry, WorkshopMetadata};
use a3s_types::constants::GameDlc;
use a3s_types::java_string::java_is_blank;
use a3s_types::local::{Addon, TreeDirectory, TreeLeaf, TreeNode};

use super::paths::absolute_normalized;
use super::{DEFAULT_EXPORT_PRESET_NAME, WriteError, meta_cpp, workshop_url};

/// Where [`generate`] looks up the Workshop id and name of an addon whose `meta.cpp` lacks them.
pub trait WorkshopMetadataLookup {
    /// The stored entry of `addon_key`, trimmed; `None` when there is none or it is blank.
    fn find(&self, addon_key: &str) -> Option<WorkshopEntry>;
}

impl WorkshopMetadataLookup for WorkshopMetadata<'_> {
    fn find(&self, addon_key: &str) -> Option<WorkshopEntry> {
        WorkshopMetadata::find(self, addon_key)
    }
}

impl<F: Fn(&str) -> Option<WorkshopEntry>> WorkshopMetadataLookup for F {
    fn find(&self, addon_key: &str) -> Option<WorkshopEntry> {
        self(addon_key)
    }
}

/// One row of the exported preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetEntry {
    /// Display name, written as is (HTML-escaped).
    pub name: String,
    pub url: String,
}

/// Why an addon has no usable Workshop id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataIssueReason {
    /// The addon folder has no `meta.cpp` (and the store gave no valid id).
    MetaCppMissing,

    /// `meta.cpp` (and the store) have no valid id.
    PublishedIdInvalid,
}

impl MetadataIssueReason {
    /// The reason as Java words it.
    // Mirrors service/Arma3LauncherPresetExporter.java:177-179
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MetaCppMissing => "meta.cpp is missing",
            Self::PublishedIdInvalid => "publishedid is missing or invalid",
        }
    }
}

impl fmt::Display for MetadataIssueReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A selected addon whose Workshop id must be entered (then saved to the Workshop metadata
/// store) before the preset can be exported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataIssue {
    pub addon_key: String,

    /// The addon's folder name.
    pub addon_name: String,
    pub reason: MetadataIssueReason,

    /// The invalid id found, if any.
    pub published_id: Option<String>,

    /// The display name found, or the folder name.
    pub name: String,
}

impl MetadataIssue {
    /// Initial values of the Workshop id and display name fields: the id found (or empty) and
    /// the name found (or the folder name when blank).
    // Mirrors ui/main/AddonsPanel.java:1330-1332
    pub fn prefill(&self) -> (String, String) {
        let name = if java_is_blank(&self.name) {
            &self.addon_name
        } else {
            &self.name
        };
        (self.published_id.clone().unwrap_or_default(), name.clone())
    }
}

/// The outcome of [`generate`]: the entries, the problems found and, when there are none, the
/// HTML.
#[derive(Clone, Debug)]
pub struct ExportResult {
    preset_name: String,
    workshop: Vec<PresetEntry>,
    dlc: Vec<PresetEntry>,
    metadata_issues: Vec<MetadataIssue>,
    warnings: Vec<String>,
    errors: Vec<String>,
    html: Option<String>,
}

// Mirrors service/Arma3LauncherPresetExporter.java:295-344
impl ExportResult {
    /// No errors and the HTML was rendered (so no metadata issues either).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty() && self.html.is_some()
    }

    pub fn html(&self) -> Option<&str> {
        self.html.as_deref()
    }

    /// The modset's name, or [`DEFAULT_EXPORT_PRESET_NAME`] when blank.
    pub fn preset_name(&self) -> &str {
        &self.preset_name
    }

    pub fn workshop_entries(&self) -> &[PresetEntry] {
        &self.workshop
    }

    pub fn dlc_entries(&self) -> &[PresetEntry] {
        &self.dlc
    }

    pub fn workshop_count(&self) -> usize {
        self.workshop.len()
    }

    pub fn dlc_count(&self) -> usize {
        self.dlc.len()
    }

    /// Duplicate Workshop ids that were left out.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Problems that prevent the export.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Addons without a usable Workshop id.
    pub fn metadata_issues(&self) -> &[MetadataIssue] {
        &self.metadata_issues
    }

    /// `<n> Workshop mods and <m> CDLCs exported.`
    pub fn summary(&self) -> String {
        format!(
            "{} Workshop mods and {} CDLCs exported.",
            self.workshop_count(),
            self.dlc_count()
        )
    }
}

/// Collects the selected leaves of `modset` (depth first, every directory descended into
/// whatever its own check state) as Workshop mods and CDLCs.
///
/// A leaf named like a CDLC is that CDLC. Any other leaf is resolved to a local addon by
/// `resolve` (by name, as the addon registry does); its Workshop id and name come from its
/// `meta.cpp`, then from `known` for what is missing, and the name falls back to the folder
/// name. Problems are collected, never fatal: unresolvable or unreadable addons are errors,
/// addons without a valid id are metadata issues, repeated Workshop ids are warnings (the first
/// leaf wins). The HTML is rendered only when there are no errors and no metadata issues.
// Mirrors service/Arma3LauncherPresetExporter.java:51-72
pub fn generate<'a>(
    modset: &TreeDirectory,
    resolve: &dyn Fn(&str) -> Option<&'a Addon>,
    known: Option<&dyn WorkshopMetadataLookup>,
) -> ExportResult {
    tracing::debug!("Launcher preset metadata scan started: {}", modset.name);
    let mut result = ExportResult {
        preset_name: if java_is_blank(&modset.name) {
            DEFAULT_EXPORT_PRESET_NAME.to_owned()
        } else {
            modset.name.clone()
        },
        workshop: Vec::new(),
        dlc: Vec::new(),
        metadata_issues: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        html: None,
    };
    let mut collector = Collector {
        resolve,
        known,
        workshop_ids: Vec::new(),
        dlcs: Vec::new(),
        result: &mut result,
    };
    collector.directory(modset);

    if result.workshop.is_empty() && result.dlc.is_empty() && result.errors.is_empty() {
        result
            .errors
            .push("The selected modset contains no selected Workshop mods or CDLCs.".to_owned());
    }
    if result.errors.is_empty() && result.metadata_issues.is_empty() {
        result.html = Some(render_html(
            &result.preset_name,
            &result.workshop,
            &result.dlc,
        ));
    }
    tracing::debug!(
        "Launcher preset metadata scan completed: issues={}, errors={}",
        result.metadata_issues.len(),
        result.errors.len()
    );
    result
}

/// The traversal state of [`generate`].
struct Collector<'r, 'a> {
    resolve: &'r dyn Fn(&str) -> Option<&'a Addon>,
    known: Option<&'r dyn WorkshopMetadataLookup>,

    /// Ids of `result.workshop`, in the same order.
    workshop_ids: Vec<String>,

    /// CDLCs of `result.dlc`, in the same order.
    dlcs: Vec<GameDlc>,
    result: &'r mut ExportResult,
}

impl Collector<'_, '_> {
    // Mirrors service/Arma3LauncherPresetExporter.java:194-197
    fn directory(&mut self, directory: &TreeDirectory) {
        for child in &directory.children {
            match child {
                TreeNode::Directory(directory) => self.directory(directory),
                TreeNode::Leaf(leaf) => self.leaf(leaf),
            }
        }
    }

    // Mirrors service/Arma3LauncherPresetExporter.java:123-192
    fn leaf(&mut self, leaf: &TreeLeaf) {
        if !leaf.selected {
            return;
        }
        if let Some(dlc) = GameDlc::from_name(&leaf.name) {
            if !self.dlcs.contains(&dlc) {
                self.dlcs.push(dlc);
                self.result.dlc.push(PresetEntry {
                    name: dlc.display_name().to_owned(),
                    url: dlc.steam_store_url(),
                });
            }
            return;
        }

        let Some(addon) = (self.resolve)(&leaf.name) else {
            self.result.errors.push(format!(
                "Selected addon is not available locally: {}",
                leaf.name
            ));
            return;
        };
        let meta_file = Path::new(&addon.path).join(&addon.name).join("meta.cpp");
        let (mut published_id, mut name) = if meta_file.is_file() {
            match fs::read_to_string(&meta_file) {
                // Mirrors service/Arma3LauncherPresetExporter.java:200-208
                Ok(text) => (
                    meta_cpp::published_id_any(&text).map(str::to_owned),
                    meta_cpp::name_raw(&text).map(meta_cpp::unescape_meta_value),
                ),
                Err(err) => {
                    self.result
                        .errors
                        .push(format!("Could not read {}: {err}", meta_file.display()));
                    return;
                }
            }
        } else {
            (None, None)
        };

        // Mirrors service/Arma3LauncherPresetExporter.java:159-174
        let addon_key = &addon.key;
        let known = self.known.and_then(|store| store.find(addon_key));
        if let Some(known) = known {
            if !is_valid_published_id(published_id.as_deref()) {
                published_id = Some(known.published_id);
            }
            if name.is_none() {
                name = Some(known.name);
            }
        }
        // A missing display name is recoverable through the folder name; only the id is not.
        let name = match name {
            Some(name) if !java_is_blank(&name) => name,
            _ => addon.name.clone(),
        };

        let published_id = match published_id {
            Some(id) if is_valid_published_id(Some(&id)) => id,
            invalid => {
                self.result.metadata_issues.push(MetadataIssue {
                    addon_key: addon_key.clone(),
                    addon_name: addon.name.clone(),
                    reason: if meta_file.is_file() {
                        MetadataIssueReason::PublishedIdInvalid
                    } else {
                        MetadataIssueReason::MetaCppMissing
                    },
                    published_id: invalid,
                    name,
                });
                return;
            }
        };
        if self.workshop_ids.contains(&published_id) {
            self.result
                .warnings
                .push(format!("Duplicate Workshop ID omitted: {published_id}"));
        } else {
            self.result.workshop.push(PresetEntry {
                name,
                url: workshop_url(&published_id),
            });
            self.workshop_ids.push(published_id);
        }
    }
}

/// A positive decimal number without a leading zero that fits an `i64`.
// Mirrors service/Arma3LauncherPresetExporter.java:210-220
fn is_valid_published_id(published_id: Option<&str>) -> bool {
    published_id.is_some_and(|id| {
        id.as_bytes()
            .first()
            .is_some_and(|b| matches!(b, b'1'..=b'9'))
            && id.bytes().all(|b| b.is_ascii_digit())
            && id.parse::<i64>().is_ok()
    })
}

/// Java's HTML escaping: `&`, `<`, `>`, `"` and `'` (as `&#39;`).
// Mirrors service/Arma3LauncherPresetExporter.java:284-293
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The preset document, byte for byte as Java renders it: LF line endings, no BOM, the
/// `mod-list` table always present and the `dlc-list` block only when there is a CDLC.
// Mirrors service/Arma3LauncherPresetExporter.java:222-274
pub fn render_html(preset_name: &str, workshop: &[PresetEntry], dlc: &[PresetEntry]) -> String {
    let name = escape_html(preset_name);
    let mut html = String::with_capacity(4096);
    html.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    html.push_str("<html>\n  <!--Created by Arma 3 Launcher: https://arma3.com-->\n  <head>\n");
    html.push_str("    <meta name=\"arma:Type\" content=\"preset\" />\n");
    html.push_str(&format!(
        "    <meta name=\"arma:PresetName\" content=\"{name}\" />\n"
    ));
    html.push_str(
        "    <meta name=\"generator\" content=\"Arma3Sync - https://github.com/vpzbrig21/Arma3Sync\" />\n",
    );
    html.push_str("    <title>Arma 3</title>\n");
    html.push_str(
        "    <link href=\"https://fonts.googleapis.com/css?family=Roboto\" rel=\"stylesheet\" type=\"text/css\" />\n",
    );
    html.push_str("    <style>\n");
    html.push_str("body { margin: 0; padding: 0; color: #fff; background: #000; }\n");
    html.push_str(
        "body, th, td { font: 95%/1.3 Roboto, Segoe UI, Tahoma, Arial, Helvetica, sans-serif; }\n",
    );
    html.push_str("td { padding: 3px 30px 3px 0; }\n");
    html.push_str(
        "h1 { padding: 20px 20px 0 20px; color: white; font-weight: 200; font-family: segoe ui; font-size: 3em; margin: 0; }\n",
    );
    html.push_str("em { font-variant: italic; color: silver; }\n");
    html.push_str(".before-list { padding: 5px 20px 10px 20px; }\n");
    html.push_str(".mod-list, .dlc-list { background: #222222; padding: 20px; }\n");
    html.push_str(".footer { padding: 20px; color: gray; }\n");
    html.push_str(".whups { color: gray; }\n");
    html.push_str("a { color: #D18F21; text-decoration: underline; }\n");
    html.push_str("a:hover { color: #F1AF41; text-decoration: none; }\n");
    html.push_str(".from-steam { color: #449EBD; }\n");
    html.push_str(".from-local { color: gray; }\n");
    html.push_str("    </style>\n  </head>\n  <body>\n");
    html.push_str(&format!(
        "    <h1>Arma 3 - Preset <strong>{name}</strong></h1>\n"
    ));
    html.push_str(
        "    <p class=\"before-list\"><em>To import this preset, drag this file onto the Launcher window. Or click the MODS tab, then PRESET in the top right, then IMPORT at the bottom, and finally select this file.</em></p>\n",
    );
    html.push_str("    <div class=\"mod-list\">\n      <table>\n");
    for entry in workshop {
        let url = escape_html(&entry.url);
        html.push_str("        <tr data-type=\"ModContainer\">\n");
        html.push_str(&format!(
            "          <td data-type=\"DisplayName\">{}</td>\n",
            escape_html(&entry.name)
        ));
        html.push_str("          <td><span class=\"from-steam\">Steam</span></td>\n");
        html.push_str(&format!(
            "          <td><a href=\"{url}\" data-type=\"Link\">{url}</a></td>\n"
        ));
        html.push_str("        </tr>\n");
    }
    html.push_str("      </table>\n    </div>\n");
    if !dlc.is_empty() {
        html.push_str("    <div class=\"dlc-list\">\n      <table>\n");
        for entry in dlc {
            let url = escape_html(&entry.url);
            html.push_str("        <tr data-type=\"DlcContainer\">\n");
            html.push_str(&format!(
                "          <td data-type=\"DisplayName\">{}</td>\n",
                escape_html(&entry.name)
            ));
            html.push_str(&format!(
                "          <td><a href=\"{url}\" data-type=\"Link\">{url}</a></td>\n"
            ));
            html.push_str("        </tr>\n");
        }
        html.push_str("      </table>\n    </div>\n");
    }
    html.push_str(
        "    <div class=\"footer\"><span>Created by Arma 3 Launcher by Bohemia Interactive. Exported by Arma3Sync.</span></div>\n",
    );
    html.push_str("  </body>\n</html>\n");
    html
}

/// Writes a valid result to `target`, made absolute against the current directory.
///
/// Missing folders are created. The HTML goes to a temporary `<file name><random>.tmp` next to
/// the target, which is then renamed over it, so an existing preset is replaced whole or not at
/// all; the temporary file never survives. The written file is checked to be non-empty and
/// complete.
// Mirrors service/Arma3LauncherPresetExporter.java:77-105
pub fn write(target: &Path, result: &ExportResult) -> Result<(), WriteError> {
    if !result.is_valid() {
        return Err(WriteError::InvalidResult);
    }
    let target = absolute_normalized(target)?;
    tracing::debug!("Writing launcher preset {}", target.display());
    let written = write_checked(&target, result);
    match &written {
        Ok(()) => tracing::debug!("Launcher preset written: {}", target.display()),
        Err(err) => tracing::debug!("Launcher preset write failed: {}: {err}", target.display()),
    }
    written
}

fn write_checked(target: &Path, result: &ExportResult) -> Result<(), WriteError> {
    let parent = match target.parent() {
        Some(parent) => parent.to_path_buf(),
        None => absolute_normalized(Path::new("."))?,
    };
    fs::create_dir_all(&parent)?;
    let html = match result.html() {
        Some(html) if html.contains("</html>") => html,
        _ => return Err(WriteError::Incomplete),
    };
    let file_name = target.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "the target has no file name")
    })?;
    let mut temporary = tempfile::Builder::new()
        .prefix(file_name)
        .suffix(".tmp")
        .tempfile_in(&parent)?;
    temporary.write_all(html.as_bytes())?;
    // A failed rename drops the temporary file with the error, which deletes it.
    temporary.persist(target).map_err(|err| err.error)?;

    let metadata = fs::metadata(target);
    if !metadata.is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0) {
        return Err(WriteError::Empty(target.to_path_buf()));
    }
    if !fs::read_to_string(target)?.contains("</html>") {
        return Err(WriteError::Integrity(target.to_path_buf()));
    }
    Ok(())
}
