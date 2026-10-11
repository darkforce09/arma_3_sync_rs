//! Reading an Arma 3 Launcher preset and comparing it with the local addons, without changing
//! anything.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use a3s_types::constants::GameDlc;
use a3s_types::java_string::{java_is_blank, java_trim};
use a3s_types::local::Addon;
use regex::Regex;

use super::paths::absolute_normalized;
use super::text::{clean_text, pattern};
use super::{DEFAULT_IMPORT_PRESET_NAME, ImportError, meta_cpp, workshop_url};

/// The patterns use `[ \t\n\x0B\x0C\r]` for Java's ASCII-only `\s`.
// Mirrors service/Arma3LauncherPresetImporter.java:28-29
static MOD_ROW: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<tr\b[^>]*data-type[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]ModContainer['"][^>]*>(.*?)</tr[ \t\n\x0B\x0C\r]*>"#,
    )
});

// Mirrors service/Arma3LauncherPresetImporter.java:30-31
static DLC_ROW: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<tr\b[^>]*data-type[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]DlcContainer['"][^>]*>(.*?)</tr[ \t\n\x0B\x0C\r]*>"#,
    )
});

// Mirrors service/Arma3LauncherPresetImporter.java:32-33
static DISPLAY_NAME: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<[^>]*data-type[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]DisplayName['"][^>]*>(.*?)</[^>]+>"#,
    )
});

// Mirrors service/Arma3LauncherPresetImporter.java:34-35
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<a\b[^>]*data-type[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]Link['"][^>]*href[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]([^'"]+)['"][^>]*>"#,
    )
});

// Mirrors service/Arma3LauncherPresetImporter.java:36-37
static LINK_REVERSED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<a\b[^>]*href[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]([^'"]+)['"][^>]*data-type[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]Link['"][^>]*>"#,
    )
});

// Mirrors service/Arma3LauncherPresetImporter.java:38-39
static WORKSHOP_ID: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)[?&]id=([1-9][0-9]*)"));

// Mirrors service/Arma3LauncherPresetImporter.java:40-41
static STEAM_APP_ID: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)(?:/app/|[?&]appid=)([1-9][0-9]*)"));

// Mirrors service/Arma3LauncherPresetImporter.java:247
static PRESET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?is)<meta\b[^>]*name[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]arma:PresetName['"][^>]*content[ \t\n\x0B\x0C\r]*=[ \t\n\x0B\x0C\r]*['"]([^'"]*)['"]"#,
    )
});

/// The comparison of a preset with the local addons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportResult {
    /// The preset's name, or [`DEFAULT_IMPORT_PRESET_NAME`].
    pub preset_name: String,

    /// Distinct Workshop ids in the preset.
    pub requested_count: usize,

    /// Distinct CDLC app ids in the preset.
    pub requested_dlc_count: usize,

    /// Workshop mods found locally, in preset order.
    pub matched: Vec<WorkshopMod>,

    /// Workshop mods not found locally, in preset order.
    pub missing: Vec<WorkshopMod>,

    /// CDLCs installed locally, in preset order.
    pub matched_dlc: Vec<Cdlc>,

    /// CDLCs not installed or unknown, in preset order.
    pub missing_dlc: Vec<Cdlc>,
}

/// A Workshop mod row of a preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopMod {
    pub published_id: String,

    /// The row's display name, or `Workshop item <id>`.
    pub name: String,

    /// The canonical Workshop URL (not the row's link).
    pub url: String,

    /// The local addon's `meta.cpp` name (or folder name) when matched.
    pub local_name: Option<String>,

    /// The local addon's key when matched.
    pub local_addon_key: Option<String>,
}

/// A CDLC row of a preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cdlc {
    /// The known CDLC of the app id; `None` for an unknown one.
    pub dlc: Option<GameDlc>,

    /// The known CDLC's Java name (`GM`, `WS`, …).
    pub addon_key: Option<String>,

    /// The row's display name, or the known CDLC's, or `CDLC <app id>`.
    pub name: String,

    pub steam_app_id: String,

    /// The known CDLC's store URL, or the row's raw link.
    pub url: String,

    /// The Java name of the installed CDLC when matched; equal to [`Self::addon_key`] otherwise.
    pub local_addon_key: Option<String>,
}

