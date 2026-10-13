//! The first-run sheet: where Arma 3 and the mods are, and the one-time import of the Java
//! client's data.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use a3s_foundation::error::UserFacing as _;
use a3s_foundation::paths::legacy::LegacyJavaData;
use a3s_platform::detect::ARMA3_X64_EXE;
use a3s_repo_files::legacy::{self, ImportError, ImportOptions, ImportReport};
use a3s_repo_files::services::{ClientData, LoadOutcome};
use a3s_types::local::Profile;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::{button, form_row};
use a3s_viewer_context::{Change, Command, ViewerContext, repositories};
use egui::{Label, RichText, Ui};

use super::Shown;
use crate::picker::FolderPicker;
use crate::toasts::Toaster;

/// What the Welcome sheet starts from, found before the window opened.
#[derive(Clone, Debug, Default)]
pub struct WelcomeSetup {
    /// The detected Arma 3 folder.
    pub arma3_dir: Option<PathBuf>,

    /// Java client data to offer for import; empty when there is none or it was imported.
    pub legacy_sources: Vec<LegacyJavaData>,
}

enum Import {
    NotStarted,
    Running(mpsc::Receiver<Result<ImportReport, ImportError>>),
    Done(Vec<String>),
}

pub(crate) struct Welcome {
    arma3_dir: Option<PathBuf>,
    mods_dir: Option<PathBuf>,
    legacy_sources: Vec<LegacyJavaData>,
    arma3_picker: FolderPicker,
    mods_picker: FolderPicker,
    import: Import,
}

impl Welcome {
    pub(super) fn new(setup: WelcomeSetup) -> Self {
        Self {
            arma3_dir: setup.arma3_dir,
            mods_dir: None,
            legacy_sources: setup.legacy_sources,
            arma3_picker: FolderPicker::default(),
            mods_picker: FolderPicker::default(),
            import: Import::NotStarted,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        if let Some(dir) = self.arma3_picker.poll() {
            self.arma3_dir = Some(dir);
        }
        if let Some(dir) = self.mods_picker.poll() {
            self.mods_dir = Some(dir);
        }
        self.finish_import(viewer);

        let busy = matches!(self.import, Import::Running(_))
            || self.arma3_picker.is_open()
            || self.mods_picker.is_open();
        let action = Dialog::new("a3s_viewer::sheet::welcome", "Welcome to Arma3Sync")
            .ok_label("Continue")
            .cancel_label("Skip")
            .ok_enabled(!busy)
            .default_button(Some(DialogButton::Ok))
            .show(ctx, |ui| self.body(ui, viewer));
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => {
                self.apply(viewer, toasts);
                Shown::Closed(None)
            }
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }

    fn body(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let ctx = ui.ctx().clone();
        let secondary = theme::palette(&ctx).text_secondary;
        ui.set_width(460.0);
        ui.label("Tell Arma3Sync where the game and your mods are.");
        ui.add_space(8.0);
        frames::group(ui, |ui| {
            folder_row(
                ui,
                "Arma 3 folder",
                self.arma3_dir.as_deref(),
                "Not found",
                || {
                    self.arma3_picker
                        .open(&ctx, "Arma 3 folder", self.arma3_dir.as_deref());
                },
            );
            frames::group_separator(ui);
            folder_row(ui, "Mods folder", self.mods_dir.as_deref(), "None", || {
                self.mods_picker
                    .open(&ctx, "Mods folder", self.mods_dir.as_deref());
            });
        });
        if self.legacy_sources.is_empty() {
            return;
        }
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            ui.label("Settings of the Java version of Arma3Sync were found in:");
            for source in &self.legacy_sources {
                ui.add(
                    Label::new(
                        RichText::new(source.configuration_dir.display().to_string())
                            .color(secondary),
                    )
                    .truncate(),
                );
            }
            ui.add_space(4.0);
            match &self.import {
                Import::NotStarted => {
                    if button(ui, "Import profiles and repositories").clicked() {
                        self.start_import(viewer, &ctx);
                    }
                }
                Import::Running(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Importing…");
                    });
                }
                Import::Done(lines) => {
                    for line in lines {
                        ui.label(RichText::new(line).color(secondary));
                    }
                }
            }
        });
    }

    fn start_import(&mut self, viewer: &ViewerContext, ctx: &egui::Context) {
        let store = viewer.read().client.store().clone();
        let sources = self.legacy_sources.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-legacy-import".to_owned())
            .spawn(move || {
                let result = legacy::import(&store, &sources, &ImportOptions::default());
                sender.send(result).ok();
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.import = Import::Running(receiver),
            Err(err) => {
                self.import = Import::Done(vec![format!("The import could not start: {err}")]);
            }
        }
    }

    /// Takes the import result once it arrived; after a successful import the client data is
    /// loaded again and every repository is checked.
    fn finish_import(&mut self, viewer: &ViewerContext) {
        let Import::Running(receiver) = &self.import else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.import = Import::Done(vec!["The import stopped unexpectedly.".to_owned()]);
                return;
            }
        };
        let lines = match result {
            Ok(report) => {
                let mut lines = report.summary();
                lines.extend(report.warnings.iter().cloned());
                if report.imported_anything() {
                    lines.extend(reload(viewer));
                }
                lines
            }
            Err(err) => vec![err.user_message()],
        };
        self.import = Import::Done(lines);
    }

    /// Stores the chosen folders in the active profile: the game executable and both folders
    /// as addon folders.
    fn apply(&self, viewer: &ViewerContext, toasts: &mut Toaster) {
        if self.arma3_dir.is_none() && self.mods_dir.is_none() {
            return;
        }
        let arma3_dir = self.arma3_dir.clone();
        let mods_dir = self.mods_dir.clone();
        let saved = viewer.mutate(Some(Change::Profile), |state| {
            state.client.update_active_profile(|profile| {
                if let Some(dir) = &arma3_dir {
                    profile.launcher_options.arma3_exe_path =
                        Some(dir.join(ARMA3_X64_EXE).display().to_string());
                    add_addon_folder(profile, dir);
                }
                if let Some(dir) = &mods_dir {
                    add_addon_folder(profile, dir);
                }
                true
            })
        });
        if let Err(err) = saved {
            toasts.error(err.user_message());
        }
    }
}

