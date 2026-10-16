//! The Settings segment: source folder, upload connection, the public auto-config link, the
//! build and upload options, the favourite servers of the auto-config, and the path lists.
//! Every change is stored at once.

use std::path::PathBuf;

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::ServiceError;
use a3s_types::autoconfig_url::{autoconfig_display_url, determine_autoconfig_url};
use a3s_types::local::Repository;
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::{ROW_HEIGHT, button, form_row, toggle};
use a3s_viewer_context::admin;
use a3s_viewer_context::{Change, Command, ViewerContext};
use egui::{Align, Label, Layout, RichText, Ui, WidgetInfo, WidgetType, vec2};

use super::View;
use crate::picker::FolderPicker;
use crate::screens::{ScreenCx, count_text, section_title};
use crate::sheets::PathListKind;
use crate::shell::Action;

/// The folder dialog of the source folder.
#[derive(Default)]
pub(super) struct SettingsState {
    picker: FolderPicker,
}

pub(super) fn show(ui: &mut Ui, view: &View, state: &mut SettingsState, cx: &mut ScreenCx<'_>) {
    let name = view.name();
    if let Some(folder) = state.picker.poll() {
        set_source(cx, name, &folder);
    }
    let palette = theme::palette(ui.ctx());
    let repository = &view.repository;

    section_title(ui, "Source");
    frames::group(ui, |ui| {
        form_row(ui, "Source folder", |ui| {
            if button(ui, "Choose…").clicked() {
                let start = repository.path.as_deref().map(PathBuf::from);
                state
                    .picker
                    .open(ui.ctx(), "Source folder", start.as_deref());
            }
            let text = repository.path.clone().unwrap_or_else(|| "None".to_owned());
            ui.add(Label::new(text).truncate());
        });
        frames::group_separator(ui);
        form_row(ui, "Publishing from this computer", |ui| {
            if button(ui, "Stop").clicked() {
                stop_publishing(cx, repository);
            }
        });
    });

    section_title(ui, "Upload");
    frames::group(ui, |ui| {
        form_row(ui, "Upload connection", |ui| {
            if button(ui, "Edit connection…").clicked() {
                cx.actions
                    .push(Action::EditUploadConnection(name.to_owned()));
            }
            let text = repository.upload_protocol.as_ref().map_or_else(
                || "Not set".to_owned(),
                |protocol| {
                    format!(
                        "{}{}:{}",
                        protocol.protocol_type.prompt(),
                        protocol.url,
                        protocol.port
                    )
                },
            );
            ui.add(Label::new(text).truncate());
        });
        frames::group_separator(ui);
        let link = autoconfig_display_url(
            repository.protocol.protocol_type,
            Some(&determine_autoconfig_url(&repository.protocol)),
        )
        .unwrap_or_default();
        form_row(ui, "Auto-config link", |ui| {
            if button(ui, "Copy").clicked() {
                ui.ctx().copy_text(link.clone());
                cx.toasts.info("Copied the auto-config link");
            }
            ui.add(Label::new(RichText::new(&link).color(palette.text_secondary)).truncate());
        });
    });

    section_title(ui, "Options");
    frames::group(ui, |ui| {
        let mut compressed = repository.compressed;
        if toggle_row(ui, "Compress files", &mut compressed) {
            store(cx, |data| data.set_compressed(name, compressed));
        }
        frames::group_separator(ui);
        let mut delta = repository.use_partial_file_transfer();
        if toggle_row(ui, "Delta updates", &mut delta) {
            store(cx, |data| data.set_use_partial_file_transfer(name, delta));
        }
        frames::group_separator(ui);
        let mut compressed_only = repository.upload_compressed_pbo_files_only;
        if toggle_row(ui, "Upload only compressed files", &mut compressed_only) {
            store(cx, |data| {
                data.set_upload_compressed_pbo_files_only(name, compressed_only)
            });
        }
        frames::group_separator(ui);
        let mut parallel = repository.parallel_upload_connections() > 1;
        if toggle_row(ui, "Parallel uploads", &mut parallel) {
            let connections = if parallel {
                Repository::DEFAULT_PARALLEL_UPLOAD_CONNECTIONS
            } else {
                1
            };
            store(cx, |data| {
                data.set_parallel_upload_connections(name, connections)
            });
        }
    });

    section_title(ui, "Auto-config servers");
    servers(ui, name, cx);

    section_title(ui, "Exclusions");
    frames::group(ui, |ui| {
        for (index, kind) in [PathListKind::ExcludedFiles, PathListKind::HiddenFolders]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                frames::group_separator(ui);
            }
            let count = kind.paths(cx.viewer, name).len();
            form_row(ui, kind.title(), |ui| {
                if button(ui, &format!("Edit {}…", kind.title().to_lowercase())).clicked() {
                    cx.actions.push(Action::EditPathList(name.to_owned(), kind));
                }
                ui.label(
                    RichText::new(count_text(count, "entry", "entries"))
                        .color(palette.text_secondary),
                );
            });
        }
    });
}

