use std::time::SystemTime;

use a3s_repo_files::services::ClientData;
use a3s_sync::{EventError, RemoteMetadata};
use a3s_types::constants::RepositoryStatus;

use crate::{admin, launch, profiles, repositories, sync, tools};

/// Everything the UI shows: the client data and the runtime state of each domain.
#[derive(Debug)]
pub struct AppState {
    /// Configuration, preferences, profiles and repositories, persisted on every change.
    pub client: ClientData,

    /// Start-up flags.
    pub flags: AppFlags,

    /// Runtime state of the repositories and the update notifications.
    pub repositories: repositories::State,

    /// Synchronization state.
    pub sync: sync::State,

    /// Repository administration state.
    pub admin: admin::State,

    /// Profiles and addons state.
    pub profiles: profiles::State,

    /// Game launch state.
    pub launch: launch::State,

    /// Game tools state.
    pub tools: tools::State,
}

impl AppState {
    pub(crate) fn new(client: ClientData, flags: AppFlags) -> Self {
        Self {
            client,
            flags,
            repositories: repositories::State::default(),
            sync: sync::State::default(),
            admin: admin::State::default(),
            profiles: profiles::State::default(),
            launch: launch::State::default(),
            tools: tools::State::default(),
        }
    }
}

/// Flags set at start-up.
// Mirrors ui/Facade.java:16-46
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppFlags {
    /// Development mode.
    pub dev_mode: bool,

    /// Started with the operating system; the preferences may change it later.
    pub run_mode: bool,
}

/// What the client knows about a repository beyond its stored settings, for this session
/// only. The UI's repository badges are drawn from it.
// Mirrors domain/repository/Repository.java:36, 46-50, 61, 73, 416-425
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRuntime {
    /// The synchronization status of the last check or synchronization.
    pub status: RepositoryStatus,

    /// The remote metadata of the last check.
    pub metadata: RemoteMetadata,

    /// Why the last check failed, if it did.
    pub last_check_error: Option<EventError>,

    /// A check of this repository is running.
    pub checking: bool,

    /// When the last check of this repository ended (succeeded or failed).
    pub last_checked: Option<SystemTime>,

    /// The report of the last download.
    pub download_report: Option<String>,

    /// Index of the last file transferred by an interrupted upload.
    pub last_index_file_transferred: i32,
}

impl Default for RepositoryRuntime {
    // Mirrors service/RepositoryService.java:1457-1465 (a missing repository is INDETERMINATED)
    fn default() -> Self {
        Self {
            status: RepositoryStatus::Indeterminated,
            metadata: RemoteMetadata::default(),
            last_check_error: None,
            checking: false,
            last_checked: None,
            download_report: None,
            last_index_file_transferred: 0,
        }
    }
}
