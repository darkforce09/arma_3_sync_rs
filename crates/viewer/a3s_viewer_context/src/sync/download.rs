//! The user's synchronization of one repository: the comparison with the local folder, the
//! download with pause and resume, and the results the Activity view lists.
//!
//! A comparison ([`compare`]) checks the repository and keeps the checked session, so the
//! download that follows ([`download`]) starts at once. Pausing cancels the download and keeps
//! the session: the next download resumes it (finished files are skipped, partial ones
//! continue). After failed files the session is kept too, so a retry downloads only those.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_sync::cleanup::RemovedFiles;
use a3s_sync::pipeline::{
    CheckError, CheckRequest, RunEnd, RunReport, StoreChanges, StoreError, SyncStore,
    UserconfigAction, UserconfigOutcome, copy_userconfig,
};
use a3s_sync::plan::StatusTree;
use a3s_sync::{EventError, EventSender, Outcome, Phase, SyncEvent, SyncSession};
use a3s_types::constants::RepositoryStatus;

use super::Event;
use crate::error::ContextError;
use crate::event::{AppEvent, Change};
use crate::state::AppState;
use crate::store::StateStore;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// How many finished downloads [`Downloads::recent`] keeps.
pub const RECENT_LIMIT: usize = 20;

/// How long a download waits after a lost connection before it goes on.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// How many times in a row a download goes on after a lost connection.
pub const RECONNECT_ATTEMPTS: u32 = 3;

/// What the viewer knows about the user's synchronizations, for this session only.
#[derive(Default)]
pub struct Downloads {
    /// The last comparison of each repository with its local folder.
    pub plans: BTreeMap<String, PlanState>,

    /// Finished downloads, newest first, at most [`RECENT_LIMIT`].
    pub recent: Vec<DownloadRecord>,

    /// Files being transferred, by repository.
    pub active_files: BTreeMap<String, Vec<String>>,

    /// Downloads waiting to go on after a lost connection: the attempt that comes next.
    pub reconnecting: BTreeMap<String, u32>,

    /// Checked sessions kept for the next download of their repository.
    sessions: BTreeMap<String, KeptSession>,

    /// Repositories whose running download is discarded instead of paused when it stops.
    stop_requested: BTreeSet<String>,
}

impl Downloads {
    /// The bytes still to download of a paused or interrupted download of `repository`.
    pub fn paused(&self, repository: &str) -> Option<u64> {
        self.sessions
            .get(repository)
            .filter(|kept| kept.reason == KeptReason::Paused)
            .map(|kept| remaining_bytes(&kept.session))
    }

    /// Every paused or interrupted download with the bytes it still has to download, in name
    /// order.
    pub fn paused_all(&self) -> Vec<(String, u64)> {
        self.sessions
            .iter()
            .filter(|(_, kept)| kept.reason == KeptReason::Paused)
            .map(|(name, kept)| (name.clone(), remaining_bytes(&kept.session)))
            .collect()
    }

    /// The newest finished download of `repository`.
    pub fn last_result(&self, repository: &str) -> Option<&DownloadRecord> {
        self.recent
            .iter()
            .find(|record| record.repository == repository)
    }
}

impl fmt::Debug for Downloads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Downloads")
            .field("plans", &self.plans)
            .field("recent", &self.recent)
            .field("active_files", &self.active_files)
            .field("sessions", &self.sessions.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// The comparison of a repository with its local folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanState {
    /// The server revision the comparison was made against; a newer one makes it stale.
    pub server_revision: Option<i32>,

    /// The comparison found addon folders through the profile's addon scan (auto-discover);
    /// one made without it is redone once the scan is there ([`super::plan_is_stale`]).
    pub addon_scan: bool,

    /// What a download would change, or why the comparison failed.
    pub result: Result<SyncPlan, EventError>,
}

/// What a download of the whole repository would do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// Bytes to download.
    pub download_size: u64,

    /// Files to download.
    pub file_count: usize,

    /// Files and folders to remove.
    pub delete_count: usize,

    /// Every file and folder of the repository with what a download does to it.
    pub tree: Arc<StatusTree>,
}

