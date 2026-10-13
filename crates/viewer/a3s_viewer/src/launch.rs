//! The Play button: starting the game through the context, and what the window does when the
//! game starts.

use std::path::Path;

use a3s_foundation::error::UserFacing as _;
use a3s_launch::{LaunchEvent, LaunchTarget};
use a3s_types::constants::MinimizationType;
use a3s_viewer_context::{Command, ViewerContext, launch};

use crate::shell::Action;
use crate::toasts::Toaster;
use crate::windows::SettingsPane;

/// What the Play button asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlayRequest {
    /// The profile to launch.
    pub(crate) profile: String,

    /// The profile's repositories have updates to download before the game starts.
    pub(crate) update_first: bool,
}

/// Starts the game of the active profile, after the updates when asked. Without a usable Arma 3
/// executable nothing starts and a toast offers to choose it in Settings > Game.
pub(crate) fn request(viewer: &ViewerContext, play: &PlayRequest, toasts: &mut Toaster) {
    let exe = viewer
        .read()
        .client
        .launcher_options()
        .and_then(|options| options.arma3_exe_path.clone())
        .filter(|path| !path.trim().is_empty());
    let problem = match &exe {
        None => Some("Arma 3 is not set up yet.".to_owned()),
        Some(path) if !Path::new(path).exists() => Some(format!("Arma 3 was not found at {path}.")),
        Some(_) => None,
    };
    if let Some(problem) = problem {
        toasts.action(
            problem,
            "Choose folder…",
            Action::OpenSettings(SettingsPane::Game),
        );
        return;
    }
    tracing::info!(
        "Play requested for profile {} (update first: {})",
        play.profile,
        play.update_first
    );
    let command = Command::Launch(launch::Command::Play {
        update_first: play.update_first,
    });
    if let Err(err) = viewer.send(command) {
        toasts.error(err.user_message());
    }
}

/// Shows a launch event and returns what the window does about it: once the game started, the
/// "game launch" preference hides it to the tray, minimizes it, closes it or does nothing.
// Mirrors ui/main/LaunchPanel.java:345-360
pub(crate) fn on_event(
    event: &launch::Event,
    viewer: &ViewerContext,
    toasts: &mut Toaster,
    tray_available: bool,
) -> Option<Action> {
    match event {
        launch::Event::UpdateIssues { repositories } => {
            toasts.warning(format!(
                "Not everything was updated ({}); starting the game anyway",
                repositories.join(", ")
            ));
            None
        }
        launch::Event::MissingAddons { names } => {
            toasts.warning(format!(
                "Not on this computer, starting without: {}",
                names.join(", ")
            ));
            None
        }
        launch::Event::Game(LaunchEvent::Started {
            target: LaunchTarget::Arma3,
            ..
        }) => {
            let (preference, auto_restart) = {
                let state = viewer.read();
                let auto_restart = state
                    .client
                    .launcher_options()
                    .is_some_and(|options| options.auto_restart);
                (
                    state.client.preferences().launch_panel_game_launch,
                    auto_restart,
                )
            };
            match preference {
                MinimizationType::Nothing => None,
                MinimizationType::Tray if tray_available => Some(Action::Hide),
                MinimizationType::TaskBar | MinimizationType::Tray => Some(Action::Minimize),
                // Auto-restart needs the application running.
                MinimizationType::Close if auto_restart => None,
                MinimizationType::Close => Some(Action::Quit),
            }
        }
        launch::Event::Game(LaunchEvent::Restarting { run }) => {
            toasts.info(format!("Restarting Arma 3 ({run})"));
            None
        }
        launch::Event::Game(LaunchEvent::Error {
            target: LaunchTarget::External { name },
            error,
        }) => {
            toasts.error(format!("{name} could not start: {error}"));
            None
        }
        _ => None,
    }
}
