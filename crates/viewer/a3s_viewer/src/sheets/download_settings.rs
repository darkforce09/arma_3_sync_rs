//! The Download settings sheet of one repository: where it downloads, whether it notifies or
//! updates by itself, connections, speed and timeouts, and the folders kept when updating.
//! Every change is stored at once. How local files outside the repository are treated is on
//! the Repository screen ([`local_files_options`]).

use std::path::{Path, PathBuf};

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::{ClientData, ServiceError};
use a3s_types::constants::CONNECTION_TIMEOUT_MS;
use a3s_types::local::Repository;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::{button, form_row, popup_button, toggle};
use a3s_viewer_context::{Change, ViewerContext, sync};
use egui::{Align, DragValue, Label, Layout, Response, RichText, Ui};

use super::Shown;
use crate::fields::{labelled_row, toggle_row};
use crate::picker::FolderPicker;
use crate::toasts::Toaster;

/// Bytes per second in one MB/s, as the Java download settings count them.
// Mirrors ui/repository/dialogs/DownloadSettingsDialog.java:243-247
const MEGABYTE: f64 = 1_000_000.0;

/// The speed limit a newly enabled limit starts at, in MB/s.
const DEFAULT_LIMIT_MB: f64 = 5.0;

/// The connections offered when the server does not say how many it allows.
const DEFAULT_SERVER_CONNECTIONS: i32 = 10;

/// Game executables whose folder must not be cleaned of files outside the repository.
// Mirrors ui/repository/DownloadPanel.java:823-847
const GAME_EXECUTABLES: [&str; 4] = [
    "arma3.exe",
    "arma3_x64.exe",
    "arma3server.exe",
    "arma3server_x64.exe",
];

/// Which folder dialog is open.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Picking {
    DownloadFolder,
    KeptFolder,
}

/// The sheet's state.
pub(crate) struct DownloadSettings {
    repository: String,
    picker: FolderPicker,
    picking: Picking,
    error: Option<String>,
}

impl DownloadSettings {
    pub(crate) fn new(repository: &str) -> Self {
        Self {
            repository: repository.to_owned(),
            picker: FolderPicker::default(),
            picking: Picking::DownloadFolder,
            error: None,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        let (stored, server_connections) = {
            let state = viewer.read();
            let Ok(stored) = state.client.repository(&self.repository).cloned() else {
                return Shown::Closed(None);
            };
            let server_connections = state
                .repositories
                .runtime(&self.repository)
                .metadata
                .server_info
                .as_ref()
                .map(|info| info.number_of_connections)
                .filter(|count| *count > 0);
            (stored, server_connections)
        };
        if let Some(folder) = self.picker.poll() {
            self.chosen(viewer, &stored, &folder);
        }
        let title = format!("Download settings of {}", self.repository);
        let action = Dialog::new("a3s_viewer::sheet::download_settings", title)
            .show_ok(false)
            .cancel_label("Done")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| {
                self.body(ui, viewer, toasts, &stored, server_connections);
            });
        if action == DialogAction::None {
            Shown::Open
        } else {
            Shown::Closed(None)
        }
    }

    fn body(
        &mut self,
        ui: &mut Ui,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
        stored: &Repository,
        server_connections: Option<i32>,
    ) {
        let palette = theme::palette(ui.ctx());
        let name = self.repository.clone();
        ui.set_width(480.0);
        let folder = stored
            .default_download_location
            .as_deref()
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from);

        frames::group(ui, |ui| {
            form_row(ui, "Download folder", |ui| {
                if button(ui, "Choose…").clicked() {
                    self.picking = Picking::DownloadFolder;
                    self.picker
                        .open(ui.ctx(), "Download folder", folder.as_deref());
                }
                let text = folder
                    .as_deref()
                    .map_or_else(|| "Not set".to_owned(), |f| f.display().to_string());
                ui.add(Label::new(text).truncate());
            });
            frames::group_separator(ui);
            let mut notify = stored.notify;
            if toggle_row(ui, "Notify me about updates", &mut notify).changed() {
                self.store(viewer, false, |client| client.set_notify(&name, notify));
            }
            frames::group_separator(ui);
            let mut auto = stored.auto;
            if toggle_row(ui, "Download updates automatically", &mut auto).changed() {
                self.store(viewer, false, |client| client.set_auto_update(&name, auto));
            }
        });

