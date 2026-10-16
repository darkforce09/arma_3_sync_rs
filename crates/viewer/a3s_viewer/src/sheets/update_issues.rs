//! The Update issues sheet: after a download with failed files, one plain line per file, Retry
//! per file or for all, the report for the clipboard, and Play anyway (not while the game is
//! already starting after Update and play).

use std::collections::BTreeSet;
use std::time::SystemTime;

use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_sync::EventError;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::button;
use a3s_viewer_context::sync::{self, DownloadResult};
use a3s_viewer_context::{Command, ViewerContext};
use egui::{Align, Label, Layout, RichText, Ui};

use super::{SheetOutcome, Shown};
use crate::toasts::Toaster;

/// The sheet's state.
pub(crate) struct UpdateIssues {
    repository: String,

    /// Files whose Retry was clicked, for the download that finished at `shown`.
    retried: BTreeSet<String>,
    shown: Option<SystemTime>,

    /// The download was part of Update and play: the game starts anyway, so the sheet offers
    /// no Play.
    game_starting: bool,
}

impl UpdateIssues {
    /// The repository whose failed files it lists.
    pub(crate) fn repository(&self) -> &str {
        &self.repository
    }

    pub(crate) fn new(repository: impl Into<String>, game_starting: bool) -> Self {
        Self {
            repository: repository.into(),
            retried: BTreeSet::new(),
            shown: None,
            game_starting,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        let last = viewer
            .read()
            .sync
            .downloads
            .last_result(&self.repository)
            .cloned();
        let errors = match last {
            Some(record) => match record.result {
                DownloadResult::Issues(errors) => {
                    if self.shown != Some(record.finished) {
                        self.shown = Some(record.finished);
                        self.retried.clear();
                    }
                    errors
                }
                // Replaced by a download without failed files: nothing left to show.
                _ => return Shown::Closed(None),
            },
            None => return Shown::Closed(None),
        };
        let mut retry_all = false;
        let title = format!("Some files of {} were not updated", self.repository);
        let action = Dialog::new("a3s_viewer::sheet::update_issues", title)
            .ok_label("Play anyway")
            .show_ok(!self.game_starting)
            .cancel_label("Close")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| {
                retry_all = self.body(ui, viewer, toasts, &errors);
            });
        if retry_all {
            self.retry(viewer, toasts, Vec::new());
            return Shown::Closed(None);
        }
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => Shown::Closed(Some(SheetOutcome::Play)),
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }

    /// The lines and the footer buttons; returns whether Retry all was clicked.
    fn body(
        &mut self,
        ui: &mut Ui,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
        errors: &[EventError],
    ) -> bool {
        let palette = theme::palette(ui.ctx());
        ui.set_width(520.0);
        ui.label(if self.game_starting {
            "The game starts anyway; these mods may be out of date."
        } else {
            "The game can still start; these mods may be out of date."
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                frames::group(ui, |ui| {
                    for (index, error) in errors.iter().enumerate() {
                        if index > 0 {
                            frames::group_separator(ui);
                        }
                        let file = error.relative_path.clone().unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
                            ui.vertical(|ui| {
                                ui.add(Label::new(RichText::new(&file).strong()).truncate());
                                ui.add(
                                    Label::new(
                                        RichText::new(plain_text(error))
                                            .color(palette.text_secondary),
                                    )
                                    .wrap(),
                                )
                                .on_hover_text(&error.message);
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if self.retried.contains(&file) {
                                    ui.add_enabled(false, egui::Button::new("Retrying…"));
                                } else if !file.is_empty() && button(ui, "Retry").clicked() {
                                    self.retry(viewer, toasts, vec![file.clone()]);
                                    self.retried.insert(file.clone());
                                }
                            });
                        });
                    }
                });
            });
        ui.add_space(8.0);
        let mut retry_all = false;
        ui.horizontal(|ui| {
            if button(ui, &format!("{} Copy report", icons::COPY)).clicked() {
                ui.ctx().copy_text(report(&self.repository, errors));
                toasts.info("Report copied");
            }
            if button(ui, &format!("{} Retry all", icons::ARROW_CLOCKWISE)).clicked() {
                retry_all = true;
            }
        });
        retry_all
    }

    fn retry(&self, viewer: &ViewerContext, toasts: &mut Toaster, files: Vec<String>) {
        let command = Command::Sync(sync::Command::Retry {
            repository: self.repository.clone(),
            files,
        });
        if let Err(err) = viewer.send(command) {
            toasts.error(err.user_message());
        }
    }
}

/// What went wrong with a file, in plain words.
fn plain_text(error: &EventError) -> String {
    let text = match error.kind {
        ErrorKind::NotFound => "The file is missing on the server.",
        ErrorKind::ConnectionFailed => "The server could not be reached.",
        ErrorKind::ConnectTimeout | ErrorKind::ReadTimeout | ErrorKind::Timeout => {
            "The server stopped answering."
        }
        ErrorKind::Unauthorized => "The server refused the login.",
        ErrorKind::TlsValidation => "The server's certificate could not be verified.",
        ErrorKind::IncompleteTransfer => "The download stopped before the file was complete.",
        ErrorKind::PermissionDenied => "The file could not be written: access was denied.",
        ErrorKind::InvalidData => "The downloaded file is damaged.",
        ErrorKind::WriteFailed | ErrorKind::Io => "The file could not be saved to disk.",
        ErrorKind::Cancelled => "The download was cancelled.",
        _ => return error.message.clone(),
    };
    text.to_owned()
}

/// The report copied to the clipboard: one line per file with the plain text and the error.
fn report(repository: &str, errors: &[EventError]) -> String {
    let mut lines = vec![format!(
        "Update of {repository}: {} files failed",
        errors.len()
    )];
    for error in errors {
        lines.push(format!(
            "{}: {} ({})",
            error.relative_path.as_deref().unwrap_or("-"),
            plain_text(error),
            error.message
        ));
    }
    lines.join("\n")
}
