//! Detection of the Arma 3, Steam and `TeamSpeak` 3 install folders, the Arma 3 RPT log folder and
//! the Java Arma3Sync install folders.
//!
//! Detection never fails: an unreadable source counts as "not found" (and is logged at `debug`).
//! On a non-Windows [`PlatformEnv`] the registry is never queried, as in Java.

use std::path::PathBuf;

use a3s_foundation::paths::{Os, PlatformEnv};

use crate::registry::{Hive, RegQuery, RegistryRead};
use crate::steam::{find_arma3, proton_paths};

/// File name of the 64-bit Arma 3 executable inside the Arma 3 folder.
// Mirrors constant/GameExecutables.java:5
pub const ARMA3_X64_EXE: &str = "arma3_x64.exe";

const ARMA3_KEY: &str = r"SOFTWARE\Wow6432Node\Bohemia Interactive\Arma 3";
const STEAM_MACHINE_KEY: &str = r"SOFTWARE\Wow6432Node\Valve\Steam";
const STEAM_USER_KEY: &str = r"Software\Valve\Steam";
const TS3_USER_KEY: &str = r"Software\TeamSpeak 3 Client";
const TS3_MACHINE_KEY: &str = r"SOFTWARE\TeamSpeak 3 Client";
const TS3_MACHINE_32_BIT_KEY: &str = r"SOFTWARE\Wow6432Node\TeamSpeak 3 Client";

/// Registry keys of the Java client's NSIS installer (both publishers it has had), 32-bit view
/// first: NSIS has no `SetRegView`, so a 64-bit Windows stores them under `Wow6432Node`.
// Mirrors installer/nsis/Arma3Sync.nsi:36, 69, 141 and installer/nsis/include/Version.nsh:6-8
const JAVA_INSTALL_KEYS: [&str; 4] = [
    r"SOFTWARE\Wow6432Node\Virtuelle Panzerbrigade 21 (vPzBrig21)\Arma3Sync",
    r"SOFTWARE\Wow6432Node\Arma3Sync community\Arma3Sync",
    r"SOFTWARE\Virtuelle Panzerbrigade 21 (vPzBrig21)\Arma3Sync",
    r"SOFTWARE\Arma3Sync community\Arma3Sync",
];

/// Reads a registry string on Windows; `None` elsewhere and on any failure.
fn read(
    env: &PlatformEnv,
    registry: &dyn RegistryRead,
    hive: Hive,
    key: &str,
    value: Option<&str>,
) -> Option<String> {
    if env.os() != Os::Windows {
        return None;
    }
    let query = RegQuery::new(hive, key, value);
    registry.read_string(&query).unwrap_or_else(|err| {
        tracing::debug!("registry query {query} failed: {err}");
        None
    })
}

/// The Arma 3 folder recorded by its installer. Not checked for existence (Java did not).
// Mirrors dao/ConfigurationDAO.java:69-76
pub fn arma3_dir_from_registry(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<PathBuf> {
    read(env, registry, Hive::LocalMachine, ARMA3_KEY, Some("MAIN")).map(PathBuf::from)
}

/// The Steam folder: the machine-wide `InstallPath`, else the per-user `SteamPath` (which Steam
/// writes with `/` separators, turned into `\`).
// Mirrors dao/ConfigurationDAO.java:60-67
pub fn steam_dir_from_registry(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<PathBuf> {
    read(
        env,
        registry,
        Hive::LocalMachine,
        STEAM_MACHINE_KEY,
        Some("InstallPath"),
    )
    .or_else(|| {
        read(
            env,
            registry,
            Hive::CurrentUser,
            STEAM_USER_KEY,
            Some("SteamPath"),
        )
        .map(|path| path.replace('/', "\\"))
    })
    .map(PathBuf::from)
}

/// The `TeamSpeak` 3 client folder: the per-user install's default value, else the all-users
/// install (64-bit, then 32-bit view).
// Mirrors dao/ConfigurationDAO.java:78-85
pub fn ts3_dir(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<PathBuf> {
    [
        (Hive::CurrentUser, TS3_USER_KEY),
        (Hive::LocalMachine, TS3_MACHINE_KEY),
        (Hive::LocalMachine, TS3_MACHINE_32_BIT_KEY),
    ]
    .into_iter()
    .find_map(|(hive, key)| read(env, registry, hive, key, None))
    .map(PathBuf::from)
}

/// The Arma 3 RPT log folder.
///
/// Windows: `<parent of %APPDATA%>\Local\Arma 3`, not checked for existence (Java did not).
/// Elsewhere: the Proton prefix's `AppData/Local/Arma 3` of a Steam-installed Arma 3, if it
/// exists.
// Mirrors dao/ConfigurationDAO.java:161-172
pub fn rpt_dir(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<PathBuf> {
    if env.os() == Os::Windows {
        return env
            .var("APPDATA")
            .and_then(windows_rpt_dir_from_appdata)
            .map(PathBuf::from);
    }
    find_arma3(env, registry)
        .as_ref()
        .and_then(proton_paths)
        .map(|paths| paths.local_appdata_arma3)
        .filter(|dir| dir.is_dir())
}

/// `<parent of appdata>\Local\Arma 3`, computed on the string.
///
/// Trailing separators are stripped and the string is cut at its last `\` or `/`, so it works on
/// any host. `None` when `appdata` has no parent (`C:\`, `Roaming`).
// Mirrors dao/ConfigurationDAO.java:166-169
pub fn windows_rpt_dir_from_appdata(appdata: &str) -> Option<String> {
    let trimmed = appdata.trim_end_matches(['\\', '/']);
    let parent = &trimmed[..trimmed.rfind(['\\', '/'])?];
    (!parent.is_empty()).then(|| format!(r"{parent}\Local\Arma 3"))
}

/// The Arma 3 folder: the installer's registry value on Windows (returned as is), else the Steam
/// library that has Arma 3 installed (on every OS).
pub fn detect_arma3_dir(env: &PlatformEnv, registry: &dyn RegistryRead) -> Option<PathBuf> {
    arma3_dir_from_registry(env, registry)
        .or_else(|| find_arma3(env, registry).map(|app| app.install_dir))
}

/// The install folders of the Java client recorded by its Windows installer, in lookup order,
/// without case-insensitive duplicates. Empty on other OSes.
pub fn java_install_dirs(env: &PlatformEnv, registry: &dyn RegistryRead) -> Vec<PathBuf> {
    let mut dirs: Vec<String> = Vec::new();
    for key in JAVA_INSTALL_KEYS {
        if let Some(dir) = read(env, registry, Hive::LocalMachine, key, Some("InstallDir"))
            && !dirs
                .iter()
                .any(|known| known.to_lowercase() == dir.to_lowercase())
        {
            dirs.push(dir);
        }
    }
    dirs.into_iter().map(PathBuf::from).collect()
}
