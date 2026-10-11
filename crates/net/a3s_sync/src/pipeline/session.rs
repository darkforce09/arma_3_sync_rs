//! One synchronization session: check, selection, then download runs.

// `CheckError` keeps the remote and completion errors inline; errors end a check, so their size is
// irrelevant.
#![allow(clippy::result_large_err)]

use std::path::Path;
use std::sync::Arc;

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_transport::proxy::{ProxyError, transport_options};
use a3s_transport::{CancellationToken, TransferCtx, TransportOptions};
use a3s_types::Protocol;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::protocol::host_url;

use super::store::{StoreChanges, StoreError, SyncStore};
use super::userconfig::{UserconfigAction, plan_userconfig_copy};
use crate::cleanup::{CleanupReport, DeleteRoots, delete_extra_files, protected_paths};
use crate::completion::{CompletionError, determine_completion};
use crate::download::{DownloadReport, DownloadRequest, DownloadedZip, ZIP_EXTENSION, ZipSink};
use crate::events::{EventError, EventSender, Outcome, SyncEvent};
use crate::model::{DownloadStatus, DownloadTaskKind, PlanTree};
use crate::options::{SyncOptions, SyncOptionsError};
use crate::plan::{
    AddonLocator, PlanError, PlanSummary, downloaded_compressed, plan_repository,
    select_all_except_root,
};
use crate::remote::{RemoteMetadata, check_repository};
use crate::report::{ReportInput, report_for};
use crate::unzip::{UnzipPool, UnzipSummary};
use crate::{RemoteError, download, status};

/// What to check.
#[derive(Clone)]
pub struct CheckRequest {
    /// The repository name in the store.
    pub repository_name: String,

    /// The event (modset) to synchronize, if any.
    pub event_name: Option<String>,

    /// The repository's protocol. Connections use the proxy of the stored configuration.
    pub protocol: Protocol,

    /// Where existing addon folders are (auto-discovery).
    pub addons: Arc<dyn AddonLocator + Send + Sync>,
}

impl std::fmt::Debug for CheckRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckRequest")
            .field("repository_name", &self.repository_name)
            .field("event_name", &self.event_name)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}

/// A check failed; nothing of the session survives.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckError {
    /// Fetching the metadata failed, or a required file is missing.
    #[error(transparent)]
    Remote(RemoteError),

    /// The destination folder is unset or missing.
    #[error(transparent)]
    Options(SyncOptionsError),

    /// The configured proxy cannot be used.
    #[error(transparent)]
    Proxy(ProxyError),

    /// Planning failed.
    #[error(transparent)]
    Plan(PlanError),

    /// Determining the completion of outdated files failed.
    #[error(transparent)]
    Completion(CompletionError),

    /// Persisting failed.
    #[error(transparent)]
    Store(StoreError),

    /// Cancelled.
    #[error("Cancelled")]
    Cancelled,
}

impl UserFacing for CheckError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Remote(e) => e.kind(),
            Self::Options(e) => e.kind(),
            Self::Proxy(_) => ErrorKind::InvalidInput,
            Self::Plan(e) => e.kind(),
            Self::Completion(e) => e.kind(),
            Self::Store(e) => e.kind(),
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    /// The inner message, which already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// How a download run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEnd {
    /// Ended with `Finished(outcome)`, the run's last event.
    Finished(Outcome),

    /// The connection was lost (`ConnectionLost` was the last event); the session can resume.
    ConnectionLost,
}

/// What a download run did.
#[derive(Debug)]
pub struct RunReport {
    /// How the run ended.
    pub end: RunEnd,

    /// The download report stored by this run (none after a cancel or a lost connection).
    pub report_text: Option<String>,

    /// The download executor's report; `None` when the destination check failed first.
    pub download: Option<DownloadReport>,

    /// What uncompressing did.
    pub unzip: UnzipSummary,

    /// What deleting extra files did.
    pub cleanup: CleanupReport,
}