impl SyncPlan {
    /// Whether a download has nothing to do.
    pub fn is_up_to_date(&self) -> bool {
        self.tree.is_up_to_date()
    }
}

/// Which part of a repository a download takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    /// Everything that changed; a paused download goes on with the part it had.
    #[default]
    All,

    /// Everything but these paths below the repository root (folders with everything they
    /// hold): they are neither downloaded nor removed.
    Skip(Vec<String>),

    /// Only these paths below the repository root, folders with everything they hold.
    Only(Vec<String>),
}

/// A finished download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRecord {
    /// The repository.
    pub repository: String,

    /// The event it was limited to, if any.
    pub event: Option<String>,

    /// When it ended.
    pub finished: SystemTime,

    /// How it ended.
    pub result: DownloadResult,

    /// The download report (what was downloaded, saved and failed), when the download ran.
    pub report: Option<String>,
}

/// How a download ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadResult {
    /// Everything was downloaded.
    Success {
        /// Top-level folders and files moved to the Trash.
        trashed: usize,

        /// The repository's `userconfig` folder was copied into the Arma 3 folder (`Ok`), or
        /// why that failed; `None` when it was not needed.
        userconfig: Option<Result<(), String>>,
    },

    /// Finished, but these files failed (each error names its file); a retry downloads them
    /// again.
    Issues(Vec<EventError>),

    /// The download stopped with this error.
    Failed(EventError),
}

/// Why a checked session is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeptReason {
    /// Checked by a comparison; nothing downloaded yet.
    Compared,

    /// Paused, or interrupted by a lost connection.
    Paused,

    /// Finished with failed files.
    Issues,
}

struct KeptSession {
    session: SyncSession,
    reason: KeptReason,
}

/// Bytes the session still has to download.
fn remaining_bytes(session: &SyncSession) -> u64 {
    a3s_sync::plan::resumed_tasks(&session.summary().download_tasks)
        .iter()
        .map(|task| task.download_size())
        .fold(0, u64::saturating_add)
}

/// Forgets the comparison and the kept session of `repository`, e.g. after its connection or
/// download folder changed or it was removed.
pub fn forget(state: &mut AppState, repository: &str) {
    let downloads = &mut state.sync.downloads;
    downloads.plans.remove(repository);
    downloads.sessions.remove(repository);
}

/// Starts the comparison of `repository` with its local folder, unless one runs, a download of
/// it runs, or a download of it is paused. Returns the task when one started.
pub fn compare(cx: &Cx, repository: &str) -> Result<Option<TaskId>, ContextError> {
    let busy = [TaskKind::Compare, TaskKind::Download, TaskKind::AutoUpdate]
        .into_iter()
        .any(|kind| {
            cx.is_running(&TaskKey::repository(kind, repository))
                .is_some()
        });
    let paused = cx
        .state()
        .read()
        .sync
        .downloads
        .sessions
        .get(repository)
        .is_some_and(|kept| kept.reason != KeptReason::Compared);
    if busy || paused {
        return Ok(None);
    }
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::Compare, repository),
        true,
        move |task| run_compare(task, name),
    )
    .map(Some)
}

/// Starts the download of `repository` (limited to the addons of `event`): resumes its paused
/// download or uses its last comparison when they are still current, checks it otherwise.
pub fn download(cx: &Cx, repository: &str, event: Option<String>) -> Result<TaskId, ContextError> {
    start(cx, repository, event, Selection::All)
}

/// Starts the download of the `selection` of `repository` (limited to the addons of `event`), as
/// [`download`] does.
pub fn download_selection(
    cx: &Cx,
    repository: &str,
    event: Option<String>,
    selection: Selection,
) -> Result<TaskId, ContextError> {
    start(cx, repository, event, selection)
}

/// Resumes the paused download of `repository` with its event and the part it had; downloads
/// everything when none is paused.
pub fn resume(cx: &Cx, repository: &str) -> Result<TaskId, ContextError> {
    let event = cx
        .state()
        .read()
        .sync
        .downloads
        .sessions
        .get(repository)
        .and_then(|kept| kept.session.request().event_name.clone());
    start(cx, repository, event, Selection::All)
}

