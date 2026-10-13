//! The automatic update of one repository, in the background.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_foundation::error::ErrorKind;
use a3s_sync::cleanup::RemovedFiles;
use a3s_sync::pipeline::{
    CheckRequest, ReconnectPolicy, SyncEnd, SyncJob, SyncStore, UserconfigPolicy,
};
use a3s_sync::{EventError, Outcome};

use super::Synchronizer;
use crate::error::ContextError;
use crate::event::{AppEvent, Change};
use crate::store::StateStore;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// Starts the automatic update of `name`, unless the user is synchronizing it.
pub(super) fn start(
    cx: &Cx,
    name: &str,
    synchronizer: Arc<dyn Synchronizer>,
) -> Result<Option<TaskId>, ContextError> {
    if cx
        .is_running(&TaskKey::repository(TaskKind::Download, name))
        .is_some()
    {
        return Ok(None);
    }
    let key = TaskKey::repository(TaskKind::AutoUpdate, name);
    let name = name.to_owned();
    cx.spawn_task(key, true, move |task| run(task, name, synchronizer))
        .map(Some)
}

/// Checks, downloads everything, copies the `userconfig` folder without asking and checks
/// again, with the repository's settings. Extra local files go to the Trash. There is no
/// reconnect: nobody is there to confirm one.
// Mirrors ui/repository/DownloadPanel.java:1181-1269,
// ui/repository/workers/AddonsAutoUpdater.java:38-105
async fn run(
    task: TaskCtx,
    name: String,
    synchronizer: Arc<dyn Synchronizer>,
) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let job = {
        let state = cx.state().read();
        let repository = state.client.repository(&name)?;
        SyncJob {
            check: CheckRequest {
                repository_name: name.clone(),
                event_name: None,
                protocol: repository.protocol.clone(),
                addons: Arc::clone(&state.sync.addons),
                removed_files: RemovedFiles::Trash,
            },
            arma3_exe_path: state
                .client
                .launcher_options()
                .and_then(|options| options.arma3_exe_path.as_deref())
                .map(PathBuf::from),
            userconfig: UserconfigPolicy::Silent,
            reconnect: ReconnectPolicy::NONE,
        }
    };
    tracing::info!("Auto updating with repository: {name}");
    let store: Arc<dyn SyncStore> = Arc::new(StateStore::new(Arc::clone(cx.state())));
    let report = synchronizer
        .synchronize(job, store, task.cancel.clone(), task.sync_events())
        .await;

    {
        let mut state = cx.state().write();
        if state.client.repository(&name).is_ok() {
            let runtime = state.repositories.runtime_mut(&name);
            if let Some(status) = report.status {
                runtime.status = status;
            }
            if report.report_text.is_some() {
                runtime.download_report.clone_from(&report.report_text);
            }
        }
    }
    cx.emit(AppEvent::Changed(Change::Repository));

    let error = match report.end {
        SyncEnd::Finished(Outcome::Success) => {
            tracing::info!("Synchronization with repository: {name} finished.");
            cx.emit(AppEvent::Changed(Change::AddonFiles));
            return Ok(());
        }
        SyncEnd::Finished(Outcome::Cancelled) => return Err(TaskError::Cancelled),
        SyncEnd::Finished(Outcome::Failed { error }) => error,
        SyncEnd::Finished(outcome) => {
            let kind = match &outcome {
                Outcome::CompletedWithErrors { errors } | Outcome::TooManyErrors { errors, .. } => {
                    errors.first().map_or(ErrorKind::Internal, |e| e.kind)
                }
                _ => ErrorKind::Internal,
            };
            EventError::new(kind, outcome.download_report_message().unwrap_or_default())
        }
        SyncEnd::ConnectionLost => EventError::new(ErrorKind::ConnectionFailed, "Connection lost."),
    };
    tracing::warn!(
        "Synchronization with repository: {name} finished with errors: {}",
        error.message
    );
    Err(TaskError::Message(error))
}
