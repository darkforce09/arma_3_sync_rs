//! The Play button: starting the game through the context, and what the window does when the
//! game starts.

use std::path::Path;

use a3s_addons::{GroupStatus, check_duplicate_addons_selection, check_missing_selected_addons};
use a3s_foundation::error::UserFacing as _;
use a3s_launch::{LaunchEvent, LaunchTarget};
use a3s_types::constants::MinimizationType;
use a3s_types::local::TreeNode;
use a3s_viewer_context::{Command, ViewerContext, launch};

use crate::sheets::{PlayCheck, Sheet, Sheets};
use crate::shell::{Action, Screen};
use crate::toasts::Toaster;

/// What the Play button asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlayRequest {
    /// The profile to launch.
    pub(crate) profile: String,

    /// The profile's repositories have updates to download before the game starts.
    pub(crate) update_first: bool,

    /// The enabled mods were checked already (the user answered the check's sheet).
    pub(crate) checked: bool,
}

/// Starts the game of the active profile, after the updates when asked. Without a usable Arma 3
/// executable nothing starts and a toast offers to choose it in Settings > Game; with enabled
/// mods missing or enabled twice a sheet asks first.
pub(crate) fn request(
    viewer: &ViewerContext,
    play: &PlayRequest,
    toasts: &mut Toaster,
    sheets: &mut Sheets,
) {
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
        toasts.action(problem, "Choose folder…", Action::Navigate(Screen::Game));
        return;
    }
    if !play.checked
        && let Some(check) = check_enabled_mods(viewer, play.update_first)
    {
        sheets.push(Sheet::PlayCheck(check));
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

/// What is wrong with the enabled mods of the active profile, from the last addon scan; `None`
/// when they are ready or no scan is known yet (the launch checks again). With `update_first`
/// the missing mods of the repositories' groups come with the update and are no problem.
fn check_enabled_mods(viewer: &ViewerContext, update_first: bool) -> Option<PlayCheck> {
    let state = viewer.read();
    let scan = state.profiles.scan.clone()?;
    let mut tree = state.client.active_profile()?.tree.clone();
    let mut twice = check_duplicate_addons_selection(&tree, &mut GroupStatus::default());
    if !twice.is_empty() {
        twice.sort();
        twice.dedup();
        return Some(PlayCheck::EnabledTwice(twice));
    }
    if check_missing_selected_addons(&mut tree, &scan.registry).is_empty() {
        return None;
    }
    let (mut from_repositories, mut others) = (Vec::new(), Vec::new());
    for child in &tree.children {
        let in_repository = match child {
            TreeNode::Directory(group) => group
                .modset_repository_name
                .as_deref()
                .is_some_and(|repository| state.client.repository(repository).is_ok()),
            TreeNode::Leaf(_) => false,
        };
        let names = if in_repository {
            &mut from_repositories
        } else {
            &mut others
        };
        missing_names(child, names);
    }
    let names = if update_first {
        others
    } else {
        from_repositories.iter().chain(&others).cloned().collect()
    };
    (!names.is_empty()).then_some(PlayCheck::Missing {
        names,
        downloadable: !update_first && !from_repositories.is_empty(),
    })
}

/// The enabled addons at or below `node` that are marked missing.
fn missing_names(node: &TreeNode, names: &mut Vec<String>) {
    match node {
        TreeNode::Leaf(leaf) if leaf.selected && leaf.missing => names.push(leaf.name.clone()),
        TreeNode::Leaf(_) => {}
        TreeNode::Directory(directory) => {
            for child in &directory.children {
                missing_names(child, names);
            }
        }
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