/// Downloads again the failed `files` of the last download of `repository`; every file still
/// to download when `files` is empty.
pub fn retry(cx: &Cx, repository: &str, files: Vec<String>) -> Result<TaskId, ContextError> {
    let event = cx
        .state()
        .read()
        .sync
        .downloads
        .last_result(repository)
        .and_then(|record| record.event.clone());
    let selection = if files.is_empty() {
        Selection::All
    } else {
        Selection::Only(files)
    };
    start(cx, repository, event, selection)
}

/// Pauses the download of `repository`: it stops and the next download resumes it.
pub fn pause(cx: &Cx, repository: &str) -> bool {
    cancel_download(cx, repository)
}

/// Stops the download of `repository` for good: it stops and its progress is discarded (the
/// downloaded files stay on disk and count at the next comparison).
pub fn stop(cx: &Cx, repository: &str) {
    let running = cx
        .is_running(&TaskKey::repository(TaskKind::Download, repository))
        .is_some();
    {
        let mut state = cx.state().write();
        let downloads = &mut state.sync.downloads;
        downloads.sessions.remove(repository);
        downloads.plans.remove(repository);
        if running {
            downloads.stop_requested.insert(repository.to_owned());
        }
    }
    if running {
        cancel_download(cx, repository);
    }
    cx.emit(AppEvent::Changed(Change::Repository));
}

fn cancel_download(cx: &Cx, repository: &str) -> bool {
    cx.is_running(&TaskKey::repository(TaskKind::Download, repository))
        .is_some_and(|id| cx.cancel_task(id))
}

fn start(
    cx: &Cx,
    repository: &str,
    event: Option<String>,
    selection: Selection,
) -> Result<TaskId, ContextError> {
    let auto = TaskKey::repository(TaskKind::AutoUpdate, repository);
    if let Some(id) = cx.is_running(&auto) {
        return Err(ContextError::TaskBusy { id, key: auto });
    }
    if let Some(id) = cx.is_running(&TaskKey::repository(TaskKind::Compare, repository)) {
        cx.cancel_task(id);
    }
    cx.state()
        .write()
        .sync
        .downloads
        .stop_requested
        .remove(repository);
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::Download, repository),
        true,
        move |task| run_download(task, name, event, selection),
    )
}

/// Checks the repository, publishes what a download would change and keeps the session.
async fn run_compare(task: TaskCtx, name: String) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let store = state_store(&cx);
    let events = task.sync_events();
    match check(&cx, &task, &name, None, &store, &events).await {
        Ok((mut session, addon_scan)) => {
            session.select_all();
            publish_plan(&cx, &name, session, KeptReason::Compared, addon_scan);
            Ok(())
        }
        Err(CheckError::Cancelled) => Err(TaskError::Cancelled),
        Err(err) => {
            let error = EventError::new(err.kind(), err.user_message());
            let mut state = cx.state().write();
            let server_revision = state.repositories.runtime(&name).metadata.server_revision();
            state.sync.downloads.plans.insert(
                name,
                PlanState {
                    server_revision,
                    addon_scan: false,
                    result: Err(error),
                },
            );
            drop(state);
            cx.emit(AppEvent::Changed(Change::Repository));
            Ok(())
        }
    }
}

