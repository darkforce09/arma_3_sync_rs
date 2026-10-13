//! The Activity screen: running and recent transfers.

use a3s_ui::icon::icons;
use egui::Ui;

pub(super) fn show(ui: &mut Ui) {
    super::empty_state(
        ui,
        icons::PULSE,
        "Activity",
        "Downloads and checks in progress appear here.",
    );
}