/// A checked repository: the plan, its selection, and what the download runs accumulated.
///
/// A paused (cancelled) or interrupted run keeps the task statuses, so a later
/// [`SyncSession::download`] resumes: finished files are skipped, partial ones continue.
pub struct SyncSession {
    request: CheckRequest,
    transport_options: TransportOptions,
    options: SyncOptions,
    metadata: RemoteMetadata,
    tree: PlanTree,
    summary: PlanSummary,
    average_speed: u64,
    max_active_connections: u32,
    last_report: Option<String>,
}

impl SyncSession {
    /// Checks the repository: fetches its metadata, updates and persists the stored repository
    /// (hidden folders, connections, favorite servers, download location), plans the
    /// synchronization (persisting the SHA-1 cache) and determines the zsync completion.
    ///
    /// Nothing is selected yet. The status and revision are not changed (see
    /// [`Self::mark_synced`]). Emits no `Finished`.
    // Mirrors service/synchronization/FilesCheckProcessor.java:33-75,
    // ui/repository/workers/AddonsChecker.java:93-108
    pub async fn check(
        request: CheckRequest,
        store: Arc<dyn SyncStore>,
        cancel: &CancellationToken,
        events: &EventSender,
    ) -> Result<Self, CheckError> {
        let ctx = TransferCtx {
            cancel: cancel.clone(),
            ..TransferCtx::default()
        };
        let name = request.repository_name.clone();
        let event_name = request.event_name.clone();

        // 0. Connection options with the configured proxy.
        let proxy = {
            let store = Arc::clone(&store);
            let name = name.clone();
            blocking(move || {
                let mut proxy = None;
                store
                    .update(&name, &mut |_, configuration| {
                        proxy = Some(configuration.proxy.clone());
                        StoreChanges::default()
                    })
                    .map(|()| proxy.unwrap_or_default())
            })
            .await
            .ok_or(CheckError::Cancelled)?
            .map_err(CheckError::Store)?
        };
        let transport_options =
            transport_options(&request.protocol, &proxy).map_err(CheckError::Proxy)?;

        // 1. Metadata.
        let mut metadata = RemoteMetadata::default();
        check_repository(
            &name,
            &request.protocol,
            &transport_options,
            &mut metadata,
            &ctx,
            events,
        )
        .await
        .map_err(remote_error)?;
        // 2. Required files.
        metadata
            .require_for_sync(event_name.as_deref())
            .map_err(remote_error)?;
        ensure_not_cancelled(cancel)?;

        // 3. Repository update and options.
        let options = {
            let store = Arc::clone(&store);
            let stored_name = name.clone();
            let metadata = metadata.clone();
            blocking(move || {
                let mut options = None;
                store
                    .update(&stored_name, &mut |repository, configuration| {
                        let update =
                            status::update_repository(repository, configuration, &metadata);
                        let before = repository.clone();
                        options = Some(SyncOptions::from_repository(
                            repository,
                            event_name.as_deref(),
                        ));
                        StoreChanges {
                            repository: update.repository_changed || *repository != before,
                            configuration: update.configuration_changed,
                        }
                    })
                    .map(|()| options)
            })
            .await
            .ok_or(CheckError::Cancelled)?
            .map_err(CheckError::Store)?
            .ok_or_else(|| CheckError::Store(StoreError::repository_not_found(&name)))?
            .map_err(CheckError::Options)?
        };
        ensure_not_cancelled(cancel)?;

        // 4. Plan, with the SHA-1 cache, which is persisted afterwards.
        let planned = {
            let store = Arc::clone(&store);
            let name = name.clone();
            let metadata = metadata.clone();
            let options = options.clone();
            let addons = Arc::clone(&request.addons);
            let cancel = cancel.clone();
            let events = events.clone();
            blocking(move || {
                let mut cache = store.read_sync_cache(&name);
                let tree = plan_repository(
                    &metadata,
                    &options,
                    addons.as_ref(),
                    &mut cache,
                    &cancel,
                    &events,
                )
                .map_err(CheckError::Plan)?;
                store
                    .write_sync_cache(&name, &cache)
                    .map_err(CheckError::Store)?;
                Ok::<_, CheckError>(tree)
            })
            .await
            .ok_or(CheckError::Cancelled)?
        };
        let mut tree = match planned {
            Ok(tree) => tree,
            Err(CheckError::Plan(PlanError::Cancelled)) => return Err(CheckError::Cancelled),
            Err(err) => return Err(err),
        };
        ensure_not_cancelled(cancel)?;

        // 5. Completion.
        let server_info = metadata.server_info.clone().ok_or(CheckError::Remote(
            RemoteError::MissingMetadata(a3s_repo_files::A3sFile::ServerInfo),
        ))?;
        match determine_completion(
            &mut tree,
            &request.protocol,
            &transport_options,
            &server_info,
            &ctx,
            events,
        )
        .await
        {
            Ok(_) => {}
            Err(CompletionError::Cancelled) => return Err(CheckError::Cancelled),
            Err(err) => return Err(CheckError::Completion(err)),
        }
        ensure_not_cancelled(cancel)?;

        let summary = PlanSummary::compute(&tree);
        Ok(Self {
            request,
            transport_options,
            options,
            metadata,
            tree,
            summary,
            average_speed: 0,
            max_active_connections: 0,
            last_report: None,
        })
    }

