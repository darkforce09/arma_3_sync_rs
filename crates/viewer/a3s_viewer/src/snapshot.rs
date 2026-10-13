//! What the toolbar and the sidebar show, copied out of the state once per frame so no state
//! guard is held while the frame runs actions.

use std::collections::BTreeSet;

use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Repository, TreeDirectory};
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

    /// A repository of the active profile's addon groups has updates.
    pub(crate) profile_has_updates: bool,

    /// A repository check is running.
    pub(crate) checking: bool,
}

impl Snapshot {
    pub(crate) fn take(state: &AppState) -> Self {
        let client = &state.client;
        let repositories: Vec<RepositoryRow> = client
            .repositories()
            .map(|repository| RepositoryRow {
                name: repository.name.clone(),
                badge: badge(repository, &state.repositories.runtime(&repository.name)),
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
        let profile_repositories = client
            .active_profile()
            .map(|profile| group_repositories(&profile.tree))
            .unwrap_or_default();
        let profile_has_updates = repositories.iter().any(|row| {
            matches!(row.badge, Badge::Updates(_)) && profile_repositories.contains(&row.name)
        });
        Self {
            checking: repositories.iter().any(|row| row.badge == Badge::Checking),
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

/// The repositories the addon groups of a profile tree were made from.
fn group_repositories(tree: &TreeDirectory) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut pending = vec![tree];
    while let Some(directory) = pending.pop() {
        if let Some(name) = &directory.modset_repository_name {
            names.insert(name.clone());
        }
        pending.extend(directory.directories());
    }
    names
}