/// The client's favourite servers, each with a switch that includes it in the auto-config
/// players get with the repository; it is published by the next publish.
fn servers(ui: &mut Ui, name: &str, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let servers = admin::autoconfig_servers(&cx.viewer.read().client, name);
    frames::group(ui, |ui| {
        if servers.is_empty() {
            ui.label(
                RichText::new("No favourite servers yet. Add them on the Servers screen.")
                    .color(palette.text_secondary),
            );
            return;
        }
        for (index, entry) in servers.iter().enumerate() {
            if index > 0 {
                frames::group_separator(ui);
            }
            let server = &entry.server;
            let label = server.name.as_deref().unwrap_or("Unnamed server");
            let address = format!(
                "{}:{}",
                server.ip_address.as_deref().unwrap_or_default(),
                server.port
            );
            let mut included = entry.included;
            if switch_row(ui, label, Some(&address), &mut included) {
                store(cx, |data| {
                    admin::set_autoconfig_server(data, name, index, included)
                });
            }
        }
    });
    ui.label(
        RichText::new("Players get the chosen servers with the auto-config of the next publish.")
            .color(palette.text_secondary),
    );
}

/// A form row with a switch named by its label; returns whether it was flipped.
fn toggle_row(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    switch_row(ui, label, None, on)
}

/// A form row with a switch named by its label and an optional secondary detail beside the
/// label; returns whether it was flipped.
fn switch_row(ui: &mut Ui, label: &str, detail: Option<&str>, on: &mut bool) -> bool {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(label);
            if let Some(detail) = detail {
                let palette = theme::palette(ui.ctx());
                ui.label(RichText::new(detail).color(palette.text_secondary));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = toggle(ui, on);
                // The switch is named by the row's label.
                let (enabled, state) = (ui.is_enabled(), *on);
                response.widget_info(|| {
                    WidgetInfo::selected(WidgetType::Checkbox, enabled, state, label)
                });
                response.changed()
            })
            .inner
        },
    )
    .inner
}

/// Stores a repository setting; a failure is a toast.
fn store(
    cx: &mut ScreenCx<'_>,
    f: impl FnOnce(&mut a3s_repo_files::services::ClientData) -> Result<(), ServiceError>,
) {
    if let Err(err) = cx
        .viewer
        .mutate(Some(Change::Repository), |state| f(&mut state.client))
    {
        cx.toasts.error(err.user_message());
    }
}

/// Before a source folder is chosen: what publishing does, and the folder dialog.
pub(super) fn choose_source(
    ui: &mut Ui,
    view: &View,
    state: &mut SettingsState,
    cx: &mut ScreenCx<'_>,
) {
    let name = view.name();
    if let Some(folder) = state.picker.poll() {
        set_source(cx, name, &folder);
    }
    let palette = theme::palette(ui.ctx());
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() / 4.0).max(24.0));
        ui.label(
            RichText::new(a3s_ui::icon::icons::CLOUD_ARROW_UP)
                .size(40.0)
                .color(palette.text_secondary),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Publish {name} from this computer")).heading());
        ui.label(
            RichText::new(
                "Choose the folder that holds the repository's mods. Arma3Sync builds the \
                 repository in it and uploads it to the server.",
            )
            .color(palette.text_secondary),
        );
        ui.add_space(12.0);
        if a3s_ui::widgets::primary_button(ui, "Choose source folder…").clicked() {
            state.picker.open(ui.ctx(), "Source folder", None);
        }
    });
}

/// Stops publishing `repository` from this computer: forgets its source folder, with Undo.
fn stop_publishing(cx: &mut ScreenCx<'_>, repository: &Repository) {
    let name = repository.name.clone();
    let previous = repository.path.clone();
    let stored = cx.viewer.mutate(Some(Change::Repository), |state| {
        state.client.set_repository_path(&name, None)?;
        admin::forget(state, &name);
        Ok::<(), ServiceError>(())
    });
    match stored {
        Ok(()) => {
            cx.toasts.undoable(
                format!("Stopped publishing {name}"),
                move |viewer: &ViewerContext| {
                    let restored = viewer.mutate(Some(Change::Repository), |state| {
                        state.client.set_repository_path(&name, previous)
                    });
                    match restored {
                        Ok(()) => refresh(viewer, &name),
                        Err(err) => tracing::warn!("Cannot undo: {}", err.user_message()),
                    }
                },
            );
            cx.actions
                .push(Action::Navigate(crate::shell::Screen::Repository(
                    repository.name.clone(),
                )));
        }
        Err(err) => cx.toasts.error(err.user_message()),
    }
}

/// Stores the new source folder; the publishing status is read again for it.
fn set_source(cx: &mut ScreenCx<'_>, name: &str, folder: &std::path::Path) {
    let path = folder.display().to_string();
    let stored = cx.viewer.mutate(Some(Change::Repository), |state| {
        state.client.set_repository_path(name, Some(path))?;
        admin::forget(state, name);
        Ok::<(), ServiceError>(())
    });
    match stored {
        Ok(()) => refresh(cx.viewer, name),
        Err(err) => cx.toasts.error(err.user_message()),
    }
}

fn refresh(viewer: &ViewerContext, name: &str) {
    let command = Command::Admin(admin::Command::Refresh {
        repository: name.to_owned(),
        remote: true,
    });
    if let Err(err) = viewer.send(command) {
        tracing::warn!("Cannot refresh the publishing status of {name}: {err}");
    }
}
