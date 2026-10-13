//! The radio plugin installer: where the TFAR or ACRE 2 plugin is, where `TeamSpeak` 3 is, and
//! the copy into `TeamSpeak`'s `plugins` folder.

use std::path::{Path, PathBuf};

use a3s_addons::{ScanOptions, scan};
use a3s_repo_files::services::ServiceError;
use a3s_tools::radio::{
    Acre2Form, Acre2Plan, HostInfo, InstallerIssue, RadioMod, TfarForm, TfarPlan, Ts3ProcessProbe,
    UNKNOWN, arma3_dir_from_exe, install_acre2, install_tfar, plugin_label, plugin_version,
    ts3_label, validate_acre2, validate_tfar,
};
use a3s_types::local::Profile;

use super::{Platform, blocking};
use crate::error::ContextError;
use crate::tasks::{Cx, TaskId, TaskKey, TaskKind};

/// What the radio plugin installer knows.
#[derive(Clone, Debug, Default)]
pub struct RadioState {
    /// The last [`inspect_radio`].
    pub inspection: Option<RadioInspection>,

    /// An inspection runs.
    pub inspecting: bool,

    /// The result of the last [`install_radio`].
    pub last_install: Option<RadioInstall>,
}

/// The plugin of a radio mod, `TeamSpeak` 3 and the plugin installed in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioInspection {
    /// The radio mod.
    pub radio: RadioMod,

    /// The mod folder holding the plugin, among the active profile's addons.
    pub mod_dir: Option<PathBuf>,

    /// The version of the mod's plugin dll.
    pub mod_version: Option<String>,

    /// The `TeamSpeak` 3 folder: the stored one, else the detected one.
    pub ts3_dir: Option<PathBuf>,

    /// `TeamSpeak`'s version and architecture, `Unknown` when not found.
    pub ts3_version: String,

    /// The plugin in `TeamSpeak`'s `plugins` folder.
    pub installed: Installed,

    /// A `TeamSpeak` 3 client runs.
    pub ts3_running: bool,

    /// What an install copies, or why it cannot run.
    pub plan: Result<RadioPlan, InstallerIssue>,
}

/// The plugin in `TeamSpeak`'s `plugins` folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// Not there.
    Missing,

    /// There.
    Present {
        /// Its version.
        version: Option<String>,

        /// It is byte for byte the mod's plugin.
        same_as_mod: bool,
    },
}

/// What an install copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RadioPlan {
    /// The TFAR plugin folder and userconfig.
    Tfar(TfarPlan),

    /// The ACRE 2 plugin dll.
    Acre2(Acre2Plan),
}

/// The result of an install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioInstall {
    /// The radio mod.
    pub radio: RadioMod,

    /// The failed steps; empty when everything was copied.
    pub errors: Vec<String>,
}

/// Inspects `radio` on a blocking thread into [`RadioState::inspection`]; returns `false` and
/// does nothing while an inspection runs.
pub fn inspect_radio(cx: &Cx, radio: RadioMod) -> bool {
    let (profile, stored, platform) = {
        let mut state = cx.state().write();
        if state.tools.radio.inspecting {
            return false;
        }
        state.tools.radio.inspecting = true;
        let options = &state.client.configuration();
        let stored = match radio {
            RadioMod::Tfar => options.tfar_options.ts3_path.clone(),
            RadioMod::Acre2 => options.acre2_options.ts3_path.clone(),
        };
        (
            state.client.active_profile().cloned(),
            stored,
            state.tools.platform.clone(),
        )
    };
    let cx = cx.clone();
    cx.handle().clone().spawn_blocking(move || {
        let profile = profile.unwrap_or_else(|| Profile::new(""));
        let inspection = inspect(radio, &profile, stored.as_deref(), &platform);
        {
            let mut state = cx.state().write();
            state.tools.radio.inspection = Some(inspection);
            state.tools.radio.inspecting = false;
        }
        cx.shared.events.repaint();
    });
    true
}

/// Stores the `TeamSpeak` 3 folder of `radio`.
pub fn set_ts3_dir(cx: &Cx, radio: RadioMod, folder: &Path) -> Result<(), ServiceError> {
    let value = Some(folder.display().to_string());
    let mut state = cx.state().write();
    match radio {
        RadioMod::Tfar => state.client.set_tfar_ts3_path(value),
        RadioMod::Acre2 => state.client.set_acre2_ts3_path(value),
    }
}

