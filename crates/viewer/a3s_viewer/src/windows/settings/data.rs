//! Data: importing the Java client's data, auto-config import and export, and the logs.

use a3s_foundation::paths::legacy::{LegacyJavaData, discover_legacy_java};
use a3s_platform::detect::java_install_dirs;
use a3s_platform::folders::open_in_file_manager;
use a3s_repo_files::services::AutoConfigImportSummary;
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row};
use a3s_viewer_context::{Change, Command, repositories};
use egui::{Label, Ui};

use super::{PaneCx, group_heading, note};
use crate::desktop::Desktop;
use crate::legacy_import::LegacyImport;
use crate::picker::{FilePicker, FolderPicker};

#[derive(Default)]
pub(super) struct DataPane {
    /// The Java client's data found on this computer, looked up once.
    legacy_sources: Option<Vec<LegacyJavaData>>,
    legacy_import: LegacyImport,
    import_picker: FilePicker,
    export_picker: FolderPicker,

    /// What the last auto-config import did.
    import_summary: Vec<String>,
}

impl DataPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        self.legacy_import.poll(cx.viewer);
        if let Some(file) = self.import_picker.poll() {
            self.import_autoconfig(cx, &file);
        }
        if let Some(folder) = self.export_picker.poll() {
            export_autoconfig(cx, &folder);
        }
        let sources = self
            .legacy_sources
            .get_or_insert_with(|| legacy_sources(cx.desktop))
            .clone();

        group_heading(ui, "Java Arma3Sync");
        frames::group(ui, |ui| {
            if sources.is_empty() {
                note(
                    ui,
                    "No settings of the Java version of Arma3Sync were found.",
                );
                return;
            }
            form_row(ui, "Profiles, repositories and settings", |ui| {
                let enabled = !self.legacy_import.is_running();
                if ui
                    .add_enabled_ui(enabled, |ui| button(ui, "Import"))
                    .inner
                    .clicked()
                {
                    self.legacy_import.start(cx.viewer, ui.ctx(), &sources);
                }
                if !enabled {
                    ui.spinner();
                }
            });
            if let LegacyImport::Done(lines) = &self.legacy_import {
                for line in lines {
                    note(ui, line.clone());
                }
            }
        });

        group_heading(ui, "Auto-config");
        frames::group(ui, |ui| {
            form_row(ui, "Profiles, servers and repositories", |ui| {
                if button(ui, "Export…").clicked() {
                    self.export_picker.open(ui.ctx(), "Export to folder", None);
                }
                if button(ui, "Import…").clicked() {
                    self.import_picker
                        .open(ui.ctx(), "Import auto-config", None);
                }
            });
            for line in &self.import_summary {
                note(ui, line.clone());
            }
        });

        group_heading(ui, "Logs");
        frames::group(ui, |ui| {
            let log_dir = cx.desktop.log_dir.clone();
            form_row(ui, "Log files", |ui| {
                let enabled = log_dir.is_some();
                if ui
                    .add_enabled_ui(enabled, |ui| button(ui, "Show in folder"))
                    .inner
                    .clicked()
                    && let (Some(dir), Some(env)) = (&log_dir, &cx.desktop.env)
                {
                    let opened = std::fs::create_dir_all(dir)
                        .map_err(|source| a3s_platform::Error::Io {
                            path: Some(dir.clone()),
                            source,
                        })
                        .and_then(|()| open_in_file_manager(env, dir));
                    cx.report(opened);
                }
                if let Some(dir) = &log_dir {
                    ui.add(Label::new(dir.display().to_string()).truncate());
                }
            });
        });
    }

    fn import_autoconfig(&mut self, cx: &mut PaneCx<'_>, file: &std::path::Path) {
        let Some(summary) = cx.save(Some(Change::Profile), |client| {
            client.import_autoconfig(file)
        }) else {
            return;
        };
        cx.viewer
            .cx()
            .emit(a3s_viewer_context::AppEvent::Changed(Change::Repository));
        if !summary.repositories_added.is_empty() {
            let command = Command::Repositories(repositories::Command::Check {
                names: summary.repositories_added.clone(),
            });
            cx.report(cx.viewer.send(command));
        }
        self.import_summary = summary_lines(&summary);
    }
}

fn export_autoconfig(cx: &mut PaneCx<'_>, folder: &std::path::Path) {
    let exported = {
        let state = cx.viewer.read();
        let client = &state.client;
        let servers: Vec<usize> = (0..client.favorite_servers().len()).collect();
        client.export_autoconfig(
            &client.profile_names(),
            &servers,
            &client.repository_names(),
            folder,
        )
    };
    if let Some(path) = cx.report(exported) {
        cx.toasts.success(format!("Exported to {}", path.display()));
    }
}

fn summary_lines(summary: &AutoConfigImportSummary) -> Vec<String> {
    let mut lines = vec![format!(
        "Imported {} profiles, {} favourite servers and {} repositories.",
        summary.profiles_imported.len(),
        summary.favorite_servers_added,
        summary.repositories_added.len()
    )];
    if !summary.repositories_skipped.is_empty() {
        lines.push(format!(
            "Kept the existing repositories {}.",
            summary.repositories_skipped.join(", ")
        ));
    }
    lines.extend(summary.warnings.iter().cloned());
    lines
}

fn legacy_sources(desktop: &Desktop) -> Vec<LegacyJavaData> {
    desktop.env.as_ref().map_or_else(Vec::new, |env| {
        discover_legacy_java(env, &java_install_dirs(env, desktop.registry.as_ref()))
    })
}
