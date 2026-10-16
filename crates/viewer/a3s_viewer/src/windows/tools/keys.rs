//! The Server keys window: the keys of the active profile's enabled mods, or of any folder,
//! copied into a folder.

use a3s_foundation::error::UserFacing as _;
use a3s_tools::bikey::BikeyOutcome;
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row, primary_button, segmented};
use a3s_viewer_context::tools::{self, KeyCopy, KeyScan, KeySource};
use a3s_viewer_context::{Cx, TaskId, TaskKey, TaskKind, ViewerContext};
use egui::{Align, Layout, RichText, Ui};

use super::{file_name, finished, intro, window_toasts};
use crate::picker::FolderPicker;
use crate::screens::count_text;
use crate::toasts::Toaster;

pub(super) struct KeysWindow {
    /// 0: the enabled mods; 1: a folder.
    from: usize,

    /// The source whose keys were last looked for since the window opened.
    searched: Option<KeySource>,
    copying: Option<TaskId>,
    picker: FolderPicker,
    source_picker: FolderPicker,
    toasts: Toaster,
}

impl Default for KeysWindow {
    fn default() -> Self {
        Self {
            from: 0,
            searched: None,
            copying: None,
            picker: FolderPicker::default(),
            source_picker: FolderPicker::default(),
            toasts: window_toasts("keys"),
        }
    }
}

impl KeysWindow {
    pub(super) fn show(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let cx = viewer.cx();
        self.update(cx);
        let (scan, target, profile, folder) = {
            let state = cx.state().read();
            (
                state.tools.keys.scan.clone(),
                tools::keys_target(&state),
                state.client.active_profile_name().to_owned(),
                tools::keys_source_folder(&state),
            )
        };
        let busy = cx
            .is_running(&TaskKey::new(TaskKind::ExtractBikeys))
            .is_some();
        let source = if self.from == 0 {
            Some(KeySource::Profile(profile.clone()))
        } else {
            folder.clone().map(KeySource::Folder)
        };
        // Opening the window, switching the profile or choosing a folder looks for the keys again.
        if !busy
            && let Some(source) = &source
            && self.searched.as_ref() != Some(source)
        {
            self.searched = Some(source.clone());
            find(cx, source);
        }
        let scan = scan.filter(|scan| source.as_ref() == Some(&scan.source));

        intro(
            ui,
            "Copies the server keys (.bikey) of the enabled mods, or of a folder, into one folder.",
        );
        segmented(ui, &mut self.from, &["Enabled mods", "A folder"]);
        ui.add_space(8.0);
        frames::group(ui, |ui| {
            form_row(ui, "From", |ui| {
                if self.from == 0 {
                    let mods = scan.as_ref().map_or(0, |scan| scan.mods.len());
                    ui.label(format!(
                        "{} · {profile}",
                        count_text(mods, "enabled mod", "enabled mods")
                    ));
                } else {
                    if button(ui, "Choose…").clicked() {
                        self.source_picker
                            .open(ui.ctx(), "Copy the keys from", folder.as_deref());
                    }
                    match &folder {
                        Some(folder) => {
                            ui.label(folder.display().to_string());
                        }
                        None => {
                            ui.weak("Not chosen");
                        }
                    }
                }
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
            form_row(ui, "Keys found", |ui| match (&scan, &source) {
                (Some(scan), _) => {
                    ui.label(scan.keys().len().to_string());
                }
                (None, Some(_)) => {
                    ui.spinner();
                }
                (None, None) => {
                    ui.weak("Choose a folder");
                }
            });
            if let Some(scan) = &scan
                && self.from == 0
            {
                unsigned(ui, scan);
            }
        });
        ui.add_space(12.0);

        let keys = scan.as_ref().map_or(0, |scan| scan.keys().len());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let enabled = keys > 0 && target.is_some() && !busy;
            let label = format!("Copy {}", count_text(keys, "key", "keys"));
            let reason = if busy {
                "The keys are being looked for or copied."
            } else if target.is_none() {
                "Choose where to copy the keys (To)."
            } else {
                "No keys were found."
            };
            if ui
                .add_enabled_ui(enabled, |ui| primary_button(ui, &label))
                .inner
                .on_disabled_hover_text(reason)
                .clicked()
                && let Some(target) = target
            {
                match tools::copy_keys(cx, target) {
                    Ok(id) => self.copying = Some(id),
                    Err(err) => self.toasts.error(err.user_message()),
                }
            }
        });
        self.toasts.show(ui, viewer);
    }

    /// Takes the chosen folders and reports a finished copy.
    fn update(&mut self, cx: &Cx) {
        if let Some(folder) = self.source_picker.poll()
            && let Err(err) = tools::set_keys_source(cx, &folder)
        {
            self.toasts.error(err.user_message());
        }
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

/// Looks for the keys of `source`.
fn find(cx: &Cx, source: &KeySource) {
    let found = match source {
        KeySource::Profile(_) => tools::find_keys(cx),
        KeySource::Folder(folder) => tools::find_keys_in_folder(cx, folder.clone()),
    };
    if let Err(err) = found {
        tracing::debug!("Keys not looked for: {err}");
    }
}