/// A form row showing a folder (or `missing`) and a Choose button that calls `choose`.
fn folder_row(
    ui: &mut Ui,
    label: &str,
    folder: Option<&Path>,
    missing: &str,
    choose: impl FnOnce(),
) {
    form_row(ui, label, |ui| {
        if button(ui, "Choose…").clicked() {
            choose();
        }
        let text = folder.map_or_else(|| missing.to_owned(), |dir| dir.display().to_string());
        ui.add(Label::new(text).truncate());
    });
}

/// Adds `dir` to the profile's addon folders unless it is there (ignoring case on Windows).
// Mirrors ui/main/dialogs/WelcomeDialog.java:150-170
fn add_addon_folder(profile: &mut Profile, dir: &Path) {
    let dir = dir.display().to_string();
    let present = profile.addon_search_directories.iter().any(|known| {
        if cfg!(windows) {
            known.eq_ignore_ascii_case(&dir)
        } else {
            *known == dir
        }
    });
    if !present {
        profile.addon_search_directories.push(dir);
    }
}

/// Loads the client data again into the context and starts a check of every repository;
/// returns the load warnings.
fn reload(viewer: &ViewerContext) -> Vec<String> {
    let store = viewer.read().client.store().clone();
    let LoadOutcome { data, warnings } = ClientData::load(store);
    let names = data.repository_names();
    viewer
        .mutate(Some(Change::Repository), |state| {
            state.client = data;
            Ok::<(), Infallible>(())
        })
        .ok();
    if let Err(err) = viewer.send(Command::Repositories(repositories::Command::Check {
        names,
    })) {
        tracing::warn!("Cannot check the imported repositories: {err}");
    }
    warnings
}
