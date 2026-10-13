//! The content area: one module per screen of the sidebar.

mod activity;
mod mods;
mod publishing;
mod repository;
mod servers;

use a3s_ui::theme;
use a3s_viewer_context::ViewerContext;
use egui::{Align, Layout, RichText, Ui};

use crate::shell::Screen;
use crate::toasts::Toaster;

/// The state screens keep between frames.
#[derive(Default)]
pub(crate) struct ScreenStates {
    servers: servers::ServersScreen,
}

/// Shows `screen` in the content area.
pub(crate) fn show(
    ui: &mut Ui,
    screen: &Screen,
    states: &mut ScreenStates,
    viewer: &ViewerContext,
    toasts: &mut Toaster,
) {
    match screen {
        Screen::Mods => mods::show(ui),
        Screen::Servers => states.servers.show(ui, viewer, toasts),
        Screen::Activity => activity::show(ui),
        Screen::Repository(name) => repository::show(ui, name),
        Screen::Publishing(name) => publishing::show(ui, name),
    }
}

/// A centred icon, title and one line of explanation, for a screen without content.
fn empty_state(ui: &mut Ui, icon: &str, title: &str, line: &str) {
    let palette = theme::palette(ui.ctx());
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() / 3.0).max(24.0));
        ui.label(RichText::new(icon).size(40.0).color(palette.text_secondary));
        ui.add_space(8.0);
        ui.label(RichText::new(title).heading());
        ui.label(RichText::new(line).color(palette.text_secondary));
    });
}