/// Checks (or takes the kept session), downloads, and on success copies the `userconfig`
/// folder, compares again and stores the server revision. Ends with
/// [`Event::DownloadFinished`] unless it was paused or stopped.
async fn run_download(
    task: TaskCtx,
    name: String,
    event: Option<String>,
    selection: Selection,
) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let store = state_store(&cx);
    let events = track_files(&cx, &name, task.sync_events());

    let kept = take_session(&cx, &name, event.as_deref());
    let paused = kept
        .as_ref()
        .is_some_and(|kept| kept.reason == KeptReason::Paused);
    let mut session = match kept {
        Some(kept) => kept.session,
        None => match check(&cx, &task, &name, event.clone(), &store, &events).await {
            Ok((session, _)) => session,
            Err(CheckError::Cancelled) => return Err(TaskError::Cancelled),
            Err(err) => {
                let error = EventError::new(err.kind(), err.user_message());
                finish(&cx, &name, event, DownloadResult::Failed(error), None);
                return Ok(());
            }
        },
    };
    match &selection {
        Selection::All if paused => {}
        Selection::All => session.select_all(),
        Selection::Skip(names) => skip(&mut session, names),
        Selection::Only(paths) => select_only(&mut session, paths),
    }
    let removed_addons = session
        .summary()
        .delete_tasks
        .iter()
        .filter(|task| !task.relative_path.contains('/'))
        .count();

    let run = download_reconnecting(&task, &name, &mut session, &events).await;
    let report = run.report_text.clone();
    if report.is_some() {
        let mut state = cx.state().write();
        if state.client.repository(&name).is_ok() {
            state
                .repositories
                .runtime_mut(&name)
                .download_report
                .clone_from(&report);
        }
    }
    cx.state().write().sync.downloads.active_files.remove(&name);

    let outcome = match run.end {
        // Paused or stopped while waiting to reconnect.
        RunEnd::ConnectionLost if task.cancel.is_cancelled() => Outcome::Cancelled,
        RunEnd::ConnectionLost => {
            let error = EventError::new(ErrorKind::ConnectionFailed, "The connection was lost.");
            keep(&cx, &name, session, KeptReason::Paused);
            mark_failed(&cx, &store, &name).await;
            finish(&cx, &name, event, DownloadResult::Failed(error), report);
            return Ok(());
        }
        RunEnd::Finished(outcome) => outcome,
    };
    match outcome {
        Outcome::Success => {}
        Outcome::Cancelled => {
            let stopped = cx
                .state()
                .write()
                .sync
                .downloads
                .stop_requested
                .remove(&name);
            if !stopped {
                keep(&cx, &name, session, KeptReason::Paused);
            }
            cx.emit(AppEvent::Changed(Change::Repository));
            return Err(TaskError::Cancelled);
        }
        Outcome::CompletedWithErrors { errors } | Outcome::TooManyErrors { errors, .. } => {
            keep(&cx, &name, session, KeptReason::Issues);
            mark_failed(&cx, &store, &name).await;
            finish(&cx, &name, event, DownloadResult::Issues(errors), report);
            return Ok(());
        }
        other => {
            let error = match other {
                Outcome::Failed { error } => error,
                other => EventError::new(
                    ErrorKind::Internal,
                    other.download_report_message().unwrap_or_default(),
                ),
            };
            mark_failed(&cx, &store, &name).await;
            finish(&cx, &name, event, DownloadResult::Failed(error), report);
            return Ok(());
        }
    }

    let userconfig = copy_userconfig_folder(&cx, &session, &events).await;
    let trashed = removed_addons.min(run.cleanup.deleted);
    match check(
        &cx,
        &task,
        &name,
        event.clone(),
        &store,
        &EventSender::noop(),
    )
    .await
    {
        Ok((mut second, addon_scan)) => {
            second.select_all();
            let metadata = second.metadata().clone();
            let synced = {
                let store = Arc::clone(&store);
                blocking(move || {
                    second
                        .mark_synced(store.as_ref())
                        .map(|status| (status, second))
                })
                .await
            };
            match synced {
                Some(Ok((status, second))) => {
                    set_status(&cx, &name, status);
                    if event.is_none() {
                        publish_plan(&cx, &name, second, KeptReason::Compared, addon_scan);
                    }
                }
                Some(Err(err)) => tracing::warn!("Failed to save repository {name}: {err}"),
                None => {}
            }
            let mut state = cx.state().write();
            state.repositories.runtime_mut(&name).metadata = metadata;
            if let Err(err) = crate::profiles::refresh_modset_groups(&mut state, &name) {
                tracing::warn!(
                    "Cannot update the groups of repository {name}: {}",
                    err.user_message()
                );
            }
        }
        Err(CheckError::Cancelled) => {}
        Err(err) => tracing::warn!(
            "Checking repository {name} after its download failed: {}",
            err.user_message()
        ),
    }
    tracing::info!("Synchronization with repository: {name} finished.");
    cx.emit(AppEvent::Changed(Change::AddonFiles));
    finish(
        &cx,
        &name,
        event,
        DownloadResult::Success {
            trashed,
            userconfig,
        },
        report,
    );
    Ok(())
}

