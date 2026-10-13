//! The Repository screen: one repository's state, changes and changelog.

use a3s_ui::icon::icons;
use egui::Ui;

pub(super) fn show(ui: &mut Ui, name: &str) {
    super::empty_state(
        ui,
        icons::DATABASE,
        name,
        "The repository's changes and changelog appear here.",
    );
}
