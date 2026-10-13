//! The Publishing screen: building and uploading a repository from its local folder.

use a3s_ui::icon::icons;
use egui::Ui;

pub(super) fn show(ui: &mut Ui, name: &str) {
    super::empty_state(
        ui,
        icons::CLOUD_ARROW_UP,
        &format!("Publish {name}"),
        "Building and uploading this repository appear here.",
    );
}