fn state_store(cx: &Cx) -> Arc<dyn SyncStore> {
    Arc::new(StateStore::new(Arc::clone(cx.state())))
}

/// Downloads the session's selection; after a lost connection it waits
/// [`RECONNECT_DELAY`] and goes on where it stopped, at most [`RECONNECT_ATTEMPTS`] times in a
/// row. While it waits, [`Downloads::reconnecting`] holds the attempt.
// Mirrors ui/repository/dialogs/ConnectionLostDialog.java:30-90
async fn download_reconnecting(
    task: &TaskCtx,
    name: &str,
    session: &mut SyncSession,
    events: &EventSender,
) -> RunReport {
    let cx = &task.cx;
    let mut attempt = 0;
    loop {
        let run = session.download(&task.cancel, events).await;
        if run.end != RunEnd::ConnectionLost || attempt == RECONNECT_ATTEMPTS {
            return run;
        }
        attempt += 1;
        tracing::info!("Lost the connection to {name}; reconnecting (attempt {attempt}).");
        set_reconnecting(cx, name, Some(attempt));
        let deadline = cx.clock().now() + RECONNECT_DELAY;
        let waited = tokio::select! {
            () = cx.clock().sleep_until(deadline) => true,
            () = task.cancel.cancelled() => false,
        };
        set_reconnecting(cx, name, None);
        if !waited {
            return run;
        }
    }
}

fn set_reconnecting(cx: &Cx, name: &str, attempt: Option<u32>) {
    {
        let mut state = cx.state().write();
        let reconnecting = &mut state.sync.downloads.reconnecting;
        match attempt {
            Some(attempt) => reconnecting.insert(name.to_owned(), attempt),
            None => reconnecting.remove(name),
        };
    }
    cx.emit(AppEvent::Changed(Change::Repository));
}

/// Checks `name` with its stored protocol; extra local files go to the Trash. Also says
/// whether auto-discovery could use the profile's addon scan.
async fn check(
    cx: &Cx,
    task: &TaskCtx,
    name: &str,
    event: Option<String>,
    store: &Arc<dyn SyncStore>,
    events: &EventSender,
) -> Result<(SyncSession, bool), CheckError> {
    let (request, addon_scan) = {
        let state = cx.state().read();
        let repository = state
            .client
            .repository(name)
            .map_err(|_not_found| CheckError::Store(StoreError::repository_not_found(name)))?;
        let (addons, addon_scan) = super::addon_locator(&state);
        let request = CheckRequest {
            repository_name: name.to_owned(),
            event_name: event,
            protocol: repository.protocol.clone(),
            addons,
            removed_files: RemovedFiles::Trash,
        };
        (request, addon_scan)
    };
    tracing::info!("Checking repository {name} against its download folder");
    SyncSession::check(request, Arc::clone(store), &task.cancel, events)
        .await
        .map(|session| (session, addon_scan))
}

/// The kept session of `name` for `event`, unless the server has a newer revision since.
fn take_session(cx: &Cx, name: &str, event: Option<&str>) -> Option<KeptSession> {
    let mut state = cx.state().write();
    let current = state.repositories.runtime(name).metadata.server_revision();
    let kept = state.sync.downloads.sessions.remove(name)?;
    let same_event = kept.session.request().event_name.as_deref() == event;
    let checked = kept.session.metadata().server_revision();
    let fresh = current.is_none() || checked == current;
    (same_event && fresh).then_some(kept)
}

fn keep(cx: &Cx, name: &str, session: SyncSession, reason: KeptReason) {
    let mut state = cx.state().write();
    if state.client.repository(name).is_ok() {
        state
            .sync
            .downloads
            .sessions
            .insert(name.to_owned(), KeptSession { session, reason });
    }
}

