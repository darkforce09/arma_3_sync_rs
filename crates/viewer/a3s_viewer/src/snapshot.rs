//! What the toolbar and the sidebar show, copied out of the state once per frame so no state
//! guard is held while the frame runs actions.

use std::collections::BTreeSet;

use a3s_types::constants::RepositoryStatus;
use a3s_types::local::Repository;
use a3s_viewer_context::launch::enabled_modset_repositories;
use a3s_viewer_context::{AppState, RepositoryRuntime};

/// The sidebar badge of a repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Badge {
    /// Up to date or never checked.
    None,

    /// A check is running.
    Checking,

    /// The last check failed.
    Failed,

    /// The server has a newer revision; the number of addons changed since the local revision
    /// (0 when the changelogs do not tell).
    Updates(usize),
}

/// One repository row of the sidebar.
#[derive(Clone, Debug)]
pub(crate) struct RepositoryRow {
    pub(crate) name: String,
    pub(crate) badge: Badge,

    /// Why the last check failed, if it did.
    pub(crate) error: Option<String>,
}

/// The frame's copy of the state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    /// Every stored repository, in name order.
    pub(crate) repositories: Vec<RepositoryRow>,

    /// Repositories built from a local folder on this computer, in name order.
    pub(crate) publishing: Vec<String>,

    /// Profile names, in name order.
    pub(crate) profiles: Vec<String>,

    /// The active profile.
    pub(crate) active_profile: String,

    /// A repository with an enabled mod of the active profile has updates: Play updates it
    /// first.
    pub(crate) profile_has_updates: bool,

    /// A repository check is running.
    pub(crate) checking: bool,

    /// The game runs and starts again when it closes (the profile's auto-restart).
    pub(crate) restarting: bool,
}

impl Snapshot {
    pub(crate) fn take(state: &AppState) -> Self {
        let client = &state.client;
        let repositories: Vec<RepositoryRow> = client
            .repositories()
            .map(|repository| {
                let runtime = state.repositories.runtime(&repository.name);
                RepositoryRow {
                    name: repository.name.clone(),
                    badge: badge(repository, &runtime),
                    error: runtime.last_check_error.map(|error| error.message),
                }
            })
            .collect();
        let publishing = client
            .repositories()
            .filter(|repository| {
                repository
                    .path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty())
            })
            .map(|repository| repository.name.clone())
            .collect();
        let profile_repositories: BTreeSet<String> = client
            .active_profile()
            .map(enabled_modset_repositories)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let profile_has_updates = repositories.iter().any(|row| {
            matches!(row.badge, Badge::Updates(_)) && profile_repositories.contains(&row.name)
        });
        let restarting = state
            .launch
            .game
            .as_ref()
            .is_some_and(|game| !game.is_finished())
            && client
                .launcher_options()
                .is_some_and(|options| options.auto_restart);
        Self {
            checking: repositories.iter().any(|row| row.badge == Badge::Checking),
            restarting,
            repositories,
            publishing,
            profiles: client.profile_names(),
            active_profile: client.active_profile_name().to_owned(),
            profile_has_updates,
        }
    }
}

fn badge(repository: &Repository, runtime: &RepositoryRuntime) -> Badge {
    if runtime.checking {
        Badge::Checking
    } else if runtime.last_check_error.is_some() {
        Badge::Failed
    } else if runtime.status == RepositoryStatus::Updated {
        Badge::Updates(changed_addons(repository, runtime))
    } else {
        Badge::None
    }
}

/// Distinct addons new, updated or deleted in the changelogs above the local revision.
fn changed_addons(repository: &Repository, runtime: &RepositoryRuntime) -> usize {
    let Some(changelogs) = &runtime.metadata.changelogs else {
        return 0;
    };
    changelogs
        .list
        .iter()
        .filter(|changelog| changelog.revision > repository.revision)
        .flat_map(|changelog| {
            changelog
                .new_addons
                .iter()
                .chain(&changelog.updated_addons)
                .chain(&changelog.deleted_addons)
        })
        .collect::<BTreeSet<_>>()
        .len()
}
