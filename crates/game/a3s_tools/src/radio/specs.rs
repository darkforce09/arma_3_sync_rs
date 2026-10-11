//! The "System specifications" panel of the installer wizards.

use std::path::Path;

use super::ts3::{is_ts3_32bit, is_ts3_64bit, ts3_version};
use super::{RadioMod, UNKNOWN};

/// The host facts the wizards show and check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// Whether the host runs Windows (the installers support nothing else).
    pub is_windows: bool,

    /// Operating system name, as Java's `os.name` (`Windows`, `Linux`, `Mac OS X`, …).
    pub os_name: String,

    /// Whether the operating system is 64-bit.
    pub is_64bit: bool,
}

impl HostInfo {
    /// The current host. On Windows the system counts as 64-bit when `ProgramFiles(x86)` is set;
    /// elsewhere when the target architecture name contains `64`.
    // Mirrors ui/tools/WizardDialog.java:134-149
    pub fn detect() -> Self {
        let is_windows = cfg!(windows);
        let os_name = match std::env::consts::OS {
            "windows" => "Windows",
            "linux" => "Linux",
            "macos" => "Mac OS X",
            other => other,
        };
        let is_64bit = if is_windows {
            std::env::var_os("ProgramFiles(x86)").is_some()
        } else {
            std::env::consts::ARCH.contains("64")
        };
        Self {
            is_windows,
            os_name: os_name.to_owned(),
            is_64bit,
        }
    }
}

/// The `OS:` value: `<os name> - 64 bit` or `<os name> - 32 bit`.
// Mirrors ui/tools/WizardDialog.java:145-149
pub fn os_label(host: &HostInfo) -> String {
    let bits = if host.is_64bit { 64 } else { 32 };
    format!("{} - {bits} bit", host.os_name)
}

/// The `TS3:` value: `<version> - 64 bit` or `<version> - 32 bit` (the version may itself be
/// [`UNKNOWN`]), or [`UNKNOWN`] when the field is empty or holds neither client executable.
// Mirrors ui/tools/WizardDialog.java:151-178
pub fn ts3_label(ts3_dir: &str) -> String {
    if ts3_dir.is_empty() {
        return UNKNOWN.to_owned();
    }
    let dir = Path::new(ts3_dir);
    let version = ts3_version(dir).unwrap_or_else(|| UNKNOWN.to_owned());
    if is_ts3_64bit(dir) {
        format!("{version} - 64 bit")
    } else if is_ts3_32bit(dir) {
        format!("{version} - 32 bit")
    } else {
        UNKNOWN.to_owned()
    }
}

/// The plugin value: the dll the installer expects, or [`UNKNOWN`] when either field is empty.
///
/// TFAR falls back to its 32-bit dll whenever the 64-bit client is absent; ACRE 2 needs one of
/// the two clients and is [`UNKNOWN`] otherwise.
// Mirrors ui/tools/tfar/FirstPageTFARInstallerPanel.java:346-361 and
// ui/tools/acre2/FirstPageACRE2InstallerDialog.java:222-243
pub fn plugin_label(radio: RadioMod, ts3_dir: &str, plugin_dir: &str) -> String {
    if plugin_dir.is_empty() || ts3_dir.is_empty() {
        return UNKNOWN.to_owned();
    }
    let dir = Path::new(ts3_dir);
    let label = match radio {
        RadioMod::Tfar if is_ts3_64bit(dir) => "task_force_radio_win64.dll",
        RadioMod::Tfar => "task_force_radio_win32.dll",
        RadioMod::Acre2 if is_ts3_64bit(dir) => "acre2_win64.dll",
        RadioMod::Acre2 if is_ts3_32bit(dir) => "acre2_win32.dll",
        RadioMod::Acre2 => UNKNOWN,
    };
    label.to_owned()
}

/// The three values of the "System specifications" panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemSpecs {
    /// See [`os_label`].
    pub os: String,

    /// See [`ts3_label`].
    pub ts3: String,

    /// See [`plugin_label`].
    pub plugin: String,
}

/// The panel's values for the current field contents.
pub fn system_specs(
    radio: RadioMod,
    host: &HostInfo,
    ts3_dir: &str,
    plugin_dir: &str,
) -> SystemSpecs {
    SystemSpecs {
        os: os_label(host),
        ts3: ts3_label(ts3_dir),
        plugin: plugin_label(radio, ts3_dir, plugin_dir),
    }
}
