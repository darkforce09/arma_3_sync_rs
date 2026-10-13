//! The content area: one module per screen of the sidebar.

mod activity;
mod mods;
mod publishing;
mod repository;
mod servers;

use std::time::{Duration, SystemTime};

use a3s_ui::theme;
use a3s_viewer_context::ViewerContext;
use egui::{Align, Layout, RichText, Ui};

use self::mods::ModsScreen;
use self::servers::ServersScreen;
use crate::sheets::Sheets;
use crate::shell::{Action, Screen};
use crate::toasts::Toaster;

/// The state screens keep between frames, kept by the window.
#[derive(Default)]
pub(crate) struct ScreenStates {
    mods: ModsScreen,
    servers: ServersScreen,
}

/// What a screen works with besides its `Ui` and its own state.
pub(crate) struct ScreenCx<'a> {
    pub(crate) viewer: &'a ViewerContext,

    /// The toolbar search text.
    pub(crate) search: &'a str,
    pub(crate) toasts: &'a mut Toaster,
    pub(crate) sheets: &'a mut Sheets,

    /// What the screen asks the window to do, applied after the frame.
    pub(crate) actions: Vec<Action>,
}

/// Shows `screen` in the content area.
pub(crate) fn show(ui: &mut Ui, screen: &Screen, states: &mut ScreenStates, cx: &mut ScreenCx<'_>) {
    match screen {
        Screen::Mods => states.mods.show(ui, cx),
        Screen::Servers => states.servers.show(ui, cx),
        Screen::Activity => activity::show(ui, cx),
        Screen::Repository(name) => repository::show(ui, name, cx),
        Screen::Publishing(name) => publishing::show(ui, name, cx),
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

/// A section title above a group.
fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(4.0);
}

/// `bytes` as `1.12 MB`.
pub(crate) fn size_text(bytes: u64) -> String {
    a3s_foundation::format::size(i64::try_from(bytes).unwrap_or(i64::MAX))
}

/// A remaining time as `about 3 min`.
fn time_left_text(remaining: Duration) -> String {
    let seconds = remaining.as_secs();
    match seconds {
        0..60 => "less than a minute left".to_owned(),
        60..3600 => format!("about {} min left", seconds.div_ceil(60)),
        _ => format!(
            "about {} h {} min left",
            seconds / 3600,
            (seconds % 3600) / 60
        ),
    }
}

/// How long ago `then` was, as `5 min ago`.
fn ago_text(then: SystemTime, now: SystemTime) -> String {
    let seconds = now.duration_since(then).unwrap_or_default().as_secs();
    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// `count` with the singular or plural noun: `1 mod`, `3 mods`.
pub(crate) fn count_text(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}
