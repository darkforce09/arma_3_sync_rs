//! The Add repository sheet.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::theme;
use egui::RichText;

use super::Shown;

pub(super) fn show(ctx: &egui::Context) -> Shown {
    let action = Dialog::new("a3s_viewer::sheet::add_repository", "Add repository")
        .show_ok(false)
        .cancel_label("Close")
        .default_button(Some(DialogButton::Cancel))
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label("Paste the auto-config link your group shared.");
            ui.label(
                RichText::new("Adding repositories is not available in this window yet.")
                    .color(theme::palette(ctx).text_secondary),
            );
        });
    if action == DialogAction::None {
        Shown::Open
    } else {
        Shown::Closed(None)
    }
}
