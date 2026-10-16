//! The Play task: optional updates, then the companion apps and the game.

use std::collections::BTreeSet;
use std::sync::Arc;

use a3s_addons::check_missing_selected_addons;
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_launch::{
    Arma3LaunchRequest, EventSink, LaunchError, LaunchEvent, LaunchTarget, RestartGuard,
    SteamWatch, plan_launch,
};
use a3s_sync::EventError;
use a3s_types::SyncTreeDirectory;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Profile, TreeDirectory, TreeNode};
use futures::future::join_all;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use super::Event;
use crate::error::ContextError;
use crate::event::AppEvent;
use crate::profiles::AddonScan;
use crate::state::AppState;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskKey, TaskKind};

/// Starts the Play task; fails with [`ContextError::TaskBusy`] while one runs.
pub(super) fn start(cx: &Cx, update_first: bool) -> Result<(), ContextError> {
    cx.spawn_task(TaskKey::new(TaskKind::Launch), true, move |task| {
        run(task, update_first)
    })?;
    Ok(())
}

/// The repositories behind the enabled addon groups of `profile` (a top-level modset group with
/// a selected addon), in name order.
pub fn enabled_modset_repositories(profile: &Profile) -> Vec<String> {
    let mut names: Vec<String> = profile
        .tree
        .directories()
        .filter(|group| has_selected_leaf(group))
        .filter_map(|group| group.modset_repository_name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The addons the game loads, in load order: the selected addons in priority order, then the
/// others (lowercase registry keys).
pub fn launch_order(profile: &Profile) -> Vec<String> {
    a3s_launch::launch_addon_names(profile)
}

fn has_selected_leaf(directory: &TreeDirectory) -> bool {
    directory.children.iter().any(|node| match node {
        TreeNode::Leaf(leaf) => leaf.selected,
        TreeNode::Directory(child) => has_selected_leaf(child),
    })
}

async fn run(task: TaskCtx, update_first: bool) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    use_joined_server_preset(&cx);
    if update_first {
        update(&task).await;
        if task.cancel.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
    }

    let (profile, configuration, scan, launcher) = {
        let state = cx.state().read();
        let profile = state
            .client
            .active_profile()
            .cloned()
            .ok_or_else(|| launch_failure(&LaunchError::ExecutableLocation))?;
        (
            profile,
            state.client.configuration().clone(),
            state.profiles.scan.clone(),
            Arc::clone(&state.launch.launcher),
        )
    };
    let scanned = profile.clone();
    let scan = tokio::task::spawn_blocking(move || crate::profiles::reuse_or_scan(scan, &scanned))
        .await
        .map_err(|err| internal(format!("The addon scan stopped: {err}")))?;
    report_missing(&cx, &profile, &scan);

    let environment = {
        let launcher = Arc::clone(&launcher);
        tokio::task::spawn_blocking(move || launcher.environment())
            .await
            .map_err(|err| internal(format!("The launch environment is unknown: {err}")))?
    };
    let request = Arma3LaunchRequest {
        os: environment.os,
        profile: Some(&profile),
        configuration: &configuration,
        registry: &scan.registry,
        steam: environment.steam.clone(),
        arma3_documents_dir: environment.arma3_documents_dir.clone(),
        restart_guard: RestartGuard::default(),
        steam_watch: SteamWatch::default(),
    };
    let planned = plan_launch(&request)
        .map_err(|err| launch_failure(&err))?
        .ok_or_else(|| launch_failure(&LaunchError::ExecutableLocation))?;
    if let Some(dir) = &environment.arma3_documents_dir
        && let Err(err) = a3s_launch::cfg::erase_arma3_cfg_mod_launcher_list(dir)
    {
        tracing::warn!(
            "cannot remove the mod list from {}: {err}",
            dir.join(a3s_launch::cfg::ARMA3_CFG).display()
        );
    }
    tracing::info!("Launching: {}", planned.plan.shown_command.join(" "));

    let (started_sender, started) = oneshot::channel();
    let events = game_events(&cx, started_sender);
    launcher.start_companions(
        environment.os,
        &configuration.external_applications,
        &events,
    );
    let handle = launcher
        .start_game(planned.plan, planned.options, events)
        .map_err(|err| launch_failure(&err))?;
    let Some(handle) = handle else {
        return Ok(());
    };
    cx.state().write().launch.game = Some(handle);
    tokio::select! {
        outcome = started => match outcome {
            Ok(Ok(())) | Err(_) => Ok(()),
            Ok(Err(error)) => Err(TaskError::Message(error)),
        },
        () = task.cancel.cancelled() => Err(TaskError::Cancelled),
    }
}

/// Forwards launch events to the UI; the first start or failure of the game resolves
/// `started`.
fn game_events(cx: &Cx, started: oneshot::Sender<Result<(), EventError>>) -> EventSink {
    let cx = cx.clone();
    let started = Mutex::new(Some(started));
    Arc::new(move |event: LaunchEvent| {
        let outcome = match &event {
            LaunchEvent::Started {
                target: LaunchTarget::Arma3,
                ..
            } => Some(Ok(())),
            LaunchEvent::Error {
                target: LaunchTarget::Arma3,
                error,
            } => Some(Err(launch_event_error(error))),
            _ => None,
        };
        if let Some(outcome) = outcome
            && let Some(sender) = started.lock().take()
        {
            sender.send(outcome).ok();
        }
        cx.emit(AppEvent::Launch(Event::Game(event)));
    })
}

/// Enables exactly the preset group of the server the launch joins, when it has one, so the
/// game starts with the mods the server expects.
// Mirrors ui/main/LaunchPanel.java:221-257,270
fn use_joined_server_preset(cx: &Cx) {
    let mut state = cx.state().write();
    let configuration = state.client.configuration();
    let Some(group) = configuration
        .server_name
        .as_ref()
        .and(configuration.default_modset.clone())
    else {
        return;
    };
    match crate::profiles::use_group_preset(&mut state, &group) {
        Ok(true) => {
            drop(state);
            cx.emit(AppEvent::Changed(crate::event::Change::AddonSelection));
        }
        Ok(false) => {}
        Err(err) => tracing::warn!(
            "Cannot enable the server preset {group}: {}",
            err.user_message()
        ),
    }
}

/// Emits [`Event::MissingAddons`] for selected addons that are not on this computer.
fn report_missing(cx: &Cx, profile: &Profile, scan: &AddonScan) {
    let mut tree = profile.tree.clone();
    let names = check_missing_selected_addons(&mut tree, &scan.registry);
    if !names.is_empty() {
        cx.emit(AppEvent::Launch(Event::MissingAddons { names }));
    }
}

/// Downloads the enabled mods of the repositories of the enabled addon groups that have updates,
/// and waits for them. Each is the user's [`TaskKind::Download`]
/// ([`crate::sync::download_selection`]): its result lands in
/// [`crate::sync::Downloads::recent`] and [`crate::sync::Event::DownloadFinished`], where the UI
/// shows failed files. A download already running is waited for.
async fn update(task: &TaskCtx) {
    let cx = &task.cx;
    let wanted: Vec<(String, Vec<String>)> = {
        let state = cx.state().read();
        outdated_repositories(&state)
            .into_iter()
            .map(|name| {
                let paths = enabled_addon_paths(&state, &name);
                (name, paths)
            })
            .filter(|(_, paths)| !paths.is_empty())
            .collect()
    };
    let downloads = wanted.into_iter().map(|(name, paths)| async move {
        let selection = crate::sync::Selection::Only(paths);
        match crate::sync::download_selection(cx, &name, None, selection) {
            Ok(id) | Err(ContextError::TaskBusy { id, .. }) => cx.wait_task(id).await,
            Err(err) => {
                tracing::warn!("Cannot update {name} before playing: {err}");
                cx.emit(AppEvent::Error {
                    context: format!("Updating {name} before playing"),
                    error: EventError::new(err.kind(), err.user_message()),
                });
            }
        }
    });
    join_all(downloads).await;
}

/// Where the addons enabled in the groups made from `repository` are in its content: the
/// paths below its root of the addon folders with their names (any case).
fn enabled_addon_paths(state: &AppState, repository: &str) -> Vec<String> {
    let Some(profile) = state.client.active_profile() else {
        return Vec::new();
    };
    let mut names = BTreeSet::new();
    for group in profile
        .tree
        .directories()
        .filter(|group| group.modset_repository_name.as_deref() == Some(repository))
    {
        selected_leaf_names(group, &mut names);
    }
    let Some(sync) = state.repositories.runtime(repository).metadata.sync else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    addon_paths(&sync, "", &names, &mut paths);
    paths
}

fn selected_leaf_names(directory: &TreeDirectory, names: &mut BTreeSet<String>) {
    for child in &directory.children {
        match child {
            TreeNode::Leaf(leaf) if leaf.selected => {
                names.insert(leaf.name.to_lowercase());
            }
            TreeNode::Leaf(_) => {}
            TreeNode::Directory(child) => selected_leaf_names(child, names),
        }
    }
}

fn addon_paths(
    directory: &SyncTreeDirectory,
    prefix: &str,
    names: &BTreeSet<String>,
    paths: &mut Vec<String>,
) {
    for child in directory.directories() {
        let path = if prefix.is_empty() {
            child.name.clone()
        } else {
            format!("{prefix}/{}", child.name)
        };
        if child.mark_as_addon {
            if names.contains(&child.name.to_lowercase()) {
                paths.push(path);
            }
        } else {
            addon_paths(child, &path, names, paths);
        }
    }
}

/// The repositories of the enabled addon groups whose last check found updates.
fn outdated_repositories(state: &AppState) -> Vec<String> {
    let Some(profile) = state.client.active_profile() else {
        return Vec::new();
    };
    enabled_modset_repositories(profile)
        .into_iter()
        .filter(|name| {
            state.repositories.runtime(name).status == RepositoryStatus::Updated
                && state.client.repository(name).is_ok()
        })
        .collect()
}

fn launch_event_error(error: &LaunchError) -> EventError {
    EventError::new(ErrorKind::Launch, error.to_string())
}

fn launch_failure(error: &LaunchError) -> TaskError {
    TaskError::Message(launch_event_error(error))
}

fn internal(message: String) -> TaskError {
    TaskError::Message(EventError::new(ErrorKind::Internal, message))
}
