//! Headless synchronization: check, select all, download, userconfig, check again.

use std::path::PathBuf;
use std::time::Duration;

use a3s_foundation::error::UserFacing as _;
use a3s_foundation::format;
use a3s_transport::CancellationToken;
use a3s_types::constants::RepositoryStatus;

use super::session::{CheckError, CheckRequest, RunEnd, RunReport, SyncSession, mark_failed};
use super::store::SyncStore;
use super::userconfig::{UserconfigAction, UserconfigOutcome, UserconfigPolicy, copy_userconfig};
use crate::events::{EventError, EventSender, Outcome, Phase, SyncEvent};

/// Pause before reconnecting in the command-line client.
// Mirrors console/CommandGeneral.java:324
pub const CLI_RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// How often a download is resumed after the connection was lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// Resumes allowed.
    pub max_attempts: u32,

    /// Pause before each resume.
    pub delay: Duration,
}

impl ReconnectPolicy {
    /// Never resume.
    pub const NONE: Self = Self {
        max_attempts: 0,
        delay: Duration::ZERO,
    };
}

/// A headless synchronization.
#[derive(Clone, Debug)]
pub struct SyncJob {
    /// What to check.
    pub check: CheckRequest,

    /// The Arma 3 executable, for the `userconfig` copy.
    pub arma3_exe_path: Option<PathBuf>,

    /// Whether to copy the `userconfig` folder.
    pub userconfig: UserconfigPolicy,

    /// Resumes after a lost connection.
    pub reconnect: ReconnectPolicy,
}

/// How a headless synchronization ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncEnd {
    /// Ended with `Finished(outcome)`.
    Finished(Outcome),

    /// The connection was lost and no resume was left.
    ConnectionLost,
}

/// What [`sync_repository`] did.
#[derive(Debug)]
pub struct SyncRunReport {
    /// How it ended.
    pub end: SyncEnd,

    /// The status persisted with the revision (`Ok` or `Error`); `None` when the revision was
    /// left alone (cancel) or persisting failed.
    pub status: Option<RepositoryStatus>,

    /// The last download run, if the check succeeded.
    pub download: Option<RunReport>,

    /// Resumes after a lost connection.
    pub reconnects: u32,

    /// The `userconfig` copy, if one was attempted or declined.
    pub userconfig: Option<UserconfigOutcome>,

    /// Files still to download after the second check (0 = in sync).
    pub remaining_downloads: Option<usize>,

    /// The download report of the session.
    pub report_text: Option<String>,
}

