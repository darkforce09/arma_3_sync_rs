//! The Servers screen: favourite servers to join.

use a3s_ui::icon::icons;
use egui::Ui;

pub(super) fn show(ui: &mut Ui) {
    super::empty_state(
        ui,
        icons::HARD_DRIVES,
        "Servers",
        "Your favourite servers appear here.",
    );
}
