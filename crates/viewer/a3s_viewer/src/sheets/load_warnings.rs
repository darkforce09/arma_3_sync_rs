//! The sheet listing what went wrong while loading the client data.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::icon::icons;
use a3s_ui::theme;
use egui::RichText;

use super::Shown;

pub(super) fn show(ctx: &egui::Context, warnings: &[String]) -> Shown {
    let action = Dialog::new(
        "a3s_viewer::sheet::load_warnings",
        "Some settings could not be loaded",
    )
    .show_ok(false)
    .cancel_label("OK")
    .default_button(Some(DialogButton::Cancel))
    .show(ctx, |ui| {
        ui.set_max_width(480.0);
        ui.label("Arma3Sync started with defaults for these files:");
        ui.add_space(4.0);
        let color = theme::palette(ctx).status_warning;
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for warning in warnings {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(icons::WARNING).color(color));
                        ui.label(warning);
                    });
                }
            });
    });
    if action == DialogAction::None {
        Shown::Open
    } else {
        Shown::Closed(None)
    }
}
