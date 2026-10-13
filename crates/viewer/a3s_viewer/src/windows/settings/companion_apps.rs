//! Companion apps: programs started together with the game (`TeamSpeak`, overlays, …). Add with
//! the button or by dropping an executable onto the pane; removal offers Undo.

use std::path::Path;

use a3s_repo_files::services::ServiceError;
use a3s_types::local::ExternalApplication;
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::widgets::{button, form_row, icon_text_button};
use a3s_viewer_context::{Change, ViewerContext};
use egui::{Label, Ui};

use super::{PaneCx, note};
use crate::fields::{text_row, toggle_row};
use crate::picker::FilePicker;

#[derive(Default)]
pub(super) struct CompanionAppsPane {
    /// The list while a text field has the focus.
    draft: Option<Vec<ExternalApplication>>,
    picker: FilePicker,

    /// The row the file dialog chooses the program of.
    picking: Option<usize>,
}

impl CompanionAppsPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        let stored = cx
            .viewer
            .read()
            .client
            .configuration()
            .external_applications
            .clone();
        let mut apps = self.draft.clone().unwrap_or_else(|| stored.clone());
        let mut commit = false;
        let mut focused = false;
        let mut removed: Option<usize> = None;

        if let Some(path) = self.picker.poll()
            && let Some(app) = self.picking.take().and_then(|index| apps.get_mut(index))
        {
            app.executable_path = Some(path.display().to_string());
            commit = true;
        }
        let dropped: Vec<_> = ui.ctx().input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        for path in dropped {
            apps.push(application(&path));
            commit = true;
        }

        if apps.is_empty() {
            frames::group(ui, |ui| {
                note(ui, "No companion apps. Drop a program here or add one.");
            });
        }
        for (index, app) in apps.iter_mut().enumerate() {
            frames::group(ui, |ui| {
                commit |= toggle_row(ui, "Start with the game", &mut app.enable).changed();
                frames::group_separator(ui);
                let mut name = app.name.clone().unwrap_or_default();
                let response = text_row(ui, "Name", &mut name, false);
                focused |= response.has_focus();
                commit |= response.lost_focus();
                app.name = Some(name);
                frames::group_separator(ui);
                form_row(ui, "Program", |ui| {
                    if button(ui, "Choose…").clicked() {
                        let start = app
                            .executable_path
                            .as_deref()
                            .map(Path::new)
                            .and_then(Path::parent);
                        self.picking = Some(index);
                        self.picker.open(ui.ctx(), "Program", start);
                    }
                    let text = app.executable_path.as_deref().unwrap_or("Not set");
                    ui.add(Label::new(text).truncate());
                });
                frames::group_separator(ui);
                let mut arguments = app.parameters.clone().unwrap_or_default();
                let response = text_row(ui, "Arguments", &mut arguments, false);
                focused |= response.has_focus();
                commit |= response.lost_focus();
                app.parameters = (!arguments.is_empty()).then_some(arguments);
                frames::group_separator(ui);
                form_row(ui, "", |ui| {
                    if icon_text_button(ui, icons::MINUS, "Remove").clicked() {
                        removed = Some(index);
                    }
                });
            });
            ui.add_space(8.0);
        }
        if icon_text_button(ui, icons::PLUS, "Add").clicked() {
            apps.push(ExternalApplication {
                name: Some("New app".to_owned()),
                enable: true,
                ..ExternalApplication::default()
            });
            commit = true;
        }

        if let Some(index) = removed {
            let app = apps.remove(index);
            if save(cx, apps) {
                let label = app.name.clone().unwrap_or_default();
                cx.toasts
                    .undoable(format!("Removed {label}"), move |viewer| {
                        restore(viewer, app);
                    });
            }
            self.draft = None;
            return;
        }
        if commit && apps != stored {
            save(cx, apps);
            self.draft = None;
        } else {
            self.draft = focused.then_some(apps);
        }
    }
}

/// A companion app for the program at `path`, named after its file, started with the game.
fn application(path: &Path) -> ExternalApplication {
    let name = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    ExternalApplication {
        name: Some(name),
        executable_path: Some(path.display().to_string()),
        parameters: None,
        enable: true,
    }
}

fn save(cx: &mut PaneCx<'_>, apps: Vec<ExternalApplication>) -> bool {
    cx.save(Some(Change::Online), |client| {
        client.save_external_applications(apps)
    })
    .is_some()
}

fn restore(viewer: &ViewerContext, app: ExternalApplication) {
    let restored = viewer.mutate(Some(Change::Online), |state| {
        let mut apps = state.client.configuration().external_applications.clone();
        apps.push(app);
        state.client.save_external_applications(apps)
    });
    if let Err(err) = restored {
        let err: ServiceError = err;
        tracing::warn!("Cannot restore the companion app: {err}");
    }
}
