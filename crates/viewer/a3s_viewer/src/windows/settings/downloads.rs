//! Downloads: the mods folder, connections, speed limit and timeouts. The client keeps these per
//! repository, so the pane shows the first repository's values and writes every repository.
//! Removed mods always go to the Trash; there is no setting for it.

use std::path::Path;

use a3s_repo_files::services::{ClientData, ServiceError};
use a3s_types::constants::CONNECTION_TIMEOUT_MS;
use a3s_types::local::Repository;
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row, popup_button};
use a3s_viewer_context::Change;
use egui::{DragValue, Label, Ui};

use super::{PaneCx, note, number_committed};
use crate::fields::{labelled_row, toggle_row};
use crate::picker::FolderPicker;

/// Bytes in a megabyte, for the speed limit.
const MEGABYTE: f64 = 1024.0 * 1024.0;

/// The speed limit a newly enabled limit starts at, in MB/s.
const DEFAULT_LIMIT_MB: f64 = 5.0;

#[derive(Default)]
pub(super) struct DownloadsPane {
    folder_picker: FolderPicker,
}

/// The values the pane shows, from one repository.
#[derive(Clone, Debug, PartialEq)]
struct Values {
    mods_folder: Option<String>,
    connections: i32,
    speed_limit: f64,
    connection_timeout_s: u32,
    read_timeout_s: u32,
}

impl Values {
    fn of(repository: &Repository) -> Self {
        Self {
            mods_folder: repository.default_download_location.clone(),
            connections: repository.number_of_client_connections,
            speed_limit: repository.maximum_client_download_speed,
            connection_timeout_s: seconds(repository.protocol.connection_timeout.as_deref()),
            read_timeout_s: seconds(repository.protocol.read_timeout.as_deref()),
        }
    }
}

impl DownloadsPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        let (names, current) = {
            let state = cx.viewer.read();
            let names = state.client.repository_names();
            let current = names
                .first()
                .and_then(|name| state.client.repository(name).ok())
                .map(Values::of);
            (names, current)
        };
        let Some(current) = current else {
            frames::group(ui, |ui| note(ui, "Add a repository to set up downloads."));
            return;
        };
        let mut values = current.clone();
        if let Some(dir) = self.folder_picker.poll() {
            values.mods_folder = Some(dir.display().to_string());
        }

        let mut commit = false;
        frames::group(ui, |ui| {
            form_row(ui, "Mods folder", |ui| {
                if button(ui, "Choose…").clicked() {
                    let start = values.mods_folder.as_deref().map(Path::new);
                    self.folder_picker.open(ui.ctx(), "Mods folder", start);
                }
                let text = values.mods_folder.as_deref().unwrap_or("Not set");
                ui.add(Label::new(text).truncate());
            });
        });
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            labelled_row(ui, "Parallel connections", |ui| {
                let labels: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
                let mut choices: Vec<(i32, &str)> = vec![(0, "As the server says")];
                choices.extend((1..=10).zip(labels.iter().map(String::as_str)));
                popup_button(
                    ui,
                    "a3s_viewer::settings::connections",
                    &mut values.connections,
                    &choices,
                )
            });
            frames::group_separator(ui);
            let mut limited = values.speed_limit > 0.0;
            if toggle_row(ui, "Limit download speed", &mut limited).changed() {
                values.speed_limit = if limited {
                    DEFAULT_LIMIT_MB * MEGABYTE
                } else {
                    0.0
                };
            }
            if values.speed_limit > 0.0 {
                frames::group_separator(ui);
                form_row(ui, "Speed limit", |ui| {
                    let mut megabytes = values.speed_limit / MEGABYTE;
                    let response = ui.add(
                        DragValue::new(&mut megabytes)
                            .range(0.1..=1000.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" MB/s"),
                    );
                    if number_committed(&response) {
                        values.speed_limit = megabytes * MEGABYTE;
                        commit = true;
                    }
                });
            }
        });
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            commit |= seconds_row(ui, "Connection timeout", &mut values.connection_timeout_s);
            frames::group_separator(ui);
            commit |= seconds_row(ui, "Read timeout", &mut values.read_timeout_s);
        });
        ui.add_space(4.0);
        note(ui, "These settings apply to every repository.");

        let changed = values.mods_folder != current.mods_folder
            || values.connections != current.connections
            || (values.speed_limit > 0.0) != (current.speed_limit > 0.0);
        if changed || commit {
            cx.save(Some(Change::Repository), |client| {
                names
                    .iter()
                    .try_for_each(|name| apply(client, name, &values))
            });
        }
    }
}

fn seconds_row(ui: &mut Ui, label: &str, seconds: &mut u32) -> bool {
    form_row(ui, label, |ui| {
        let response = ui.add(DragValue::new(seconds).range(1..=3600).suffix(" s"));
        number_committed(&response)
    })
    .inner
}

/// Writes `values` into the repository `name` (one file write).
fn apply(client: &mut ClientData, name: &str, values: &Values) -> Result<(), ServiceError> {
    client
        .update_repository(name, |repository| {
            repository
                .default_download_location
                .clone_from(&values.mods_folder);
            repository.number_of_client_connections = values.connections;
            repository.maximum_client_download_speed = values.speed_limit;
            repository.protocol.connection_timeout = Some(millis(values.connection_timeout_s));
            repository.protocol.read_timeout = Some(millis(values.read_timeout_s));
            Ok(true)
        })
        .map(drop)
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
