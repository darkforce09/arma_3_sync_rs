//! Windows besides the main one: Settings and the tool windows, each its own OS window
//! (an egui deferred viewport; embedded as an in-app window where the backend has only one).

mod settings;
mod tools;

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{ViewportBuilder, ViewportClass, ViewportId};
use parking_lot::Mutex;

pub use self::settings::SettingsPane;

/// A window opened from the main window's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecondaryWindow {
    /// Settings, with its pane list.
    Settings,

    /// Extracting server keys (`.bikey`) from the mods.
    ServerKeys,

    /// Installing the TFAR or ACRE 2 radio plugin.
    RadioPlugin,

    /// Following the game's RPT log.
    GameLog,
}

impl SecondaryWindow {
    /// The tool windows, in menu order.
    pub const TOOLS: [Self; 3] = [Self::ServerKeys, Self::RadioPlugin, Self::GameLog];

    /// The window title, also the menu item.
    pub fn title(self) -> &'static str {
        match self {
            Self::Settings => "Settings",
            Self::ServerKeys => "Server keys",
            Self::RadioPlugin => "Radio plugin",
            Self::GameLog => "Game log",
        }
    }

    fn viewport_id(self) -> ViewportId {
        ViewportId::from_hash_of(("a3s_viewer::window", self))
    }

    fn builder(self) -> ViewportBuilder {
        let (size, min) = match self {
            Self::Settings => ([720.0, 480.0], [560.0, 360.0]),
            Self::GameLog => ([760.0, 520.0], [420.0, 260.0]),
            Self::ServerKeys | Self::RadioPlugin => ([520.0, 360.0], [360.0, 240.0]),
        };
        ViewportBuilder::default()
            .with_title(self.title())
            .with_inner_size(size)
            .with_min_inner_size(min)
    }
}

/// Which secondary windows are open, shared with their viewport callbacks.
#[derive(Debug, Default)]
struct OpenWindows {
    open: BTreeSet<SecondaryWindow>,
    settings_pane: SettingsPane,
}

/// The secondary windows. Call [`Self::show`] every frame of the main window: a deferred
/// viewport closes when its parent stops showing it.
#[derive(Clone, Debug, Default)]
pub(crate) struct SecondaryWindows {
    state: Arc<Mutex<OpenWindows>>,
}

impl SecondaryWindows {
    /// Opens `window`, or brings it to the front when it is open.
    pub(crate) fn open(&self, ctx: &egui::Context, window: SecondaryWindow) {
        if !self.state.lock().open.insert(window) {
            ctx.send_viewport_cmd_to(window.viewport_id(), egui::ViewportCommand::Focus);
        }
    }

    /// Opens the Settings window on `pane`.
    pub(crate) fn open_settings(&self, ctx: &egui::Context, pane: SettingsPane) {
        self.state.lock().settings_pane = pane;
        self.open(ctx, SecondaryWindow::Settings);
    }

    /// Whether `window` is open.
    pub(crate) fn is_open(&self, window: SecondaryWindow) -> bool {
        self.state.lock().open.contains(&window)
    }

    /// Shows every open window.
    pub(crate) fn show(&self, ctx: &egui::Context) {
        let open: Vec<SecondaryWindow> = self.state.lock().open.iter().copied().collect();
        for window in open {
            let state = Arc::clone(&self.state);
            ctx.show_viewport_deferred(window.viewport_id(), window.builder(), move |ui, class| {
                let close = ui.ctx().input(|i| i.viewport().close_requested());
                content(ui, class, window, &state);
                if close {
                    state.lock().open.remove(&window);
                }
            });
        }
    }
}

fn content(
    ui: &mut egui::Ui,
    class: ViewportClass,
    window: SecondaryWindow,
    state: &Mutex<OpenWindows>,
) {
    let body = |ui: &mut egui::Ui| match window {
        SecondaryWindow::Settings => {
            let mut pane = state.lock().settings_pane;
            settings::show(ui, &mut pane);
            state.lock().settings_pane = pane;
        }
        SecondaryWindow::ServerKeys | SecondaryWindow::RadioPlugin | SecondaryWindow::GameLog => {
            tools::show(ui, window);
        }
    };
    if class == ViewportClass::EmbeddedWindow {
        body(ui);
    } else {
        let fill = a3s_ui::theme::palette(ui.ctx()).window_background;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(fill))
            .show(ui, body);
    }
}
