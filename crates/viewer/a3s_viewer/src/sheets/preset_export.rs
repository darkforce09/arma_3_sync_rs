//! The sheet exporting an addon group as an Arma 3 Launcher preset: it reads the Workshop id of
//! each enabled mod of the group, asks for the ones it cannot find (remembered for the next
//! export), then saves the HTML where the user chooses.
// Mirrors ui/main/AddonsPanel.java:1203-1394

use std::sync::{Arc, mpsc};

use a3s_addons::AddonRegistry;
use a3s_repo_files::local::LocalStore;
use a3s_tools::preset::{
    ExportResult, MetadataIssue, ensure_html_extension, generate, suggested_file_name,
    validate_metadata_input, write,
};
use a3s_types::local::TreeDirectory;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use egui::{Label, RichText, TextEdit};

use super::Shown;
use crate::picker::SavePicker;
use crate::toasts::Toaster;

/// Where the export stands.
enum Stage {
    /// The mods' `meta.cpp` files are read on a worker thread.
    Reading(mpsc::Receiver<ExportResult>),

    /// Some mods have no Workshop id: one id and name field per mod.
    Ids {
        issues: Vec<MetadataIssue>,
        fields: Vec<(String, String)>,
        error: Option<String>,
    },

    /// The save dialog is open.
    Saving(Box<ExportResult>),

    /// The group cannot be exported.
    Failed(Vec<String>),
}

/// The sheet's state.
pub(crate) struct PresetExport {
    group: TreeDirectory,
    registry: Arc<AddonRegistry>,
    store: Arc<LocalStore>,
    stage: Stage,
    picker: SavePicker,
}

impl PresetExport {
    /// The export of the enabled mods of `group`, found in `registry`.
    pub(crate) fn new(
        ctx: &egui::Context,
        group: TreeDirectory,
        registry: Arc<AddonRegistry>,
        store: Arc<LocalStore>,
    ) -> Self {
        let mut export = Self {
            group,
            registry,
            store,
            stage: Stage::Failed(Vec::new()),
            picker: SavePicker::default(),
        };
        export.read(ctx);
        export
    }

    /// Reads the Workshop ids again, on a worker thread.
    fn read(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let (group, registry, store) = (
            self.group.clone(),
            Arc::clone(&self.registry),
            Arc::clone(&self.store),
        );
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-preset-export".to_owned())
            .spawn(move || {
                let known = store.workshop();
                let result = generate(&group, &|name| registry.get(name), Some(&known));
                sender.send(result).ok();
                ctx.request_repaint();
            });
        self.stage = match spawned {
            Ok(_) => Stage::Reading(receiver),
            Err(err) => Stage::Failed(vec![format!("Cannot read the mods: {err}")]),
        };
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, toasts: &mut Toaster) -> Shown {
        if self.advance(ctx, toasts) {
            return Shown::Closed(None);
        }
        let title = format!("Export {}", self.group.name);
        let (ok, ok_enabled) = match &self.stage {
            Stage::Ids { .. } => (Some("Save and export"), true),
            Stage::Reading(_) | Stage::Saving(_) | Stage::Failed(_) => (None, false),
        };
        let mut dialog = Dialog::new("a3s_viewer::sheet::preset_export", title)
            .default_button(Some(DialogButton::Cancel));
        dialog = match ok {
            Some(label) => dialog
                .ok_label(label)
                .ok_enabled(ok_enabled)
                .default_button(Some(DialogButton::Ok)),
            None => dialog.show_ok(false).cancel_label("Close"),
        };
        let action = dialog.show(ctx, |ui| self.body(ui));
        match action {
            DialogAction::None => {
                if matches!(self.stage, Stage::Saving(_)) && !self.picker.is_open() {
                    // The save dialog was cancelled.
                    return Shown::Closed(None);
                }
                Shown::Open
            }
            DialogAction::Ok => {
                self.save_ids(ctx);
                Shown::Open
            }
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }

    /// Takes the read result and the chosen file; returns whether the preset was written (or
    /// failed to be).
    fn advance(&mut self, ctx: &egui::Context, toasts: &mut Toaster) -> bool {
        if let Stage::Reading(receiver) = &self.stage {
            match receiver.try_recv() {
                Ok(result) => self.read_done(ctx, result),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.stage = Stage::Failed(vec!["The mods could not be read.".to_owned()]);
                }
            }
        }
        if let Stage::Saving(result) = &self.stage
            && let Some(path) = self.picker.poll()
        {
            let path = ensure_html_extension(&path);
            match write(&path, result) {
                Ok(()) => {
                    toasts.success(format!("{} Saved to {}", result.summary(), path.display()));
                    for warning in result.warnings() {
                        toasts.warning(warning.clone());
                    }
                }
                Err(err) => toasts.error(format!("The preset was not saved: {err}")),
            }
            return true;
        }
        false
    }