    /// The plan.
    pub fn tree(&self) -> &PlanTree {
        &self.tree
    }

    /// The plan, to change the selection; call [`Self::refresh_summary`] afterwards.
    pub fn tree_mut(&mut self) -> &mut PlanTree {
        &mut self.tree
    }

    /// Recomputes the summary (and the work lists) from the plan's selection.
    pub fn refresh_summary(&mut self) {
        self.summary = PlanSummary::compute(&self.tree);
    }

    /// Selects everything below the root.
    // Mirrors ui/repository/workers/AddonsAutoUpdater.java:94-96, console/CommandGeneral.java:430-441
    pub fn select_all(&mut self) {
        select_all_except_root(&mut self.tree);
        self.refresh_summary();
    }

    /// The selection's totals and work lists.
    pub fn summary(&self) -> &PlanSummary {
        &self.summary
    }

    /// The metadata of the check.
    pub fn metadata(&self) -> &RemoteMetadata {
        &self.metadata
    }

    /// The options of the check.
    pub fn options(&self) -> &SyncOptions {
        &self.options
    }

    /// The request of the check.
    pub fn request(&self) -> &CheckRequest {
        &self.request
    }

    /// The report of the last download run that stored one.
    pub fn last_report(&self) -> Option<&str> {
        self.last_report.as_deref()
    }

