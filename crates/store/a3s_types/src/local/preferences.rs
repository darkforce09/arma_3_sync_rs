use serde::{Deserialize, Serialize};

use crate::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, StartWithOs,
};

/// UI preferences (`fr.soe.a3s.domain.Preferences`, `a3s.prefs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Main window behaviour when the game is launched.
    pub launch_panel_game_launch: MinimizationType,

    /// Main window behaviour when it is minimized.
    pub launch_panel_minimized: MinimizationType,
    pub look_and_feel: LookAndFeel,

    /// `iconResizeSize`. The initializer is `Disabled` (Java `NONE`), but a null value reads
    /// as `Auto`.
    pub icon_resize_size: IconResize,

    /// `startWithOS`.
    pub start_with_os: StartWithOs,
    pub check_repositories_frequency: CheckRepositoriesFrequency,

    /// `preferGitHubUpdates`; a null value reads as `true`.
    pub prefer_github_updates: bool,
}

impl Default for Preferences {
    /// The Java field initializer values.
    // Mirrors domain/Preferences.java:18-26,81-83
    fn default() -> Self {
        Self {
            launch_panel_game_launch: MinimizationType::TaskBar,
            launch_panel_minimized: MinimizationType::TaskBar,
            look_and_feel: LookAndFeel::LafDefault,
            icon_resize_size: IconResize::Disabled,
            start_with_os: StartWithOs::Disabled,
            check_repositories_frequency: CheckRepositoriesFrequency::Freq3,
            prefer_github_updates: true,
        }
    }
}
