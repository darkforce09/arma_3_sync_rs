//! The About sheet: the version, the licence, where the project lives, and the credits.
// Mirrors ui/help/AboutDialog.java:38-179, ui/help/AboutCreditsDialog.java:64-101

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::theme;
use a3s_ui::widgets::button;
use egui::RichText;

use super::{SheetOutcome, Shown};
use crate::windows::SettingsPane;

/// The application version, shared by the whole workspace.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the project lives.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub(super) fn show(ctx: &egui::Context) -> Shown {
    let mut logs = false;
    let action = Dialog::new("a3s_viewer::sheet::about", "About Arma3Sync")
        .show_ok(false)
        .cancel_label("Close")
        .default_button(Some(DialogButton::Cancel))
        .show(ctx, |ui| {
            let palette = theme::palette(ui.ctx());
            ui.set_width(400.0);
            ui.label(RichText::new(format!("Arma3Sync {VERSION}")).heading());
            ui.label("Synchronizes, publishes and starts Arma 3 mod repositories.");
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "A rewrite of Arma3Sync, originally developed by the [S.o.E] Team. Its \
                     repositories stay compatible with the Java version.",
                )
                .color(palette.text_secondary),
            );
            ui.hyperlink_to("Project page", REPOSITORY);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Licensed under the GNU General Public License 3.0 or later. Geist fonts \
                     under the SIL Open Font License 1.1, Phosphor icons under the MIT \
                     License.",
                )
                .small()
                .color(palette.text_secondary),
            );
            ui.add_space(8.0);
            logs = button(ui, "Log files…").clicked();
        });
    if logs {
        return Shown::Closed(Some(SheetOutcome::OpenSettings(SettingsPane::Data)));
    }
    match action {
        DialogAction::None => Shown::Open,
        DialogAction::Ok | DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
    }
}
