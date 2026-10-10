//! Repository synchronization status, revision handling and update notifications.
//!
//! Pure functions over the client [`Repository`] / [`Configuration`] and the
//! [`RemoteMetadata`] of the last check. Nothing is persisted here: callers write the repository
//! or configuration when a function reports a change.

use std::collections::{BTreeMap, HashMap};

use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Configuration, Repository};
use a3s_types::{Changelogs, FavoriteServer, ServerInfo};

use crate::remote::RemoteMetadata;

/// The status of a repository at `local_revision` against the server state.
///
/// Equal revisions are `Ok`. A newer server revision is `Updated` if the server says the
/// content changed, or if a changelog after `local_revision` changed content; it is
/// `Indeterminated` without changelogs or when `local_revision` is not in them. Anything else
/// (no server info, local revision ahead) is `Indeterminated`.
// Mirrors service/RepositoryService.java:1475-1521 (identical to :347-386)
pub fn determine_sync_status(
    local_revision: i32,
    server_info: Option<&ServerInfo>,
    changelogs: Option<&Changelogs>,
) -> RepositoryStatus {
    let Some(server_info) = server_info else {
        return RepositoryStatus::Indeterminated;
    };
    if local_revision == server_info.revision {
        return RepositoryStatus::Ok;
    }
    if local_revision > server_info.revision {
        return RepositoryStatus::Indeterminated;
    }
    if server_info.repository_content_updated {
        return RepositoryStatus::Updated;
    }
    let Some(changelogs) = changelogs else {
        return RepositoryStatus::Indeterminated;
    };
    // `TreeMap.put`: a later duplicate revision overwrites an earlier one.
    let map: BTreeMap<i32, bool> = changelogs
        .list
        .iter()
        .map(|c| (c.revision, c.content_updated))
        .collect();
    if !map.contains_key(&local_revision) {
        return RepositoryStatus::Indeterminated;
    }
    let mut change = false;
    for (_, &content_updated) in map.range(local_revision.saturating_add(1)..) {
        change = content_updated;
        if change {
            break;
        }
    }
    if change {
        RepositoryStatus::Updated
    } else {
        RepositoryStatus::Ok
    }
}

/// Result of [`update_repository`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryUpdate {
    /// The new synchronization status.
    pub status: RepositoryStatus,

    /// The repository was modified (hidden folders, client connections): persist it.
    pub repository_changed: bool,

    /// The configuration was modified (favorite servers): persist it.
    pub configuration_changed: bool,
}

/// Applies the metadata of a check: merges the server's hidden folders and connection count
/// into the repository, the autoconfig favorite servers into the configuration, and computes
/// the status.
// Mirrors service/RepositoryService.java:256-388
pub fn update_repository(
    repository: &mut Repository,
    configuration: &mut Configuration,
    metadata: &RemoteMetadata,
) -> RepositoryUpdate {
    let mut repository_changed = false;
    if let Some(server_info) = &metadata.server_info {
        // A union: entries are never removed here.
        for path in &server_info.hidden_folder_paths {
            repository_changed |= repository.hidden_folder_paths.insert(path.clone());
        }
        if repository.number_of_client_connections == 0 && server_info.number_of_connections != 0 {
            repository.number_of_client_connections = server_info.number_of_connections;
            repository_changed = true;
        }
    }

    let mut configuration_changed = false;
    if let Some(auto_config) = &metadata.auto_config {
        let before = configuration.favorite_servers.clone();
        // Java's getter sorts the list on every read, so the merge matches against the sorted
        // list (the first match in sorted order wins).
        // Mirrors domain/configration/Configuration.java:59-65
        configuration.sort_favorite_servers();
        merge_favorite_servers(
            &mut configuration.favorite_servers,
            &auto_config.favorite_servers,
            &repository.name,
        );
        configuration.sort_favorite_servers();
        configuration_changed = configuration.favorite_servers != before;
    }

    let status = determine_sync_status(
        repository.revision,
        metadata.server_info.as_deref(),
        metadata.changelogs.as_deref(),
    );
    RepositoryUpdate {
        status,
        repository_changed,
        configuration_changed,
    }
}

/// Merges the favorite servers of a repository's autoconfig into the local list.
///
/// An empty local list receives a copy of every remote server. Otherwise a remote server
/// matching a local one (same IP and port, looked up in the list as it was before the merge)
/// updates its password, modset (if the remote one is set) and repository name; an unmatched
/// one is appended. Returns whether the list changed. Sorting is left to the caller
/// ([`Configuration::sort_favorite_servers`]).
// Mirrors service/RepositoryService.java:270-345
pub fn merge_favorite_servers(
    local: &mut Vec<FavoriteServer>,
    remote: &[FavoriteServer],
    repository_name: &str,
) -> bool {
    let before = local.clone();
    if local.is_empty() {
        local.extend(remote.iter().map(|r| new_server(r, repository_name)));
    } else {
        // Servers appended during this merge are not matched (Java iterates a snapshot), so
        // duplicate remote entries are appended twice.
        let snapshot_len = local.len();
        for remote_server in remote {
            // Java throws on a null remote IP; comparing options is the approved deviation.
            let found = local[..snapshot_len]
                .iter_mut()
                .find(|l| l.ip_address == remote_server.ip_address && l.port == remote_server.port);
            match found {
                Some(local_server) => {
                    local_server.password.clone_from(&remote_server.password);
                    if let Some(modset) = &remote_server.modset_name
                        && !modset.is_empty()
                    {
                        local_server.modset_name = Some(modset.clone());
                    }
                    local_server.repository_name = Some(repository_name.to_owned());
                    if local_server.name.as_deref().is_none_or(str::is_empty) {
                        local_server.name = Some(repository_name.to_owned());
                    }
                }
                None => local.push(new_server(remote_server, repository_name)),
            }
        }
    }
    *local != before
}

