//! The tool windows: server keys, radio plugin and game log.

use a3s_ui::icon::icons;
use a3s_ui::theme;
use egui::{Align, Layout, RichText, Ui};

use super::SecondaryWindow;

/// Shows the tool window `window`.
pub(super) fn show(ui: &mut Ui, window: SecondaryWindow) {
    let (icon, line) = match window {
        SecondaryWindow::ServerKeys => (
            icons::KEY,
            "Copies the server keys (.bikey) of your mods into one folder.",
        ),
        SecondaryWindow::RadioPlugin => (
            icons::HEADSET,
            "Installs the TFAR or ACRE 2 plugin into TeamSpeak.",
        ),
        SecondaryWindow::GameLog | SecondaryWindow::Settings => (
            icons::FILE_TEXT,
            "Follows the game's RPT log while it runs.",
        ),
    };
    let palette = theme::palette(ui.ctx());
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(24.0);
        ui.label(RichText::new(icon).size(32.0).color(palette.text_secondary));
        ui.label(RichText::new(window.title()).heading());
        ui.label(RichText::new(line).color(palette.text_secondary));
    });
}