/// Synchronizes a repository without user interaction.
///
/// Checks, selects everything, downloads (resuming after a lost connection as `job.reconnect`
/// allows), copies the `userconfig` folder, checks again, and persists the revision (the
/// server's after a success, -1 after a failure; untouched after a cancel).
///
/// The last event is `Finished(..)`, or `ConnectionLost` when no resume was left.
// Mirrors ui/repository/workers/AddonsAutoUpdater.java:38-105,
// console/CommandGeneral.java:234-329, ui/repository/DownloadPanel.java:1181-1269
pub async fn sync_repository(
    job: SyncJob,
    store: &dyn SyncStore,
    cancel: &CancellationToken,
    events: &EventSender,
) -> SyncRunReport {
    let name = job.check.repository_name.clone();
    let mut result = SyncRunReport {
        end: SyncEnd::Finished(Outcome::Cancelled),
        status: None,
        download: None,
        reconnects: 0,
        userconfig: None,
        remaining_downloads: None,
        report_text: None,
    };

    // 1. Check.
    let mut session = match SyncSession::check(job.check.clone(), store, cancel, events).await {
        Ok(session) => session,
        Err(err) => {
            let outcome = check_failed(&err);
            if !matches!(err, CheckError::Cancelled) {
                result.status = persist_failed(store, &name);
            }
            events.emit(SyncEvent::Finished(outcome.clone()));
            result.end = SyncEnd::Finished(outcome);
            return result;
        }
    };

    // 2. Select everything.
    session.select_all();
    let summary = session.summary();
    // Mirrors console/CommandGeneral.java:413-416
    tracing::info!(
        "Number of files to update = {}",
        summary.download_tasks.len()
    );
    tracing::info!("Number of files to delete = {}", summary.delete_tasks.len());
    tracing::info!(
        "Update files size: {}",
        format::size(summary.total_download_size as i64)
    );

    // 3. Download, resuming after lost connections.
    let mut run = session.download(cancel, events).await;
    while run.end == RunEnd::ConnectionLost && result.reconnects < job.reconnect.max_attempts {
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(job.reconnect.delay) => {}
        }
        result.reconnects += 1;
        tracing::info!("Synchronization with repository: {name} - reconnecting");
        run = session.download(cancel, events).await;
    }
    result.report_text = session.last_report().map(str::to_owned);
    let end = run.end.clone();
    result.download = Some(run);

    let outcome = match end {
        RunEnd::ConnectionLost => {
            if cancel.is_cancelled() {
                // Cancelled while waiting to reconnect: the revision is left alone.
                events.emit(SyncEvent::Finished(Outcome::Cancelled));
                result.end = SyncEnd::Finished(Outcome::Cancelled);
            } else {
                result.status = persist_failed(store, &name);
                result.end = SyncEnd::ConnectionLost;
            }
            return result;
        }
        RunEnd::Finished(outcome) => outcome,
    };
    match outcome {
        Outcome::Success => {}
        Outcome::Cancelled => {
            result.end = SyncEnd::Finished(Outcome::Cancelled);
            return result;
        }
        other => {
            result.status = persist_failed(store, &name);
            result.end = SyncEnd::Finished(other);
            return result;
        }
    }

    // 4. Userconfig.
    result.userconfig = update_userconfig(&session, &job, events).await;

    // 5. Second check, then the revision.
    // Mirrors ui/repository/workers/AddonsAutoUpdater.java:81-105
    match SyncSession::check(job.check.clone(), store, cancel, events).await {
        Ok(second) => {
            result.remaining_downloads = Some(second.summary().download_tasks.len());
            result.status = match second.mark_synced(store) {
                Ok(status) => Some(status),
                Err(err) => {
                    tracing::warn!("Failed to save repository {name}: {err}");
                    None
                }
            };
            result.end = SyncEnd::Finished(Outcome::Success);
        }
        Err(CheckError::Cancelled) => {
            events.emit(SyncEvent::Finished(Outcome::Cancelled));
            result.end = SyncEnd::Finished(Outcome::Cancelled);
        }
        Err(err) => {
            let outcome = check_failed(&err);
            result.status = persist_failed(store, &name);
            events.emit(SyncEvent::Finished(outcome.clone()));
            result.end = SyncEnd::Finished(outcome);
        }
    }
    result
}

/// The `userconfig` step after a successful download.
// Mirrors ui/repository/DownloadPanel.java:1031-1048,1205-1212
async fn update_userconfig(
    session: &SyncSession,
    job: &SyncJob,
    events: &EventSender,
) -> Option<UserconfigOutcome> {
    let UserconfigAction::Copy(copy) = session.plan_userconfig_copy(job.arma3_exe_path.as_deref())
    else {
        return None;
    };
    match &job.userconfig {
        UserconfigPolicy::Skip => return None,
        UserconfigPolicy::Silent => {}
        UserconfigPolicy::Ask(confirm) => {
            if !confirm(&copy) {
                return Some(UserconfigOutcome::Declined);
            }
        }
    }
    events.emit(SyncEvent::Phase(Phase::UpdatingUserconfig));
    match tokio::task::spawn_blocking(move || copy_userconfig(&copy)).await {
        Ok(outcome) => Some(outcome),
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        Err(_) => None,
    }
}

/// The outcome of a failed check.
fn check_failed(err: &CheckError) -> Outcome {
    match err {
        CheckError::Cancelled => Outcome::Cancelled,
        other => Outcome::Failed {
            error: EventError::new(other.kind(), other.user_message()),
        },
    }
}

/// Persists the revision -1; logs and returns `None` if that fails.
fn persist_failed(store: &dyn SyncStore, name: &str) -> Option<RepositoryStatus> {
    match mark_failed(store, name) {
        Ok(status) => Some(status),
        Err(err) => {
            tracing::warn!("Failed to save repository {name}: {err}");
            None
        }
    }
}