    /// Downloads the selection: uncompresses the archives a previous run left, deletes the
    /// selected extra files, downloads (uncompressing as archives arrive), then stores the
    /// report and emits `Finished(outcome)` as the run's last event, except after a lost
    /// connection (`ConnectionLost` is then the last event and the session can resume).
    ///
    /// Cancelling `cancel` pauses: statuses and partial files are kept for the next call.
    // Mirrors service/synchronization/FilesSynchronizationProcessor.java:62-203,263-347,
    // service/connection/ConnectionDownloadProcessor.java:139-192,
    // ui/repository/workers/AddonsDownloader.java:286-415
    pub async fn download(
        &mut self,
        cancel: &CancellationToken,
        events: &EventSender,
    ) -> RunReport {
        // Mirrors service/synchronization/FilesSynchronizationProcessor.java:162-170,196-202
        if let Err(err) = check_destination(&self.options.destination) {
            let outcome = Outcome::Failed {
                error: EventError::new(err.kind(), err.to_string()),
            };
            return self.end_run(
                outcome,
                None,
                UnzipSummary::default(),
                CleanupReport::default(),
                events,
            );
        }

        // Mirrors service/ConnectionService.java:54
        let pool = UnzipPool::new();
        // Archives a previous run downloaded but did not uncompress. A zsync-patched leaf is
        // never replaced by a stale archive.
        // Mirrors service/ConnectionService.java:497-511
        let leftovers = leftover_archives(&self.summary.download_tasks);
        let leftovers = blocking(move || {
            leftovers
                .into_iter()
                .filter(|zip| zip.zip_path.is_file())
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        for zip in leftovers {
            pool.submit(zip);
        }

        // Mirrors service/synchronization/FilesSynchronizationProcessor.java:186
        let cleanup = {
            let tasks = self.summary.delete_tasks.clone();
            let roots = DeleteRoots::from_plan(&self.tree, &self.options.destination);
            let protected = protected_paths(&self.summary.download_tasks);
            let hidden = self.options.hidden_folder_paths.clone();
            let cancel = cancel.clone();
            let events = events.clone();
            blocking(move || {
                delete_extra_files(&tasks, &roots, &protected, &hidden, &cancel, &events)
            })
            .await
            .unwrap_or_default()
        };

        let request = DownloadRequest::new(
            self.request.protocol.clone(),
            self.transport_options.clone(),
            &self.options,
            self.metadata.server_info.as_deref(),
        );
        let sink: Arc<dyn ZipSink> = pool.clone();
        let report = {
            let download = download::download(
                &request,
                &mut self.summary.download_tasks,
                sink,
                cancel,
                events,
            );
            tokio::pin!(download);
            // A cancel stops uncompressing at once, while the download winds down.
            // Mirrors service/ConnectionService.java:551-556
            tokio::select! {
                report = &mut download => report,
                () = cancel.cancelled() => {
                    pool.cancel();
                    download.await
                }
            }
        };
        download::apply_statuses(&mut self.tree, &self.summary.download_tasks);

        // Mirrors service/synchronization/FilesSynchronizationProcessor.java:263-270,294-297
        self.average_speed = download::average_speed(self.average_speed, report.average_speed);
        self.max_active_connections = self
            .max_active_connections
            .max(report.max_active_connections);

        // Uncompressing ends with the download, or is cancelled with it.
        // Mirrors service/ConnectionService.java:551-556,
        // ui/repository/workers/AddonsDownloader.java:312-336,413-415
        let finished = if matches!(report.end, download::DownloadEnd::Completed) {
            tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                summary = pool.finish(events) => Some(summary),
            }
        } else {
            None
        };
        let (unzip, unzip_cancelled) = if let Some(summary) = finished {
            (summary, false)
        } else {
            // The next run may extract the same archives: wait for this pool to stop.
            pool.cancel();
            (pool.finish(&EventSender::noop()).await, true)
        };
        // A failed archive was deleted: its file is downloaded again by the next run.
        for error in &unzip.errors {
            let Some(path) = &error.relative_path else {
                continue;
            };
            for task in &mut self.summary.download_tasks {
                if task.relative_path == *path {
                    task.status = DownloadStatus::Pending;
                }
            }
        }
        download::apply_statuses(&mut self.tree, &self.summary.download_tasks);

        let outcome = if unzip_cancelled && matches!(report.end, download::DownloadEnd::Completed) {
            Some(Outcome::Cancelled)
        } else {
            report.outcome(unzip.errors.clone())
        };
        match outcome {
            Some(outcome) => self.end_run(outcome, Some(report), unzip, cleanup, events),
            None => RunReport {
                end: RunEnd::ConnectionLost,
                report_text: None,
                download: Some(report),
                unzip,
                cleanup,
            },
        }
    }

    /// Stores the report of `outcome` and emits `Finished(outcome)`.
    fn end_run(
        &mut self,
        outcome: Outcome,
        download: Option<DownloadReport>,
        unzip: UnzipSummary,
        cleanup: CleanupReport,
        events: &EventSender,
    ) -> RunReport {
        let url = host_url(&self.request.protocol);
        let finished_on = a3s_foundation::format::datetime_medium(&chrono::Local::now());
        let report_text = report_for(
            &ReportInput {
                repository_name: &self.request.repository_name,
                repository_url: &url,
                finished_on: &finished_on,
                average_speed: self.average_speed,
                max_active_connections: self.max_active_connections,
                tasks: &self.summary.download_tasks,
            },
            &outcome,
        );
        if report_text.is_some() {
            self.last_report.clone_from(&report_text);
        }
        events.emit(SyncEvent::Finished(outcome.clone()));
        RunReport {
            end: RunEnd::Finished(outcome),
            report_text,
            download,
            unzip,
            cleanup,
        }
    }

    /// After a successful synchronization: the stored repository takes the server revision.
    /// Returns `Ok`. Blocking (it writes the store).
    // Mirrors ui/repository/DownloadPanel.java:970-976
    pub fn mark_synced(&self, store: &dyn SyncStore) -> Result<RepositoryStatus, StoreError> {
        mark_synced(store, &self.request.repository_name, &self.metadata)
    }

    /// After a failed synchronization: the stored revision is reset to -1. Returns `Error`.
    /// Blocking (it writes the store).
    // Mirrors ui/repository/DownloadPanel.java:978-984
    pub fn mark_failed(&self, store: &dyn SyncStore) -> Result<RepositoryStatus, StoreError> {
        mark_failed(store, &self.request.repository_name)
    }

    /// What to do with the `userconfig` folder after a successful download.
    pub fn plan_userconfig_copy(&self, arma3_exe_path: Option<&Path>) -> UserconfigAction {
        plan_userconfig_copy(
            &self.tree,
            self.summary.userconfig_dir.as_deref(),
            &self.options.destination,
            arma3_exe_path,
        )
    }
}

/// Persists the server revision of `metadata` as the repository's revision.
pub(super) fn mark_synced(
    store: &dyn SyncStore,
    repository: &str,
    metadata: &RemoteMetadata,
) -> Result<RepositoryStatus, StoreError> {
    let mut status = RepositoryStatus::Ok;
    store.update(repository, &mut |stored, _| {
        status = status::mark_synced(stored, metadata);
        StoreChanges {
            repository: true,
            configuration: false,
        }
    })?;
    Ok(status)
}

/// Persists the revision -1 (Java only resets it in memory).
pub(super) fn mark_failed(
    store: &dyn SyncStore,
    repository: &str,
) -> Result<RepositoryStatus, StoreError> {
    let mut status = RepositoryStatus::Error;
    store.update(repository, &mut |stored, _| {
        status = status::mark_failed(stored);
        StoreChanges {
            repository: true,
            configuration: false,
        }
    })?;
    Ok(status)
}

/// The archives of the `Done` compressed files, which a previous run may have left.
fn leftover_archives(tasks: &[crate::DownloadTask]) -> Vec<DownloadedZip> {
    downloaded_compressed(tasks)
        .into_iter()
        .filter_map(|task| {
            let DownloadTaskKind::File {
                size,
                ref sha1,
                complete,
                ..
            } = task.kind
            else {
                return None;
            };
            let target = task.local_path();
            let mut zip_path = target.as_os_str().to_owned();
            zip_path.push(ZIP_EXTENSION);
            (complete == 0.0).then(|| DownloadedZip {
                relative_path: task.relative_path.clone(),
                zip_path: std::path::PathBuf::from(zip_path),
                target_path: target,
                sha1: sha1.clone(),
                size,
            })
        })
        .collect()
}

/// Runs blocking work (file system, store, user prompts) off the async threads. `None` when
/// the runtime is shutting down.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Some(value),
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        Err(_) => None,
    }
}

fn remote_error(err: RemoteError) -> CheckError {
    match err {
        RemoteError::Cancelled => CheckError::Cancelled,
        other => CheckError::Remote(other),
    }
}

fn ensure_not_cancelled(cancel: &CancellationToken) -> Result<(), CheckError> {
    if cancel.is_cancelled() {
        Err(CheckError::Cancelled)
    } else {
        Ok(())
    }
}

/// The destination may have vanished since the check.
// Mirrors service/synchronization/FilesSynchronizationProcessor.java:162-170
fn check_destination(destination: &Path) -> Result<(), SyncOptionsError> {
    if destination.as_os_str().is_empty() {
        return Err(SyncOptionsError::DestinationEmpty);
    }
    if !destination.exists() {
        return Err(SyncOptionsError::DestinationMissing(
            destination.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}
