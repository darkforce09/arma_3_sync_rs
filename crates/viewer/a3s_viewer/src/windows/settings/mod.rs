//! The Settings window: a pane list on the left, the selected pane's grouped form on the right.
//! Every change is written at once through the client data; there is no OK or Apply, and
//! removals offer Undo in the window's own toasts.

mod companion_apps;
mod data;
mod game;
mod general;
mod network;
mod profiles;

pub(crate) use self::general::Appearance;

use a3s_foundation::error::UserFacing;
use a3s_repo_files::services::ClientData;
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::SidebarItem;
use a3s_viewer_context::{Change, ViewerContext};
use egui::{Id, Label, RichText, Ui};

use crate::desktop::Desktop;
use crate::toasts::Toaster;

/// A pane of the Settings window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsPane {
    /// Appearance, start-up, tray, repository checks and the application version.
    #[default]
    General,

    /// The game of one profile: folder, start, in-game name and launch options.
    Game,

    /// Profiles: create, duplicate, rename, remove, desktop shortcut.
    Profiles,

    /// The proxy.
    Network,

    /// Programs started with the game.
    CompanionApps,

    /// Java data import, auto-config import and export, logs.
    Data,
}

impl SettingsPane {
    /// Every pane, in list order.
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Game,
        Self::Profiles,
        Self::Network,
        Self::CompanionApps,
        Self::Data,
    ];

    /// The pane's name in the list.
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Game => "Game",
            Self::Profiles => "Profiles",
            Self::Network => "Network",
            Self::CompanionApps => "Companion apps",
            Self::Data => "Data",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => icons::GEAR_SIX,
            Self::Game => icons::GAME_CONTROLLER,
            Self::Profiles => icons::USER_CIRCLE,
            Self::Network => icons::GLOBE,
            Self::CompanionApps => icons::HEADSET,
            Self::Data => icons::HARD_DRIVES,
        }
    }
}

/// The Settings window's content and state: the selected pane, each pane's drafts and pending
/// dialogs, and the window's toasts.
pub struct SettingsWindow {
    pane: SettingsPane,
    desktop: Desktop,
    toasts: Toaster,
    general: general::GeneralPane,
    game: game::GamePane,
    profiles: profiles::ProfilesPane,
    network: network::NetworkPane,
    companion_apps: companion_apps::CompanionAppsPane,
    data: data::DataPane,
}

impl SettingsWindow {
    /// The window over `desktop`, showing General.
    pub fn new(desktop: Desktop) -> Self {
        Self {
            pane: SettingsPane::default(),
            desktop,
            toasts: Toaster::with_id(Id::new("a3s_viewer::settings::toasts")),
            general: general::GeneralPane::default(),
            game: game::GamePane::default(),
            profiles: profiles::ProfilesPane::default(),
            network: network::NetworkPane::default(),
            companion_apps: companion_apps::CompanionAppsPane::default(),
            data: data::DataPane::default(),
        }
    }

    /// The selected pane.
    pub fn pane(&self) -> SettingsPane {
        self.pane
    }

    /// Selects `pane`.
    pub fn set_pane(&mut self, pane: SettingsPane) {
        self.pane = pane;
    }

    /// Shows the pane list, the selected pane and the window's toasts.
    pub fn show(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let def = theme::current(ui.ctx());
        egui::Panel::left("a3s_viewer::settings::panes")
            .exact_size(frames::SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(frames::sidebar_frame(def))
            .show(ui, |ui| {
                for candidate in SettingsPane::ALL {
                    if SidebarItem::new(candidate.icon(), candidate.label())
                        .show(ui, self.pane == candidate)
                        .clicked()
                    {
                        self.pane = candidate;
                    }
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(20))
            .show(ui, |ui| {
                ui.label(RichText::new(self.pane.label()).heading());
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| self.show_pane(ui, viewer));
            });
        self.toasts.show(ui, viewer);
    }

    fn show_pane(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let mut cx = PaneCx {
            viewer,
            toasts: &mut self.toasts,
            desktop: &self.desktop,
        };
        match self.pane {
            SettingsPane::General => self.general.show(ui, &mut cx),
            SettingsPane::Game => self.game.show(ui, &mut cx),
            SettingsPane::Profiles => self.profiles.show(ui, &mut cx),
            SettingsPane::Network => self.network.show(ui, &cx),
            SettingsPane::CompanionApps => self.companion_apps.show(ui, &mut cx),
            SettingsPane::Data => self.data.show(ui, &mut cx),
        }
    }
}

/// What a pane works with.
struct PaneCx<'a> {
    viewer: &'a ViewerContext,
    toasts: &'a mut Toaster,
    desktop: &'a Desktop,
}

impl PaneCx<'_> {
    /// Runs `f` on the client data under the write lock and announces `change`; a failure
    /// becomes an error toast and `None`.
    fn save<R, E: UserFacing>(
        &mut self,
        change: Option<Change>,
        f: impl FnOnce(&mut ClientData) -> Result<R, E>,
    ) -> Option<R> {
        self.report(self.viewer.mutate(change, |state| f(&mut state.client)))
    }

    /// The value of `result`, or `None` after an error toast.
    fn report<R, E: UserFacing>(&mut self, result: Result<R, E>) -> Option<R> {
        result
            .map_err(|err| self.toasts.error(err.user_message()))
            .ok()
    }
}

/// A line of secondary text.
fn note(ui: &mut Ui, text: impl Into<String>) {
    let color = theme::palette(ui.ctx()).text_secondary;
    ui.add(Label::new(RichText::new(text.into()).color(color)).wrap());
}

/// A heading above a group.
fn group_heading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}