/// Reads the preset at `path` (UTF-8) and [`compare`]s it with `local_addons`.
// Mirrors service/Arma3LauncherPresetImporter.java:58-68
pub fn import_preset<'a>(
    path: &Path,
    local_addons: impl IntoIterator<Item = &'a Addon>,
) -> Result<ImportResult, ImportError> {
    if !path.is_file() {
        return Err(ImportError::NotAFile(path.to_path_buf()));
    }
    let html = fs::read_to_string(path).map_err(|source| ImportError::ReadPreset {
        path: path.to_path_buf(),
        source,
    })?;
    compare(&html, local_addons)
}

/// Parses the Workshop and CDLC rows of a preset's HTML and splits them into those present in
/// `local_addons` and those missing.
///
/// A Workshop mod matches an addon whose `meta.cpp` declares its id (the first such addon in
/// `local_addons` order wins); a CDLC matches an addon named like the CDLC whose folder exists.
/// A `meta.cpp` that cannot be read is skipped with a warning.
// Mirrors service/Arma3LauncherPresetImporter.java:73-142
pub fn compare<'a>(
    html: &str,
    local_addons: impl IntoIterator<Item = &'a Addon>,
) -> Result<ImportResult, ImportError> {
    if java_is_blank(html) {
        return Err(ImportError::Empty);
    }
    let local_addons: Vec<&Addon> = local_addons.into_iter().collect();
    let local_mods = local_workshop_mods(&local_addons);
    let local_dlcs = local_dlcs(&local_addons);

    let mut requested: Vec<WorkshopMod> = Vec::new();
    for row in MOD_ROW.captures_iter(html) {
        let row = &row[1];
        let link = extract_link(row);
        let Some(id) = first_group(&WORKSHOP_ID, link) else {
            continue;
        };
        if requested.iter().any(|known| known.published_id == id) {
            continue;
        }
        let mut name = clean_text(extract_display_name(row));
        if name.is_empty() {
            name = format!("Workshop item {id}");
        }
        requested.push(WorkshopMod {
            published_id: id.to_owned(),
            name,
            url: workshop_url(id),
            local_name: None,
            local_addon_key: None,
        });
    }

    let mut requested_dlc: Vec<Cdlc> = Vec::new();
    for row in DLC_ROW.captures_iter(html) {
        let row = &row[1];
        let link = extract_link(row);
        let Some(app_id) = first_group(&STEAM_APP_ID, link) else {
            continue;
        };
        if requested_dlc
            .iter()
            .any(|known| known.steam_app_id == app_id)
        {
            continue;
        }
        let known = GameDlc::from_steam_app_id(app_id);
        let mut name = clean_text(extract_display_name(row));
        if name.is_empty() {
            name = known.map_or_else(
                || format!("CDLC {app_id}"),
                |dlc| dlc.display_name().to_owned(),
            );
        }
        let addon_key = known.map(|dlc| dlc.java_name().to_owned());
        requested_dlc.push(Cdlc {
            dlc: known,
            local_addon_key: addon_key.clone(),
            addon_key,
            name,
            steam_app_id: app_id.to_owned(),
            url: known.map_or_else(|| link.to_owned(), GameDlc::steam_store_url),
        });
    }

    if requested.is_empty() && requested_dlc.is_empty() {
        return Err(ImportError::NoEntries);
    }

    let requested_count = requested.len();
    let requested_dlc_count = requested_dlc.len();
    let (mut matched, mut missing) = (Vec::new(), Vec::new());
    for mut workshop_mod in requested {
        match local_mods
            .iter()
            .find(|local| local.published_id == workshop_mod.published_id)
        {
            Some(local) => {
                workshop_mod.local_name = Some(local.name.clone());
                workshop_mod.local_addon_key = Some(local.addon_key.clone());
                matched.push(workshop_mod);
            }
            None => missing.push(workshop_mod),
        }
    }
    let (mut matched_dlc, mut missing_dlc) = (Vec::new(), Vec::new());
    for mut cdlc in requested_dlc {
        match local_dlcs
            .iter()
            .find(|dlc| dlc.steam_app_id() == cdlc.steam_app_id)
        {
            Some(local) => {
                cdlc.local_addon_key = Some(local.java_name().to_owned());
                matched_dlc.push(cdlc);
            }
            None => missing_dlc.push(cdlc),
        }
    }

    let preset_name = preset_name(html);
    Ok(ImportResult {
        // Mirrors service/Arma3LauncherPresetImporter.java:271
        preset_name: if java_is_blank(&preset_name) {
            DEFAULT_IMPORT_PRESET_NAME.to_owned()
        } else {
            preset_name
        },
        requested_count,
        requested_dlc_count,
        matched,
        missing,
        matched_dlc,
        missing_dlc,
    })
}

