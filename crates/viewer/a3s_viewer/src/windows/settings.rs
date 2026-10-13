//! The Settings window: a pane list on the left, the selected pane's grouped form on the right.
//! Settings save as soon as they change; there is no OK or Apply.

use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::SidebarItem;
use egui::{RichText, Ui};

/// A pane of the Settings window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsPane {
    /// Appearance, start-up, tray and repository checks.
    #[default]
    General,

    /// The Arma 3 executable, launch parameters and addon folders.
    Game,

    /// Profiles: create, duplicate, rename, remove.
    Profiles,

    /// Download connections, speed limit and partial transfers.
    Downloads,

    /// Proxy and timeouts.
    Network,

    /// External applications and radio plugins.
    CompanionApps,

    /// Auto-config import and export, Java data import, logs.
    Data,
}

impl SettingsPane {
    /// Every pane, in list order.
    pub const ALL: [Self; 7] = [
        Self::General,
        Self::Game,
        Self::Profiles,
        Self::Downloads,
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
            Self::Downloads => "Downloads",
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
            Self::Downloads => icons::DOWNLOAD_SIMPLE,
            Self::Network => icons::GLOBE,
            Self::CompanionApps => icons::HEADSET,
            Self::Data => icons::HARD_DRIVES,
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::General => "Appearance, start-up, tray and repository checks.",
            Self::Game => "The Arma 3 executable, launch parameters and addon folders.",
            Self::Profiles => "Create, duplicate, rename and remove profiles.",
            Self::Downloads => "Connections, speed limit and partial file transfers.",
            Self::Network => "Proxy and connection timeouts.",
            Self::CompanionApps => "Programs started with the game, and radio plugins.",
            Self::Data => "Auto-config import and export, Java data import and logs.",
        }
    }
}

/// Shows the pane list and the selected pane.
pub(super) fn show(ui: &mut Ui, pane: &mut SettingsPane) {
    let def = theme::current(ui.ctx());
    egui::Panel::left("a3s_viewer::settings::panes")
        .exact_size(frames::SIDEBAR_WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(frames::sidebar_frame(def))
        .show(ui, |ui| {
            for candidate in SettingsPane::ALL {
                if SidebarItem::new(candidate.icon(), candidate.label())
                    .show(ui, *pane == candidate)
                    .clicked()
                {
                    *pane = candidate;
                }
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(20))
        .show(ui, |ui| {
            ui.label(RichText::new(pane.label()).heading());
            ui.add_space(8.0);
            frames::group(ui, |ui| {
                ui.label(RichText::new(pane.summary()).color(def.palette.text_secondary));
            });
        });
}