/// Copies the plugin as `plan` says; the result goes to [`RadioState::last_install`].
pub fn install_radio(cx: &Cx, plan: RadioPlan) -> Result<TaskId, ContextError> {
    cx.spawn_task(
        TaskKey::new(TaskKind::InstallRadioPlugin),
        true,
        move |task| async move {
            let install = blocking(&task, move |_| match plan {
                RadioPlan::Tfar(plan) => {
                    let report = install_tfar(&plan);
                    RadioInstall {
                        radio: RadioMod::Tfar,
                        errors: [report.userconfig, report.plugin]
                            .into_iter()
                            .filter_map(Result::err)
                            .map(|err| err.to_string())
                            .collect(),
                    }
                }
                RadioPlan::Acre2(plan) => RadioInstall {
                    radio: RadioMod::Acre2,
                    errors: install_acre2(&plan)
                        .plugin
                        .err()
                        .map(|err| err.to_string())
                        .into_iter()
                        .collect(),
                },
            })
            .await?;
            task.cx.state().write().tools.radio.last_install = Some(install);
            Ok(())
        },
    )
}

/// A process probe that answers what was probed once before validation.
struct Probed(bool);

impl Ts3ProcessProbe for Probed {
    fn is_ts3_running(&self) -> bool {
        self.0
    }
}

fn inspect(
    radio: RadioMod,
    profile: &Profile,
    stored_ts3: Option<&str>,
    platform: &Platform,
) -> RadioInspection {
    let ts3_dir = stored_ts3
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| platform.ts3_dir());
    let ts3_text = ts3_dir
        .as_deref()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let mod_dir = find_radio_mod(radio, profile);
    let plugin_dir = mod_dir
        .as_deref()
        .map(|dir| dir.join(plugin_folder(radio)).display().to_string())
        .unwrap_or_default();
    let ts3_running = platform.is_ts3_running();

    // The plugin is a Windows dll: TeamSpeak runs natively on Windows and under Wine elsewhere,
    // so the folders are checked as on Windows everywhere.
    let host = HostInfo {
        is_windows: true,
        ..HostInfo::detect()
    };
    let probe = Probed(ts3_running);
    let plan = match radio {
        RadioMod::Tfar => {
            let form = TfarForm {
                arma3_dir: arma3_dir_from_exe(profile.launcher_options.arma3_exe_path.as_deref())
                    .unwrap_or_default(),
                ts3_dir: ts3_text.clone(),
                plugin_dir: plugin_dir.clone(),
                userconfig_dir: mod_dir
                    .as_deref()
                    .map(|dir| dir.join("userconfig").display().to_string())
                    .unwrap_or_default(),
            };
            validate_tfar(&form, &host, &probe).map(RadioPlan::Tfar)
        }
        RadioMod::Acre2 => {
            let form = Acre2Form {
                ts3_dir: ts3_text.clone(),
                plugin_dir: plugin_dir.clone(),
            };
            validate_acre2(&form, &host, &probe).map(RadioPlan::Acre2)
        }
    };

    let dll = match plugin_label(radio, &ts3_text, &plugin_dir) {
        label if label == UNKNOWN => plugin_dlls(radio)[0].to_owned(),
        label => label,
    };
    let mod_dll = mod_dir
        .as_deref()
        .map(|dir| dir.join(plugin_folder(radio)).join(&dll));
    let installed_dll = ts3_dir.as_deref().map(|dir| dir.join("plugins").join(&dll));
    let installed = match &installed_dll {
        Some(path) if path.is_file() => Installed::Present {
            version: plugin_version(path),
            same_as_mod: mod_dll
                .as_deref()
                .is_some_and(|mod_dll| same_bytes(mod_dll, path)),
        },
        _ => Installed::Missing,
    };
    RadioInspection {
        radio,
        mod_version: mod_dll.as_deref().and_then(plugin_version),
        mod_dir,
        ts3_version: ts3_label(&ts3_text),
        ts3_dir,
        installed,
        ts3_running,
        plan,
    }
}

/// The folder of a radio mod that holds its plugin.
fn plugin_folder(radio: RadioMod) -> &'static str {
    match radio {
        RadioMod::Tfar => "plugins",
        RadioMod::Acre2 => "plugin",
    }
}

/// The plugin dlls of a radio mod, 64-bit first.
fn plugin_dlls(radio: RadioMod) -> [&'static str; 2] {
    match radio {
        RadioMod::Tfar => ["task_force_radio_win64.dll", "task_force_radio_win32.dll"],
        RadioMod::Acre2 => ["acre2_win64.dll", "acre2_win32.dll"],
    }
}

/// The first addon of the profile's search folders that holds the plugin of `radio`.
fn find_radio_mod(radio: RadioMod, profile: &Profile) -> Option<PathBuf> {
    let found = scan(profile, &ScanOptions::default());
    found
        .registry
        .addons()
        .map(|addon| Path::new(&addon.path).join(&addon.name))
        .find(|dir| {
            plugin_dlls(radio)
                .iter()
                .any(|dll| dir.join(plugin_folder(radio)).join(dll).is_file())
        })
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