        ui.add_space(12.0);
        frames::group(ui, |ui| {
            let cap = server_connections.unwrap_or(DEFAULT_SERVER_CONNECTIONS);
            let labels: Vec<String> = (1..=cap).map(|n| n.to_string()).collect();
            let mut choices: Vec<(i32, &str)> = vec![(0, "As the server allows")];
            choices.extend((1..=cap).zip(labels.iter().map(String::as_str)));
            let mut connections = stored.number_of_client_connections.clamp(0, cap);
            if labelled_row(ui, "Connections", |ui| {
                popup_button(
                    ui,
                    "a3s_viewer::download_settings::connections",
                    &mut connections,
                    &choices,
                )
            })
            .changed()
            {
                self.store(viewer, false, |client| {
                    client.set_number_of_client_connections(&name, connections)
                });
            }
            frames::group_separator(ui);
            let mut limited = stored.maximum_client_download_speed > 0.0;
            if toggle_row(ui, "Limit download speed", &mut limited).changed() {
                let speed = if limited {
                    DEFAULT_LIMIT_MB * MEGABYTE
                } else {
                    0.0
                };
                self.store(viewer, false, |client| {
                    client.set_maximum_client_download_speed(&name, speed)
                });
            }
            if limited {
                frames::group_separator(ui);
                form_row(ui, "Speed limit", |ui| {
                    let mut megabytes = stored.maximum_client_download_speed / MEGABYTE;
                    let response = ui.add(
                        DragValue::new(&mut megabytes)
                            .range(0.1..=1000.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" MB/s"),
                    );
                    if committed(&response) {
                        self.store(viewer, false, |client| {
                            client.set_maximum_client_download_speed(&name, megabytes * MEGABYTE)
                        });
                    }
                });
            }
            frames::group_separator(ui);
            let mut connection_timeout = seconds(stored.protocol.connection_timeout.as_deref());
            if seconds_row(ui, "Connection timeout", &mut connection_timeout) {
                self.store(viewer, false, |client| {
                    client.set_connection_timeout(&name, Some(millis(connection_timeout)))
                });
            }
            frames::group_separator(ui);
            let mut read_timeout = seconds(stored.protocol.read_timeout.as_deref());
            if seconds_row(ui, "Read timeout", &mut read_timeout) {
                self.store(viewer, false, |client| {
                    client.set_read_timeout(&name, Some(millis(read_timeout)))
                });
            }
        });

        ui.add_space(12.0);
        ui.label(RichText::new("Folders kept when updating").strong());
        ui.label(
            RichText::new("Your own files in these folders are never removed.")
                .color(palette.text_secondary),
        );
        ui.add_space(4.0);
        let kept: Vec<String> = stored.hidden_folder_paths.iter().cloned().collect();
        frames::group(ui, |ui| {
            if kept.is_empty() {
                ui.label(RichText::new("None").color(palette.text_secondary));
            }
            for (index, path) in kept.iter().enumerate() {
                if index > 0 {
                    frames::group_separator(ui);
                }
                ui.horizontal(|ui| {
                    ui.add(Label::new(path).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(icons::MINUS_CIRCLE)
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            self.remove_kept(viewer, toasts, path);
                        }
                    });
                });
            }
        });
        ui.add_space(6.0);
        ui.add_enabled_ui(folder.is_some(), |ui| {
            if button(ui, "Add folder…")
                .on_disabled_hover_text("Choose the download folder first.")
                .clicked()
            {
                self.picking = Picking::KeptFolder;
                self.picker
                    .open(ui.ctx(), "Folder to keep", folder.as_deref());
            }
        });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(palette.status_danger));
        }
    }

    /// Stores the folder the dialog returned.
    fn chosen(&mut self, viewer: &ViewerContext, stored: &Repository, folder: &Path) {
        let name = self.repository.clone();
        match self.picking {
            Picking::DownloadFolder => {
                let text = folder.display().to_string();
                self.store(viewer, true, |client| {
                    client.set_default_download_location(&name, None, Some(text))
                });
            }
            Picking::KeptFolder => {
                let relative = stored
                    .default_download_location
                    .as_deref()
                    .and_then(|base| folder.strip_prefix(base).ok())
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .filter(|relative| !relative.is_empty());
                let Some(relative) = relative else {
                    self.error = Some("Choose a folder inside the download folder.".to_owned());
                    return;
                };
                self.store(viewer, true, |client| {
                    keep_folder(client, &name, &relative).map(drop)
                });
            }
        }
    }

    fn remove_kept(&mut self, viewer: &ViewerContext, toasts: &mut Toaster, path: &str) {
        let name = self.repository.clone();
        let removed = path.to_owned();
        let stored = self.store(viewer, true, |client| {
            client.update_repository(&name, |repository| {
                Ok(repository.hidden_folder_paths.remove(&removed))
            })
        });
        if stored {
            let path = path.to_owned();
            toasts.undoable(format!("Removed {path}"), move |viewer: &ViewerContext| {
                let restored = viewer.mutate(Some(Change::Repository), |state| {
                    keep_folder(&mut state.client, &name, &path)?;
                    sync::forget(state, &name);
                    Ok::<(), ServiceError>(())
                });
                if let Err(err) = restored {
                    tracing::warn!("Cannot undo the removal: {}", err.user_message());
                }
            });
        }
    }

    /// Runs `f` on the client data; `replan` drops the repository's comparison, which no
    /// longer matches. Returns whether it was stored; a failure shows under the form.
    fn store<T>(
        &mut self,
        viewer: &ViewerContext,
        replan: bool,
        f: impl FnOnce(&mut ClientData) -> Result<T, ServiceError>,
    ) -> bool {
        let name = self.repository.clone();
        let stored = viewer.mutate(Some(Change::Repository), |state| {
            f(&mut state.client)?;
            if replan {
                sync::forget(state, &name);
            }
            Ok::<(), ServiceError>(())
        });
        self.error = stored.as_ref().err().map(|err| err.user_message());
        stored.is_ok()
    }
}