/// Selects only `paths`, what they hold and the folders leading to them.
fn select_only(session: &mut SyncSession, paths: &[String]) {
    session.tree_mut().walk_mut(&mut |path, node| {
        let wanted = paths
            .iter()
            .any(|wanted| is_below(wanted, path) || is_below(path, wanted));
        node.set_selected(wanted);
    });
    session.refresh_summary();
}

/// Selects everything but the paths `names` (folders with everything they hold).
fn skip(session: &mut SyncSession, names: &[String]) {
    session.select_all();
    for name in names {
        session.tree_mut().set_selected_recursive(name, false);
    }
    session.refresh_summary();
}

/// Stores the session's plan as the comparison of `name` and keeps the session; `addon_scan`
/// says whether its check found addon folders through the profile's addon scan.
fn publish_plan(cx: &Cx, name: &str, session: SyncSession, reason: KeptReason, addon_scan: bool) {
    let plan = plan_of(&session);
    {
        let mut state = cx.state().write();
        if state.client.repository(name).is_err() {
            return;
        }
        let metadata = session.metadata().clone();
        state.sync.downloads.plans.insert(
            name.to_owned(),
            PlanState {
                server_revision: metadata.server_revision(),
                addon_scan,
                result: Ok(plan),
            },
        );
        let runtime = state.repositories.runtime_mut(name);
        runtime.metadata = metadata;
        state
            .sync
            .downloads
            .sessions
            .insert(name.to_owned(), KeptSession { session, reason });
    }
    cx.emit(AppEvent::Changed(Change::Repository));
}

/// What downloading the whole session does, with the status of every file and folder.
fn plan_of(session: &SyncSession) -> SyncPlan {
    let summary = session.summary();
    SyncPlan {
        download_size: summary.total_download_size,
        file_count: summary
            .download_tasks
            .iter()
            .filter(|task| !matches!(task.kind, a3s_sync::DownloadTaskKind::Directory))
            .count(),
        delete_count: summary.delete_tasks.len(),
        tree: Arc::new(StatusTree::from_plan(session.tree())),
    }
}

