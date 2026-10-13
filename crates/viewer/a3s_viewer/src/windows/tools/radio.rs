//! The Radio plugin window: installs the TFAR or ACRE 2 plugin of the mods into `TeamSpeak` 3.

use std::time::{Duration, Instant};

use a3s_foundation::error::UserFacing as _;
use a3s_tools::radio::{InstallerIssue, IssueKind, RadioMod};
use a3s_ui::frames;
use a3s_ui::theme::ProgressTone;
use a3s_ui::widgets::{button, form_row, primary_button, segmented, status_pill};
use a3s_viewer_context::tools::{self, Installed, RadioInspection};
use a3s_viewer_context::{Cx, TaskId, ViewerContext};
use egui::{Align, Layout, Ui};

use super::{file_name, finished, intro, warning_row, window_toasts};
use crate::picker::FolderPicker;
use crate::toasts::Toaster;

/// How often the window looks again, e.g. whether `TeamSpeak` still runs.
const REFRESH: Duration = Duration::from_secs(2);

/// The radio mods, in segment order.
const RADIOS: [RadioMod; 2] = [RadioMod::Tfar, RadioMod::Acre2];

pub(super) struct RadioWindow {
    /// The selected segment of [`RADIOS`].
    segment: usize,
    last_inspection: Option<Instant>,
    installing: Option<TaskId>,
    picker: FolderPicker,
    toasts: Toaster,
}

impl Default for RadioWindow {
    fn default() -> Self {
        Self {
            segment: 0,
            last_inspection: None,
            installing: None,
            picker: FolderPicker::default(),
            toasts: window_toasts("radio"),
        }
    }
}

impl RadioWindow {
    pub(super) fn show(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let cx = viewer.cx();
        let radio = RADIOS[self.segment];
        self.update(ui, cx, radio);
        let inspection = cx
            .state()
            .read()
            .tools
            .radio
            .inspection
            .clone()
            .filter(|inspection| inspection.radio == radio);

        intro(
            ui,
            "Installs the radio mod's TeamSpeak 3 plugin, so the in-game radios reach TeamSpeak.",
        );
        ui.vertical_centered(|ui| {
            if segmented(ui, &mut self.segment, &["TFAR", "ACRE 2"]).changed() {
                self.last_inspection = None;
            }
        });
        ui.add_space(12.0);
        let Some(inspection) = inspection else {
            ui.vertical_centered(|ui| ui.spinner());
            self.toasts.show(ui, viewer);
            return;
        };
        self.details(ui, &inspection);
        ui.add_space(8.0);
        if inspection.ts3_running {
            warning_row(ui, "TeamSpeak 3 is running. Quit it to install the plugin.");
        } else if let Err(issue) = &inspection.plan {
            warning_row(ui, &issue_text(issue));
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let label = match inspection.installed {
                Installed::Missing => "Install plugin",
                Installed::Present { .. } => "Update plugin",
            };
            let enabled = inspection.plan.is_ok() && self.installing.is_none();
            if ui
                .add_enabled_ui(enabled, |ui| primary_button(ui, label))
                .inner
                .clicked()
                && let Ok(plan) = inspection.plan.clone()
            {
                match tools::install_radio(cx, plan) {
                    Ok(id) => self.installing = Some(id),
                    Err(err) => self.toasts.error(err.user_message()),
                }
            }
        });
        self.toasts.show(ui, viewer);
    }

    /// Takes the chosen `TeamSpeak` folder, reports a finished install and inspects again when
    /// due.
    fn update(&mut self, ui: &Ui, cx: &Cx, radio: RadioMod) {
        if let Some(folder) = self.picker.poll() {
            match tools::set_ts3_dir(cx, radio, &folder) {
                Ok(()) => self.last_inspection = None,
                Err(err) => self.toasts.error(err.user_message()),
            }
        }
        if let Some(id) = self.installing
            && finished(cx, id)
        {
            self.installing = None;
            self.last_inspection = None;
            let install = cx.state().read().tools.radio.last_install.clone();
            if let Some(install) = install {
                let name = install.radio.display_name();
                if install.errors.is_empty() {
                    self.toasts.success(format!("{name} plugin installed"));
                } else {
                    self.toasts.error(format!(
                        "{name} plugin not installed: {}",
                        install.errors.join("; ")
                    ));
                }
            }
        }
        let due = self
            .last_inspection
            .is_none_or(|last| last.elapsed() >= REFRESH);
        if due && self.installing.is_none() && tools::inspect_radio(cx, radio) {
            self.last_inspection = Some(Instant::now());
        }
        ui.ctx().request_repaint_after(REFRESH);
    }

    fn details(&mut self, ui: &mut Ui, inspection: &RadioInspection) {
        frames::group(ui, |ui| {
            form_row(ui, "Plugin in mod", |ui| match &inspection.mod_dir {
                Some(dir) => {
                    let version = inspection
                        .mod_version
                        .as_deref()
                        .unwrap_or("Unknown version");
                    ui.label(format!("{version} · {}", file_name(dir)))
                        .on_hover_text(dir.display().to_string());
                }
                None => {
                    ui.weak("Mod not found in the mod folders");
                }
            });
            frames::group_separator(ui);
            form_row(ui, "TeamSpeak 3", |ui| {
                if button(ui, "Change…").clicked() {
                    self.picker.open(
                        ui.ctx(),
                        "TeamSpeak 3 folder",
                        inspection.ts3_dir.as_deref(),
                    );
                }
                match &inspection.ts3_dir {
                    Some(dir) => {
                        ui.label(dir.display().to_string());
                    }
                    None => {
                        ui.weak("Not found");
                    }
                }
            });
            frames::group_separator(ui);
            form_row(ui, "TeamSpeak version", |ui| {
                ui.label(&inspection.ts3_version);
            });
            frames::group_separator(ui);
            form_row(ui, "Installed plugin", |ui| match &inspection.installed {
                Installed::Missing => {
                    ui.weak("Not installed");
                }
                Installed::Present {
                    version,
                    same_as_mod,
                } => {
                    if *same_as_mod {
                        status_pill(ui, "Up to date", ProgressTone::Success);
                    }
                    ui.label(version.as_deref().unwrap_or("Unknown version"));
                }
            });
        });
    }
}

/// Why the plugin cannot be installed, in the window's words.
fn issue_text(issue: &InstallerIssue) -> String {
    let name = issue.radio.display_name();
    match &issue.kind {
        IssueKind::Unsupported => "Installing the plugin is not supported here.".to_owned(),
        IssueKind::Arma3DirMissing => {
            "Choose the Arma 3 executable in Settings › Game; TFAR copies its settings there."
                .to_owned()
        }
        IssueKind::Ts3DirMissing => "Choose the TeamSpeak 3 folder.".to_owned(),
        IssueKind::PluginDirMissing | IssueKind::PluginMissing { file: None } => {
            format!("No {name} mod with its plugin is in the mod folders.")
        }
        IssueKind::UserconfigDirMissing | IssueKind::UserconfigHppMissing => {
            format!("The {name} mod has no userconfig settings file.")
        }
        IssueKind::Ts3ArchUnknown => "The TeamSpeak 3 folder holds no TeamSpeak client.".to_owned(),
        IssueKind::Ts3PluginsFolderMissing => {
            "The TeamSpeak 3 folder has no plugins folder.".to_owned()
        }
        IssueKind::PluginMissing { file: Some(file) } => {
            format!("The {name} mod has no {file}.")
        }
        IssueKind::Ts3Running => {
            "TeamSpeak 3 is running. Quit it to install the plugin.".to_owned()
        }
    }
}