/// How a download of `stored` treats local files outside the repository, as two switches: "Use
/// copies in other mods folders" (auto-discover) and "Remove extra files" (exact match),
/// disabled unless `enabled`. Every change is stored at once and
/// drops the comparison; returns the error of a failed save.
pub(crate) fn local_files_options(
    ui: &mut Ui,
    viewer: &ViewerContext,
    stored: &Repository,
    enabled: bool,
) -> Option<String> {
    let name = stored.name.as_str();
    let game_folder = stored
        .default_download_location
        .as_deref()
        .filter(|folder| !folder.is_empty())
        .is_some_and(|folder| holds_game(Path::new(folder)));
    let mut exact = stored.exact_match && !game_folder;
    let mut discover = !stored.no_auto_discover && !exact;
    let mut error = None;
    let response = labelled_switch(
        ui,
        enabled && !exact,
        &mut discover,
        "Use copies in other mods folders",
    )
    .on_hover_text(
        "A mod already in another mods folder is updated there instead of downloaded again.",
    )
    .on_disabled_hover_text(
        "Not with \"Remove extra files\": every mod goes to the download folder.",
    );
    if response.changed() {
        error = replan(viewer, name, |client| {
            client.set_no_auto_discover(name, !discover)
        });
    }
    let response = labelled_switch(
        ui,
        enabled && !game_folder,
        &mut exact,
        "Remove extra files",
    )
    .on_hover_text(
        "Files and folders in the download folder that the repository does not have go to the \
             Trash with the next update.",
    )
    .on_disabled_hover_text(
        "The download folder holds the game; its other files are never removed.",
    );
    if response.changed() {
        error = replan(viewer, name, |client| {
            client.set_exact_match(name, exact)?;
            if exact {
                // Mirrors ui/repository/DownloadPanel.java:1336-1340
                client.set_no_auto_discover(name, true)?;
            }
            Ok(())
        });
    }
    error
}

/// A switch with its label after it; a click on either flips it.
fn labelled_switch(ui: &mut Ui, enabled: bool, on: &mut bool, label: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            let mut switch = toggle(ui, on);
            let text = ui.add(Label::new(label).sense(egui::Sense::click()));
            if text.clicked() {
                *on = !*on;
                switch.mark_changed();
            }
            switch.union(text)
        })
        .inner
    })
    .inner
}

/// Sets the download folder of `repository` and drops its comparison; returns the error of a
/// failed save.
pub(crate) fn set_download_folder(
    viewer: &ViewerContext,
    repository: &str,
    folder: &Path,
) -> Option<String> {
    let text = folder.display().to_string();
    replan(viewer, repository, |client| {
        client.set_default_download_location(repository, None, Some(text))
    })
}

/// Runs `f` on the client data and drops the comparison of `repository`, which no longer
/// matches; returns the error of a failed save.
fn replan<T>(
    viewer: &ViewerContext,
    repository: &str,
    f: impl FnOnce(&mut ClientData) -> Result<T, ServiceError>,
) -> Option<String> {
    viewer
        .mutate(Some(Change::Repository), |state| {
            f(&mut state.client)?;
            sync::forget(state, repository);
            Ok::<(), ServiceError>(())
        })
        .err()
        .map(|err| err.user_message())
}

/// Adds `relative` to the folders of `repository` kept when updating; returns whether it was
/// new.
pub(crate) fn keep_folder(
    client: &mut ClientData,
    repository: &str,
    relative: &str,
) -> Result<bool, ServiceError> {
    client.update_repository(repository, |stored| {
        Ok(stored.hidden_folder_paths.insert(relative.to_owned()))
    })
}

/// Whether `folder` holds the game, whose other files must stay.
fn holds_game(folder: &Path) -> bool {
    GAME_EXECUTABLES
        .iter()
        .any(|executable| folder.join(executable).is_file())
}

fn seconds_row(ui: &mut Ui, label: &str, seconds: &mut u32) -> bool {
    form_row(ui, label, |ui| {
        let response = ui.add(DragValue::new(seconds).range(1..=3600).suffix(" s"));
        committed(&response)
    })
    .inner
}

/// Whether an edited number should be written: a click or typed value, or the end of a drag.
fn committed(response: &Response) -> bool {
    (response.changed() && !response.dragged()) || response.drag_stopped()
}

/// A stored timeout (milliseconds as text, unset for the default) in whole seconds.
fn seconds(stored: Option<&str>) -> u32 {
    stored
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(CONNECTION_TIMEOUT_MS)
        / 1000
}

fn millis(seconds: u32) -> String {
    (u64::from(seconds) * 1000).to_string()
}