fn is_below(path: &str, top: &str) -> bool {
    path == top
        || path
            .strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Copies the downloaded `userconfig` folder into the Arma 3 folder, without asking; returns
/// whether it did, `None` when nothing was to copy.
// Mirrors ui/repository/DownloadPanel.java:1031-1048
async fn copy_userconfig_folder(
    cx: &Cx,
    session: &SyncSession,
    events: &EventSender,
) -> Option<Result<(), String>> {
    let arma3_exe = cx
        .state()
        .read()
        .client
        .launcher_options()
        .and_then(|options| options.arma3_exe_path.as_deref())
        .map(PathBuf::from);
    let UserconfigAction::Copy(copy) = session.plan_userconfig_copy(arma3_exe.as_deref()) else {
        return None;
    };
    events.emit(SyncEvent::Phase(Phase::UpdatingUserconfig));
    let outcome = blocking(move || copy_userconfig(&copy)).await?;
    tracing::info!("Userconfig: {outcome:?}");
    match outcome {
        UserconfigOutcome::Copied(_) => Some(Ok(())),
        UserconfigOutcome::SourceMissing(_) | UserconfigOutcome::Declined => None,
        UserconfigOutcome::CreateFailed(reason) | UserconfigOutcome::CopyFailed(reason) => {
            Some(Err(reason))
        }
    }
}

/// Stores -1 as the repository's revision (a failed synchronization) and its status.
async fn mark_failed(cx: &Cx, store: &Arc<dyn SyncStore>, name: &str) {
    let marked = {
        let store = Arc::clone(store);
        let name = name.to_owned();
        blocking(move || {
            let mut status = RepositoryStatus::Error;
            store
                .update(&name, &mut |repository, _| {
                    status = a3s_sync::status::mark_failed(repository);
                    StoreChanges {
                        repository: true,
                        configuration: false,
                    }
                })
                .map(|()| status)
        })
        .await
    };
    match marked {
        Some(Ok(status)) => set_status(cx, name, status),
        Some(Err(err)) => tracing::warn!("Failed to save repository {name}: {err}"),
        None => {}
    }
}

fn set_status(cx: &Cx, name: &str, status: RepositoryStatus) {
    let mut state = cx.state().write();
    if state.client.repository(name).is_ok() {
        state.repositories.runtime_mut(name).status = status;
    }
}

/// Records a finished download and announces it.
fn finish(
    cx: &Cx,
    name: &str,
    event: Option<String>,
    result: DownloadResult,
    report: Option<String>,
) {
    match &result {
        DownloadResult::Success { .. } => {}
        DownloadResult::Issues(errors) => tracing::warn!(
            "Synchronization with repository: {name} finished with {} failed files",
            errors.len()
        ),
        DownloadResult::Failed(error) => tracing::warn!(
            "Synchronization with repository: {name} failed: {}",
            error.message
        ),
    }
    {
        let mut state = cx.state().write();
        let recent = &mut state.sync.downloads.recent;
        recent.insert(
            0,
            DownloadRecord {
                repository: name.to_owned(),
                event,
                finished: cx.clock().now(),
                result: result.clone(),
                report,
            },
        );
        recent.truncate(RECENT_LIMIT);
    }
    // Even a failed download may have written or trashed mods.
    crate::profiles::files_changed(cx);
    cx.emit(AppEvent::Changed(Change::Repository));
    cx.emit(AppEvent::Sync(Event::DownloadFinished {
        repository: name.to_owned(),
        result,
    }));
}

/// Passes events on and keeps the files being transferred in [`Downloads::active_files`].
fn track_files(cx: &Cx, name: &str, inner: EventSender) -> EventSender {
    let cx = cx.clone();
    let name = name.to_owned();
    EventSender::from_fn(move |event| {
        match &event {
            SyncEvent::FileStarted { relative_path } => {
                let mut state = cx.state().write();
                let files = state
                    .sync
                    .downloads
                    .active_files
                    .entry(name.clone())
                    .or_default();
                if !files.contains(relative_path) {
                    files.push(relative_path.clone());
                }
            }
            SyncEvent::FileDone { relative_path } | SyncEvent::FileFailed { relative_path, .. } => {
                let mut state = cx.state().write();
                if let Some(files) = state.sync.downloads.active_files.get_mut(&name) {
                    files.retain(|file| file != relative_path);
                }
            }
            _ => {}
        }
        inner.emit(event);
    })
}

/// Runs blocking work off the async threads; `None` when the runtime shuts down.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Some(value),
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        Err(_) => None,
    }
}

/// Handles the download commands; a command that cannot start is reported as
/// [`AppEvent::Error`].
pub(super) fn handle(cx: &Cx, command: super::Command) {
    let context = format!("{command:?}");
    let started = match command {
        super::Command::AutoUpdate { .. } => return,
        super::Command::Compare { repository } => compare(cx, &repository).map(drop),
        super::Command::Download {
            repository,
            event,
            selection,
        } => {
            // Mirrors ui/repository/DownloadPanel.java:1005-1014
            if cx.state().read().tools.platform.is_arma3_running() {
                cx.emit(AppEvent::Sync(Event::GameRunning {
                    repository,
                    event,
                    selection,
                }));
                return;
            }
            download_selection(cx, &repository, event, selection).map(drop)
        }
        super::Command::DownloadAnyway {
            repository,
            event,
            selection,
        } => download_selection(cx, &repository, event, selection).map(drop),
        super::Command::Resume { repository } => resume(cx, &repository).map(drop),
        super::Command::Retry { repository, files } => retry(cx, &repository, files).map(drop),
        super::Command::Pause { repository } => {
            pause(cx, &repository);
            Ok(())
        }
        super::Command::Stop { repository } => {
            stop(cx, &repository);
            Ok(())
        }
    };
    if let Err(err) = started {
        tracing::warn!("{context}: {err}");
        cx.emit(AppEvent::Error {
            context,
            error: EventError::new(err.kind(), err.user_message()),
        });
    }
}
