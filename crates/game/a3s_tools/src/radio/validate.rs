//! First-page validation of the installer wizards.
//!
//! Java does not stop at the first failure: after an if/else chain over the fields it runs
//! independent checks that each overwrite the message, so the **last** failing check wins. The
//! functions here return that same winning issue.

use std::fmt;
use std::path::{Path, PathBuf};

use super::specs::{HostInfo, plugin_label, ts3_label};
use super::{RadioMod, UNKNOWN};

/// Tells whether a TS3 client (`ts3client_win32.exe` or `ts3client_win64.exe`) is
/// running. Implemented by the platform layer.
pub trait Ts3ProcessProbe {
    /// Whether a TS3 client process is running.
    fn is_ts3_running(&self) -> bool;
}

/// The TFAR wizard's folder fields, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TfarForm {
    /// Arma 3 installation folder.
    pub arma3_dir: String,

    /// TS3 installation folder.
    pub ts3_dir: String,

    /// `@task_force_radio\plugins` folder.
    pub plugin_dir: String,

    /// `@task_force_radio\userconfig` folder.
    pub userconfig_dir: String,
}

/// The ACRE 2 wizard's folder fields, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Acre2Form {
    /// TS3 installation folder.
    pub ts3_dir: String,

    /// `@ACRE2\plugin` folder.
    pub plugin_dir: String,
}

/// What a validated TFAR form installs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TfarPlan {
    /// Arma 3 installation folder; receives `userconfig`.
    pub arma3_dir: PathBuf,

    /// TS3 installation folder; its `plugins` folder receives the plugin folder.
    pub ts3_dir: PathBuf,

    /// The mod's `userconfig` folder.
    pub userconfig_dir: PathBuf,

    /// The mod's `plugins` folder, copied whole.
    pub plugin_dir: PathBuf,
}

/// What a validated ACRE 2 form installs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acre2Plan {
    /// TS3 installation folder; its `plugins` folder receives the plugin.
    pub ts3_dir: PathBuf,

    /// The plugin dll matching the TS3 architecture.
    pub plugin_file: PathBuf,
}

/// Why a wizard cannot proceed. Its Display is the warning text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallerIssue {
    /// The wizard that raised it.
    pub radio: RadioMod,

    /// What is wrong.
    pub kind: IssueKind,
}

/// What is wrong with an installer form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// The host is not Windows.
    Unsupported,

    /// The Arma 3 field is empty (TFAR).
    Arma3DirMissing,

    /// The TS3 field is empty.
    Ts3DirMissing,

    /// The plugin field is empty.
    PluginDirMissing,

    /// The userconfig field is empty (TFAR).
    UserconfigDirMissing,

    /// The TS3 folder holds neither client executable.
    Ts3ArchUnknown,

    /// The TS3 folder has no `plugins` folder.
    Ts3PluginsFolderMissing,

    /// The plugin folder lacks the expected dll; `None` when the dll is unknown.
    PluginMissing {
        /// The expected dll file name.
        file: Option<String>,
    },

    /// `task_force_radio/radio_settings.hpp` is missing from the userconfig folder (TFAR).
    UserconfigHppMissing,

    /// A TS3 client is running.
    Ts3Running,
}

impl InstallerIssue {
    /// Title of the warning box. Java titles the unsupported-system warning of the ACRE 2 wizard
    /// `ACRE installer`, every other ACRE 2 warning `ACRE 2 installer`.
    // Mirrors ui/tools/acre2/FirstPageACRE2InstallerDialog.java:291, 328
    pub fn dialog_title(&self) -> &'static str {
        match (self.radio, &self.kind) {
            (RadioMod::Tfar, _) => "TFAR installer",
            (RadioMod::Acre2, IssueKind::Unsupported) => "ACRE installer",
            (RadioMod::Acre2, _) => "ACRE 2 installer",
        }
    }
}

impl fmt::Display for InstallerIssue {
    // Mirrors ui/tools/tfar/FirstPageTFARInstallerPanel.java:466-513 and
    // ui/tools/acre2/FirstPageACRE2InstallerDialog.java:288-324
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.radio.display_name();
        match &self.kind {
            IssueKind::Unsupported => f.write_str("This feature is not supported for your system"),
            IssueKind::Arma3DirMissing => f.write_str("ArmA 3 installation directory is missing."),
            IssueKind::Ts3DirMissing => f.write_str("TS3 installation directory is missing."),
            IssueKind::PluginDirMissing => write!(f, "{name} plugin directory is missing."),
            IssueKind::UserconfigDirMissing => f.write_str("TFAR userconfig directory is missing."),
            IssueKind::Ts3ArchUnknown => f.write_str("Can't determine between TS3 32/64 bit."),
            IssueKind::Ts3PluginsFolderMissing => {
                f.write_str("TS3 installation directory is missing \\plugins folder")
            }
            IssueKind::PluginMissing { file: None } => write!(f, "{name} plugin is missing."),
            IssueKind::PluginMissing { file: Some(file) } => {
                write!(f, "{name} plugin {file} is missing.")
            }
            IssueKind::UserconfigHppMissing => {
                f.write_str("TFAR userconfig file radio_settings.hpp not found")
            }
            IssueKind::Ts3Running => f.write_str("TS3 is running and must be close to proceed."),
        }
    }
}

