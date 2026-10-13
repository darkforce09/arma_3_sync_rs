//! The Server keys window: the keys of the active profile's enabled mods, copied into a folder.

use a3s_foundation::error::UserFacing as _;
use a3s_tools::bikey::BikeyOutcome;
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row, primary_button};
use a3s_viewer_context::tools::{self, KeyCopy, KeyScan};
use a3s_viewer_context::{Cx, TaskId, TaskKey, TaskKind};
use egui::{Align, Layout, RichText, Ui};

use super::{file_name, finished, intro, window_toasts};
use crate::picker::FolderPicker;
use crate::screens::count_text;
use crate::toasts::Toaster;

pub(super) struct KeysWindow {
    /// The profile whose keys were last looked for since the window opened.
    searched_profile: Option<String>,
    copying: Option<TaskId>,
    picker: FolderPicker,
    toasts: Toaster,
}

impl Default for KeysWindow {
    fn default() -> Self {
        Self {
            searched_profile: None,
            copying: None,
            picker: FolderPicker::default(),
            toasts: window_toasts("keys"),
        }
    }
}

impl KeysWindow {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &Cx) {
        self.update(cx);
        let (scan, target, profile) = {
            let state = cx.state().read();
            (
                state.tools.keys.scan.clone(),
                tools::keys_target(&state),
                state.client.active_profile_name().to_owned(),
            )
        };
        let busy = cx
            .is_running(&TaskKey::new(TaskKind::ExtractBikeys))
            .is_some();
        // Opening the window or switching the profile looks for the keys again.
        if !busy && self.searched_profile.as_deref() != Some(profile.as_str()) {
            self.searched_profile = Some(profile.clone());
            find(cx);
        }
        let scan = scan.filter(|scan| scan.profile == profile);

        intro(
            ui,
            "Copies the server keys (.bikey) of the enabled mods into one folder.",
        );
        frames::group(ui, |ui| {
            form_row(ui, "From", |ui| {
                let mods = scan.as_ref().map_or(0, |scan| scan.mods.len());
                ui.label(format!(
                    "{} · {profile}",
                    count_text(mods, "enabled mod", "enabled mods")
                ));
            });
            frames::group_separator(ui);
            form_row(ui, "To", |ui| {
                if button(ui, "Change…").clicked() {
                    self.picker
                        .open(ui.ctx(), "Copy the keys to", target.as_deref());
                }
                match &target {
                    Some(target) => {
                        ui.label(target.display().to_string());
                    }
                    None => {
                        ui.weak("Not chosen");
                    }
                }
            });
            frames::group_separator(ui);
            form_row(ui, "Keys found", |ui| match &scan {
                Some(scan) => {
                    ui.label(scan.keys().len().to_string());
                }
                None => {
                    ui.spinner();
                }
            });
            if let Some(scan) = &scan {
                unsigned(ui, scan);
            }
        });
        ui.add_space(12.0);

        let keys = scan.as_ref().map_or(0, |scan| scan.keys().len());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let enabled = keys > 0 && target.is_some() && !busy;
            let label = format!("Copy {}", count_text(keys, "key", "keys"));
            if ui
                .add_enabled_ui(enabled, |ui| primary_button(ui, &label))
                .inner
                .clicked()
                && let Some(target) = target
            {
                match tools::copy_keys(cx, target) {
                    Ok(id) => self.copying = Some(id),
                    Err(err) => self.toasts.error(err.user_message()),
                }
            }
        });
        self.toasts.show_notes(ui);
    }

    /// Takes the chosen folder and reports a finished copy.
    fn update(&mut self, cx: &Cx) {
        if let Some(folder) = self.picker.poll()
            && let Err(err) = tools::set_keys_target(cx, &folder)
        {
            self.toasts.error(err.user_message());
        }
        if let Some(id) = self.copying
            && finished(cx, id)
        {
            self.copying = None;
            let copy = cx.state().read().tools.keys.last_copy.clone();
            if let Some(copy) = copy {
                self.report(&copy);
            }
        }
    }

    fn report(&mut self, copy: &KeyCopy) {
        match &copy.result {
            Ok(BikeyOutcome::Copied { count }) => self.toasts.success(format!(
                "Copied {} to {}",
                count_text(*count, "key", "keys"),
                file_name(&copy.target)
            )),
            Ok(BikeyOutcome::NoneFound) => self.toasts.warning("No keys to copy"),
            Ok(BikeyOutcome::Cancelled { copied, .. }) => self.toasts.info(format!(
                "Stopped after {}",
                count_text(*copied, "key", "keys")
            )),
            Err(message) => self.toasts.error(format!("Keys not copied: {message}")),
        }
    }
}

/// The mods without a key, behind a disclosure.
fn unsigned(ui: &mut Ui, scan: &KeyScan) {
    let names = scan.unsigned();
    if names.is_empty() {
        return;
    }
    frames::group_separator(ui);
    egui::CollapsingHeader::new(format!(
        "{} without a key",
        count_text(names.len(), "mod", "mods")
    ))
    .id_salt("a3s_viewer::keys::unsigned")
    .show(ui, |ui| {
        for name in names {
            ui.label(RichText::new(name).small());
        }
    });
}

/// Looks for the keys of the active profile.
fn find(cx: &Cx) {
    if let Err(err) = tools::find_keys(cx) {
        tracing::debug!("Keys not looked for: {err}");
    }
}
