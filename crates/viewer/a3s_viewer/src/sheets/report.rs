//! The Download report sheet: what a finished download of a repository did (files and bytes
//! downloaded, what partial and compressed transfers saved, the errors), to read or copy.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::widgets::button;
use egui::{Label, RichText};

use super::Shown;
use crate::toasts::Toaster;

/// The sheet's state.
pub(crate) struct ReportSheet {
    repository: String,
    text: String,
}

impl ReportSheet {
    pub(crate) fn new(repository: &str, text: &str) -> Self {
        Self {
            repository: repository.to_owned(),
            text: text.to_owned(),
        }
    }

    pub(super) fn show(&self, ctx: &egui::Context, toasts: &mut Toaster) -> Shown {
        let title = format!("Download report of {}", self.repository);
        let action = Dialog::new("a3s_viewer::sheet::report", title)
            .show_ok(false)
            .cancel_label("Close")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.add(Label::new(RichText::new(&self.text).monospace()).wrap());
                    });
                ui.add_space(8.0);
                if button(ui, "Copy report").clicked() {
                    ui.ctx().copy_text(self.text.clone());
                    toasts.info("Report copied");
                }
            });
        if action == DialogAction::None {
            Shown::Open
        } else {
            Shown::Closed(None)
        }
    }
}
