//! Synchronization: the user's downloads and the automatic updates after a periodic check.

mod auto_update;
mod download;

use std::fmt;
use std::sync::Arc;

use a3s_sync::EventSender;
use a3s_sync::pipeline::{SyncJob, SyncRunReport, SyncStore, sync_repository};
use a3s_sync::plan::{AddonLocator, NoAddons};
use a3s_transport::CancellationToken;
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::event::AppEvent;
use crate::tasks::Cx;

pub use self::download::{
    AddonChange, ChangeKind, DownloadRecord, DownloadResult, Downloads, PlanState, RECENT_LIMIT,
    RECONNECT_ATTEMPTS, RECONNECT_DELAY, Selection, SyncPlan, compare, download,
    download_selection, forget, pause, resume, retry, stop,
};

/// Synchronization commands.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Updates the repositories automatically, as after a periodic check.
    AutoUpdate {
        /// The repositories.
        repositories: Vec<String>,
    },

    /// Compares a repository with its local folder ([`compare`]).
    Compare {
        /// The repository.
        repository: String,
    },

    /// Downloads a repository, limited to the addons of `event` when set
    /// ([`download_selection`]).
    Download {
        /// The repository.
        repository: String,
        /// The event.
        event: Option<String>,
        /// Which part of it.
        selection: Selection,
    },

    /// Downloads like [`Command::Download`] even though Arma 3 is running.
    DownloadAnyway {
        /// The repository.
        repository: String,
        /// The event.
        event: Option<String>,
        /// Which part of it.
        selection: Selection,
    },

    /// Resumes a paused download with its event ([`resume`]).
    Resume {
        /// The repository.
        repository: String,
    },

    /// Downloads failed files again; every file still to download when `files` is empty
    /// ([`retry`]).
    Retry {
        /// The repository.
        repository: String,
        /// Paths below the repository root.
        files: Vec<String>,
    },

    /// Pauses a download; the next download resumes it ([`pause`]).
    Pause {
        /// The repository.
        repository: String,
    },

    /// Stops a download and discards its progress ([`stop`]).
    Stop {
        /// The repository.
        repository: String,
    },
}

/// Synchronization events.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// Repositories are due for an automatic update. With a [`Synchronizer`] configured, each
    /// one is synchronized by an [`crate::TaskKind::AutoUpdate`] task.
    AutoUpdateRequested {
        /// The repositories, in name order.
        repositories: Vec<String>,
    },

    /// A [`Command::Download`] did not start because Arma 3 is running: files the game has open
    /// may fail to update. [`Command::DownloadAnyway`] starts it.
    GameRunning {
        /// The repository.
        repository: String,
        /// The event.
        event: Option<String>,
        /// Which part of it.
        selection: Selection,
    },

    /// A download started by the user ended (it was not paused or stopped); the result is also
    /// first in [`Downloads::recent`].
    DownloadFinished {
        /// The repository.
        repository: String,
        /// How it ended.
        result: DownloadResult,
    },
}

/// Synchronization state.
pub struct State {
    /// Where existing addon folders are, for auto-discovery during synchronizations.
    pub addons: Arc<dyn AddonLocator + Send + Sync>,

    /// The user's comparisons, paused downloads and recent results.
    pub downloads: Downloads,
}

impl Default for State {
    fn default() -> Self {
        Self {
            addons: Arc::new(NoAddons),
            downloads: Downloads::default(),
        }
    }
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State").finish_non_exhaustive()
    }
}

/// Runs a headless synchronization: the seam of the background downloads.
pub trait Synchronizer: Send + Sync + 'static {
    /// Synchronizes `job` (see [`a3s_sync::pipeline::sync_repository`]).
    fn synchronize(
        &self,
        job: SyncJob,
        store: Arc<dyn SyncStore>,
        cancel: CancellationToken,
        events: EventSender,
    ) -> BoxFuture<'static, SyncRunReport>;
}

/// The [`Synchronizer`] that downloads from the repository.
#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkSynchronizer;

impl Synchronizer for NetworkSynchronizer {
    fn synchronize(
        &self,
        job: SyncJob,
        store: Arc<dyn SyncStore>,
        cancel: CancellationToken,
        events: EventSender,
    ) -> BoxFuture<'static, SyncRunReport> {
        async move { sync_repository(job, store, &cancel, &events).await }.boxed()
    }
}

/// Repositories are due for an automatic update.
///
/// Emits [`Event::AutoUpdateRequested`] and, when the context has a [`Synchronizer`], starts
/// one automatic update per repository. A repository that is already being synchronized is
/// skipped.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:119-134
pub fn auto_update_due(cx: &Cx, repositories: &[String]) {
    cx.emit(AppEvent::Sync(Event::AutoUpdateRequested {
        repositories: repositories.to_vec(),
    }));
    let Some(synchronizer) = &cx.shared.synchronizer else {
        return;
    };
    // Mirrors ui/repository/DownloadPanel.java:1165-1173
    if cx.state().read().tools.platform.is_arma3_running() {
        tracing::info!("Arma 3 is running; the automatic updates wait for the next check.");
        return;
    }
    for name in repositories {
        if let Err(err) = auto_update::start(cx, name, Arc::clone(synchronizer)) {
            tracing::info!("Auto update of repository {name} not started: {err}");
        }
    }
}

pub(crate) fn handle(cx: &Cx, command: Command) {
    match command {
        Command::AutoUpdate { repositories } => auto_update_due(cx, &repositories),
        other => download::handle(cx, other),
    }
}