/// A copy of a remote favorite server for the local list.
// Mirrors service/RepositoryService.java:279-296
fn new_server(remote: &FavoriteServer, repository_name: &str) -> FavoriteServer {
    let name = match remote.name.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => repository_name.to_owned(),
    };
    FavoriteServer {
        name: Some(name),
        ip_address: remote.ip_address.clone(),
        port: remote.port,
        password: remote.password.clone(),
        selected: false,
        modset_name: remote.modset_name.clone(),
        repository_name: Some(repository_name.to_owned()),
    }
}

/// After a successful synchronization (or check): the repository takes the server revision.
/// Returns `Ok`, even without server info.
// Mirrors ui/repository/DownloadPanel.java:970-976, service/RepositoryService.java:754-768
pub fn mark_synced(repository: &mut Repository, metadata: &RemoteMetadata) -> RepositoryStatus {
    if let Some(server_info) = &metadata.server_info {
        repository.revision = server_info.revision;
    }
    RepositoryStatus::Ok
}

/// After a failed synchronization: the revision is reset to -1. Returns `Error`.
// Mirrors ui/repository/DownloadPanel.java:978-984, service/RepositoryService.java:746-752
pub fn mark_failed(repository: &mut Repository) -> RepositoryStatus {
    repository.revision = -1;
    RepositoryStatus::Error
}

/// Remembers the server revision each repository was last notified for (in memory only).
#[derive(Clone, Debug, Default)]
pub struct NotificationTracker {
    notified: HashMap<String, i32>,
}

impl NotificationTracker {
    /// A tracker that has notified nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to notify the user that `name` was updated to `server_revision`.
    ///
    /// Never for automatically updated or non-notifying repositories; always when the revision
    /// is unknown; otherwise once per revision.
    // Mirrors ui/main/tasks/TaskCheckRepositories.java:173-190
    pub fn should_notify(
        &mut self,
        name: &str,
        notify: bool,
        auto: bool,
        server_revision: Option<i32>,
    ) -> bool {
        if !notify || auto {
            return false;
        }
        let Some(server_revision) = server_revision else {
            return true;
        };
        if self.notified.get(name) == Some(&server_revision) {
            return false;
        }
        self.notified.insert(name.to_owned(), server_revision);
        true
    }

    /// Forgets `name`, so its next update notifies again.
    pub fn forget(&mut self, name: &str) {
        self.notified.remove(name);
    }
}

/// A repository after a periodic check, as seen by [`decide_check_cycle`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedRepository {
    /// Repository name.
    pub name: String,

    /// Notify the user about updates.
    pub notify: bool,

    /// Synchronize automatically.
    pub auto: bool,

    /// Status after [`update_repository`].
    pub status: RepositoryStatus,

    /// The server revision, if known.
    pub server_revision: Option<i32>,
}

/// What a periodic check cycle should do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckCycleDecision {
    /// Repositories with status `Updated`, in configuration order.
    pub updated: Vec<String>,

    /// Updated repositories to synchronize automatically.
    pub auto_update: Vec<String>,

    /// Updated repositories to notify the user about.
    pub notify: Vec<String>,
}

impl CheckCycleDecision {
    /// The console message listing the updated repositories, or `None` if there are none.
    // Mirrors ui/main/tasks/TaskCheckRepositories.java:100-107
    pub fn console_message(&self) -> Option<String> {
        if self.updated.is_empty() {
            return None;
        }
        let mut message = String::from("The following repositories have been updated: ");
        for name in &self.updated {
            message.push('\n');
            message.push_str(name);
        }
        Some(message)
    }
}

/// Decides the follow-up of a periodic check from the repositories in configuration order.
///
/// Repositories that are not `Updated` are forgotten by the tracker, so a later update notifies
/// again.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:85-158
pub fn decide_check_cycle(
    repositories: &[CheckedRepository],
    tracker: &mut NotificationTracker,
) -> CheckCycleDecision {
    let mut decision = CheckCycleDecision::default();
    let mut updated = Vec::new();
    for repository in repositories {
        if repository.status == RepositoryStatus::Updated {
            updated.push(repository);
            decision.updated.push(repository.name.clone());
        } else {
            tracker.forget(&repository.name);
        }
    }
    decision.auto_update = updated
        .iter()
        .filter(|r| r.auto)
        .map(|r| r.name.clone())
        .collect();
    for repository in updated {
        if tracker.should_notify(
            &repository.name,
            repository.notify,
            repository.auto,
            repository.server_revision,
        ) {
            decision.notify.push(repository.name.clone());
        }
    }
    decision
}