impl std::error::Error for InstallerIssue {}

/// Validates the TFAR form, returning the issue Java would show or the install plan.
///
/// The process probe runs only on Windows.
// Mirrors ui/tools/tfar/FirstPageTFARInstallerPanel.java:461-528
pub fn validate_tfar(
    form: &TfarForm,
    host: &HostInfo,
    probe: &dyn Ts3ProcessProbe,
) -> Result<TfarPlan, InstallerIssue> {
    let issue = |kind| InstallerIssue {
        radio: RadioMod::Tfar,
        kind,
    };
    if !host.is_windows {
        return Err(issue(IssueKind::Unsupported));
    }
    let plugin = plugin_label(RadioMod::Tfar, &form.ts3_dir, &form.plugin_dir);

    let mut winner = if form.arma3_dir.is_empty() {
        Some(IssueKind::Arma3DirMissing)
    } else if form.ts3_dir.is_empty() {
        Some(IssueKind::Ts3DirMissing)
    } else if form.plugin_dir.is_empty() {
        Some(IssueKind::PluginDirMissing)
    } else if form.userconfig_dir.is_empty() {
        Some(IssueKind::UserconfigDirMissing)
    } else if ts3_label(&form.ts3_dir) == UNKNOWN {
        Some(IssueKind::Ts3ArchUnknown)
    } else {
        None
    };
    if !exists_below(&form.ts3_dir, &["plugins"]) {
        winner = Some(IssueKind::Ts3PluginsFolderMissing);
    }
    if let Some(kind) = plugin_issue(&form.plugin_dir, &plugin) {
        winner = Some(kind);
    }
    if !exists_below(
        &form.userconfig_dir,
        &["task_force_radio", "radio_settings.hpp"],
    ) {
        winner = Some(IssueKind::UserconfigHppMissing);
    }
    if probe.is_ts3_running() {
        winner = Some(IssueKind::Ts3Running);
    }

    match winner {
        Some(kind) => Err(issue(kind)),
        None => Ok(TfarPlan {
            arma3_dir: PathBuf::from(&form.arma3_dir),
            ts3_dir: PathBuf::from(&form.ts3_dir),
            userconfig_dir: PathBuf::from(&form.userconfig_dir),
            plugin_dir: PathBuf::from(&form.plugin_dir),
        }),
    }
}

/// Validates the ACRE 2 form, returning the issue Java would show or the install plan.
///
/// The process probe runs only on Windows.
// Mirrors ui/tools/acre2/FirstPageACRE2InstallerDialog.java:283-337
pub fn validate_acre2(
    form: &Acre2Form,
    host: &HostInfo,
    probe: &dyn Ts3ProcessProbe,
) -> Result<Acre2Plan, InstallerIssue> {
    let issue = |kind| InstallerIssue {
        radio: RadioMod::Acre2,
        kind,
    };
    if !host.is_windows {
        return Err(issue(IssueKind::Unsupported));
    }
    let plugin = plugin_label(RadioMod::Acre2, &form.ts3_dir, &form.plugin_dir);

    let mut winner = if form.ts3_dir.is_empty() {
        Some(IssueKind::Ts3DirMissing)
    } else if form.plugin_dir.is_empty() {
        Some(IssueKind::PluginDirMissing)
    } else if ts3_label(&form.ts3_dir) == UNKNOWN {
        Some(IssueKind::Ts3ArchUnknown)
    } else {
        None
    };
    if !exists_below(&form.ts3_dir, &["plugins"]) {
        winner = Some(IssueKind::Ts3PluginsFolderMissing);
    }
    if let Some(kind) = plugin_issue(&form.plugin_dir, &plugin) {
        winner = Some(kind);
    }
    if probe.is_ts3_running() {
        winner = Some(IssueKind::Ts3Running);
    }

    match winner {
        Some(kind) => Err(issue(kind)),
        None => Ok(Acre2Plan {
            ts3_dir: PathBuf::from(&form.ts3_dir),
            plugin_file: Path::new(&form.plugin_dir).join(plugin),
        }),
    }
}

/// The issue when `plugin_dir/plugin` does not exist. An [`UNKNOWN`] plugin is missing without
/// probing a file of that name.
fn plugin_issue(plugin_dir: &str, plugin: &str) -> Option<IssueKind> {
    if plugin == UNKNOWN {
        Some(IssueKind::PluginMissing { file: None })
    } else if exists_below(plugin_dir, &[plugin]) {
        None
    } else {
        Some(IssueKind::PluginMissing {
            file: Some(plugin.to_owned()),
        })
    }
}

/// Whether `dir/<parts…>` exists. An empty `dir` contains nothing (Java would resolve the
/// remainder against the filesystem root).
fn exists_below(dir: &str, parts: &[&str]) -> bool {
    if dir.is_empty() {
        return false;
    }
    let mut path = PathBuf::from(dir);
    path.extend(parts);
    path.exists()
}
