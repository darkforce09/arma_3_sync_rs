//! The Mods screen: the active profile's addons and addon groups.

use a3s_ui::icon::icons;
use egui::Ui;

pub(super) fn show(ui: &mut Ui) {
    super::empty_state(
        ui,
        icons::PUZZLE_PIECE,
        "Mods",
        "The addons of the active profile appear here.",
    );
}
