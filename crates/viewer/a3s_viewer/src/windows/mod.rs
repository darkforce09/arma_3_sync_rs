//! Windows besides the main one: Settings and the tool windows, each its own OS window
//! (an egui viewport; embedded as an in-app window where the backend has only one).

mod settings;
mod tools;

use std::collections::BTreeSet;

use a3s_viewer_context::ViewerContext;
use egui::{ViewportBuilder, ViewportClass, ViewportId};

pub(crate) use self::settings::Appearance;
pub use self::settings::{SettingsPane, SettingsWindow};
use crate::desktop::Desktop;

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
            Self::Settings => ([760.0, 560.0], [600.0, 400.0]),
            Self::GameLog => ([760.0, 520.0], [420.0, 260.0]),
            Self::ServerKeys | Self::RadioPlugin => ([520.0, 360.0], [360.0, 240.0]),
        };
        ViewportBuilder::default()
            .with_title(self.title())
            .with_inner_size(size)
            .with_min_inner_size(min)
    }
}

/// The secondary windows. Call [`Self::show`] every frame of the main window: a viewport
/// closes when its parent stops showing it.
///
/// Every window is an immediate viewport, drawn within the main window's frame so it can use
/// the context.
pub(crate) struct SecondaryWindows {
    open_tools: BTreeSet<SecondaryWindow>,
    tools: tools::ToolWindows,
    settings: Option<SettingsWindow>,
    desktop: Desktop,
}

impl SecondaryWindows {
    pub(crate) fn new(desktop: Desktop) -> Self {
        Self {
            open_tools: BTreeSet::new(),
            tools: tools::ToolWindows::default(),
            settings: None,
            desktop,
        }
    }

    /// The desktop the Settings window integrates with, from the next time it opens.
    pub(crate) fn set_desktop(&mut self, desktop: Desktop) {
        self.desktop = desktop;
    }

    /// Opens `window`, or brings it to the front when it is open.
    pub(crate) fn open(&mut self, ctx: &egui::Context, window: SecondaryWindow) {
        let opened = if window == SecondaryWindow::Settings {
            let closed = self.settings.is_none();
            if closed {
                self.settings = Some(SettingsWindow::new(self.desktop.clone()));
            }
            closed
        } else {
            self.open_tools.insert(window)
        };
        if !opened {
            ctx.send_viewport_cmd_to(window.viewport_id(), egui::ViewportCommand::Focus);
        }
    }

    /// Opens the Settings window on `pane`.
    pub(crate) fn open_settings(&mut self, ctx: &egui::Context, pane: SettingsPane) {
        self.open(ctx, SecondaryWindow::Settings);
        if let Some(settings) = &mut self.settings {
            settings.set_pane(pane);
        }
    }

    /// Whether `window` is open.
    pub(crate) fn is_open(&self, window: SecondaryWindow) -> bool {
        if window == SecondaryWindow::Settings {
            self.settings.is_some()
        } else {
            self.open_tools.contains(&window)
        }
    }

    /// The pane the Settings window shows, when it is open.
    pub(crate) fn settings_pane(&self) -> Option<SettingsPane> {
        self.settings.as_ref().map(SettingsWindow::pane)
    }

    /// Shows every open window.
    pub(crate) fn show(&mut self, ctx: &egui::Context, viewer: &ViewerContext) {
        if let Some(settings) = &mut self.settings {
            let window = SecondaryWindow::Settings;
            let close =
                ctx.show_viewport_immediate(window.viewport_id(), window.builder(), |ui, class| {
                    let close = ui.ctx().input(|i| i.viewport().close_requested());
                    in_window(ui, class, |ui| settings.show(ui, viewer));
                    close
                });
            if close {
                self.settings = None;
            }
        }

        let open: Vec<SecondaryWindow> = self.open_tools.iter().copied().collect();
        for window in open {
            let tools = &mut self.tools;
            let close =
                ctx.show_viewport_immediate(window.viewport_id(), window.builder(), |ui, class| {
                    let close = ui.ctx().input(|i| i.viewport().close_requested());
                    in_window(ui, class, |ui| tools.show(ui, window, viewer));
                    close
                });
            if close {
                self.open_tools.remove(&window);
                self.tools.close(window, viewer);
            }
        }
    }
}

/// Runs `body` in the viewport: directly when embedded, else over the window background.
fn in_window(ui: &mut egui::Ui, class: ViewportClass, body: impl FnOnce(&mut egui::Ui)) {
    if class == ViewportClass::EmbeddedWindow {
        body(ui);
    } else {
        let fill = a3s_ui::theme::palette(ui.ctx()).window_background;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(fill))
            .show(ui, body);
    }
}