/// A local addon that declares a Workshop id.
struct LocalWorkshopMod {
    published_id: String,
    name: String,
    addon_key: String,
}

/// The Workshop id, name and key of every local addon with a `meta.cpp`, first addon per id.
/// Two addons whose folders differ only in case are read once.
// Mirrors service/Arma3LauncherPresetImporter.java:144-181
fn local_workshop_mods(local_addons: &[&Addon]) -> Vec<LocalWorkshopMod> {
    let mut mods: Vec<LocalWorkshopMod> = Vec::new();
    let mut visited = HashSet::new();
    for addon in local_addons {
        let Ok(directory) = absolute_normalized(&Path::new(&addon.path).join(&addon.name)) else {
            continue;
        };
        if !visited.insert(directory.to_string_lossy().to_lowercase()) {
            continue;
        }
        let meta_file = directory.join("meta.cpp");
        if !meta_file.is_file() {
            continue;
        }
        let text = match fs::read_to_string(&meta_file) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!("Skipping unreadable {}: {err}", meta_file.display());
                continue;
            }
        };
        let Some(published_id) = meta_cpp::published_id_strict(&text) else {
            continue;
        };
        if mods.iter().any(|known| known.published_id == published_id) {
            continue;
        }
        let parsed_name = meta_cpp::name_raw(&text)
            .map(|raw| java_trim(&meta_cpp::unescape_meta_value(raw)).to_owned())
            .filter(|name| !name.is_empty());
        let addon_key = if java_is_blank(&addon.key) {
            &addon.name
        } else {
            &addon.key
        };
        mods.push(LocalWorkshopMod {
            published_id: published_id.to_owned(),
            name: parsed_name.unwrap_or_else(|| addon.name.clone()),
            addon_key: addon_key.clone(),
        });
    }
    mods
}

/// The CDLCs among the local addons whose folder exists, first addon per CDLC.
// Mirrors service/Arma3LauncherPresetImporter.java:183-199
fn local_dlcs(local_addons: &[&Addon]) -> Vec<GameDlc> {
    let mut dlcs: Vec<GameDlc> = Vec::new();
    for addon in local_addons {
        let Some(dlc) = GameDlc::from_name(&addon.name) else {
            continue;
        };
        if !dlcs.contains(&dlc) && Path::new(&addon.path).join(&addon.name).is_dir() {
            dlcs.push(dlc);
        }
    }
    dlcs
}

/// The raw `href` of the row's `Link` anchor, in either attribute order, or `""`.
// Mirrors service/Arma3LauncherPresetImporter.java:201-208
fn extract_link(row: &str) -> &str {
    first_group(&LINK, row)
        .or_else(|| first_group(&LINK_REVERSED, row))
        .unwrap_or("")
}

/// The raw content of the row's `DisplayName` element, up to the first closing tag of any kind.
// Mirrors service/Arma3LauncherPresetImporter.java:210-213
fn extract_display_name(row: &str) -> &str {
    first_group(&DISPLAY_NAME, row).unwrap_or("")
}

// Mirrors service/Arma3LauncherPresetImporter.java:246-250
fn preset_name(html: &str) -> String {
    first_group(&PRESET_NAME, html)
        .map_or_else(|| DEFAULT_IMPORT_PRESET_NAME.to_owned(), clean_text)
}

fn first_group<'t>(regex: &Regex, text: &'t str) -> Option<&'t str> {
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|group| group.as_str())
}