    fn read_done(&mut self, ctx: &egui::Context, result: ExportResult) {
        if !result.metadata_issues().is_empty() {
            let issues = result.metadata_issues().to_vec();
            let fields = issues.iter().map(MetadataIssue::prefill).collect();
            self.stage = Stage::Ids {
                issues,
                fields,
                error: None,
            };
        } else if !result.errors().is_empty() {
            self.stage = Stage::Failed(result.errors().to_vec());
        } else {
            let file_name = suggested_file_name(result.preset_name());
            self.picker
                .open(ctx, "Export Arma 3 Launcher preset", None, &file_name);
            self.stage = Stage::Saving(Box::new(result));
        }
    }

    /// Remembers the entered Workshop ids and reads the mods again.
    fn save_ids(&mut self, ctx: &egui::Context) {
        let Stage::Ids {
            issues,
            fields,
            error,
        } = &mut self.stage
        else {
            return;
        };
        let mut entries = Vec::new();
        for (issue, (id, name)) in issues.iter().zip(fields.iter()) {
            match validate_metadata_input(id, name) {
                Ok(entry) => entries.push((issue.addon_key.clone(), entry)),
                Err(message) => {
                    *error = Some(format!("{}: {message}", issue.addon_name));
                    return;
                }
            }
        }
        let workshop = self.store.workshop();
        for (key, (id, name)) in &entries {
            if let Err(err) = workshop.save(key, id, name) {
                *error = Some(format!("The Workshop ids were not saved: {err}"));
                return;
            }
        }
        self.read(ctx);
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        ui.set_width(480.0);
        match &mut self.stage {
            Stage::Reading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading the Workshop ids of the enabled mods…");
                });
            }
            Stage::Ids {
                issues,
                fields,
                error,
            } => {
                ui.add(
                    Label::new(
                        "These mods do not say which Workshop item they are. Enter their \
                         Workshop id and name; they are remembered for the next export.",
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (issue, (id, name)) in issues.iter().zip(fields.iter_mut()) {
                            frames::group(ui, |ui| {
                                ui.label(RichText::new(&issue.addon_name).strong());
                                ui.label(
                                    RichText::new(issue.reason.as_str())
                                        .small()
                                        .color(palette.text_secondary),
                                );
                                ui.horizontal(|ui| {
                                    ui.label("Workshop id");
                                    ui.add(TextEdit::singleline(id).desired_width(140.0));
                                    ui.label("Name");
                                    ui.add(TextEdit::singleline(name).desired_width(160.0));
                                });
                            });
                            ui.add_space(4.0);
                        }
                    });
                if let Some(error) = error {
                    ui.label(RichText::new(error.as_str()).color(palette.status_danger));
                }
            }
            Stage::Saving(_) => {
                ui.label("Choose where to save the preset.");
            }
            Stage::Failed(errors) => {
                ui.label("The group cannot be exported:");
                for error in errors {
                    ui.add(Label::new(format!("• {error}")).wrap());
                }
            }
        }
    }
}
