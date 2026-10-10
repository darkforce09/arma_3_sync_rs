use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::constants::{DEFAULT_PROFILE_NAME, GAME_VERSION_ARMA3};
use crate::java_string::compare_ignore_case;
use crate::{FavoriteServer, Protocol};

/// Client configuration (`fr.soe.a3s.domain.configration.Configuration`, `a3s.cfg`).
///
/// The deprecated Java fields `addonSearchDirectoryPaths` and `launcherOptions` are not part of
/// this type; the legacy decoder returns them in
/// [`super::legacy::LegacyConfiguration`] for migration into the active profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    /// Main window height.
    pub height: i32,

    /// Main window width.
    pub width: i32,

    /// Active profile.
    pub profile_name: String,
    pub game_version: String,
    pub server_name: Option<String>,
    pub default_modset: Option<String>,

    /// Addon list shown as a tree rather than a flat list.
    pub view_mode_tree: bool,

    /// Sorted by name, case-insensitively; see [`Self::sort_favorite_servers`].
    pub favorite_servers: Vec<FavoriteServer>,
    pub external_applications: Vec<ExternalApplication>,
    pub acre_options: AcreOptions,
    pub acre2_options: Acre2Options,
    pub tfar_options: TfarOptions,
    pub rpt_options: RptOptions,
    pub bikey_extract_options: BikeyExtractOptions,
    pub proxy: Proxy,
}

impl Default for Configuration {
    /// The Java field initializer values.
    // Mirrors domain/configration/Configuration.java:16-34
    fn default() -> Self {
        Self {
            height: 0,
            width: 0,
            profile_name: DEFAULT_PROFILE_NAME.to_owned(),
            game_version: GAME_VERSION_ARMA3.to_owned(),
            server_name: None,
            default_modset: None,
            view_mode_tree: true,
            favorite_servers: Vec::new(),
            external_applications: Vec::new(),
            acre_options: AcreOptions::default(),
            acre2_options: Acre2Options::default(),
            tfar_options: TfarOptions::default(),
            rpt_options: RptOptions::default(),
            bikey_extract_options: BikeyExtractOptions::default(),
            proxy: Proxy::default(),
        }
    }
}

impl Configuration {
    /// Stable sort of [`Self::favorite_servers`] by name, `compareToIgnoreCase`, a missing name
    /// sorting as `""`.
    ///
    /// Java sorts the list in place on every `getFavoriteServers()` call; indices of an
    /// autoconfig export refer to this order.
    // Mirrors domain/configration/Configuration.java:60-66
    pub fn sort_favorite_servers(&mut self) {
        self.favorite_servers.sort_by(favorite_server_java_cmp);
    }
}

/// `FavoriteServer.compareTo`: `compareToIgnoreCase` of the names, null as `""`.
// Mirrors domain/configration/FavoriteServer.java:72-77
pub fn favorite_server_java_cmp(a: &FavoriteServer, b: &FavoriteServer) -> Ordering {
    compare_ignore_case(
        a.name.as_deref().unwrap_or_default(),
        b.name.as_deref().unwrap_or_default(),
    )
}

/// Launch settings of a profile (`fr.soe.a3s.domain.configration.LauncherOptions`).
///
/// The deprecated Java fields `noFilePatching`, `runBeta` and `xpCompatibilityMode` are dropped.
// Mirrors domain/configration/LauncherOptions.java:10-38
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherOptions {
    /// `-name=`.
    pub game_profile: Option<String>,
    pub show_script_errors: bool,
    pub no_pause: bool,
    pub file_patching: bool,
    pub window_mode: bool,
    pub check_signatures: bool,
    pub auto_restart: bool,

    /// Windows only: elevate the game executable through UAC.
    pub launch_arma3_as_administrator: bool,
    pub mission_file: bool,
    pub mission_file_path: Option<String>,
    pub max_memory_selection: Option<String>,
    pub cpu_count_selection: i32,
    pub ex_threads_selection: Option<String>,
    pub malloc_selection: Option<String>,

    /// `enableHT`.
    pub enable_ht: bool,
    pub huge_pages: bool,
    pub no_splash_screen: bool,
    pub default_world: bool,
    pub nologs: bool,
    pub arma3_exe_path: Option<String>,
    pub steam_exe_path: Option<String>,
}

/// A program started together with the game
/// (`fr.soe.a3s.domain.configration.ExternalApplication`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalApplication {
    pub name: Option<String>,
    pub executable_path: Option<String>,
    pub parameters: Option<String>,
    pub enable: bool,
}

impl ExternalApplication {
    /// Java `compareTo`: `compareToIgnoreCase` of the names, null as `""`.
    // Mirrors domain/configration/ExternalApplication.java:41-46
    pub fn java_cmp(&self, other: &Self) -> Ordering {
        compare_ignore_case(
            self.name.as_deref().unwrap_or_default(),
            other.name.as_deref().unwrap_or_default(),
        )
    }
}

/// ACRE installer paths (`fr.soe.a3s.domain.configration.AcreOptions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcreOptions {
    pub ts3_path: Option<String>,
    pub acre_plugin_path: Option<String>,
    pub acre_userconfig_path: Option<String>,
}

/// ACRE 2 installer paths (`fr.soe.a3s.domain.configration.Acre2Options`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Acre2Options {
    pub ts3_path: Option<String>,
    pub acre2_plugin_path: Option<String>,
}

/// TFAR installer paths (`fr.soe.a3s.domain.configration.TfarOptions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TfarOptions {
    pub ts3_path: Option<String>,
    pub tfar_plugin_path: Option<String>,
    pub tfar_userconfig_path: Option<String>,
}

/// RPT log viewer settings (`fr.soe.a3s.domain.configration.RptOptions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RptOptions {
    pub rpt_path: Option<String>,
}

/// Bikey extractor paths (`fr.soe.a3s.domain.configration.BikeyExtractOptions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BikeyExtractOptions {
    pub source_directory_path: Option<String>,
    pub target_directory_path: Option<String>,
}

/// Proxy settings (`fr.soe.a3s.domain.configration.Proxy`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Proxy {
    pub enable_proxy: bool,

    /// A `Socks` or `Http` protocol; may be absent.
    pub proxy_protocol: Option<Protocol>,
}
